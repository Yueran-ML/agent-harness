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
//! - **Frame mode**: "changes you make to the position or shape of a window create a keyframe at
//!   the frame at the position of the playhead" (p. 3236), for rotoscoping (pp. 3250–3252). A
//!   change to the whole window (Pan, Tilt, Size, Aspect, Rotate) becomes a keyframe of the
//!   tracker there, and the window's own Transform is kept. A change to points of a Linear,
//!   Polygon or Curve window keys the coordinates it changed at the playhead, at the value shown
//!   there plus the change, and sets the tracker's keyframe there at its motion (as Add
//!   Keyframe), so that the ruler of the graph shows it. A window never tracked is keyed too
//!   (p. 3250: "you don't even have to have performed a track"): its tracker is set up on frames
//!   of the size a track of its clip would have ([`analysis_size`]), anchored at its centre at
//!   the playhead. Softness, and points added or a Curve closed, apply to the whole clip.
//!
//! The viewer's controls are affine. A track with Perspective 3D (yaw or pitch) is drawn by the
//! affine map that best agrees with it at the window's centre (its derivative there); the key
//! itself follows the full homography. Drawing the outline itself through the homography is
//! left for later.

use columbina_model::grade::window::bounds;
use columbina_model::grade::window::tracking::{Mode, TrackData, WindowTracking};
use columbina_model::grade::{Interp, Key, Param, Target, Window, WindowShape};
use columbina_model::{Project, Source, VideoFacts};
use columbina_track::{Adjusted, Mat3};
use columbina_view::window_overlay::Affine;

use crate::window::compose;

/// Perspective terms below this (per unit of the picture) are taken as none.
const AFFINE: f64 = 1e-12;

/// The most lines of the frames tracked: tracking needs the motion, not every pixel, and a
/// frame of this size is decoded and tracked many times a second on the processor. A project
/// choice.
pub const ANALYSIS_HEIGHT: u32 = 720;

/// The size of the frames a picture is tracked on, `[width, height]`: upright, at most
/// [`ANALYSIS_HEIGHT`] lines, at the picture's own aspect. Its pixels are taken as square, as
/// the engine shows every picture (the model keeps no pixel aspect).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "sides of pictures within 16384"
)]
#[must_use]
pub fn analysis_size(video: &VideoFacts) -> [usize; 2] {
    let (w0, h0) = if video.rotation % 180 != 0 {
        (video.height, video.width)
    } else {
        (video.width, video.height)
    };
    let h = h0.clamp(1, ANALYSIS_HEIGHT);
    let w = ((f64::from(w0) * f64::from(h) / f64::from(h0.max(1))).round() as u32).max(1);
    [w as usize, h as usize]
}

/// [`analysis_size`] of the moving picture that `target`, a clip's grade, shows; `None` for
/// any other grade, a title or colour, or a still, none of which is tracked.
#[must_use]
pub fn analysis_of(project: &Project, target: &Target) -> Option<[usize; 2]> {
    let Target::Clip { timeline, clip } = target else {
        return None;
    };
    let clip = project
        .timelines
        .iter()
        .find(|t| &t.id == timeline)?
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .find(|c| &c.id == clip)?;
    let Source::Media { media, .. } = &clip.source else {
        return None;
    };
    let video = project
        .media
        .iter()
        .find(|m| &m.id == media)?
        .facts
        .video
        .as_ref()
        .filter(|v| !v.still)?;
    Some(analysis_size(video))
}

/// Track data with nothing tracked for `window` on frames of `size`, anchored at its centre at
/// clock frame `at`, where a first track starts from. `None` where its transform collapses it.
#[allow(
    clippy::cast_precision_loss,
    reason = "the model evaluates keyframes as f64; sides within 16384"
)]
#[must_use]
pub fn untracked(window: &Window, at: i64, size: [usize; 2]) -> Option<TrackData> {
    let (w, h) = (size[0] as f64, size[1] as f64);
    let own = window.transform.affine_at(at as f64, w / h)?;
    Some(TrackData::new([own[4] * w - 0.5, own[5] * h - 0.5], size))
}

