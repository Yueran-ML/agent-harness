//! The chain of a clip (plan T4b, p. 3855): its sound read from its media ([`ClipFeed`]), at its
//! speed and direction (a rational mapping and a windowed sinc), in the channels of its track,
//! through its equalizer and its effects, by its gain (keyframes, fades and crossfades), and
//! panned, added to the sum of its track.
//!
//! The effects of a clip ([`ClipEffects`], T4c) delay what goes through them by their latency.
//! The clip makes it up itself: everything of it up to its effects runs that far ahead of the
//! timeline (its media read as far ahead), so that what leaves them is its sound on its own
//! samples, and the chain has no latency, as a stretched one has none. Its effects take its sound
//! from where it starts and silence after it ends, until what is in them has come out; what they
//! give outside the clip is not heard, its gain being none there.
//!
//! The mapping and the resampling are those of the mixer of phases 1 and 2 (`server/src/play/
//! mix.rs`), moved here unchanged, so that a project of before sounds as it did, to the bit.
//!
//! A clip whose change of speed keeps its pitch (Pitch Correction, p. 3827), or whose pitch is
//! moved (p. 3826), is stretched instead ([`Stretched`], T13): with Pitch Correction its source is
//! stretched to the time of the timeline by Signalsmith Stretch, the pitch moved besides; with a
//! pitch moved alone, the resampled sound is moved by it at a speed of 1. Its stretchers start a
//! little before it sounds and read its media ahead, so that the chain has no latency; they are
//! taken from a pool that the clips of a graph share ([`Pool`]) for as long as each clip sounds.

use std::ops::Range;

use columbina_model::resolve::AudioPiece;
use columbina_model::{Clip, Timeline};

use crate::constants::MAX_CHANNELS;
use crate::feed::{ClipFeed, FeedError};
use crate::fx::common::finite;
use crate::fx::eq::Eq;
use crate::fx::stretch::{Input, Plan, Stretcher, Transpose};
use crate::node::{Block, Node, Planar};
use crate::rt::{Domain, Smoothed};

use super::route::balance;
use super::Warp;

/// Absolute rational positions keep the fractional phase of a speed or a trim across blocks: the
/// sample of the source, at the rate of the timeline, that a sample of the timeline reads, and
/// how far past it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Mapping {
    start: i128,
    step: i128,
    denominator: i128,
}

impl Mapping {
    /// The mapping of `clip`, cut from media, on `timeline`.
    pub(crate) fn new(clip: &Clip, timeline: &Timeline) -> Option<Self> {
        let source = clip.source_in()?;
        let sample_at = |frame: i64| {
            i128::from(frame) * i128::from(timeline.sample_rate) * i128::from(timeline.rate.den)
                / i128::from(timeline.rate.num)
        };
        let direction = if clip.reverse { -1 } else { 1 };
        let origin = if clip.reverse {
            sample_at(clip.end()) - 1
        } else {
            -sample_at(clip.start)
        };
        let step = i128::from(clip.speed.num) * i128::from(source.scale);
        Some(Self {
            start: i128::from(source.ticks)
                * i128::from(timeline.sample_rate)
                * i128::from(clip.speed.den)
                + origin * step,
            step: step * direction,
            denominator: i128::from(source.scale) * i128::from(clip.speed.den),
        })
    }

    /// The sample of the source that `sample` reads, and the share of the way to the next, if
    /// it is a sample that an `i64` holds.
    #[allow(
        clippy::cast_precision_loss,
        reason = "only a rational fraction in [0, 1) is converted to f64"
    )]
    pub(crate) fn checked_at(&self, sample: i64) -> Option<(i64, f64)> {
        let numerator = self.start + i128::from(sample) * self.step;
        let whole = numerator.div_euclid(self.denominator);
        let fraction = numerator.rem_euclid(self.denominator) as f64 / self.denominator as f64;
        Some((i64::try_from(whole).ok()?, fraction))
    }

    /// [`Mapping::checked_at`] for a sample of the clip, which the graph checked when it was
    /// built.
    pub(crate) fn at(&self, sample: i64) -> (i64, f64) {
        self.checked_at(sample).unwrap_or((0, 0.0))
    }

    /// The samples of the source played in some samples of the timeline, and those samples:
    /// the speed as a ratio, its sign the direction.
    pub(crate) fn speed(&self) -> (i128, i128) {
        (self.step, self.denominator)
    }

    /// The mapping of the samples out of a graph played through `warp` (T18): sample `u` out
    /// reads what sample `warp.timeline(u)` of the timeline reads, exactly (the warp is a whole
    /// line, so that the composition is rational still).
    pub(crate) fn warped(&self, warp: Warp) -> Self {
        let (speed, offset) = (i128::from(warp.speed()), i128::from(warp.timeline(0)));
        Self {
            start: self.start + offset * self.step,
            step: self.step * speed,
            denominator: self.denominator,
        }
    }
}

/// How many zero crossings of the sinc lie on each side of its centre, at full bandwidth.
const CROSSINGS: i64 = 16;

/// The most source samples on each side of the centre that one output sample reads: 32 times
/// the reach at full bandwidth, which keeps 16 zero crossings up to a speed of 32.
pub(crate) const LONGEST: i64 = CROSSINGS * 32;

/// How many source samples on each side of the centre the filter reads at `cutoff` (the new
/// band as a part of the old). The sinc is stretched as the band narrows, so that a speed-up
/// keeps as many zero crossings, and with them as deep a stopband, as playing at speed 1 does:
/// a 32-tap filter at 16 times would have barely two lobes and let much of what is above the
/// new Nyquist frequency fold back into what is heard.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "the reach is at most LONGEST"
)]
#[must_use]
pub fn radius(cutoff: f64) -> i64 {
    ((CROSSINGS as f64 / cutoff).ceil() as i64).clamp(CROSSINGS, LONGEST)
}

