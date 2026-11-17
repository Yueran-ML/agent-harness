//! The patterns of a Tracker of a composition (Reference Manual 20, Chapter 82, pp. 1724–1759):
//! each followed through the frames on its own, as Fusion's trackers are, by the pyramidal
//! Lucas–Kanade of the point tracker with a square window of its own size, checked as the
//! crosshairs of the point tracker are, and kept within its search area.
//!
//! - **The window** of a pattern is the largest square of an odd side within its box, from 7×7
//!   to 63×63 samples ([`radius_for`]): what lies about the pattern is not compared, and a
//!   window larger than that would cost much and tell little more.
//! - **The search area** is centred where the pattern was in the frame before, and the pattern
//!   is to stay within it: its middle moves no further than half the difference of the two,
//!   across and down ([`reach_for`]), and half a sample more for where it lies between samples.
//!   A search area no larger than the pattern lets it move no more than that half sample.
//! - **The pyramid** has as many levels as the farthest reach needs, the reach at its coarsest
//!   level being within half the window ([`levels_for`]), at most [`MOST_LEVELS`]. All the
//!   patterns are followed on one pyramid of each frame.
//! - **Each frame** the window is taken from the frame before, where the pattern was found there
//!   (Fusion's Adaptive Mode of Every Frame), and the pattern is looked for first where it would
//!   be if it went on as it went from the frame before.
//! - **A pattern is lost**, and followed no more, where its window is too flat, where it leaves
//!   the frame, where tracked back it lands further than
//!   [`PatternOptions::forward_backward`] from where it was, where its windows in the two frames
//!   correlate less than [`PatternOptions::min_ncc`], and where it is found beyond its search
//!   area. The thresholds are those of the crosshairs of the point tracker.

use crate::cloud::MIN_EIGEN;
use crate::image::{Luma, Pyramid};
use crate::linalg::Point;
use crate::lk::{par_map, threads, track, Lk, Scratch};
use crate::TrackError;

/// The least half side of the window of a pattern: 7×7 samples.
pub const LEAST_RADIUS: usize = 3;

/// The most half side of the window of a pattern: 63×63 samples, about four times the samples
/// of the 31×31 window of a crosshair of the point tracker.
pub const MOST_RADIUS: usize = 31;

/// The most levels of the pyramid that patterns are followed on: as many as the point tracker
/// has at the most.
pub const MOST_LEVELS: usize = 6;

/// The half side of the window of a pattern whose box is `size` samples wide and high: that of
/// the largest square of an odd side within the box, held from [`LEAST_RADIUS`] to
/// [`MOST_RADIUS`].
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "a half side held within a small range before it is made a whole number"
)]
pub fn radius_for(size: [f64; 2]) -> usize {
    let least = size[0].min(size[1]);
    let half = ((least - 1.0) / 2.0).floor();
    if half.is_nan() {
        return LEAST_RADIUS;
    }
    half.clamp(LEAST_RADIUS as f64, MOST_RADIUS as f64) as usize
}

/// How far the middle of a pattern whose box is `size` samples wide and high may move from a
/// frame to the next, across and down, where its search area is `search` samples wide and high:
/// as far as the box stays within the search area centred where the pattern was, half the
/// difference of the two, and none where the search area is no larger than the box.
#[must_use]
pub fn reach_for(size: [f64; 2], search: [f64; 2]) -> [f64; 2] {
    // Of no number, none too.
    [0, 1].map(|k| ((search[k] - size[k]) / 2.0).max(0.0))
}

/// The levels of a pyramid on which a pattern with a window of half side `radius` is found as
/// far as `reach` away, across and down: enough that the farther of the two, halved at each
/// level, is within half the window at the coarsest, from 1 to [`MOST_LEVELS`].
#[must_use]
#[allow(clippy::cast_precision_loss, reason = "a small half side")]
pub fn levels_for(radius: usize, reach: [f64; 2]) -> usize {
    let half = radius as f64 / 2.0;
    let mut far = reach[0].max(reach[1]);
    let mut levels = 1;
    while far > half && levels < MOST_LEVELS {
        far /= 2.0;
        levels += 1;
    }
    levels
}

/// A pattern to follow, in samples of the frames (see the coordinates of the crate).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pattern {
    /// Where its middle is in the first frame.
    pub at: Point,
    /// The half side of its window: it is `2·radius + 1` samples a side ([`radius_for`]).
    pub radius: usize,
    /// How far its middle may move from a frame to the next, across and down ([`reach_for`]).
    pub reach: [f64; 2],
}

impl Pattern {
    /// A pattern at `at` whose box is `size` samples wide and high, searched for in an area
    /// `search` samples wide and high.
    #[must_use]
    pub fn new(at: Point, size: [f64; 2], search: [f64; 2]) -> Self {
        Self {
            at,
            radius: radius_for(size),
            reach: reach_for(size, search),
        }
    }

    /// The levels of a pyramid that it needs ([`levels_for`]).
    #[must_use]
    pub fn levels(&self) -> usize {
        levels_for(self.radius, self.reach)
    }
}

/// The settings of a [`PatternTracker`].
#[derive(Clone, Debug, PartialEq)]
pub struct PatternOptions {
    /// The most steps of Lucas–Kanade at each level.
    pub iterations: usize,
    /// A pattern is lost when tracking it back lands farther than this from where it was, in
    /// samples.
    pub forward_backward: f64,
    /// A pattern is lost when its window in the frame before and its window where it was found
    /// correlate less.
    pub min_ncc: f64,
    /// How much further than its reach a pattern may be found, in samples, for where it lies
    /// between samples.
    pub slack: f64,
    /// The threads to make the pyramids and follow the patterns on, or 0 for those of the
    /// machine (at most 8).
    pub threads: usize,
}

