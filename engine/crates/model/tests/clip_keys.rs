//! The keys of the settings of a clip (schema 5's transform): the operations that set them by
//! their paths ([`Op::SetClipKeys`], [`Op::SpliceClipKeys`]), what takes them back, what they
//! refuse, and where a keyed transform puts the picture at a frame of the timeline. And those
//! of the style of a title, the size of its letters and the place of its anchor, which are set
//! by the same operations on a clip that is a title.

use columbina_model::catalog;
use columbina_model::grade::{Interp, Key, Param};
use columbina_model::resolve::{video_at, Content};
use columbina_model::{
    apply, apply_done, Clip, Op, OpError, Placed, Project, Rgba, Source, Speed, Styled, TextAlign,
    TitleStyle, Transform,
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

fn set_of(id: &str, path: &str, param: Param) -> Op {
    Op::SetClipKeys {
        timeline: "t1".into(),
        id: id.into(),
        path: path.into(),
        param,
    }
}

fn set(path: &str, param: Param) -> Op {
    set_of("c1", path, param)
}

fn splice_of(id: &str, path: &str, at: usize, remove: usize, keys: Vec<Key>) -> Op {
    Op::SpliceClipKeys {
        timeline: "t1".into(),
        id: id.into(),
        path: path.into(),
        at,
        remove,
        keys,
    }
}

fn splice(path: &str, at: usize, remove: usize, keys: Vec<Key>) -> Op {
    splice_of("c1", path, at, remove, keys)
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
            "crop.softness",
            "title.size",
            "title.x",
            "title.y"
        ]
    );
    for entry in catalog::clip() {
        starts_as_listed(entry);
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

// The style of a title: the size of its letters and the place of its anchor take keys.

/// Check that what a new clip has is what the catalog says it has at first, and that the
/// setting is taken at both its bounds: on the clip of media of the sample, or, for a setting
/// that only a title has, on a new title.
fn starts_as_listed(entry: &catalog::Entry) {
    let (mut probe, id) = if entry.path.starts_with("title.") {
        (with_a_title(), "k1")
    } else {
        (Project::sample(), "c1")
    };
    let high = set_of(id, &entry.path, Param::Fixed(entry.high));
    let (_, inverse) = applied(&mut probe, high);
    let first = set_of(id, &entry.path, Param::Fixed(entry.default));
    assert_eq!(inverse, first);
    applied(&mut probe, set_of(id, &entry.path, Param::Fixed(entry.low)));
}

#[test]
fn the_catalog_lists_the_size_of_a_title_in_pixels_under_the_title_of_the_inspector() {
    let size = catalog::of_clip("title.size").unwrap();
    assert_eq!((size.default, size.low, size.high), (72.0, 1.0, 4096.0));
    assert_eq!(
        serde_json::to_value(size).unwrap(),
        json!({
            "path": "title.size", "default": 72.0, "low": 1.0, "high": 4096.0,
            "unit": "pixels", "page": "edit", "palette": "title", "clock": "source_frame",
        })
    );
    // Its anchor is a share of the picture, at the middle at first, and may be keyed from far
    // beyond one edge to far beyond the other.
    for (path, unit) in [("title.x", "widths"), ("title.y", "heights")] {
        let entry = catalog::of_clip(path).unwrap();
        assert_eq!((entry.default, entry.low, entry.high), (0.5, -16.0, 16.0));
        assert_eq!(serde_json::to_value(entry.unit).unwrap(), json!(unit));
    }
}

/// The sample with a title `k1` on V1 from frame 200 for 50 frames, as a client makes a new
/// one: letters 72 pixels high, anchored at the middle of the picture.
fn with_a_title() -> Project {
    let mut project = Project::sample();
    let title = Source::Title {
        text: "Title".into(),
        style: TitleStyle {
            font: String::new(),
            size: 72.0.into(),
            color: Rgba([255, 255, 255, 255]),
            x: 0.5.into(),
            y: 0.5.into(),
            align: TextAlign::Center,
        },
    };
    project.timelines[0].tracks[0]
        .clips
        .push(Clip::new("k1", title, 200, 50));
    project.next_id = 10;
    project.validate().unwrap();
    project
}

fn style_of<'p>(project: &'p Project, id: &str) -> &'p TitleStyle {
    let clip = project.timelines[0].tracks[0]
        .clips
        .iter()
        .find(|c| c.id == id)
        .unwrap();
    match &clip.source {
        Source::Title { style, .. } => style,
        other => panic!("{id} is no title: {other:?}"),
    }
}

