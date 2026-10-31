//! The cloud tracker on frames of known motion.

mod support;

use columbina_track::{
    CloudOptions, CloudTracker, Components, Direction, Luma, Mat3, Motion, Region, TrackData,
    TrackError, Tracker,
};
use proptest::prelude::*;
use support::{corners, error_at, render, similarity, track, Look, Scene};

const W: usize = 256;
const H: usize = 192;

/// The window of most tests, and its corners.
fn window() -> (Region, Vec<[f64; 2]>) {
    let c = corners(80.0, 56.0, 176.0, 136.0);
    (Region::Polygon(c.clone()), c)
}

fn frames_of(scene: &Scene, motions: &[Mat3], look: impl Fn(usize) -> Look) -> Vec<Luma> {
    motions
        .iter()
        .enumerate()
        .map(|(i, m)| render(scene, W, H, m, look(i), None))
        .collect()
}

fn cloud(perspective: bool) -> Tracker {
    Tracker::Cloud(CloudOptions {
        perspective,
        ..CloudOptions::default()
    })
}

/// Track forward from frame 0 and give the data.
fn forward(
    frames: &[Luma],
    perspective: bool,
    components: Components,
    region: &Region,
) -> TrackData {
    let anchor = region.centroid().unwrap();
    let mut data = TrackData::new(anchor, [W, H]);
    let report = track(
        frames,
        &mut data,
        cloud(perspective),
        components,
        region.clone(),
        0,
        Direction::Forward,
    );
    assert_eq!(report.frames, frames.len() - 1);
    assert_eq!(report.lost, 0);
    data
}

fn worst(data: &TrackData, truths: &[Mat3], points: &[[f64; 2]]) -> f64 {
    truths
        .iter()
        .enumerate()
        .map(|(i, t)| error_at(data, i64::try_from(i).unwrap(), t, points))
        .fold(0.0, f64::max)
}

#[test]
fn a_pan_and_tilt_are_followed_to_a_twentieth_of_a_pixel() {
    let scene = Scene::new(1, W, H);
    let truths: Vec<Mat3> = (0..12)
        .map(|k| Mat3::translation(1.7 * f64::from(k), -0.9 * f64::from(k)))
        .collect();
    let frames = frames_of(&scene, &truths, |_| Look::default());
    let (region, points) = window();
    let data = forward(&frames, false, Components::default(), &region);
    let err = worst(&data, &truths, &points);
    assert!(err < 0.05, "{err}");
    let last = data.motion_at(11);
    assert!(
        (last.pan - 18.7).abs() < 0.05 && (last.tilt + 9.9).abs() < 0.05,
        "{last:?}"
    );
    assert!(
        (last.zoom - 1.0).abs() < 1e-3 && last.rotate.abs() < 1e-3,
        "{last:?}"
    );
    assert!(data.confidence_at(11).unwrap() > 0.8);
    // The frames before the start and after the end hold the ends (p. 3232).
    assert_eq!(data.motion_at(40), last);
}

#[test]
fn a_zoom_is_followed() {
    let scene = Scene::new(2, W, H);
    let c = [130.0, 100.0];
    let truths: Vec<Mat3> = (0..12)
        .map(|k| similarity(c, 1.012_f64.powi(k), 0.0, [0.4 * f64::from(k), 0.0]))
        .collect();
    let frames = frames_of(&scene, &truths, |_| Look::default());
    let (region, points) = window();
    let data = forward(&frames, false, Components::default(), &region);
    let err = worst(&data, &truths, &points);
    assert!(err < 0.1, "{err}");
    let zoom = data.motion_at(11).zoom;
    assert!((zoom - 1.012_f64.powi(11)).abs() < 2e-3, "{zoom}");
}

#[test]
fn a_rotation_is_followed() {
    let scene = Scene::new(3, W, H);
    let step = 0.6_f64.to_radians();
    let truths: Vec<Mat3> = (0..12)
        .map(|k| similarity([128.0, 96.0], 1.0, step * f64::from(k), [0.0, 0.0]))
        .collect();
    let frames = frames_of(&scene, &truths, |_| Look::default());
    let (region, points) = window();
    let data = forward(&frames, false, Components::default(), &region);
    let err = worst(&data, &truths, &points);
    assert!(err < 0.1, "{err}");
    let turn = data.motion_at(11).rotate.to_degrees();
    assert!((turn - 6.6).abs() < 0.05, "{turn}");
}

