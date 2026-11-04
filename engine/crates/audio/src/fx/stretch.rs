//! The stretch that keeps the pitch of a clip and moves the pitch of a clip or a track (plan T13):
//! Signalsmith Stretch (`columbina-stretch`), run so that every sample out depends only on the
//! project, never on where the sound started or how it was cut into blocks.
//!
//! # Segments
//!
//! A phase vocoder carries the phases of its sound on for ever, so that a stretch started in the
//! middle would never sound as one started earlier. The sound is therefore stretched in segments,
//! each by a stretcher started afresh ([`Grid`]):
//!
//! - a clip's segment 0 starts at the first sample the clip sounds on, and segment `k ≥ 1` where
//!   `k` periods of [`STRETCH_RESET_INTERVAL`] of its source have played (counted from there in
//!   the direction it plays); an insert's segments start every period of the timeline;
//! - each segment `k ≥ 1` takes over from the one before over a crossfade of
//!   [`STRETCH_RESET_CROSSFADE`] centred on its point, equal power and corrected by how alike the
//!   two are there ([`Stretcher::render`]);
//! - a segment's stretcher starts [`Stretcher::warm`] samples before its own sound (a block and
//!   an interval of the library): it is reset, handed the input before that point
//!   ([`Stretch::seek`]), and its output until its own sound is thrown away.
//!
//! Two stretchers serve the segments in turn (even and odd), since a segment's warming starts
//! after the one two before it has faded out: a period is made long enough for that (at a fast
//! speed the period of the source is lengthened, [`Grid::clip`]).
//!
//! # Chunks
//!
//! A stretcher's output depends on how its calls are cut. Each one's calls are cut on a grid of
//! its own: [`CHUNK`] samples of output from where it started, each fed the input that its
//! [`Input::cursor`] says. A run that starts in the middle of a segment brings that segment's
//! stretcher from the start of the segment to there, chunk by chunk, as the run from the start
//! did: so a clip, whose sound can be read anywhere, sounds the same to the bit wherever it starts.
//! An insert, which has only the sound handed to it, starts the segments under way at its start
//! late, and sounds as one started earlier once they are over ([`Grid::Timeline`]).
//!
//! # The two times
//!
//! [`Stretcher::new`] takes all the memory, off the thread of the mix. [`Stretcher::render`] and
//! [`Stretcher::release`] take none.

use std::ops::Range;

use columbina_stretch::{Config, Error, Stretch};

use crate::constants::{samples, STRETCH_RESET_CROSSFADE, STRETCH_RESET_INTERVAL};
use crate::feed::FeedError;
use crate::node::Planar;

/// The samples of output of each call of a stretcher: the grid that its calls are cut on.
pub const CHUNK: usize = 64;

/// Where the segments start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grid {
    /// Of a clip: segment 0 at `from`, the first sample of the timeline it sounds on; segment
    /// `k ≥ 1` at `from + ⌈k · period · denominator / step⌉ − half`, where `period` samples of its
    /// source (at the rate of the timeline) have played at `step / denominator` of them a sample.
    Clip {
        /// The first sample it sounds on.
        from: i64,
        /// The samples of the source a period (at the rate of the timeline).
        period: i128,
        /// The samples of the source played in `denominator` samples of the timeline.
        step: i128,
        /// See `step`.
        denominator: i128,
        /// Half the crossfade.
        half: i64,
    },
    /// Of an insert: segment `k` at `k · period − half` of the timeline.
    Timeline {
        /// The samples of the timeline a period.
        period: i64,
        /// Half the crossfade.
        half: i64,
    },
}

impl Grid {
    /// The grid of a clip that sounds from `from` and plays `step / denominator` samples of its
    /// source a sample, at `rate`, crossfaded over `fade` and warmed over `warm`: segments of
    /// [`STRETCH_RESET_INTERVAL`] of its source, lengthened where it plays so fast that a segment
    /// would last less than a crossfade and a warming (two stretchers must do).
    #[must_use]
    pub fn clip(
        from: i64,
        (step, denominator): (i128, i128),
        rate: u32,
        fade: i64,
        warm: i64,
    ) -> Grid {
        let (step, denominator) = (step.abs().max(1), denominator.max(1));
        let period = i128::try_from(samples(STRETCH_RESET_INTERVAL, rate)).unwrap_or(i128::MAX);
        let half = fade / 2;
        let shortest = i128::from(fade + warm + half + 3);
        let least = (shortest * step + denominator - 1) / denominator;
        Grid::Clip {
            from,
            period: period.max(least),
            step,
            denominator,
            half,
        }
    }

    /// The grid of an insert at `rate`, crossfaded over `fade`.
    #[must_use]
    pub fn timeline(rate: u32, fade: i64) -> Grid {
        Grid::Timeline {
            period: i64::try_from(samples(STRETCH_RESET_INTERVAL, rate)).unwrap_or(i64::MAX),
            half: fade / 2,
        }
    }

