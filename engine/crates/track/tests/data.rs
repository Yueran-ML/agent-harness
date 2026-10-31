//! The track data: its curves, their clearing and interpolation, Clip and Frame mode, copy and
//! paste, and the change of scale.

use columbina_track::{Adjusted, Component, Components, Mat3, Mode, Motion, TrackData};

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

/// Track data with a sample at frames 0 to 10: pan `f²`, tilt `−f`, zoom `1 + f/100`.
fn tracked() -> TrackData {
    let mut data = TrackData::new([100.0, 50.0], [640, 360]);
    for f in 0..=10_i32 {
        let x = f64::from(f);
        let m = Motion {
            pan: x * x,
            tilt: -x,
            zoom: 1.0 + x / 100.0,
            rotate: 0.01 * x,
            ..Motion::IDENTITY
        };
        data.set_sample(i64::from(f), m, 0.9);
    }
    data
}

#[test]
fn an_empty_track_is_no_motion_and_the_ends_are_held() {
    let empty = TrackData::new([1.0, 2.0], [10, 10]);
    assert!(empty.is_empty());
    assert_eq!(empty.motion_at(7), Motion::IDENTITY);
    assert_eq!(empty.window_matrix(7), Mat3::IDENTITY);
    let data = tracked();
    assert!(!data.is_empty());
    assert_eq!(data.tracked_range(), Some((0, 10)));
    assert_eq!(data.motion_at(-5), data.motion_at(0));
    assert_eq!(data.motion_at(25), data.motion_at(10));
}

#[test]
fn clearing_a_stretch_of_some_curves_bridges_it_with_a_line() {
    let mut data = tracked();
    data.clear_range(6, 3, &[Component::Pan]);
    // Between frame 2 (4) and frame 7 (49).
    assert!(close(
        data.value_at(Component::Pan, 4),
        4.0 + 45.0 * 2.0 / 5.0
    ));
    assert!(close(data.value_at(Component::Tilt, 4), -4.0));
    assert!(!data.has_sample(4));
    assert_eq!(data.confidence_at(4), Some(0.9));
    // Every curve: the confidence goes too.
    data.clear_range(3, 6, &Component::ALL);
    assert_eq!(data.confidence_at(4), None);
    assert!(close(data.value_at(Component::Zoom, 5), 1.05));
    assert_eq!(data.tracked_range(), Some((0, 10)));
}

#[test]
fn interpolating_a_range_writes_the_line_between_its_ends() {
    let mut data = tracked();
    data.interpolate_range(2, 8, &[Component::Pan, Component::Rotate]);
    for f in 2..=8_i32 {
        let want = 4.0 + 60.0 * f64::from(f - 2) / 6.0;
        assert!(close(data.value_at(Component::Pan, i64::from(f)), want));
        assert!(close(
            data.value_at(Component::Rotate, i64::from(f)),
            0.01 * f64::from(f)
        ));
    }
    assert!(close(data.value_at(Component::Pan, 9), 81.0));
    assert!(data.has_sample(5));
}

#[test]
fn reset_takes_the_track_and_leaves_the_keyframes() {
    let mut data = tracked();
    data.set_key(
        3,
        Motion {
            pan: 2.0,
            ..Motion::IDENTITY
        },
    );
    data.set_point_count(1);
    data.set_point(0, 4, Some([1.0, 2.0]));
    data.reset();
    assert!(data.is_empty());
    assert_eq!(data.points().len(), 0);
    assert_eq!(data.keys().len(), 1);
}

#[test]
fn frame_mode_keys_a_change_and_interpolates_between_keys() {
    let mut data = tracked();
    let before = data.window_matrix(5).apply(data.anchor);
    assert_eq!(
        data.adjust(Mode::Frame, 5, &Mat3::translation(10.0, 0.0)),
        Adjusted::Keyed
    );
    let after = data.window_matrix(5).apply(data.anchor);
    assert!(close(after[0] - before[0], 10.0) && close(after[1], before[1]));
    // One key holds everywhere; a second one is reached along a line.
    assert!(close(data.key_at(0).pan, 10.0));
    data.adjust(Mode::Frame, 9, &Mat3::translation(20.0, 4.0));
    assert!(close(data.key_at(9).pan, 30.0) && close(data.key_at(9).tilt, 4.0));
    assert!(close(data.key_at(7).pan, 20.0) && close(data.key_at(7).tilt, 2.0));
    assert_eq!(data.previous_key(9), Some(5));
    assert_eq!(data.next_key(5), Some(9));
    assert_eq!(data.next_key(9), None);
    assert!(data.delete_key(9));
    assert!(!data.delete_key(9));
    assert!(close(data.key_at(7).pan, 10.0));
    // A key about the tracked anchor: a zoom there keeps the tracked anchor in place.
    let mut zoomed = tracked();
    let zoom = Motion {
        zoom: 2.0,
        ..Motion::IDENTITY
    }
    .matrix(zoomed.tracked_anchor(4));
    zoomed.adjust(Mode::Frame, 4, &zoom);
    let a = zoomed.window_matrix(4).apply(zoomed.anchor);
    let t = zoomed.tracked_anchor(4);
    assert!(close(a[0], t[0]) && close(a[1], t[1]));
}

