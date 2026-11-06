//! What the graph lets the meters and the analysis take of its sound (plan T4b, "电平接点"; T9b).
//!
//! # The meters
//!
//! Before and after the fader of each track and bus, a [`LevelTap`] of T9 (cells of 10 ms of the
//! peak and the mean square of each channel, which the meters read on another thread through a
//! [`crate::meter::LevelReader`]), and on the output of the bus that is monitored a
//! [`LoudnessTap`] (BS.1770 with the true peak, read into a [`crate::loudness::Loudness`]): what
//! the graph sends there, before anything is clipped (`Graph::write_device`), with a level tap of
//! the sum of its channels (the Control Room meter, p. 3983).
//!
//! The point before the fader is after the strip's processing and before its mute, so that a
//! track that is muted or soloed away still shows its level there (p. 3887); the point after is
//! after the mute, the solo and the fader (`after the fader level and mute button`, p. 3887).
//!
//! The taps hear only while the graph meters ([`Graph::set_metering`]): never while it prerolls
//! ([`Graph::start`] throws that sound away, and the meters must not show it), and, in playback,
//! only the graph that the thread of the mix plays (`play::engine` hands the metering from a
//! graph to the one swapped in for it, so that the cells that the readers read follow one
//! another as the sound does, whatever graphs make it). A graph is built not metering.
//!
//! # The points of measure
//!
//! The sound before and after the fader of each strip, as the last block left it
//! ([`Graph::sound_at`]), is what an analysis of a track, a bus or a clip measures (T9b): a strip
//! gives the sound of the timeline [`Graph::lead`] samples ahead of the outputs, the latency of
//! the paths after it that the outputs make up.

use crate::loudness::{loudness_tap, LoudnessReader, LoudnessTap};
use crate::meter::{level_tap, LevelReader, LevelTap};
use crate::node::{Layout, Planar};

use super::{Graph, Meter, Owner};

/// A point where the level is taken, and its first reader.
#[derive(Debug)]
pub(crate) struct Tap {
    level: LevelTap,
    pub(crate) reader: LevelReader,
    /// Whether it hears.
    pub(crate) on: bool,
}

impl Tap {
    /// A point of `channels` at `rate`, not hearing. Off the thread of the mix.
    pub(crate) fn new(rate: u32, channels: usize) -> Self {
        let (level, reader) = level_tap(rate, channels);
        Tap {
            level,
            reader,
            on: false,
        }
    }

    /// Hears the first `frames` samples of the first `channels` channels of `buffer`, if it
    /// hears. Does not allocate.
    pub(crate) fn take(&mut self, buffer: &Planar, channels: usize, frames: usize) {
        if !self.on || frames == 0 {
            return;
        }
        let level = &mut self.level;
        buffer.read(frames, |sound| {
            level.take(&sound[..channels.min(sound.len())]);
        });
    }

    /// The cell under way cleared.
    pub(crate) fn reset(&mut self) {
        self.level.reset();
    }
}

/// The loudness of the bus that is monitored, the level of the sum of its channels (the Control
/// Room meter, p. 3983), and their first readers.
#[derive(Debug)]
pub(crate) struct Loud {
    tap: LoudnessTap,
    reader: LoudnessReader,
    room: LevelTap,
    room_reader: LevelReader,
    /// The sum of the channels through a block.
    sum: Box<[f32]>,
    pub(crate) on: bool,
}

impl Loud {
    /// The loudness of a bus of `layout` at `rate`, with its true peak, and the level of the sum
    /// of its channels in blocks of at most `max_block`; not hearing. Off the thread of the mix.
    pub(crate) fn new(rate: u32, layout: Layout, max_block: usize) -> Self {
        let (tap, reader) = loudness_tap(rate, layout, true);
        let (room, room_reader) = level_tap(rate, 1);
        Loud {
            tap,
            reader,
            room,
            room_reader,
            sum: vec![0.0; max_block].into_boxed_slice(), // rt-ok: made off the thread of the mix
            on: false,
        }
    }

    /// Hears the first `frames` samples of `buffer`, if it hears. Does not allocate.
    pub(crate) fn take(&mut self, buffer: &Planar, frames: usize) {
        if !self.on || frames == 0 {
            return;
        }
        let (tap, sum) = (&mut self.tap, &mut self.sum[..frames]);
        buffer.read(frames, |sound| {
            tap.take(sound);
            sum.fill(0.0);
            for channel in sound {
                for (sum, sample) in sum.iter_mut().zip(*channel) {
                    *sum += sample;
                }
            }
        });
        self.room.take(&[&self.sum[..frames]]);
    }