    /// The sample where segment `k`'s own sound starts (its crossfade in, for all but a clip's
    /// first).
    #[must_use]
    pub fn start(&self, k: i64) -> i64 {
        match *self {
            Grid::Clip {
                from,
                period,
                step,
                denominator,
                half,
            } => {
                if k <= 0 {
                    return from;
                }
                let played = i128::from(k) * period * denominator;
                let after = (played + step - 1).div_euclid(step);
                clamped(i128::from(from) + after - i128::from(half))
            }
            Grid::Timeline { period, half } => k.saturating_mul(period).saturating_sub(half),
        }
    }

    /// The segment whose own sound holds `v`: the last that starts at or before it (0 for a
    /// clip's samples before it sounds).
    #[must_use]
    pub fn segment(&self, v: i64) -> i64 {
        match *self {
            Grid::Clip {
                from,
                period,
                step,
                denominator,
                half,
            } => {
                let played = (i128::from(v) + i128::from(half) - i128::from(from)) * step;
                clamped(played.div_euclid(period * denominator)).max(0)
            }
            Grid::Timeline { period, half } => v.saturating_add(half).div_euclid(period.max(1)),
        }
    }
}

/// `value` within an `i64`.
fn clamped(value: i128) -> i64 {
    i64::try_from(value).unwrap_or(if value < 0 { i64::MIN } else { i64::MAX })
}

/// How a stretch is run: its grid, its first and last segment, the earliest sample a stretcher
/// may start at, and its change of speed.
#[derive(Clone, Copy, Debug)]
pub struct Plan {
    /// Where its segments start.
    pub grid: Grid,
    /// The first segment that sounds (0 for a clip), and the last (that of the last sample a
    /// clip sounds on): no other is stretched.
    pub segments: (i64, i64),
    /// No stretcher starts before this sample: the first an insert processed since its reset
    /// (a segment under way there starts there, late); `i64::MIN` for a clip, whose stretchers
    /// start where their segments do.
    pub floor: i64,
    /// Input samples for each output sample (what a seek is told).
    pub rate: f64,
}

/// What a stretch is fed.
pub trait Input {
    /// The position of the input (counted in the order it is fed) that a stretcher has been fed
    /// up to, not included, when its output reaches `v`.
    fn cursor(&self, v: i64) -> i64;

    /// Writes the input at positions `range` into `into` (a slice of that length for each
    /// channel). On an error it writes silence and says so.
    ///
    /// # Errors
    ///
    /// The sound could not be read.
    fn fill(&mut self, range: Range<i64>, into: &mut [&mut [f32]]) -> Result<(), FeedError>;
}

/// The move of the pitch: one factor, or one for each sample of what is rendered (an insert's,
/// ramped and automated), taken where a chunk is worked out.
#[derive(Clone, Copy, Debug)]
pub enum Transpose<'a> {
    /// One factor.
    Fixed(f32),
    /// A factor for each sample of the span rendered.
    Each(&'a [f32]),
}

impl Transpose<'_> {
    fn at(&self, n: usize) -> f32 {
        match *self {
            Transpose::Fixed(factor) => factor,
            Transpose::Each(factors) => factors.get(n).copied().unwrap_or(1.0),
        }
    }
}

/// One stretcher and where it is in its segment.
#[derive(Debug)]
struct Lane {
    stretch: Stretch,
    /// The segment it is stretching.
    segment: Option<i64>,
    /// The sample of its first output.
    start: i64,
    /// The sample up to which its output is worked out, not included.
    done: i64,
    /// Its last outputs, a whole number of chunks: the sample `v` at `(v − start) mod` its length,
    /// for `v` from `done` less its length to `done`.
    out: Planar,
    /// The correlation of its output with the one before it over the crossfade into it, once
    /// worked out.
    correlation: Option<f64>,
}

impl Lane {
    /// Where the output of the sample `v` is in [`Lane::out`].
    fn at(&self, v: i64) -> usize {
        let ring = i64::try_from(self.out.frames()).unwrap_or(i64::MAX).max(1);
        usize::try_from((v - self.start).rem_euclid(ring)).unwrap_or(0)
    }
}

/// Two stretchers that take the segments of a stretch in turn, and what they are fed from: all
/// the memory of a stretch of a number of channels.
#[derive(Debug)]
pub struct Stretcher {
    lanes: [Lane; 2],
    input: Planar,
    config: Config,
    channels: usize,
    warm: i64,
    fade: i64,
}

