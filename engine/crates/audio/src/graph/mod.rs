//! The graph of the sound of a timeline (plan T4b): built from a snapshot of the project
//! ([`Graph::build`]), then processed block after block ([`Graph::process`]), the same code for
//! playback (T5), export (T6) and analysis (T9).
//!
//! # What it is made of (p. 3855)
//!
//! 1. The chain of each clip: its sound read through a [`crate::feed::ClipFeed`], at its speed
//!    and direction, in the channels of its track, through its equalizer, by its gain
//!    (keyframes, fades, crossfades) and pan (`clip.rs`).
//! 2. The strip of each track: the sum of its clips → Trim and polarity → its Ducker (T11), keyed
//!    by the sum of its sources before their faders → the inserts, Dynamics and equalizer in their
//!    Order → the point before the fader → mute and solo → the fader with its VCAs → the point
//!    after the fader → its outputs and sends, each panned into the format of its bus
//!    (`strip.rs`, `route.rs`).
//! 3. The strip of each bus, the same, fed by the tracks and buses that output and send to it.
//! 4. The outputs: a bus that outputs to no bus is a main output; each main output, and the bus
//!    that is monitored, can be read after each block ([`Graph::output`]). Nothing is clipped in
//!    the graph: the sound stays in 32-bit floats to the end, and only the copy for a device or a
//!    fixed-point encoder is clipped ([`Graph::write_device`]).
//! 5. The levels before and after the fader of every track and bus ([`Graph::meters`]).
//!
//! # Order and time
//!
//! The strips run in an order where every strip comes after all that feed it and after the
//! sources of its sidechain, tracks in the order of the timeline. Every block is processed once,
//! the blocks one after the other, never the same samples twice: the output does not depend on
//! how the sound is cut into blocks. The latency of the paths is made up where they meet
//! (`pdc.rs`), and the latency left at the outputs, [`Graph::latency`], is taken off by
//! [`Graph::start`], so that sample `n` of the timeline is sample `n` of the output.
//!
//! # The thread of the mix
//!
//! [`Graph::process`], [`Graph::start`] and [`Graph::set`] neither lock, allocate, nor wait
//! (convention 3; `tests/no_alloc.rs`, `tests/rt_audit.rs`, which reads every file here but
//! `build.rs`). Everything is taken when the graph is built.

mod automation;
mod build;
mod build_fx;
mod build_retune;
mod build_warp;
mod clip;
mod pdc;
mod process;
pub mod route;
mod strip;
mod taps;
#[cfg(test)]
mod tests;

use columbina_model::Id;

pub use build::{
    audible, lanes_need_a_new_graph, node_of, BuildError, Context, Controls, Made, Maker,
};
pub use build_retune::{retune, without_clip_levels};
pub use clip::{radius, resampled, Reach};
pub use taps::Level;

/// A clip of a graph as its controls find it: its track, its id, and whether its gain and its
/// pan are fixed.
pub(crate) type ClipId = (Id, Id, bool, bool);

/// How the samples out of a graph lie on the timeline (T18): sample `u` out is sample
/// `offset + speed · u` of the timeline, so that a graph plays the timeline at a whole speed,
/// forward or backward, its clips resampled to it (their pitch following the speed), its fades,
/// keyframes and lanes read where the timeline is. The identity (speed 1, offset 0) is the graph
/// as it was built, to the bit. [`Graph::set_warp`] gives a graph its warp.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Warp {
    speed: i64,
    offset: i64,
}

impl Default for Warp {
    fn default() -> Self {
        Warp::IDENTITY
    }
}

impl Warp {
    /// Each sample out is that sample of the timeline.
    pub const IDENTITY: Warp = Warp {
        speed: 1,
        offset: 0,
    };