/// A homography `share` of the way to a plane tipped away at its top and turned a little.
fn tipped(share: f64) -> Mat3 {
    let c = [128.0, 96.0];
    let m = Motion {
        pan: 6.0 * share,
        tilt: -3.0 * share,
        zoom: 1.0 + 0.05 * share,
        rotate: 0.04 * share,
        aspect: 1.0 + 0.03 * share,
        shear: 0.02 * share,
        yaw: 4e-4 * share,
        pitch: -6e-4 * share,
    };
    m.matrix(c)
}

#[test]
fn a_change_of_perspective_is_followed_with_perspective_3d() {
    let scene = Scene::new(4, W, H);
    let truths: Vec<Mat3> = (0..12).map(|k| tipped(f64::from(k) / 11.0)).collect();
    let frames = frames_of(&scene, &truths, |_| Look::default());
    let (region, points) = window();
    let data = forward(&frames, true, Components::ALL, &region);
    let err = worst(&data, &truths, &points);
    assert!(err < 0.25, "{err}");
    // Without it, the four numbers of perspective stay at no motion.
    let flat = forward(&frames, false, Components::default(), &region);
    let m = flat.motion_at(11);
    assert!(
        (m.aspect - 1.0).abs() < 1e-12
            && m.shear.abs() < 1e-12
            && m.yaw.abs() < 1e-12
            && m.pitch.abs() < 1e-12,
        "{m:?}"
    );
}

#[test]
fn noise_and_a_change_of_brightness_do_not_move_the_track() {
    let scene = Scene::new(5, W, H);
    let truths: Vec<Mat3> = (0..12)
        .map(|k| Mat3::translation(-1.3 * f64::from(k), 0.8 * f64::from(k)))
        .collect();
    let frames = frames_of(&scene, &truths, |i| {
        #[allow(clippy::cast_precision_loss, reason = "small")]
        let t = i as f64 / 11.0;
        Look {
            noise: 0.02,
            gain: 1.0 - 0.3 * t,
            offset: 0.12 * t,
            seed: 100 + i as u64,
        }
    });
    let (region, points) = window();
    let data = forward(&frames, false, Components::default(), &region);
    let err = worst(&data, &truths, &points);
    assert!(err < 0.15, "{err}");
}

#[test]
fn a_still_occluder_over_part_of_the_window_is_left_out() {
    let scene = Scene::new(6, W, H);
    let cover_scene = Scene::new(66, W, H);
    let truths: Vec<Mat3> = (0..14)
        .map(|k| Mat3::translation(1.5 * f64::from(k), 0.6 * f64::from(k)))
        .collect();
    // From frame 4, a post stands over the left third of the window, not moving.
    let frames: Vec<Luma> = truths
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let post = |p: [f64; 2]| (p[0] >= 70.0 && p[0] < 115.0).then(|| cover_scene.value(p));
            if i >= 4 {
                render(&scene, W, H, m, Look::default(), Some(&post))
            } else {
                render(&scene, W, H, m, Look::default(), None)
            }
        })
        .collect();
    let (region, points) = window();
    let data = forward(&frames, false, Components::default(), &region);
    let err = worst(&data, &truths, &points);
    assert!(err < 0.06, "{err}");
}

#[test]
fn tracking_forward_then_back_comes_home() {
    let scene = Scene::new(7, W, H);
    let truths: Vec<Mat3> = (0..11)
        .map(|k| {
            similarity(
                [128.0, 96.0],
                1.0 + 0.004 * f64::from(k),
                0.003 * f64::from(k),
                [1.1 * f64::from(k), 0.5 * f64::from(k)],
            )
        })
        .collect();
    let frames = frames_of(&scene, &truths, |_| Look::default());
    let (region, points) = window();
    let mut data = forward(&frames, false, Components::default(), &region);
    // Back from the last frame, with the window where the track put it.
    let at_end = region.transformed(&data.window_matrix(10));
    let report = track(
        &frames,
        &mut data,
        cloud(false),
        Components::default(),
        at_end,
        10,
        Direction::Reverse,
    );
    assert_eq!(report.frames, 10);
    let home = error_at(&data, 0, &Mat3::IDENTITY, &points);
    assert!(home < 0.1, "{home}");
}

