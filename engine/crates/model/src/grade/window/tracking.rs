//! The tracking of a Power Window (T24; Reference Manual of Resolve 20, chapter 140,
//! pp. 3229–3255), kept apart from the window's own Transform and its keyframes (p. 3232).
//!
//! # How a tracked window is placed
//!
//! A point `x` of the window's local shape is put on the picture, in normalized picture
//! coordinates (`u` right, `v` down, 0 to 1), as
//!
//! ```text
//! picture(x, f) = S · K(f) · T(f) · S⁻¹ · A(f) · x
//! ```
//!
//! - `A(f)`: the window's own Transform (Size, Aspect, Pan, Tilt, Rotate), with its own
//!   keyframes, [`super::WindowTransform::affine_at`]: what the Window palette shows and edits.
//! - `T(f)`: the tracked motion, in pixels of the frames that were tracked
//!   ([`TrackData::track_matrix`]); `K(f)`: the keyframes of Frame mode about the tracked anchor
//!   ([`TrackData::window_matrix`] is `K·T`). Both are homographies (Perspective 3D, p. 3235).
//! - `S`: from those pixels to normalized coordinates, the centre of pixel `(i, j)` of a
//!   `W × H` frame being `((i + 0.5) / W, (j + 0.5) / H)`. The frames are tracked upright and at
//!   the picture's own aspect, so `S` does not depend on the size the picture is shown at.
//!
//! So the track moves the window as it is placed by hand, on every frame: a change made to the
//! Transform in Clip mode moves the window along its whole path (p. 3236), and a change made in
//! Frame mode is a keyframe of `K` (pp. 3236, 3251). This order (own transform first, then the
//! track, then Frame mode) is that of `columbina_track` and of the manual's account; Resolve
//! does not publish its formula.

use serde::{Deserialize, Serialize};

pub use columbina_track::{Component, Mode, Motion, TrackData};

use super::bounds;
use crate::ops::LARGEST;
use crate::OpError;

/// The most crosshairs of the point tracker a window keeps. The manual sets no limit
/// (p. 3238: "as many as you like"); this one keeps a project to a size that can be read.
pub const MOST_CROSSHAIRS: usize = 64;

/// The largest side of the frames that a window can have been tracked on.
pub const LARGEST_FRAME: usize = 16384;

/// The tracking of one window.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowTracking {
    /// How a change made by hand to the tracked window is applied: along the whole track (Clip,
    /// the default) or as a keyframe of the tracker at the playhead (Frame) (p. 3236). Kept with
    /// the window so that the engine applies a drag in the viewer as the palette shows it.
    #[serde(default)]
    pub mode: Mode,
    /// The crosshairs of the point tracker at the frame its next track starts from, in
    /// normalized picture coordinates (p. 3238). A new crosshair is put at the centre of the
    /// frame.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub crosshairs: Vec<[f64; 2]>,
    /// The curves, confidence, paths and keyframes of Frame mode.
    pub data: TrackData,
}

/// The frames of `samples` within the bounds of the model's frames.
fn frames_fit<T>(samples: &std::collections::BTreeMap<i64, T>) -> bool {
    samples.keys().all(|f| (-LARGEST..=LARGEST).contains(f))
}

/// A frame of the grade's clock as the track counts it: whole, within the model's frames.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "a finite frame, clamped within 2^53"
)]
fn whole(frame: f64) -> i64 {
    frame.round().clamp(-LARGEST as f64, LARGEST as f64) as i64
}

fn motion_fits(m: &Motion) -> bool {
    Component::ALL.iter().all(|&c| m.get(c).is_finite()) && m.zoom > 0.0 && m.aspect > 0.0
}

impl WindowTracking {
    /// Whether it moves the window at all: it has a tracked sample or a keyframe of Frame mode.
    #[must_use]
    pub fn moves(&self) -> bool {
        !self.data.is_empty() || !self.data.keys().is_empty()
    }