/// How the title that V1 shows at a frame is drawn there.
fn styled_at(project: &Project, frame: i64) -> Styled {
    let layers = video_at(&project.timelines[0], project, frame);
    assert_eq!(layers.len(), 1, "{frame}");
    match &layers[0].content {
        Content::Title { style, .. } => style.clone(),
        other => panic!("frame {frame} shows no title: {other:?}"),
    }
}

#[test]
fn the_style_of_a_title_is_written_as_numbers_where_it_has_no_keys_and_read_with_keys() {
    let mut project = with_a_title();
    // A title with no keys is written as it was before its style could have them, to the byte:
    // its size a whole number with no fraction.
    let source = &project.timelines[0].tracks[0].clips[1].source;
    assert_eq!(
        serde_json::to_string(source).unwrap(),
        concat!(
            r##"{"kind":"title","text":"Title","style":{"font":"","size":72,"##,
            r##""color":"#ffffffff","x":0.5,"y":0.5,"align":"center"}}"##
        )
    );
    // A size written with a fraction of nothing is read as the same, and written without it.
    let read: Source = serde_json::from_value(json!({
        "kind": "title", "text": "Title",
        "style": {"font": "", "size": 72.0, "color": "#ffffffff", "x": 0.5, "y": 0.5, "align": "center"},
    }))
    .unwrap();
    assert_eq!(&read, source);
    assert_eq!(
        serde_json::to_value(&read).unwrap()["style"]["size"],
        json!(72)
    );
    // One that is not a whole number is a size too, as a key between two others leaves it.
    applied(&mut project, set_of("k1", "title.size", Param::Fixed(72.5)));
    let written = serde_json::to_value(style_of(&project, "k1")).unwrap();
    assert_eq!(written["size"], json!(72.5));
    applied(
        &mut project,
        set_of(
            "k1",
            "title.size",
            Param::Keys(vec![key(0, 64.0), key(40, 128.0)]),
        ),
    );
    applied(
        &mut project,
        set_of("k1", "title.y", Param::Keys(vec![key(10, 0.25)])),
    );
    let written = serde_json::to_value(&project).unwrap();
    let style = &written["timelines"][0]["tracks"][0]["clips"][1]["source"]["style"];
    assert_eq!(
        *style,
        json!({
            "font": "",
            "size": {"keys": [
                {"frame": 0, "value": 64.0, "interp": "linear"},
                {"frame": 40, "value": 128.0, "interp": "linear"},
            ]},
            "color": "#ffffffff",
            "x": 0.5,
            "y": {"keys": [{"frame": 10, "value": 0.25, "interp": "linear"}]},
            "align": "center",
        })
    );
    assert_eq!(Project::from_value(written).unwrap(), project);
}

