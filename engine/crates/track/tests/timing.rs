//! How long the trackers take on frames of 1080p (ignored; run in release with `--ignored
//! --nocapture`).

mod support;

use std::time::Instant;

use columbina_track::{
    CloudOptions, CloudTracker, Luma, PointOptions, PointTracker, Pyramid, Region,
};
use support::{render, similarity, Look, Scene};

const W: usize = 1920;
const H: usize = 1080;

fn frames(n: i32) -> Vec<Luma> {
    let scene = Scene::new(42, W, H);
    (0..n)
        .map(|k| {
            let m = similarity(
                [960.0, 540.0],
                1.0 + 0.002 * f64::from(k),
                0.002 * f64::from(k),
                [6.0 * f64::from(k), -3.0 * f64::from(k)],
            );
            let look = Look {
                noise: 0.01,
                seed: u64::from(k.unsigned_abs()),
                ..Look::default()
            };
            render(&scene, W, H, &m, look, None)
        })
        .collect()
}

#[test]
#[ignore = "a measure of time, for release builds"]
fn how_long_a_frame_of_1080p_takes() {
    let frames = frames(31);
    let region = Region::rect(660.0, 290.0, 1260.0, 790.0);
    for threads in [1, 8] {
        let owned: Vec<Luma> = frames[..10].to_vec();
        let clock = Instant::now();
        for f in owned {
            std::hint::black_box(Pyramid::on_threads(f, 5, threads));
        }
        let pyramid = clock.elapsed().as_secs_f64() * 1e3 / 10.0;
        println!("pyramid of 5 levels, {threads} threads: {pyramid:.2} ms");
    }
    for threads in [1, 0] {
        let options = CloudOptions {
            threads,
            max_features: 100,
            ..CloudOptions::default()
        };
        let clock = Instant::now();
        let mut tracker = CloudTracker::new(&frames[0], region.clone(), options).unwrap();
        let start = clock.elapsed().as_secs_f64() * 1e3;
        let found = tracker.features().len();
        // The frames as the server hands them over, each its own plane.
        let owned: Vec<Luma> = frames[1..].to_vec();
        let clock = Instant::now();
        let mut lost = 0;
        for f in owned {
            lost += usize::from(tracker.step_owned(f).unwrap().lost);
        }
        let per = clock.elapsed().as_secs_f64() * 1e3 / 30.0;
        let truth = similarity([960.0, 540.0], 1.06, 0.06, [180.0, -90.0]);
        let err = tracker
            .motion()
            .distance_on(&truth, &[[660.0, 290.0], [1260.0, 790.0]]);
        println!(
            "cloud, {} threads: {found} features found in {start:.1} ms; {per:.2} ms a frame; lost {lost}; error after 30 frames {err:.3} px",
            if threads == 0 { "all (up to 8)".to_owned() } else { threads.to_string() }
        );
        assert_eq!(lost, 0);
        assert!(err < 0.3);
    }
    let points = [
        [700.0, 330.0],
        [1200.0, 340.0],
        [1210.0, 760.0],
        [690.0, 750.0],
    ];
    for threads in [1, 0] {
        let options = PointOptions {
            threads,
            ..PointOptions::default()
        };
        let mut tracker = PointTracker::new(&frames[0], &points, options).unwrap();
        let owned: Vec<Luma> = frames[1..].to_vec();
        let clock = Instant::now();
        for f in owned {
            tracker.step_owned(f).unwrap();
        }
        let per = clock.elapsed().as_secs_f64() * 1e3 / 30.0;
        println!("point, 4 crosshairs, {threads} threads (0 = all): {per:.2} ms a frame");
    }
}