#[test]
fn forward_and_back_from_the_middle_covers_the_clip() {
    let scene = Scene::new(8, W, H);
    let truths: Vec<Mat3> = (0..11)
        .map(|k| Mat3::translation(1.2 * f64::from(k), -0.7 * f64::from(k)))
        .collect();
    let frames = frames_of(&scene, &truths, |_| Look::default());
    let (_, points) = window();
    // The window placed at frame 5, where the scene has moved by truths[5].
    let placed: Vec<[f64; 2]> = points.iter().map(|&p| truths[5].apply(p)).collect();
    let region = Region::Polygon(placed.clone());
    let mut data = TrackData::new(region.centroid().unwrap(), [W, H]);
    let report = track(
        &frames,
        &mut data,
        cloud(false),
        Components::default(),
        region,
        5,
        Direction::ForwardAndBack,
    );
    assert_eq!(report.frames, 10);
    assert_eq!(data.tracked_range(), Some((0, 10)));
    let back = truths[5].inverse().unwrap();
    for (k, t) in truths.iter().enumerate() {
        let truth = t.mul(&back);
        let err = error_at(&data, i64::try_from(k).unwrap(), &truth, &placed);
        assert!(err < 0.06, "frame {k}: {err}");
    }
}

#[test]
fn one_frame_either_way_tracks_one_frame() {
    let scene = Scene::new(9, W, H);
    let truths: Vec<Mat3> = (0..6)
        .map(|k| Mat3::translation(2.0 * f64::from(k), 0.0))
        .collect();
    let frames = frames_of(&scene, &truths, |_| Look::default());
    let (region, _) = window();
    let mut data = TrackData::new(region.centroid().unwrap(), [W, H]);
    let report = track(
        &frames,
        &mut data,
        cloud(false),
        Components::default(),
        region.clone(),
        0,
        Direction::OneForward,
    );
    assert_eq!(report.frames, 1);
    assert_eq!(data.tracked_range(), Some((0, 1)));
    assert!((data.motion_at(1).pan - 2.0).abs() < 0.05);
    // One back from frame 0 is outside the clip: nothing more.
    let report = track(
        &frames,
        &mut data,
        cloud(false),
        Components::default(),
        region,
        0,
        Direction::OneReverse,
    );
    assert_eq!(report.frames, 0);
    assert_eq!(data.tracked_range(), Some((0, 1)));
}

#[test]
fn the_same_frames_give_the_same_track_to_the_bit_whatever_the_threads() {
    let scene = Scene::new(10, W, H);
    let truths: Vec<Mat3> = (0..8)
        .map(|k| {
            similarity(
                [120.0, 90.0],
                1.0 + 0.01 * f64::from(k),
                0.01 * f64::from(k),
                [f64::from(k), 0.0],
            )
        })
        .collect();
    let frames = frames_of(&scene, &truths, |i| Look {
        noise: 0.03,
        seed: i as u64,
        ..Look::default()
    });
    let (region, _) = window();
    let run_with = |threads| {
        let mut data = TrackData::new(region.centroid().unwrap(), [W, H]);
        let tracker = Tracker::Cloud(CloudOptions {
            threads,
            perspective: true,
            ..CloudOptions::default()
        });
        track(
            &frames,
            &mut data,
            tracker,
            Components::ALL,
            region.clone(),
            0,
            Direction::Forward,
        );
        data
    };
    let one = run_with(1);
    assert_eq!(one, run_with(1));
    assert_eq!(one, run_with(4));
}

#[test]
fn a_flat_window_is_refused_and_a_lost_subject_bridged() {
    let flat = Luma::new(W, H, vec![0.5; W * H]).unwrap();
    let (region, _) = window();
    assert_eq!(
        CloudTracker::new(&flat, region.clone(), CloudOptions::default()).unwrap_err(),
        TrackError::NoFeatures
    );
    assert_eq!(
        CloudTracker::new(
            &flat,
            Region::rect(-50.0, -50.0, -10.0, -10.0),
            CloudOptions::default()
        )
        .unwrap_err(),
        TrackError::EmptyRegion
    );
    // A clip whose frames 3 and 4 are blank: the subject is lost, the curves bridge them.
    let scene = Scene::new(11, W, H);
    let truths: Vec<Mat3> = (0..8)
        .map(|k| Mat3::translation(f64::from(k), 0.0))
        .collect();
    let mut frames = frames_of(&scene, &truths, |_| Look::default());
    frames[3] = flat.clone();
    frames[4] = flat;
    let mut data = TrackData::new(region.centroid().unwrap(), [W, H]);
    let report = track(
        &frames,
        &mut data,
        cloud(false),
        Components::default(),
        region,
        0,
        Direction::Forward,
    );
    // Frame 5 is lost too: the cloud is found again there, within the window where it was held,
    // and followed from it on.
    assert_eq!(report.lost, 3);
    assert!((3..=5).all(|f| data.confidence_at(f).is_none()));
    let pan = data.motion_at(7).pan;
    assert!(pan > 1.5, "{pan}");
}

