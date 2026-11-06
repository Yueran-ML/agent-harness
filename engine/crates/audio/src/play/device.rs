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

use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
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
    /// The frames of sound that a capture could not take (its ring was full).
    dropped: AtomicU64,
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

    /// The frames of sound that a capture could not take, so far: its file lacks them.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// Where a test's device writes every frame of sound that it gives (`--sound capture`): a ring
/// of the frames, and a ring of where each session starts among them (the frames captured
/// before it), so that the thread that writes them puts each session in a file of its own.
#[derive(Debug)]
pub struct Capture {
    /// The frames.
    pub ring: SampleProducer,
    /// The frame at which each session after the first starts.
    pub sessions: MessageProducer<u64>,
}

/// The callback's side. See the module.
pub struct Player {
    orders: MessageConsumer<Order>,
    back: MessageProducer<Feeding>,
    current: Option<Feeding>,
    /// One that could not be sent back yet: while it waits, no order is taken.
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
    capture: Option<Capture>,
    /// The frames captured so far, and whether a session was captured yet.
    captured: (u64, bool),
    /// The gain of what is heard, as `f32` bits (the monitor's level, mute and DIM), where one
    /// is given ([`Player::monitored`]), and the gain applied to the last frame.
    monitor: Option<Arc<AtomicU32>>,
    level: f32,
}

/// How long the gain of the monitor takes to go from silence to full, or back: 10 ms, so that a
/// change of the level, mute or DIM is heard without a click.
pub const MONITOR_RAMP_SECONDS: f32 = 0.010;

/// The frames of `samples` (of `channels` channels at `rate`) at the monitor's gain `gain`,
/// `level` the gain of the frame before, moved towards it frame by frame.
fn monitor_gain(gain: Option<&AtomicU32>, level: &mut f32, (rate, channels): (u32, usize), samples: &mut [f32]) {
    let Some(gain) = gain else {
        return;
    };
    let target = f32::from_bits(gain.load(Ordering::Relaxed));
    #[allow(clippy::float_cmp, reason = "exactly unity: the sound is given as it is")]
    if target == 1.0 && *level == 1.0 {
        return;
    }
    #[allow(clippy::cast_precision_loss, reason = "a rate of samples")]
    let step = 1.0 / (rate as f32 * MONITOR_RAMP_SECONDS).max(1.0);
    for frame in samples.chunks_exact_mut(channels) {
        *level = if *level < target {
            (*level + step).min(target)
        } else {
            (*level - step).max(target)
        };
        for sample in frame {
            *sample *= *level;
        }
    }
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
    capture: Option<Capture>,
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
            captured: (0, false),
            monitor: None,
            level: 1.0,
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
    /// What it gives is heard at the gain held in `gain` (its `f32` bits, from 0 to 1): the
    /// monitor's level, mute and DIM, which never reach a render. The gain moves to a new value
    /// over [`MONITOR_RAMP_SECONDS`] at most; at 1, the sound is given as it is.
    #[must_use]
    pub fn monitored(mut self, gain: Arc<AtomicU32>) -> Self {
        self.level = f32::from_bits(gain.load(Ordering::Relaxed));
        self.monitor = Some(gain);
        self
    }

    /// Sends `feeding` back to be dropped, or keeps it to send at the next run (no order is
    /// taken until it is sent, so that there is never more than one).
    fn give_back(&mut self, feeding: Feeding) {
        if let Err(feeding) = self.back.push(feeding) {
            self.stranded = Some(feeding);
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
        while self.stranded.is_none() {
            let Some(order) = self.orders.pop() else {
                break;
            };
            if let Some(old) = self.current.take() {
                self.give_back(old);
            }
            if let Order::Play(feeding) = order {
                self.current = Some(feeding);
                self.primed = false;
                self.played = 0;
                self.fed = None;
                if let Some(capture) = &mut self.capture {
                    if std::mem::replace(&mut self.captured.1, true) {
                        let _ = capture.sessions.push(self.captured.0);
                    }
                }
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
            monitor_gain(
                self.monitor.as_deref(),
                &mut self.level,
                (self.rate, channels),
                &mut self.scratch[..got * channels],
            );
            let samples = &self.scratch[..got * channels];
            for (slot, &sample) in data[done * channels..(done + got) * channels]
                .iter_mut()
                .zip(samples)
            {
                *slot = convert(if self.mute { 0.0 } else { sample });
            }
            if let Some(capture) = &mut self.capture {
                let taken = capture.ring.push(samples);
                self.captured.0 += u64::try_from(taken).unwrap_or(0);
                let lost = u64::try_from(got - taken).unwrap_or(0);
                if lost > 0 {
                    self.shared.dropped.fetch_add(lost, Ordering::Relaxed);
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rt::samples;

    #[test]
    fn the_monitor_gain_ramps_to_its_value_and_leaves_unity_as_it_is() {
        // At 1000 frames a second the ramp of 10 ms is 0.1 a frame.
        let gain = Arc::new(AtomicU32::new(1.0_f32.to_bits()));
        let (player, mut control) = player((1, 1000), false, false, None);
        let mut player = player.monitored(Arc::clone(&gain));
        let (mut producer, consumer) = samples(64, 1);
        assert_eq!(producer.push(&[0.5; 16]), 16);
        control.orders.push(Order::Play(Feeding { generation: 1, ring: consumer })).unwrap();
        let mut heard = [0.0_f32; 4];
        player.fill(&mut heard, (0, 0), 0.0, |sample| sample);
        assert_eq!(heard, [0.5; 4], "at unity the sound is given as it is");
        gain.store(0.5_f32.to_bits(), Ordering::Relaxed);
        let mut heard = [0.0_f32; 8];
        player.fill(&mut heard, (0, 0), 0.0, |sample| sample);
        let expected = [0.9_f32, 0.8, 0.7, 0.6, 0.5, 0.5, 0.5, 0.5].map(|gain| gain * 0.5);
        for (heard, expected) in heard.iter().zip(expected) {
            assert!((heard - expected).abs() < 1e-6, "{heard} {expected}");
        }
        // Muted: down towards silence, 0.1 a frame.
        gain.store(0.0_f32.to_bits(), Ordering::Relaxed);
        let mut heard = [1.0_f32; 4];
        player.fill(&mut heard, (0, 0), 0.0, |sample| sample);
        for (heard, expected) in heard.iter().zip([0.4_f32, 0.3, 0.2, 0.1].map(|gain| gain * 0.5)) {
            assert!((heard - expected).abs() < 1e-6, "{heard} {expected}");
        }
    }
}
