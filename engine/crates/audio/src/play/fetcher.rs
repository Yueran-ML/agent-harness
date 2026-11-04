//! Reading the clips of a graph ahead of it (plan T5), off the threads of the sound: each clip's
//! media is read in pieces ([`Chunk`]) from where the graph reads ([`super::feed::Reading`]) to
//! [`PREFETCH_AHEAD`] after, nearest first, through any [`ClipFeed`] that reads as it is asked
//! (the media readers of the server, which open, seek and decode; a test's own). A piece is read
//! into memory that a piece the graph let go of had, where there is one. A media that is slow to
//! read holds this thread up, never the thread of the mix, which plays silence for what has not
//! come and counts it.
//!
//! Here memory is taken and the thread sleeps: this file is not one of those of the threads of
//! the sound (`tests/rt_audit.rs`).
//!
//! [`PREFETCH_AHEAD`]: crate::constants::PREFETCH_AHEAD

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use super::feed::{Chunk, Reading, RingFeed};
use super::Counters;
use crate::constants::{samples, PREFETCH_AHEAD};
use crate::feed::ClipFeed;
use crate::graph::{Graph, Reach};
use crate::rt::{messages, MessageConsumer, MessageProducer};

/// The samples of a piece of a clip played at speed 1 or less: a piece of a clip played faster
/// is as many times longer, so that a block reads few pieces.
pub const PIECE: usize = 2048;

/// The most pieces in the ring from a reader to the graph.
const IN_FLIGHT: usize = 256;

/// The most readers ahead of a graph: the clips of a media are read by one of them, the media in
/// turn, so that a media that is slow to read (a drive asleep, a decoder that hangs) holds up the
/// clips of the media it shares a reader with and no others.
pub const READERS: usize = 4;

/// The threads reading ahead that have not ended, of every graph: those whose graph is gone end
/// once the read they are in returns.
static LIVE: AtomicUsize = AtomicUsize::new(0);

/// How long the reader sleeps when it has read all that is wanted.
const NAP: Duration = Duration::from_millis(2);

/// How a clip is read ahead.
#[derive(Debug)]
struct Plan {
    reach: Reach,
    channels: usize,
    /// The samples of a piece.
    frames: usize,
    /// The most pieces of it out at once.
    budget: usize,
    /// The pieces sent and not given back.
    sent: BTreeSet<i64>,
    /// The memory of pieces given back.
    spare: Vec<Box<[f32]>>,
}

/// The reading ahead of some of the clips of a graph (those of the media given to one reader):
/// the ends of the rings that the reader holds.
#[derive(Debug)]
pub struct Fetch {
    out: MessageProducer<Chunk>,
    back: MessageConsumer<Chunk>,
    reading: Arc<Reading>,
    /// The plan of each clip of the graph that this reader reads, by its place in the graph.
    clips: Vec<Option<Plan>>,
    /// How far ahead of where the graph reads.
    ahead: i64,
    /// The clip whose turn it is first.
    turn: usize,
}