/// The coordinates of the points of `shape`, which Frame mode keys (pp. 3250–3252): a Linear or
/// Polygon point's, and a Curve anchor's and its handles'.
fn coordinates(shape: &mut WindowShape) -> Vec<&mut Param> {
    match shape {
        WindowShape::Circular { .. } | WindowShape::Gradient { .. } => vec![],
        WindowShape::Linear { points, .. } => points
            .iter_mut()
            .flat_map(|p| [&mut p.x, &mut p.y])
            .collect(),
        WindowShape::Polygon { points, .. } => points
            .iter_mut()
            .flat_map(|p| [&mut p.x, &mut p.y])
            .collect(),
        WindowShape::Curve { points, .. } => points
            .iter_mut()
            .flat_map(|p| {
                [
                    &mut p.position.x,
                    &mut p.position.y,
                    &mut p.into.x,
                    &mut p.into.y,
                    &mut p.out.x,
                    &mut p.out.y,
                ]
            })
            .collect(),
    }
}

/// `param` with a key at `frame` of `value`: the key there changed, or one put in, on a line to
/// the next, as the tracker's keyframes go. A value without keys becomes that one key.
fn keyed(param: &Param, frame: i64, value: f64) -> Param {
    let key = Key {
        frame,
        value,
        interp: Interp::Linear,
    };
    match param {
        Param::Fixed(_) => Param::Keys(vec![key]),
        Param::Keys(keys) => {
            let mut keys = keys.clone();
            match keys.binary_search_by_key(&frame, |k| k.frame) {
                Ok(i) => keys[i].value = value,
                Err(i) => keys.insert(i, key),
            }
            Param::Keys(keys)
        }
    }
}

/// Key every coordinate of `window`'s points at clock frame `at` at its value there (Add
/// Keyframe in Frame mode, p. 3251), so that a later change is interpolated from there.
#[allow(
    clippy::cast_precision_loss,
    reason = "the model evaluates keyframes as f64"
)]
pub fn key_points(window: &mut Window, at: i64) {
    for param in coordinates(&mut window.shape) {
        *param = keyed(param, at, param.value_at(at as f64));
    }
}

/// Take the keys at clock frame `at` off every coordinate of `window`'s points (Delete
/// Keyframe, p. 3251); a coordinate left without keys keeps the value it had there.
pub fn unkey_points(window: &mut Window, at: i64) {
    for param in coordinates(&mut window.shape) {
        if let Param::Keys(keys) = param {
            if let [only] = keys.as_slice() {
                if only.frame == at {
                    *param = Param::Fixed(only.value);
                }
            } else {
                keys.retain(|k| k.frame != at);
            }
        }
    }
}

/// The shape `after`, a drag of points of `before` that wrote their left keys (as every drag
/// does), made keys at clock frame `at` instead: each coordinate it changed keyed there, at its
/// value shown there plus the change. `None` where it is not a change of values alone, which
/// then applies to the whole clip.
#[allow(
    clippy::cast_precision_loss,
    reason = "the model evaluates keyframes as f64"
)]
fn key_shape(before: &WindowShape, after: &WindowShape, at: i64) -> Option<WindowShape> {
    let mut old_shape = before.clone();
    let olds: Vec<Param> = coordinates(&mut old_shape)
        .into_iter()
        .map(|p| p.clone())
        .collect();
    let mut out = after.clone();
    let news = coordinates(&mut out);
    if olds.len() != news.len() {
        return None;
    }
    for (old, new) in olds.iter().zip(news) {
        if old == &*new {
            continue;
        }
        let change: f64 = match (old, &*new) {
            (Param::Fixed(a), Param::Fixed(b)) => b - a,
            (Param::Keys(a), Param::Keys(b))
                if a.len() == b.len()
                    && a.iter()
                        .zip(b)
                        .all(|(x, y)| x.frame == y.frame && x.interp == y.interp) =>
            {
                a.iter().zip(b).map(|(x, y)| y.value - x.value).sum()
            }
            _ => return None,
        };
        let value = (old.value_at(at as f64) + change)
            .clamp(bounds::COORDINATE.low, bounds::COORDINATE.high);
        *new = keyed(old, at, value);
    }
    Some(out)
}

