//! Power Windows survive project edits, reject invalid geometry atomically, and share the
//! grade's keyframe clock. These tests use the wire form as the editors do.

use columbina_model::grade::{
    Corrector, Interp, Key, Link, Node, Param, Window, WindowShape, WindowTransform,
};
use columbina_model::{apply, apply_done, expand, Edit, Grade, Op, OpError, Project, Target};
use serde_json::{json, Value};

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

fn window(kind: &str) -> Value {
    let points = json!([
        {"x": -0.25, "y": -0.25}, {"x": 0.25, "y": -0.25},
        {"x": 0.25, "y": 0.25}, {"x": -0.25, "y": 0.25},
    ]);
    let shape = match kind {
        "linear" => json!({"kind": kind, "points": points, "softness": [1.0, 1.0, 1.0, 1.0]}),
        "circular" | "gradient" => json!({"kind": kind, "softness": 1.0}),
        "polygon" => json!({"kind": kind, "points": points, "inside": 0.0, "outside": 0.0}),
        "curve" => {
            json!({"kind": kind, "points": [], "closed": false, "inside": 0.0, "outside": 0.0})
        }
        _ => unreachable!(),
    };
    json!({
        "id": "w1", "label": "Subject", "enabled": true, "invert": false,
        "combine": "add", "opacity": 100.0,
        "transform": {"size": 50.0, "aspect": 50.0, "pan": 50.0, "tilt": 50.0, "rotate": 0.0},
        "shape": shape,
    })
}

fn written_windows(project: &Project) -> Value {
    project.timelines[0].grade.nodes[0].kind.params()["windows"].clone()
}

fn refused(project: &Project, windows: Value) -> OpError {
    let mut copy = project.clone();
    let error = apply(&mut copy, set(windows)).unwrap_err();
    assert_eq!(&copy, project, "a refused window edit changes nothing");
    error
}

#[test]
fn an_old_corrector_reads_without_windows_and_writes_an_empty_list() {
    let mut old = serde_json::to_value(Corrector::default()).unwrap();
    old.as_object_mut().unwrap().remove("windows");
    let read: Corrector = serde_json::from_value(old).unwrap();
    assert_eq!(serde_json::to_value(read).unwrap()["windows"], json!([]));
}

#[test]
fn all_five_shapes_are_saved_and_the_whole_list_is_undoable() {
    let mut project = project();
    let before = project.clone();
    let windows: Vec<_> = ["linear", "circular", "polygon", "curve", "gradient"]
        .into_iter()
        .enumerate()
        .map(|(i, kind)| {
            let mut w = window(kind);
            w["id"] = json!(format!("w{i}"));
            if i == 1 {
                w["combine"] = json!("subtract");
                w["invert"] = json!(true);
            }
            w
        })
        .collect();
    let (done, inverse) = apply_done(&mut project, set(json!(windows))).unwrap();
    assert_eq!(written_windows(&project), json!(windows));
    let saved = serde_json::to_string(&project).unwrap();
    assert_eq!(serde_json::from_str::<Project>(&saved).unwrap(), project);
    apply(&mut project, inverse).unwrap();
    assert_eq!(project, before);
    apply(&mut project, done).unwrap();
    let undo = apply(&mut project, set(json!([window("gradient")]))).unwrap();
    assert_eq!(written_windows(&project).as_array().unwrap().len(), 1);
    apply(&mut project, undo).unwrap();
    assert_eq!(written_windows(&project), json!(windows));
}

#[test]
fn window_ids_are_unique_within_a_node_but_may_repeat_on_another_node() {
    let mut project = project();
    let w = window("circular");
    apply(&mut project, set(json!([w]))).unwrap();
    assert!(matches!(
        refused(&project, json!([w, w])),
        OpError::Taken { what: "window", .. }
    ));
    let mut empty = w.clone();
    empty["id"] = json!("");
    assert!(matches!(refused(&project, json!([empty])), OpError::NoId));
    let mut copied = project.timelines[0].grade.nodes[0].clone();
    copied.id.clear();
    apply(
        &mut project,
        Op::AddNode {
            grade: target(),
            node: copied,
            index: None,
            links: vec![],
        },
    )
    .unwrap();
    assert_eq!(
        project.timelines[0].grade.nodes[1].kind.params()["windows"],
        json!([w])
    );
}