#[test]
fn clip_mode_gives_the_change_of_the_window_itself() {
    let mut data = tracked();
    data.set_key(
        2,
        Motion {
            rotate: 0.1,
            ..Motion::IDENTITY
        },
    );
    let delta = Mat3::translation(5.0, -3.0).mul(
        &Motion {
            zoom: 1.2,
            ..Motion::IDENTITY
        }
        .matrix([60.0, 70.0]),
    );
    let Adjusted::Window(change) = data.adjust(Mode::Clip, 6, &delta) else {
        panic!("a change of the window");
    };
    // Applied to the window before the track, it shows as `delta` at frame 6...
    let probe = [[10.0, 20.0], [300.0, 40.0], [150.0, 200.0]];
    let seen = data.window_matrix(6).mul(&change);
    assert!(seen.distance_on(&delta.mul(&data.window_matrix(6)), &probe) < 1e-9);
    // ...and moves the window along its whole path.
    assert!(data.keys().len() == 1 && data.motion_at(6) == tracked().motion_at(6));
}

#[test]
fn pasted_track_data_moves_another_window_with_the_same_motion() {
    let source = tracked();
    let mut target = TrackData::new([400.0, 300.0], [640, 360]);
    target.set_key(
        1,
        Motion {
            pan: 3.0,
            ..Motion::IDENTITY
        },
    );
    target.paste(&source);
    let probe = [[0.0, 0.0], [640.0, 0.0], [320.0, 360.0], [400.0, 300.0]];
    for f in 0..=10 {
        let err = target
            .track_matrix(f)
            .distance_on(&source.track_matrix(f), &probe);
        assert!(err < 1e-9, "{f}: {err}");
    }
    assert_eq!(target.keys().len(), 1);
    assert_eq!(target.confidence(), source.confidence());
    assert_eq!(target.anchor, [400.0, 300.0]);
}

#[test]
fn a_track_on_a_proxy_scales_to_the_original() {
    let mut data = tracked();
    let mut m = data.motion_at(4);
    m.yaw = 2e-4;
    m.pitch = -1e-4;
    m.aspect = 1.05;
    data.set_sample(4, m, 0.5);
    data.set_point_count(1);
    data.set_point(0, 4, Some([10.0, 20.0]));
    let big = data.rescaled(3.0);
    assert_eq!(big.frame_size, [1920, 1080]);
    assert_eq!(big.points()[0][&4], [30.0, 60.0]);
    let s = Mat3([[3.0, 0.0, 0.0], [0.0, 3.0, 0.0], [0.0, 0.0, 1.0]]);
    let want = s.mul(&data.track_matrix(4)).mul(&s.inverse().unwrap());
    let probe = [[0.0, 0.0], [1920.0, 1080.0], [300.0, 150.0]];
    assert!(big.track_matrix(4).distance_on(&want, &probe) < 1e-9);
}

#[test]
fn track_data_goes_to_json_and_back() {
    let mut data = tracked();
    data.components = Components::ALL;
    data.set_key(
        3,
        Motion {
            shear: 0.1,
            ..Motion::IDENTITY
        },
    );
    data.set_point_count(2);
    data.set_point(1, 5, Some([3.5, 4.25]));
    let json = serde_json::to_string(&data).unwrap();
    assert!(json.contains("\"pan\""));
    let back: TrackData = serde_json::from_str(&json).unwrap();
    // serde_json reads a float back to within its last bit.
    for f in 0..=10 {
        for c in Component::ALL {
            let (a, b) = (data.value_at(c, f), back.value_at(c, f));
            assert!((a - b).abs() <= 1e-15 * a.abs().max(1.0));
        }
    }
    assert_eq!(back.keys().len(), 1);
    assert_eq!(back.points(), data.points());
    assert_eq!(back.components, data.components);
    assert_eq!(back.confidence(), data.confidence());
}
