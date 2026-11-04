//! Playback (plan T5): the graph of a timeline run ahead of the device on a thread of its own,
//! its settings changed and new graphs swapped in while it plays, the clips' sound read ahead of
//! it, and the device fed from a ring.
//!
//! # The threads
//!
//! 1. **Reading ahead** ([`fetcher`]; not real time, one for each graph that plays): reads the
//!    sound of each clip some [`PREFETCH_AHEAD`] ahead of where the graph reads, in pieces
//!    ([`feed::Chunk`]), into a ring. Opening and decoding media, and memory, are taken here.
//! 2. **The mix** ([`engine::Engine`]; real time): runs the graph on what was read ahead
//!    ([`feed::RingFeed`]: a clip whose sound is not there yet is silent for the block, and an
//!    underrun is counted; it never waits), makes it into the sound of the device
//!    ([`crate::stream::Resampler`]), and writes it into the ring of the device, keeping
//!    [`OUTPUT_AHEAD`] (and at least [`OUTPUT_AHEAD_PERIODS`] runs of the device) in it. It takes
//!    the changes of the settings as they come, at the edge of the next block, ramped by the
//!    graph over [`crate::constants::SMOOTHING`], and swaps in a new graph at the sample that it
//!    was started at, the old and the new crossfaded over [`GRAPH_SWAP_CROSSFADE`].
//! 3. **The device** ([`device::Player`]; real time): takes what the ring holds, silence for what
//!    it does not (a run short, counted), and writes what it gave and when it is heard into a
//!    seqlock ([`device::Mark`]), the clock of playing.
//!
//! Graphs are built, started (their preroll processed) and caught up off these threads, and what
//! they let go (an old graph, its reader, a ring) goes back through rings to be dropped there:
//! nothing on the two real-time threads locks, takes or frees memory, does I/O, logs or waits
//! (convention 3; `tests/rt_audit.rs` reads `device.rs`, `engine.rs`, `feed.rs` and this file,
//! `tests/no_alloc.rs` counts a session of ten seconds). [`thread`] runs the engine and naps
//! between its steps; [`fetcher`] is the reader ahead.
//!
//! [`PREFETCH_AHEAD`]: crate::constants::PREFETCH_AHEAD
//! [`OUTPUT_AHEAD`]: crate::constants::OUTPUT_AHEAD
//! [`OUTPUT_AHEAD_PERIODS`]: crate::constants::OUTPUT_AHEAD_PERIODS
//! [`GRAPH_SWAP_CROSSFADE`]: crate::constants::GRAPH_SWAP_CROSSFADE

pub mod device;
pub mod engine;
pub mod feed;
pub mod fetcher;
pub mod thread;

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

/// What the thread of the mix counts of a session, for the thread that started it.
#[derive(Debug, Default)]
pub struct Counters {
    /// Reads of a clip whose sound was not all read ahead yet: that much of it was silent.
    underruns: AtomicU64,
    /// The samples of clips that were silent so.
    missing: AtomicU64,
    /// The sample of the timeline mixed next.
    mixed: AtomicI64,
    /// The graphs swapped in.
    swaps: AtomicU64,
    /// The graphs sent too late to be swapped in at their sample, sent back.
    late: AtomicU64,
    /// The sample at which the graph taken last to be swapped in is swapped in.
    scheduled: AtomicI64,
}

impl Counters {
    /// Reads of a clip whose sound was not all read ahead yet.
    #[must_use]
    pub fn underruns(&self) -> u64 {
        self.underruns.load(Ordering::Relaxed)
    }

    /// The samples of clips that were silent for want of being read ahead.
    #[must_use]
    pub fn missing(&self) -> u64 {
        self.missing.load(Ordering::Relaxed)
    }

    /// The sample of the timeline mixed next.
    #[must_use]
    pub fn mixed(&self) -> i64 {
        self.mixed.load(Ordering::Acquire)
    }

    /// The graphs swapped in.
    #[must_use]
    pub fn swaps(&self) -> u64 {
        self.swaps.load(Ordering::Relaxed)
    }

    /// The graphs that came too late to be swapped in at their sample.
    #[must_use]
    pub fn late(&self) -> u64 {
        self.late.load(Ordering::Relaxed)
    }

    /// The sample at which the graph taken last to be swapped in is swapped in (`i64::MIN` where
    /// none was in the session):
    /// a graph built to follow it is to be swapped in a crossfade after it at the least.
    #[must_use]
    pub fn scheduled(&self) -> i64 {
        self.scheduled.load(Ordering::Acquire)
    }

    pub(crate) fn underrun(&self, samples: u64) {
        self.underruns.fetch_add(1, Ordering::Relaxed);
        self.missing.fetch_add(samples, Ordering::Relaxed);
    }

    pub(crate) fn mix_to(&self, sample: i64) {
        self.mixed.store(sample, Ordering::Release);
    }

    pub(crate) fn swapped(&self) {
        self.swaps.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn schedule(&self, sample: i64) {
        self.scheduled.store(sample, Ordering::Release);
    }

    pub(crate) fn too_late(&self) {
        self.late.fetch_add(1, Ordering::Relaxed);
    }
}