#[test]
fn geometry_counts_and_drafts_are_checked_without_restricting_curve_points() {
    let mut project = project();
    apply(&mut project, set(json!([window("curve")]))).unwrap();
    let anchor =
        json!({"position": {"x": 0, "y": 0}, "in": {"x": -0.2, "y": 0}, "out": {"x": 0.2, "y": 0}});
    let mut curve = window("curve");
    curve["shape"]["points"] = json!([anchor]);
    apply(&mut project, set(json!([curve]))).unwrap();
    curve["shape"]["closed"] = json!(true);
    assert!(matches!(
        refused(&project, json!([curve])),
        OpError::OutOfRange { .. }
    ));
    curve["shape"]["points"] = json!([anchor, anchor]);
    apply(&mut project, set(json!([curve]))).unwrap();
    curve["shape"]["points"] = json!(vec![anchor; 129]);
    apply(&mut project, set(json!([curve]))).unwrap();
    let mut polygon = window("polygon");
    for count in [0, 1, 2, 129] {
        polygon["shape"]["points"] = json!(vec![json!({"x": 0, "y": 0}); count]);
        assert!(matches!(
            refused(&project, json!([polygon])),
            OpError::OutOfRange { .. }
        ));
    }
    for count in [3, 128] {
        polygon["shape"]["points"] = json!(vec![json!({"x": 0, "y": 0}); count]);
        apply(&mut project, set(json!([polygon]))).unwrap();
    }
    let mut linear = window("linear");
    linear["shape"]["points"].as_array_mut().unwrap().pop();
    assert!(matches!(
        refused(&project, json!([linear])),
        OpError::Params { .. }
    ));
}

#[test]
fn window_ranges_types_and_unknown_fields_are_refused_atomically() {
    let mut project = project();
    apply(&mut project, set(json!([window("linear")]))).unwrap();
    for (path, invalid) in [
        ("/opacity", json!(101)),
        ("/opacity", json!(-1)),
        ("/transform/size", json!(-0.1)),
        ("/transform/size", json!(1001)),
        ("/transform/aspect", json!(101)),
        ("/transform/pan", json!(1001)),
        ("/transform/tilt", json!(-1001)),
        ("/transform/rotate", json!(36001)),
        ("/shape/softness/0", json!(101)),
        ("/shape/points/0/x", json!(17)),
        ("/opacity", json!({"keys": []})),
        (
            "/transform/pan",
            json!({"keys": [
                {"frame": 2, "value": 50, "interp": "linear"},
                {"frame": 1, "value": 50, "interp": "linear"},
            ]}),
        ),
    ] {
        let mut w = window("linear");
        *w.pointer_mut(path).unwrap() = invalid;
        assert!(
            matches!(refused(&project, json!([w])), OpError::OutOfRange { .. }),
            "{path}"
        );
    }
    for (path, invalid) in [
        ("/enabled", json!(1)),
        ("/combine", json!("union")),
        ("/opacity", Value::Null),
    ] {
        let mut w = window("linear");
        *w.pointer_mut(path).unwrap() = invalid;
        assert!(matches!(
            refused(&project, json!([w])),
            OpError::Params { .. }
        ));
    }
    for parent in ["", "/transform", "/shape", "/shape/points/0"] {
        let mut w = window("linear");
        w.pointer_mut(parent)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("unknown".into(), json!(0));
        assert!(matches!(
            refused(&project, json!([w])),
            OpError::Params { .. }
        ));
    }
    assert!(matches!(
        refused(&project, Value::Null),
        OpError::Params { .. }
    ));
}

#[test]
fn window_edits_obey_node_lock_and_reset_clears_them_with_an_inverse() {
    let mut project = project();
    apply(&mut project, set(json!([window("circular")]))).unwrap();
    project.timelines[0].grade.nodes[0].locked = true;
    assert!(matches!(
        refused(&project, json!([])),
        OpError::NodeLocked { .. }
    ));
    project.timelines[0].grade.nodes[0].locked = false;
    let before = project.clone();
    let reset = expand(
        &mut project,
        Edit::ResetNode {
            grade: target(),
            node: "n3".into(),
        },
    )
    .unwrap();
    assert_eq!(written_windows(&project), json!([]));
    for op in reset.inverse {
        apply(&mut project, op).unwrap();
    }
    assert_eq!(project, before);
}

