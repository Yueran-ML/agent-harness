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
use std::sync::atomic::{AtomicBool, Ordering};
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

/// The most pieces in the ring from the reader to the graph.
const IN_FLIGHT: usize = 256;

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

/// The reading ahead of the clips of a graph: the end of the rings that the reader holds.
#[derive(Debug)]
pub struct Fetch {
    out: MessageProducer<Chunk>,
    back: MessageConsumer<Chunk>,
    reading: Arc<Reading>,
    clips: Vec<Plan>,
    /// How far ahead of where the graph reads.
    ahead: i64,
    /// The clip whose turn it is first.
    turn: usize,
}

/// The feed of `graph` from what is read ahead, and the reading ahead that fills it, the graph
/// reading at `from` first. Made off the threads of the sound.
#[must_use]
pub fn ring_feed(graph: &Graph, from: i64, counters: Arc<Counters>) -> (RingFeed, Fetch) {
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
                // The window ahead and the filter's reach on both sides, in pieces, and the
                // pieces at its ends.
                let span = ahead as f64 * speed + 2.0 * 512.0 + 2.0;
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
    let (out, incoming) = messages(IN_FLIGHT);
    let (give_back, back) = messages(held + IN_FLIGHT);
    let reading = Arc::new(Reading::default());
    reading.set(from);
    let backward = clips.iter().map(|plan| plan.reach.backward()).collect();
    let feed = RingFeed::new(
        (incoming, give_back),
        backward,
        per,
        Arc::clone(&reading),
        counters,
    );
    let fetch = Fetch {
        out,
        back,
        reading,
        clips,
        ahead: i64::try_from(ahead).unwrap_or(i64::MAX),
        turn: 0,
    };
    (feed, fetch)
}

impl Fetch {
    /// The pieces given back made ready to be read into again.
    fn recycle(&mut self) {
        while let Some(chunk) = self.back.pop() {
            if let Some(plan) = self.clips.get_mut(chunk.clip) {
                plan.sent.remove(&chunk.index);
                plan.spare.push(chunk.data);
            }
        }
    }

    /// The pieces that clip `clip` needs from where the graph reads to the window ahead, nearest
    /// first, that are not sent.
    fn wanted(&self, clip: usize) -> Vec<i64> {
        let plan = &self.clips[clip];
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
            self.wanted(clip).is_empty() || self.clips[clip].sent.len() >= self.clips[clip].budget
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
            let plan = &self.clips[clip];
            let next = if plan.sent.len() < plan.budget {
                self.wanted(clip).first().copied()
            } else {
                None
            };
            let Some(index) = next else {
                idle += 1;
                continue;
            };
            if self.out.free() == 0 {
                break;
            }
            idle = 0;
            let plan = &mut self.clips[clip];
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

/// The thread that reads ahead for a graph: it ends when it is dropped (without waiting for a
/// read under way) or when the graph's feed is gone.
#[derive(Debug)]
pub struct Fetcher {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Fetcher {
    /// Starts reading ahead with `fetch` through `feed`.
    ///
    /// # Errors
    ///
    /// The thread cannot be started.
    pub fn spawn(mut fetch: Fetch, mut feed: Box<dyn ClipFeed + Send>) -> std::io::Result<Fetcher> {
        let stop = Arc::new(AtomicBool::new(false));
        let thread = std::thread::Builder::new()
            .name("columbina-sound-ahead".into())
            .spawn({
                let stop = Arc::clone(&stop);
                move || {
                    while !stop.load(Ordering::Acquire) && !fetch.abandoned() {
                        if fetch.pass(feed.as_mut(), 4) == 0 {
                            std::thread::sleep(NAP);
                        }
                    }
                }
            })?;
        Ok(Fetcher {
            stop,
            thread: Some(thread),
        })
    }

    /// Whether its thread has ended.
    #[must_use]
    pub fn ended(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }
}

impl Drop for Fetcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // A read under way in a media that does not answer is not waited for: the thread ends
        // once it returns.
        if let Some(thread) = self.thread.take() {
            if thread.is_finished() {
                let _ = thread.join();
            }
        }
    }
}
