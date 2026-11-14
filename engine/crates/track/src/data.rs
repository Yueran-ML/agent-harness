//! The track data of a window: a curve for each component of its motion, the paths of the
//! crosshairs, and the keyframes of Frame mode, kept apart from the window's own parameters
//! (p. 3232).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::linalg::{Mat3, Point};
use crate::motion::{Component, Components, Motion};

/// Where a change made to a tracked window goes (p. 3236).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// The default: a change moves the window along its whole path, by changing the window
    /// itself.
    #[default]
    Clip,
    /// A change makes a keyframe of the tracker graph at the playhead; between keyframes the
    /// change is interpolated (pp. 3236, 3251).
    Frame,
}

/// What [`TrackData::adjust`] did with a change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Adjusted {
    /// Clip mode: apply this to the window's own shape, before the track (in the window's
    /// untracked place), so that it moves the same way at every frame.
    Window(Mat3),
    /// Frame mode: a keyframe was set at the frame.
    Keyed,
    /// The change turns the window over, or is not finite: nothing was done.
    Refused,
}

/// The track data of one window.
///
/// - **Curves** (the tracker graph, p. 3236): for each [`Component`], its value at the frames
///   tracked, the motion from where the window was placed ([`Motion`], about [`Self::anchor`]).
///   A frame between samples takes the line between them, which is how cleared stretches are
///   filled (p. 3238); a frame before the first sample or after the last takes that sample, so an
///   untracked part of the clip holds the first or last tracked place (p. 3232). A curve with no
///   sample is no motion.
/// - **Confidence**: how well each tracked frame was fitted, from 0 to 1.
/// - **Points**: the path of each crosshair of the point tracker (Show Track, p. 3238).
/// - **Keyframes** of Frame mode: a further motion about the tracked anchor at each keyframe,
///   interpolated between them and held beyond them.
///
/// Frames are those of the clip, as the keyframes of the grade count them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackData {
    /// The point the motion turns and scales about: the centre of the window, in its untracked
    /// place, in pixels of [`Self::frame_size`].
    pub anchor: Point,
    /// The size of the frames that were tracked, `[width, height]`.
    pub frame_size: [usize; 2],
    /// The boxes that were on when it was tracked: the curves of those off stay at no motion.
    pub components: Components,
    curves: BTreeMap<Component, BTreeMap<i64, f64>>,
    confidence: BTreeMap<i64, f32>,
    points: Vec<BTreeMap<i64, Point>>,
    keys: BTreeMap<i64, Motion>,
}

/// The value at `frame` of samples that change linearly between them and are held beyond.
fn between<T: Copy>(
    samples: &BTreeMap<i64, T>,
    frame: i64,
    lerp: impl Fn(T, T, f64) -> T,
) -> Option<T> {
    let before = samples.range(..=frame).next_back();
    let after = samples.range(frame..).next();
    match (before, after) {
        (Some((&a, &va)), Some((&b, &vb))) => {
            if a == b {
                Some(va)
            } else {
                #[allow(clippy::cast_precision_loss, reason = "frames are within 2^53")]
                let share = (frame - a) as f64 / (b - a) as f64;
                Some(lerp(va, vb, share))
            }
        }
        (Some((_, &v)), None) | (None, Some((_, &v))) => Some(v),
        (None, None) => None,
    }
}

impl TrackData {
    /// Empty track data of a window centred at `anchor`, for frames of `frame_size`.
    #[must_use]
    pub fn new(anchor: Point, frame_size: [usize; 2]) -> Self {
        Self {
            anchor,
            frame_size,
            ..Self::default()
        }
    }