/// The feed of `graph` from what is read ahead, and the readers ahead that fill it (at most
/// [`READERS`], the clips of each media given to one), the graph reading at `from` first. Made
/// off the threads of the sound.
#[must_use]
pub fn ring_feed(graph: &Graph, from: i64, counters: Arc<Counters>) -> (RingFeed, Vec<Fetch>) {
    let rate = graph.rate();
    let ahead = samples(PREFETCH_AHEAD, rate);
    let reaches = graph.reaches();
    let clips: Vec<Plan> = reaches
        .iter()
        .zip(graph.clips())
        .map(|(reach, source)| {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                clippy::cast_precision_loss,
                reason = "a speed and samples of a few seconds"
            )]
            let (frames, budget) = {
                let speed = reach.speed().max(1.0);
                let frames = PIECE * speed.ceil() as usize;
                // The window ahead and the reach on both sides (the filter's, or a stretch's,
                // T13), in pieces, and the pieces at its ends.
                let span = ahead as f64 * speed + 2.0 * reach.radius() as f64 + 2.0;
                (frames, (span / frames as f64).ceil() as usize + 3)
            };
            Plan {
                reach: *reach,
                channels: source.channels,
                frames,
                budget,
                sent: BTreeSet::new(),
                spare: Vec::new(),
            }
        })
        .collect();
    let per = clips.iter().map(|plan| plan.budget).max().unwrap_or(1);
    let held = per * clips.len().max(1);
    // The media in the order they come, each given to a reader in turn.
    let mut media: Vec<&str> = Vec::new();
    for source in graph.clips() {
        if !media.contains(&source.media.as_str()) {
            media.push(&source.media);
        }
    }
    let readers = media.len().clamp(1, READERS);
    let reader_of: Vec<usize> = graph
        .clips()
        .iter()
        .map(|source| media.iter().position(|m| *m == source.media).unwrap_or(0) % readers)
        .collect();
    let reading = Arc::new(Reading::default());
    reading.set(from);
    let backward = clips.iter().map(|plan| plan.reach.backward()).collect();
    let mut incoming = Vec::new();
    let mut give_back = Vec::new();
    let mut fetches = Vec::new();
    let mut plans: Vec<Option<Plan>> = clips.into_iter().map(Some).collect();
    for reader in 0..readers {
        let (out, taken) = messages(IN_FLIGHT);
        let (gives, back) = messages(held + IN_FLIGHT);
        incoming.push(taken);
        give_back.push(gives);
        let own = plans
            .iter_mut()
            .zip(&reader_of)
            .map(|(plan, &of)| if of == reader { plan.take() } else { None })
            .collect();
        fetches.push(Fetch {
            out,
            back,
            reading: Arc::clone(&reading),
            clips: own,
            ahead: i64::try_from(ahead).unwrap_or(i64::MAX),
            turn: 0,
        });
    }
    let feed = RingFeed::new(
        (incoming.into_boxed_slice(), give_back.into_boxed_slice()),
        (backward, reader_of.into_boxed_slice()),
        per,
        Arc::clone(&reading),
        counters,
    )
    .with_keep(reaches.iter().map(Reach::radius).collect());
    (feed, fetches)
}

impl Fetch {
    /// The pieces given back made ready to be read into again.
    fn recycle(&mut self) {
        while let Some(chunk) = self.back.pop() {
            if let Some(Some(plan)) = self.clips.get_mut(chunk.clip) {
                plan.sent.remove(&chunk.index);
                plan.spare.push(chunk.data);
            }
        }
    }

    /// The pieces that clip `clip` needs from where the graph reads to the window ahead, nearest
    /// first, that are not sent.
    fn wanted(&self, clip: usize) -> Vec<i64> {
        let Some(plan) = &self.clips[clip] else {
            return Vec::new();
        };
        let from = self.reading.at();
        let Some(source) = plan.reach.source(from..from.saturating_add(self.ahead)) else {
            return Vec::new();
        };
        let frames = i64::try_from(plan.frames).unwrap_or(i64::MAX);
        let (first, last) = (
            source.start.div_euclid(frames),
            (source.end - 1).div_euclid(frames),
        );
        let pieces: Vec<i64> = if plan.reach.backward() {
            (first..=last).rev().collect()
        } else {
            (first..=last).collect()
        };
        pieces
            .into_iter()
            .filter(|index| !plan.sent.contains(index))
            .collect()
    }

    /// Whether every clip has all that it needs from where the graph reads to the window ahead
    /// sent, or as much as it may have out.
    #[must_use]
    pub fn primed(&mut self) -> bool {
        self.recycle();
        (0..self.clips.len()).all(|clip| {
            self.wanted(clip).is_empty()
                || self.clips[clip]
                    .as_ref()
                    .is_none_or(|plan| plan.sent.len() >= plan.budget)
        })
    }

