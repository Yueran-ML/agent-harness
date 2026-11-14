//! The keys of the settings of a clip (schema 5's transform): the operations that set them by
//! their paths ([`Op::SetClipKeys`], [`Op::SpliceClipKeys`]), what takes them back, what they
//! refuse, and where a keyed transform puts the picture at a frame of the timeline.

use columbina_model::catalog;
use columbina_model::grade::{Interp, Key, Param};
use columbina_model::resolve::video_at;
use columbina_model::{
    apply, apply_done, Clip, Op, OpError, Placed, Project, Rgba, Source, Speed, Transform,
};
use serde_json::{json, Value};

fn reason(error: &OpError) -> Value {
    let mut value = serde_json::to_value(error).unwrap();
    value.as_object_mut().unwrap().remove("message");
    value
}

fn key(frame: i64, value: f64) -> Key {
    Key {
        frame,
        value,
        interp: Interp::Linear,
    }
}

fn set(path: &str, param: Param) -> Op {
    Op::SetClipKeys {
        timeline: "t1".into(),
        id: "c1".into(),
        path: path.into(),
        param,
    }
}

fn splice(path: &str, at: usize, remove: usize, keys: Vec<Key>) -> Op {
    Op::SpliceClipKeys {
        timeline: "t1".into(),
        id: "c1".into(),
        path: path.into(),
        at,
        remove,
        keys,
    }
}

fn c1(project: &Project) -> &Clip {
    &project.timelines[0].tracks[0].clips[0]
}

/// Apply `op`, and check that its inverse takes it back and that the inverse of that is the
/// operation as it was done.
fn applied(project: &mut Project, op: Op) -> (Op, Op) {
    let before = project.clone();
    let (done, inverse) = apply_done(project, op).unwrap();
    let mut undone = project.clone();
    let again = apply(&mut undone, inverse.clone()).unwrap();
    assert_eq!(undone, before);
    assert_eq!(again, done);
    (done, inverse)
}

fn refused(project: &mut Project, op: Op) -> Value {
    let before = project.clone();
    let error = apply(project, op).unwrap_err();
    assert_eq!(*project, before);
    reason(&error)
}

#[test]
fn a_transform_is_written_as_numbers_where_it_has_no_keys_and_read_with_keys_where_it_has() {
    let mut project = Project::sample();
    let written = serde_json::to_value(c1(&project)).unwrap();
    assert_eq!(
        written["transform"],
        json!({"x": 0.0, "y": 0.0, "scale": 1.0, "rotation": 0.0, "opacity": 1.0})
    );
    applied(
        &mut project,
        set("transform.x", Param::Keys(vec![key(0, -0.5), key(24, 0.0)])),
    );
    let written = serde_json::to_value(&project).unwrap();
    let transform = &written["timelines"][0]["tracks"][0]["clips"][0]["transform"];
    assert_eq!(
        transform["x"],
        json!({"keys": [
            {"frame": 0, "value": -0.5, "interp": "linear"},
            {"frame": 24, "value": 0.0, "interp": "linear"},
        ]})
    );
    assert_eq!(transform["y"], json!(0.0));
    assert_eq!(Project::from_value(written).unwrap(), project);
}

#[test]
fn keys_are_set_whole_and_spliced_and_each_is_taken_back() {
    let mut project = Project::sample();
    // The first keys of a number are done as the keys set whole, and taken back as the number.
    let (done, inverse) = applied(
        &mut project,
        splice("transform.scale", 0, 0, vec![key(10, 2.0)]),
    );
    assert_eq!(
        done,
        set("transform.scale", Param::Keys(vec![key(10, 2.0)]))
    );
    assert_eq!(inverse, set("transform.scale", Param::Fixed(1.0)));
    // A key put after it, and the first moved and changed.
    let (_, inverse) = applied(
        &mut project,
        splice("transform.scale", 1, 0, vec![key(20, 4.0)]),
    );
    assert_eq!(inverse, splice("transform.scale", 1, 1, vec![]));
    let (_, inverse) = applied(
        &mut project,
        splice("transform.scale", 0, 1, vec![key(5, 1.5)]),
    );
    assert_eq!(inverse, splice("transform.scale", 0, 1, vec![key(10, 2.0)]));
    assert_eq!(
        c1(&project).transform.scale,
        Param::Keys(vec![key(5, 1.5), key(20, 4.0)])
    );
    // A number in place of the keys.
    let (_, inverse) = applied(&mut project, set("transform.scale", Param::Fixed(3.0)));
    assert_eq!(
        inverse,
        set(
            "transform.scale",
            Param::Keys(vec![key(5, 1.5), key(20, 4.0)])
        )
    );
    assert_eq!(
        c1(&project).transform,
        Transform {
            scale: 3.0.into(),
            ..Transform::NONE
        }
    );
}

