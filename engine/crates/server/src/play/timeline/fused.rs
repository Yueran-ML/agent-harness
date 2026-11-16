//! The Fusion view of the timeline monitor as its thread draws it (`viewer.fusion`,
//! `crate::fusion_view`): the picture of a node of the composition of a clip, alone, fitted to
//! the frame and not graded, in place of the frame of the timeline; black where the playhead
//! is not on the clip; drawn again as the composition is edited; shown in place of the other
//! modes of the viewer; and never in a still.

use columbina_model::fusion::{Composition, Input, Kind, Link, Node, NodeKind};
use columbina_model::Track;

use super::grades::{colour, grey, monitor, near, pixel, shown, timeline_of, ORANGE, TEAL};
use super::*;

/// Half of [`ORANGE`] and half of the blue of the Background of [`composed`]: what comes out
/// of its composition.
const MIXED: [u8; 3] = [100, 55, 148];

fn view(clip: &str, node: Option<&str>) -> FusionView {
    FusionView {
        clip: clip.into(),
        node: node.map(str::to_owned),
    }
}

/// t1 of 64 × 36. On V1, c1, orange, over the frames 0 to 39: moved, made smaller, half
/// opaque and graded grey; its composition lays it over a blue Background (`bg`) by a Blend of
/// a half (`over`), and has a Rectangle (`r`) that is not linked. Over it on V2, c2, teal, over
/// the frames 0 to 59, which covers the frame.
fn composed() -> Project {
    let mut behind = Node::new("bg", Kind::Background);
    if let NodeKind::Background(background) = &mut behind.kind {
        background.color.b = 1.0.into();
    }
    let mut over = Node::new("over", Kind::Merge);
    if let NodeKind::Merge(merge) = &mut over.kind {
        merge.blend = 0.5.into();
    }
    let mut composition = Composition {
        nodes: vec![
            Node::new("in", Kind::MediaIn),
            behind,
            over,
            Node::new("r", Kind::Rectangle),
            Node::new("out", Kind::MediaOut),
        ],
        links: vec![
            Link::new("bg", "over", Input::Background),
            Link::new("in", "over", Input::Foreground),
            Link::new("over", "out", Input::Input),
        ],
    };
    composition.sort_links();
    let mut under = colour("c1", ORANGE, 0, 40);
    under.fusion = Some(composition);
    under.grade = grey();
    under.transform.x = 0.25.into();
    under.transform.scale = 0.5.into();
    under.transform.opacity = 0.5.into();
    let mut project = timeline_of(vec![under], 64, 36);
    project.timelines[0]
        .tracks
        .push(Track::new("V2", TrackKind::Video).with_clips(vec![colour("c2", TEAL, 0, 60)]));
    project
}

/// What the monitor shows now, once the pipelines of its frame are made: R, G, B and A of 8
/// bits a pixel.
fn settled(monitor: &mut Worker) -> Vec<u8> {
    monitor.finish_compiling();
    monitor.current.as_ref().unwrap().read_rgba().unwrap()
}