impl Default for PatternOptions {
    /// Those of the crosshairs of the point tracker ([`PointOptions`](crate::PointOptions)):
    /// 20 steps at each level, a sample back at the most and a correlation of 0.7 at the least;
    /// and half a sample of slack.
    fn default() -> Self {
        Self {
            iterations: 20,
            forward_backward: 1.0,
            min_ncc: 0.7,
            slack: 0.5,
            threads: 0,
        }
    }
}

/// A follower of patterns from the frame it was made on, one frame at a time, each on its own.
#[derive(Debug)]
pub struct PatternTracker {
    options: PatternOptions,
    patterns: Vec<Pattern>,
    prev: Pyramid,
    /// Where each is now, `None` once lost.
    now: Vec<Option<Point>>,
    /// How far each moved from the frame before the last one to the last one.
    velocity: Vec<Point>,
}

impl PatternTracker {
    /// A follower of `patterns` from `first`, on pyramids of as many levels as the farthest of
    /// them needs. A pattern whose middle lies outside the frame is lost from the start.
    ///
    /// # Errors
    /// [`TrackError::NoPoints`] when there are no patterns.
    pub fn new(
        first: &Luma,
        patterns: &[Pattern],
        options: PatternOptions,
    ) -> Result<Self, TrackError> {
        if patterns.is_empty() {
            return Err(TrackError::NoPoints);
        }
        #[allow(clippy::cast_precision_loss, reason = "sides of at most 2^24")]
        let (w, h) = (first.width() as f64, first.height() as f64);
        let inside = |p: Point| p[0] >= 0.0 && p[1] >= 0.0 && p[0] <= w - 1.0 && p[1] <= h - 1.0;
        let levels = patterns.iter().map(Pattern::levels).max().unwrap_or(1);
        Ok(Self {
            prev: Pyramid::on_threads(first.clone(), levels, threads(options.threads)),
            now: patterns
                .iter()
                .map(|pattern| inside(pattern.at).then_some(pattern.at))
                .collect(),
            velocity: vec![[0.0; 2]; patterns.len()],
            patterns: patterns.to_vec(),
            options,
        })
    }

    /// Where each pattern is in the last frame, in the order they were given; `None` once lost.
    #[must_use]
    pub fn points(&self) -> &[Option<Point>] {
        &self.now
    }

    /// Follow the patterns that are not lost into `next`, which then becomes the frame that
    /// they are followed from; where each is there, as [`Self::points`] has it.
    ///
    /// # Errors
    /// [`TrackError::FrameSize`] when `next` is not the size of the first frame.
    pub fn step_owned(&mut self, next: Luma) -> Result<&[Option<Point>], TrackError> {
        let base = self.prev.base();
        if (next.width(), next.height()) != (base.width(), base.height()) {
            return Err(TrackError::FrameSize {
                expected: [base.width(), base.height()],
                got: [next.width(), next.height()],
            });
        }
        let next = Pyramid::on_threads(next, self.prev.len(), threads(self.options.threads));
        // The patterns still followed, each with where it is.
        let work: Vec<(usize, Point)> = self
            .now
            .iter()
            .enumerate()
            .filter_map(|(at, point)| point.map(|point| (at, point)))
            .collect();
        let (prev, options, patterns, velocity) =
            (&self.prev, &self.options, &self.patterns, &self.velocity);
        // Each pattern has a window of its own size, and so scratch space of its own: that of
        // the threads, made for the least window, is not used.
        let results = par_map(
            &work,
            threads(options.threads),
            |&(at, point), _| follow(prev, &next, &patterns[at], point, velocity[at], options),
            LEAST_RADIUS,
        );
        for (&(at, from), found) in work.iter().zip(results) {
            self.now[at] = found;
            if let Some(to) = found {
                self.velocity[at] = [to[0] - from[0], to[1] - from[1]];
            }
        }
        self.prev = next;
        Ok(&self.now)
    }
}

/// Where `pattern`, at `at` in `prev`, is in `next`, looked for first `velocity` further on;
/// `None` where it is lost (see the module).
fn follow(
    prev: &Pyramid,
    next: &Pyramid,
    pattern: &Pattern,
    at: Point,
    velocity: Point,
    options: &PatternOptions,
) -> Option<Point> {
    let lk = Lk {
        radius: pattern.radius,
        iterations: options.iterations,
        epsilon: 0.01,
        min_eigen: MIN_EIGEN,
    };
    let mut scratch = Scratch::new(pattern.radius);
    #[allow(clippy::cast_possible_truncation, reason = "a point in a frame")]
    let to32 = |p: Point| [p[0] as f32, p[1] as f32];
    let start = to32(at);
    let guess = to32([at[0] + velocity[0], at[1] + velocity[1]]);
    let ahead = track(prev, next, start, guess, &lk, &mut scratch)?;
    if f64::from(ahead.ncc) < options.min_ncc {
        return None;
    }
    let back = track(next, prev, ahead.pos, start, &lk, &mut scratch)?;
    let off = f64::from(back.pos[0] - start[0]).hypot(f64::from(back.pos[1] - start[1]));
    if off > options.forward_backward {
        return None;
    }
    let found = [f64::from(ahead.pos[0]), f64::from(ahead.pos[1])];
    let within = (0..2).all(|k| (found[k] - at[k]).abs() <= pattern.reach[k] + options.slack);
    within.then_some(found)
}
