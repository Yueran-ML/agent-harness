//! The patterns of a Tracker followed on blobs of known motion, each on its own.

#![allow(
    clippy::float_cmp,
    reason = "the sizes compared are numbers that floats hold whole, as written"
)]

use columbina_track::pattern::{
    levels_for, radius_for, reach_for, LEAST_RADIUS, MOST_LEVELS, MOST_RADIUS,
};
use columbina_track::{Luma, Pattern, PatternOptions, PatternTracker, Point, TrackError};

const W: usize = 256;
const H: usize = 192;

/// The flat ground of the frames.
const GROUND: f64 = 0.2;

/// A bright textured blob about `at`, as much of it as is at `p`.
fn blob(at: Point, p: Point) -> f64 {
    let (dx, dy) = (p[0] - at[0], p[1] - at[1]);
    let fall = (-(dx * dx + dy * dy) / (2.0 * 8.0 * 8.0)).exp();
    fall * (0.5 + 0.2 * (0.3 * dx).sin() * (0.25 * dy).cos())
}

/// Where the first blob is at frame `k`: along a curve.
fn first(k: i32) -> Point {
    let k = f64::from(k);
    [80.0 + 2.5 * k + 1.5 * (k / 2.0).sin(), 70.0 + k]
}

/// Where the second blob is at frame `k`: the other way.
fn second(k: i32) -> Point {
    let k = f64::from(k);
    [180.0 - 1.5 * k, 120.0 + 0.5 * k]
}

/// Frame `k`: the two blobs on the flat ground, each sample at the middle of its pixel.
fn frame(k: i32) -> Luma {
    let (a, b) = (first(k), second(k));
    let mut data = Vec::with_capacity(W * H);
    for y in 0..H {
        for x in 0..W {
            #[allow(clippy::cast_precision_loss, reason = "small")]
            let p = [x as f64, y as f64];
            #[allow(clippy::cast_possible_truncation, reason = "a sample")]
            let sample = (GROUND + blob(a, p) + blob(b, p)) as f32;
            data.push(sample);
        }
    }
    Luma::new(W, H, data).unwrap()
}

fn apart(a: Point, b: Point) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

#[test]
fn two_patterns_are_followed_each_on_its_own_with_a_window_of_its_own_size() {
    let patterns = [
        Pattern::new(first(0), [24.0, 24.0], [60.0, 60.0]),
        Pattern::new(second(0), [40.0, 30.0], [100.0, 100.0]),
    ];
    assert_eq!((patterns[0].radius, patterns[1].radius), (11, 14));
    let mut tracker = PatternTracker::new(&frame(0), &patterns, PatternOptions::default()).unwrap();
    assert_eq!(tracker.points(), [Some(first(0)), Some(second(0))]);
    for k in 1..12 {
        let points = tracker.step_owned(frame(k)).unwrap().to_vec();
        let (a, b) = (points[0].unwrap(), points[1].unwrap());
        assert!(apart(a, first(k)) < 0.1, "frame {k}: {a:?}");
        assert!(apart(b, second(k)) < 0.1, "frame {k}: {b:?}");
    }
}

#[test]
fn a_pattern_on_flat_ground_beyond_its_search_area_or_outside_the_frame_is_lost() {
    let patterns = [
        // Followed.
        Pattern::new(first(0), [24.0, 24.0], [60.0, 60.0]),
        // On the flat ground: its window is too flat at the finest level.
        Pattern::new([30.0, 175.0], [24.0, 24.0], [60.0, 60.0]),
        // On the first blob, searched for in no more than its box: it moves further.
        Pattern::new(first(0), [24.0, 24.0], [24.0, 24.0]),
        // Outside the frame: lost from the start.
        Pattern::new([-3.0, 40.0], [24.0, 24.0], [60.0, 60.0]),
    ];
    let mut tracker = PatternTracker::new(&frame(0), &patterns, PatternOptions::default()).unwrap();
    assert_eq!(tracker.points()[3], None);
    let points = tracker.step_owned(frame(1)).unwrap().to_vec();
    assert!(apart(points[0].unwrap(), first(1)) < 0.1, "{points:?}");
    assert_eq!(points[1..], [None, None, None]);
    // A pattern lost is followed no more; the others go on.
    let points = tracker.step_owned(frame(2)).unwrap().to_vec();
    assert!(apart(points[0].unwrap(), first(2)) < 0.1, "{points:?}");
    assert_eq!(points[1..], [None, None, None]);
    // No patterns, and a frame of another size, are refused.
    assert_eq!(
        PatternTracker::new(&frame(0), &[], PatternOptions::default()).unwrap_err(),
        TrackError::NoPoints
    );
    let small = Luma::new(16, 16, vec![0.5; 256]).unwrap();
    assert_eq!(
        tracker.step_owned(small).unwrap_err(),
        TrackError::FrameSize {
            expected: [W, H],
            got: [16, 16]
        }
    );
}

#[test]
fn the_window_the_reach_and_the_levels_are_sized_from_the_boxes() {
    // The largest square of an odd side within the box: 35 of a box 64 by 36.
    assert_eq!(radius_for([64.0, 36.0]), 17);
    assert_eq!(radius_for([36.0, 64.0]), 17);
    assert_eq!(radius_for([35.0, 35.0]), 17);
    assert_eq!(radius_for([34.0, 35.0]), 16);
    // Held from 7×7 to 63×63.
    assert_eq!(radius_for([3.0, 3.0]), LEAST_RADIUS);
    assert_eq!(radius_for([0.0, 0.0]), LEAST_RADIUS);
    assert_eq!(radius_for([500.0, 400.0]), MOST_RADIUS);
    assert_eq!(radius_for([f64::INFINITY, f64::INFINITY]), MOST_RADIUS);
    assert_eq!(radius_for([f64::NAN, f64::NAN]), LEAST_RADIUS);
    // Half the difference of the search area and the box, and none where it is no larger.
    assert_eq!(reach_for([64.0, 36.0], [160.0, 90.0]), [48.0, 27.0]);
    assert_eq!(reach_for([64.0, 36.0], [32.0, 90.0]), [0.0, 27.0]);
    assert_eq!(reach_for([64.0, 36.0], [f64::NAN, 36.0]), [0.0, 0.0]);
    // Enough levels that the farther reach is within half the window at the coarsest.
    assert_eq!(levels_for(17, [48.0, 27.0]), 4);
    assert_eq!(levels_for(11, [18.0, 18.0]), 3);
    assert_eq!(levels_for(10, [5.0, 0.0]), 1);
    assert_eq!(levels_for(10, [0.0, 0.0]), 1);
    assert_eq!(levels_for(3, [1e9, 0.0]), MOST_LEVELS);
    assert_eq!(levels_for(3, [f64::NAN, f64::NAN]), 1);
    let pattern = Pattern::new([10.0, 20.0], [64.0, 36.0], [160.0, 90.0]);
    assert_eq!(
        pattern,
        Pattern {
            at: [10.0, 20.0],
            radius: 17,
            reach: [48.0, 27.0]
        }
    );
    assert_eq!(pattern.levels(), 4);
}