#[test]
fn window_geometry_and_transform_keys_follow_grade_copy_frame_mapping() {
    let keys = json!({"keys": [
        {"frame": 100, "value": 0.1, "interp": "linear"},
        {"frame": 120, "value": 0.2, "interp": "hold"},
    ]});
    let mut curve = window("curve");
    curve["transform"]["pan"] = keys.clone();
    curve["shape"]["points"] = json!([{
        "position": {"x": keys, "y": 0}, "in": {"x": 0, "y": keys}, "out": {"x": keys, "y": 0},
    }]);
    let mut project = project();
    apply(&mut project, set(json!([curve]))).unwrap();
    let original = project.timelines[0].grade.clone();
    let moved: Grade = original.with_key_frames(|frame| frame - 80);
    let w = &moved.nodes[0].kind.params()["windows"][0];
    for path in [
        "/transform/pan",
        "/shape/points/0/position/x",
        "/shape/points/0/in/y",
        "/shape/points/0/out/x",
    ] {
        assert_eq!(w.pointer(path).unwrap()["keys"][0]["frame"], 20);
        assert_eq!(w.pointer(path).unwrap()["keys"][1]["frame"], 40);
    }
    assert_eq!(
        original.nodes[0].kind.params()["windows"][0]["transform"]["pan"]["keys"][0]["frame"],
        100
    );
}

fn map(matrix: [f64; 6], point: [f64; 2]) -> [f64; 2] {
    let [xx, yx, xy, yy, tx, ty] = matrix;
    [
        xx * point[0] + xy * point[1] + tx,
        yx * point[0] + yy * point[1] + ty,
    ]
}

fn near(actual: [f64; 2], expected: [f64; 2]) {
    for (a, e) in actual.into_iter().zip(expected) {
        assert!(
            (a - e).abs() < 1e-10,
            "{actual:?} differs from {expected:?}"
        );
    }
}

#[test]
fn shape_constructors_match_the_wire_defaults_and_curve_is_a_saved_draft() {
    for (kind, shape) in [
        ("linear", WindowShape::linear()),
        ("circular", WindowShape::circular()),
        ("polygon", WindowShape::polygon()),
        ("curve", WindowShape::curve()),
        ("gradient", WindowShape::gradient()),
    ] {
        assert_eq!(shape.is_draft(), kind == "curve");
        let mut w = Window::new("w1", shape);
        w.label = "Subject".into();
        assert_eq!(serde_json::to_value(w).unwrap(), window(kind));
    }
}

#[test]
fn local_geometry_uses_image_height_and_transform_directions_match_the_palette() {
    let transform = WindowTransform::default();
    for (width, height) in [(1920.0, 1080.0), (1080.0, 1920.0), (1080.0, 1080.0)] {
        let matrix = transform.affine_at(0.0, width / height).unwrap();
        near(map(matrix, [0.0, 0.0]), [0.5, 0.5]);
        let right = map(matrix, [0.25, 0.0]);
        let bottom = map(matrix, [0.0, 0.25]);
        near(
            [(right[0] - 0.5) * width, (bottom[1] - 0.5) * height],
            [height * 0.25; 2],
        );
    }
    let transform = WindowTransform {
        pan: 75.0.into(),
        tilt: 80.0.into(),
        rotate: 90.0.into(),
        ..WindowTransform::default()
    };
    let matrix = transform.affine_at(0.0, 2.0).unwrap();
    near(map(matrix, [0.0, 0.0]), [0.75, 0.2]);
    near(map(matrix, [0.25, 0.0]), [0.75, 0.45]);
    near(map(matrix, [0.0, 0.25]), [0.625, 0.2]);
    let transform = WindowTransform {
        size: 100.0.into(),
        aspect: 100.0.into(),
        ..WindowTransform::default()
    };
    let matrix = transform.affine_at(0.0, 2.0).unwrap();
    near(map(matrix, [0.25, 0.25]), [1.0, 0.75]);
}

#[test]
fn affine_inverse_recovers_local_handles_and_zero_size_has_no_inverse() {
    for aspect in [9.0 / 16.0, 1.0, 16.0 / 9.0] {
        for rotate in [-137.0, 0.0, 67.0, 36000.0] {
            let transform = WindowTransform {
                size: 21.0.into(),
                aspect: 75.0.into(),
                rotate: rotate.into(),
                pan: 120.0.into(),
                tilt: (-35.0).into(),
            };
            let forward = transform.affine_at(200.0, aspect).unwrap();
            let inverse = transform.inverse_at(200.0, aspect).unwrap();
            for point in [[0.0, 0.0], [-0.25, 0.25], [1.0, -2.0]] {
                near(map(inverse, map(forward, point)), point);
            }
        }
    }
    let transform = WindowTransform {
        size: 0.0.into(),
        ..WindowTransform::default()
    };
    near(
        map(transform.affine_at(0.0, 2.0).unwrap(), [4.0, -3.0]),
        [0.5, 0.5],
    );
    assert_eq!(transform.inverse_at(0.0, 2.0), None);
    for aspect in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(transform.affine_at(0.0, aspect), None);
        assert_eq!(transform.inverse_at(0.0, aspect), None);
    }
    assert_eq!(transform.affine_at(f64::NAN, 1.0), None);
}