    /// Its filters and the cells under way cleared.
    pub(crate) fn reset(&mut self) {
        self.tap.reset();
        self.room.reset();
    }
}

/// What the meters of a graph read: the levels of its tracks and buses, and the loudness of the
/// bus that it monitors, where it monitors one. A graph's taps are read into a
/// [`crate::metering::Board`], which outlives the graph.
#[derive(Clone, Debug)]
pub struct Taps {
    /// The levels of each track and bus, in the order of the graph.
    pub meters: Vec<Meter>,
    /// The loudness of the bus that is monitored.
    pub loudness: Option<LoudnessReader>,
    /// The level of the sum of the channels of the bus that is monitored: the Control Room meter
    /// (p. 3983), a meter of one channel.
    pub room: Option<LevelReader>,
}

/// Where a strip's sound is measured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Point {
    /// After its processing (Trim, the Ducker, the inserts, Dynamics, the equalizer), before its
    /// mute, solo and fader.
    BeforeFader,
    /// After its mute, solo, fader and VCAs.
    AfterFader,
}

impl Graph {
    /// Turns its meters on or off: the taps of the levels and of the loudness hear its sound
    /// from the next block, or hear nothing. Does not allocate (the thread of the mix hands the
    /// metering from a graph to the next).
    pub fn set_metering(&mut self, on: bool) {
        self.metering = on;
        if !on { self.fx_blocks = 0; }
        self.fx_state.write((on, self.fx_blocks));
        for strip in &mut self.strips {
            strip.pre_tap.on = on;
            strip.post_tap.on = on;
        }
        if let Some(loudness) = &mut self.loudness {
            loudness.on = on;
        }
    }

    /// Whether its meters hear its sound.
    #[must_use]
    pub fn metering(&self) -> bool {
        self.metering
    }

    /// The levels of each track and bus.
    #[must_use]
    pub fn meters(&self) -> Vec<Meter> {
        self.owners
            .iter()
            .zip(&self.strips)
            .map(|(owner, strip)| Meter {
                owner: owner.clone(),
                channels: strip.channels(),
                pre: strip.pre_tap.reader.clone(),
                post: strip.post_tap.reader.clone(),
            })
            .collect() // rt-ok: read off the thread of the mix
    }

    /// A reader of the loudness of the bus that is monitored (the loudness meter, p. 3985), if
    /// it monitors one: what goes to the device, before it is clipped.
    #[must_use]
    pub fn loudness(&self) -> Option<LoudnessReader> {
        self.loudness
            .as_ref()
            .map(|loudness| loudness.reader.clone())
    }

    /// What its meters read ([`Graph::meters`], [`Graph::loudness`], and the Control Room
    /// meter).
    #[must_use]
    pub fn taps(&self) -> Taps {
        Taps {
            meters: self.meters(),
            loudness: self.loudness(),
            room: self
                .loudness
                .as_ref()
                .map(|loudness| loudness.room_reader.clone()),
        }
    }

    /// The strip of `owner` (a track or a bus), if the graph has one.
    #[must_use]
    pub fn strip_of(&self, owner: &Owner) -> Option<usize> {
        self.owners.iter().position(|of| of == owner)
    }

    /// The channels of strip `strip`.
    ///
    /// # Panics
    ///
    /// When the graph has no such strip.
    #[must_use]
    pub fn layout_of(&self, strip: usize) -> Layout {
        self.strips[strip].layout
    }

    /// The sound at `point` of strip `strip` through the last block: its first
    /// [`Graph::frames`] samples, of the timeline [`Graph::lead`] samples after the outputs'.
    ///
    /// # Panics
    ///
    /// When the graph has no such strip.
    #[must_use]
    pub fn sound_at(&self, strip: usize, point: Point) -> &Planar {
        let strip = &self.strips[strip];
        match point {
            Point::BeforeFader => &strip.work,
            Point::AfterFader => &strip.post,
        }
    }

    /// How many samples of the timeline the sound of strip `strip` is ahead of the outputs: the
    /// latency of the paths after it, which the outputs make up ([`Graph::latency`] less the
    /// latency of the paths that come to it). A strip is measured over a range of the timeline
    /// by a graph started this much before it.
    ///
    /// # Panics
    ///
    /// When the graph has no such strip.
    #[must_use]
    pub fn strip_lead(&self, strip: usize) -> u32 {
        self.latency.saturating_sub(self.exits[strip])
    }
}