impl Stretcher {
    /// Two stretchers of `channels` at `rate`, fed at most `max_input` samples by a chunk (and at
    /// least a seek's): all the memory they will take.
    ///
    /// # Errors
    ///
    /// As [`Stretch::new`].
    pub fn new(channels: usize, rate: u32, max_input: usize) -> Result<Stretcher, Error> {
        let fade = samples(STRETCH_RESET_CROSSFADE, rate);
        // A crossfade and a chunk on either side of it, in whole chunks.
        let ring = (fade + 2 * CHUNK).div_ceil(CHUNK) * CHUNK;
        let lane = || -> Result<Lane, Error> {
            Ok(Lane {
                stretch: Stretch::new(channels, rate)?,
                segment: None,
                start: 0,
                done: 0,
                out: Planar::new(channels, ring),
                correlation: None,
            })
        };
        let lanes = [lane()?, lane()?];
        let config = lanes[0].stretch.config();
        Ok(Stretcher {
            input: Planar::new(channels, max_input.max(config.seek_length).max(CHUNK)),
            config,
            channels,
            warm: Stretcher::warm_of(&config),
            fade: i64::try_from(fade).unwrap_or(0),
            lanes,
        })
    }

    /// The samples that a stretcher runs before its segment's own sound (a block and an interval
    /// of the library), at `config`.
    #[must_use]
    pub fn warm_of(config: &Config) -> i64 {
        i64::try_from(config.block + config.interval).unwrap_or(i64::MAX)
    }

    /// The samples that a stretcher runs before its segment's own sound.
    #[must_use]
    pub fn warm(&self) -> i64 {
        self.warm
    }

    /// The samples of the crossfade between two segments.
    #[must_use]
    pub fn fade(&self) -> i64 {
        self.fade
    }

    /// The library's configuration.
    #[must_use]
    pub fn config(&self) -> Config {
        self.config
    }

    /// The channels.
    #[must_use]
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// The most input samples a chunk may be fed.
    #[must_use]
    pub fn max_input(&self) -> usize {
        self.input.frames()
    }

    /// Lets both stretchers go: the next segment asked for starts afresh. Takes no memory.
    pub fn release(&mut self) {
        for lane in &mut self.lanes {
            lane.segment = None;
        }
    }