/// How many source samples on each side of the centre the filter of a clip read while the
/// timeline is shuttled (played at more than [`crate::constants::SHUTTLE_FROM`] times, T18)
/// reaches at `cutoff`: [`crate::constants::SHUTTLE_CROSSINGS`] zero crossings in place of 16,
/// so that a shuttle of sixteen tracks fits in the time of the mix.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "the reach is at most LONGEST"
)]
#[must_use]
pub fn shuttle_radius(cutoff: f64) -> i64 {
    let crossings = crate::constants::SHUTTLE_CROSSINGS;
    ((crossings as f64 / cutoff).ceil() as i64).clamp(crossings, LONGEST)
}

/// The cutoff of a clip played at `speed` (its numbers): the new band as a part of the old.
pub(crate) fn cutoff(speed: (u32, u32)) -> f64 {
    (f64::from(speed.1) / f64::from(speed.0)).min(1.0)
}

/// Windowed sinc resampling low-passes a speed-up before decimation: sample `index` of each of
/// `channels`, `phase` of the way to the next, at `cutoff`, into `out`. Unity speed at an integer
/// position is exact, so the ordinary playback path does no filtering at all. The sine and cosine
/// of each tap are stepped from the last by a rotation, which a filter of a thousand taps needs
/// to be cheap. Each channel is summed as the stereo mixer of phase 2 summed each of its two.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    reason = "normalized audio samples are f32; filter taps are at most LONGEST from the centre; \
              the exact unity-rate, integer-phase case must preserve PCM samples"
)]
pub fn resampled(channels: &[&[f32]], index: usize, phase: f64, cutoff: f64, out: &mut [f32]) {
    resampled_with(channels, index, phase, (cutoff, radius(cutoff)), out);
}

/// [`resampled`] with a filter that reaches `radius` source samples on each side of its centre
/// (T18: a shorter one for shuttling, [`shuttle_radius`]).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    reason = "normalized audio samples are f32; filter taps are at most LONGEST from the centre;               the exact unity-rate, integer-phase case must preserve PCM samples"
)]
pub(crate) fn resampled_with(
    channels: &[&[f32]],
    index: usize,
    phase: f64,
    (cutoff, radius): (f64, i64),
    out: &mut [f32],
) {
    use std::f64::consts::PI;
    if phase == 0.0 && cutoff == 1.0 {
        for (out, channel) in out.iter_mut().zip(channels) {
            *out = channel[index];
        }
        return;
    }
    let reach = radius as f64;
    let first = 1 - radius;
    let distance = first as f64 - phase;
    // The angle of the sinc and of the window at the first tap, each stepped by a rotation.
    let (step_sin, step_cos) = (PI * cutoff).sin_cos();
    let (mut sin, mut cos) = (PI * cutoff * distance).sin_cos();
    let (window_step_sin, window_step_cos) = (PI / reach).sin_cos();
    let (mut window_sin, mut window_cos) = (PI * distance / reach).sin_cos();
    let mut sum = [0.0_f64; MAX_CHANNELS];
    let mut weights = 0.0;
    for tap in first..=radius {
        let angle = PI * cutoff * (tap as f64 - phase);
        let sinc = if angle.abs() < 1e-12 {
            1.0
        } else {
            sin / angle
        };
        let weight = sinc * 0.5_f64.mul_add(window_cos, 0.5);
        let at = index.wrapping_add_signed(tap as isize);
        for (sum, channel) in sum.iter_mut().zip(channels) {
            *sum += f64::from(channel[at]) * weight;
        }
        weights += weight;
        (sin, cos) = (
            sin.mul_add(step_cos, cos * step_sin),
            cos.mul_add(step_cos, -sin * step_sin),
        );
        (window_sin, window_cos) = (
            window_sin.mul_add(window_step_cos, window_cos * window_step_sin),
            window_cos.mul_add(window_step_cos, -window_sin * window_step_sin),
        );
    }
    for (out, sum) in out.iter_mut().zip(sum) {
        *out = (sum / weights) as f32;
    }
}

/// The pan of a clip: none (on a track of other than two channels), one, or keyframes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ClipPan {
    /// No pan: the channels as they are.
    None,
    /// The gains of a fixed pan, a balance ([`balance`]).
    Fixed([f32; 2]),
    /// Keyframes, read at each sample.
    Keyed,
}

/// The order of the channels of 5.1 in Film order (L C R Ls Rs LFE) as SMPTE has them (L R C LFE
/// Ls Rs): the channel of SMPTE that each of Film's is (C-F8).
pub(crate) const FILM_TO_SMPTE: [usize; 6] = [0, 2, 1, 4, 5, 3];

/// The sound read from the media of the clips, the clips one after the other: the source window
/// of a block, a channel after another.
#[derive(Debug)]
pub(crate) struct Window {
    pub(crate) data: Box<[f32]>,
    /// The samples of each channel.
    pub(crate) capacity: usize,
}

impl Window {
    /// The first `frames` samples of the first `channels` channels, to write, the channel of
    /// SMPTE that each of Film's is put first where `film` (the feed gives them in the order of
    /// the track).
    fn slices_mut(
        &mut self,
        channels: usize,
        frames: usize,
        film: bool,
    ) -> [&mut [f32]; MAX_CHANNELS] {
        let mut slices: [&mut [f32]; MAX_CHANNELS] =
            std::array::from_fn(|_| <&mut [f32]>::default());
        for (slice, channel) in slices
            .iter_mut()
            .zip(self.data.chunks_exact_mut(self.capacity))
            .take(channels)
        {
            *slice = &mut channel[..frames];
        }
        if film && channels == FILM_TO_SMPTE.len() {
            let mut ordered: [&mut [f32]; MAX_CHANNELS] =
                std::array::from_fn(|_| <&mut [f32]>::default());
            for (film, &smpte) in FILM_TO_SMPTE.iter().enumerate() {
                ordered[film] = std::mem::take(&mut slices[smpte]);
            }
            return ordered;
        }
        slices
    }

    /// The first `frames` samples of the first `channels` channels.
    fn slices(&self, channels: usize, frames: usize) -> [&[f32]; MAX_CHANNELS] {
        let mut slices: [&[f32]; MAX_CHANNELS] = [&[]; MAX_CHANNELS];
        for (slice, channel) in slices
            .iter_mut()
            .zip(self.data.chunks_exact(self.capacity))
            .take(channels)
        {
            *slice = &channel[..frames];
        }
        slices
    }
}

