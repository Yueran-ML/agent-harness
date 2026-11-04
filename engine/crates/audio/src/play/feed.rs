//! The sound of the clips read ahead, as the graph takes it on the thread of the mix
//! ([`RingFeed`]): pieces of each clip's media ([`Chunk`]) come through a ring from the thread
//! that reads them ([`super::fetcher`]), are kept until the graph has gone past them, and go back
//! through another ring to be filled again. What has not come yet is silence, counted
//! ([`super::Counters::underruns`]); the feed never waits, locks or takes memory.

use std::ops::Range;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use super::Counters;
use crate::feed::{ClipFeed, FeedError};
use crate::rt::{MessageConsumer, MessageProducer};

/// A piece of the sound of a clip: samples `index · frames` to `(index + 1) · frames` of its
/// media (at the rate of the timeline, as [`ClipFeed::read`] counts them), a channel after
/// another.
#[derive(Debug)]
pub struct Chunk {
    /// The clip, by its place in the graph's clips.
    pub clip: usize,
    /// Which piece.
    pub index: i64,
    /// The samples of a channel.
    pub frames: usize,
    /// The channels.
    pub channels: usize,
    /// The samples, `frames` of each channel in turn.
    pub data: Box<[f32]>,
}

impl Chunk {
    /// The first sample of the media that it holds.
    #[must_use]
    pub fn first(&self) -> i64 {
        self.index * i64::try_from(self.frames).unwrap_or(i64::MAX)
    }

    /// The sample after its last.
    #[must_use]
    pub fn end(&self) -> i64 {
        self.first() + i64::try_from(self.frames).unwrap_or(i64::MAX)
    }

    /// The samples of `channel`.
    #[must_use]
    pub fn channel(&self, channel: usize) -> &[f32] {
        &self.data[channel * self.frames..(channel + 1) * self.frames]
    }
}

/// Where the graph reads: what the reader ahead reads ahead of.
#[derive(Debug, Default)]
pub struct Reading(AtomicI64);

impl Reading {
    /// The sample of the timeline whose clips the graph reads next.
    #[must_use]
    pub fn at(&self) -> i64 {
        self.0.load(Ordering::Acquire)
    }

    /// Says that the graph reads at `sample` next.
    pub fn set(&self, sample: i64) {
        self.0.store(sample, Ordering::Release);
    }
}

/// The rings from each reader ahead of a graph, and back to each.
pub type Rings = (Box<[MessageConsumer<Chunk>]>, Box<[MessageProducer<Chunk>]>);

/// The clips' sound read ahead, as the graph takes it. See the module.
#[derive(Debug)]
pub struct RingFeed {
    /// From each reader ahead.
    incoming: Box<[MessageConsumer<Chunk>]>,
    /// Back to each.
    back: Box<[MessageProducer<Chunk>]>,
    /// The reader of each clip.
    reader_of: Box<[usize]>,
    /// The pieces held, `per` places for each clip.
    held: Box<[Option<Chunk>]>,
    per: usize,
    /// Whether each clip reads its media backwards.
    backward: Box<[bool]>,
    reading: Arc<Reading>,
    counters: Arc<Counters>,
}

impl RingFeed {
    /// The feed of `backward.len()` clips (whether each reads backwards, and which reader reads
    /// it), holding `per` pieces of each, from the rings `incoming` of the readers, giving them
    /// back through `back`. Made off the thread of the mix.
    #[must_use]
    pub fn new(
        (incoming, back): Rings,
        (backward, reader_of): (Box<[bool]>, Box<[usize]>),
        per: usize,
        reading: Arc<Reading>,
        counters: Arc<Counters>,
    ) -> Self {
        let places = backward.len() * per;
        let mut held = Vec::with_capacity(places); // rt-ok: made off the thread of the mix
        held.resize_with(places, || None);
        RingFeed {
            incoming,
            back,
            reader_of,
            held: held.into_boxed_slice(),
            per,
            backward,
            reading,
            counters,
        }
    }

    /// Says where the graph reads next, for the reader ahead.
    pub fn advance(&self, sample: i64) {
        self.reading.set(sample);
    }

    /// The pieces that have come, each put in a free place of its clip.
    fn take(&mut self) {
        for reader in 0..self.incoming.len() {
            self.take_from(reader);
        }
    }

    /// The pieces that have come from reader `reader`.
    fn take_from(&mut self, reader: usize) {
        while let Some(chunk) = self.incoming[reader].pop() {
            let places = chunk.clip * self.per..(chunk.clip + 1) * self.per;
            match self
                .held
                .get_mut(places)
                .and_then(|places| places.iter_mut().find(|place| place.is_none()))
            {
                Some(place) => *place = Some(chunk),
                None => self.give_back(chunk),
            }
        }
    }

    /// Sends `chunk` back to be read into again. The ring back holds every piece there is, so
    /// that it is never full; were it, the piece would be let go of without being freed here.
    fn give_back(&mut self, chunk: Chunk) {
        let reader = self.reader_of.get(chunk.clip).copied().unwrap_or(0);
        let refused = match self.back.get_mut(reader) {
            Some(back) => back.push(chunk).err(),
            None => Some(chunk),
        };
        if let Some(chunk) = refused {
            std::mem::forget(chunk);
        }
    }
}

impl ClipFeed for RingFeed {
    fn read(
        &mut self,
        clip: usize,
        range: Range<i64>,
        out: &mut [&mut [f32]],
    ) -> Result<(), FeedError> {
        self.take();
        for out in out.iter_mut() {
            out.fill(0.0);
        }
        let backward = self.backward.get(clip).copied().unwrap_or(false);
        let mut covered = 0_u64;
        for place in clip * self.per..(clip + 1) * self.per {
            let behind = self
                .held
                .get(place)
                .and_then(Option::as_ref)
                .is_some_and(|chunk| {
                    if backward {
                        chunk.first() >= range.end
                    } else {
                        chunk.end() <= range.start
                    }
                });
            if behind {
                if let Some(chunk) = self.held[place].take() {
                    self.give_back(chunk);
                }
                continue;
            }
            let Some(chunk) = self.held.get(place).and_then(Option::as_ref) else {
                continue;
            };
            let (from, to) = (chunk.first().max(range.start), chunk.end().min(range.end));
            if from >= to {
                continue;
            }
            let into = usize::try_from(from - range.start).unwrap_or(0);
            let at = usize::try_from(from - chunk.first()).unwrap_or(0);
            let count = usize::try_from(to - from).unwrap_or(0);
            for (channel, out) in out.iter_mut().enumerate().take(chunk.channels) {
                out[into..into + count].copy_from_slice(&chunk.channel(channel)[at..at + count]);
            }
            covered += u64::try_from(count).unwrap_or(0);
        }
        let wanted = u64::try_from(range.end - range.start).unwrap_or(0);
        if covered < wanted {
            self.counters.underrun(wanted - covered);
        }
        Ok(())
    }
}