    /// The homography from where the window is placed by hand to where it is shown at `frame`
    /// of the grade's clock, in normalized picture coordinates (row-major, `[x, y, 1]` as a
    /// column). `None` where it does not move the window, so that an untracked window is placed
    /// exactly as before tracking existed. Frames are whole: `frame` is rounded.
    #[must_use]
    pub fn picture_matrix(&self, frame: f64) -> Option<[[f64; 3]; 3]> {
        if !self.moves() || !frame.is_finite() {
            return None;
        }
        let [w, h] = self.data.frame_size;
        if w == 0 || h == 0 {
            return None;
        }
        let at = whole(frame);
        #[allow(clippy::cast_precision_loss, reason = "a side of a frame")]
        let (w, h) = (w as f64, h as f64);
        let s = columbina_track::Mat3([
            [1.0 / w, 0.0, 0.5 / w],
            [0.0, 1.0 / h, 0.5 / h],
            [0.0, 0.0, 1.0],
        ]);
        let s_inv = columbina_track::Mat3([[w, 0.0, -0.5], [0.0, h, -0.5], [0.0, 0.0, 1.0]]);
        let m = s.mul(&self.data.window_matrix(at)).mul(&s_inv).normalized();
        m.0.iter().flatten().all(|v| v.is_finite()).then_some(m.0)
    }

    /// The inverse of [`Self::picture_matrix`], from where the window is shown back to where it
    /// is placed by hand, signed so that its third coordinate is positive on the side of the
    /// plane the window is on: a point of the picture where it is not positive is beyond the
    /// vanishing line of a Perspective 3D track, and none of the window is there.
    #[must_use]
    pub fn picture_inverse(&self, frame: f64) -> Option<[[f64; 3]; 3]> {
        let m = columbina_track::Mat3(self.picture_matrix(frame)?);
        let mut inv = m.inverse()?;
        let [w, h] = self.data.frame_size;
        #[allow(clippy::cast_precision_loss, reason = "a side of a frame")]
        let anchor = [
            (self.data.anchor[0] + 0.5) / w as f64,
            (self.data.anchor[1] + 0.5) / h as f64,
        ];
        let shown = m.apply(anchor);
        let r = inv.0[2];
        if r[0] * shown[0] + r[1] * shown[1] + r[2] < 0.0 {
            inv = inv.scaled(-1.0);
        }
        inv.0
            .iter()
            .flatten()
            .all(|v| v.is_finite())
            .then_some(inv.0)
    }

    /// The largest scale of the window by its track and keyframes at `frame`, at least 1: so
    /// that the Curve's tessellation keeps its error once the window is zoomed by the track.
    #[must_use]
    pub fn scale_at(&self, frame: f64) -> f64 {
        if !self.moves() || !frame.is_finite() {
            return 1.0;
        }
        let at = whole(frame);
        let (t, k) = (self.data.motion_at(at), self.data.key_at(at));
        let stretch = |m: &Motion| m.zoom * m.aspect.max(1.0 / m.aspect) * (1.0 + m.shear.abs());
        let scale = stretch(&t) * stretch(&k);
        if scale.is_finite() {
            scale.max(1.0)
        } else {
            1.0
        }
    }

    /// Check what a project file or a client gave: every number finite, sizes and frames within
    /// bounds, confidence from 0 to 1, zooms and aspects positive.
    ///
    /// # Errors
    /// [`OpError::OutOfRange`] naming the part that is not.
    pub fn check(&self) -> Result<(), OpError> {
        let bad = |what| Err(OpError::OutOfRange { what });
        let d = &self.data;
        let [w, h] = d.frame_size;
        if !d.anchor.iter().all(|v| v.is_finite()) {
            return bad("windows.tracking.anchor");
        }
        if w > LARGEST_FRAME || h > LARGEST_FRAME || (self.moves() && (w == 0 || h == 0)) {
            return bad("windows.tracking.frame_size");
        }
        for c in Component::ALL {
            if let Some(curve) = d.curve(c) {
                let positive = matches!(c, Component::Zoom | Component::Aspect);
                if !frames_fit(curve)
                    || !curve
                        .values()
                        .all(|v| v.is_finite() && (!positive || *v > 0.0))
                {
                    return bad("windows.tracking.curves");
                }
            }
        }
        if !frames_fit(d.confidence())
            || !d
                .confidence()
                .values()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
        {
            return bad("windows.tracking.confidence");
        }
        if d.points().len() > MOST_CROSSHAIRS
            || !d
                .points()
                .iter()
                .all(|p| frames_fit(p) && p.values().flatten().all(|v| v.is_finite()))
        {
            return bad("windows.tracking.points");
        }
        if !frames_fit(d.keys()) || !d.keys().values().all(motion_fits) {
            return bad("windows.tracking.keys");
        }
        let range = bounds::COORDINATE.low..=bounds::COORDINATE.high;
        if self.crosshairs.len() > MOST_CROSSHAIRS
            || !self.crosshairs.iter().flatten().all(|v| range.contains(v))
        {
            return bad("windows.tracking.crosshairs");
        }
        Ok(())
    }
}