#[test]
fn the_settings_that_take_keys_are_those_of_the_catalog_and_are_checked_as_the_clip_is() {
    let mut project = Project::sample();
    let paths: Vec<&str> = catalog::clip().iter().map(|e| e.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "transform.x",
            "transform.y",
            "transform.scale",
            "transform.rotation",
            "transform.opacity",
            "crop.left",
            "crop.right",
            "crop.top",
            "crop.bottom",
            "crop.softness"
        ]
    );
    for entry in catalog::clip() {
        // What a new clip has is what the catalog says it has at first.
        let mut probe = Project::sample();
        let (_, inverse) = applied(&mut probe, set(&entry.path, Param::Fixed(entry.high)));
        assert_eq!(inverse, set(&entry.path, Param::Fixed(entry.default)));
        applied(&mut probe, set(&entry.path, Param::Fixed(entry.low)));
    }
    for path in [
        "transform",
        "transform.z",
        "x",
        "gain_db",
        "transform.x.0",
        "",
        "crop",
        "crop.x",
        "composite",
    ] {
        assert_eq!(
            refused(&mut project, set(path, Param::Fixed(0.5))),
            json!({"reason": "missing", "what": "setting", "id": path}),
            "{path}"
        );
    }
    let out = json!({"reason": "out_of_range", "what": "transform"});
    for (path, param) in [
        ("transform.scale", Param::Fixed(0.0)),
        ("transform.scale", Param::Fixed(16.5)),
        (
            "transform.opacity",
            Param::Keys(vec![key(0, 0.5), key(9, 1.5)]),
        ),
        ("transform.x", Param::Fixed(f64::INFINITY)),
        // Keys are at frames in order, and there is one at least.
        ("transform.x", Param::Keys(vec![key(9, 0.0), key(9, 0.5)])),
        ("transform.x", Param::Keys(vec![])),
        (
            "transform.y",
            Param::Keys(vec![Key {
                frame: 0,
                value: 0.0,
                interp: Interp::Bezier {
                    out: [1.5, 0.0],
                    into: [0.5, 0.0],
                },
            }]),
        ),
    ] {
        assert_eq!(
            refused(&mut project, set(path, param.clone())),
            out,
            "{path} {param:?}"
        );
    }
    let out = json!({"reason": "out_of_range", "what": "crop"});
    for (path, param) in [
        ("crop.left", Param::Fixed(-0.1)),
        ("crop.softness", Param::Fixed(1.5)),
        ("crop.top", Param::Keys(vec![key(0, 0.5), key(9, f64::NAN)])),
    ] {
        assert_eq!(
            refused(&mut project, set(path, param.clone())),
            out,
            "{path} {param:?}"
        );
    }
    assert_eq!(
        refused(
            &mut project,
            Op::SetClipKeys {
                timeline: "t1".into(),
                id: "c9".into(),
                path: "transform.x".into(),
                param: Param::Fixed(0.0),
            }
        ),
        json!({"reason": "missing", "what": "clip", "id": "c9"})
    );
}

#[test]
fn a_splice_is_within_the_keys_that_there_are_and_leaves_one_at_least() {
    let mut project = Project::sample();
    assert_eq!(
        refused(&mut project, splice("transform.x", 0, 1, vec![key(0, 0.0)])),
        json!({"reason": "out_of_list", "at": 1, "len": 0})
    );
    applied(
        &mut project,
        set("transform.x", Param::Keys(vec![key(0, 0.0), key(10, 0.5)])),
    );
    assert_eq!(
        refused(&mut project, splice("transform.x", 1, 2, vec![])),
        json!({"reason": "out_of_list", "at": 3, "len": 2})
    );
    assert_eq!(
        refused(&mut project, splice("transform.x", 0, 2, vec![])),
        json!({"reason": "out_of_range", "what": "keys"})
    );
    // Keys that would not be in order are refused as the transform is.
    assert_eq!(
        refused(&mut project, splice("transform.x", 2, 0, vec![key(5, 1.0)])),
        json!({"reason": "out_of_range", "what": "transform"})
    );
}