/// What the clips share: the window of the source, the sound of a clip before and after its
/// equalizer, and the stretchers of the clips that are stretched, a pool for each count of
/// channels.
#[derive(Debug)]
pub(crate) struct ClipScratch {
    pub(crate) window: Window,
    pub(crate) before: Planar,
    pub(crate) after: Planar,
    pub(crate) pools: Vec<Pool>,
}

impl ClipScratch {
    /// Every stretcher free again (a reset of the graph).
    pub(crate) fn reset(&mut self) {
        for pool in &mut self.pools {
            pool.reset();
        }
    }
}

/// The stretchers of the clips of a count of channels: as many as the most of them that are
/// stretched at once (a block either side), each held by a clip from a little before it sounds
/// to its end.
#[derive(Debug)]
pub(crate) struct Pool {
    pub(crate) stretchers: Vec<Stretcher>,
    /// Those not held, by their places; made as long as `stretchers`, so that it never grows.
    pub(crate) free: Vec<usize>,
}

impl Pool {
    /// Every stretcher free.
    pub(crate) fn reset(&mut self) {
        for stretcher in &mut self.stretchers {
            stretcher.release();
        }
        self.free.clear();
        for at in (0..self.stretchers.len()).rev() {
            self.free.push(at);
        }
    }
}

/// How a clip is stretched (T13).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StretchMode {
    /// Pitch Correction: its source is stretched to the timeline, its pitch kept (and moved by
    /// its pitch).
    Source,
    /// Its pitch moved alone: its sound, resampled to its speed as before, moved at a speed of 1.
    Resampled,
}

/// The stretch of a clip (T13): how it runs, and the stretcher it holds while it sounds.
#[derive(Debug)]
pub(crate) struct Stretched {
    pub(crate) mode: StretchMode,
    pub(crate) plan: Plan,
    /// The factor that its pitch is moved by.
    pub(crate) transpose: f32,
    /// The latencies of the library, input and output.
    pub(crate) latencies: (i64, i64),
    /// Where its first stretcher starts: a warming before it sounds.
    pub(crate) busy_from: i64,
    /// How far from the sample of the source that a sample of the timeline maps to its stretch
    /// reads, either way, while it plays on: what a reader ahead must have (T5).
    pub(crate) reach: i64,
    /// Its pool, by its place in [`ClipScratch::pools`].
    pub(crate) pool: usize,
    /// The stretcher it holds, by its place in its pool.
    pub(crate) held: Option<usize>,
}

/// What a clip's stretch is fed: its media through the feed, in the direction it plays (Pitch
/// Correction), or its sound resampled to its speed (a pitch moved alone).
struct ClipInput<'a> {
    feed: &'a mut dyn ClipFeed,
    window: &'a mut Window,
    index: usize,
    channels: usize,
    film: bool,
    mapping: Mapping,
    mode: StretchMode,
    latencies: (i64, i64),
    cutoff: f64,
    radius: i64,
    max_block: usize,
}

/// Silence in `into` from `offset`, `length` long.
fn silence(into: &mut [&mut [f32]], offset: usize, length: usize) {
    for channel in into.iter_mut() {
        channel[offset..offset + length].fill(0.0);
    }
}

impl ClipInput<'_> {
    /// Reads `length` samples of the source from position `x` in the direction it plays into
    /// `into` from `done`.
    fn source(
        &mut self,
        x: i64,
        length: usize,
        done: usize,
        into: &mut [&mut [f32]],
    ) -> Result<(), FeedError> {
        let channels = self.channels.min(into.len());
        let backward = self.mapping.step < 0;
        let last = x + i64::try_from(length).unwrap_or(0);
        let source = if backward { 1 - last..1 - x } else { x..last };
        {
            let mut slices = self.window.slices_mut(channels, length, self.film);
            if let Err(error) = self.feed.read(self.index, source, &mut slices[..channels]) {
                silence(into, done, length);
                return Err(error);
            }
        }
        let window = self.window.slices(channels, length);
        for (into, window) in into.iter_mut().zip(&window[..channels]) {
            let into = &mut into[done..done + length];
            if backward {
                for (sample, value) in into.iter_mut().zip(window.iter().rev()) {
                    *sample = sanitized(*value);
                }
            } else {
                for (sample, value) in into.iter_mut().zip(window.iter()) {
                    *sample = sanitized(*value);
                }
            }
        }
        Ok(())
    }

    /// Writes the clip's sound resampled to its speed at the samples `piece` of the timeline
    /// (at most a block) into `into` from `done`.
    fn resampled(
        &mut self,
        piece: Range<i64>,
        done: usize,
        into: &mut [&mut [f32]],
    ) -> Result<(), FeedError> {
        let channels = self.channels.min(into.len());
        let length = usize::try_from(piece.end - piece.start).unwrap_or(0);
        let source = source_of(&self.mapping, self.radius, &piece);
        let span = usize::try_from(source.end - source.start).unwrap_or(usize::MAX);
        if span > self.window.capacity {
            silence(into, done, length);
            return Err(FeedError { clip: self.index });
        }
        {
            let mut slices = self.window.slices_mut(channels, span, self.film);
            if let Err(error) = self
                .feed
                .read(self.index, source.clone(), &mut slices[..channels])
            {
                silence(into, done, length);
                return Err(error);
            }
        }
        let window = self.window.slices(channels, span);
        let mut values = [0.0_f32; MAX_CHANNELS];
        for (n, sample) in piece.enumerate() {
            let (at, phase) = self.mapping.at(sample);
            let at = usize::try_from(at - source.start).unwrap_or(0);
            resampled_with(
                &window[..channels],
                at,
                phase,
                (self.cutoff, self.radius),
                &mut values[..channels],
            );
            for (into, value) in into.iter_mut().zip(&values[..channels]) {
                into[done + n] = sanitized(*value);
            }
        }
        Ok(())
    }
}