#[test]
fn the_keys_of_the_style_of_a_title_are_set_whole_and_spliced_and_each_is_taken_back() {
    let mut project = with_a_title();
    // The first keys of a number are done as the keys set whole, and taken back as the number.
    let (done, inverse) = applied(
        &mut project,
        splice_of("k1", "title.size", 0, 0, vec![key(10, 96.0)]),
    );
    assert_eq!(
        done,
        set_of("k1", "title.size", Param::Keys(vec![key(10, 96.0)]))
    );
    assert_eq!(inverse, set_of("k1", "title.size", Param::Fixed(72.0)));
    // A key put after it, and the first moved and changed.
    let (_, inverse) = applied(
        &mut project,
        splice_of("k1", "title.size", 1, 0, vec![key(20, 144.0)]),
    );
    assert_eq!(inverse, splice_of("k1", "title.size", 1, 1, vec![]));
    let (_, inverse) = applied(
        &mut project,
        splice_of("k1", "title.size", 0, 1, vec![key(5, 80.5)]),
    );
    assert_eq!(
        inverse,
        splice_of("k1", "title.size", 0, 1, vec![key(10, 96.0)])
    );
    assert_eq!(
        style_of(&project, "k1").size,
        Param::Keys(vec![key(5, 80.5), key(20, 144.0)])
    );
    // The place of the anchor, across and down, each apart from the other.
    let (_, inverse) = applied(
        &mut project,
        set_of(
            "k1",
            "title.x",
            Param::Keys(vec![key(0, -0.5), key(49, 1.5)]),
        ),
    );
    assert_eq!(inverse, set_of("k1", "title.x", Param::Fixed(0.5)));
    applied(&mut project, set_of("k1", "title.y", Param::Fixed(0.875)));
    // A number in place of the keys.
    let (_, inverse) = applied(&mut project, set_of("k1", "title.size", Param::Fixed(40.0)));
    assert_eq!(
        inverse,
        set_of(
            "k1",
            "title.size",
            Param::Keys(vec![key(5, 80.5), key(20, 144.0)])
        )
    );
    let style = style_of(&project, "k1");
    assert_eq!(
        (&style.size, &style.y),
        (&Param::Fixed(40.0), &Param::Fixed(0.875))
    );
    assert!(style.is_animated());
}

#[test]
fn the_settings_of_a_title_are_those_of_a_title_only_and_are_checked_as_its_style_is() {
    let mut project = with_a_title();
    // A clip of media has no style of a title, and neither has a colour: as a path that the
    // catalog does not list.
    let colour = Clip::new(
        "k2",
        Source::Color {
            color: Rgba([255, 0, 0, 255]),
        },
        300,
        50,
    );
    project.timelines[0].tracks[0].clips.push(colour);
    project.validate().unwrap();
    for id in ["c1", "k2"] {
        for path in ["title.size", "title.x", "title.y"] {
            let missing = json!({"reason": "missing", "what": "setting", "id": path});
            assert_eq!(
                refused(&mut project, set_of(id, path, Param::Fixed(0.5))),
                missing,
                "{id} {path}"
            );
            assert_eq!(
                refused(&mut project, splice_of(id, path, 0, 0, vec![key(0, 1.0)])),
                missing,
                "{id} {path}"
            );
        }
    }
    // What a title has under another name takes no keys.
    for path in [
        "title",
        "title.color",
        "title.font",
        "title.align",
        "title.text",
        "style.size",
    ] {
        assert_eq!(
            refused(&mut project, set_of("k1", path, Param::Fixed(0.5))),
            json!({"reason": "missing", "what": "setting", "id": path}),
            "{path}"
        );
    }
    let out = json!({"reason": "out_of_range", "what": "size"});
    for param in [
        Param::Fixed(0.5),
        Param::Fixed(4096.5),
        Param::Fixed(f64::NAN),
        Param::Keys(vec![key(0, 64.0), key(9, 0.0)]),
        Param::Keys(vec![key(0, 4097.0)]),
        // Keys are at frames in order, and there is one at least.
        Param::Keys(vec![key(9, 64.0), key(9, 72.0)]),
        Param::Keys(vec![]),
    ] {
        assert_eq!(
            refused(&mut project, set_of("k1", "title.size", param.clone())),
            out,
            "{param:?}"
        );
    }
    // The least and the most are sizes.
    applied(&mut project, set_of("k1", "title.size", Param::Fixed(1.0)));
    applied(
        &mut project,
        set_of(
            "k1",
            "title.size",
            Param::Keys(vec![key(0, 1.0), key(9, 4096.0)]),
        ),
    );
    assert_eq!(
        refused(
            &mut project,
            splice_of("k1", "title.size", 2, 0, vec![key(5, 64.0)])
        ),
        out
    );
    let out = json!({"reason": "out_of_range", "what": "position"});
    for (path, param) in [
        ("title.x", Param::Fixed(f64::INFINITY)),
        ("title.y", Param::Keys(vec![key(0, 0.5), key(9, f64::NAN)])),
    ] {
        assert_eq!(
            refused(&mut project, set_of("k1", path, param.clone())),
            out,
            "{path} {param:?}"
        );
    }
    // The anchor may be far beyond the picture.
    applied(&mut project, set_of("k1", "title.x", Param::Fixed(-40.0)));
}