#[test]
fn interactive_mode_deletes_and_inserts_features() {
    let scene = Scene::new(12, W, H);
    let first = render(&scene, W, H, &Mat3::IDENTITY, Look::default(), None);
    let (region, _) = window();
    let mut tracker = CloudTracker::new(
        &first,
        region,
        CloudOptions {
            replenish: false,
            ..CloudOptions::default()
        },
    )
    .unwrap();
    let all = tracker.features().len();
    assert!(all > 40, "{all}");
    let gone = tracker.delete_in([0.0, 0.0], [1e4, 1e4]);
    assert_eq!(gone, all);
    let added = tracker.insert_in([90.0, 60.0], [130.0, 100.0]);
    assert!(added > 3, "{added}");
    assert!(tracker
        .features()
        .iter()
        .all(|p| (90.0..=130.0).contains(&p[0]) && (60.0..=100.0).contains(&p[1])));
    let next = render(
        &scene,
        W,
        H,
        &Mat3::translation(1.0, 1.0),
        Look::default(),
        None,
    );
    let step = tracker.step(&next).unwrap();
    assert!(!step.lost);
    let moved = step
        .motion
        .distance_on(&Mat3::translation(1.0, 1.0), &[[110.0, 80.0]]);
    assert!(moved < 0.05, "{moved}");
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 12, ..ProptestConfig::default() })]

    #[test]
    fn a_small_motion_between_two_frames_is_found(
        dx in -4.0..4.0_f64, dy in -4.0..4.0_f64,
        zoom in 0.97..1.03_f64, turn in -2.0..2.0_f64,
        seed in 0..1000_u64,
    ) {
        let scene = Scene::new(seed, W, H);
        let truth = similarity([128.0, 96.0], zoom, turn.to_radians(), [dx, dy]);
        let frames = frames_of(&scene, &[Mat3::IDENTITY, truth], |_| Look::default());
        let (region, points) = window();
        let data = forward(&frames, false, Components::default(), &region);
        let err = error_at(&data, 1, &truth, &points);
        prop_assert!(err < 0.08, "{err}");
    }
}

#[test]
fn stop_ends_a_track_and_a_bad_job_is_refused() {
    use columbina_track::{run, Job};
    use std::sync::atomic::{AtomicBool, Ordering};
    let scene = Scene::new(13, W, H);
    let truths: Vec<Mat3> = (0..6)
        .map(|k| Mat3::translation(f64::from(k), 0.0))
        .collect();
    let frames = frames_of(&scene, &truths, |_| Look::default());
    let (region, _) = window();
    let job = Job {
        tracker: cloud(false),
        components: Components::default(),
        region: region.clone(),
        start: 0,
        first: 0,
        last: 5,
        direction: Direction::Forward,
    };
    let source = |f: i64| usize::try_from(f).ok().and_then(|i| frames.get(i)).cloned();
    // Stop pressed after the second frame.
    let stop = AtomicBool::new(false);
    let mut data = TrackData::new(region.centroid().unwrap(), [W, H]);
    let report = run(&job, &mut data, &mut { source }, &stop, &mut |f, _| {
        if f == 2 {
            stop.store(true, Ordering::Relaxed);
        }
    })
    .unwrap();
    assert!(report.stopped);
    assert_eq!(report.frames, 2);
    assert_eq!(data.tracked_range(), Some((0, 2)));
    stop.store(false, Ordering::Relaxed);
    let outside = Job {
        start: 9,
        ..job.clone()
    };
    assert!(matches!(
        run(&outside, &mut data, &mut { source }, &stop, &mut |_, _| {}),
        Err(TrackError::OutOfRange { start: 9, .. })
    ));
    let short = |f: i64| (f < 3).then(|| frames[usize::try_from(f).unwrap()].clone());
    assert_eq!(
        run(&job, &mut data, &mut { short }, &stop, &mut |_, _| {}),
        Err(TrackError::MissingFrame(3))
    );
    // What was tracked before the frame that could not be had stays.
    assert_eq!(data.tracked_range(), Some((0, 2)));
}