fn matrix_of(a: Affine) -> Mat3 {
    Mat3([[a.xx, a.xy, a.tx], [a.yx, a.yy, a.ty], [0.0, 0.0, 1.0]])
}

/// The track of `window` at clock frame `at` as an affine map of normalized picture
/// coordinates: exact where it has no perspective, otherwise its derivative at the window's
/// centre. `None` where the window is not tracked.
#[allow(
    clippy::cast_precision_loss,
    clippy::many_single_char_names,
    reason = "the model evaluates keyframes as f64; the letters of the formulas"
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

/// A drag of the window `index` that gave `edited` from `base`, as Frame mode takes it (see the
/// module): at clock frame `at`, a change of its own Transform becomes the tracker's keyframe
/// there, the same change of the window as it is seen at `at`, and the Transform is kept; a
/// change of its points' coordinates is keyed there, with the tracker's keyframe there at its
/// motion. A window never tracked is set up on frames of `analysis` first (see [`untracked`]).
/// Anything else (Clip mode, a window not tracked with no `analysis`, softness) is `edited` as
/// it is. A change the tracker cannot take as a keyframe (one that turns the window over)
/// changes nothing. `aspect` is the picture's width over its height.
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
    analysis: Option<[usize; 2]>,
) -> Vec<Window> {
    let (Some(before), Some(after)) = (base.get(index), edited.get(index)) else {
        return edited;
    };
    let Some(tracking) = before.tracking.as_ref().filter(|t| t.mode == Mode::Frame) else {
        return edited;
    };
    let moved = before.transform != after.transform;
    let shape = (before.shape != after.shape)
        .then(|| key_shape(&before.shape, &after.shape, at))
        .flatten();
    if !moved && shape.is_none() {
        return edited;
    }
    let mut tracking = tracking.clone();
    if pixels(&tracking).is_none() {
        let Some(data) = analysis.and_then(|size| untracked(before, at, size)) else {
            return edited;
        };
        tracking.data = data;
    }
    let mut window = after.clone();
    if let Some(shape) = shape {
        window.shape = shape;
    }
    if !moved {
        let here = tracking.data.key_at(at);
        tracking.data.set_key(at, here);
        window.tracking = Some(tracking);
        edited[index] = window;
        return edited;
    }
    let Some((to_pixels, to_picture)) = pixels(&tracking) else {
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
    match tracking.data.adjust(Mode::Frame, at, &delta) {
        Adjusted::Keyed => {
            window.transform = before.transform.clone();
            window.tracking = Some(tracking);
            edited[index] = window;
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
    #[allow(clippy::cast_precision_loss)]
    fn shown(w: &Window, at: i64, aspect: f64, local: [f64; 2]) -> [f64; 2] {
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
            let out = frame_mode(&base, vec![moved.clone()], 0, 6, aspect, None);
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

    fn polygon_x(w: &Window, i: usize) -> &Param {
        match &w.shape {
            WindowShape::Polygon { points, .. } => &points[i].x,
            _ => panic!("a polygon"),
        }
    }

    fn polygon_mut(w: &mut Window) -> &mut Vec<columbina_model::grade::WindowPoint> {
        match &mut w.shape {
            WindowShape::Polygon { points, .. } => points,
            _ => panic!("a polygon"),
        }
    }

    fn line(keys: &[(i64, f64)]) -> Param {
        Param::Keys(
            keys.iter()
                .map(|&(frame, value)| Key {
                    frame,
                    value,
                    interp: Interp::Linear,
                })
                .collect(),
        )
    }

    /// Frame mode keys a change of points at the playhead, at the value shown there plus the
    /// change, whatever key the drag wrote, and puts the tracker's keyframe there (p. 3236).
    #[test]
    fn frame_mode_keys_a_change_of_points_at_the_playhead() {
        let mut w = tracked(Mode::Frame, Motion::IDENTITY);
        w.shape = WindowShape::polygon();
        polygon_mut(&mut w)[0].x = line(&[(0, -0.2), (10, -0.4)]);
        let y1 = polygon_mut(&mut w)[1].y.clone();
        let mut moved = w.clone();
        // The drag at frame 4 wrote the left key (frame 0) and the fixed y of point 1.
        polygon_mut(&mut moved)[0].x = line(&[(0, -0.1), (10, -0.4)]);
        polygon_mut(&mut moved)[1].y = Param::Fixed(y1.value_at(0.0) + 0.05);
        let out = frame_mode(&[w.clone()], vec![moved], 0, 4, 1.0, None);
        let got = &out[0];
        let x0 = polygon_x(got, 0);
        // Shown at frame 4 on the line from -0.2 to -0.4, moved by 0.1.
        let shown_x = -0.2 + (-0.4 - -0.2) * 0.4;
        assert_eq!(
            x0,
            &line(&[(0, -0.2), (4, shown_x + ((-0.1 - -0.2) + 0.0)), (10, -0.4)])
        );
        assert_eq!(
            polygon_mut(&mut got.clone())[1].y,
            line(&[(4, y1.value_at(0.0) + 0.05)])
        );
        // Every other coordinate is as it was.
        assert_eq!(
            polygon_mut(&mut got.clone())[2],
            polygon_mut(&mut w.clone())[2]
        );
        assert_eq!(got.transform, w.transform);
        let data = &got.tracking.as_ref().unwrap().data;
        assert_eq!(data.keys().keys().copied().collect::<Vec<_>>(), vec![4]);
        assert_eq!(data.key_at(4), Motion::IDENTITY);
        // Clip mode keeps the drag as it was written.
        let mut clip = w.clone();
        clip.tracking.as_mut().unwrap().mode = Mode::Clip;
        let mut moved = clip.clone();
        polygon_mut(&mut moved)[0].x = line(&[(0, -0.1), (10, -0.4)]);
        assert_eq!(
            frame_mode(&[clip], vec![moved.clone()], 0, 4, 1.0, None),
            vec![moved]
        );
    }

    /// A point added changes the shape of the whole clip: nothing to key.
    #[test]
    fn frame_mode_leaves_a_point_added_to_the_whole_clip() {
        let mut w = tracked(Mode::Frame, Motion::IDENTITY);
        w.shape = WindowShape::polygon();
        let mut moved = w.clone();
        let extra = polygon_mut(&mut moved)[0].clone();
        polygon_mut(&mut moved).push(extra);
        assert_eq!(
            frame_mode(&[w], vec![moved.clone()], 0, 2, 1.0, None),
            vec![moved]
        );
    }

    /// A window never tracked is keyed in Frame mode on frames of its clip's analysis size,
    /// anchored at its centre at the playhead (p. 3250); without one it is changed as in Clip.
    #[test]
    fn frame_mode_keys_a_window_never_tracked_on_frames_of_its_clip() {
        let aspect = 16.0 / 9.0;
        let mut w = Window::new("w1", WindowShape::circular());
        w.transform.pan = 40.0.into();
        w.transform.rotate = 20.0.into();
        w.tracking = Some(WindowTracking {
            mode: Mode::Frame,
            ..WindowTracking::default()
        });
        let mut moved = w.clone();
        moved.transform.pan = 47.0.into();
        moved.transform.size = 60.0.into();
        assert_eq!(
            frame_mode(&[w.clone()], vec![moved.clone()], 0, 6, aspect, None),
            vec![moved.clone()]
        );
        let out = frame_mode(
            &[w.clone()],
            vec![moved.clone()],
            0,
            6,
            aspect,
            Some([1280, 720]),
        );
        let got = &out[0];
        assert_eq!(got.transform, w.transform, "Frame mode keeps the Transform");
        let data = &got.tracking.as_ref().unwrap().data;
        assert_eq!(data.frame_size, [1280, 720]);
        let own = w.transform.affine_at(6.0, aspect).unwrap();
        assert_eq!(data.anchor, [own[4] * 1280.0 - 0.5, own[5] * 720.0 - 0.5]);
        assert_eq!(data.keys().keys().copied().collect::<Vec<_>>(), vec![6]);
        let mut clip = moved.clone();
        clip.tracking = None;
        for local in [[0.0, 0.0], [0.25, 0.0], [0.0, 0.25]] {
            near(
                shown(got, 6, aspect, local),
                shown(&clip, 6, aspect, local),
                1e-9,
            );
        }
        // A change of points of a window never tracked is keyed too.
        w.shape = WindowShape::polygon();
        let mut moved = w.clone();
        let x = polygon_x(&w, 0).value_at(0.0);
        polygon_mut(&mut moved)[0].x = Param::Fixed(x + 0.1);
        let out = frame_mode(&[w], vec![moved], 0, 3, aspect, Some([1280, 720]));
        assert_eq!(polygon_x(&out[0], 0), &line(&[(3, x + 0.1)]));
        let data = &out[0].tracking.as_ref().unwrap().data;
        assert_eq!(data.frame_size, [1280, 720]);
        assert_eq!(data.keys().keys().copied().collect::<Vec<_>>(), vec![3]);
    }

    /// Add Keyframe keys every coordinate of the points at its value there; Delete Keyframe takes
    /// those keys away again, a coordinate left without keys keeping its value.
    #[test]
    fn keyframes_of_frame_mode_key_and_unkey_the_points() {
        let mut w = Window::new("w1", WindowShape::polygon());
        polygon_mut(&mut w)[0].x = line(&[(0, -0.2), (10, -0.4)]);
        let before = w.clone();
        key_points(&mut w, 5);
        assert_eq!(
            polygon_x(&w, 0),
            &line(&[(0, -0.2), (5, -0.2 + (-0.4 - -0.2) * 0.5), (10, -0.4)])
        );
        let y = polygon_mut(&mut before.clone())[0].y.value_at(0.0);
        assert_eq!(polygon_mut(&mut w)[0].y, line(&[(5, y)]));
        unkey_points(&mut w, 5);
        assert_eq!(w, before);
        // A circle has no points to key.
        let mut c = Window::new("w2", WindowShape::circular());
        key_points(&mut c, 5);
        assert_eq!(c, Window::new("w2", WindowShape::circular()));
    }

    #[test]
    fn the_analysis_frame_is_upright_and_at_most_720_lines_at_the_pictures_aspect() {
        let video = |width, height, rotation| VideoFacts {
            still: false,
            width,
            height,
            rate: columbina_model::Rate { num: 25, den: 1 },
            codec: String::new(),
            bits: 8,
            rotation,
            color: None,
        };
        assert_eq!(analysis_size(&video(1920, 1080, 0)), [1280, 720]);
        assert_eq!(analysis_size(&video(1920, 1080, 90)), [405, 720]);
        assert_eq!(analysis_size(&video(640, 480, 0)), [640, 480]);
        let project = Project::sample();
        let clip = Target::Clip {
            timeline: "t1".into(),
            clip: "c1".into(),
        };
        assert_eq!(analysis_of(&project, &clip), Some([1280, 720]));
        let timeline = Target::Timeline {
            timeline: "t1".into(),
        };
        assert_eq!(analysis_of(&project, &timeline), None);
    }
}