#[test]
fn set_clip_puts_a_transform_with_keys_whole_and_puts_back_the_one_that_was() {
    let mut project = Project::sample();
    let keyed = Transform {
        rotation: Param::Keys(vec![key(0, 0.0), key(50, 360.0)]),
        ..Placed {
            x: 0.25,
            ..Placed::NONE
        }
        .into()
    };
    let op: Op = serde_json::from_value(json!({
        "op": "set_clip", "timeline": "t1", "id": "c1",
        "transform": {"x": 0.25, "y": 0, "scale": 1, "opacity": 1, "rotation": {"keys": [
            {"frame": 0, "value": 0, "interp": "linear"},
            {"frame": 50, "value": 360, "interp": "linear"},
        ]}},
    }))
    .unwrap();
    let (_, inverse) = applied(&mut project, op);
    assert_eq!(c1(&project).transform, keyed);
    let Op::SetClip { transform, .. } = inverse else {
        panic!("{inverse:?}")
    };
    assert_eq!(transform, Some(Transform::NONE));
}

/// The transform of the one layer of the sample's picture at a frame.
fn placed_at(project: &Project, frame: i64) -> (Placed, f32) {
    let layers = video_at(&project.timelines[0], project, frame);
    assert_eq!(layers.len(), 1, "{frame}");
    (layers[0].transform, layers[0].opacity)
}

#[test]
fn a_keyed_transform_is_at_each_frame_what_its_keys_are_at_the_picture_shown_there() {
    // c1 shows m1 from its start, a frame of the media to a frame of the timeline.
    let mut project = Project::sample();
    apply(
        &mut project,
        set("transform.x", Param::Keys(vec![key(10, 0.0), key(30, 1.0)])),
    )
    .unwrap();
    apply(
        &mut project,
        set(
            "transform.opacity",
            Param::Keys(vec![key(0, 1.0), key(40, 0.0)]),
        ),
    )
    .unwrap();
    for (frame, x, opacity) in [
        (0, 0.0, 1.0),
        (10, 0.0, 0.75),
        (20, 0.5, 0.5),
        (30, 1.0, 0.25),
        (40, 1.0, 0.0),
        (99, 1.0, 0.0),
    ] {
        let (placed, seen) = placed_at(&project, frame);
        assert!((placed.x - x).abs() < 1e-12, "{frame}: {}", placed.x);
        assert!((f64::from(seen) - opacity).abs() < 1e-6, "{frame}: {seen}");
        assert_eq!((placed.y, placed.scale, placed.rotation), (0.0, 1.0, 0.0));
    }
    // At twice the speed the pictures come twice as soon, and their keys with them.
    let faster: Op = serde_json::from_value(json!({
        "op": "set_clip", "timeline": "t1", "id": "c1", "speed": Speed { num: 2, den: 1 },
    }))
    .unwrap();
    apply(&mut project, faster).unwrap();
    for (frame, x) in [(5, 0.0), (10, 0.5), (15, 1.0)] {
        let (placed, _) = placed_at(&project, frame);
        assert!((placed.x - x).abs() < 1e-12, "{frame}: {}", placed.x);
    }
}

#[test]
fn the_keys_of_a_clip_of_no_source_count_its_own_frames_from_its_first() {
    let mut project = Project::sample();
    let mut colour = Clip::new(
        "k1",
        Source::Color {
            color: Rgba([255, 0, 0, 255]),
        },
        200,
        50,
    );
    colour.transform.y = Param::Keys(vec![key(0, 0.0), key(40, 1.0)]);
    project.timelines[0].tracks[0].clips.push(colour);
    project.validate().unwrap();
    for (frame, y) in [(200, 0.0), (220, 0.5), (249, 1.0)] {
        let layers = video_at(&project.timelines[0], &project, frame);
        assert_eq!(layers.len(), 1);
        assert!((layers[0].transform.y - y).abs() < 1e-12, "{frame}");
    }
}