    /// Playing at `speed` (not 0; 1 where it is), sample `at` out being sample `timeline` of the
    /// timeline.
    #[must_use]
    pub fn new(speed: i64, at: i64, timeline: i64) -> Warp {
        let speed = if speed == 0 { 1 } else { speed };
        Warp {
            speed,
            offset: timeline.saturating_sub(speed.saturating_mul(at)),
        }
    }

    /// The same timeline at sample `at` out, played from there at `speed`: a change of speed
    /// that keeps the place.
    #[must_use]
    pub fn with_speed(&self, speed: i64, at: i64) -> Warp {
        Warp::new(speed, at, self.timeline(at))
    }

    /// The samples of the timeline that a sample out moves on by: its sign the direction.
    #[must_use]
    pub fn speed(&self) -> i64 {
        self.speed
    }

    /// Whether each sample out is that sample of the timeline.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        *self == Warp::IDENTITY
    }

    /// The sample of the timeline that sample `sample` out is.
    #[must_use]
    pub fn timeline(&self, sample: i64) -> i64 {
        self.offset
            .saturating_add(self.speed.saturating_mul(sample))
    }

    /// The samples out whose samples of the timeline lie in `range`, in the order they come out.
    #[must_use]
    pub fn span(&self, range: std::ops::Range<i64>) -> std::ops::Range<i64> {
        let (from, to, offset) = (
            i128::from(range.start),
            i128::from(range.end),
            i128::from(self.offset),
        );
        let speed = i128::from(self.speed);
        let ceil = |over: i128, under: i128| -(-over).div_euclid(under);
        let (first, end) = if speed > 0 {
            (ceil(from - offset, speed), ceil(to - offset, speed))
        } else {
            let k = -speed;
            (
                (offset - to).div_euclid(k) + 1,
                (offset - from).div_euclid(k) + 1,
            )
        };
        let clamp = |x: i128| i64::try_from(x).unwrap_or(if x < 0 { i64::MIN } else { i64::MAX });
        let (first, end) = (clamp(first), clamp(end));
        first..end.max(first)
    }

    /// The first sample out from which nothing of a timeline that ends at sample `end` is
    /// played: past its end forward, before its start (sample 0) backward.
    #[must_use]
    pub fn end_of(&self, end: i64) -> i64 {
        if self.speed > 0 {
            self.span(i64::MIN..end).end
        } else {
            self.span(0..i64::MAX).end
        }
    }
}

use crate::constants::{samples, PREROLL_MAX};
use crate::feed::ClipSource;
use crate::node::{Layout, ParamId, Planar};
use crate::rt::SeqReader;

use clip::{ClipChain, ClipScratch};
use pdc::DelayLine;
use strip::{Strip, VcaState};

/// What a graph is built for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// Playback: what is heard, with the Listen of a sidechain where it is on.
    Preview,
    /// Export: never a Listen (T8; `Listen` is only for the monitor).
    Export,
    /// Measuring offline (T9): as export.
    Analysis,
}

/// How a graph is built.
#[derive(Clone, Copy)]
pub struct Options {
    /// The most samples of a block.
    pub max_block: usize,
    /// What it is for.
    pub purpose: Purpose,
    /// What makes the node of an effect of an insert ([`node_of`]).
    pub maker: Maker,
}

impl Options {
    /// Blocks of at most `max_block`, for `purpose`, the effects made by [`node_of`].
    #[must_use]
    pub fn new(max_block: usize, purpose: Purpose) -> Self {
        Options {
            max_block,
            purpose,
            maker: node_of,
        }
    }

    /// The same, the effects made by `maker`.
    #[must_use]
    pub fn with_maker(self, maker: Maker) -> Self {
        Options { maker, ..self }
    }
}

impl std::fmt::Debug for Options {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Options")
            .field("max_block", &self.max_block)
            .field("purpose", &self.purpose)
            .finish_non_exhaustive()
    }
}

/// What a strip, a lane or a meter belongs to: a track, a bus or a VCA of the timeline, by id.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Owner {
    /// A track of sound.
    Track(Id),
    /// A bus.
    Bus(Id),
    /// A VCA.
    Vca(Id),
}