    /// Works out the stretch of the samples `span` (one after the other, never back while a
    /// segment lasts) and writes it into `out` (a slice of `span`'s length for each channel), or
    /// only moves the stretchers on (a warming before a clip sounds) where there is none.
    ///
    /// The crossfade into a segment is equal power, `sin` in and `cos` out, both divided by
    /// `√(1 + 2ρ·sin·cos)`, where `ρ` is the correlation of the two segments' sound over the
    /// crossfade: two that are alike (`ρ` = 1) then add up to the sound itself, two that are not
    /// (`ρ` = 0) keep its power, and two that cancel are raised by at most 12 dB.
    ///
    /// # Errors
    ///
    /// The input could not all be read: what could not is silence, and the first error is told.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::many_single_char_names,
        reason = "a crossfade of a few hundred samples, samples of f32, the names of the module's                   account (segment j, sample v, gains a and b)"
    )]
    pub fn render(
        &mut self,
        plan: &Plan,
        span: Range<i64>,
        input: &mut dyn Input,
        transpose: Transpose,
        mut out: Option<&mut [&mut [f32]]>,
    ) -> Result<(), FeedError> {
        let mut result = Ok(());
        if span.start >= span.end {
            return result;
        }
        let (first, last) = plan.segments;
        let mut j = plan.grid.segment(span.start).clamp(first, last);
        let mut own = plan.grid.start(j);
        let mut next = if j < last {
            plan.grid.start(j + 1)
        } else {
            i64::MAX
        };
        let half_pi = std::f64::consts::FRAC_PI_2;
        for v in span.clone() {
            let n = usize::try_from(v - span.start).unwrap_or(0);
            while v >= next {
                j += 1;
                own = next;
                next = if j < last {
                    plan.grid.start(j + 1)
                } else {
                    i64::MAX
                };
            }
            let factor = transpose.at(n);
            if v < own {
                // Before the first segment sounds: its stretcher warms.
                self.advance(j, plan, v, input, factor, &mut result);
            } else {
                let i = v - own;
                let lane = self.advance(j, plan, v, input, factor, &mut result);
                let fading = j > first && i < self.fade;
                let (gain, before) = if fading {
                    let correlation = self.correlation(j, own, plan, input, factor, &mut result);
                    let t = (i as f64 + 0.5) / self.fade as f64 * half_pi;
                    let (a, b) = (t.sin(), t.cos());
                    let norm = 1.0 / (1.0 + 2.0 * correlation * a * b).max(1.0 / 16.0).sqrt();
                    let before = self.advance(j - 1, plan, v, input, factor, &mut result);
                    ((a * norm) as f32, Some((before, (b * norm) as f32)))
                } else {
                    (1.0, None)
                };
                if let Some(out) = out.as_deref_mut() {
                    let (l, at) = lane;
                    for (channel, out) in out.iter_mut().enumerate().take(self.channels) {
                        let mut y = gain * self.lanes[l].out.channel(channel)[at];
                        if let Some(((l, at), g)) = before {
                            y += g * self.lanes[l].out.channel(channel)[at];
                        }
                        out[n] = y;
                    }
                }
            }
            if j < last && v >= next - self.warm {
                // The next segment's stretcher warms.
                self.advance(j + 1, plan, v, input, factor, &mut result);
            }
        }
        result
    }

    /// The correlation of the sound of segment `k`, which starts at `own`, with that of the one
    /// before over the crossfade, worked out once (both stretchers brought to its end first):
    /// from −1 to 1, 0 where either is silent.
    #[allow(
        clippy::many_single_char_names,
        reason = "the names of the module's account (segment k, sample v)"
    )]
    fn correlation(
        &mut self,
        k: i64,
        own: i64,
        plan: &Plan,
        input: &mut dyn Input,
        factor: f32,
        result: &mut Result<(), FeedError>,
    ) -> f64 {
        let l = usize::from(k.rem_euclid(2) == 1);
        if self.lanes[l].segment == Some(k) {
            if let Some(correlation) = self.lanes[l].correlation {
                return correlation;
            }
        }
        let end = own + self.fade - 1;
        self.advance(k, plan, end, input, factor, result);
        self.advance(k - 1, plan, end, input, factor, result);
        let (this, that) = (&self.lanes[l], &self.lanes[1 - l]);
        let (mut both, mut ours, mut theirs) = (0.0_f64, 0.0_f64, 0.0_f64);
        for channel in 0..self.channels {
            let (a, b) = (this.out.channel(channel), that.out.channel(channel));
            for v in own..=end {
                let (x, y) = (f64::from(a[this.at(v)]), f64::from(b[that.at(v)]));
                both += x * y;
                ours += x * x;
                theirs += y * y;
            }
        }
        let power = (ours * theirs).sqrt();
        let correlation = if power > 1e-30 {
            (both / power).clamp(-1.0, 1.0)
        } else {
            0.0
        };
        self.lanes[l].correlation = Some(correlation);
        correlation
    }

    /// Brings the stretcher of segment `k` to `v`, starting it on that segment if it is on
    /// another (and bringing it from the segment's start); which stretcher it is, and where `v`
    /// is in its output.
    #[allow(
        clippy::many_single_char_names,
        reason = "the names of the module's account (segment k, sample v)"
    )]
    fn advance(
        &mut self,
        k: i64,
        plan: &Plan,
        v: i64,
        input: &mut dyn Input,
        factor: f32,
        result: &mut Result<(), FeedError>,
    ) -> (usize, usize) {
        let l = usize::from(k.rem_euclid(2) == 1);
        let chunk = i64::try_from(CHUNK).unwrap_or(i64::MAX);
        if self.lanes[l].segment != Some(k) {
            let start = (plan.grid.start(k) - self.warm).max(plan.floor);
            let seek = self.config.seek_length;
            let end = input.cursor(start);
            let lane = &mut self.lanes[l];
            lane.stretch.reset();
            lane.stretch.set_transpose(factor);
            let filled = self.input.write(seek, |slices| {
                input.fill(end - i64::try_from(seek).unwrap_or(0)..end, slices)
            });
            keep(result, filled);
            self.input
                .read(seek, |slices| lane.stretch.seek(slices, plan.rate));
            lane.segment = Some(k);
            lane.start = start;
            lane.done = start;
            lane.correlation = None;
        }
        while v >= self.lanes[l].done {
            let lane = &mut self.lanes[l];
            let (from, to) = (lane.done, lane.done + chunk);
            let (x0, x1) = (input.cursor(from), input.cursor(to));
            let count = usize::try_from(x1 - x0)
                .unwrap_or(0)
                .min(self.input.frames());
            let end = x0.saturating_add(i64::try_from(count).unwrap_or(0));
            let filled = self
                .input
                .write(count, |slices| input.fill(x0..end, slices));
            keep(result, filled);
            lane.stretch.set_transpose(factor);
            let at = lane.at(from);
            self.input.read(count, |slices| {
                lane.out
                    .write_range(at, CHUNK, |out| lane.stretch.process(slices, out));
            });
            lane.done = to;
        }
        (l, self.lanes[l].at(v))
    }
}

/// Keeps the first error of `result` and `next`.
fn keep(result: &mut Result<(), FeedError>, next: Result<(), FeedError>) {
    if let (Ok(()), Err(error)) = (&*result, next) {
        *result = Err(error);
    }
}