impl Input for ClipInput<'_> {
    fn cursor(&self, v: i64) -> i64 {
        let (input, output) = self.latencies;
        match self.mode {
            StretchMode::Source => {
                let direction = if self.mapping.step < 0 { -1 } else { 1 };
                direction * self.mapping.at(v.saturating_add(output)).0 + input
            }
            StretchMode::Resampled => v.saturating_add(output).saturating_add(input),
        }
    }

    fn fill(&mut self, range: Range<i64>, into: &mut [&mut [f32]]) -> Result<(), FeedError> {
        let mut result = Ok(());
        let total = usize::try_from(range.end - range.start).unwrap_or(0);
        let piece = match self.mode {
            StretchMode::Source => self.window.capacity,
            StretchMode::Resampled => self.max_block,
        }
        .max(1);
        let mut done = 0usize;
        while done < total {
            let length = (total - done).min(piece);
            let x = range.start + i64::try_from(done).unwrap_or(0);
            let read = match self.mode {
                StretchMode::Source => self.source(x, length, done, into),
                StretchMode::Resampled => {
                    let end = x + i64::try_from(length).unwrap_or(0);
                    self.resampled(x..end, done, into)
                }
            };
            keep(&mut result, read);
            done += length;
        }
        result
    }
}

/// A sample as a stretch takes it: 0 when it is not a number, which would stay in its phases.
#[allow(clippy::cast_possible_truncation, reason = "a sample")]
fn sanitized(value: f32) -> f32 {
    finite(value) as f32
}

/// The samples of the source that the samples `range` of the timeline read through `mapping`,
/// with `radius` on each side.
fn source_of(mapping: &Mapping, radius: i64, range: &Range<i64>) -> Range<i64> {
    let (first, _) = mapping.at(range.start);
    let (last, _) = mapping.at(range.end - 1);
    first.min(last) - radius..first.max(last) + radius + 1
}

/// Keeps the first error of `result` and `next`.
fn keep(result: &mut Result<(), FeedError>, next: Result<(), FeedError>) {
    if let (Ok(()), Err(error)) = (&*result, next) {
        *result = Err(error);
    }
}

/// Where a clip of a graph reads its media ([`crate::graph::Graph::reaches`]): the samples of the
/// timeline it sounds on, and the samples of its media (at the rate of the timeline, as
/// [`ClipFeed::read`] counts them) that any of them reads, the filter's reach and all. The
/// threads that read clips ahead of the graph (T5) read what it will ask for.
#[derive(Clone, Copy, Debug)]
pub struct Reach {
    /// The samples of the clip's sound that it reads its media for, from its warming.
    from: i64,
    to: i64,
    mapping: Mapping,
    radius: i64,
    /// How far ahead of the timeline it works out its sound: the latency of its effects (T4c).
    ahead: i64,
}

impl Reach {
    /// The same on samples out that are `shift` behind the timeline (a graph played at a speed
    /// of 1 from elsewhere, T18): what sample `u` out reads is what sample `u + shift` reads.
    #[must_use]
    pub(crate) fn shifted(self, shift: i64) -> Self {
        if shift == 0 {
            return self;
        }
        Reach {
            from: self.from.saturating_sub(shift),
            to: self.to.saturating_sub(shift),
            mapping: self.mapping.warped(Warp::new(1, 0, shift)),
            radius: self.radius,
            ahead: self.ahead,
        }
    }

    /// The samples of the timeline at which the graph reads the clip's media: where it sounds
    /// (and its stretch warms before), as far earlier as its effects are late.
    #[must_use]
    pub fn span(&self) -> Range<i64> {
        self.from - self.ahead..self.to - self.ahead
    }

    /// The samples of the media that the graph reads for the clip in the samples `timeline` of
    /// the timeline, where it reads any.
    #[must_use]
    pub fn source(&self, timeline: Range<i64>) -> Option<Range<i64>> {
        let timeline =
            timeline.start.saturating_add(self.ahead)..timeline.end.saturating_add(self.ahead);
        let (from, to) = (self.from.max(timeline.start), self.to.min(timeline.end));
        if from >= to {
            return None;
        }
        let (first, _) = self.mapping.at(from);
        let (last, _) = self.mapping.at(to - 1);
        Some(first.min(last) - self.radius..first.max(last) + self.radius + 1)
    }

    /// How far either way from the sample of the media that a sample of the timeline maps to
    /// it reads: the filter's reach, or a stretch's (T13), which also reads what it read before
    /// again (the sound before a stretcher's start).
    #[must_use]
    pub fn radius(&self) -> i64 {
        self.radius
    }

    /// Whether it reads its media backwards (a clip played in reverse).
    #[must_use]
    pub fn backward(&self) -> bool {
        self.mapping.step < 0
    }

    /// The samples of its media that a sample of the timeline moves on by: its speed.
    #[must_use]
    #[allow(clippy::cast_precision_loss, reason = "a speed")]
    pub fn speed(&self) -> f64 {
        self.mapping.step.unsigned_abs() as f64 / self.mapping.denominator as f64
    }
}

/// The fixed gain and pan of a clip, changed as it plays (T5's review: a clip's volume line or
/// pan dragged is a setting, not another graph): a multiplier of its gain from what it was built
/// with, and the two gains of its balance, each ramped over 10 ms. Until either is set, the
/// clip sounds as it was built, to the bit.
#[derive(Debug)]
pub(crate) struct ClipLevel {
    /// The fixed gain it was built with, in decibels; none for keyframes.
    pub(crate) gain_db: Option<f32>,
    pub(crate) scale: Smoothed,
    pub(crate) scaled: bool,
    /// Whether its pan is fixed (on a stereo track).
    pub(crate) fixed_pan: bool,
    pub(crate) sides: [Smoothed; 2],
    pub(crate) panned: bool,
}

impl ClipLevel {
    /// The level of a clip of a fixed gain `gain_db` (or none) and pan `pan` (or none), at
    /// `rate`. Made off the thread of the mix.
    pub(crate) fn new(gain_db: Option<f32>, pan: Option<[f32; 2]>, rate: u32) -> Self {
        let smoothed = |value: f32| {
            let mut smoothed = Smoothed::new(value, Domain::Linear);
            smoothed.prepare(rate);
            smoothed
        };
        let sides = pan.unwrap_or([1.0, 1.0]);
        ClipLevel {
            gain_db,
            scale: smoothed(1.0),
            scaled: false,
            fixed_pan: pan.is_some(),
            sides: [smoothed(sides[0]), smoothed(sides[1])],
            panned: false,
        }
    }
}