/// A setting of the graph that can be changed while it plays ([`Graph::set`]): found by its
/// owner and the path of its lane ([`Graph::control`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Control(pub(crate) u64, pub(crate) Target);

/// What a [`Control`] sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// A setting of strip `strip`.
    Strip(usize, StripParam),
    /// The fader of a VCA.
    VcaFader(usize),
    /// The mute of a VCA.
    VcaMute(usize),
    /// The fixed gain (decibels) or pan (−1 to 1) of clip `clip` (its place in the clips).
    Clip(usize, ClipParam),
}

/// A setting of a clip that a [`Control`] sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClipParam {
    Gain,
    Pan,
}

/// A setting of a strip, in the units of the model (decibels, −1 to 1, 0 to 100, switches as
/// 0 and 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StripParam {
    Trim,
    Invert,
    Fader,
    Mute,
    /// Heard (1) or silenced by a solo elsewhere (0).
    Audible,
    /// The pan of every output of the strip.
    PanX,
    PanY,
    PanDivergence,
    /// Of output or send `edge`.
    Level(usize),
    On(usize),
    EdgePanX(usize),
    EdgePanY(usize),
    /// Parameter `ParamId` of node `stage`.
    Node(usize, ParamId),
}

/// A main output, or the bus that is monitored.
#[derive(Debug)]
pub(crate) struct Out {
    pub(crate) strip: usize,
    pub(crate) bus: Id,
    pub(crate) main: bool,
    pub(crate) buffer: Planar,
    pub(crate) delay: DelayLine,
}

/// The output of a graph: a main output, or the bus that is monitored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputInfo {
    /// The bus.
    pub bus: Id,
    /// Its channels.
    pub layout: Layout,
    /// Whether it is a main output (a bus that outputs to none).
    pub main: bool,
}

/// The levels of a track or a bus: before and after its fader ([`Level`]).
#[derive(Clone, Debug)]
pub struct Meter {
    /// Whose levels.
    pub owner: Owner,
    /// Its channels.
    pub channels: usize,
    /// Before the fader.
    pub pre: SeqReader<Level>,
    /// After the fader.
    pub post: SeqReader<Level>,
}

/// The graph of the sound of a timeline. See the module.
pub struct Graph {
    /// Which graph it is, so that a control of another graph does nothing to it.
    id: u64,
    rate: u32,
    max_block: usize,
    purpose: Purpose,
    owners: Vec<Owner>,
    strips: Vec<Strip>,
    /// The order the strips run in.
    order: Vec<usize>,
    /// The clips of each strip, as a range of `clips`.
    clips_of: Vec<std::ops::Range<usize>>,
    clips: Vec<ClipChain>,
    sources: Vec<ClipSource>,
    /// The clips that read beyond what a count of samples holds, which are silent.
    unplayable: Vec<Id>,
    scratch: ClipScratch,
    /// What feeds each strip, through a block.
    inputs: Vec<Planar>,
    /// The sound before the fader of each strip that is the source of a sidechain (empty
    /// otherwise).
    pres: Vec<Planar>,
    /// Whether each strip is the source of a sidechain.
    keys: Vec<bool>,
    /// The meters of the window of each Ducker, by its track.
    duckers: Vec<(Owner, SeqReader<crate::fx::ducker::Meter>)>,
    edge: Planar,
    vca_ids: Vec<Id>,
    /// The track and the id of each clip, and whether its gain and its pan are fixed (which
    /// controls set).
    clip_ids: Vec<ClipId>,
    /// What the settings of each strip are found by.
    metas: Vec<build::Meta>,
    vcas: Vec<VcaState>,
    outputs: Vec<Out>,
    monitor: Option<usize>,
    latency: u32,
    settle: u32,
    /// The sample of the timeline of the next sample in.
    position: i64,
    /// The samples processed of the block under way.
    frames: usize,
    /// Where its samples out lie on the timeline (T18).
    warp: Warp,
}

