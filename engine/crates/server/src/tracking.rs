//! A tracked window on the viewer (T24): where its controls are drawn and hit, and what a drag
//! of it does in Clip and Frame mode (Reference Manual of Resolve 20, p. 3236).
//!
//! The window's own Transform places its local shape on the picture; its track then moves it
//! (`columbina_model::grade::window::tracking`). The controls are drawn and hit through the
//! same composition, so that what is seen is what is dragged:
//!
//! - **Clip mode** (the default): a drag is read back through the track to where the window is
//!   placed by hand and changes its own Transform there, so the window moves the same way along
//!   its whole path. This is [`columbina_track::TrackData::adjust`] in Clip mode, written into
//!   the window's own parameters.
//! - **Frame mode**: a change to the whole window (Pan, Tilt, Size, Aspect, Rotate) becomes a
//!   keyframe of the tracker at the playhead instead, and the window's own Transform is kept.
//!   A change to one point of a Polygon or Curve has no keyframe of the tracker to go into (the
//!   tracker's keyframes are motions of the whole window), so it changes the shape as in Clip
//!   mode: a project choice, the manual being silent on it.
//!
//! The viewer's controls are affine. A track with Perspective 3D (yaw or pitch) is drawn by the
//! affine map that best agrees with it at the window's centre (its derivative there); the key
//! itself follows the full homography. Drawing the outline itself through the homography is
//! left for later.

use columbina_model::grade::window::tracking::{Mode, WindowTracking};
use columbina_model::grade::Window;
use columbina_track::{Adjusted, Mat3};
use columbina_view::window_overlay::Affine;

use crate::window::compose;

/// Perspective terms below this (per unit of the picture) are taken as none.
const AFFINE: f64 = 1e-12;

fn matrix_of(a: Affine) -> Mat3 {
    Mat3([[a.xx, a.xy, a.tx], [a.yx, a.yy, a.ty], [0.0, 0.0, 1.0]])
}

/// The track of `window` at clock frame `at` as an affine map of normalized picture
/// coordinates: exact where it has no perspective, otherwise its derivative at the window's
/// centre. `None` where the window is not tracked.
#[allow(
    clippy::cast_precision_loss,
    reason = "the model evaluates keyframes as f64"
)]
pub fn picture_affine(window: &Window, at: i64) -> Option<Affine> {
    let m = window.tracking.as_ref()?.picture_matrix(at as f64)?;
    if m[2][0].abs() < AFFINE && m[2][1].abs() < AFFINE {
        let w = m[2][2];
        return Some(Affine {
            xx: m[0][0] / w,
            xy: m[0][1] / w,
            yx: m[1][0] / w,
            yy: m[1][1] / w,
            tx: m[0][2] / w,
            ty: m[1][2] / w,
        });
    }
    let [_, _, _, _, cx, cy] = window.transform.affine_at(at as f64, 1.0)?;
    let z = m[2][0] * cx + m[2][1] * cy + m[2][2];
    if !(z.is_finite() && z.abs() > AFFINE) {
        return None;
    }
    let x = (m[0][0] * cx + m[0][1] * cy + m[0][2]) / z;
    let y = (m[1][0] * cx + m[1][1] * cy + m[1][2]) / z;
    let j = [
        [(m[0][0] - x * m[2][0]) / z, (m[0][1] - x * m[2][1]) / z],
        [(m[1][0] - y * m[2][0]) / z, (m[1][1] - y * m[2][1]) / z],
    ];
    let a = Affine {
        xx: j[0][0],
        xy: j[0][1],
        yx: j[1][0],
        yy: j[1][1],
        tx: x - j[0][0] * cx - j[0][1] * cy,
        ty: y - j[1][0] * cx - j[1][1] * cy,
    };
    [a.xx, a.xy, a.yx, a.yy, a.tx, a.ty]
        .iter()
        .all(|v| v.is_finite())
        .then_some(a)
}

/// `placement` (from normalized picture coordinates to the canvas) after the track of
/// `window`: what its controls are drawn, hit and dragged through.
pub fn placed(placement: Affine, window: &Window, at: i64) -> Affine {
    picture_affine(window, at).map_or(placement, |track| compose(placement, track))
}