/// An effect of a clip in the graph: its node, and the latency of those before it in the
/// clip's chain.
pub(crate) struct ClipStage {
    pub(crate) node: Box<dyn Node>,
    pub(crate) offset: i64,
}

/// The effects of a clip (p. 3855, the interaction reference §10.1; T4c), in their order after
/// its equalizer: made as the inserts of a strip are, but with no lanes, no sidechain, and
/// Listen only in a preview.
pub(crate) struct ClipEffects {
    pub(crate) stages: Box<[ClipStage]>,
    /// Their latency, all of them: how far ahead of the timeline the clip works out its sound.
    pub(crate) latency: i64,
    /// The samples they have processed since they were reset ([`Block::start`]).
    pub(crate) processed: u64,
    /// Whether they have processed nothing since they were reset: each is lined up with the
    /// timeline ([`Node::align`]) where they start, at the clip's start or where a run starts.
    pub(crate) fresh: bool,
}

impl ClipEffects {
    /// Runs the first `count` samples of [`ClipScratch::after`], the clip's sound from the sample
    /// `first` of the timeline, through the effects of the `channels` of its track, leaving what
    /// leaves them there.
    fn run(
        &mut self,
        first: i64,
        count: usize,
        rate: u32,
        channels: usize,
        scratch: &mut ClipScratch,
    ) {
        if self.fresh {
            for stage in &mut self.stages {
                // Two's complement keeps the grid of a run that starts before the timeline does.
                stage
                    .node
                    .align(first.wrapping_sub(stage.offset).cast_unsigned());
            }
            self.fresh = false;
        }
        let block = Block::new(self.processed, rate, count);
        let ClipScratch { before, after, .. } = scratch;
        for stage in &mut self.stages {
            after.read(count, |input| {
                before.write(count, |output| {
                    stage
                        .node
                        .process(&block, &input[..channels], &mut output[..channels]);
                });
            });
            std::mem::swap(before, after);
        }
        self.processed += u64::try_from(count).unwrap_or(0);
    }

    /// Back to silence, to be lined up again where they next start.
    fn reset(&mut self) {
        for stage in &mut self.stages {
            stage.node.reset();
        }
        self.processed = 0;
        self.fresh = true;
    }
}

/// A clip in the graph.
pub(crate) struct ClipChain {
    /// The strip of its track.
    pub(crate) strip: usize,
    /// The samples of the timeline that it sounds on, crossfades and all.
    pub(crate) from: i64,
    pub(crate) to: i64,
    pub(crate) mapping: Mapping,
    pub(crate) cutoff: f64,
    pub(crate) radius: i64,
    /// Its gain and its keyframes of pan, as the model resolves them.
    pub(crate) piece: AudioPiece,
    pub(crate) pan: ClipPan,
    /// The channels of its track.
    pub(crate) channels: usize,
    /// Whether the feed gives its channels in the Film order of a 5.1 track.
    pub(crate) film: bool,
    /// Its equalizer, where it is on.
    pub(crate) eq: Option<Eq>,
    /// The samples that its equalizer has processed since it was reset.
    pub(crate) processed: u64,
    /// Its fixed gain and pan, as they are set while it plays.
    pub(crate) level: ClipLevel,
    /// Its stretch, where its speed keeps its pitch or its pitch is moved (T13).
    pub(crate) stretch: Option<Stretched>,
    /// Its effects, where it has any that work on the channels of its track (T4c).
    pub(crate) effects: Option<ClipEffects>,
    /// How long its equalizer and effects take to settle, their latency besides (T4c's review).
    pub(crate) settle: u64,
    /// How it lies on the samples out of a graph played at a speed (T18).
    pub(crate) warped: Warped,
}

/// The samples of the timeline a clip sounds on and how it reads its media, as it was built.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Base {
    from: i64,
    to: i64,
    mapping: Mapping,
    cutoff: f64,
    radius: i64,
}

/// How a clip lies on the samples out of a graph played through a [`Warp`] (T18): the warp, the
/// clip as it was built, and its stretch, put aside while it plays at another speed than 1 (the
/// speed of playing changes its pitch: Pitch Correction, and its own pitch, are not applied).
#[derive(Debug, Default)]
pub(crate) struct Warped {
    pub(crate) warp: Warp,
    base: Option<Base>,
    stretch: Option<Stretched>,
}

impl ClipChain {
    /// How far ahead of the timeline it works out its sound: the latency of its effects.
    pub(crate) fn ahead(&self) -> i64 {
        self.effects.as_ref().map_or(0, |effects| effects.latency)
    }

    /// The first sample of its sound for which it reads its media: where it sounds, or where its
    /// stretch warms before that.
    fn busy_at(&self) -> i64 {
        self.stretch
            .as_ref()
            .map_or(self.from, |stretch| stretch.busy_from.min(self.from))
    }

    /// The samples of the source that the samples `range` of the timeline read, with the reach
    /// of the filter on each side.
    pub(crate) fn source(&self, range: &Range<i64>) -> Range<i64> {
        source_of(&self.mapping, self.radius, range)
    }

    /// How far either way from the sample of the source that a sample of the timeline maps to
    /// the chain reads: the filter's reach, or its stretch's (T13), which a reader ahead of the
    /// graph must cover (T5's `Reach`). A stretched clip also reads before it sounds, from
    /// [`ClipChain::busy_from`].
    pub(crate) fn reads_within(&self) -> i64 {
        self.stretch
            .as_ref()
            .map_or(self.radius, |stretch| stretch.reach.max(self.radius))
    }

    /// The first sample of the timeline at which the chain reads its media: where it sounds, or
    /// where its stretch warms before that, as far earlier as its effects are late.
    pub(crate) fn busy_from(&self) -> i64 {
        self.busy_at() - self.ahead()
    }

    /// Where it reads its media, for the readers ahead of the graph (T5): from where it is busy,
    /// as far either way as it reads, as far ahead as its effects are late.
    pub(crate) fn reach(&self) -> Reach {
        Reach {
            from: self.busy_at(),
            to: self.to,
            mapping: self.mapping,
            radius: self.reads_within(),
            ahead: self.ahead(),
        }
    }