#[test]
fn transform_evaluates_source_frame_keys_and_clamps_bezier_overshoot() {
    let transform = WindowTransform {
        pan: Param::Keys(vec![
            Key {
                frame: 100,
                value: 25.0,
                interp: Interp::Linear,
            },
            Key {
                frame: 120,
                value: 75.0,
                interp: Interp::Hold,
            },
        ]),
        size: Param::Keys(vec![
            Key {
                frame: 100,
                value: 50.0,
                interp: Interp::Bezier {
                    out: [0.3, -1000.0],
                    into: [0.3, -1000.0],
                },
            },
            Key {
                frame: 120,
                value: 50.0,
                interp: Interp::Linear,
            },
        ]),
        ..WindowTransform::default()
    };
    near(
        map(transform.affine_at(100.0, 1.0).unwrap(), [0.25, 0.0]),
        [0.5, 0.5],
    );
    near(
        map(transform.affine_at(110.0, 1.0).unwrap(), [0.25, 0.0]),
        [0.5, 0.5],
    );
    assert_eq!(transform.inverse_at(110.0, 1.0), None);
    near(
        map(transform.affine_at(120.0, 1.0).unwrap(), [0.25, 0.0]),
        [1.0, 0.5],
    );
}

/// A window preset keeps the shape and parameters at the playhead, interpolated and clamped as
/// the mask uses them, and no keyframes (author decision 2026-11-01; RM20 p. 3199).
#[test]
fn a_window_frozen_at_a_frame_keeps_its_interpolated_values_without_keys() {
    let keys = |from: f64, to: f64| {
        Param::Keys(vec![
            Key {
                frame: 0,
                value: from,
                interp: Interp::Linear,
            },
            Key {
                frame: 10,
                value: to,
                interp: Interp::Linear,
            },
        ])
    };
    let mut window = Window::new("w2", WindowShape::curve());
    window.label = "face".into();
    // A window turned off is kept off: a preset keeps On/Off and the Layer Name (RM20 p. 3189).
    window.enabled = false;
    window.invert = true;
    window.transform.pan = keys(20.0, 40.0);
    // Rotate overshoots its bounds after interpolation; the mask clamps it, and so does a preset.
    window.transform.rotate = keys(0.0, 80_000.0);
    window.opacity = keys(100.0, 0.0);
    window.shape = serde_json::from_value(json!({"kind":"curve","closed":true,
        "points":[
            {"position":{"x":{"keys":[{"frame":0,"value":0.0,"interp":"linear"},{"frame":10,"value":0.5,"interp":"linear"}]},"y":0.1},
             "in":{"x":0.0,"y":0.0},"out":{"x":{"keys":[{"frame":0,"value":0.0,"interp":"hold"},{"frame":10,"value":1.0,"interp":"linear"}]},"y":0.0}},
            {"position":{"x":0.2,"y":0.2},"in":{"x":0.0,"y":0.0},"out":{"x":0.0,"y":0.0}}],
        "inside":{"keys":[{"frame":0,"value":0.0,"interp":"linear"},{"frame":10,"value":10.0,"interp":"linear"}]},"outside":3.0}))
    .unwrap();
    let frozen = window.frozen_at(5.0);
    let written = serde_json::to_value(&frozen).unwrap();
    assert!(!written.to_string().contains("keys"), "{written}");
    assert_eq!(written["id"], "w2");
    assert_eq!(written["label"], "face");
    assert_eq!(written["enabled"], false);
    assert_eq!(written["invert"], true);
    assert_eq!(written["transform"]["pan"], 30.0);
    assert_eq!(written["transform"]["rotate"], 36_000.0);
    assert_eq!(written["opacity"], 50.0);
    assert_eq!(written["shape"]["closed"], true);
    assert_eq!(written["shape"]["points"][0]["position"]["x"], 0.25);
    assert_eq!(written["shape"]["points"][0]["out"]["x"], 0.0);
    assert_eq!(written["shape"]["inside"], 5.0);
    assert_eq!(written["shape"]["outside"], 3.0);
    // Every shape keeps its kind and the values it has on that frame.
    for shape in [
        WindowShape::linear(),
        WindowShape::circular(),
        WindowShape::polygon(),
        WindowShape::gradient(),
    ] {
        let window = Window::new("w1", shape);
        assert_eq!(window.frozen_at(3.0), window);
    }
}