#[test]
fn set_clip_puts_a_style_with_keys_whole_and_puts_back_the_one_that_was() {
    let mut project = with_a_title();
    let before = project.timelines[0].tracks[0].clips[1].source.clone();
    let op: Op = serde_json::from_value(json!({
        "op": "set_clip", "timeline": "t1", "id": "k1",
        "source": {"kind": "title", "text": "Title", "style": {
            "font": "", "size": 48, "color": "#ffffffff", "align": "left", "y": 0.5,
            "x": {"keys": [
                {"frame": 0, "value": 0, "interp": "linear"},
                {"frame": 40, "value": 1, "interp": "hold"},
            ]},
        }},
    }))
    .unwrap();
    let (_, inverse) = applied(&mut project, op);
    let style = style_of(&project, "k1");
    assert_eq!(style.size, Param::Fixed(48.0));
    assert_eq!(
        style.x,
        Param::Keys(vec![
            key(0, 0.0),
            Key {
                frame: 40,
                value: 1.0,
                interp: Interp::Hold
            }
        ])
    );
    let Op::SetClip { source, .. } = inverse else {
        panic!("{inverse:?}")
    };
    assert_eq!(source, Some(before));
    // A style that is whole numbers again is set as it always was.
    let plain: Op = serde_json::from_value(json!({
        "op": "set_clip", "timeline": "t1", "id": "k1",
        "source": {"kind": "title", "text": "Title", "style": {
            "font": "", "size": 72, "color": "#ffffffff", "x": 0.5, "y": 0.5, "align": "center",
        }},
    }))
    .unwrap();
    applied(&mut project, plain);
    assert_eq!(project, with_a_title());
    // And one whose keys are not those of a size is refused as the size is.
    let wrong: Op = serde_json::from_value(json!({
        "op": "set_clip", "timeline": "t1", "id": "k1",
        "source": {"kind": "title", "text": "Title", "style": {
            "font": "", "color": "#ffffffff", "x": 0.5, "y": 0.5, "align": "center",
            "size": {"keys": [{"frame": 0, "value": 5000, "interp": "linear"}]},
        }},
    }))
    .unwrap();
    assert_eq!(
        refused(&mut project, wrong),
        json!({"reason": "out_of_range", "what": "size"})
    );
}

#[test]
fn a_keyed_style_is_at_each_frame_what_its_keys_are_at_that_frame_of_the_title() {
    let mut project = with_a_title();
    for (path, keys) in [
        ("title.size", vec![key(0, 64.0), key(40, 128.0)]),
        ("title.y", vec![key(10, 0.25), key(30, 0.75)]),
        (
            "title.x",
            vec![
                Key {
                    frame: 0,
                    value: 0.25,
                    interp: Interp::Hold,
                },
                key(20, 0.75),
            ],
        ),
    ] {
        apply(&mut project, set_of("k1", path, Param::Keys(keys))).unwrap();
    }
    // The title begins at frame 200 of the timeline, and its keys count from there.
    for (frame, size, x, y) in [
        (200, 64.0, 0.25, 0.25),
        (210, 80.0, 0.25, 0.25),
        (215, 88.0, 0.25, 0.375),
        (219, 94.4, 0.25, 0.475),
        (220, 96.0, 0.75, 0.5),
        (230, 112.0, 0.75, 0.75),
        (240, 128.0, 0.75, 0.75),
        (249, 128.0, 0.75, 0.75),
    ] {
        let styled = styled_at(&project, frame);
        assert!(
            (styled.size - size).abs() < 1e-9,
            "{frame}: {}",
            styled.size
        );
        assert!((styled.x - x).abs() < 1e-12, "{frame}: {}", styled.x);
        assert!((styled.y - y).abs() < 1e-12, "{frame}: {}", styled.y);
        // What takes no keys is as the style has it.
        assert_eq!(
            (styled.font.as_str(), styled.color, styled.align),
            ("", Rgba([255, 255, 255, 255]), TextAlign::Center)
        );
    }
    // A style with no keys is the same numbers at every frame.
    let plain = with_a_title();
    for frame in [200, 249] {
        let styled = styled_at(&plain, frame);
        assert_eq!((styled.size, styled.x, styled.y), (72.0, 0.5, 0.5));
    }
}

