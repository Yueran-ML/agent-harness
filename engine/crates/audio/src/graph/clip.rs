//! The chain of a clip (plan T4b, p. 3855): its sound read from its media ([`ClipFeed`]), at its
//! speed and direction (a rational mapping and a windowed sinc), in the channels of its track,
//! through its equalizer, by its gain (keyframes, fades and crossfades), and panned, added to the
//! sum of its track.
//!
//! The mapping and the resampling are those of the mixer of phases 1 and 2 (`server/src/play/
//! mix.rs`), moved here unchanged, so that a project of before sounds as it did, to the bit.

use std::ops::Range;

use columbina_model::resolve::AudioPiece;
use columbina_model::{Clip, Timeline};

use crate::constants::MAX_CHANNELS;
use crate::feed::{ClipFeed, FeedError};
use crate::fx::eq::Eq;
use crate::node::{Block, Node, Planar};
use crate::rt::{Domain, Smoothed};

use super::route::balance;

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
    use std::f64::consts::PI;
    if phase == 0.0 && cutoff == 1.0 {
        for (out, channel) in out.iter_mut().zip(channels) {
            *out = channel[index];
        }
        return;
    }
    let radius = radius(cutoff);
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

/// What the clips share: the window of the source, and the sound of a clip before and after its
/// equalizer.
#[derive(Debug)]
pub(crate) struct ClipScratch {
    pub(crate) window: Window,
    pub(crate) before: Planar,
    pub(crate) after: Planar,
}

/// Where a clip of a graph reads its media ([`crate::graph::Graph::reaches`]): the samples of the
/// timeline it sounds on, and the samples of its media (at the rate of the timeline, as
/// [`ClipFeed::read`] counts them) that any of them reads, the filter's reach and all. The
/// threads that read clips ahead of the graph (T5) read what it will ask for.
#[derive(Clone, Copy, Debug)]
pub struct Reach {
    from: i64,
    to: i64,
    mapping: Mapping,
    radius: i64,
}

impl Reach {
    /// The samples of the timeline that the clip sounds on.
    #[must_use]
    pub fn span(&self) -> Range<i64> {
        self.from..self.to
    }

    /// The samples of the media that the samples `timeline` of the timeline read, where the
    /// clip sounds on any of them.
    #[must_use]
    pub fn source(&self, timeline: Range<i64>) -> Option<Range<i64>> {
        let (from, to) = (self.from.max(timeline.start), self.to.min(timeline.end));
        if from >= to {
            return None;
        }
        let (first, _) = self.mapping.at(from);
        let (last, _) = self.mapping.at(to - 1);
        Some(first.min(last) - self.radius..first.max(last) + self.radius + 1)
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
}

impl ClipChain {
    /// The samples of the source that the samples `range` of the timeline read, with the reach
    /// of the filter on each side.
    pub(crate) fn source(&self, range: &Range<i64>) -> Range<i64> {
        let (first, _) = self.mapping.at(range.start);
        let (last, _) = self.mapping.at(range.end - 1);
        first.min(last) - self.radius..first.max(last) + self.radius + 1
    }

    /// How far either way from the sample of the source that a sample of the timeline maps to
    /// the chain reads: the filter's reach. (T13's stretch reads further: its version of this
    /// takes the place of this one when it is merged, and [`ClipChain::reach`] follows.)
    pub(crate) fn reads_within(&self) -> i64 {
        self.radius
    }

    /// The first sample of the timeline at which the chain reads its media: where it sounds.
    /// (T13's stretch warms before that: as [`ClipChain::reads_within`].)
    pub(crate) fn busy_from(&self) -> i64 {
        self.from
    }

    /// Where it reads its media, for the readers ahead of the graph (T5): from where it is busy,
    /// as far either way as it reads.
    pub(crate) fn reach(&self) -> Reach {
        Reach {
            from: self.busy_from(),
            to: self.to,
            mapping: self.mapping,
            radius: self.reads_within(),
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
                resampled(window, at, phase, self.cutoff, values);
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
                resampled(window, at, phase, self.cutoff, values);
                self.add(sample, values, into, offset(sample));
            }
        }
        Ok(())
    }

    /// Adds `values`, the sound of the clip at `sample` of the timeline, by its gain and pan,
    /// to sample `at` of `into`: `value · gain · pan`, as the mixer of phase 2 did.
    fn add(&mut self, sample: i64, values: &[f32], into: &mut Planar, at: usize) {
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

    /// Back to silence, its equalizer cleared.
    pub(crate) fn reset(&mut self) {
        if let Some(eq) = &mut self.eq {
            eq.reset();
        }
        self.processed = 0;
    }
}
