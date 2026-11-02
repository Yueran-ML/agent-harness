//! The tracking of a Power Window (T24): kept apart from the window's own Transform and keys,
//! read and written exactly, checked, and composed after the window's own placement.

use columbina_model::grade::window::tracking::{Mode, Motion, TrackData, WindowTracking};
use columbina_model::grade::{Interp, Key, Link, Node, Param, Window, WindowShape};
use columbina_model::{apply, Op, OpError, Project, Target};
use serde_json::{json, Value};

type Spoil = fn(&mut Value);

fn target() -> Target {
    Target::Timeline {
        timeline: "t1".into(),
    }
}

fn project() -> Project {
    let mut project = Project::sample();
    apply(
        &mut project,
        Op::AddNode {
            grade: target(),
            node: Node::corrector(""),
            index: None,
            links: vec![Link::rgb("source", ""), Link::rgb("", "output")],
        },
    )
    .unwrap();
    project
}

fn set(windows: Value) -> Op {
    let mut params = serde_json::Map::new();
    params.insert("windows".into(), windows);
    Op::SetNode {
        grade: target(),
        id: "n3".into(),
        label: None,
        enabled: None,
        locked: None,
        pos: None,
        params: Some(params.into()),
    }
}

/// A track of 200×100 frames: a pan of 2 pixels a frame from frame 10, a turn, a zoom.
fn tracked() -> WindowTracking {
    let mut data = TrackData::new([100.0, 50.0], [200, 100]);
    for f in 10..=20 {
        let k = f64::from(f - 10);
        data.set_sample(
            i64::from(f),
            Motion {
                pan: 2.0 * k,
                rotate: 0.01 * k,
                zoom: 1.0 + 0.01 * k,
                ..Motion::IDENTITY
            },
            0.9,
        );
    }
    data.set_point_count(1);
    data.set_point(0, 10, Some([90.0, 40.0]));
    WindowTracking {
        mode: Mode::Frame,
        crosshairs: vec![[0.5, 0.5]],
        data,
    }
}

fn apply_point(m: [[f64; 3]; 3], p: [f64; 2]) -> [f64; 2] {
    let w = m[2][0] * p[0] + m[2][1] * p[1] + m[2][2];
    [
        (m[0][0] * p[0] + m[0][1] * p[1] + m[0][2]) / w,
        (m[1][0] * p[0] + m[1][1] * p[1] + m[1][2]) / w,
    ]
}

