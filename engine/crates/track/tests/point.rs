//! The point tracker on blobs of known motion.

mod support;

use columbina_track::{
    Components, Direction, Mat3, Motion, PointOptions, PointTracker, Region, TrackData, TrackError,
    Tracker,
};
use support::{render_with, similarity, track, Look};

const W: usize = 256;
const H: usize = 192;

/// A dim, faintly textured ground with bright round blobs at `blobs`.
fn blobs(blobs: &[[f64; 2]]) -> impl Fn([f64; 2]) -> f64 + '_ {
    move |p: [f64; 2]| {
        let ground = 0.25 + 0.02 * (p[0] * 0.31).sin() * (p[1] * 0.27).cos();
        blobs.iter().fold(ground, |v, b| {
            let d2 = (p[0] - b[0]).powi(2) + (p[1] - b[1]).powi(2);
            v + 0.6 * (-d2 / (2.0 * 4.0 * 4.0)).exp()
        })
    }
}

fn point(points: Vec<[f64; 2]>, perspective: bool) -> Tracker {
    Tracker::Point {
        points,
        options: PointOptions {
            perspective,
            ..PointOptions::default()
        },
    }
}

#[test]
fn a_moving_blob_is_followed_by_one_crosshair() {
    let at = [[100.0, 80.0]];
    let scene = blobs(&at);
    // Along a curve.
    let path: Vec<[f64; 2]> = (0..15)
        .map(|k| {
            let k = f64::from(k);
            [3.0 * k + 2.0 * (k / 2.0).sin(), 1.5 * k]
        })
        .collect();
    let frames: Vec<_> = path
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let look = Look {
                noise: 0.01,
                seed: i as u64,
                ..Look::default()
            };
            render_with(&scene, W, H, &Mat3::translation(d[0], d[1]), look, None)
        })
        .collect();
    let mut data = TrackData::new(at[0], [W, H]);
    let report = track(
        &frames,
        &mut data,
        point(at.to_vec(), false),
        Components::default(),
        Region::rect(90.0, 70.0, 110.0, 90.0),
        0,
        Direction::Forward,
    );
    assert_eq!((report.frames, report.lost), (14, 0));
    for (k, d) in path.iter().enumerate() {
        let f = i64::try_from(k).unwrap();
        let p = data.points()[0][&f];
        let err = (p[0] - at[0][0] - d[0]).hypot(p[1] - at[0][1] - d[1]);
        assert!(err < 0.05, "frame {k}: {err}");
        let m = data.motion_at(f);
        assert!(
            (m.pan - d[0]).abs() < 0.05 && (m.tilt - d[1]).abs() < 0.05,
            "{m:?}"
        );
        assert!((m.zoom - 1.0).abs() < 1e-12 && m.rotate.abs() < 1e-12);
    }
}

/// The motion at frame 9 of crosshairs on blobs at `at`, while the scene zooms by 1 % and turns
/// by 0.01 rad a frame about `c`, and the motion it should be.
fn zoomed_and_turned(at: &[[f64; 2]], c: [f64; 2]) -> (Motion, Motion) {
    let scene = blobs(at);
    let truths: Vec<Mat3> = (0..10)
        .map(|k| {
            similarity(
                c,
                1.0 + 0.01 * f64::from(k),
                0.01 * f64::from(k),
                [0.8 * f64::from(k), 0.0],
            )
        })
        .collect();
    let frames: Vec<_> = truths
        .iter()
        .map(|m| render_with(&scene, W, H, m, Look::default(), None))
        .collect();
    let mut data = TrackData::new(c, [W, H]);
    track(
        &frames,
        &mut data,
        point(at.to_vec(), false),
        Components::default(),
        Region::rect(80.0, 60.0, 180.0, 130.0),
        0,
        Direction::Forward,
    );
    (
        data.motion_at(9),
        Motion::from_matrix(&truths[9], c).unwrap(),
    )
}

#[test]
fn two_crosshairs_track_rotation_and_three_zoom() {
    // "At least two tracking points [...] to track rotation, and at least three to track zoom"
    // (p. 3237).
    let c = [130.0, 95.0];
    let (m, want) = zoomed_and_turned(&[[90.0, 70.0], [170.0, 120.0]], c);
    assert!((m.rotate - want.rotate).abs() < 1e-3, "{m:?} {want:?}");
    assert!((m.zoom - 1.0).abs() < 1e-12, "{m:?}");
    // The anchor halfway between them still goes where it should.
    assert!(
        (m.pan - want.pan).abs() < 0.1 && (m.tilt - want.tilt).abs() < 0.1,
        "{m:?} {want:?}"
    );
    let (m, want) = zoomed_and_turned(&[[90.0, 70.0], [170.0, 120.0], [100.0, 125.0]], c);
    assert!((m.zoom - want.zoom).abs() < 1e-3, "{m:?} {want:?}");
    assert!((m.rotate - want.rotate).abs() < 1e-3, "{m:?} {want:?}");
    assert!(
        (m.pan - want.pan).abs() < 0.1 && (m.tilt - want.tilt).abs() < 0.1,
        "{m:?} {want:?}"
    );
}