#[test]
fn the_fusion_view_shows_the_picture_of_a_node_alone_fitted_and_not_graded() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut monitor = monitor(dir.path());
    let project = composed();
    // The frame of the timeline: the clip on top of it.
    let frame = shown(&mut monitor, &project, 5);
    near(pixel(&frame, 64, 32, 18), [40, 150, 170], "the timeline");
    // What comes out of the composition, over the whole frame: not the other layer, and not
    // the grade, the transform or the opacity of the clip.
    let answer = monitor.fuse(Some(view("c1", None))).unwrap();
    assert_eq!(
        answer,
        json!({"monitor": "timeline", "fusion": {
            "clip": "c1", "shown": true, "canvas": [64, 36],
            "size": {"width": 64, "height": 36},
            "picture": {"x": 0.0, "y": 0.0, "width": 64.0, "height": 36.0},
        }})
    );
    assert_eq!(json!(monitor.status)["fusion"], answer["fusion"]);
    let out = settled(&mut monitor);
    for (x, y) in [(32, 18), (2, 2), (61, 33)] {
        near(pixel(&out, 64, x, y), MIXED, "what comes out of it");
    }
    // Each node that the MediaOut reads, cut there: nothing after it.
    for (node, wanted) in [
        ("out", MIXED),
        ("over", MIXED),
        ("bg", [0, 0, 255]),
        ("in", [200, 110, 40]),
    ] {
        monitor.fuse(Some(view("c1", Some(node)))).unwrap();
        let picture = settled(&mut monitor);
        for (x, y) in [(32, 18), (2, 2)] {
            near(pixel(&picture, 64, x, y), wanted, node);
        }
        let told = monitor.status.fusion.as_ref().unwrap();
        assert_eq!(told.node.as_deref(), Some(node));
        assert!(told.shown, "{node}");
    }
    // A mask that is not linked: white where it is, over black.
    monitor.fuse(Some(view("c1", Some("r")))).unwrap();
    let mask = settled(&mut monitor);
    near(pixel(&mask, 64, 32, 18), [255; 3], "in the mask");
    near(pixel(&mask, 64, 2, 2), [0; 3], "about the mask");
    // The frame is known for what it is: its one layer is not where the timeline puts it.
    let snapshot = monitor.draw_inner(true, false).unwrap().unwrap();
    assert!(snapshot.fusion);
    assert_eq!(snapshot.layers.len(), 1);
    assert_eq!(snapshot.layers[0].clip, "c1");
    // Off: the frame of the timeline as it was, and the state says nothing of the view.
    assert_eq!(
        monitor.fuse(None).unwrap(),
        json!({"monitor": "timeline", "fusion": null})
    );
    assert_eq!(settled(&mut monitor), frame);
    assert!(json!(monitor.status).get("fusion").is_none());
    let snapshot = monitor.draw_inner(true, false).unwrap().unwrap();
    assert!(!snapshot.fusion);
    assert_eq!(snapshot.layers.len(), 2);
}

#[test]
fn the_fusion_view_is_black_off_its_clip_and_follows_an_edit_of_the_composition() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut monitor = monitor(dir.path());
    let project = composed();
    shown(&mut monitor, &project, 5);
    monitor.fuse(Some(view("c1", Some("bg")))).unwrap();
    near(pixel(&settled(&mut monitor), 64, 32, 18), [0, 0, 255], "bg");
    // The playhead beyond the clip: black, the view on, and its picture where it lay.
    let beyond = shown(&mut monitor, &project, 45);
    near(pixel(&beyond, 64, 32, 18), [0; 3], "off the clip");
    let told = monitor.status.fusion.clone().unwrap();
    assert!(!told.shown);
    assert_eq!(
        json!(told)["picture"],
        json!({"x": 0.0, "y": 0.0, "width": 64.0, "height": 36.0})
    );
    // And on it again.
    let back = shown(&mut monitor, &project, 5);
    near(pixel(&back, 64, 32, 18), [0, 0, 255], "on the clip again");
    assert!(monitor.status.fusion.as_ref().unwrap().shown);
    // An edit of the composition is drawn: the Background made green.
    let mut edited = project.clone();
    let composition = edited.timelines[0].tracks[0].clips[0].fusion.as_mut();
    if let NodeKind::Background(background) = &mut composition.unwrap().nodes[1].kind {
        (background.color.g, background.color.b) = (1.0.into(), 0.0.into());
    }
    assert!(
        monitor.replace(1, Arc::new(edited.clone())),
        "a composition that is edited is seen"
    );
    monitor.draw(true, false).unwrap();
    near(
        pixel(&settled(&mut monitor), 64, 32, 18),
        [0, 255, 0],
        "edited",
    );
    // The node taken away: black, and the view stays on.
    let mut gone = edited.clone();
    let composition = gone.timelines[0].tracks[0].clips[0]
        .fusion
        .as_mut()
        .unwrap();
    composition.nodes.retain(|node| node.id != "bg");
    composition.links.retain(|link| !link.touches("bg"));
    let without = shown(&mut monitor, &gone, 5);
    near(pixel(&without, 64, 32, 18), [0; 3], "a node that is gone");
    let told = monitor.status.fusion.as_ref().unwrap();
    assert!(!told.shown);
    assert_eq!(told.node.as_deref(), Some("bg"));
    // A clip without a composition shows its source, by either node of the composition that
    // a clip has at first.
    let mut plain = project.clone();
    plain.timelines[0].tracks[0].clips[0].fusion = None;
    shown(&mut monitor, &plain, 5);
    for node in [None, Some("MediaIn1"), Some("MediaOut1")] {
        monitor.fuse(Some(view("c1", node))).unwrap();
        let source = settled(&mut monitor);
        for (x, y) in [(32, 18), (2, 2)] {
            near(pixel(&source, 64, x, y), [200, 110, 40], "its source");
        }
    }
}