    /// Adds what the clip sounds on the `frames` samples of the timeline from `start` to `into`
    /// (the sum of its track), reading its media through `feed` as clip `index`.
    pub(crate) fn mix(
        &mut self,
        index: usize,
        feed: &mut dyn ClipFeed,
        (start, frames, rate): (i64, usize, u32),
        scratch: &mut ClipScratch,
        into: &mut Planar,
    ) -> Result<(), FeedError> {
        if self.effects.is_some() {
            return self.mix_through_effects(index, feed, (start, frames, rate), scratch, into);
        }
        if self.stretch.is_some() {
            return self.mix_stretched(index, feed, (start, frames, rate), scratch, into);
        }
        let end = start.saturating_add(i64::try_from(frames).unwrap_or(i64::MAX));
        let (from, to) = (self.from.max(start), self.to.min(end));
        if from >= to {
            return Ok(());
        }
        let source = self.source(&(from..to));
        let length = usize::try_from(source.end - source.start).unwrap_or(usize::MAX);
        let channels = self.channels;
        if length > scratch.window.capacity {
            return Err(FeedError { clip: index });
        }
        {
            let mut slices = scratch.window.slices_mut(channels, length, self.film);
            feed.read(index, source.clone(), &mut slices[..channels])?;
        }
        let window = scratch.window.slices(channels, length);
        let window = &window[..channels];
        let offset = |sample: i64| usize::try_from(sample - start).unwrap_or(0);
        let mut values = [0.0_f32; MAX_CHANNELS];
        let values = &mut values[..channels];
        if let Some(eq) = &mut self.eq {
            let count = usize::try_from(to - from).unwrap_or(0);
            for sample in from..to {
                let (at, phase) = self.mapping.at(sample);
                let at = usize::try_from(at - source.start).unwrap_or(0);
                resampled_with(window, at, phase, (self.cutoff, self.radius), values);
                let n = usize::try_from(sample - from).unwrap_or(0);
                for (channel, value) in values.iter().enumerate() {
                    scratch.before.channel_mut(channel)[n] = *value;
                }
            }
            let block = Block::new(self.processed, rate, count);
            scratch.before.read(count, |input| {
                scratch.after.write(count, |output| {
                    eq.process(&block, &input[..channels], &mut output[..channels]);
                });
            });
            self.processed += u64::try_from(count).unwrap_or(0);
            for sample in from..to {
                let n = usize::try_from(sample - from).unwrap_or(0);
                for (channel, value) in values.iter_mut().enumerate() {
                    *value = scratch.after.channel(channel)[n];
                }
                self.add(sample, values, into, offset(sample));
            }
        } else {
            for sample in from..to {
                let (at, phase) = self.mapping.at(sample);
                let at = usize::try_from(at - source.start).unwrap_or(0);
                resampled_with(window, at, phase, (self.cutoff, self.radius), values);
                self.add(sample, values, into, offset(sample));
            }
        }
        Ok(())
    }

    /// Adds `values`, the sound of the clip at `sample` of the timeline, by its gain and pan,
    /// to sample `at` of `into`: `value · gain · pan`, as the mixer of phase 2 did.
    fn add(&mut self, sample: i64, values: &[f32], into: &mut Planar, at: usize) {
        // Its gain and pan are where the timeline is (T18), which is `sample` but at a speed.
        let sample = self.warped.warp.timeline(sample);
        let mut gain = self.piece.gain.at(sample);
        if self.level.scaled {
            gain *= self.level.scale.next_value();
        }
        let pan = match self.pan {
            ClipPan::None => {
                for (channel, value) in values.iter().enumerate() {
                    into.channel_mut(channel)[at] += value * gain;
                }
                return;
            }
            ClipPan::Fixed(_) if self.level.panned => [
                self.level.sides[0].next_value(),
                self.level.sides[1].next_value(),
            ],
            ClipPan::Fixed(pan) => pan,
            ClipPan::Keyed => balance(self.piece.pan_at(sample)),
        };
        for (channel, (value, pan)) in values.iter().zip(pan).enumerate() {
            into.channel_mut(channel)[at] += value * gain * pan;
        }
    }

    /// Lays the clip on the samples out of a graph played through `warp` (T18), from what it was
    /// built as: the samples out it sounds on, the mapping of each to its media (the speed of
    /// playing times its own, backward where either is), and the filter for that speed; its
    /// stretch put aside at any warp but the identity, and back at the identity. The samples of
    /// its media that a block of `max_block` reads at the most, its filter's reach and all.
    /// Made off the thread of the mix.
    pub(crate) fn warp(&mut self, warp: Warp, max_block: usize) -> usize {
        let base = *self.warped.base.get_or_insert(Base {
            from: self.from,
            to: self.to,
            mapping: self.mapping,
            cutoff: self.cutoff,
            radius: self.radius,
        });
        // At a speed of 1 the clip stays on the timeline as it was built, its stretch and all:
        // the graph shifts the timeline it reads (`Graph::shift`).
        self.warped.warp = if warp.speed() == 1 {
            Warp::IDENTITY
        } else {
            warp
        };
        if warp.speed() == 1 {
            (self.from, self.to, self.mapping) = (base.from, base.to, base.mapping);
            (self.cutoff, self.radius) = (base.cutoff, base.radius);
            if let Some(stretch) = self.warped.stretch.take() {
                self.stretch = Some(stretch);
            }
            return 0;
        }
        if let Some(stretch) = self.stretch.take() {
            self.warped.stretch = Some(stretch);
        }
        let span = warp.span(base.from..base.to);
        self.mapping = base.mapping.warped(warp);
        let (step, denominator) = self.mapping.speed();
        if warp.speed() == -1 {
            // Its own speed backward: its own filter.
            (self.cutoff, self.radius) = (base.cutoff, base.radius);
        } else {
            #[allow(clippy::cast_precision_loss, reason = "a speed")]
            let speed = step.unsigned_abs() as f64 / denominator.unsigned_abs().max(1) as f64;
            self.cutoff = (1.0 / speed).min(1.0);
            self.radius = if warp.speed().abs() > crate::constants::SHUTTLE_FROM {
                shuttle_radius(self.cutoff)
            } else {
                radius(self.cutoff)
            };
        }
        // A clip whose media, its filter's reach and all, no sample count reaches at this speed
        // is silent (as the graph built it, `build.rs`).
        let reach = self.radius;
        let fits = |sample: i64| {
            self.mapping.checked_at(sample).is_some_and(|(at, _)| {
                at.checked_sub(reach).is_some() && at.checked_add(reach + 1).is_some()
            })
        };
        (self.from, self.to) = if span.is_empty() || (fits(span.start) && fits(span.end - 1)) {
            (span.start, span.end)
        } else {
            (span.start, span.start)
        };
        let block = i128::try_from(max_block).unwrap_or(i128::MAX);
        let span = (block * step.abs() + denominator - 1) / denominator.max(1)
            + 2 * i128::from(self.radius)
            + 2;
        usize::try_from(span).unwrap_or(usize::MAX)
    }