/// From normalized picture coordinates to pixels of the tracked frames, and back.
#[allow(clippy::cast_precision_loss, reason = "a side of a frame")]
fn pixels(tracking: &WindowTracking) -> Option<(Mat3, Mat3)> {
    let [w, h] = tracking.data.frame_size;
    if w == 0 || h == 0 {
        return None;
    }
    let (w, h) = (w as f64, h as f64);
    Some((
        Mat3([[w, 0.0, -0.5], [0.0, h, -0.5], [0.0, 0.0, 1.0]]),
        Mat3([
            [1.0 / w, 0.0, 0.5 / w],
            [0.0, 1.0 / h, 0.5 / h],
            [0.0, 0.0, 1.0],
        ]),
    ))
}

/// A drag of the window `index` that gave `edited` from `base`, as Frame mode takes it: a
/// change of its own Transform at clock frame `at` becomes the tracker's keyframe there, the
/// same change of the window as it is seen at `at`, and the Transform is kept. Anything else
/// (Clip mode, an untracked window, a change of its shape) is `edited` as it is. A change the
/// tracker cannot take as a keyframe (one that turns the window over) changes nothing.
/// `aspect` is the picture's width over its height.
#[allow(
    clippy::cast_precision_loss,
    reason = "the model evaluates keyframes as f64"
)]
pub fn frame_mode(
    base: &[Window],
    mut edited: Vec<Window>,
    index: usize,
    at: i64,
    aspect: f64,
) -> Vec<Window> {
    let (Some(before), Some(after)) = (base.get(index), edited.get(index)) else {
        return edited;
    };
    let Some(tracking) = before.tracking.as_ref().filter(|t| t.mode == Mode::Frame) else {
        return edited;
    };
    if before.transform == after.transform {
        return edited;
    }
    let Some((to_pixels, to_picture)) = pixels(tracking) else {
        return edited;
    };
    let frame = at as f64;
    let affine = |w: &Window| {
        w.transform
            .affine_at(frame, aspect)
            .map(|[xx, yx, xy, yy, tx, ty]| {
                matrix_of(Affine {
                    xx,
                    xy,
                    yx,
                    yy,
                    tx,
                    ty,
                })
            })
    };
    let (Some(old), Some(new)) = (affine(before), affine(after)) else {
        return edited;
    };
    let Some(change) = old.inverse().map(|inv| new.mul(&inv)) else {
        return edited;
    };
    // The change as it is seen at `at` (after the track), in pixels of the tracked frames.
    let track = Mat3(tracking.picture_matrix(frame).unwrap_or(Mat3::IDENTITY.0));
    let Some(delta) = track.inverse().map(|inv| {
        to_pixels
            .mul(&track)
            .mul(&change)
            .mul(&inv)
            .mul(&to_picture)
            .normalized()
    }) else {
        return edited;
    };
    let mut tracking = tracking.clone();
    match tracking.data.adjust(Mode::Frame, at, &delta) {
        Adjusted::Keyed => {
            let window = &mut edited[index];
            window.transform = before.transform.clone();
            window.tracking = Some(tracking);
            edited
        }
        Adjusted::Window(_) | Adjusted::Refused => base.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use columbina_model::grade::window::tracking::{Motion, TrackData};
    use columbina_model::grade::{Param, WindowShape};

    fn tracked(mode: Mode, motion: Motion) -> Window {
        let mut w = Window::new("w1", WindowShape::circular());
        w.transform.rotate = 25.0.into();
        w.transform.aspect = 65.0.into();
        w.transform.pan = 40.0.into();
        let mut data = TrackData::new([79.5, 49.5], [160, 100]);
        for f in 0..=10 {
            data.set_sample(f, motion, 1.0);
        }
        w.tracking = Some(WindowTracking {
            mode,
            data,
            ..WindowTracking::default()
        });
        w
    }

    fn apply(a: Affine, p: [f64; 2]) -> [f64; 2] {
        [
            a.xx * p[0] + a.xy * p[1] + a.tx,
            a.yx * p[0] + a.yy * p[1] + a.ty,
        ]
    }

    fn near(a: [f64; 2], b: [f64; 2], tolerance: f64) {
        assert!(
            (a[0] - b[0]).abs() < tolerance && (a[1] - b[1]).abs() < tolerance,
            "{a:?} != {b:?}"
        );
    }

    /// Where the key puts a local point of the window at `at`: the full homography.
    fn shown(w: &Window, at: i64, aspect: f64, local: [f64; 2]) -> [f64; 2] {
        #[allow(clippy::cast_precision_loss)]
        let [xx, yx, xy, yy, tx, ty] = w.transform.affine_at(at as f64, aspect).unwrap();
        let p = apply(
            Affine {
                xx,
                xy,
                yx,
                yy,
                tx,
                ty,
            },
            local,
        );
        match w
            .tracking
            .as_ref()
            .and_then(|t| t.picture_matrix(at as f64))
        {
            Some(m) => Mat3(m).apply(p),
            None => p,
        }
    }

    #[test]
    fn an_untracked_window_is_placed_as_before() {
        let w = Window::new("w1", WindowShape::circular());
        let p = Affine {
            xx: 640.0,
            xy: 0.0,
            yx: 0.0,
            yy: 360.0,
            tx: 3.0,
            ty: 4.0,
        };
        assert_eq!(placed(p, &w, 0), p);
        assert!(picture_affine(&w, 0).is_none());
    }

    /// The controls are where the key is: the placement after the track, for a rotated window
    /// of another aspect on a frame that is not square, with a turn and a zoom tracked.
    #[test]
    fn the_controls_follow_the_track_after_the_windows_own_transform() {
        let w = tracked(
            Mode::Clip,
            Motion {
                pan: 12.0,
                tilt: -7.0,
                zoom: 1.3,
                rotate: 0.4,
                aspect: 1.2,
                shear: 0.1,
                ..Motion::IDENTITY
            },
        );
        let aspect = 1.6;
        let track = picture_affine(&w, 5).unwrap();
        let [xx, yx, xy, yy, tx, ty] = w.transform.affine_at(5.0, aspect).unwrap();
        let own = Affine {
            xx,
            xy,
            yx,
            yy,
            tx,
            ty,
        };
        for local in [[0.25, 0.0], [0.0, -0.25], [-0.1, 0.2]] {
            near(
                apply(compose(track, own), local),
                shown(&w, 5, aspect, local),
                1e-12,
            );
        }
    }

    /// Perspective 3D is drawn by its derivative at the window's centre: exact there.
    #[test]
    fn a_perspective_track_is_drawn_exactly_at_the_windows_centre() {
        let w = tracked(
            Mode::Clip,
            Motion {
                yaw: 0.002,
                pitch: -0.001,
                ..Motion::IDENTITY
            },
        );
        let track = picture_affine(&w, 3).unwrap();
        let [_, _, _, _, cx, cy] = w.transform.affine_at(3.0, 1.0).unwrap();
        near(apply(track, [cx, cy]), shown(&w, 3, 1.6, [0.0, 0.0]), 1e-12);
    }

    /// Clip mode: a drag is the window's own change; Frame mode makes it a keyframe of the
    /// tracker that shows the window at the playhead where the drag put it, the Transform kept.
    #[test]
    fn frame_mode_keys_the_tracker_where_clip_mode_changes_the_transform() {
        let aspect = 1.6;
        for mode in [Mode::Clip, Mode::Frame] {
            let w = tracked(
                mode,
                Motion {
                    pan: 12.0,
                    zoom: 1.2,
                    rotate: 0.3,
                    ..Motion::IDENTITY
                },
            );
            let base = vec![w.clone()];
            let mut moved = w.clone();
            moved.transform.pan = 47.0.into();
            moved.transform.size = 60.0.into();
            moved.transform.rotate = 40.0.into();
            let out = frame_mode(&base, vec![moved.clone()], 0, 6, aspect);
            if mode == Mode::Clip {
                assert_eq!(out, vec![moved]);
                continue;
            }
            let got = &out[0];
            assert_eq!(got.transform, w.transform, "Frame mode keeps the Transform");
            assert_eq!(
                got.tracking
                    .as_ref()
                    .unwrap()
                    .data
                    .keys()
                    .keys()
                    .copied()
                    .collect::<Vec<_>>(),
                vec![6]
            );
            // At the playhead the window is where Clip mode would have put it.
            let clip = {
                let mut c = moved.clone();
                c.tracking.as_mut().unwrap().mode = Mode::Clip;
                c
            };
            for local in [[0.0, 0.0], [0.25, 0.0], [0.0, 0.25]] {
                near(
                    shown(got, 6, aspect, local),
                    shown(&clip, 6, aspect, local),
                    1e-9,
                );
            }
        }
    }

    /// A change of a Polygon's point in Frame mode stays a change of the shape.
    #[test]
    fn frame_mode_leaves_a_change_of_shape_alone() {
        let mut w = tracked(Mode::Frame, Motion::IDENTITY);
        w.shape = WindowShape::polygon();
        let mut moved = w.clone();
        if let WindowShape::Polygon { points, .. } = &mut moved.shape {
            points[0].x = Param::Fixed(-0.3);
        }
        assert_eq!(
            frame_mode(&[w], vec![moved.clone()], 0, 2, 1.0),
            vec![moved]
        );
    }
}