    /// Reads at most `most` pieces that the clips want, through `feed`, a clip after another,
    /// and sends them; how many it read. A piece that cannot be read is sent silent (the feed
    /// says why where it can).
    pub fn pass(&mut self, feed: &mut dyn ClipFeed, most: usize) -> usize {
        self.recycle();
        let count = self.clips.len();
        let mut read = 0;
        let mut idle = 0;
        while read < most && idle < count && count > 0 {
            let clip = self.turn % count;
            self.turn = (self.turn + 1) % count;
            let next = match &self.clips[clip] {
                Some(plan) if plan.sent.len() < plan.budget => self.wanted(clip).first().copied(),
                _ => None,
            };
            let Some(index) = next else {
                idle += 1;
                continue;
            };
            if self.out.free() == 0 {
                break;
            }
            idle = 0;
            let Some(plan) = &mut self.clips[clip] else {
                continue;
            };
            let (frames, channels) = (plan.frames, plan.channels);
            let mut data = plan
                .spare
                .pop()
                .unwrap_or_else(|| vec![0.0; frames * channels].into_boxed_slice());
            {
                let mut slices: Vec<&mut [f32]> = data.chunks_exact_mut(frames).collect();
                let start = index * i64::try_from(frames).unwrap_or(i64::MAX);
                let span = start..start + i64::try_from(frames).unwrap_or(i64::MAX);
                if feed.read(clip, span, &mut slices).is_err() {
                    for slice in &mut slices {
                        slice.fill(0.0);
                    }
                }
            }
            let chunk = Chunk {
                clip,
                index,
                frames,
                channels,
                data,
            };
            match self.out.push(chunk) {
                Ok(()) => {
                    plan.sent.insert(index);
                    read += 1;
                }
                Err(chunk) => {
                    plan.spare.push(chunk.data);
                    break;
                }
            }
        }
        read
    }

    /// Whether the feed that it fills is gone.
    #[must_use]
    pub fn abandoned(&self) -> bool {
        self.out.is_abandoned()
    }
}

/// The threads that read ahead for a graph, one for each [`Fetch`]: they end when it is
/// dropped or when the graph's feed is gone. Dropping it tells them to end and does not wait for
/// a read under way (a media that does not answer): such a thread ends, and lets its decoders
/// go, once the read returns; [`Fetcher::live`] counts those that have not.
#[derive(Debug)]
pub struct Fetcher {
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl Fetcher {
    /// Starts reading ahead with each of `fetches` through the feed of the same place in
    /// `feeds` (as many).
    ///
    /// # Errors
    ///
    /// A thread cannot be started; those started are told to end.
    pub fn spawn(
        fetches: Vec<Fetch>,
        feeds: Vec<Box<dyn ClipFeed + Send>>,
    ) -> std::io::Result<Fetcher> {
        let mut started = Fetcher {
            stop: Arc::new(AtomicBool::new(false)),
            threads: Vec::new(),
        };
        for (mut fetch, mut feed) in fetches.into_iter().zip(feeds) {
            let stop = Arc::clone(&started.stop);
            LIVE.fetch_add(1, Ordering::AcqRel);
            let thread = std::thread::Builder::new()
                .name("columbina-sound-ahead".into())
                .spawn(move || {
                    while !stop.load(Ordering::Acquire) && !fetch.abandoned() {
                        if fetch.pass(feed.as_mut(), 4) == 0 {
                            std::thread::sleep(NAP);
                        }
                    }
                    drop(feed);
                    LIVE.fetch_sub(1, Ordering::AcqRel);
                });
            match thread {
                Ok(thread) => started.threads.push(thread),
                Err(error) => {
                    LIVE.fetch_sub(1, Ordering::AcqRel);
                    return Err(error);
                }
            }
        }
        Ok(started)
    }

    /// Whether all its threads have ended.
    #[must_use]
    pub fn ended(&self) -> bool {
        self.threads.iter().all(JoinHandle::is_finished)
    }

    /// The threads reading ahead, of every graph, that have not ended yet.
    #[must_use]
    pub fn live() -> usize {
        LIVE.load(Ordering::Acquire)
    }
}

impl Drop for Fetcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // A read under way in a media that does not answer is not waited for: the thread ends
        // once it returns.
        for thread in self.threads.drain(..) {
            if thread.is_finished() {
                let _ = thread.join();
            }
        }
    }
}