/// The sample with a title whose letters grow over its first 40 frames and whose anchor goes
/// right from its frame 10 to its frame 20.
fn with_a_title_that_grows() -> Project {
    let mut project = with_a_title();
    for (path, keys) in [
        ("title.size", vec![key(0, 64.0), key(40, 128.0)]),
        ("title.x", vec![key(10, 0.25), key(20, 0.75)]),
    ] {
        apply(&mut project, set_of("k1", path, Param::Keys(keys))).unwrap();
    }
    project
}

/// The size and the place across of what V1 shows at each of `frames`.
fn styled_over(project: &Project, frames: &[i64]) -> Vec<(f64, f64)> {
    frames
        .iter()
        .map(|&frame| {
            let styled = styled_at(project, frame);
            (styled.size, styled.x)
        })
        .collect()
}

fn frames_of(param: &Param) -> Vec<i64> {
    match param {
        Param::Keys(keys) => keys.iter().map(|k| k.frame).collect(),
        Param::Fixed(_) => vec![],
    }
}

#[test]
fn a_trim_of_the_head_of_a_title_leaves_the_keys_of_its_style_at_their_frames_of_the_timeline() {
    let frames = [210, 215, 220, 230, 245];
    let mut project = with_a_title_that_grows();
    let before = styled_over(&project, &frames);
    edited(
        &mut project,
        json!({"edit": "trim", "timeline": "t1", "clip": "k1", "edge": "in", "delta": 10}),
    );
    let style = style_of(&project, "k1");
    assert_eq!(frames_of(&style.size), [-10, 30]);
    assert_eq!(frames_of(&style.x), [0, 10]);
    // What takes no keys is left as it was.
    assert_eq!(style.y, Param::Fixed(0.5));
    assert_eq!(styled_over(&project, &frames), before);
    // Its head moved out again, by more than it was moved in.
    edited(
        &mut project,
        json!({"edit": "trim", "timeline": "t1", "clip": "k1", "edge": "in", "delta": -30}),
    );
    let style = style_of(&project, "k1");
    assert_eq!(frames_of(&style.size), [20, 60]);
    assert_eq!(frames_of(&style.x), [30, 40]);
    assert_eq!(styled_over(&project, &frames), before);
    // Its tail leaves its keys alone.
    edited(
        &mut project,
        json!({"edit": "trim", "timeline": "t1", "clip": "k1", "edge": "out", "delta": -5}),
    );
    assert_eq!(frames_of(&style_of(&project, "k1").size), [20, 60]);
}

#[test]
fn the_halves_of_a_title_cut_by_the_blade_are_drawn_as_it_was() {
    let frames: Vec<i64> = (200..250).step_by(5).collect();
    let mut project = with_a_title_that_grows();
    let before = styled_over(&project, &frames);
    edited(
        &mut project,
        json!({"edit": "blade", "timeline": "t1", "at": 215, "tracks": ["V1"]}),
    );
    let clips = &project.timelines[0].tracks[0].clips;
    assert_eq!(clips.len(), 3);
    let first = style_of(&project, "k1");
    assert_eq!(
        (frames_of(&first.size), frames_of(&first.x)),
        (vec![0, 40], vec![10, 20])
    );
    let second = clips.iter().find(|c| c.start == 215).unwrap().id.clone();
    let second = style_of(&project, &second);
    assert_eq!(
        (frames_of(&second.size), frames_of(&second.x)),
        (vec![-15, 25], vec![-5, 5])
    );
    assert_eq!(styled_over(&project, &frames), before);
}