#[test]
fn a_crop_and_a_composite_mode_are_set_whole_and_a_crop_is_keyed_and_read_at_the_frame() {
    use columbina_model::grade::Composite;
    use columbina_model::{Crop, Cropped};
    let mut project = Project::sample();
    assert_eq!(
        (c1(&project).crop.clone(), c1(&project).composite),
        (Crop::NONE, Composite::Normal)
    );
    let op: Op = serde_json::from_value(json!({
        "op": "set_clip", "timeline": "t1", "id": "c1", "composite": "screen",
        "crop": {"left": 0.25, "right": 0, "top": 0.1, "bottom": 0, "softness": 0.05},
    }))
    .unwrap();
    let (done, inverse) = applied(&mut project, op);
    assert_eq!(
        serde_json::to_value(&done).unwrap()["composite"],
        json!("screen")
    );
    let Op::SetClip {
        crop, composite, ..
    } = inverse
    else {
        panic!("a set_clip")
    };
    assert_eq!(
        (crop, composite),
        (Some(Crop::NONE), Some(Composite::Normal))
    );
    // The left side opens over the first 20 pictures.
    applied(
        &mut project,
        set("crop.left", Param::Keys(vec![key(0, 1.0), key(20, 0.0)])),
    );
    let layers = video_at(&project.timelines[0], &project, 10);
    assert_eq!(layers[0].composite, Composite::Screen);
    assert_eq!(
        layers[0].crop,
        Cropped {
            left: 0.5,
            right: 0.0,
            top: 0.1,
            bottom: 0.0,
            softness: 0.05
        }
    );
    // The sound of the same media is not cropped: a clip of sound keeps the settings all the same.
    let written = serde_json::to_value(&project).unwrap();
    assert_eq!(
        written["timelines"][0]["tracks"][1]["clips"][0]["crop"],
        json!({"left": 0.0, "right": 0.0, "top": 0.0, "bottom": 0.0, "softness": 0.0})
    );
    assert_eq!(Project::from_value(written).unwrap(), project);
}

// A clip that counts its own frames, under the edits that move its head.

/// The sample with a colour `k1` on V1 from frame 200 for 50 frames, moved right over its
/// first 40 frames and graded with nothing.
fn with_a_colour_that_moves() -> Project {
    let mut project = Project::sample();
    let mut colour = Clip::new(
        "k1",
        Source::Color {
            color: Rgba([255, 0, 0, 255]),
        },
        200,
        50,
    );
    colour.transform.x = Param::Keys(vec![key(0, 0.0), key(40, 1.0)]);
    colour.crop.left = Param::Keys(vec![key(10, 0.5), key(20, 0.0)]);
    project.timelines[0].tracks[0].clips.push(colour);
    project.next_id = 10;
    project.validate().unwrap();
    project
}

/// How far right what V1 shows at each of `frames` is moved, and how much of its left is cut.
fn seen_at(project: &Project, frames: &[i64]) -> Vec<(f64, f64)> {
    frames
        .iter()
        .map(|&frame| {
            let layers = video_at(&project.timelines[0], project, frame);
            assert_eq!(layers.len(), 1, "{frame}");
            (layers[0].transform.x, layers[0].crop.left)
        })
        .collect()
}

/// Apply an edit, and check that its inverse takes the project back.
fn edited(project: &mut Project, edit: Value) {
    let before = project.clone();
    let edit: columbina_model::Edit = serde_json::from_value(edit).unwrap();
    let expansion = columbina_model::expand(project, edit).unwrap();
    let mut undone = project.clone();
    for inverse in expansion.inverse.iter().rev() {
        apply(&mut undone, inverse.clone()).unwrap();
    }
    undone.next_id = before.next_id;
    assert_eq!(undone, before);
}

fn x_keys(project: &Project, id: &str) -> Vec<(i64, f64)> {
    let clip = project.timelines[0].tracks[0]
        .clips
        .iter()
        .find(|c| c.id == id)
        .unwrap();
    match &clip.transform.x {
        Param::Keys(keys) => keys.iter().map(|k| (k.frame, k.value)).collect(),
        Param::Fixed(value) => vec![(i64::MIN, *value)],
    }
}

#[test]
fn a_trim_of_the_head_of_a_colour_leaves_its_keys_at_their_frames_of_the_timeline() {
    let frames = [210, 215, 220, 230, 245];
    let mut project = with_a_colour_that_moves();
    let before = seen_at(&project, &frames);
    edited(
        &mut project,
        json!({"edit": "trim", "timeline": "t1", "clip": "k1", "edge": "in", "delta": 10}),
    );
    assert_eq!(x_keys(&project, "k1"), [(-10, 0.0), (30, 1.0)]);
    assert_eq!(seen_at(&project, &frames), before);
    // Its head moved out again, by more than it was moved in.
    edited(
        &mut project,
        json!({"edit": "trim", "timeline": "t1", "clip": "k1", "edge": "in", "delta": -30}),
    );
    assert_eq!(x_keys(&project, "k1"), [(20, 0.0), (60, 1.0)]);
    assert_eq!(seen_at(&project, &frames), before);
    // Its tail leaves its keys alone.
    edited(
        &mut project,
        json!({"edit": "trim", "timeline": "t1", "clip": "k1", "edge": "out", "delta": -5}),
    );
    assert_eq!(x_keys(&project, "k1"), [(20, 0.0), (60, 1.0)]);
}