#[test]
fn the_fusion_view_is_shown_in_place_of_the_other_modes_and_a_still_is_of_the_timeline() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut monitor = monitor(dir.path());
    let project = composed();
    shown(&mut monitor, &project, 5);
    // A Highlight mode, Bypass Color and a split screen are on.
    monitor.status.highlight = Some(Highlight {
        mode: crate::highlight::Mode::Bw,
        grade: columbina_model::grade::Target::Clip {
            timeline: "t1".into(),
            clip: "c1".into(),
        },
        node: "n1".into(),
    });
    monitor.status.bypass = true;
    let raw = serde_json::value::RawValue::from_string(r#"{"items":[{"kind":"current"}]}"#.into())
        .unwrap();
    let change = Change::read("viewer.split", Some(&raw)).unwrap();
    monitor.compare_with(change).unwrap();
    assert!(monitor.status.compared.is_some());
    // The view is shown as it is, and nothing is compared with it.
    monitor.fuse(Some(view("c1", None))).unwrap();
    let out = settled(&mut monitor);
    for (x, y) in [(32, 18), (2, 2), (61, 33)] {
        near(pixel(&out, 64, x, y), MIXED, "in place of the other modes");
    }
    assert_eq!(monitor.status.compared, None);
    let snapshot = monitor.draw_inner(true, false).unwrap().unwrap();
    assert_eq!(snapshot.comparison["mode"], "off");
    assert!(snapshot.highlight.is_none());
    // A still is of the frame of the timeline as it is graded, and the view is shown again
    // after it.
    monitor.status.highlight = None;
    monitor.status.bypass = false;
    let still = monitor.graded_original().unwrap();
    let grabbed = still.target.read_rgba().unwrap();
    near(pixel(&grabbed, 64, 32, 18), [40, 150, 170], "a still");
    near(pixel(&settled(&mut monitor), 64, 32, 18), MIXED, "after it");
    assert!(monitor.status.fusion.is_some());
    // Off: the split screen that was on is shown again.
    monitor.fuse(None).unwrap();
    let split = monitor.draw_inner(true, false).unwrap().unwrap();
    assert_eq!(split.comparison["mode"], "split");
    assert!(monitor.status.compared.is_some());
}

#[test]
fn a_fusion_view_of_what_the_timeline_has_not_is_refused_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut monitor = monitor(dir.path());
    let reason = |fault: Fault| fault.data.map(|data| data["reason"].clone());
    // No timeline is loaded: there is no clip to view, and nothing to turn off.
    let fault = monitor.fuse(Some(view("c1", None))).unwrap_err();
    assert_eq!(fault.code, rpc::TRANSPORT_FAILED);
    assert_eq!(reason(fault), Some(json!("not_loaded")));
    assert_eq!(
        monitor.fuse(None).unwrap(),
        json!({"monitor": "timeline", "fusion": null})
    );
    let project = composed();
    monitor.replace(0, Arc::new(project.clone()));
    for (asked, why) in [
        (view("c404", None), "clip"),
        (view("c1", Some("n404")), "node"),
        (view("c404", Some("bg")), "clip"),
    ] {
        let fault = monitor.fuse(Some(asked)).unwrap_err();
        assert_eq!(fault.code, rpc::INVALID_PARAMS);
        assert_eq!(reason(fault), Some(json!(why)));
        assert_eq!(monitor.status.fusion, None);
    }
    // A timeline that is unloaded, as when another is loaded, takes the view with it.
    monitor.status.fusion = Some(view("c1", None).state(&project, &project.timelines[0], 5));
    assert!(monitor.status.fusion.as_ref().unwrap().shown);
    monitor.unload();
    assert_eq!(monitor.status.fusion, None);
}
