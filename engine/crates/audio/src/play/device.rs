//! What the callback of the device does (plan T5): it takes the sound of the session that plays
//! from its ring, silence for what the ring does not hold, and tells what it gave and when that is
//! heard through a seqlock ([`Mark`]), the clock of playing. It neither locks, takes or frees
//! memory, nor waits: a new session comes through a ring of [`Order`]s, and the one it ends goes
//! back through another to be dropped by the thread that started it.
//!
//! The first run that a device of WASAPI asks for once a session starts is silence, not counted
//! (`prime`): WASAPI says that it is heard at once while it is laid ahead of what plays, some
//! 20 ms, so that sound in it would be counted as heard that much too early (as phases 1 and 2
//! did, `server/src/audio.rs`).

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use crate::rt::{
    messages, seqlock, MessageConsumer, MessageProducer, SampleConsumer, SampleProducer, SeqReader,
    SeqWriter, Words,
};

/// What the device was given in its last run: which session's, the frames of sound before it,
/// how many of its first frames were sound (the rest silence), whether all were, when its first
/// is heard and when it was asked for (in nanoseconds from an instant of the caller's), how long
/// it lasts, and when the session first had sound waiting for it (`None` before).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mark {
    /// The session, as its [`Feeding`] numbers it; 0 for none yet.
    pub generation: u64,
    /// The frames of sound given before this run.
    pub before: u64,
    /// The frames of sound at the start of this run.
    pub sound: u64,
    /// Whether all of it was sound.
    pub full: bool,
    /// When its first frame is heard.
    pub heard: i64,
    /// When it was asked for.
    pub asked: i64,
    /// How long it lasts, in nanoseconds.
    pub run: u64,
    /// When the session first had sound waiting in its ring.
    pub fed: Option<i64>,
}

impl Words for Mark {
    const WORDS: usize = 16;

    fn word(&self, index: usize) -> u32 {
        match index {
            0 | 1 => self.generation.word(index),
            2 | 3 => self.before.word(index - 2),
            4 | 5 => self.sound.word(index - 4),
            6 => u32::from(self.full),
            7 | 8 => self.heard.word(index - 7),
            9 | 10 => self.asked.word(index - 9),
            11 | 12 => self.run.word(index - 11),
            13 => u32::from(self.fed.is_some()),
            _ => self.fed.unwrap_or(0).word(index - 14),
        }
    }

    fn from_words(word: impl Fn(usize) -> u32) -> Self {
        let pair = |at: usize| u64::from_words(|index| word(at + index));
        let signed = |at: usize| i64::from_words(|index| word(at + index));
        Mark {
            generation: pair(0),
            before: pair(2),
            sound: pair(4),
            full: word(6) != 0,
            heard: signed(7),
            asked: signed(9),
            run: pair(11),
            fed: (word(13) != 0).then(|| signed(14)),
        }
    }
}

/// The sound of a session, as the device takes it: its ring, interleaved in the channels of the
/// device, and its number.
#[derive(Debug)]
pub struct Feeding {
    /// Which session (from 1).
    pub generation: u64,
    /// Its sound.
    pub ring: SampleConsumer,
}

/// What the callback is told.
#[derive(Debug)]
pub enum Order {
    /// Play this session in place of the one before.
    Play(Feeding),
    /// Play no session: silence.
    Stop,
}

/// What the callback counts, for the thread that plays.
#[derive(Debug, Default)]
pub struct Shared {
    /// The runs that had less sound than they asked for after the first sound of their
    /// session: the mix fell behind.
    short: AtomicU64,
    /// The frames of the device's last run.
    period: AtomicUsize,
}

impl Shared {
    /// The runs short of sound so far.
    #[must_use]
    pub fn short(&self) -> u64 {
        self.short.load(Ordering::Relaxed)
    }

    /// The frames of the device's last run (0 before the first).
    #[must_use]
    pub fn period(&self) -> usize {
        self.period.load(Ordering::Relaxed)
    }
}

/// The callback's side. See the module.
pub struct Player {
    orders: MessageConsumer<Order>,
    back: MessageProducer<Feeding>,
    current: Option<Feeding>,
    /// One that could not be sent back yet.
    stranded: Option<Feeding>,
    channels: usize,
    rate: u32,
    prime: bool,
    mute: bool,
    primed: bool,
    played: u64,
    fed: Option<i64>,
    mark: SeqWriter<Mark>,
    shared: Arc<Shared>,
    scratch: Box<[f32]>,
    capture: Option<SampleProducer>,
}

impl std::fmt::Debug for Player {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Player")
            .field("channels", &self.channels)
            .field("rate", &self.rate)
            .field("played", &self.played)
            .finish_non_exhaustive()
    }
}