    /// Back to silence, its equalizer cleared and its stretcher let go (the pools are freed by
    /// [`ClipScratch::reset`]).
    pub(crate) fn reset(&mut self) {
        if let Some(eq) = &mut self.eq {
            eq.reset();
        }
        self.processed = 0;
        if let Some(stretch) = &mut self.stretch {
            stretch.held = None;
        }
        if let Some(effects) = &mut self.effects {
            effects.reset();
        }
    }

    /// Brings the stretch of the clip to the sample `at` of the timeline, where a run starts
    /// (`Graph::start`): its stretchers worked out from the starts of their segments, as a run
    /// from earlier did, so that the work is done before the sound plays and the blocks after
    /// sound as that run's. Nothing for a clip that is not stretched or does not read its media
    /// just before `at` (where its effects are late, as far ahead).
    pub(crate) fn prime(
        &mut self,
        index: usize,
        feed: &mut dyn ClipFeed,
        at: i64,
        scratch: &mut ClipScratch,
    ) -> Result<(), FeedError> {
        let at = at.saturating_add(self.ahead());
        if self.stretch.is_none() || at <= self.busy_at() || at > self.to {
            return Ok(());
        }
        self.stretch_into(index, feed, at - 1..at, None, scratch)
    }

    /// Adds what a clip with effects sounds on the `frames` samples of the timeline from `start`:
    /// its sound worked out as far ahead as its effects are late (its stretch moved on where it
    /// warms), run through them, and what leaves them added on its own samples.
    fn mix_through_effects(
        &mut self,
        index: usize,
        feed: &mut dyn ClipFeed,
        (start, frames, rate): (i64, usize, u32),
        scratch: &mut ClipScratch,
        into: &mut Planar,
    ) -> Result<(), FeedError> {
        let ahead = self.ahead();
        let length = i64::try_from(frames).unwrap_or(i64::MAX);
        let (first, last) = (
            start.saturating_add(ahead),
            start.saturating_add(length).saturating_add(ahead),
        );
        let mut result = Ok(());
        if let Some(stretch) = &self.stretch {
            let (warm, sounds) = (stretch.busy_from.max(first), self.from.min(last));
            if warm < sounds {
                let warmed = self.stretch_into(index, feed, warm..sounds, None, scratch);
                keep(&mut result, warmed);
            }
        }
        // Its sound from where it starts, then silence until what is in its effects is out.
        let (from, to) = (
            self.from.max(first),
            self.to.saturating_add(ahead).min(last),
        );
        if from < to {
            let count = usize::try_from(to - from).unwrap_or(0);
            let sounding = usize::try_from(self.to.min(to) - from)
                .unwrap_or(0)
                .min(count);
            if sounding > 0 {
                let end = from + i64::try_from(sounding).unwrap_or(0);
                keep(
                    &mut result,
                    self.sound_of(index, feed, from..end, rate, scratch),
                );
            }
            let channels = self.channels;
            for channel in 0..channels {
                scratch.after.channel_mut(channel)[sounding..count].fill(0.0);
            }
            if let Some(effects) = &mut self.effects {
                effects.run(from, count, rate, channels, scratch);
            }
            let mut values = [0.0_f32; MAX_CHANNELS];
            for n in 0..count {
                let sample = from + i64::try_from(n).unwrap_or(0) - ahead;
                if sample < self.from || sample >= self.to {
                    continue;
                }
                for (channel, value) in values[..channels].iter_mut().enumerate() {
                    *value = scratch.after.channel(channel)[n];
                }
                let at = usize::try_from(sample - start).unwrap_or(0);
                self.add(sample, &values[..channels], into, at);
            }
        }
        if self.stretch.is_some() && last >= self.to {
            self.release(scratch);
        }
        result
    }

    /// Works out the clip's sound on the samples `span` of the timeline (where it sounds, at
    /// most a block), through its equalizer, into the first samples of [`ClipScratch::after`]:
    /// stretched, or resampled to its speed. Silence where the feed fails.
    fn sound_of(
        &mut self,
        index: usize,
        feed: &mut dyn ClipFeed,
        span: Range<i64>,
        rate: u32,
        scratch: &mut ClipScratch,
    ) -> Result<(), FeedError> {
        let count = usize::try_from(span.end - span.start).unwrap_or(0);
        let channels = self.channels;
        let read = if self.stretch.is_some() {
            self.stretch_into(index, feed, span, Some(count), scratch)
        } else {
            self.resampled_into(index, feed, span, scratch)
        };
        if let Some(eq) = &mut self.eq {
            let block = Block::new(self.processed, rate, count);
            scratch.before.read(count, |input| {
                scratch.after.write(count, |output| {
                    eq.process(&block, &input[..channels], &mut output[..channels]);
                });
            });
            self.processed += u64::try_from(count).unwrap_or(0);
        } else {
            std::mem::swap(&mut scratch.before, &mut scratch.after);
        }
        read
    }

