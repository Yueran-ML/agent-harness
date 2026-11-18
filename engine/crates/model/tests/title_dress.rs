//! The stroke, the shadow, the box and the runs of the style of a title: how they are written
//! and read (a title without them as it always was), what is refused, the keys of their lengths
//! by their paths ([`Op::SetClipKeys`], [`Op::SpliceClipKeys`]), and how those keys are read at
//! a frame and moved with the clip's own.

use columbina_model::grade::{Interp, Key, Param};
use columbina_model::resolve::{video_at, Content};
use columbina_model::{
    apply, apply_done, Clip, Op, OpError, Project, Rgba, Source, Styled, TextAlign,
    TitleBackground, TitleShadow, TitleSpan, TitleStroke, TitleStyle,
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

/// White letters 72 pixels high, anchored at the middle of the picture, with nothing about them.
fn plain() -> TitleStyle {
    TitleStyle {
        font: String::new(),
        size: 72.0.into(),
        color: Rgba([255, 255, 255, 255]),
        x: 0.5.into(),
        y: 0.5.into(),
        align: TextAlign::Center,
        stroke: None,
        shadow: None,
        background: None,
        spans: Vec::new(),
    }
}

/// [`plain`] with a stroke 4 wide, a shadow moved 4 and 4 and blurred by 4, a box 16 beyond the
/// lines with corners of 8, and its first two characters red, half as large again and bold.
fn dressed() -> TitleStyle {
    TitleStyle {
        stroke: Some(TitleStroke {
            color: Rgba([0, 0, 0, 255]),
            width: 4.0.into(),
        }),
        shadow: Some(TitleShadow {
            color: Rgba([0, 0, 0, 128]),
            x: 4.0.into(),
            y: 4.0.into(),
            blur: 4.0.into(),
        }),
        background: Some(TitleBackground {
            color: Rgba([32, 32, 32, 192]),
            padding: 16.0.into(),
            radius: 8.0.into(),
        }),
        spans: vec![TitleSpan {
            from: 0,
            to: 2,
            color: Some(Rgba([255, 0, 0, 255])),
            scale: Some(1.5),
            bold: true,
            italic: false,
            font: None,
        }],
        ..plain()
    }
}

/// The sample with a title `k1` that says `text` on V1, from frame 200 for 50 frames.
fn with_title(text: &str, style: TitleStyle) -> Project {
    let mut project = Project::sample();
    let source = Source::Title {
        text: text.into(),
        style,
    };
    project.timelines[0].tracks[0]
        .clips
        .push(Clip::new("k1", source, 200, 50));
    project.next_id = 10;
    project.validate().unwrap();
    project
}

fn style_of(project: &Project) -> &TitleStyle {
    let clip = project.timelines[0].tracks[0]
        .clips
        .iter()
        .find(|c| c.id == "k1")
        .unwrap();
    match &clip.source {
        Source::Title { style, .. } => style,
        other => panic!("k1 is no title: {other:?}"),
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

/// `set_clip` of k1 to a title that says `text` in `style`.
fn set_source(text: &str, style: &Value) -> Op {
    serde_json::from_value(json!({
        "op": "set_clip", "timeline": "t1", "id": "k1",
        "source": {"kind": "title", "text": text, "style": style},
    }))
    .unwrap()
}

fn set_keys(path: &str, param: Param) -> Op {
    Op::SetClipKeys {
        timeline: "t1".into(),
        id: "k1".into(),
        path: path.into(),
        param,
    }
}

/// Apply `op`, check that its inverse takes it back, and give the inverse.
fn applied(project: &mut Project, op: Op) -> Op {
    let before = project.clone();
    let (done, inverse) = apply_done(project, op).unwrap();
    let mut undone = project.clone();
    let again = apply(&mut undone, inverse.clone()).unwrap();
    assert_eq!(undone, before);
    assert_eq!(again, done);
    inverse
}

fn refused(project: &mut Project, op: Op) -> Value {
    let before = project.clone();
    let error = apply(project, op).unwrap_err();
    assert_eq!(*project, before);
    reason(&error)
}

#[test]
fn a_style_with_a_stroke_a_shadow_a_box_and_runs_is_written_as_it_was_read() {
    let written = json!({
        "font": "", "size": 72, "color": "#ffffffff", "x": 0.5, "y": 0.5, "align": "center",
        "stroke": {"color": "#000000ff", "width": 4.0},
        "shadow": {"color": "#00000080", "x": 4.0, "y": {"keys": [
            {"frame": 0, "value": 4.0, "interp": "linear"},
            {"frame": 24, "value": -8.0, "interp": "linear"},
        ]}, "blur": 2.5},
        "background": {"color": "#202020c0", "padding": 16.0, "radius": 8.0},
        "spans": [
            {"from": 0, "to": 2, "color": "#ff0000ff", "scale": 1.5, "bold": true},
            {"from": 3, "to": 5, "italic": true, "font": "Georgia"},
        ],
    });
    let style: TitleStyle = serde_json::from_value(written.clone()).unwrap();
    assert_eq!(serde_json::to_value(&style).unwrap(), written);
    assert_eq!(
        style.spans[1],
        TitleSpan {
            from: 3,
            to: 5,
            color: None,
            scale: None,
            bold: false,
            italic: true,
            font: Some("Georgia".into()),
        }
    );
    assert_eq!(
        style.shadow.as_ref().map(|shadow| shadow.y.clone()),
        Some(Param::Keys(vec![key(0, 4.0), key(24, -8.0)]))
    );
    // Colours are read in either case and written in lowercase; what a run does not set, bold
    // and italic where they are not set, and runs where there are none, are left out.
    let read: TitleStyle = serde_json::from_value(json!({
        "font": "", "size": 72, "color": "#ffffffff", "x": 0.5, "y": 0.5, "align": "center",
        "stroke": {"color": "#FF00AAFF", "width": 2},
        "spans": [{"from": 0, "to": 1, "bold": false, "italic": false}],
    }))
    .unwrap();
    let again = serde_json::to_value(&read).unwrap();
    assert_eq!(again["stroke"], json!({"color": "#ff00aaff", "width": 2.0}));
    assert_eq!(again["spans"], json!([{"from": 0, "to": 1}]));
    let none: TitleStyle = serde_json::from_value(json!({
        "font": "", "size": 72, "color": "#ffffffff", "x": 0.5, "y": 0.5, "align": "center",
        "spans": [],
    }))
    .unwrap();
    assert_eq!(none, plain());
    // Each is whole, and knows its fields.
    for (part, value) in [
        ("stroke", json!({"color": "#000000ff"})),
        (
            "stroke",
            json!({"color": "#000000ff", "width": 1, "join": "round"}),
        ),
        ("shadow", json!({"color": "#000000ff", "x": 1, "y": 1})),
        ("background", json!({"padding": 1, "radius": 1})),
        ("spans", json!([{"from": 0, "to": 1, "underline": true}])),
        ("spans", json!([{"from": -1, "to": 1}])),
    ] {
        let mut style = serde_json::to_value(plain()).unwrap();
        style[part] = value.clone();
        let read = serde_json::from_value::<TitleStyle>(style);
        assert!(read.is_err(), "{part} {value}: {read:?}");
    }
}

#[test]
fn a_title_without_them_is_written_as_it_always_was_to_the_byte() {
    let project = with_title("Title", plain());
    let source = &project.timelines[0].tracks[0].clips[1].source;
    assert_eq!(
        serde_json::to_string(source).unwrap(),
        concat!(
            r##"{"kind":"title","text":"Title","style":{"font":"","size":72,"##,
            r##""color":"#ffffffff","x":0.5,"y":0.5,"align":"center"}}"##
        )
    );
    // A project written so is read and written again as the same bytes.
    let text = serde_json::to_string(&project).unwrap();
    let read = Project::from_value(serde_json::from_str(&text).unwrap()).unwrap();
    assert_eq!(read, project);
    assert_eq!(serde_json::to_string(&read).unwrap(), text);
    // And it is drawn with nothing about its letters.
    let styled = styled_at(&project, 210);
    assert_eq!(
        (styled.stroke, styled.shadow, styled.background),
        (None, None, None)
    );
    assert_eq!(styled.spans, Vec::new());
}

#[test]
#[allow(clippy::too_many_lines, reason = "a refusal for each way to be wrong")]
fn lengths_beyond_their_ranges_and_runs_not_within_the_text_in_order_are_refused() {
    let mut project = with_title("Hello", plain());
    let base = serde_json::to_value(plain()).unwrap();
    let with = |part: &str, value: Value| {
        let mut style = base.clone();
        style[part] = value;
        style
    };
    let black = "#000000ff";
    for (part, value) in [
        ("stroke", json!({"color": black, "width": -1})),
        ("stroke", json!({"color": black, "width": 256.5})),
        (
            "stroke",
            json!({"color": black, "width": {"keys": [
                {"frame": 0, "value": 4, "interp": "linear"},
                {"frame": 9, "value": 300, "interp": "linear"},
            ]}}),
        ),
        (
            "shadow",
            json!({"color": black, "x": 1024.5, "y": 0, "blur": 0}),
        ),
        (
            "shadow",
            json!({"color": black, "x": 0, "y": -1025, "blur": 0}),
        ),
        (
            "shadow",
            json!({"color": black, "x": 0, "y": 0, "blur": 257}),
        ),
        (
            "shadow",
            json!({"color": black, "x": 0, "y": 0, "blur": -0.5}),
        ),
        (
            "background",
            json!({"color": black, "padding": 1025, "radius": 0}),
        ),
        (
            "background",
            json!({"color": black, "padding": 0, "radius": 512.5}),
        ),
        (
            "background",
            json!({"color": black, "padding": -1, "radius": 0}),
        ),
        // Of no character, beyond the text, overlapping, out of order.
        ("spans", json!([{"from": 2, "to": 2}])),
        ("spans", json!([{"from": 3, "to": 6}])),
        ("spans", json!([{"from": 2, "to": 4}, {"from": 3, "to": 5}])),
        ("spans", json!([{"from": 3, "to": 5}, {"from": 0, "to": 2}])),
        // Of a size beyond its range.
        ("spans", json!([{"from": 0, "to": 1, "scale": 0.05}])),
        ("spans", json!([{"from": 0, "to": 1, "scale": 10.5}])),
    ] {
        assert_eq!(
            refused(
                &mut project,
                set_source("Hello", &with(part, value.clone()))
            ),
            json!({"reason": "out_of_range", "what": part}),
            "{part} {value}"
        );
    }
    // At their bounds they are taken.
    for (part, value) in [
        ("stroke", json!({"color": black, "width": 0})),
        ("stroke", json!({"color": black, "width": 256})),
        (
            "shadow",
            json!({"color": black, "x": -1024, "y": 1024, "blur": 256}),
        ),
        (
            "background",
            json!({"color": black, "padding": 1024, "radius": 512}),
        ),
        (
            "spans",
            json!([{"from": 0, "to": 2, "scale": 0.1}, {"from": 2, "to": 5, "scale": 10}]),
        ),
    ] {
        applied(&mut project, set_source("Hello", &with(part, value)));
    }
    // A title has at most 256 runs.
    let long = "x".repeat(300);
    let runs = |count: usize| -> Value {
        (0..count)
            .map(|at| json!({"from": at, "to": at + 1}))
            .collect()
    };
    applied(&mut project, set_source(&long, &with("spans", runs(256))));
    assert_eq!(
        refused(&mut project, set_source(&long, &with("spans", runs(257)))),
        json!({"reason": "out_of_range", "what": "spans"})
    );
    // Runs count characters, not bytes.
    applied(
        &mut project,
        set_source("标题", &with("spans", json!([{"from": 0, "to": 2}]))),
    );
    // A text set shorter than its runs reach is refused: the runs are sent with it.
    let runs = with("spans", json!([{"from": 3, "to": 5}]));
    applied(&mut project, set_source("Hello", &runs));
    assert_eq!(
        refused(&mut project, set_source("Hel", &runs)),
        json!({"reason": "out_of_range", "what": "spans"})
    );
    applied(&mut project, set_source("Hel", &base));
}

#[test]
fn the_lengths_of_a_stroke_a_shadow_and_a_box_take_keys_where_the_title_has_them() {
    let parts = [
        ("title.stroke.width", 4.0),
        ("title.shadow.x", 4.0),
        ("title.shadow.y", 4.0),
        ("title.shadow.blur", 4.0),
        ("title.background.padding", 16.0),
        ("title.background.radius", 8.0),
    ];
    // A title without them has none of their lengths.
    let mut project = with_title("Hello", plain());
    for (path, _) in parts {
        assert_eq!(
            refused(&mut project, set_keys(path, Param::Fixed(4.0))),
            json!({"reason": "missing", "what": "setting", "id": path}),
            "{path}"
        );
    }
    // One with them: each is keyed apart from the others, and taken back to the number.
    let mut project = with_title("Hello", dressed());
    let keyed = Param::Keys(vec![key(0, 2.0), key(20, 12.0)]);
    for (path, was) in parts {
        let inverse = applied(&mut project, set_keys(path, keyed.clone()));
        assert_eq!(inverse, set_keys(path, Param::Fixed(was)), "{path}");
    }
    let style = style_of(&project);
    assert_eq!(
        style.stroke.as_ref().map(|stroke| stroke.width.clone()),
        Some(keyed.clone())
    );
    assert!(style.is_animated());
    // A key spliced after the others.
    let splice = Op::SpliceClipKeys {
        timeline: "t1".into(),
        id: "k1".into(),
        path: "title.shadow.x".into(),
        at: 2,
        remove: 0,
        keys: vec![key(40, 0.0)],
    };
    applied(&mut project, splice);
    // At frame 10 of the title, halfway between its first two keys.
    let styled = styled_at(&project, 210);
    assert_eq!(
        styled.stroke,
        Some(TitleStroke {
            color: Rgba([0, 0, 0, 255]),
            width: 7.0,
        })
    );
    assert_eq!(
        styled.shadow,
        Some(TitleShadow {
            color: Rgba([0, 0, 0, 128]),
            x: 7.0,
            y: 7.0,
            blur: 7.0,
        })
    );
    assert_eq!(
        styled.background,
        Some(TitleBackground {
            color: Rgba([32, 32, 32, 192]),
            padding: 7.0,
            radius: 7.0,
        })
    );
    assert_eq!(styled.spans, dressed().spans);
    // Out of its range, a key is refused as the part is.
    for (path, param, what) in [
        (
            "title.stroke.width",
            Param::Keys(vec![key(0, 2.0), key(9, 300.0)]),
            "stroke",
        ),
        ("title.shadow.blur", Param::Fixed(-1.0), "shadow"),
        ("title.shadow.x", Param::Fixed(f64::NAN), "shadow"),
        ("title.background.radius", Param::Fixed(600.0), "background"),
    ] {
        assert_eq!(
            refused(&mut project, set_keys(path, param.clone())),
            json!({"reason": "out_of_range", "what": what}),
            "{path} {param:?}"
        );
    }
}

#[test]
fn the_keys_of_the_parts_of_a_style_move_with_its_others_and_its_runs_stay() {
    let mut style = dressed();
    assert!(!style.is_animated());
    if let Some(shadow) = &mut style.shadow {
        shadow.blur = Param::Keys(vec![key(0, 1.0), key(10, 2.0)]);
    }
    // A style whose only keys are those of its shadow has keys.
    assert!(style.is_animated());
    if let Some(stroke) = &mut style.stroke {
        stroke.width = Param::Keys(vec![key(0, 1.0), key(10, 2.0)]);
    }
    if let Some(back) = &mut style.background {
        back.radius = Param::Keys(vec![key(5, 3.0)]);
    }
    let moved = style.with_key_frames(&|frame| frame + 5);
    let ten = Param::Keys(vec![key(5, 1.0), key(15, 2.0)]);
    assert_eq!(
        moved.stroke.as_ref().map(|stroke| stroke.width.clone()),
        Some(ten.clone())
    );
    assert_eq!(
        moved.shadow.as_ref().map(|shadow| shadow.blur.clone()),
        Some(ten)
    );
    assert_eq!(
        moved.background.as_ref().map(|back| back.radius.clone()),
        Some(Param::Keys(vec![key(10, 3.0)]))
    );
    assert_eq!(moved.spans, style.spans);
    assert_eq!(moved.with_key_frames(&|frame| frame - 5), style);
    // A trim of the head of the title leaves each at its frame of the timeline.
    let mut project = with_title("Hello", style);
    let before = styled_at(&project, 220);
    let edit: columbina_model::Edit = serde_json::from_value(json!({
        "edit": "trim", "timeline": "t1", "clip": "k1", "edge": "in", "delta": 10,
    }))
    .unwrap();
    columbina_model::expand(&mut project, edit).unwrap();
    assert_eq!(
        style_of(&project)
            .stroke
            .as_ref()
            .map(|stroke| stroke.width.clone()),
        Some(Param::Keys(vec![key(-10, 1.0), key(0, 2.0)]))
    );
    assert_eq!(styled_at(&project, 220), before);
}