/// The side of the thread that starts and ends sessions.
#[derive(Debug)]
pub struct Control {
    /// Where sessions are sent to the callback.
    pub orders: MessageProducer<Order>,
    /// The sessions the callback let go of, to drop.
    pub back: MessageConsumer<Feeding>,
    /// What the device was last given.
    pub mark: SeqReader<Mark>,
    /// What the callback counts.
    pub shared: Arc<Shared>,
}

/// The callback of a device of `channels` channels at `rate` samples a second, and the side that
/// tells it what to play. `prime`: the first run of each session is silence, not counted;
/// `mute`: silence is played in the place of the sound, which is counted all the same;
/// `capture`: every frame of sound it gives is also written there (a test's device). Made off
/// the threads of the sound.
///
/// # Panics
///
/// When `channels` or `rate` is 0.
#[must_use]
pub fn player(
    (channels, rate): (usize, u32),
    prime: bool,
    mute: bool,
    capture: Option<SampleProducer>,
) -> (Player, Control) {
    assert!(channels > 0 && rate > 0, "a device of nothing");
    let (orders, taken) = messages(16);
    let (back, returned) = messages(32);
    let (mark, reader) = seqlock(Mark::default());
    let shared = Arc::new(Shared::default()); // rt-ok: made off the threads of the sound
    let scratch = vec![0.0; 1024 * channels]; // rt-ok: made off the threads of the sound
    (
        Player {
            orders: taken,
            back,
            current: None,
            stranded: None,
            channels,
            rate,
            prime,
            mute,
            primed: false,
            played: 0,
            fed: None,
            mark,
            shared: Arc::clone(&shared),
            scratch: scratch.into_boxed_slice(),
            capture,
        },
        Control {
            orders,
            back: returned,
            mark: reader,
            shared,
        },
    )
}

impl Player {
    /// Sends `feeding` back to be dropped, or keeps it to send at the next run.
    fn give_back(&mut self, feeding: Feeding) {
        if let Err(feeding) = self.back.push(feeding) {
            if self.stranded.is_none() {
                self.stranded = Some(feeding);
            } else {
                std::mem::forget(feeding);
            }
        }
    }

    /// Fills a run of the device, asked for at `asked` and heard from `heard` (nanoseconds from
    /// the caller's instant), with the sound of the session, `convert`ed, and `silence` after it;
    /// marks what was sound and when it is heard.
    pub fn fill<T: Copy>(
        &mut self,
        data: &mut [T],
        (asked, heard): (i64, i64),
        silence: T,
        convert: impl Fn(f32) -> T,
    ) {
        if let Some(feeding) = self.stranded.take() {
            self.give_back(feeding);
        }
        while let Some(order) = self.orders.pop() {
            if let Some(old) = self.current.take() {
                self.give_back(old);
            }
            if let Order::Play(feeding) = order {
                self.current = Some(feeding);
                self.primed = false;
                self.played = 0;
                self.fed = None;
            }
        }
        let channels = self.channels;
        let frames = data.len() / channels;
        self.shared.period.store(frames, Ordering::Relaxed);
        let run = u64::try_from(frames).unwrap_or(0) * 1_000_000_000 / u64::from(self.rate);
        let Some(feeding) = &mut self.current else {
            for slot in data.iter_mut() {
                *slot = silence;
            }
            return;
        };
        let generation = feeding.generation;
        if self.prime && !self.primed {
            // The first run: silence, not counted, since it is told as heard too early.
            self.primed = true;
            for slot in data.iter_mut() {
                *slot = silence;
            }
            self.mark.write(Mark {
                generation,
                before: 0,
                sound: 0,
                full: false,
                heard,
                asked,
                run,
                fed: self.fed,
            });
            return;
        }
        let available = feeding.ring.available();
        if available > 0 && self.fed.is_none() {
            self.fed = Some(asked);
        }
        let wanted = frames.min(available);
        let piece = self.scratch.len() / channels;
        let mut done = 0;
        while done < wanted {
            let count = (wanted - done).min(piece);
            let got = feeding.ring.pop(&mut self.scratch[..count * channels]);
            let samples = &self.scratch[..got * channels];
            for (slot, &sample) in data[done * channels..(done + got) * channels]
                .iter_mut()
                .zip(samples)
            {
                *slot = convert(if self.mute { 0.0 } else { sample });
            }
            if let Some(capture) = &mut self.capture {
                capture.push(samples);
            }
            done += got;
            if got < count {
                break;
            }
        }
        for slot in &mut data[done * channels..] {
            *slot = silence;
        }
        let before = self.played;
        let sound = u64::try_from(done).unwrap_or(0);
        let full = done == frames;
        if before > 0 && !full {
            self.shared.short.fetch_add(1, Ordering::Relaxed);
        }
        self.played += sound;
        self.mark.write(Mark {
            generation,
            before,
            sound,
            full,
            heard,
            asked,
            run,
            fed: self.fed,
        });
    }
}