    /// The clip's sound resampled to its speed on the samples `span` of the timeline (at most a
    /// block), into the first samples of [`ClipScratch::before`]; silence where the feed fails.
    fn resampled_into(
        &self,
        index: usize,
        feed: &mut dyn ClipFeed,
        span: Range<i64>,
        scratch: &mut ClipScratch,
    ) -> Result<(), FeedError> {
        let (count, channels) = (
            usize::try_from(span.end - span.start).unwrap_or(0),
            self.channels,
        );
        let source = self.source(&span);
        let length = usize::try_from(source.end - source.start).unwrap_or(usize::MAX);
        let read = if length > scratch.window.capacity {
            Err(FeedError { clip: index })
        } else {
            let mut slices = scratch.window.slices_mut(channels, length, self.film);
            feed.read(index, source.clone(), &mut slices[..channels])
        };
        if read.is_err() {
            for channel in 0..channels {
                scratch.before.channel_mut(channel)[..count].fill(0.0);
            }
            return read;
        }
        let window = scratch.window.slices(channels, length);
        let mut values = [0.0_f32; MAX_CHANNELS];
        for (n, sample) in span.enumerate() {
            let (at, phase) = self.mapping.at(sample);
            let at = usize::try_from(at - source.start).unwrap_or(0);
            resampled_with(
                &window[..channels],
                at,
                phase,
                (self.cutoff, self.radius),
                &mut values[..channels],
            );
            for (channel, value) in values[..channels].iter().enumerate() {
                scratch.before.channel_mut(channel)[n] = *value;
            }
        }
        Ok(())
    }

    /// Adds what a stretched clip sounds on the `frames` samples of the timeline from `start`,
    /// and moves its stretchers on where it warms before it sounds; lets go of its stretcher once
    /// it has sounded its last.
    fn mix_stretched(
        &mut self,
        index: usize,
        feed: &mut dyn ClipFeed,
        (start, frames, rate): (i64, usize, u32),
        scratch: &mut ClipScratch,
        into: &mut Planar,
    ) -> Result<(), FeedError> {
        let end = start.saturating_add(i64::try_from(frames).unwrap_or(i64::MAX));
        let (first, last) = (self.busy_from().max(start), self.to.min(end));
        let mut result = Ok(());
        if first < last {
            let from = self.from.max(first);
            if first < from {
                let warmed = self.stretch_into(index, feed, first..from, None, scratch);
                keep(&mut result, warmed);
            }
            if from < last {
                let count = usize::try_from(last - from).unwrap_or(0);
                let stretched = self.stretch_into(index, feed, from..last, Some(count), scratch);
                keep(&mut result, stretched);
                self.sound(from, count, rate, start, scratch, into);
            }
        }
        if end >= self.to {
            self.release(scratch);
        }
        result
    }

    /// Works out the stretch of the samples `span` of the timeline into the first `count`
    /// samples of [`ClipScratch::before`] where it is given (else only moving the stretchers on),
    /// taking a stretcher from the pool if the clip holds none. Silence where the pool has none
    /// left (it is made large enough that it never happens).
    fn stretch_into(
        &mut self,
        index: usize,
        feed: &mut dyn ClipFeed,
        span: Range<i64>,
        count: Option<usize>,
        scratch: &mut ClipScratch,
    ) -> Result<(), FeedError> {
        let (mapping, cutoff, radius, channels, film) = (
            self.mapping,
            self.cutoff,
            self.radius,
            self.channels,
            self.film,
        );
        let Some(stretch) = &mut self.stretch else {
            return Ok(());
        };
        let ClipScratch {
            window,
            before,
            pools,
            ..
        } = scratch;
        let max_block = before.frames();
        let held = match (stretch.held, pools.get_mut(stretch.pool)) {
            (Some(held), _) => Some(held),
            (None, Some(pool)) => pool.free.pop(),
            (None, None) => None,
        };
        stretch.held = held;
        let stretcher = held.and_then(|held| {
            pools
                .get_mut(stretch.pool)
                .and_then(|pool| pool.stretchers.get_mut(held))
        });
        let Some(stretcher) = stretcher else {
            if let Some(count) = count {
                before.write(count, |out| {
                    for channel in out.iter_mut() {
                        channel[..count].fill(0.0);
                    }
                });
            }
            return Ok(());
        };
        let mut input = ClipInput {
            feed,
            window,
            index,
            channels,
            film,
            mapping,
            mode: stretch.mode,
            latencies: stretch.latencies,
            cutoff,
            radius,
            max_block,
        };
        let transpose = Transpose::Fixed(stretch.transpose);
        let plan = stretch.plan;
        match count {
            Some(count) => before.write(count, |out| {
                stretcher.render(
                    &plan,
                    span,
                    &mut input,
                    transpose,
                    Some(&mut out[..channels]),
                )
            }),
            None => stretcher.render(&plan, span, &mut input, transpose, None),
        }
    }

    /// Adds the first `count` samples of [`ClipScratch::before`], the clip's sound from the
    /// sample `from` of the timeline, through its equalizer, by its gain and pan, into `into`,
    /// whose block starts at `start`.
    fn sound(
        &mut self,
        from: i64,
        count: usize,
        rate: u32,
        start: i64,
        scratch: &mut ClipScratch,
        into: &mut Planar,
    ) {
        let channels = self.channels;
        let mut values = [0.0_f32; MAX_CHANNELS];
        let values = &mut values[..channels];
        let sound = if let Some(eq) = &mut self.eq {
            let block = Block::new(self.processed, rate, count);
            scratch.before.read(count, |input| {
                scratch.after.write(count, |output| {
                    eq.process(&block, &input[..channels], &mut output[..channels]);
                });
            });
            self.processed += u64::try_from(count).unwrap_or(0);
            &scratch.after
        } else {
            &scratch.before
        };
        for n in 0..count {
            let sample = from + i64::try_from(n).unwrap_or(0);
            for (channel, value) in values.iter_mut().enumerate() {
                *value = sound.channel(channel)[n];
            }
            let at = usize::try_from(sample - start).unwrap_or(0);
            self.add(sample, values, into, at);
        }
    }

    /// Lets go of the clip's stretcher, back into its pool.
    fn release(&mut self, scratch: &mut ClipScratch) {
        let Some(stretch) = &mut self.stretch else {
            return;
        };
        if let Some(held) = stretch.held.take() {
            if let Some(pool) = scratch.pools.get_mut(stretch.pool) {
                if let Some(stretcher) = pool.stretchers.get_mut(held) {
                    stretcher.release();
                }
                pool.free.push(held);
            }
        }
    }
}