/// A window preset never keeps the window's tracking (T24): its track is of the frames of one
/// clip, and the preset is recalled on any node of any project, to be tracked there again.
#[test]
fn a_window_frozen_at_a_frame_keeps_no_tracking() {
    use columbina_model::grade::window::tracking::{Mode, Motion, TrackData, WindowTracking};
    let mut data = TrackData::new([639.5, 359.5], [1280, 720]);
    data.set_sample(4, Motion::IDENTITY, 1.0);
    data.set_key(4, Motion::IDENTITY);
    let mut window = Window::new("w1", WindowShape::polygon());
    window.tracking = Some(WindowTracking {
        mode: Mode::Frame,
        crosshairs: vec![[0.5, 0.5]],
        data,
    });
    let frozen = window.frozen_at(4.0);
    assert_eq!(frozen.tracking, None);
    let written = serde_json::to_value(&frozen).unwrap();
    assert!(written.get("tracking").is_none(), "{written}");
    window.tracking = None;
    assert_eq!(frozen, window.frozen_at(4.0));
}

/// Add Serial Node + CPW, LPW, PPW and PCW (Option-C, -Q, -G and -B; p. 3187): `add_serial`
/// with `windows` adds the corrector with those windows, as one operation that one undo takes
/// back, and checks them as `set_node` does, refusing the whole edit for one that is invalid.
#[test]
fn add_serial_with_windows_adds_the_node_with_them_in_one_undoable_operation() {
    let mut project = project();
    let before = project.clone();
    let edit: Edit = serde_json::from_value(json!({
        "edit": "add_serial", "grade": {"level": "timeline", "timeline": "t1"}, "node": "n3",
        "windows": [window("circular")],
    }))
    .unwrap();
    let expansion = expand(&mut project, edit.clone()).unwrap();
    assert_eq!(expansion.done.len(), 1, "{:?}", expansion.done);
    let nodes = &project.timelines[0].grade.nodes;
    assert_eq!(nodes.len(), 2);
    assert_eq!(
        nodes[1].kind.params()["windows"],
        json!([window("circular")])
    );
    // The new node goes after n3, as a plain Add Serial puts it.
    let grade = &project.timelines[0].grade;
    assert!(grade.links.contains(&Link::rgb("n3", nodes[1].id.clone())));
    for op in expansion.inverse.iter().rev() {
        apply(&mut project, op.clone()).unwrap();
    }
    project.next_id = before.next_id;
    assert_eq!(project, before);
    // Written back as it was read; a plain Add Serial writes no `windows`.
    assert_eq!(
        serde_json::to_value(&edit).unwrap()["windows"],
        json!([window("circular")])
    );
    let plain = Edit::AddSerial {
        grade: target(),
        node: "n3".into(),
        windows: vec![],
    };
    assert!(serde_json::to_value(&plain)
        .unwrap()
        .get("windows")
        .is_none());
    // A window refused as `set_node` refuses it refuses the edit, and nothing is added.
    let mut wrong = window("linear");
    wrong["opacity"] = json!(101);
    let edit: Edit = serde_json::from_value(json!({
        "edit": "add_serial", "grade": {"level": "timeline", "timeline": "t1"}, "node": "n3",
        "windows": [wrong],
    }))
    .unwrap();
    let mut copy = before.clone();
    assert!(expand(&mut copy, edit).is_err());
    assert_eq!(copy, before);
    for windows in [json!([window("curve"), window("curve")]), json!({})] {
        let read = serde_json::from_value::<Edit>(json!({
            "edit": "add_serial", "grade": {"level": "timeline", "timeline": "t1"},
            "node": "n3", "windows": windows,
        }));
        if let Ok(edit) = read {
            let mut copy = before.clone();
            assert!(expand(&mut copy, edit).is_err(), "{windows}");
            assert_eq!(copy, before);
        }
    }
}

#[test]
fn the_feathers_of_polygons_and_curves_are_kept_within_their_range() {
    let project = project();
    for kind in ["polygon", "curve"] {
        for (path, invalid) in [("/shape/inside", json!(101)), ("/shape/outside", json!(-1))] {
            let mut w = window(kind);
            *w.pointer_mut(path).unwrap() = invalid;
            assert!(
                matches!(refused(&project, json!([w])), OpError::OutOfRange { .. }),
                "{kind} {path}"
            );
        }
    }
}