fn near(a: [f64; 2], b: [f64; 2]) {
    assert!(
        (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9,
        "{a:?} != {b:?}"
    );
}

#[test]
fn an_untracked_window_is_written_without_tracking_and_a_tracked_one_reads_back_exactly() {
    let plain = Window::new("w1", WindowShape::circular());
    let written = serde_json::to_value(&plain).unwrap();
    assert!(written.get("tracking").is_none(), "{written}");
    let mut window = plain;
    window.tracking = Some(tracked());
    let text = serde_json::to_string(&window).unwrap();
    let back: Window = serde_json::from_str(&text).unwrap();
    assert_eq!(back, window);
    // Through the project: the whole list is set, and an undo of it would put back the old.
    let mut project = project();
    apply(
        &mut project,
        set(json!([serde_json::to_value(&window).unwrap()])),
    )
    .unwrap();
    let params = project.timelines[0].grade.nodes[0].kind.params();
    let read: Vec<Window> = serde_json::from_value(params["windows"].clone()).unwrap();
    assert_eq!(read[0].tracking, window.tracking);
}

#[test]
fn tracking_that_is_not_finite_or_out_of_range_is_refused_atomically() {
    let base = project();
    let good = {
        let mut w = Window::new("w1", WindowShape::circular());
        w.tracking = Some(tracked());
        serde_json::to_value(&w).unwrap()
    };
    let cases: [(&str, Spoil); 5] = [
        ("windows.tracking.curves", |w| {
            w["tracking"]["data"]["curves"]["pan"]["12"] = json!("NaN");
        }),
        ("windows.tracking.confidence", |w| {
            w["tracking"]["data"]["confidence"]["12"] = json!(1.5);
        }),
        ("windows.tracking.frame_size", |w| {
            w["tracking"]["data"]["frame_size"] = json!([0, 100]);
        }),
        ("windows.tracking.curves", |w| {
            w["tracking"]["data"]["curves"]["zoom"]["12"] = json!(0.0);
        }),
        ("windows.tracking.crosshairs", |w| {
            w["tracking"]["crosshairs"] = json!([[40.0, 0.0]]);
        }),
    ];
    for (what, spoil) in cases {
        let mut bad = good.clone();
        spoil(&mut bad);
        let mut copy = base.clone();
        let error = apply(&mut copy, set(json!([bad])));
        match error {
            Err(OpError::OutOfRange { what: got }) => assert_eq!(got, what),
            // A string where a number is: refused as it is read.
            Err(_) if what == "windows.tracking.curves" => {}
            other => panic!("{what}: {other:?}"),
        }
        assert_eq!(copy, base, "a refused edit changes nothing");
    }
    // Unknown fields are refused too.
    let mut bad = good;
    bad["tracking"]["more"] = json!(1);
    assert!(apply(&mut base.clone(), set(json!([bad]))).is_err());
}

#[test]
fn an_empty_track_places_the_window_as_before() {
    let tracking = WindowTracking::default();
    assert_eq!(tracking.picture_matrix(12.0), None);
    assert!((tracking.scale_at(12.0) - 1.0).abs() < 1e-12);
    tracking.check().unwrap();
}

/// The track applies after the window's own transform, in pixels of the tracked frames whose
/// centres are at `(i + 0.5) / W`: checked against the same composition done by hand in pixels,
/// for a rotated window of another aspect on a frame that is not square.
#[test]
fn the_track_composes_after_the_windows_own_rotated_and_keyed_transform() {
    let mut window = Window::new("w1", WindowShape::circular());
    window.transform.rotate = 30.0.into();
    window.transform.aspect = 70.0.into();
    window.transform.pan = Param::Keys(vec![
        Key {
            frame: 0,
            value: 30.0,
            interp: Interp::Linear,
        },
        Key {
            frame: 20,
            value: 70.0,
            interp: Interp::Linear,
        },
    ]);
    let tracking = tracked();
    let image_aspect = 2.0;
    for frame in [5_i64, 10, 15, 20, 25] {
        #[allow(clippy::cast_precision_loss)]
        let f = frame as f64;
        let own = window.transform.affine_at(f, image_aspect).unwrap();
        let local = [0.25, 0.1];
        let placed = [
            own[0] * local[0] + own[2] * local[1] + own[4],
            own[1] * local[0] + own[3] * local[1] + own[5],
        ];
        // By hand: to pixels of the 200×100 frame, the track there, and back.
        let px = [placed[0] * 200.0 - 0.5, placed[1] * 100.0 - 0.5];
        let moved = tracking.data.window_matrix(frame).apply(px);
        let expected = [(moved[0] + 0.5) / 200.0, (moved[1] + 0.5) / 100.0];
        let m = tracking.picture_matrix(f).unwrap();
        near(apply_point(m, placed), expected);
        // Before the track starts the window holds its first tracked place (p. 3232).
        if frame == 5 {
            near(apply_point(m, placed), placed);
        }
    }
    // The pan of frame 20 is 20 pixels of 200: a tenth of the width, at the anchor.
    let m = tracking.picture_matrix(20.0).unwrap();
    let anchor = [100.5 / 200.0, 50.5 / 100.0];
    near(apply_point(m, anchor), [anchor[0] + 0.1, anchor[1]]);
    assert!(tracking.scale_at(20.0) > 1.09);
}

/// Frame mode's keyframes apply after the track, about the tracked anchor (p. 3236).
#[test]
fn a_frame_mode_keyframe_moves_the_tracked_window_at_its_frame_only_as_interpolated() {
    let mut tracking = tracked();
    tracking.data.set_key(
        12,
        Motion {
            pan: 10.0,
            ..Motion::IDENTITY
        },
    );
    tracking.data.set_key(16, Motion::IDENTITY);
    let at = |f: f64| apply_point(tracking.picture_matrix(f).unwrap(), [0.5, 0.5]);
    let track_only = {
        let mut t = tracked();
        t.mode = Mode::Clip;
        t
    };
    let plain = |f: f64| apply_point(track_only.picture_matrix(f).unwrap(), [0.5, 0.5]);
    near(at(12.0), [plain(12.0)[0] + 10.0 / 200.0, plain(12.0)[1]]);
    near(at(14.0), [plain(14.0)[0] + 5.0 / 200.0, plain(14.0)[1]]);
    near(at(16.0), plain(16.0));
}