#[test]
fn four_crosshairs_give_a_perspective_and_a_lost_one_is_left_out() {
    let at = [[80.0, 60.0], [180.0, 64.0], [176.0, 136.0], [84.0, 130.0]];
    let scene = blobs(&at);
    let c = [130.0, 97.0];
    let truths: Vec<Mat3> = (0..10)
        .map(|k| {
            let s = f64::from(k) / 9.0;
            Motion {
                pan: 5.0 * s,
                zoom: 1.0 + 0.04 * s,
                yaw: 5e-4 * s,
                pitch: -4e-4 * s,
                ..Motion::IDENTITY
            }
            .matrix(c)
        })
        .collect();
    let frames: Vec<_> = truths
        .iter()
        .map(|m| render_with(&scene, W, H, m, Look::default(), None))
        .collect();
    let mut data = TrackData::new(c, [W, H]);
    track(
        &frames,
        &mut data,
        point(at.to_vec(), true),
        Components::ALL,
        Region::rect(70.0, 50.0, 190.0, 140.0),
        0,
        Direction::Forward,
    );
    let err = data.track_matrix(9).distance_on(&truths[9], &at);
    assert!(err < 0.1, "{err}");

    // A fifth crosshair on a flat patch cannot be tracked: it is lost at once, and the four
    // others still fit the motion.
    let flat = |p: [f64; 2]| (p[0] < 70.0 && p[1] > 140.0).then_some(0.25);
    let covered: Vec<_> = truths[..2]
        .iter()
        .map(|m| render_with(&scene, W, H, m, Look::default(), Some(&flat)))
        .collect();
    let mut five = at.to_vec();
    five.push([30.0, 170.0]);
    let options = PointOptions {
        perspective: true,
        ..PointOptions::default()
    };
    let mut tracker = PointTracker::new(&covered[0], &five, options).unwrap();
    let step = tracker.step(&covered[1]).unwrap();
    assert!(step.points[4].is_none());
    assert_eq!(step.step.features, 4);
    assert!(step.step.motion.distance_on(&truths[1], &at) < 0.1);
}

#[test]
fn crosshairs_outside_the_frame_are_refused() {
    let scene = blobs(&[]);
    let frame = render_with(&scene, W, H, &Mat3::IDENTITY, Look::default(), None);
    assert_eq!(
        PointTracker::new(&frame, &[], PointOptions::default()).unwrap_err(),
        TrackError::NoPoints
    );
    assert_eq!(
        PointTracker::new(&frame, &[[-1.0, 5.0]], PointOptions::default()).unwrap_err(),
        TrackError::NoPoints
    );
}

#[test]
fn a_crosshair_moved_to_another_feature_carries_on_the_same_path() {
    // Frame mode, p. 3249: the first feature is tracked to frame 5, the crosshair is then put on
    // another one, and the track from there continues the motion of the first.
    let at = [[60.0, 90.0], [150.0, 100.0]];
    let scene = blobs(&at);
    let shift = |k: i32| [4.0 * f64::from(k), 0.5 * f64::from(k)];
    let frames: Vec<_> = (0..10)
        .map(|k| {
            let d = shift(k);
            render_with(
                &scene,
                W,
                H,
                &Mat3::translation(d[0], d[1]),
                Look::default(),
                None,
            )
        })
        .collect();
    let mut data = TrackData::new([60.0, 90.0], [W, H]);
    let window = Region::rect(50.0, 80.0, 70.0, 100.0);
    track(
        &frames[..6],
        &mut data,
        point(vec![at[0]], false),
        Components::default(),
        window.clone(),
        0,
        Direction::Forward,
    );
    assert!((data.motion_at(5).pan - 20.0).abs() < 0.05);
    let d5 = shift(5);
    let moved = [at[1][0] + d5[0], at[1][1] + d5[1]];
    track(
        &frames,
        &mut data,
        point(vec![moved], false),
        Components::default(),
        window,
        5,
        Direction::Forward,
    );
    let m = data.motion_at(9);
    assert!(
        (m.pan - 36.0).abs() < 0.05 && (m.tilt - 4.5).abs() < 0.05,
        "{m:?}"
    );
    assert!((data.motion_at(3).pan - 12.0).abs() < 0.05);
}