#[test]
fn a_ripple_trim_of_the_head_of_a_colour_shows_from_its_start_what_it_showed_later() {
    let mut project = with_a_colour_that_moves();
    let before = seen_at(&project, &[210, 220, 240]);
    edited(
        &mut project,
        json!({"edit": "trim", "timeline": "t1", "clip": "k1", "edge": "in", "delta": 10,
            "ripple": true}),
    );
    // It still starts at 200, ten frames shorter, and shows there what it showed at 210.
    assert_eq!(x_keys(&project, "k1"), [(-10, 0.0), (30, 1.0)]);
    assert_eq!(seen_at(&project, &[200, 210, 230]), before);
}

#[test]
fn the_halves_of_a_colour_cut_by_the_blade_play_as_it_did() {
    let frames: Vec<i64> = (200..250).step_by(5).collect();
    let mut project = with_a_colour_that_moves();
    let before = seen_at(&project, &frames);
    edited(
        &mut project,
        json!({"edit": "blade", "timeline": "t1", "at": 215, "tracks": ["V1"]}),
    );
    let clips = &project.timelines[0].tracks[0].clips;
    assert_eq!(clips.len(), 3);
    assert_eq!(x_keys(&project, "k1"), [(0, 0.0), (40, 1.0)]);
    let second = clips.iter().find(|c| c.start == 215).unwrap().id.clone();
    assert_eq!(x_keys(&project, &second), [(-15, 0.0), (25, 1.0)]);
    assert_eq!(seen_at(&project, &frames), before);
}

#[test]
fn a_roll_leaves_the_keys_of_the_colours_at_both_sides_where_they_were() {
    let mut project = with_a_colour_that_moves();
    // A second colour, which meets the first at 250.
    let mut next = Clip::new(
        "k2",
        Source::Color {
            color: Rgba([0, 0, 255, 255]),
        },
        250,
        50,
    );
    next.transform.x = Param::Keys(vec![key(0, -1.0), key(20, 0.0)]);
    project.timelines[0].tracks[0].clips.push(next);
    project.validate().unwrap();
    let frames = [205, 235, 262, 270, 290];
    let before = seen_at(&project, &frames);
    edited(
        &mut project,
        json!({"edit": "roll", "timeline": "t1", "left": "k1", "right": "k2", "delta": 8}),
    );
    assert_eq!(x_keys(&project, "k1"), [(0, 0.0), (40, 1.0)]);
    assert_eq!(x_keys(&project, "k2"), [(-8, -1.0), (12, 0.0)]);
    assert_eq!(seen_at(&project, &frames), before);
}

#[test]
fn a_still_keeps_its_keys_at_their_frames_of_the_timeline_and_media_on_its_pictures() {
    // c1 shows m1 from its start. As a still it counts its own frames.
    let keys = Param::Keys(vec![key(20, 0.0), key(60, 1.0)]);
    let trim = json!({"edit": "trim", "timeline": "t1", "clip": "c1", "edge": "in", "delta": 10,
        "linked": false});
    let mut still = Project::sample();
    still.media[0].facts.video.as_mut().unwrap().still = true;
    still.timelines[0].tracks[0].clips[0].transform.x = keys.clone();
    edited(&mut still, trim.clone());
    assert_eq!(x_keys(&still, "c1"), [(10, 0.0), (50, 1.0)]);
    assert_eq!(seen_at(&still, &[40]), [(0.5, 0.0)]);
    // As moving pictures, the keys are of its pictures, and are not moved: picture 40 is shown
    // at frame 40 still.
    let mut moving = Project::sample();
    moving.timelines[0].tracks[0].clips[0].transform.x = keys;
    edited(&mut moving, trim);
    assert_eq!(x_keys(&moving, "c1"), [(20, 0.0), (60, 1.0)]);
    assert_eq!(seen_at(&moving, &[40]), [(0.5, 0.0)]);
}