impl std::fmt::Debug for Graph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Graph")
            .field("rate", &self.rate)
            .field("strips", &self.owners)
            .field("clips", &self.sources.len())
            .field("latency", &self.latency)
            .field("settle", &self.settle)
            .finish_non_exhaustive()
    }
}

impl Graph {
    /// Samples a second: the rate of the timeline.
    #[must_use]
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// The most samples of a block.
    #[must_use]
    pub fn max_block(&self) -> usize {
        self.max_block
    }

    /// What it was built for.
    #[must_use]
    pub fn purpose(&self) -> Purpose {
        self.purpose
    }

    /// The latency left at the outputs, which [`Graph::start`] takes off: the longest path,
    /// its delays made up.
    #[must_use]
    pub fn latency(&self) -> u32 {
        self.latency
    }

    /// How long the graph takes to settle: of every path from a clip to an output, the sum of
    /// the settling of its nodes, the longest; each node's for the slowest settings that its
    /// lanes take anywhere (T7, T8: `settle_for`). A run started this long before a sample
    /// sounds there as a run started from the beginning (T6).
    #[must_use]
    pub fn settle(&self) -> u32 {
        self.settle
    }

    /// The samples that [`Graph::start`] runs before the sample it starts at:
    /// [`Graph::settle`], but at most [`PREROLL_MAX`] (T8's review). A graph that settles
    /// longer sounds there as from the beginning but for what is left after this.
    #[must_use]
    pub fn preroll(&self) -> u32 {
        let most = u32::try_from(samples(PREROLL_MAX, self.rate)).unwrap_or(u32::MAX);
        self.settle.min(most)
    }

    /// The clips that it reads, in the order of the `clip` of [`ClipFeed::read`].
    ///
    /// [`ClipFeed::read`]: crate::feed::ClipFeed::read
    #[must_use]
    pub fn clips(&self) -> &[ClipSource] {
        &self.sources
    }

    /// The clips left silent because they read beyond the samples that a count holds (a speed
    /// and a place in their media that no `i64` reaches): the rest of the graph plays.
    #[must_use]
    pub fn unplayable(&self) -> &[Id] {
        &self.unplayable
    }

    /// The output of bus `bus`, if it is one.
    #[must_use]
    pub fn output_of(&self, bus: &str) -> Option<usize> {
        self.outputs.iter().position(|out| out.bus == bus)
    }

    /// The output of the bus that is monitored, if there is one.
    #[must_use]
    pub fn monitor(&self) -> Option<usize> {
        self.monitor
    }

    /// Output `output` of the last block processed: its first [`Graph::frames`] samples.
    #[must_use]
    pub fn output(&self, output: usize) -> &Planar {
        &self.outputs[output].buffer
    }

    /// The samples of the last block processed.
    #[must_use]
    pub fn frames(&self) -> usize {
        self.frames
    }

    /// Where its samples out lie on the timeline ([`Graph::set_warp`]).
    #[must_use]
    pub fn warp(&self) -> Warp {
        self.warp
    }

    /// The sample out that the next sample out is: the sample of the timeline, where the graph
    /// has no [`Warp`] but the identity.
    #[must_use]
    pub fn at(&self) -> i64 {
        self.position - i64::from(self.latency)
    }

    /// The sample of the timeline whose clips are read next: [`Graph::at`] and the latency
    /// ahead of it.
    #[must_use]
    pub fn reads_at(&self) -> i64 {
        self.position
    }

    /// What the window of the Ducker of track `owner` reads (T11: the track as it comes in, its
    /// sources, the reduction), if it has a Ducker in the graph.
    #[must_use]
    pub fn ducker(&self, owner: &Owner) -> Option<SeqReader<crate::fx::ducker::Meter>> {
        self.duckers
            .iter()
            .find(|(of, _)| of == owner)
            .map(|(_, meter)| meter.clone())
    }
}