    /// Whether nothing has been tracked (no curve has a sample). A tracked clip shows the badge
    /// of tracking on its thumbnail (p. 3231).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.curves.values().all(BTreeMap::is_empty)
    }

    /// The samples of the curve of `c`, by frame.
    #[must_use]
    pub fn curve(&self, c: Component) -> Option<&BTreeMap<i64, f64>> {
        self.curves.get(&c)
    }

    /// The first and last frames with a sample on any curve.
    #[must_use]
    pub fn tracked_range(&self) -> Option<(i64, i64)> {
        let first = self.curves.values().filter_map(|c| c.keys().next()).min()?;
        let last = self
            .curves
            .values()
            .filter_map(|c| c.keys().next_back())
            .max()?;
        Some((*first, *last))
    }

    /// Whether every curve has a sample at `frame`.
    #[must_use]
    pub fn has_sample(&self, frame: i64) -> bool {
        Component::ALL.iter().all(|c| {
            self.curves
                .get(c)
                .is_some_and(|curve| curve.contains_key(&frame))
        })
    }

    /// The value of `c` at `frame`.
    #[must_use]
    pub fn value_at(&self, c: Component, frame: i64) -> f64 {
        self.curves
            .get(&c)
            .and_then(|curve| between(curve, frame, |a, b, t| a + (b - a) * t))
            .unwrap_or_else(|| c.identity())
    }

    /// The tracked motion at `frame`.
    #[must_use]
    pub fn motion_at(&self, frame: i64) -> Motion {
        let mut out = Motion::IDENTITY;
        for c in Component::ALL {
            out.set(c, self.value_at(c, frame));
        }
        out
    }

    /// How well `frame` was fitted, when it was tracked.
    #[must_use]
    pub fn confidence_at(&self, frame: i64) -> Option<f32> {
        self.confidence.get(&frame).copied()
    }

    /// The confidence of every tracked frame.
    #[must_use]
    pub fn confidence(&self) -> &BTreeMap<i64, f32> {
        &self.confidence
    }

    /// Put `motion` on every curve at `frame`, fitted with `confidence`.
    pub fn set_sample(&mut self, frame: i64, motion: Motion, confidence: f32) {
        for c in Component::ALL {
            self.curves
                .entry(c)
                .or_default()
                .insert(frame, motion.get(c));
        }
        self.confidence.insert(frame, confidence);
    }

    /// Take `frame` off every curve (a frame that a new track could not fit, so that the old
    /// track does not stay there).
    pub fn remove_frame(&mut self, frame: i64) {
        self.clear_range(frame, frame, &Component::ALL);
    }

    /// Clear Selected Track Data (p. 3238): the samples of the curves `which` from `from` to `to`
    /// taken away, so that the line between the samples on either side fills the stretch.
    /// Clearing every curve takes away the confidence of those frames too.
    pub fn clear_range(&mut self, from: i64, to: i64, which: &[Component]) {
        let (from, to) = (from.min(to), from.max(to));
        for c in which {
            if let Some(curve) = self.curves.get_mut(c) {
                let gone: Vec<i64> = curve.range(from..=to).map(|(&f, _)| f).collect();
                for f in gone {
                    curve.remove(&f);
                }
            }
        }
        if Component::ALL.iter().all(|c| which.contains(c)) {
            let gone: Vec<i64> = self.confidence.range(from..=to).map(|(&f, _)| f).collect();
            for f in gone {
                self.confidence.remove(&f);
            }
        }
    }

    /// The samples of the curves `which` strictly between `from` and `to` made the line between
    /// their values at `from` and at `to`, at every frame (as the interpolation of keyframes
    /// does, p. 3238).
    pub fn interpolate_range(&mut self, from: i64, to: i64, which: &[Component]) {
        let (from, to) = (from.min(to), from.max(to));
        if to - from < 2 {
            return;
        }
        for &c in which {
            let (a, b) = (self.value_at(c, from), self.value_at(c, to));
            let curve = self.curves.entry(c).or_default();
            curve.insert(from, a);
            curve.insert(to, b);
            for f in from + 1..to {
                #[allow(clippy::cast_precision_loss, reason = "frames are within 2^53")]
                let share = (f - from) as f64 / (to - from) as f64;
                curve.insert(f, a + (b - a) * share);
            }
        }
    }

    /// Reset Track Data on Active Window (p. 3238): every curve, confidence and path cleared.
    /// The keyframes of Frame mode stay: they are the window's.
    pub fn reset(&mut self) {
        self.curves.clear();
        self.confidence.clear();
        self.points.clear();
    }

    /// The path of each crosshair of the point tracker.
    #[must_use]
    pub fn points(&self) -> &[BTreeMap<i64, Point>] {
        &self.points
    }

    /// Set the crosshairs to `n`, each with no path, unless there are `n` already.
    pub fn set_point_count(&mut self, n: usize) {
        if self.points.len() != n {
            self.points = vec![BTreeMap::new(); n];
        }
    }

    /// Where crosshair `i` is at `frame` (`None` to take it off that frame).
    pub fn set_point(&mut self, i: usize, frame: i64, at: Option<Point>) {
        if let Some(path) = self.points.get_mut(i) {
            match at {
                Some(p) => {
                    path.insert(frame, p);
                }
                None => {
                    path.remove(&frame);
                }
            }
        }
    }

    /// The keyframes of Frame mode.
    #[must_use]
    pub fn keys(&self) -> &BTreeMap<i64, Motion> {
        &self.keys
    }

    /// The motion of Frame mode at `frame`: its keyframes interpolated, no motion without any.
    #[must_use]
    pub fn key_at(&self, frame: i64) -> Motion {
        between(&self.keys, frame, |a, b, t| a.lerp(&b, t)).unwrap_or(Motion::IDENTITY)
    }

    /// A keyframe at `frame` (the Add Keyframe button sets one at the motion there, p. 3252).
    pub fn set_key(&mut self, frame: i64, motion: Motion) {
        self.keys.insert(frame, motion);
    }

    /// Delete Keyframe (p. 3252): the keyframe at `frame`, if there is one, taken away.
    pub fn delete_key(&mut self, frame: i64) -> bool {
        self.keys.remove(&frame).is_some()
    }

    /// The last keyframe before `frame` (Previous Keyframe).
    #[must_use]
    pub fn previous_key(&self, frame: i64) -> Option<i64> {
        self.keys.range(..frame).next_back().map(|(&f, _)| f)
    }

    /// The first keyframe after `frame` (Next Keyframe).
    #[must_use]
    pub fn next_key(&self, frame: i64) -> Option<i64> {
        self.keys.range(frame + 1..).next().map(|(&f, _)| f)
    }

    /// The homography of the tracked motion at `frame`, in pixels of the frame.
    #[must_use]
    pub fn track_matrix(&self, frame: i64) -> Mat3 {
        self.motion_at(frame).matrix(self.anchor)
    }

    /// Where the anchor is at `frame` by the track.
    #[must_use]
    pub fn tracked_anchor(&self, frame: i64) -> Point {
        let m = self.motion_at(frame);
        [self.anchor[0] + m.pan, self.anchor[1] + m.tilt]
    }

    /// What is done to the window's shape at `frame`: the track, then the keyframes of Frame mode
    /// about the tracked anchor. The window's own parameters come before both.
    #[must_use]
    pub fn window_matrix(&self, frame: i64) -> Mat3 {
        let key = self.key_at(frame).matrix(self.tracked_anchor(frame));
        key.mul(&self.track_matrix(frame))
    }

    /// A change `delta` made by hand to the window as it is seen at `frame` (a homography of the
    /// frame: a drag, a turn, a resize), applied as `mode` says (p. 3236).
    pub fn adjust(&mut self, mode: Mode, frame: i64, delta: &Mat3) -> Adjusted {
        let whole = self.window_matrix(frame);
        match mode {
            Mode::Clip => match whole.inverse() {
                Some(inv) => Adjusted::Window(inv.mul(delta).mul(&whole).normalized()),
                None => Adjusted::Refused,
            },
            Mode::Frame => {
                let at = self.tracked_anchor(frame);
                let key = delta.mul(&self.key_at(frame).matrix(at));
                match Motion::from_matrix(&key, at) {
                    Some(motion) => {
                        self.keys.insert(frame, motion);
                        Adjusted::Keyed
                    }
                    None => Adjusted::Refused,
                }
            }
        }
    }

    /// Paste Track Data (p. 3233): the curves, confidence and paths of `source` put on this
    /// window, as the same motion of the frame about this window's anchor, so that windows on
    /// one subject move as one. This window's keyframes of Frame mode stay.
    pub fn paste(&mut self, source: &Self) {
        let mut frames: Vec<i64> = source
            .curves
            .values()
            .flat_map(|c| c.keys().copied())
            .collect();
        frames.sort_unstable();
        frames.dedup();
        self.curves.clear();
        for f in frames {
            if let Some(m) = Motion::from_matrix(&source.track_matrix(f), self.anchor) {
                for c in Component::ALL {
                    self.curves.entry(c).or_default().insert(f, m.get(c));
                }
            }
        }
        self.confidence.clone_from(&source.confidence);
        self.points.clone_from(&source.points);
        self.components = source.components;
        self.frame_size = source.frame_size;
    }

    /// The same track for frames `factor` times as large (tracked on a proxy, used on the
    /// original): distances scale by `factor`, perspective (per pixel) by its inverse.
    #[must_use]
    pub fn rescaled(&self, factor: f64) -> Self {
        let scale = |m: Motion| Motion {
            pan: m.pan * factor,
            tilt: m.tilt * factor,
            yaw: m.yaw / factor,
            pitch: m.pitch / factor,
            ..m
        };
        let mut out = self.clone();
        out.anchor = [self.anchor[0] * factor, self.anchor[1] * factor];
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a size of a frame"
        )]
        let size = |v: usize| (v as f64 * factor).round().max(0.0) as usize;
        out.frame_size = [size(self.frame_size[0]), size(self.frame_size[1])];
        for (c, curve) in &mut out.curves {
            let k = match c {
                Component::Pan | Component::Tilt => factor,
                Component::Yaw | Component::Pitch => 1.0 / factor,
                _ => 1.0,
            };
            for v in curve.values_mut() {
                *v *= k;
            }
        }
        for path in &mut out.points {
            for p in path.values_mut() {
                *p = [p[0] * factor, p[1] * factor];
            }
        }
        for key in out.keys.values_mut() {
            *key = scale(*key);
        }
        out
    }
}
