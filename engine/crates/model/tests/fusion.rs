//! The composition of a clip (the Fusion page): how a project with one is written and read,
//! the operations on it, one by one, with what each gives back and what it refuses, the keys
//! of its settings by their paths, what the edits of a clip do to it, and what a frame of the
//! timeline is given of it.

use std::collections::BTreeMap;

use columbina_model::edit::copy_clips;
use columbina_model::fusion::{
    size_of, BlurFilter, Blurred, Composition, DeltaKeyed, FillMethod, Follow, FollowOutput, Fused,
    FusedKind, FusedNode, Input, KeyerView, Kind, Link, Mosaicked, Node, NodeKind, PaintMode,
    Pattern, Place, Point, ReplaceMode, SpillMethod, Tracked, Tracker, TrackerMerge,
    TrackerOperation, TrackerPivot, MEDIA_IN, MEDIA_OUT, MOST_NODES, MOST_POINTS, MOST_TRACKERS,
};
use columbina_model::fusion::{
    PlanarMotion, PlanarOperation, PlanarTracked, PlanarTracker, MOST_REGION_POINTS, STILL,
};
use columbina_model::grade::{Composite, Interp, Key, Param, Pos, LONGEST_LABEL};
use columbina_model::resolve::{video_at, Layer};
use columbina_model::{
    apply, apply_done, expand, Change, Clip, Edit, History, Invalid, Material, Op, OpError,
    Project, ReadError, Rgba, Source, Time, TrackKind, TransactionError,
};
use columbina_track::matchmove;
use serde_json::json;

fn key(frame: i64, value: f64) -> Key {
    Key {
        frame,
        value,
        interp: Interp::Linear,
    }
}

fn keys(frames: &[(i64, f64)]) -> Param {
    Param::Keys(
        frames
            .iter()
            .map(|&(frame, value)| key(frame, value))
            .collect(),
    )
}

fn clip<'p>(project: &'p Project, id: &str) -> &'p Clip {
    project.timelines[0]
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
        .find(|clip| clip.id == id)
        .expect("the clip is there")
}

/// The composition of clip `id`.
fn composition<'p>(project: &'p Project, id: &str) -> &'p Composition {
    clip(project, id)
        .fusion
        .as_ref()
        .expect("the clip has a composition")
}

fn set(clip: &str, value: Option<Composition>) -> Op {
    Op::SetFusion {
        timeline: "t1".into(),
        clip: clip.into(),
        value,
    }
}

fn add(node: Node, links: Vec<Link>) -> Op {
    Op::AddFusionNode {
        timeline: "t1".into(),
        clip: "c1".into(),
        node,
        index: None,
        links,
    }
}

fn remove(id: &str, links: Option<Vec<Link>>) -> Op {
    Op::RemoveFusionNode {
        timeline: "t1".into(),
        clip: "c1".into(),
        id: id.into(),
        links,
    }
}

fn connect(link: Link) -> Op {
    Op::ConnectFusion {
        timeline: "t1".into(),
        clip: "c1".into(),
        link,
    }
}

fn disconnect(to: &str, input: Input) -> Op {
    Op::DisconnectFusion {
        timeline: "t1".into(),
        clip: "c1".into(),
        to: to.into(),
        input,
    }
}

/// `set_fusion_node` of node `id` of c1, with nothing set.
fn set_node(id: &str) -> Op {
    Op::SetFusionNode {
        timeline: "t1".into(),
        clip: "c1".into(),
        id: id.into(),
        label: None,
        pass_through: None,
        pos: None,
        params: None,
    }
}

/// `set_fusion_node` of the `params` of node `id` of c1.
fn set_params(id: &str, patch: serde_json::Value) -> Op {
    let mut op = set_node(id);
    if let Op::SetFusionNode { params, .. } = &mut op {
        *params = Some(patch);
    }
    op
}

fn set_keys(clip: &str, path: &str, param: Param) -> Op {
    Op::SetFusionKeys {
        timeline: "t1".into(),
        clip: clip.into(),
        path: path.into(),
        param,
    }
}

fn splice(path: &str, at: usize, remove: usize, keys: Vec<Key>) -> Op {
    Op::SpliceFusionKeys {
        timeline: "t1".into(),
        clip: "c1".into(),
        path: path.into(),
        at,
        remove,
        keys,
    }
}

/// Apply an operation, check that its inverse takes it back and that what was done does it
/// again, and give back what was done and the inverse.
fn applied(project: &mut Project, op: Op) -> (Op, Op) {
    let before = project.clone();
    let op_text = format!("{op:?}");
    let (done, inverse) = apply_done(project, op).unwrap_or_else(|e| panic!("{op_text}: {e}"));
    let after = project.clone();
    let mut undone = after.clone();
    let again = apply(&mut undone, inverse.clone()).unwrap();
    let mut expected = before;
    expected.next_id = after.next_id;
    assert_eq!(undone, expected, "{op_text} undone by {inverse:?}");
    assert_eq!(again, done, "{op_text}");
    apply(&mut undone, done.clone()).unwrap();
    assert_eq!(undone, after, "{op_text} done again as {done:?}");
    (done, inverse)
}

/// A refused operation changes nothing, `next_id` neither.
fn refused(project: &Project, op: Op) -> OpError {
    let mut copy = project.clone();
    let error = apply(&mut copy, op).unwrap_err();
    assert_eq!(&copy, project);
    error
}

fn range(what: &'static str) -> OpError {
    OpError::OutOfRange { what }
}

fn missing(what: &'static str, id: &str) -> OpError {
    OpError::Missing {
        what,
        id: id.into(),
    }
}

/// The sample with the composition that a clip has at first on c1, the picture on V1.
fn fused() -> Project {
    let mut project = Project::sample();
    applied(&mut project, set("c1", Some(Composition::default())));
    project
}

/// The sample with c1 made of its picture, n3 a Merge, over a Background, n4: `next_id` is 5.
fn merged() -> Project {
    let mut project = fused();
    applied(
        &mut project,
        add(
            Node::new("", Kind::Merge),
            vec![
                Link::new(MEDIA_IN, "", Input::Foreground),
                Link::new("", MEDIA_OUT, Input::Input),
            ],
        ),
    );
    applied(
        &mut project,
        add(
            Node::new("", Kind::Background),
            vec![Link::new("", "n3", Input::Background)],
        ),
    );
    assert_eq!(project.next_id, 5);
    project
}

/// A composition that moves the picture of its clip: `in` through the Transform `move`, whose
/// Center X has keys at `frames`, into `out`.
fn moving(frames: &[(i64, f64)]) -> Composition {
    let mut transform = Node::new("move", Kind::Transform);
    if let NodeKind::Transform(settings) = &mut transform.kind {
        settings.center.x = keys(frames);
    }
    let mut composition = Composition {
        nodes: vec![
            Node::new("in", Kind::MediaIn),
            transform,
            Node::new("out", Kind::MediaOut),
        ],
        links: vec![
            Link::new("in", "move", Input::Input),
            Link::new("move", "out", Input::Input),
        ],
    };
    composition.sort_links();
    composition
}

/// The frames and the values of the keys of Center X of the Transform `move` of clip `id`.
fn x_keys(project: &Project, id: &str) -> Vec<(i64, f64)> {
    let node = composition(project, id)
        .node("move")
        .expect("the composition has the transform");
    let NodeKind::Transform(settings) = &node.kind else {
        panic!("the node is a transform")
    };
    match &settings.center.x {
        Param::Keys(keys) => keys.iter().map(|k| (k.frame, k.value)).collect(),
        Param::Fixed(value) => vec![(i64::MIN, *value)],
    }
}

/// The one layer that the timeline shows at a frame.
fn layer_at(project: &Project, frame: i64) -> Layer {
    let mut layers = video_at(&project.timelines[0], project, frame);
    assert_eq!(layers.len(), 1, "frame {frame}");
    layers.remove(0)
}

/// The Center X of the first Transform that the layer of each of `frames` draws.
fn seen_at(project: &Project, frames: &[i64]) -> Vec<f64> {
    frames
        .iter()
        .map(|&frame| {
            let fused = layer_at(project, frame)
                .fusion
                .expect("the layer has its composition");
            fused
                .nodes
                .iter()
                .find_map(|node| match &node.kind {
                    FusedKind::Transform { settings, .. } => Some(settings.center[0]),
                    _ => None,
                })
                .expect("the composition draws a transform")
        })
        .collect()
}

/// The sample with a colour `k1` on V1 from frame 200 for 50 frames, which its composition
/// moves right over its first 40 frames.
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
    colour.fusion = Some(moving(&[(0, 0.0), (40, 1.0)]));
    project.timelines[0].tracks[0].clips.push(colour);
    project.next_id = 10;
    project.validate().unwrap();
    project
}

/// Apply an edit, and check that its inverse takes the project back.
fn edited(project: &mut Project, edit: Edit) {
    let before = project.clone();
    let expansion = expand(project, edit).unwrap();
    let mut undone = project.clone();
    for inverse in expansion.inverse.iter().rev() {
        apply(&mut undone, inverse.clone()).unwrap();
    }
    undone.next_id = before.next_id;
    assert_eq!(undone, before);
}

fn edit(value: serde_json::Value) -> Edit {
    serde_json::from_value(value).unwrap()
}

// How a composition is written.

#[test]
fn a_clip_without_a_composition_is_written_as_it_was_and_one_with_it_is_read_back() {
    let plain = Project::sample();
    let text = serde_json::to_string(&plain).unwrap();
    assert!(!text.contains("\"fusion\""));
    let read: Project = serde_json::from_str(&text).unwrap();
    assert_eq!(read, plain);
    assert_eq!(serde_json::to_string(&read).unwrap(), text);
    assert_eq!(clip(&plain, "c1").fusion, None);
    // With one, the clip is written as it was, and its composition after all the rest.
    let project = fused();
    let without = serde_json::to_string(clip(&plain, "c1")).unwrap();
    let with = serde_json::to_string(clip(&project, "c1")).unwrap();
    let first = serde_json::to_string(&Composition::default()).unwrap();
    assert_eq!(
        with,
        format!("{},\"fusion\":{first}}}", &without[..without.len() - 1])
    );
    let written = serde_json::to_value(&project).unwrap();
    assert_eq!(
        written["timelines"][0]["tracks"][0]["clips"][0]["fusion"],
        json!({
            "nodes": [
                {"id": "MediaIn1", "kind": "media_in", "label": "", "pass_through": false,
                 "pos": null, "params": {}},
                {"id": "MediaOut1", "kind": "media_out", "label": "", "pass_through": false,
                 "pos": null, "params": {}},
            ],
            "links": [{"from": "MediaIn1", "to": "MediaOut1", "input": "input"}],
        })
    );
    assert!(written["timelines"][0]["tracks"][1]["clips"][0]
        .get("fusion")
        .is_none());
    assert_eq!(Project::from_value(written).unwrap(), project);
}

#[test]
fn a_project_that_is_read_is_checked_and_its_links_are_put_in_order() {
    let mut whole = merged();
    let composition = composition(&whole, "c1").clone();
    let read = |change: &dyn Fn(&mut serde_json::Value)| {
        let mut value = serde_json::to_value(merged()).unwrap();
        change(&mut value["timelines"][0]["tracks"]);
        Project::from_value(value)
    };
    // Links that are written in another order are read in theirs.
    let mut reversed = composition.clone();
    reversed.links.reverse();
    let unordered = serde_json::to_value(&reversed).unwrap();
    let project = read(&|tracks| tracks[0]["clips"][0]["fusion"] = unordered.clone()).unwrap();
    assert_eq!(project, whole);
    // A clip of sound has none.
    let given = serde_json::to_value(&composition).unwrap();
    let invalid = match read(&|tracks| tracks[1]["clips"][0]["fusion"] = given.clone()) {
        Err(ReadError::Invalid(invalid)) => invalid,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        invalid,
        Invalid {
            what: "clip",
            id: "c2".into(),
            error: OpError::WrongTrack {
                what: "fusion",
                kind: TrackKind::Audio
            },
        }
    );
    // One that is not valid is refused with its clip.
    let invalid = match read(&|tracks| {
        tracks[0]["clips"][0]["fusion"]["nodes"][1]["kind"] = json!("media_in");
    }) {
        Err(ReadError::Invalid(invalid)) => invalid,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        (invalid.what, invalid.id.as_str(), invalid.error),
        ("clip", "c1", OpError::OneMediaOut { count: 0 })
    );
    // And `next_id` is raised above the ids of its nodes that the engine could have made.
    whole.next_id = 1;
    let read = Project::from_value(serde_json::to_value(&whole).unwrap()).unwrap();
    assert_eq!(read.next_id, 5);
}

#[test]
fn every_operation_on_a_composition_is_written_with_the_names_that_other_languages_read() {
    let link = Link::new("MediaIn1", "n3", Input::Foreground);
    let ops = [
        (
            set("c1", None),
            json!({"op": "set_fusion", "timeline": "t1", "clip": "c1", "value": null}),
        ),
        (
            add(Node::new("", Kind::MediaIn), vec![link.clone()]),
            json!({"op": "add_fusion_node", "timeline": "t1", "clip": "c1", "index": null,
                "node": {"id": "", "kind": "media_in", "label": "", "pass_through": false,
                    "pos": null, "params": {}},
                "links": [{"from": "MediaIn1", "to": "n3", "input": "foreground"}]}),
        ),
        (
            remove("n3", None),
            json!({"op": "remove_fusion_node", "timeline": "t1", "clip": "c1", "id": "n3"}),
        ),
        (
            remove("n3", Some(vec![])),
            json!({"op": "remove_fusion_node", "timeline": "t1", "clip": "c1", "id": "n3",
                "links": []}),
        ),
        (
            connect(link),
            json!({"op": "connect_fusion", "timeline": "t1", "clip": "c1",
                "link": {"from": "MediaIn1", "to": "n3", "input": "foreground"}}),
        ),
        (
            disconnect("n3", Input::Background),
            json!({"op": "disconnect_fusion", "timeline": "t1", "clip": "c1", "to": "n3",
                "input": "background"}),
        ),
        (
            Op::SetFusionNode {
                timeline: "t1".into(),
                clip: "c1".into(),
                id: "n3".into(),
                label: Some("Over".into()),
                pass_through: Some(true),
                pos: Some(None),
                params: Some(json!({"blend": 0.5})),
            },
            json!({"op": "set_fusion_node", "timeline": "t1", "clip": "c1", "id": "n3",
                "label": "Over", "pass_through": true, "pos": null,
                "params": {"blend": 0.5}}),
        ),
        (
            set_node("n3"),
            json!({"op": "set_fusion_node", "timeline": "t1", "clip": "c1", "id": "n3"}),
        ),
        (
            set_keys("c1", "n3.blend", Param::Fixed(0.25)),
            json!({"op": "set_fusion_keys", "timeline": "t1", "clip": "c1", "path": "n3.blend",
                "param": 0.25}),
        ),
        (
            splice("n3.blend", 1, 2, vec![key(4, 0.5)]),
            json!({"op": "splice_fusion_keys", "timeline": "t1", "clip": "c1",
                "path": "n3.blend", "at": 1, "remove": 2,
                "keys": [{"frame": 4, "value": 0.5, "interp": "linear"}]}),
        ),
    ];
    for (op, written) in ops {
        assert_eq!(serde_json::to_value(&op).unwrap(), written);
        assert_eq!(serde_json::from_value::<Op>(written).unwrap(), op);
    }
    // The composition that is set must be written, as `null` where it is taken away; and a
    // field that is not known is refused.
    for wrong in [
        json!({"op": "set_fusion", "timeline": "t1", "clip": "c1"}),
        json!({"op": "set_fusion", "timeline": "t1", "clip": "c1", "value": null, "id": "n3"}),
        json!({"op": "disconnect_fusion", "timeline": "t1", "clip": "c1", "to": "n3",
            "input": "background", "kind": "rgb"}),
    ] {
        assert!(
            serde_json::from_value::<Op>(wrong.clone()).is_err(),
            "{wrong}"
        );
    }
}

// The operations.

#[test]
fn a_composition_is_set_whole_on_a_clip_of_picture_and_taken_away() {
    let mut project = Project::sample();
    let (done, inverse) = applied(&mut project, set("c1", Some(Composition::default())));
    assert_eq!(done, set("c1", Some(Composition::default())));
    assert_eq!(inverse, set("c1", None));
    assert_eq!(clip(&project, "c1").fusion, Some(Composition::default()));
    assert_eq!(project.next_id, 3);
    // Another in its place, its links given in another order: it is done with them in theirs.
    let ordered = moving(&[(0, 0.0), (10, 1.0)]);
    let mut given = ordered.clone();
    given.links.reverse();
    assert_ne!(given, ordered);
    let (done, inverse) = applied(&mut project, set("c1", Some(given)));
    assert_eq!(done, set("c1", Some(ordered.clone())));
    assert_eq!(inverse, set("c1", Some(Composition::default())));
    assert_eq!(composition(&project, "c1"), &ordered);
    let (done, inverse) = applied(&mut project, set("c1", None));
    assert_eq!(done, set("c1", None));
    assert_eq!(inverse, set("c1", Some(ordered)));
    assert_eq!(project, Project::sample());
    // A title, a colour and a still take one as media does.
    let mut project = with_a_colour_that_moves();
    applied(&mut project, set("k1", Some(Composition::default())));
    let mut still = Project::sample();
    still.media[0].facts.video.as_mut().unwrap().still = true;
    applied(&mut still, set("c1", Some(Composition::default())));
}

#[test]
#[allow(clippy::too_many_lines, reason = "a refusal for each way to be wrong")]
fn a_composition_that_is_set_is_checked_and_a_clip_of_sound_takes_none() {
    let project = Project::sample();
    assert_eq!(
        refused(&project, set("c2", Some(Composition::default()))),
        OpError::WrongTrack {
            what: "fusion",
            kind: TrackKind::Audio
        }
    );
    assert_eq!(
        refused(&project, set("c9", Some(Composition::default()))),
        missing("clip", "c9")
    );
    assert_eq!(
        refused(
            &fused(),
            Op::SetFusion {
                timeline: "t9".into(),
                clip: "c1".into(),
                value: None,
            }
        ),
        missing("timeline", "t9")
    );
    let with = |change: &dyn Fn(&mut Composition)| {
        let mut composition = Composition::default();
        change(&mut composition);
        refused(&project, set("c1", Some(composition)))
    };
    // The ids of its nodes: each has one that is its own, that the engine could not make
    // later, and that has no dot in it.
    assert_eq!(with(&|c| c.nodes[0].id = String::new()), OpError::NoId);
    assert_eq!(
        with(&|c| c.nodes[0].id = "$in".into()),
        OpError::Dollar { id: "$in".into() }
    );
    assert_eq!(
        with(&|c| c.nodes[0].id = "n3".into()),
        OpError::Reserved { id: "n3".into() }
    );
    assert_eq!(with(&|c| c.nodes[0].id = "in.1".into()), range("fusion_id"));
    assert_eq!(
        with(&|c| c.nodes[0].id = MEDIA_OUT.into()),
        OpError::Taken {
            what: "node",
            id: MEDIA_OUT.into()
        }
    );
    // One MediaOut, and no more nodes than the most.
    assert_eq!(
        with(&|c| c.nodes.truncate(1)),
        OpError::OneMediaOut { count: 0 }
    );
    assert_eq!(
        with(&|c| c.nodes.push(Node::new("out", Kind::MediaOut))),
        OpError::OneMediaOut { count: 2 }
    );
    assert_eq!(
        with(&|c| {
            for n in 0..=MOST_NODES - 2 {
                c.nodes
                    .push(Node::new(format!("back{n}"), Kind::Background));
            }
        }),
        range("fusion_nodes")
    );
    // Its links.
    assert_eq!(
        with(&|c| c.links[0].from = "nowhere".into()),
        missing("node", "nowhere")
    );
    assert_eq!(
        with(&|c| c.links[0].input = Input::Foreground),
        OpError::NoInput {
            node: MEDIA_OUT.into(),
            input: Input::Foreground
        }
    );
    assert_eq!(
        with(&|c| c.links[0] = Link::new(MEDIA_OUT, MEDIA_OUT, Input::Input)),
        OpError::NoOutput {
            node: MEDIA_OUT.into()
        }
    );
    assert_eq!(
        with(&|c| {
            c.nodes.push(Node::new("again", Kind::MediaIn));
            c.links.push(Link::new("again", MEDIA_OUT, Input::Input));
        }),
        OpError::InputLinked {
            node: MEDIA_OUT.into(),
            input: Input::Input
        }
    );
    assert_eq!(
        with(&|c| {
            c.nodes.push(Node::new("a", Kind::Transform));
            c.nodes.push(Node::new("b", Kind::Transform));
            c.links.push(Link::new("a", "b", Input::Input));
            c.links.push(Link::new("b", "a", Input::Input));
        }),
        OpError::Loop { node: "a".into() }
    );
    // The label, the place and the settings of its nodes.
    assert_eq!(
        with(&|c| c.nodes[0].label = "x".repeat(LONGEST_LABEL + 1)),
        range("label")
    );
    assert_eq!(
        with(&|c| c.nodes[0].pos = Some(Pos { x: 2e6, y: 0.0 })),
        range("pos")
    );
    assert_eq!(
        with(&|c| {
            let mut far = Node::new("far", Kind::Transform);
            if let NodeKind::Transform(settings) = &mut far.kind {
                settings.size = Param::Fixed(17.0);
            }
            c.nodes.push(far);
        }),
        range("transform.size")
    );
    // A MediaOut that nothing is linked to is kept: the clip is then transparent.
    let mut unlinked = Composition::default();
    unlinked.links.clear();
    let mut project = Project::sample();
    applied(&mut project, set("c1", Some(unlinked)));
    assert_eq!(layer_at(&project, 10).fusion, Some(Fused::default()));
}

#[test]
fn a_node_is_added_with_its_links_in_place_of_those_into_the_same_inputs() {
    let mut project = fused();
    assert_eq!(project.next_id, 3);
    let (done, inverse) = applied(
        &mut project,
        add(
            Node::new("", Kind::Merge),
            vec![
                Link::new(MEDIA_IN, "", Input::Foreground),
                Link::new("", MEDIA_OUT, Input::Input),
            ],
        ),
    );
    assert_eq!(project.next_id, 4);
    let made = composition(&project, "c1");
    assert_eq!(
        made.nodes,
        [
            Node::new(MEDIA_IN, Kind::MediaIn),
            Node::new(MEDIA_OUT, Kind::MediaOut),
            Node::new("n3", Kind::Merge),
        ]
    );
    let links = vec![
        Link::new("n3", MEDIA_OUT, Input::Input),
        Link::new(MEDIA_IN, "n3", Input::Foreground),
    ];
    assert_eq!(made.links, links);
    assert_eq!(
        done,
        Op::AddFusionNode {
            timeline: "t1".into(),
            clip: "c1".into(),
            node: Node::new("n3", Kind::Merge),
            index: Some(2),
            links,
        }
    );
    // What takes it back puts back the link that it took the place of.
    assert_eq!(
        inverse,
        remove(
            "n3",
            Some(vec![Link::new(MEDIA_IN, MEDIA_OUT, Input::Input)])
        )
    );
    // A node with an id of its own, put first, with no links.
    let mut named = Node::new("sky", Kind::Background);
    named.label = "Sky".into();
    named.pos = Some(Pos { x: 40.0, y: -30.0 });
    let (done, inverse) = applied(
        &mut project,
        Op::AddFusionNode {
            timeline: "t1".into(),
            clip: "c1".into(),
            node: named.clone(),
            index: Some(0),
            links: vec![],
        },
    );
    assert_eq!(project.next_id, 4);
    assert_eq!(composition(&project, "c1").nodes[0], named);
    assert!(matches!(done, Op::AddFusionNode { index: Some(0), .. }));
    assert_eq!(inverse, remove("sky", Some(vec![])));
    // Several MediaIn show the same picture.
    applied(
        &mut project,
        add(
            Node::new("", Kind::MediaIn),
            vec![Link::new("", "n3", Input::Background)],
        ),
    );
    assert_eq!(composition(&project, "c1").nodes.len(), 5);
}

#[test]
#[allow(clippy::too_many_lines, reason = "a refusal for each way to be wrong")]
fn a_node_that_is_added_is_checked_with_its_links() {
    let project = merged();
    let background = || Node::new("", Kind::Background);
    assert_eq!(
        refused(&Project::sample(), add(background(), vec![])),
        missing("fusion", "c1")
    );
    assert_eq!(
        refused(
            &project,
            Op::AddFusionNode {
                timeline: "t1".into(),
                clip: "c1".into(),
                node: background(),
                index: Some(5),
                links: vec![],
            }
        ),
        OpError::OutOfList { at: 5, len: 4 }
    );
    // Its id.
    for (id, error) in [
        (
            MEDIA_IN,
            OpError::Taken {
                what: "node",
                id: MEDIA_IN.into(),
            },
        ),
        ("n5", OpError::Reserved { id: "n5".into() }),
        ("$new", OpError::Dollar { id: "$new".into() }),
        ("a.b", range("fusion_id")),
    ] {
        assert_eq!(
            refused(&project, add(Node::new(id, Kind::Background), vec![])),
            error
        );
    }
    // A composition has one MediaOut.
    assert_eq!(
        refused(&project, add(Node::new("", Kind::MediaOut), vec![])),
        OpError::OneMediaOut { count: 2 }
    );
    // Its links: each touches it, no two go into one input, and each is one that its
    // composition may have.
    let transform = || Node::new("", Kind::Transform);
    for links in [
        vec![Link::new(MEDIA_IN, "n3", Input::Background)],
        vec![
            Link::new(MEDIA_IN, "", Input::Input),
            Link::new("n4", "", Input::Input),
        ],
    ] {
        assert_eq!(refused(&project, add(transform(), links)), range("links"));
    }
    assert_eq!(
        refused(
            &project,
            add(
                transform(),
                vec![Link::new(MEDIA_IN, "", Input::Foreground)]
            )
        ),
        OpError::NoInput {
            node: "n5".into(),
            input: Input::Foreground
        }
    );
    assert_eq!(
        refused(
            &project,
            add(transform(), vec![Link::new(MEDIA_OUT, "", Input::Input)])
        ),
        OpError::NoOutput {
            node: MEDIA_OUT.into()
        }
    );
    assert_eq!(
        refused(
            &project,
            add(transform(), vec![Link::new("nowhere", "", Input::Input)])
        ),
        missing("node", "nowhere")
    );
    assert!(matches!(
        refused(
            &project,
            add(
                transform(),
                vec![
                    Link::new("n3", "", Input::Input),
                    Link::new("", "n3", Input::Background),
                ]
            )
        ),
        OpError::Loop { .. }
    ));
    // Its label, its place and its settings.
    let mut long = transform();
    long.label = "x".repeat(LONGEST_LABEL + 1);
    assert_eq!(refused(&project, add(long, vec![])), range("label"));
    let mut far = transform();
    far.pos = Some(Pos {
        x: 0.0,
        y: f64::NAN,
    });
    assert_eq!(refused(&project, add(far, vec![])), range("pos"));
    let mut turned = transform();
    if let NodeKind::Transform(settings) = &mut turned.kind {
        settings.angle = Param::Fixed(4000.0);
    }
    assert_eq!(
        refused(&project, add(turned, vec![])),
        range("transform.angle")
    );
}

#[test]
fn a_node_is_taken_away_with_its_links_and_comes_back_with_them() {
    let mut project = merged();
    let own = vec![
        Link::new("n3", MEDIA_OUT, Input::Input),
        Link::new("n4", "n3", Input::Background),
        Link::new(MEDIA_IN, "n3", Input::Foreground),
    ];
    assert_eq!(composition(&project, "c1").links, own);
    let before = project.clone();
    let (done, inverse) = applied(&mut project, remove("n3", None));
    // Nothing is linked in its place: the MediaOut shows nothing.
    assert_eq!(done, remove("n3", Some(vec![])));
    assert_eq!(
        inverse,
        Op::AddFusionNode {
            timeline: "t1".into(),
            clip: "c1".into(),
            node: Node::new("n3", Kind::Merge),
            index: Some(2),
            links: own,
        }
    );
    let left = composition(&project, "c1");
    assert_eq!(left.links, [] as [Link; 0]);
    assert_eq!(left.nodes.len(), 3);
    assert_eq!(layer_at(&project, 0).fusion, Some(Fused::default()));
    // With links to put in its place, those are put into the inputs that it went into.
    let mut project = before;
    let bridge = vec![Link::new(MEDIA_IN, MEDIA_OUT, Input::Input)];
    let (done, _) = applied(&mut project, remove("n3", Some(bridge.clone())));
    assert_eq!(done, remove("n3", Some(bridge.clone())));
    assert_eq!(composition(&project, "c1").links, bridge);
}

#[test]
fn a_node_that_is_taken_away_leaves_a_composition_that_is_one() {
    let project = merged();
    assert_eq!(refused(&project, remove("n9", None)), missing("node", "n9"));
    assert_eq!(
        refused(&Project::sample(), remove("n3", None)),
        missing("fusion", "c1")
    );
    // The MediaOut stays.
    assert_eq!(
        refused(&project, remove(MEDIA_OUT, None)),
        OpError::OneMediaOut { count: 0 }
    );
    // A link that is put in its place goes into an input that it went into, and does not
    // touch it; and no two go into one input.
    for links in [
        vec![Link::new(MEDIA_IN, "n3", Input::Background)],
        vec![Link::new("n3", MEDIA_OUT, Input::Input)],
        vec![
            Link::new(MEDIA_IN, MEDIA_OUT, Input::Input),
            Link::new("n4", MEDIA_OUT, Input::Input),
        ],
    ] {
        assert_eq!(refused(&project, remove("n3", Some(links))), range("links"));
    }
    // Nor does the MediaIn have to stay: a composition may be made of a Background alone.
    let mut project = project;
    applied(&mut project, remove(MEDIA_IN, None));
    assert_eq!(composition(&project, "c1").nodes.len(), 3);
}

#[test]
fn an_input_is_linked_in_place_of_the_link_that_it_had_and_unlinked() {
    let mut project = merged();
    // In place of a link: what takes it back links the input as it was.
    let (done, inverse) = applied(
        &mut project,
        connect(Link::new(MEDIA_IN, MEDIA_OUT, Input::Input)),
    );
    assert_eq!(done, connect(Link::new(MEDIA_IN, MEDIA_OUT, Input::Input)));
    assert_eq!(inverse, connect(Link::new("n3", MEDIA_OUT, Input::Input)));
    assert_eq!(
        composition(&project, "c1").links,
        [
            Link::new(MEDIA_IN, MEDIA_OUT, Input::Input),
            Link::new("n4", "n3", Input::Background),
            Link::new(MEDIA_IN, "n3", Input::Foreground),
        ]
    );
    // Unlinked, and linked again where there was no link.
    let (done, inverse) = applied(&mut project, disconnect("n3", Input::Background));
    assert_eq!(done, disconnect("n3", Input::Background));
    assert_eq!(inverse, connect(Link::new("n4", "n3", Input::Background)));
    let (_, inverse) = applied(
        &mut project,
        connect(Link::new(MEDIA_IN, "n3", Input::Background)),
    );
    assert_eq!(inverse, disconnect("n3", Input::Background));
    // The input of the MediaOut may be left with no link.
    applied(&mut project, disconnect(MEDIA_OUT, Input::Input));
    assert_eq!(layer_at(&project, 0).fusion, Some(Fused::default()));
}

#[test]
fn a_link_is_one_that_its_composition_may_have() {
    let project = merged();
    assert_eq!(
        refused(&project, connect(Link::new("n9", "n3", Input::Background))),
        missing("node", "n9")
    );
    assert_eq!(
        refused(&project, connect(Link::new("n4", "n9", Input::Input))),
        missing("node", "n9")
    );
    assert_eq!(
        refused(&project, connect(Link::new("n4", "n3", Input::Input))),
        OpError::NoInput {
            node: "n3".into(),
            input: Input::Input
        }
    );
    assert_eq!(
        refused(&project, connect(Link::new("n3", "n4", Input::Input))),
        OpError::NoInput {
            node: "n4".into(),
            input: Input::Input
        }
    );
    assert_eq!(
        refused(
            &project,
            connect(Link::new(MEDIA_OUT, "n3", Input::Background))
        ),
        OpError::NoOutput {
            node: MEDIA_OUT.into()
        }
    );
    assert_eq!(
        refused(&project, connect(Link::new("n3", "n3", Input::Background))),
        OpError::Loop { node: "n3".into() }
    );
    assert_eq!(
        refused(&project, disconnect("n3", Input::Input)),
        missing("link", "n3")
    );
    assert_eq!(
        refused(&project, disconnect("n9", Input::Input)),
        missing("link", "n9")
    );
    assert_eq!(
        refused(
            &Project::sample(),
            connect(Link::new(MEDIA_IN, MEDIA_OUT, Input::Input))
        ),
        missing("fusion", "c1")
    );
    assert_eq!(
        refused(&Project::sample(), disconnect(MEDIA_OUT, Input::Input)),
        missing("fusion", "c1")
    );
}

#[test]
fn the_settings_of_a_node_that_are_given_are_set_and_set_back_by_the_inverse() {
    let mut project = merged();
    let (done, inverse) = applied(
        &mut project,
        Op::SetFusionNode {
            timeline: "t1".into(),
            clip: "c1".into(),
            id: "n3".into(),
            label: Some("Over the sky".into()),
            pass_through: Some(true),
            pos: Some(Some(Pos { x: 120.0, y: 30.0 })),
            params: Some(json!({"apply": "screen", "blend": 0.5, "center": {"x": 0.25}})),
        },
    );
    let node = composition(&project, "c1").node("n3").unwrap();
    assert_eq!(node.label, "Over the sky");
    assert!(node.pass_through);
    assert_eq!(node.pos, Some(Pos { x: 120.0, y: 30.0 }));
    let NodeKind::Merge(merge) = &node.kind else {
        panic!("n3 is a merge")
    };
    assert_eq!(merge.apply, Composite::Screen);
    assert_eq!(merge.blend, Param::Fixed(0.5));
    assert_eq!(merge.center.x, Param::Fixed(0.25));
    // What the patch does not name is as it was.
    assert_eq!(merge.center.y, Param::Fixed(0.5));
    assert_eq!(merge.alpha_gain, Param::Fixed(1.0));
    // It is done with the values as they are written, and taken back with those that were.
    let params_of = |op: &Op| match op {
        Op::SetFusionNode { params, .. } => params.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        params_of(&done),
        Some(json!({"apply": "screen", "blend": 0.5, "center": {"x": 0.25}}))
    );
    assert_eq!(
        inverse,
        Op::SetFusionNode {
            timeline: "t1".into(),
            clip: "c1".into(),
            id: "n3".into(),
            label: Some(String::new()),
            pass_through: Some(false),
            pos: Some(None),
            params: Some(json!({"apply": "normal", "blend": 1.0, "center": {"x": 0.5}})),
        }
    );
    // A setting may be given keys so, whole.
    let rising = json!({"keys": [
        {"frame": 0, "value": 0.0, "interp": "linear"},
        {"frame": 20, "value": 1.0, "interp": "hold"},
    ]});
    let (_, inverse) = applied(&mut project, set_params("n3", json!({"blend": rising})));
    assert_eq!(params_of(&inverse), Some(json!({"blend": 0.5})));
    // One setting alone, and a node put back where the editor lays it out.
    let mut back = set_node("n3");
    if let Op::SetFusionNode { pos, .. } = &mut back {
        *pos = Some(None);
    }
    let (_, inverse) = applied(&mut project, back);
    let mut there = set_node("n3");
    if let Op::SetFusionNode { pos, .. } = &mut there {
        *pos = Some(Some(Pos { x: 120.0, y: 30.0 }));
    }
    assert_eq!(inverse, there);
    // The flags of a Transform and the colour of a Background are settings like the others.
    applied(
        &mut project,
        set_params("n4", json!({"color": {"r": 0.25, "a": 0.5}})),
    );
    let NodeKind::Background(background) = &composition(&project, "c1").node("n4").unwrap().kind
    else {
        panic!("n4 is a background")
    };
    assert_eq!(background.at(0.0), [0.25, 0.0, 0.0, 0.5]);
}

#[test]
fn the_settings_of_a_node_are_those_of_its_kind_within_their_ranges() {
    let project = merged();
    assert_eq!(refused(&project, set_node("n9")), missing("node", "n9"));
    assert_eq!(
        refused(&Project::sample(), set_node("n3")),
        missing("fusion", "c1")
    );
    for patch in [
        json!({"inputs": 3}),
        json!({"apply": "dissolve"}),
        json!({"operator": "under"}),
        json!({"center": 0.5}),
        json!({"blend": "half"}),
        json!(7),
    ] {
        let error = refused(&project, set_params("n3", patch.clone()));
        assert!(
            matches!(&error, OpError::Params { node, .. } if node == "n3"),
            "{patch}: {error:?}"
        );
    }
    // A MediaIn has no settings.
    assert!(matches!(
        refused(&project, set_params(MEDIA_IN, json!({"size": 2}))),
        OpError::Params { .. }
    ));
    for (patch, what) in [
        (json!({"blend": 1.5}), "merge.blend"),
        (json!({"burn_in": -0.1}), "merge.burn_in"),
        (json!({"alpha_gain": 2}), "merge.alpha_gain"),
        (
            json!({"subtractive_additive": 1.01}),
            "merge.subtractive_additive",
        ),
        (json!({"center": {"y": 17}}), "merge.center.y"),
        (json!({"size": -1}), "merge.size"),
        (json!({"angle": 3601}), "merge.angle"),
    ] {
        assert_eq!(
            refused(&project, set_params("n3", patch)),
            range(what),
            "{what}"
        );
    }
    assert_eq!(
        refused(&project, set_params("n4", json!({"color": {"g": 1.5}}))),
        range("background.color.g")
    );
    let mut long = set_node("n3");
    if let Op::SetFusionNode { label, .. } = &mut long {
        *label = Some("x".repeat(LONGEST_LABEL + 1));
    }
    assert_eq!(refused(&project, long), range("label"));
    let mut far = set_node("n3");
    if let Op::SetFusionNode { pos, .. } = &mut far {
        *pos = Some(Some(Pos { x: -2e6, y: 0.0 }));
    }
    assert_eq!(refused(&project, far), range("pos"));
}

#[test]
fn a_refusal_of_a_composition_is_written_with_its_reason_its_message_and_its_fields() {
    let written = |error: OpError| serde_json::to_value(error).unwrap();
    assert_eq!(
        written(OpError::NoInput {
            node: "n3".into(),
            input: Input::Foreground
        }),
        json!({"reason": "no_input", "node": "n3", "input": "foreground",
            "message": "node n3 has no input foreground"})
    );
    assert_eq!(
        written(OpError::NoOutput {
            node: MEDIA_OUT.into()
        }),
        json!({"reason": "no_output", "node": "MediaOut1",
            "message": "node MediaOut1 has no output"})
    );
    assert_eq!(
        written(OpError::InputLinked {
            node: "n3".into(),
            input: Input::Background
        }),
        json!({"reason": "input_linked", "node": "n3", "input": "background",
            "message": "two links go into input background of node n3"})
    );
    assert_eq!(
        written(OpError::OneMediaOut { count: 2 }),
        json!({"reason": "one_media_out", "count": 2,
            "message": "a composition has one media_out, and this would have 2"})
    );
    assert_eq!(
        written(missing("fusion", "c1")),
        json!({"reason": "missing", "what": "fusion", "id": "c1",
            "message": "there is no fusion c1"})
    );
    assert_eq!(
        written(OpError::WrongTrack {
            what: "fusion",
            kind: TrackKind::Audio
        })["message"],
        "a fusion cannot be on a audio track"
    );
    assert_eq!(
        written(range("fusion_nodes"))["message"],
        format!("a composition has at most {MOST_NODES} nodes")
    );
    assert_eq!(
        written(range("fusion_id"))["message"],
        "the id of a node of a composition has a dot in it, which ends the id in the path of a \
         setting"
    );
    assert_eq!(
        written(range("merge.blend")),
        json!({"reason": "out_of_range", "what": "merge.blend",
            "message": "the merge.blend is out of its range"})
    );
}

// Keys.

#[test]
fn the_keys_of_a_setting_are_set_by_its_path_and_spliced_and_each_is_taken_back() {
    let mut project = merged();
    let (done, inverse) = applied(
        &mut project,
        set_keys("c1", "n3.blend", keys(&[(0, 0.0), (20, 1.0)])),
    );
    assert_eq!(
        done,
        set_keys("c1", "n3.blend", keys(&[(0, 0.0), (20, 1.0)]))
    );
    assert_eq!(inverse, set_keys("c1", "n3.blend", Param::Fixed(1.0)));
    // A key put between them, and the last taken away.
    let (done, inverse) = applied(&mut project, splice("n3.blend", 1, 1, vec![key(10, 0.75)]));
    assert_eq!(done, splice("n3.blend", 1, 1, vec![key(10, 0.75)]));
    assert_eq!(inverse, splice("n3.blend", 1, 1, vec![key(20, 1.0)]));
    let blend = |project: &Project| match &composition(project, "c1").node("n3").unwrap().kind {
        NodeKind::Merge(merge) => merge.blend.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(blend(&project), keys(&[(0, 0.0), (10, 0.75)]));
    // The first keys of a number are done as the keys set whole, and taken back as the number.
    let (done, inverse) = applied(
        &mut project,
        splice("n3.center.x", 0, 0, vec![key(5, 0.25)]),
    );
    assert_eq!(done, set_keys("c1", "n3.center.x", keys(&[(5, 0.25)])));
    assert_eq!(inverse, set_keys("c1", "n3.center.x", Param::Fixed(0.5)));
    // A number in place of keys.
    let (_, inverse) = applied(&mut project, set_keys("c1", "n3.blend", Param::Fixed(0.5)));
    assert_eq!(
        inverse,
        set_keys("c1", "n3.blend", keys(&[(0, 0.0), (10, 0.75)]))
    );
    // The settings of the other kinds, by their paths.
    applied(
        &mut project,
        set_keys("c1", "n4.color.a", keys(&[(0, 0.0), (25, 1.0)])),
    );
    let mut moved = fused();
    applied(
        &mut moved,
        add(
            Node::new("", Kind::Transform),
            vec![
                Link::new(MEDIA_IN, "", Input::Input),
                Link::new("", MEDIA_OUT, Input::Input),
            ],
        ),
    );
    for path in ["n3.center.x", "n3.center.y", "n3.size", "n3.angle"] {
        applied(
            &mut moved,
            set_keys("c1", path, keys(&[(0, 0.5), (9, 1.0)])),
        );
    }
}

#[test]
fn a_path_names_a_setting_of_a_node_that_takes_keys_and_keys_are_checked() {
    let project = merged();
    let fixed = Param::Fixed(0.5);
    for (path, error) in [
        ("n9.blend", missing("node", "n9")),
        ("n3.apply", missing("setting", "n3.apply")),
        ("n3.center", missing("setting", "n3.center")),
        ("n3.color.r", missing("setting", "n3.color.r")),
        ("n3.blend.more", missing("setting", "n3.blend.more")),
        ("n3.", missing("setting", "n3.")),
        ("n3", missing("setting", "n3")),
        ("blend", missing("setting", "blend")),
        ("", missing("setting", "")),
        ("n4.size", missing("setting", "n4.size")),
        ("MediaIn1.size", missing("setting", "MediaIn1.size")),
        ("MediaOut1.input", missing("setting", "MediaOut1.input")),
    ] {
        assert_eq!(
            refused(&project, set_keys("c1", path, fixed.clone())),
            error,
            "{path}"
        );
        assert_eq!(
            refused(&project, splice(path, 0, 0, vec![key(0, 0.5)])),
            error,
            "{path}"
        );
    }
    assert_eq!(
        refused(
            &Project::sample(),
            set_keys("c1", "n3.blend", fixed.clone())
        ),
        missing("fusion", "c1")
    );
    assert_eq!(
        refused(&project, set_keys("c2", "n3.blend", fixed)),
        missing("fusion", "c2")
    );
    // What is put is checked as the node is: values within the range, at frames in order.
    assert_eq!(
        refused(&project, set_keys("c1", "n3.blend", Param::Fixed(1.5))),
        range("merge.blend")
    );
    assert_eq!(
        refused(
            &project,
            set_keys("c1", "n3.size", keys(&[(0, 1.0), (10, 17.0)]))
        ),
        range("merge.size")
    );
    assert_eq!(
        refused(
            &project,
            set_keys("c1", "n4.color.b", keys(&[(10, 0.0), (10, 1.0)]))
        ),
        range("background.color.b")
    );
    assert_eq!(
        refused(&project, set_keys("c1", "n3.angle", Param::Keys(vec![]))),
        range("merge.angle")
    );
    // A splice is within the keys that there are, and leaves one at least.
    let mut keyed = project.clone();
    applied(
        &mut keyed,
        set_keys("c1", "n3.blend", keys(&[(0, 0.0), (20, 1.0)])),
    );
    assert_eq!(
        refused(&keyed, splice("n3.blend", 1, 2, vec![])),
        OpError::OutOfList { at: 3, len: 2 }
    );
    assert_eq!(
        refused(&project, splice("n3.blend", 0, 1, vec![key(0, 0.5)])),
        OpError::OutOfList { at: 1, len: 0 }
    );
    let error = refused(&keyed, splice("n3.blend", 0, 2, vec![]));
    assert!(
        matches!(&error, OpError::Params { node, why } if node == "n3" && why.contains("n3.blend")),
        "{error:?}"
    );
    assert_eq!(
        refused(&keyed, splice("n3.blend", 2, 0, vec![key(15, 0.5)])),
        range("merge.blend")
    );
}

#[test]
fn a_change_of_a_composition_that_changes_nothing_makes_no_step() {
    let mut history = History::new(merged());
    let revision = history.revision();
    for op in [
        set("c2", None),
        set("c1", Some(composition(history.project(), "c1").clone())),
        set_params("n3", json!({"blend": 1.0, "apply": "normal"})),
        set_keys("c1", "n3.blend", Param::Fixed(1.0)),
        connect(Link::new("n4", "n3", Input::Background)),
    ] {
        let nothing = history.change(vec![Change::Op(op.clone())], "desktop", |change, _| change);
        assert_eq!(nothing.err(), Some(TransactionError::Empty), "{op:?}");
    }
    assert_eq!(history.revision(), revision);
    assert!(!history.can_undo());
    // One that changes something is a step, and is undone.
    let before = history.project().clone();
    history
        .change(
            vec![Change::Op(set_keys("c1", "n3.blend", Param::Fixed(0.5)))],
            "desktop",
            |change, _| change,
        )
        .unwrap();
    assert_ne!(history.project(), &before);
    history.undo("desktop").unwrap().unwrap();
    assert_eq!(history.project(), &before);
    // A splice that takes no key away and puts none is no step either.
    let keyed = set_keys("c1", "n3.blend", keys(&[(0, 0.0), (20, 1.0)]));
    history
        .change(vec![Change::Op(keyed)], "desktop", |change, _| change)
        .unwrap();
    let nothing = splice("n3.blend", 1, 0, vec![]);
    let nothing = history.change(vec![Change::Op(nothing)], "desktop", |change, _| change);
    assert_eq!(nothing.err(), Some(TransactionError::Empty));
}

// The picture.

#[test]
fn a_layer_carries_the_composition_of_its_clip_at_the_frame_and_one_without_is_as_it_was() {
    let plain = Project::sample();
    let was = layer_at(&plain, 20);
    assert_eq!(was.fusion, None);
    // The composition that a clip has at first: the picture of its source, as it is.
    let first = layer_at(&fused(), 20);
    assert_eq!(
        first.fusion,
        Some(Fused {
            nodes: vec![FusedNode {
                id: MEDIA_IN.into(),
                kind: FusedKind::MediaIn,
            }],
            output: Some(0),
        })
    );
    // Nothing else of the layer is changed by it.
    assert_eq!(
        Layer {
            fusion: None,
            ..first
        },
        was
    );
    // A clip of sound, and a clip that is off, give no layer, as before.
    let mut off = fused();
    off.timelines[0].tracks[0].clips[0].enabled = false;
    assert_eq!(video_at(&off.timelines[0], &off, 20), [] as [Layer; 0]);
}

#[test]
fn the_nodes_of_a_layer_are_those_that_its_media_out_reads_each_after_its_inputs() {
    let mut project = merged();
    // A node that nothing reads is left out.
    applied(&mut project, add(Node::new("", Kind::Transform), vec![]));
    applied(
        &mut project,
        set_keys("c1", "n3.blend", keys(&[(10, 0.0), (30, 1.0)])),
    );
    let fused = layer_at(&project, 20).fusion.unwrap();
    let ids: Vec<&str> = fused.nodes.iter().map(|node| node.id.as_str()).collect();
    assert_eq!(ids, ["n4", MEDIA_IN, "n3"]);
    assert_eq!(fused.output, Some(2));
    assert_eq!(
        fused.nodes[0].kind,
        FusedKind::Background {
            rgba: [0.0, 0.0, 0.0, 1.0],
            mask: None,
        }
    );
    let FusedKind::Merge {
        background,
        foreground,
        mask,
        settings,
    } = &fused.nodes[2].kind
    else {
        panic!("n3 is a merge")
    };
    assert_eq!((*background, *foreground, *mask), (Some(0), Some(1), None));
    // c1 shows m1 from its start at the rate of the timeline: frame 20 shows picture 20.
    assert_eq!((settings.blend, settings.apply), (0.5, Composite::Normal));
    assert_eq!((settings.center, settings.size), ([0.5, 0.5], 1.0));
    // A node that is passed through hands on its first input: the Merge, its background.
    let mut through = set_node("n3");
    if let Op::SetFusionNode { pass_through, .. } = &mut through {
        *pass_through = Some(true);
    }
    applied(&mut project, through);
    let fused = layer_at(&project, 20).fusion.unwrap();
    assert_eq!(fused.nodes.len(), 1);
    assert_eq!((fused.nodes[0].id.as_str(), fused.output), ("n4", Some(0)));
}

#[test]
fn the_keys_of_a_composition_are_read_at_the_frame_of_the_keys_of_its_clip() {
    // Of media, the frame of its picture: at half the speed, picture 20 is shown at frame 40.
    let mut project = Project::sample();
    applied(
        &mut project,
        set("c1", Some(moving(&[(10, 0.0), (30, 1.0)]))),
    );
    assert_eq!(seen_at(&project, &[10, 20, 30]), [0.0, 0.5, 1.0]);
    let slow: Op = serde_json::from_value(json!({
        "op": "set_clip", "timeline": "t1", "id": "c1", "speed": {"num": 1, "den": 2},
    }))
    .unwrap();
    apply(&mut project, slow).unwrap();
    assert_eq!(seen_at(&project, &[20, 40, 41, 60]), [0.0, 0.5, 0.5, 1.0]);
    // Of a colour, its own frames from its first.
    let colour = with_a_colour_that_moves();
    assert_eq!(
        seen_at(&colour, &[200, 210, 240, 249]),
        [0.0, 0.25, 1.0, 1.0]
    );
}

// The edits of a clip.

#[test]
fn a_trim_of_the_head_leaves_the_keys_of_a_composition_where_they_were() {
    // Of a colour, which counts its own frames, at their frames of the timeline.
    let frames = [210, 215, 220, 230, 245];
    let mut project = with_a_colour_that_moves();
    let before = seen_at(&project, &frames);
    edited(
        &mut project,
        edit(json!({"edit": "trim", "timeline": "t1", "clip": "k1", "edge": "in", "delta": 10})),
    );
    assert_eq!(x_keys(&project, "k1"), [(-10, 0.0), (30, 1.0)]);
    assert_eq!(seen_at(&project, &frames), before);
    // Its tail leaves them alone.
    edited(
        &mut project,
        edit(json!({"edit": "trim", "timeline": "t1", "clip": "k1", "edge": "out", "delta": -5})),
    );
    assert_eq!(x_keys(&project, "k1"), [(-10, 0.0), (30, 1.0)]);
    // Of media, on their pictures: they are not moved, and each frame shows what it showed.
    let mut moving_media = Project::sample();
    applied(
        &mut moving_media,
        set("c1", Some(moving(&[(20, 0.0), (60, 1.0)]))),
    );
    let before = seen_at(&moving_media, &[30, 40, 70]);
    edited(
        &mut moving_media,
        edit(
            json!({"edit": "trim", "timeline": "t1", "clip": "c1", "edge": "in", "delta": 10,
            "linked": false}),
        ),
    );
    assert_eq!(x_keys(&moving_media, "c1"), [(20, 0.0), (60, 1.0)]);
    assert_eq!(seen_at(&moving_media, &[30, 40, 70]), before);
}

#[test]
fn both_halves_of_a_clip_cut_by_the_blade_keep_its_composition_and_play_as_it_did() {
    let frames: Vec<i64> = (200..250).step_by(5).collect();
    let mut project = with_a_colour_that_moves();
    let before = seen_at(&project, &frames);
    edited(
        &mut project,
        edit(json!({"edit": "blade", "timeline": "t1", "at": 215, "tracks": ["V1"]})),
    );
    let clips = &project.timelines[0].tracks[0].clips;
    assert_eq!(clips.len(), 3);
    let second = clips.iter().find(|c| c.start == 215).unwrap().id.clone();
    assert_eq!(x_keys(&project, "k1"), [(0, 0.0), (40, 1.0)]);
    assert_eq!(x_keys(&project, &second), [(-15, 0.0), (25, 1.0)]);
    // The second half has the nodes and the links of the first.
    let (first, second) = (composition(&project, "k1"), composition(&project, &second));
    assert_eq!(first.links, second.links);
    let ids = |composition: &Composition| -> Vec<String> {
        composition.nodes.iter().map(|n| n.id.clone()).collect()
    };
    assert_eq!(ids(first), ids(second));
    assert_eq!(seen_at(&project, &frames), before);
    // A clip of media: both halves keep the keys on their pictures.
    let mut media = Project::sample();
    applied(&mut media, set("c1", Some(moving(&[(20, 0.0), (60, 1.0)]))));
    let before = seen_at(&media, &[10, 30, 50, 70]);
    edited(
        &mut media,
        edit(json!({"edit": "blade", "timeline": "t1", "at": 40, "tracks": ["V1"]})),
    );
    let second = media.timelines[0].tracks[0].clips[1].id.clone();
    assert_eq!(x_keys(&media, "c1"), [(20, 0.0), (60, 1.0)]);
    assert_eq!(x_keys(&media, &second), [(20, 0.0), (60, 1.0)]);
    assert_eq!(seen_at(&media, &[10, 30, 50, 70]), before);
}

#[test]
fn a_clip_that_is_copied_and_pasted_takes_its_composition_and_its_keys_along() {
    let mut project = with_a_colour_that_moves();
    // The whole of it.
    let whole = copy_clips(
        &project.timelines[0],
        &["k1".into()],
        200,
        None,
        false,
        false,
    )
    .unwrap();
    assert_eq!(whole.clips[0].clip.fusion, clip(&project, "k1").fusion);
    edited(
        &mut project,
        Edit::PasteClips {
            timeline: "t1".into(),
            clipboard: whole,
            at: 300,
            tracks: BTreeMap::new(),
        },
    );
    let pasted = project.timelines[0].tracks[0]
        .clips
        .iter()
        .find(|c| c.start == 300)
        .unwrap()
        .id
        .clone();
    assert_eq!(x_keys(&project, &pasted), [(0, 0.0), (40, 1.0)]);
    assert_eq!(
        seen_at(&project, &[300, 320, 340]),
        seen_at(&project, &[200, 220, 240])
    );
    // A part of it: its keys stay where they were on its frames.
    let part = copy_clips(
        &project.timelines[0],
        &["k1".into()],
        210,
        Some((210, 230)),
        false,
        false,
    )
    .unwrap();
    assert_eq!(
        (part.clips[0].clip.start, part.clips[0].clip.length),
        (210, 20)
    );
    edited(
        &mut project,
        Edit::PasteClips {
            timeline: "t1".into(),
            clipboard: part,
            at: 400,
            tracks: BTreeMap::new(),
        },
    );
    let pasted = project.timelines[0].tracks[0]
        .clips
        .iter()
        .find(|c| c.start == 400)
        .unwrap()
        .id
        .clone();
    assert_eq!(x_keys(&project, &pasted), [(-10, 0.0), (30, 1.0)]);
    assert_eq!(
        seen_at(&project, &[400, 410, 419]),
        seen_at(&project, &[210, 220, 229])
    );
}

#[test]
fn a_replacing_source_takes_the_composition_with_its_keys_on_its_frames() {
    let mut project = with_a_colour_that_moves();
    // m1 from two seconds in, its picture 50, in place of the colour: the keys that counted
    // the frames of the colour from its first count the pictures of m1 from picture 50.
    edited(
        &mut project,
        Edit::Replace {
            timeline: "t1".into(),
            source: Material::Media {
                media: "m1".into(),
                from: Some(Time {
                    ticks: 2 * 15360,
                    scale: 15360,
                }),
                to: None,
                length: None,
            },
            at: 200,
            video_track: None,
            audio_track: None,
        },
    );
    let replaced = project.timelines[0].tracks[0]
        .clips
        .iter()
        .find(|c| c.start == 200)
        .unwrap();
    assert!(matches!(replaced.source, Source::Media { .. }));
    let id = replaced.id.clone();
    assert_eq!(x_keys(&project, &id), [(50, 0.0), (90, 1.0)]);
    assert_eq!(seen_at(&project, &[200, 220, 240]), [0.0, 0.5, 1.0]);
}

// Masks.

/// The sample of `merged` with its Merge, n3, limited by an Ellipse, n5, into which the mask
/// of a Rectangle, n6, comes: `next_id` is 7.
fn masked() -> Project {
    let mut project = merged();
    applied(
        &mut project,
        add(
            Node::new("", Kind::Ellipse),
            vec![Link::new("", "n3", Input::Mask)],
        ),
    );
    applied(
        &mut project,
        add(
            Node::new("", Kind::Rectangle),
            vec![Link::new("", "n5", Input::Mask)],
        ),
    );
    assert_eq!(project.next_id, 7);
    project
}

/// A Polygon of points that have no keys, for the engine to give an id.
fn polygon(points: &[[f64; 2]]) -> Node {
    let mut node = Node::new("", Kind::Polygon);
    if let NodeKind::Polygon(polygon) = &mut node.kind {
        polygon.points = points.iter().map(|&[x, y]| Point::new(x, y)).collect();
    }
    node
}

/// `masked` with a triangle, the Polygon n7, that nothing reads: `next_id` is 8.
fn with_a_polygon() -> Project {
    let mut project = masked();
    let triangle = polygon(&[[-0.1, -0.1], [0.1, -0.1], [0.0, 0.1]]);
    applied(&mut project, add(triangle, vec![]));
    assert_eq!(project.next_id, 8);
    project
}

/// The Polygon n7 of c1.
fn polygon_of(project: &Project) -> &columbina_model::fusion::Polygon {
    match &composition(project, "c1").node("n7").unwrap().kind {
        NodeKind::Polygon(polygon) => polygon,
        other => panic!("{other:?}"),
    }
}

/// The ids of the nodes that the layer of c1 draws at frame 20, in their order, with the
/// place of the mask of each.
fn drawn(project: &Project) -> Vec<(String, Option<usize>)> {
    let fused = layer_at(project, 20).fusion.unwrap();
    fused
        .nodes
        .iter()
        .map(|node| (node.id.clone(), node.kind.mask()))
        .collect()
}

#[test]
fn a_composition_without_masks_is_written_as_it_was_to_the_byte() {
    // What the engine wrote of a Merge over a Background before there were masks.
    let old = concat!(
        r#"{"nodes":[{"id":"MediaIn1","kind":"media_in","label":"","pass_through":false,"#,
        r#""pos":null,"params":{}},{"id":"MediaOut1","kind":"media_out","label":"","#,
        r#""pass_through":false,"pos":null,"params":{}},{"id":"n3","kind":"merge","#,
        r#""label":"","pass_through":false,"pos":null,"params":{"apply":"normal","#,
        r#""operator":"over","subtractive_additive":1.0,"alpha_gain":1.0,"burn_in":0.0,"#,
        r#""blend":1.0,"center":{"x":0.5,"y":0.5},"size":1.0,"angle":0.0}},{"id":"n4","#,
        r#""kind":"background","label":"","pass_through":false,"pos":null,"params":"#,
        r#"{"color":{"r":0.0,"g":0.0,"b":0.0,"a":1.0}}}],"links":[{"from":"n3","#,
        r#""to":"MediaOut1","input":"input"},{"from":"n4","to":"n3","input":"background"},"#,
        r#"{"from":"MediaIn1","to":"n3","input":"foreground"}]}"#,
    );
    let project = merged();
    let written = serde_json::to_string(composition(&project, "c1")).unwrap();
    assert_eq!(written, old);
    let read: Composition = serde_json::from_str(old).unwrap();
    assert_eq!(&read, composition(&project, "c1"));
    assert_eq!(serde_json::to_string(&read).unwrap(), old);
    // And a project with masks is read back as it was written, and written the same again.
    let project = with_a_polygon();
    let text = serde_json::to_string(&project).unwrap();
    let read: Project = serde_json::from_str(&text).unwrap();
    assert_eq!(read, project);
    assert_eq!(serde_json::to_string(&read).unwrap(), text);
    let value = serde_json::to_value(&project).unwrap();
    assert_eq!(Project::from_value(value).unwrap(), project);
}

#[test]
fn a_mask_is_added_with_the_settings_of_its_kind_all_of_them_written() {
    let mut project = masked();
    let written = serde_json::to_value(composition(&project, "c1")).unwrap();
    assert_eq!(
        written["nodes"][4],
        json!({
            "id": "n5", "kind": "ellipse", "label": "", "pass_through": false, "pos": null,
            "params": {
                "level": 1.0, "soft_edge": 0.0, "border_width": 0.0, "invert": false,
                "solid": true, "paint_mode": "merge", "center": {"x": 0.5, "y": 0.5},
                "width": 0.5, "height": 0.5, "angle": 0.0,
            },
        })
    );
    assert_eq!(
        written["nodes"][5]["params"],
        json!({
            "level": 1.0, "soft_edge": 0.0, "border_width": 0.0, "invert": false,
            "solid": true, "paint_mode": "merge", "center": {"x": 0.5, "y": 0.5},
            "width": 0.5, "height": 0.5, "corner_radius": 0.0, "angle": 0.0,
        })
    );
    // A mask is the last of the inputs of its node, as the links are kept.
    assert_eq!(
        written["links"],
        json!([
            {"from": "n3", "to": "MediaOut1", "input": "input"},
            {"from": "n4", "to": "n3", "input": "background"},
            {"from": "MediaIn1", "to": "n3", "input": "foreground"},
            {"from": "n5", "to": "n3", "input": "mask"},
            {"from": "n6", "to": "n5", "input": "mask"},
        ])
    );
    // A polygon as another program writes it, with every setting and its points.
    let op: Op = serde_json::from_value(json!({
        "op": "add_fusion_node", "timeline": "t1", "clip": "c1",
        "node": {"id": "", "kind": "polygon", "label": "Garbage", "params": {
            "level": 0.75, "soft_edge": 0.01, "border_width": -0.005, "invert": true,
            "solid": true, "paint_mode": "subtract", "center": {"x": 0.25, "y": 0.75},
            "size": 2, "angle": 30,
            "points": [{"x": -0.1, "y": -0.1}, {"x": 0.1, "y": -0.1}, {"x": 0, "y": 0.1}],
            "fill": "non_zero_winding",
        }},
        "links": [{"from": "", "to": "n6", "input": "mask"}],
    }))
    .unwrap();
    let (done, inverse) = applied(&mut project, op);
    assert_eq!(inverse, remove("n7", Some(vec![])));
    let Op::AddFusionNode { node, links, .. } = done else {
        panic!("a node was added")
    };
    assert_eq!(node.id, "n7");
    assert_eq!(links, [Link::new("n7", "n6", Input::Mask)]);
    let polygon = polygon_of(&project);
    assert_eq!(polygon.paint_mode, PaintMode::Subtract);
    assert_eq!(polygon.fill, FillMethod::NonZeroWinding);
    assert!(polygon.invert && polygon.solid);
    assert_eq!(polygon.size, Param::Fixed(2.0));
    assert_eq!(
        polygon.points,
        [
            Point::new(-0.1, -0.1),
            Point::new(0.1, -0.1),
            Point::new(0.0, 0.1)
        ]
    );
    // A new one has no points, and a mask whose settings are not all there cannot be read.
    let NodeKind::Polygon(first) = Node::new("", Kind::Polygon).kind else {
        panic!("it is a polygon")
    };
    assert_eq!(first.points, [] as [Point; 0]);
    for params in [
        json!({}),
        json!({"level": 1.0, "soft_edge": 0.0, "border_width": 0.0, "invert": false,
            "solid": true, "paint_mode": "merge", "center": {"x": 0.5, "y": 0.5},
            "width": 0.5, "height": 0.5}),
        json!({"level": 1.0, "soft_edge": 0.0, "border_width": 0.0, "invert": false,
            "solid": true, "paint_mode": "merge", "center": {"x": 0.5, "y": 0.5},
            "width": 0.5, "height": 0.5, "angle": 0.0, "corner_radius": 0.0}),
    ] {
        let node = json!({"id": "", "kind": "ellipse", "label": "", "params": params});
        assert!(
            serde_json::from_value::<Node>(node.clone()).is_err(),
            "{node}"
        );
    }
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "a refusal for each setting of each mask"
)]
fn the_settings_of_a_mask_are_those_of_its_kind_within_their_ranges() {
    let mut project = with_a_polygon();
    for (id, patch, what) in [
        ("n5", json!({"level": 1.5}), "ellipse.level"),
        ("n5", json!({"soft_edge": -0.1}), "ellipse.soft_edge"),
        ("n5", json!({"border_width": 1.5}), "ellipse.border_width"),
        ("n5", json!({"center": {"x": 10.5}}), "ellipse.center.x"),
        ("n5", json!({"center": {"y": -10.5}}), "ellipse.center.y"),
        ("n5", json!({"width": 10.5}), "ellipse.width"),
        ("n5", json!({"height": -1}), "ellipse.height"),
        ("n5", json!({"angle": 36001}), "ellipse.angle"),
        ("n6", json!({"level": -0.5}), "rectangle.level"),
        ("n6", json!({"soft_edge": 1.5}), "rectangle.soft_edge"),
        (
            "n6",
            json!({"border_width": -1.5}),
            "rectangle.border_width",
        ),
        ("n6", json!({"center": {"x": -11}}), "rectangle.center.x"),
        ("n6", json!({"center": {"y": 11}}), "rectangle.center.y"),
        ("n6", json!({"width": -0.5}), "rectangle.width"),
        ("n6", json!({"height": 11}), "rectangle.height"),
        (
            "n6",
            json!({"corner_radius": 1.5}),
            "rectangle.corner_radius",
        ),
        ("n6", json!({"angle": -36001}), "rectangle.angle"),
        ("n7", json!({"level": 2}), "polygon.level"),
        ("n7", json!({"soft_edge": 2}), "polygon.soft_edge"),
        ("n7", json!({"border_width": 2}), "polygon.border_width"),
        ("n7", json!({"center": {"x": 10.5}}), "polygon.center.x"),
        ("n7", json!({"center": {"y": 10.5}}), "polygon.center.y"),
        ("n7", json!({"size": 10.5}), "polygon.size"),
        ("n7", json!({"angle": 36000.5}), "polygon.angle"),
        (
            "n7",
            json!({"points": [{"x": 16.5, "y": 0}]}),
            "polygon.points.x",
        ),
        (
            "n7",
            json!({"points": [{"x": 0, "y": 0}, {"x": 0, "y": -16.5}]}),
            "polygon.points.y",
        ),
    ] {
        assert_eq!(
            refused(&project, set_params(id, patch)),
            range(what),
            "{what}"
        );
    }
    // A polygon has at most so many points.
    let most: Vec<serde_json::Value> = (0..MOST_POINTS)
        .map(|_| json!({"x": 0.0, "y": 0.0}))
        .collect();
    let mut more = most.clone();
    more.push(json!({"x": 0.0, "y": 0.0}));
    assert_eq!(
        refused(&project, set_params("n7", json!({"points": more}))),
        range("polygon.points")
    );
    // What is not a setting of the kind, or not written as one, is refused.
    for (id, patch) in [
        ("n5", json!({"paint_mode": "maximum"})),
        ("n5", json!({"corner_radius": 0.5})),
        ("n5", json!({"fill": "alternate"})),
        ("n5", json!({"center": 0.5})),
        ("n6", json!({"invert": 1})),
        ("n6", json!({"size": 1.0})),
        ("n7", json!({"points": [{"x": 0.0}]})),
        ("n7", json!({"points": {"x": 0.0, "y": 0.0}})),
        ("n7", json!({"fill": "even_odd"})),
        ("n7", json!({"width": 0.5})),
    ] {
        let error = refused(&project, set_params(id, patch.clone()));
        assert!(
            matches!(&error, OpError::Params { node, .. } if node == id),
            "{patch}: {error:?}"
        );
    }
    // What is given is set, and set back by the inverse.
    let params_of = |op: &Op| match op {
        Op::SetFusionNode { params, .. } => params.clone(),
        other => panic!("{other:?}"),
    };
    let (_, inverse) = applied(
        &mut project,
        set_params(
            "n5",
            json!({"invert": true, "solid": false, "paint_mode": "add", "border_width": 0.02,
                "center": {"y": 0.25}}),
        ),
    );
    assert_eq!(
        params_of(&inverse),
        Some(
            json!({"invert": false, "solid": true, "paint_mode": "merge",
            "border_width": 0.0, "center": {"y": 0.5}})
        )
    );
    let NodeKind::Ellipse(ellipse) = &composition(&project, "c1").node("n5").unwrap().kind else {
        panic!("n5 is an ellipse")
    };
    assert!(ellipse.invert && !ellipse.solid);
    assert_eq!(ellipse.paint_mode, PaintMode::Add);
    assert_eq!(ellipse.border_width, Param::Fixed(0.02));
    assert_eq!(ellipse.center.y, Param::Fixed(0.25));
    assert_eq!(ellipse.center.x, Param::Fixed(0.5));
    // The points of a polygon are set whole: as many as it may have, and none.
    let (_, inverse) = applied(
        &mut project,
        set_params(
            "n7",
            json!({"fill": "non_zero_winding", "points": [{"x": 0.25, "y": 0.5}]}),
        ),
    );
    assert_eq!(
        params_of(&inverse),
        Some(json!({"fill": "alternate", "points": [
            {"x": -0.1, "y": -0.1}, {"x": 0.1, "y": -0.1}, {"x": 0.0, "y": 0.1},
        ]}))
    );
    assert_eq!(polygon_of(&project).points, [Point::new(0.25, 0.5)]);
    applied(&mut project, set_params("n7", json!({"points": most})));
    assert_eq!(polygon_of(&project).points.len(), MOST_POINTS);
    applied(&mut project, set_params("n7", json!({"points": []})));
    assert_eq!(polygon_of(&project).points, [] as [Point; 0]);
    // The ends of each range are within it.
    applied(
        &mut project,
        set_params(
            "n6",
            json!({"level": 0, "soft_edge": 1, "border_width": -1, "center": {"x": -10, "y": 10},
                "width": 0, "height": 10, "corner_radius": 1, "angle": -36000}),
        ),
    );
    applied(
        &mut project,
        set_params(
            "n7",
            json!({"size": 10, "angle": 36000, "points": [{"x": -16, "y": 16}]}),
        ),
    );
}

#[test]
#[allow(clippy::too_many_lines, reason = "a key for each setting of a mask")]
fn the_keys_of_a_mask_are_set_by_the_paths_of_its_settings_and_of_its_points() {
    let mut project = with_a_polygon();
    for path in [
        "n5.level",
        "n5.soft_edge",
        "n5.border_width",
        "n5.center.x",
        "n5.center.y",
        "n5.width",
        "n5.height",
        "n5.angle",
        "n6.height",
        "n6.corner_radius",
        "n7.size",
        "n7.angle",
        "n7.points.0.x",
        "n7.points.2.y",
    ] {
        let rising = keys(&[(0, 0.0), (9, 0.25)]);
        let (done, inverse) = applied(&mut project, set_keys("c1", path, rising.clone()));
        assert_eq!(done, set_keys("c1", path, rising));
        assert!(
            matches!(
                &inverse,
                Op::SetFusionKeys {
                    param: Param::Fixed(_),
                    ..
                }
            ),
            "{path}: {inverse:?}"
        );
    }
    assert!(composition(&project, "c1").is_animated());
    // A key of a point is put in place of another, and the first keys of a number of a
    // point are done as the keys set whole.
    let (done, inverse) = applied(
        &mut project,
        splice("n7.points.2.y", 1, 1, vec![key(5, 0.5)]),
    );
    assert_eq!(done, splice("n7.points.2.y", 1, 1, vec![key(5, 0.5)]));
    assert_eq!(inverse, splice("n7.points.2.y", 1, 1, vec![key(9, 0.25)]));
    let (done, inverse) = applied(
        &mut project,
        splice("n7.points.1.x", 0, 0, vec![key(3, 0.2)]),
    );
    assert_eq!(done, set_keys("c1", "n7.points.1.x", keys(&[(3, 0.2)])));
    assert_eq!(inverse, set_keys("c1", "n7.points.1.x", Param::Fixed(0.1)));
    let polygon = polygon_of(&project);
    assert_eq!(polygon.points[2].y, keys(&[(0, 0.0), (5, 0.5)]));
    assert_eq!(polygon.points[2].x, Param::Fixed(0.0));
    assert_eq!(polygon.points[1].y, Param::Fixed(-0.1));
    // The keys of a mask are moved with the frames that they count, as the others are.
    let later = composition(&project, "c1").with_key_frames(&|frame| frame + 100);
    let NodeKind::Polygon(moved) = &later.node("n7").unwrap().kind else {
        panic!("n7 is a polygon")
    };
    assert_eq!(moved.points[2].y, keys(&[(100, 0.0), (105, 0.5)]));
    assert_eq!(moved.points[1].x, keys(&[(103, 0.2)]));
    // What is no number takes no keys, a kind has its own settings only, and a point is
    // one that the polygon has, named by its place as the engine writes a number.
    for path in [
        "n5.invert",
        "n5.solid",
        "n5.paint_mode",
        "n5.corner_radius",
        "n5.center",
        "n5.size",
        "n6.points.0.x",
        "n7.fill",
        "n7.points",
        "n7.points.0",
        "n7.points.3.x",
        "n7.points.x",
        "n7.points.00.x",
        "n7.points.-1.x",
        "n7.width",
    ] {
        assert_eq!(
            refused(&project, set_keys("c1", path, Param::Fixed(0.5))),
            missing("setting", path),
            "{path}"
        );
        assert_eq!(
            refused(&project, splice(path, 0, 0, vec![key(0, 0.5)])),
            missing("setting", path),
            "{path}"
        );
    }
    // What is put is checked as the node is.
    assert_eq!(
        refused(
            &project,
            set_keys("c1", "n7.points.0.y", Param::Fixed(16.5))
        ),
        range("polygon.points.y")
    );
    assert_eq!(
        refused(
            &project,
            set_keys("c1", "n5.soft_edge", keys(&[(0, 0.0), (5, 1.5)]))
        ),
        range("ellipse.soft_edge")
    );
    assert_eq!(
        refused(
            &project,
            splice("n6.corner_radius", 1, 0, vec![key(20, 1.5)])
        ),
        range("rectangle.corner_radius")
    );
    // The points set whole take the place of those that had keys: a point that is no
    // longer there has no setting.
    applied(
        &mut project,
        set_params("n7", json!({"points": [{"x": 0.0, "y": 0.0}]})),
    );
    assert_eq!(
        refused(&project, set_keys("c1", "n7.points.2.y", Param::Fixed(0.0))),
        missing("setting", "n7.points.2.y")
    );
}

#[test]
fn a_mask_is_linked_into_the_kinds_that_have_one_and_refused_elsewhere() {
    let mut project = masked();
    // A Transform, n7, between the Merge and the MediaOut.
    applied(
        &mut project,
        add(
            Node::new("", Kind::Transform),
            vec![
                Link::new("n3", "", Input::Input),
                Link::new("", MEDIA_OUT, Input::Input),
            ],
        ),
    );
    // The mask of the rectangle limits a Background and a Transform too: one output may
    // feed several inputs.
    for to in ["n4", "n7"] {
        let (done, inverse) = applied(&mut project, connect(Link::new("n6", to, Input::Mask)));
        assert_eq!(done, connect(Link::new("n6", to, Input::Mask)));
        assert_eq!(inverse, disconnect(to, Input::Mask));
    }
    // Any picture may be a mask, in place of the one that the input had.
    let (_, inverse) = applied(
        &mut project,
        connect(Link::new(MEDIA_IN, "n3", Input::Mask)),
    );
    assert_eq!(inverse, connect(Link::new("n5", "n3", Input::Mask)));
    applied(&mut project, inverse);
    // And a mask may be a picture.
    let (_, inverse) = applied(
        &mut project,
        connect(Link::new("n5", "n3", Input::Foreground)),
    );
    assert_eq!(
        inverse,
        connect(Link::new(MEDIA_IN, "n3", Input::Foreground))
    );
    applied(&mut project, inverse);
    // A MediaIn and a MediaOut have no mask, and a mask node no input but its mask.
    for node in [MEDIA_IN, MEDIA_OUT] {
        assert_eq!(
            refused(&project, connect(Link::new("n6", node, Input::Mask))),
            OpError::NoInput {
                node: node.into(),
                input: Input::Mask
            }
        );
    }
    for input in [Input::Input, Input::Background, Input::Foreground] {
        assert_eq!(
            refused(&project, connect(Link::new(MEDIA_IN, "n5", input))),
            OpError::NoInput {
                node: "n5".into(),
                input
            }
        );
    }
    // A loop through a mask is a loop.
    assert_eq!(
        refused(&project, connect(Link::new("n5", "n5", Input::Mask))),
        OpError::Loop { node: "n5".into() }
    );
    for from in ["n3", "n7"] {
        assert!(
            matches!(
                refused(&project, connect(Link::new(from, "n6", Input::Mask))),
                OpError::Loop { .. }
            ),
            "{from}"
        );
    }
    // A mask is unlinked as another input is.
    let (_, inverse) = applied(&mut project, disconnect("n3", Input::Mask));
    assert_eq!(inverse, connect(Link::new("n5", "n3", Input::Mask)));
    assert_eq!(
        refused(&project, disconnect("n3", Input::Mask)),
        missing("link", "n3")
    );
    assert_eq!(
        refused(&project, disconnect(MEDIA_IN, Input::Mask)),
        missing("link", MEDIA_IN)
    );
}

#[test]
fn the_masks_of_a_layer_are_among_its_nodes_each_before_the_node_that_reads_it() {
    let at = |id: &str, mask: Option<usize>| (id.to_owned(), mask);
    let mut project = masked();
    applied(
        &mut project,
        set_keys("c1", "n5.soft_edge", keys(&[(10, 0.0), (30, 0.5)])),
    );
    // The masks are read through `mask` alone, and are there: the pictures of a node
    // before its mask, and a mask that is not linked none.
    assert_eq!(
        drawn(&project),
        [
            at("n4", None),
            at(MEDIA_IN, None),
            at("n6", None),
            at("n5", Some(2)),
            at("n3", Some(3))
        ]
    );
    let fused = layer_at(&project, 20).fusion.unwrap();
    assert_eq!(fused.output, Some(4));
    assert!(matches!(
        fused.nodes[2].kind,
        FusedKind::Rectangle { mask: None, .. }
    ));
    let FusedKind::Ellipse { common, shape, .. } = &fused.nodes[3].kind else {
        panic!("n5 is an ellipse")
    };
    // c1 shows m1 from its start at the rate of the timeline: frame 20 shows picture 20.
    assert_eq!((common.level, common.soft_edge), (1.0, 0.25));
    assert_eq!((common.solid, common.invert), (true, false));
    assert_eq!(common.paint_mode, PaintMode::Merge);
    assert_eq!(
        (shape.center, shape.width, shape.height),
        ([0.5, 0.5], 0.5, 0.5)
    );
    // A mask node that is passed through hands on the mask that comes into it.
    let through = |id: &str| {
        let mut op = set_node(id);
        if let Op::SetFusionNode { pass_through, .. } = &mut op {
            *pass_through = Some(true);
        }
        op
    };
    applied(&mut project, through("n5"));
    assert_eq!(
        drawn(&project),
        [
            at("n4", None),
            at(MEDIA_IN, None),
            at("n6", None),
            at("n3", Some(2))
        ]
    );
    // One into which none comes hands on none, and the Merge is not limited.
    applied(&mut project, through("n6"));
    assert_eq!(
        drawn(&project),
        [at("n4", None), at(MEDIA_IN, None), at("n3", None)]
    );
    // A node that is passed through does not hand on its effect mask: a Background that
    // has one gives no picture.
    let mut limited = masked();
    applied(&mut limited, connect(Link::new("n6", "n4", Input::Mask)));
    assert_eq!(
        drawn(&limited),
        [
            at("n6", None),
            at("n4", Some(0)),
            at(MEDIA_IN, None),
            at("n5", Some(0)),
            at("n3", Some(3))
        ]
    );
    applied(&mut limited, through("n4"));
    assert_eq!(
        drawn(&limited),
        [
            at(MEDIA_IN, None),
            at("n6", None),
            at("n5", Some(1)),
            at("n3", Some(2))
        ]
    );
    // A mask that is taken away leaves its node without one; with the links to put in its
    // place, the mask that came into it comes into the node.
    let mut bare = masked();
    applied(&mut bare, remove("n5", None));
    assert_eq!(
        drawn(&bare),
        [at("n4", None), at(MEDIA_IN, None), at("n3", None)]
    );
    let mut joined = masked();
    let bridge = vec![Link::new("n6", "n3", Input::Mask)];
    let (_, inverse) = applied(&mut joined, remove("n5", Some(bridge)));
    assert!(matches!(&inverse, Op::AddFusionNode { links, .. } if links.len() == 2));
    assert_eq!(
        drawn(&joined),
        [
            at("n4", None),
            at(MEDIA_IN, None),
            at("n6", None),
            at("n3", Some(2))
        ]
    );
}

#[test]
fn a_change_of_a_mask_that_changes_nothing_makes_no_step() {
    let mut history = History::new(masked());
    let revision = history.revision();
    for op in [
        set_params(
            "n5",
            json!({"level": 1.0, "invert": false, "paint_mode": "merge"}),
        ),
        set_keys("c1", "n6.corner_radius", Param::Fixed(0.0)),
        connect(Link::new("n5", "n3", Input::Mask)),
    ] {
        let nothing = history.change(vec![Change::Op(op.clone())], "desktop", |change, _| change);
        assert_eq!(nothing.err(), Some(TransactionError::Empty), "{op:?}");
    }
    assert_eq!(history.revision(), revision);
    assert!(!history.can_undo());
    // One that changes something is a step, and is undone.
    let before = history.project().clone();
    history
        .change(
            vec![Change::Op(set_params("n5", json!({"invert": true})))],
            "desktop",
            |change, _| change,
        )
        .unwrap();
    assert_ne!(history.project(), &before);
    history.undo("desktop").unwrap().unwrap();
    assert_eq!(history.project(), &before);
}

// Blur and Mosaic.

/// The sample of `merged` with the picture of its Merge, n3, blurred by the Blur n5, and
/// that laid out in cells by the Mosaic n6, which the `MediaOut` shows: `next_id` is 7.
fn filtered() -> Project {
    let mut project = merged();
    for (kind, from) in [(Kind::Blur, "n3"), (Kind::Mosaic, "n5")] {
        applied(
            &mut project,
            add(
                Node::new("", kind),
                vec![
                    Link::new(from, "", Input::Input),
                    Link::new("", MEDIA_OUT, Input::Input),
                ],
            ),
        );
    }
    assert_eq!(project.next_id, 7);
    project
}

/// The `params` that a `set_fusion_node` sets.
fn patch_of(op: &Op) -> Option<serde_json::Value> {
    match op {
        Op::SetFusionNode { params, .. } => params.clone(),
        other => panic!("{other:?}"),
    }
}

/// `set_fusion_node` that passes node `id` of c1 through.
fn passed_through(id: &str) -> Op {
    let mut op = set_node(id);
    if let Op::SetFusionNode { pass_through, .. } = &mut op {
        *pass_through = Some(true);
    }
    op
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "each of the two kinds as it is written, read and taken away"
)]
fn a_blur_and_a_mosaic_are_added_with_every_setting_written_and_are_read_back() {
    let mut project = filtered();
    let written = serde_json::to_value(composition(&project, "c1")).unwrap();
    assert_eq!(
        written["nodes"][4],
        json!({
            "id": "n5", "kind": "blur", "label": "", "pass_through": false, "pos": null,
            "params": {
                "filter": "gaussian", "lock_xy": true, "size": {"x": 5.0, "y": 5.0},
                "blend": 1.0,
            },
        })
    );
    assert_eq!(
        written["nodes"][5],
        json!({
            "id": "n6", "kind": "mosaic", "label": "", "pass_through": false, "pos": null,
            "params": {
                "frequency": 32.0, "aspect": 1.0, "offset": {"x": 0.0, "y": 0.0},
                "whole_cells_only": false,
            },
        })
    );
    // Each has its picture for its first input, as the links are kept.
    assert_eq!(
        written["links"],
        json!([
            {"from": "n6", "to": "MediaOut1", "input": "input"},
            {"from": "n4", "to": "n3", "input": "background"},
            {"from": "MediaIn1", "to": "n3", "input": "foreground"},
            {"from": "n3", "to": "n5", "input": "input"},
            {"from": "n5", "to": "n6", "input": "input"},
        ])
    );
    // To the byte, in the order of their settings.
    let nodes = &composition(&project, "c1").nodes;
    assert_eq!(
        serde_json::to_string(&nodes[4]).unwrap(),
        concat!(
            r#"{"id":"n5","kind":"blur","label":"","pass_through":false,"pos":null,"#,
            r#""params":{"filter":"gaussian","lock_xy":true,"size":{"x":5.0,"y":5.0},"#,
            r#""blend":1.0}}"#,
        )
    );
    assert_eq!(
        serde_json::to_string(&nodes[5]).unwrap(),
        concat!(
            r#"{"id":"n6","kind":"mosaic","label":"","pass_through":false,"pos":null,"#,
            r#""params":{"frequency":32.0,"aspect":1.0,"offset":{"x":0.0,"y":0.0},"#,
            r#""whole_cells_only":false}}"#,
        )
    );
    // A project with them is read back as it was written, and written the same again.
    let text = serde_json::to_string(&project).unwrap();
    let read: Project = serde_json::from_str(&text).unwrap();
    assert_eq!(read, project);
    assert_eq!(serde_json::to_string(&read).unwrap(), text);
    let value = serde_json::to_value(&project).unwrap();
    assert_eq!(Project::from_value(value).unwrap(), project);
    // A Blur as another program writes it, with every setting, limited by the Mosaic.
    let op: Op = serde_json::from_value(json!({
        "op": "add_fusion_node", "timeline": "t1", "clip": "c1",
        "node": {"id": "", "kind": "blur", "label": "Soften", "params": {
            "filter": "box", "lock_xy": false, "size": {"x": 12, "y": 0}, "blend": 0.5,
        }},
        "links": [{"from": "n6", "to": "", "input": "mask"}],
    }))
    .unwrap();
    let (done, inverse) = applied(&mut project, op);
    assert_eq!(inverse, remove("n7", Some(vec![])));
    let Op::AddFusionNode { node, links, .. } = done else {
        panic!("a node was added")
    };
    assert_eq!(node.id, "n7");
    assert_eq!(links, [Link::new("n6", "n7", Input::Mask)]);
    let NodeKind::Blur(blur) = &composition(&project, "c1").node("n7").unwrap().kind else {
        panic!("n7 is a blur")
    };
    assert_eq!(blur.filter, BlurFilter::Box);
    assert!(!blur.lock_xy);
    assert_eq!(blur.size.x, Param::Fixed(12.0));
    assert_eq!(blur.size.y, Param::Fixed(0.0));
    assert_eq!(blur.blend, Param::Fixed(0.5));
    // One whose settings are not all there, or that has a setting of another kind, cannot
    // be read: a Mosaic has no shape to set.
    for (kind, params) in [
        ("blur", json!({})),
        (
            "blur",
            json!({"filter": "gaussian", "lock_xy": true, "size": {"x": 5.0, "y": 5.0}}),
        ),
        (
            "blur",
            json!({"filter": "gaussian", "lock_xy": true, "size": {"x": 5.0}, "blend": 1.0}),
        ),
        (
            "mosaic",
            json!({"frequency": 32.0, "aspect": 1.0, "offset": {"x": 0.0, "y": 0.0}}),
        ),
        (
            "mosaic",
            json!({"frequency": 32.0, "aspect": 1.0, "offset": {"x": 0.0, "y": 0.0},
                "whole_cells_only": false, "shape": "square"}),
        ),
    ] {
        let node = json!({"id": "", "kind": kind, "label": "", "params": params});
        assert!(
            serde_json::from_value::<Node>(node.clone()).is_err(),
            "{node}"
        );
    }
    // Taken away again, each with the link that takes its place, they leave the composition
    // as it was before them, and written as it was.
    applied(&mut project, remove("n7", None));
    let bridge = |from: &str| Some(vec![Link::new(from, MEDIA_OUT, Input::Input)]);
    applied(&mut project, remove("n6", bridge("n5")));
    applied(&mut project, remove("n5", bridge("n3")));
    let before = merged();
    assert_eq!(composition(&project, "c1"), composition(&before, "c1"));
    assert_eq!(
        serde_json::to_string(composition(&project, "c1")).unwrap(),
        serde_json::to_string(composition(&before, "c1")).unwrap()
    );
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "a refusal for each setting of a blur and of a mosaic"
)]
fn the_settings_of_a_blur_and_of_a_mosaic_are_those_of_their_kinds_within_their_ranges() {
    let mut project = filtered();
    for (id, patch, what) in [
        ("n5", json!({"size": {"x": 500.5}}), "blur.size.x"),
        ("n5", json!({"size": {"x": -0.5}}), "blur.size.x"),
        ("n5", json!({"size": {"y": 501}}), "blur.size.y"),
        ("n5", json!({"size": {"y": -1}}), "blur.size.y"),
        ("n5", json!({"blend": 1.5}), "blur.blend"),
        ("n5", json!({"blend": -0.5}), "blur.blend"),
        ("n6", json!({"frequency": 0.5}), "mosaic.frequency"),
        ("n6", json!({"frequency": 16385}), "mosaic.frequency"),
        ("n6", json!({"aspect": 0.05}), "mosaic.aspect"),
        ("n6", json!({"aspect": 16.5}), "mosaic.aspect"),
        ("n6", json!({"offset": {"x": 16385}}), "mosaic.offset.x"),
        ("n6", json!({"offset": {"x": -16385}}), "mosaic.offset.x"),
        ("n6", json!({"offset": {"y": 16384.5}}), "mosaic.offset.y"),
        ("n6", json!({"offset": {"y": -16384.5}}), "mosaic.offset.y"),
    ] {
        assert_eq!(
            refused(&project, set_params(id, patch.clone())),
            range(what),
            "{patch}"
        );
    }
    // `size.y` is held to its range though Lock X/Y leaves it unread.
    assert_eq!(
        refused(
            &project,
            set_params("n5", json!({"lock_xy": true, "size": {"y": 600}}))
        ),
        range("blur.size.y")
    );
    // What is not a setting of the kind, or not written as one, is refused.
    for (id, patch) in [
        ("n5", json!({"filter": "bartlett"})),
        ("n5", json!({"lock_xy": 1})),
        ("n5", json!({"size": 5.0})),
        ("n5", json!({"size": {"z": 5.0}})),
        ("n5", json!({"frequency": 8.0})),
        ("n5", json!({"clipping": "frame"})),
        ("n6", json!({"whole_cells_only": "yes"})),
        ("n6", json!({"shape": "square"})),
        ("n6", json!({"offset": 2.0})),
        ("n6", json!({"blend": 0.5})),
        ("n6", json!({"size": {"x": 5.0}})),
    ] {
        let error = refused(&project, set_params(id, patch.clone()));
        assert!(
            matches!(&error, OpError::Params { node, .. } if node == id),
            "{patch}: {error:?}"
        );
    }
    // What is given is set, and set back by the inverse.
    let (_, inverse) = applied(
        &mut project,
        set_params(
            "n5",
            json!({"filter": "box", "lock_xy": false, "size": {"y": 12.5}, "blend": 0.25}),
        ),
    );
    assert_eq!(
        patch_of(&inverse),
        Some(json!({"filter": "gaussian", "lock_xy": true, "size": {"y": 5.0}, "blend": 1.0}))
    );
    let NodeKind::Blur(blur) = &composition(&project, "c1").node("n5").unwrap().kind else {
        panic!("n5 is a blur")
    };
    assert_eq!(blur.filter, BlurFilter::Box);
    assert!(!blur.lock_xy);
    assert_eq!(blur.size.x, Param::Fixed(5.0));
    assert_eq!(blur.size.y, Param::Fixed(12.5));
    assert_eq!(blur.blend, Param::Fixed(0.25));
    let (_, inverse) = applied(
        &mut project,
        set_params(
            "n6",
            json!({"frequency": 12, "aspect": 2, "offset": {"x": -8.5}, "whole_cells_only": true}),
        ),
    );
    assert_eq!(
        patch_of(&inverse),
        Some(
            json!({"frequency": 32.0, "aspect": 1.0, "offset": {"x": 0.0},
            "whole_cells_only": false})
        )
    );
    let NodeKind::Mosaic(mosaic) = &composition(&project, "c1").node("n6").unwrap().kind else {
        panic!("n6 is a mosaic")
    };
    assert_eq!(mosaic.frequency, Param::Fixed(12.0));
    assert_eq!(mosaic.aspect, Param::Fixed(2.0));
    assert_eq!(mosaic.offset.x, Param::Fixed(-8.5));
    assert_eq!(mosaic.offset.y, Param::Fixed(0.0));
    assert!(mosaic.whole_cells_only);
    // The ends of each range are within it.
    applied(
        &mut project,
        set_params("n5", json!({"size": {"x": 0, "y": 500}, "blend": 0})),
    );
    applied(
        &mut project,
        set_params(
            "n6",
            json!({"frequency": 1, "aspect": 0.0625, "offset": {"x": -16384, "y": 16384}}),
        ),
    );
    applied(
        &mut project,
        set_params(
            "n6",
            json!({"frequency": 16384, "aspect": 16, "offset": {"x": 16384, "y": -16384}}),
        ),
    );
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "a key for each setting of a blur and of a mosaic"
)]
fn the_keys_of_a_blur_and_of_a_mosaic_are_set_by_the_paths_of_their_settings() {
    let mut project = filtered();
    // The first keys of a setting that had none are done as the keys set whole.
    let mut first = project.clone();
    let (done, inverse) = applied(&mut first, splice("n5.blend", 0, 0, vec![key(3, 0.5)]));
    assert_eq!(done, set_keys("c1", "n5.blend", keys(&[(3, 0.5)])));
    assert_eq!(inverse, set_keys("c1", "n5.blend", Param::Fixed(1.0)));
    for (path, value) in [
        ("n5.size.x", 20.0),
        ("n5.size.y", 40.0),
        ("n5.blend", 0.5),
        ("n6.frequency", 8.0),
        ("n6.aspect", 2.0),
        ("n6.offset.x", 12.0),
        ("n6.offset.y", -7.5),
    ] {
        let rising = keys(&[(0, 1.0), (9, value)]);
        let (done, inverse) = applied(&mut project, set_keys("c1", path, rising.clone()));
        assert_eq!(done, set_keys("c1", path, rising));
        assert!(
            matches!(
                &inverse,
                Op::SetFusionKeys {
                    param: Param::Fixed(_),
                    ..
                }
            ),
            "{path}: {inverse:?}"
        );
    }
    assert!(composition(&project, "c1").is_animated());
    // A key is put in place of another, and taken back.
    let (done, inverse) = applied(&mut project, splice("n6.offset.y", 1, 1, vec![key(5, 3.0)]));
    assert_eq!(done, splice("n6.offset.y", 1, 1, vec![key(5, 3.0)]));
    assert_eq!(inverse, splice("n6.offset.y", 1, 1, vec![key(9, -7.5)]));
    let NodeKind::Mosaic(mosaic) = &composition(&project, "c1").node("n6").unwrap().kind else {
        panic!("n6 is a mosaic")
    };
    assert_eq!(mosaic.offset.y, keys(&[(0, 1.0), (5, 3.0)]));
    assert_eq!(mosaic.offset.x, keys(&[(0, 1.0), (9, 12.0)]));
    assert_eq!(mosaic.frequency, keys(&[(0, 1.0), (9, 8.0)]));
    let NodeKind::Blur(blur) = &composition(&project, "c1").node("n5").unwrap().kind else {
        panic!("n5 is a blur")
    };
    assert_eq!(blur.size.x, keys(&[(0, 1.0), (9, 20.0)]));
    assert_eq!(blur.size.y, keys(&[(0, 1.0), (9, 40.0)]));
    // The keys are moved with the frames that they count, as the others are.
    let later = composition(&project, "c1").with_key_frames(&|frame| frame + 100);
    let NodeKind::Mosaic(moved) = &later.node("n6").unwrap().kind else {
        panic!("n6 is a mosaic")
    };
    assert_eq!(moved.offset.y, keys(&[(100, 1.0), (105, 3.0)]));
    let NodeKind::Blur(moved) = &later.node("n5").unwrap().kind else {
        panic!("n5 is a blur")
    };
    assert_eq!(moved.blend, keys(&[(100, 1.0), (109, 0.5)]));
    // What is no number takes no keys, and a kind has its own settings only.
    for path in [
        "n5.filter",
        "n5.lock_xy",
        "n5.size",
        "n5.size.z",
        "n5.frequency",
        "n5.center.x",
        "n6.whole_cells_only",
        "n6.offset",
        "n6.shape",
        "n6.blend",
        "n6.size.x",
    ] {
        assert_eq!(
            refused(&project, set_keys("c1", path, Param::Fixed(0.5))),
            missing("setting", path),
            "{path}"
        );
        assert_eq!(
            refused(&project, splice(path, 0, 0, vec![key(0, 0.5)])),
            missing("setting", path),
            "{path}"
        );
    }
    // What is put is checked as the node is.
    assert_eq!(
        refused(&project, set_keys("c1", "n5.size.x", Param::Fixed(500.5))),
        range("blur.size.x")
    );
    assert_eq!(
        refused(
            &project,
            set_keys("c1", "n6.frequency", keys(&[(0, 8.0), (5, 0.5)]))
        ),
        range("mosaic.frequency")
    );
    assert_eq!(
        refused(&project, splice("n5.blend", 2, 0, vec![key(20, 1.5)])),
        range("blur.blend")
    );
    assert_eq!(
        refused(
            &project,
            splice("n6.offset.x", 2, 0, vec![key(20, 16_385.0)])
        ),
        range("mosaic.offset.x")
    );
}

#[test]
fn a_blur_and_a_mosaic_take_a_picture_and_a_mask_and_no_other_input() {
    let mut project = filtered();
    // An Ellipse, n7, is the effect mask of the Blur, and of the Mosaic too: one output may
    // feed several inputs.
    let (_, inverse) = applied(
        &mut project,
        add(
            Node::new("", Kind::Ellipse),
            vec![Link::new("", "n5", Input::Mask)],
        ),
    );
    assert_eq!(inverse, remove("n7", Some(vec![])));
    let (done, inverse) = applied(&mut project, connect(Link::new("n7", "n6", Input::Mask)));
    assert_eq!(done, connect(Link::new("n7", "n6", Input::Mask)));
    assert_eq!(inverse, disconnect("n6", Input::Mask));
    // Any picture may be the mask, in place of the one that the input had; and the picture
    // of either may be the mask of another node.
    let (_, inverse) = applied(
        &mut project,
        connect(Link::new(MEDIA_IN, "n6", Input::Mask)),
    );
    assert_eq!(inverse, connect(Link::new("n7", "n6", Input::Mask)));
    applied(&mut project, inverse);
    applied(&mut project, connect(Link::new("n4", "n5", Input::Input)));
    applied(&mut project, connect(Link::new("n3", "n5", Input::Input)));
    // Neither has a background or a foreground.
    for node in ["n5", "n6"] {
        for input in [Input::Background, Input::Foreground] {
            assert_eq!(
                refused(&project, connect(Link::new(MEDIA_IN, node, input))),
                OpError::NoInput {
                    node: node.into(),
                    input
                }
            );
        }
    }
    // A loop through either is a loop.
    assert_eq!(
        refused(&project, connect(Link::new("n5", "n5", Input::Mask))),
        OpError::Loop { node: "n5".into() }
    );
    for (from, to, input) in [
        ("n6", "n5", Input::Mask),
        ("n6", "n5", Input::Input),
        ("n6", "n3", Input::Mask),
        ("n5", "n7", Input::Mask),
    ] {
        assert!(
            matches!(
                refused(&project, connect(Link::new(from, to, input))),
                OpError::Loop { .. }
            ),
            "{from} into {to}"
        );
    }
    // Its picture and its mask are unlinked as another input is.
    let (_, inverse) = applied(&mut project, disconnect("n5", Input::Input));
    assert_eq!(inverse, connect(Link::new("n3", "n5", Input::Input)));
    assert_eq!(
        refused(&project, disconnect("n5", Input::Input)),
        missing("link", "n5")
    );
    let (_, inverse) = applied(&mut project, disconnect("n6", Input::Mask));
    assert_eq!(inverse, connect(Link::new("n7", "n6", Input::Mask)));
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "the two kinds as a layer has them, and as they are passed through"
)]
fn a_blur_and_a_mosaic_of_a_layer_are_their_numbers_at_the_frame_after_what_they_read() {
    let at = |id: &str, mask: Option<usize>| (id.to_owned(), mask);
    let mut project = filtered();
    assert_eq!(
        drawn(&project),
        [
            at("n4", None),
            at(MEDIA_IN, None),
            at("n3", None),
            at("n5", None),
            at("n6", None)
        ]
    );
    // An Ellipse, n7, limits both: it is there once, after the picture of the first that
    // reads it.
    applied(
        &mut project,
        add(
            Node::new("", Kind::Ellipse),
            vec![Link::new("", "n5", Input::Mask)],
        ),
    );
    applied(&mut project, connect(Link::new("n7", "n6", Input::Mask)));
    assert_eq!(
        drawn(&project),
        [
            at("n4", None),
            at(MEDIA_IN, None),
            at("n3", None),
            at("n7", None),
            at("n5", Some(3)),
            at("n6", Some(3))
        ]
    );
    // c1 shows m1 from its start at the rate of the timeline: frame 20 shows picture 20.
    applied(
        &mut project,
        set_keys("c1", "n5.size.x", keys(&[(10, 0.0), (30, 50.0)])),
    );
    applied(
        &mut project,
        set_keys("c1", "n5.size.y", Param::Fixed(80.0)),
    );
    applied(
        &mut project,
        set_keys("c1", "n6.frequency", keys(&[(10, 16.0), (30, 48.0)])),
    );
    applied(
        &mut project,
        set_params(
            "n6",
            json!({"whole_cells_only": true, "offset": {"x": 4.5}}),
        ),
    );
    let fused = layer_at(&project, 20).fusion.unwrap();
    assert_eq!(fused.output, Some(5));
    // With Lock X/Y the blur down is the blur across, whatever `size.y` is.
    assert_eq!(
        fused.nodes[4].kind,
        FusedKind::Blur {
            input: Some(2),
            mask: Some(3),
            settings: Blurred {
                filter: BlurFilter::Gaussian,
                size: [25.0, 25.0],
                blend: 1.0,
            },
        }
    );
    assert_eq!(
        fused.nodes[5].kind,
        FusedKind::Mosaic {
            input: Some(4),
            mask: Some(3),
            settings: Mosaicked {
                frequency: 32.0,
                aspect: 1.0,
                offset: [4.5, 0.0],
                whole_cells_only: true,
            },
        }
    );
    // Without it each is its own, and the filter is the one that is set.
    applied(
        &mut project,
        set_params(
            "n5",
            json!({"lock_xy": false, "filter": "box", "blend": 0.5}),
        ),
    );
    let fused = layer_at(&project, 20).fusion.unwrap();
    let FusedKind::Blur { settings, .. } = &fused.nodes[4].kind else {
        panic!("n5 is a blur")
    };
    assert_eq!(
        *settings,
        Blurred {
            filter: BlurFilter::Box,
            size: [25.0, 80.0],
            blend: 0.5,
        }
    );
    // A Blur that is passed through hands on its picture, and not its mask.
    applied(&mut project, passed_through("n5"));
    assert_eq!(
        drawn(&project),
        [
            at("n4", None),
            at(MEDIA_IN, None),
            at("n3", None),
            at("n7", None),
            at("n6", Some(3))
        ]
    );
    let fused = layer_at(&project, 20).fusion.unwrap();
    assert_eq!(fused.nodes[4].kind.first_input(), Some(2));
    // And a Mosaic too: the `MediaOut` shows the Merge, as it did before them.
    applied(&mut project, passed_through("n6"));
    assert_eq!(drawn(&project), drawn(&merged()));
    assert_eq!(
        drawn(&project),
        [at("n4", None), at(MEDIA_IN, None), at("n3", None)]
    );
    // One that is taken away with the link to put in its place leaves its picture to what
    // it fed.
    let mut joined = filtered();
    let bridge = vec![Link::new("n3", "n6", Input::Input)];
    let (_, inverse) = applied(&mut joined, remove("n5", Some(bridge)));
    assert!(matches!(&inverse, Op::AddFusionNode { links, .. } if links.len() == 2));
    assert_eq!(
        drawn(&joined),
        [
            at("n4", None),
            at(MEDIA_IN, None),
            at("n3", None),
            at("n6", None)
        ]
    );
}

#[test]
fn a_change_of_a_blur_or_of_a_mosaic_that_changes_nothing_makes_no_step() {
    let mut history = History::new(filtered());
    let revision = history.revision();
    for op in [
        set_params(
            "n5",
            json!({"filter": "gaussian", "lock_xy": true, "blend": 1.0, "size": {"y": 5.0}}),
        ),
        set_params(
            "n6",
            json!({"whole_cells_only": false, "offset": {"y": 0.0}}),
        ),
        set_keys("c1", "n6.aspect", Param::Fixed(1.0)),
        set_keys("c1", "n5.size.x", Param::Fixed(5.0)),
        connect(Link::new("n5", "n6", Input::Input)),
    ] {
        let nothing = history.change(vec![Change::Op(op.clone())], "desktop", |change, _| change);
        assert_eq!(nothing.err(), Some(TransactionError::Empty), "{op:?}");
    }
    assert_eq!(history.revision(), revision);
    assert!(!history.can_undo());
    // One that changes something is a step, and is undone: `size.y`, too, though Lock X/Y
    // leaves it unread.
    let before = history.project().clone();
    history
        .change(
            vec![Change::Op(set_params("n5", json!({"size": {"y": 9.0}})))],
            "desktop",
            |change, _| change,
        )
        .unwrap();
    assert_ne!(history.project(), &before);
    history.undo("desktop").unwrap().unwrap();
    assert_eq!(history.project(), &before);
}

// The Delta Keyer.

/// The sample of `merged` with the picture of c1 keyed by the Delta Keyer n5 before the Merge
/// n3 lays it over the Background n4: `next_id` is 6.
fn keyed() -> Project {
    let mut project = merged();
    applied(
        &mut project,
        add(
            Node::new("", Kind::DeltaKeyer),
            vec![
                Link::new(MEDIA_IN, "", Input::Input),
                Link::new("", "n3", Input::Foreground),
            ],
        ),
    );
    assert_eq!(project.next_id, 6);
    project
}

/// `keyed` with an Ellipse, n6, as the garbage matte of the keyer, and a Rectangle, n7, as
/// its solid matte: `next_id` is 8.
fn matted() -> Project {
    let mut project = keyed();
    for (kind, input) in [
        (Kind::Ellipse, Input::GarbageMatte),
        (Kind::Rectangle, Input::SolidMatte),
    ] {
        applied(
            &mut project,
            add(Node::new("", kind), vec![Link::new("", "n5", input)]),
        );
    }
    assert_eq!(project.next_id, 8);
    project
}

/// The Delta Keyer n5 of c1.
fn keyer_of(project: &Project) -> &columbina_model::fusion::DeltaKeyer {
    match &composition(project, "c1").node("n5").unwrap().kind {
        NodeKind::DeltaKeyer(keyer) => keyer,
        other => panic!("{other:?}"),
    }
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "a keyer as it is written, read, written by another program and taken away"
)]
fn a_delta_keyer_is_added_with_every_setting_written_and_is_read_back() {
    let mut project = keyed();
    let written = serde_json::to_value(composition(&project, "c1")).unwrap();
    assert_eq!(
        written["nodes"][4],
        json!({
            "id": "n5", "kind": "delta_keyer", "label": "", "pass_through": false, "pos": null,
            "params": {
                "screen": {"r": 0.0, "g": 1.0, "b": 0.0}, "gain": 1.0, "balance": 0.5,
                "pre_blur": 0.0, "low": 0.0, "high": 1.0, "erode_dilate": 0.0,
                "matte_blur": 0.0, "clean_foreground": 0.0, "clean_background": 0.0,
                "replace": "soft_color", "replace_color": {"r": 0.5, "g": 0.5, "b": 0.5},
                "garbage_invert": false, "spill_method": "medium", "spill_suppression": 1.0,
                "view": "final_result",
            },
        })
    );
    // The picture of the clip is its first input, and it is the foreground of the Merge.
    assert_eq!(
        written["links"],
        json!([
            {"from": "n3", "to": "MediaOut1", "input": "input"},
            {"from": "n4", "to": "n3", "input": "background"},
            {"from": "n5", "to": "n3", "input": "foreground"},
            {"from": "MediaIn1", "to": "n5", "input": "input"},
        ])
    );
    // To the byte, in the order of its settings.
    let nodes = &composition(&project, "c1").nodes;
    assert_eq!(
        serde_json::to_string(&nodes[4]).unwrap(),
        concat!(
            r#"{"id":"n5","kind":"delta_keyer","label":"","pass_through":false,"pos":null,"#,
            r#""params":{"screen":{"r":0.0,"g":1.0,"b":0.0},"gain":1.0,"balance":0.5,"#,
            r#""pre_blur":0.0,"low":0.0,"high":1.0,"erode_dilate":0.0,"matte_blur":0.0,"#,
            r#""clean_foreground":0.0,"clean_background":0.0,"replace":"soft_color","#,
            r#""replace_color":{"r":0.5,"g":0.5,"b":0.5},"garbage_invert":false,"#,
            r#""spill_method":"medium","spill_suppression":1.0,"view":"final_result"}}"#,
        )
    );
    // A project with it is read back as it was written, and written the same again.
    let text = serde_json::to_string(&project).unwrap();
    let read: Project = serde_json::from_str(&text).unwrap();
    assert_eq!(read, project);
    assert_eq!(serde_json::to_string(&read).unwrap(), text);
    let value = serde_json::to_value(&project).unwrap();
    assert_eq!(Project::from_value(value).unwrap(), project);
    // A keyer as another program writes it, with every setting, its garbage matte the
    // Background and its mask the keyer before it.
    let op: Op = serde_json::from_value(json!({
        "op": "add_fusion_node", "timeline": "t1", "clip": "c1",
        "node": {"id": "", "kind": "delta_keyer", "label": "Blue screen", "params": {
            "screen": {"r": 0.1, "g": 0.15, "b": 0.5}, "gain": 2, "balance": 0.25,
            "pre_blur": 3, "low": 0.1, "high": 0.9, "erode_dilate": -1.5, "matte_blur": 2,
            "clean_foreground": 0.2, "clean_background": 0.1, "replace": "hard_color",
            "replace_color": {"r": 0.4, "g": 0.3, "b": 0.2}, "garbage_invert": true,
            "spill_method": "well_done", "spill_suppression": 0.5, "view": "status",
        }},
        "links": [
            {"from": "n4", "to": "", "input": "garbage_matte"},
            {"from": "n5", "to": "", "input": "mask"},
        ],
    }))
    .unwrap();
    let (done, inverse) = applied(&mut project, op);
    assert_eq!(inverse, remove("n6", Some(vec![])));
    let Op::AddFusionNode { node, links, .. } = done else {
        panic!("a node was added")
    };
    assert_eq!(node.id, "n6");
    assert_eq!(
        links,
        [
            Link::new("n4", "n6", Input::GarbageMatte),
            Link::new("n5", "n6", Input::Mask),
        ]
    );
    let NodeKind::DeltaKeyer(blue) = &node.kind else {
        panic!("n6 is a delta keyer")
    };
    assert_eq!(blue.screen.b, Param::Fixed(0.5));
    assert_eq!(blue.gain, Param::Fixed(2.0));
    assert_eq!(blue.erode_dilate, Param::Fixed(-1.5));
    assert_eq!(blue.replace, ReplaceMode::HardColor);
    assert_eq!(blue.replace_color.g, Param::Fixed(0.3));
    assert_eq!(blue.spill_method, SpillMethod::WellDone);
    assert_eq!(blue.view, KeyerView::Status);
    assert!(blue.garbage_invert);
    // One whose settings are not all there, or that has a setting that a keyer has not,
    // cannot be read.
    let whole = serde_json::to_value(Node::new("", Kind::DeltaKeyer)).unwrap()["params"].clone();
    for setting in [
        "screen",
        "gain",
        "balance",
        "pre_blur",
        "low",
        "high",
        "erode_dilate",
        "matte_blur",
        "clean_foreground",
        "clean_background",
        "replace",
        "replace_color",
        "garbage_invert",
        "spill_method",
        "spill_suppression",
        "view",
    ] {
        let mut params = whole.clone();
        params.as_object_mut().unwrap().remove(setting);
        let node = json!({"id": "", "kind": "delta_keyer", "label": "", "params": params});
        assert!(serde_json::from_value::<Node>(node).is_err(), "{setting}");
    }
    let mut more = whole.clone();
    more["fringe_gamma"] = json!(1.0);
    let node = json!({"id": "", "kind": "delta_keyer", "label": "", "params": more});
    assert!(serde_json::from_value::<Node>(node).is_err());
    let node = json!({"id": "", "kind": "delta_keyer", "label": "", "params": whole});
    assert!(serde_json::from_value::<Node>(node).is_ok());
    // Taken away again, the first with the link that takes its place, they leave the
    // composition as it was before them, and written as it was: one without a keyer is
    // written as it always was.
    applied(&mut project, remove("n6", None));
    let bridge = vec![Link::new(MEDIA_IN, "n3", Input::Foreground)];
    applied(&mut project, remove("n5", Some(bridge)));
    let before = merged();
    assert_eq!(composition(&project, "c1"), composition(&before, "c1"));
    assert_eq!(
        serde_json::to_string(composition(&project, "c1")).unwrap(),
        serde_json::to_string(composition(&before, "c1")).unwrap()
    );
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "a refusal for each setting of a keyer, for its screen and for its thresholds"
)]
fn the_settings_of_a_delta_keyer_are_those_of_its_kind_within_their_ranges() {
    let mut project = keyed();
    for (patch, what) in [
        (json!({"screen": {"r": -0.1}}), "delta_keyer.screen.r"),
        (json!({"screen": {"r": 65505}}), "delta_keyer.screen.r"),
        (json!({"screen": {"g": -0.5}}), "delta_keyer.screen.g"),
        (json!({"screen": {"g": 70000}}), "delta_keyer.screen.g"),
        (json!({"screen": {"b": -1}}), "delta_keyer.screen.b"),
        (json!({"screen": {"b": 65504.5}}), "delta_keyer.screen.b"),
        (json!({"gain": -0.1}), "delta_keyer.gain"),
        (json!({"gain": 10.5}), "delta_keyer.gain"),
        (json!({"balance": -0.1}), "delta_keyer.balance"),
        (json!({"balance": 1.1}), "delta_keyer.balance"),
        (json!({"pre_blur": -1}), "delta_keyer.pre_blur"),
        (json!({"pre_blur": 100.5}), "delta_keyer.pre_blur"),
        (json!({"low": -0.1}), "delta_keyer.low"),
        (json!({"low": 1.1}), "delta_keyer.low"),
        (json!({"high": -0.1}), "delta_keyer.high"),
        (json!({"high": 1.1}), "delta_keyer.high"),
        (json!({"erode_dilate": -100.5}), "delta_keyer.erode_dilate"),
        (json!({"erode_dilate": 101}), "delta_keyer.erode_dilate"),
        (json!({"matte_blur": -0.5}), "delta_keyer.matte_blur"),
        (json!({"matte_blur": 101}), "delta_keyer.matte_blur"),
        (
            json!({"clean_foreground": -0.1}),
            "delta_keyer.clean_foreground",
        ),
        (
            json!({"clean_foreground": 1.5}),
            "delta_keyer.clean_foreground",
        ),
        (
            json!({"clean_background": -0.1}),
            "delta_keyer.clean_background",
        ),
        (
            json!({"clean_background": 1.5}),
            "delta_keyer.clean_background",
        ),
        (
            json!({"replace_color": {"r": -0.1}}),
            "delta_keyer.replace_color.r",
        ),
        (
            json!({"replace_color": {"g": 65505}}),
            "delta_keyer.replace_color.g",
        ),
        (
            json!({"replace_color": {"b": -2}}),
            "delta_keyer.replace_color.b",
        ),
        (
            json!({"spill_suppression": -0.1}),
            "delta_keyer.spill_suppression",
        ),
        (
            json!({"spill_suppression": 1.5}),
            "delta_keyer.spill_suppression",
        ),
        // A screen colour has one channel above its other two.
        (json!({"screen": {"g": 0}}), "delta_keyer.screen"),
        (
            json!({"screen": {"r": 0.5, "g": 0.5, "b": 0.5}}),
            "delta_keyer.screen",
        ),
        (json!({"screen": {"b": 1}}), "delta_keyer.screen"),
        // And the lower threshold is at most the upper.
        (json!({"low": 0.6, "high": 0.4}), "delta_keyer.low"),
    ] {
        assert_eq!(
            refused(&project, set_params("n5", patch.clone())),
            range(what),
            "{patch}"
        );
    }
    // What is not a setting of the kind, or not written as one, is refused.
    for patch in [
        json!({"replace": "colour"}),
        json!({"spill_method": "raw"}),
        json!({"view": "intermediate_result"}),
        json!({"garbage_invert": 1}),
        json!({"screen": 0.5}),
        json!({"screen": {"a": 1.0}}),
        json!({"replace_color": {"a": 1.0}}),
        json!({"size": {"x": 5.0}}),
        json!({"fringe_gamma": 1.0}),
        json!({"solid_invert": true}),
    ] {
        let error = refused(&project, set_params("n5", patch.clone()));
        assert!(
            matches!(&error, OpError::Params { node, .. } if node == "n5"),
            "{patch}: {error:?}"
        );
    }
    // What is given is set, and set back by the inverse.
    let (_, inverse) = applied(
        &mut project,
        set_params(
            "n5",
            json!({
                "screen": {"r": 0.08, "g": 0.45, "b": 0.12}, "gain": 2, "replace": "hard_color",
                "spill_method": "burnt", "garbage_invert": true, "view": "status",
            }),
        ),
    );
    assert_eq!(
        patch_of(&inverse),
        Some(json!({
            "screen": {"r": 0.0, "g": 1.0, "b": 0.0}, "gain": 1.0, "replace": "soft_color",
            "spill_method": "medium", "garbage_invert": false, "view": "final_result",
        }))
    );
    let keyer = keyer_of(&project);
    assert_eq!(keyer.screen.r, Param::Fixed(0.08));
    assert_eq!(keyer.screen.g, Param::Fixed(0.45));
    assert_eq!(keyer.screen.b, Param::Fixed(0.12));
    assert_eq!(keyer.gain, Param::Fixed(2.0));
    assert_eq!(keyer.balance, Param::Fixed(0.5));
    assert_eq!(keyer.replace, ReplaceMode::HardColor);
    assert_eq!(keyer.spill_method, SpillMethod::Burnt);
    assert_eq!(keyer.view, KeyerView::Status);
    assert!(keyer.garbage_invert);
    // The ends of each range are within it; thresholds alike will do; and any channel may
    // be the one of the screen, above 1 too.
    for patch in [
        json!({"gain": 0, "balance": 0, "pre_blur": 100, "erode_dilate": -100, "matte_blur": 100}),
        json!({"gain": 10, "balance": 1, "pre_blur": 0, "erode_dilate": 100, "matte_blur": 0}),
        json!({"clean_foreground": 1, "clean_background": 1, "spill_suppression": 0}),
        json!({"low": 0.5, "high": 0.5}),
        json!({"low": 0, "high": 0}),
        json!({"low": 1, "high": 1}),
        json!({"screen": {"r": 0, "g": 0, "b": 65504}}),
        json!({"screen": {"r": 4, "g": 0.5, "b": 0.25}}),
        json!({"replace_color": {"r": 0, "g": 65504, "b": 0}}),
        json!({"replace": "none", "spill_method": "none", "view": "pre_matte"}),
        json!({"replace": "source", "spill_method": "rare", "view": "matte"}),
    ] {
        applied(&mut project, set_params("n5", patch));
    }
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "a key for each setting of a keyer, and what its keys are not held to"
)]
fn the_keys_of_a_delta_keyer_are_set_by_the_paths_of_its_settings() {
    let mut project = keyed();
    // The first keys of a setting that had none are done as the keys set whole.
    let mut first = project.clone();
    let (done, inverse) = applied(&mut first, splice("n5.gain", 0, 0, vec![key(3, 2.5)]));
    assert_eq!(done, set_keys("c1", "n5.gain", keys(&[(3, 2.5)])));
    assert_eq!(inverse, set_keys("c1", "n5.gain", Param::Fixed(1.0)));
    for (path, value) in [
        ("n5.screen.r", 0.25),
        ("n5.screen.g", 0.75),
        ("n5.screen.b", 0.125),
        ("n5.gain", 3.0),
        ("n5.balance", 0.25),
        ("n5.pre_blur", 12.0),
        ("n5.low", 0.25),
        ("n5.high", 0.75),
        ("n5.erode_dilate", -4.0),
        ("n5.matte_blur", 6.0),
        ("n5.clean_foreground", 0.5),
        ("n5.clean_background", 0.25),
        ("n5.replace_color.r", 0.75),
        ("n5.replace_color.g", 0.25),
        ("n5.replace_color.b", 2.0),
        ("n5.spill_suppression", 0.5),
    ] {
        let rising = keys(&[(0, 1.0), (9, value)]);
        let (done, inverse) = applied(&mut project, set_keys("c1", path, rising.clone()));
        assert_eq!(done, set_keys("c1", path, rising));
        assert!(
            matches!(
                &inverse,
                Op::SetFusionKeys {
                    param: Param::Fixed(_),
                    ..
                }
            ),
            "{path}: {inverse:?}"
        );
    }
    assert!(composition(&project, "c1").is_animated());
    // A key is put in place of another, and taken back.
    let (done, inverse) = applied(
        &mut project,
        splice("n5.replace_color.b", 1, 1, vec![key(5, 3.0)]),
    );
    assert_eq!(done, splice("n5.replace_color.b", 1, 1, vec![key(5, 3.0)]));
    assert_eq!(
        inverse,
        splice("n5.replace_color.b", 1, 1, vec![key(9, 2.0)])
    );
    let keyer = keyer_of(&project);
    assert_eq!(keyer.replace_color.b, keys(&[(0, 1.0), (5, 3.0)]));
    assert_eq!(keyer.screen.g, keys(&[(0, 1.0), (9, 0.75)]));
    assert_eq!(keyer.gain, keys(&[(0, 1.0), (9, 3.0)]));
    assert_eq!(keyer.low, keys(&[(0, 1.0), (9, 0.25)]));
    assert_eq!(keyer.erode_dilate, keys(&[(0, 1.0), (9, -4.0)]));
    // The keys are moved with the frames that they count, as the others are.
    let later = composition(&project, "c1").with_key_frames(&|frame| frame + 100);
    let NodeKind::DeltaKeyer(moved) = &later.node("n5").unwrap().kind else {
        panic!("n5 is a delta keyer")
    };
    assert_eq!(moved.gain, keys(&[(100, 1.0), (109, 3.0)]));
    assert_eq!(moved.screen.b, keys(&[(100, 1.0), (109, 0.125)]));
    // What is no number takes no keys, and a kind has its own settings only.
    for path in [
        "n5.screen",
        "n5.screen.a",
        "n5.replace",
        "n5.replace_color",
        "n5.garbage_invert",
        "n5.spill_method",
        "n5.view",
        "n5.size.x",
        "n5.blend",
        "n3.gain",
        "n4.screen.g",
    ] {
        assert_eq!(
            refused(&project, set_keys("c1", path, Param::Fixed(0.5))),
            missing("setting", path),
            "{path}"
        );
        assert_eq!(
            refused(&project, splice(path, 0, 0, vec![key(0, 0.5)])),
            missing("setting", path),
            "{path}"
        );
    }
    // What is put is checked as the node is.
    assert_eq!(
        refused(&project, set_keys("c1", "n5.gain", Param::Fixed(10.5))),
        range("delta_keyer.gain")
    );
    assert_eq!(
        refused(
            &project,
            set_keys("c1", "n5.screen.g", keys(&[(0, 1.0), (5, -0.5)]))
        ),
        range("delta_keyer.screen.g")
    );
    assert_eq!(
        refused(&project, splice("n5.low", 2, 0, vec![key(20, 1.5)])),
        range("delta_keyer.low")
    );
    assert_eq!(
        refused(
            &project,
            splice("n5.replace_color.r", 2, 0, vec![key(20, -1.0)])
        ),
        range("delta_keyer.replace_color.r")
    );
    // Numbers that have no keys are held to more: a screen colour with no channel above its
    // other two, and thresholds out of their order, are refused.
    let mut fixed = keyed();
    assert_eq!(
        refused(&fixed, set_keys("c1", "n5.screen.g", Param::Fixed(0.0))),
        range("delta_keyer.screen")
    );
    assert_eq!(
        refused(&fixed, set_keys("c1", "n5.screen.r", Param::Fixed(1.0))),
        range("delta_keyer.screen")
    );
    applied(&mut fixed, set_keys("c1", "n5.high", Param::Fixed(0.5)));
    assert_eq!(
        refused(&fixed, set_keys("c1", "n5.low", Param::Fixed(0.75))),
        range("delta_keyer.low")
    );
    // But keys are not, since they cannot be checked at every frame: a screen colour whose
    // green falls to nothing, and a lower threshold that rises past the upper.
    applied(
        &mut fixed,
        set_keys("c1", "n5.screen.g", keys(&[(0, 1.0), (20, 0.0)])),
    );
    applied(
        &mut fixed,
        set_keys("c1", "n5.low", keys(&[(0, 0.0), (20, 1.0)])),
    );
    // c1 shows m1 from its start at the rate of the timeline: frame 20 shows picture 20,
    // where the screen is black and the thresholds have crossed. That is what the
    // compositor is given; what it makes of it is told with it: there the keyer keys
    // nothing.
    let fused = layer_at(&fixed, 20).fusion.unwrap();
    let settings = fused
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            FusedKind::DeltaKeyer { settings, .. } => Some(*settings),
            _ => None,
        })
        .expect("the layer draws its keyer");
    assert_eq!(settings.screen, [0.0, 0.0, 0.0]);
    assert_eq!((settings.low, settings.high), (1.0, 0.5));
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "each input of a keyer, and each kind that has no matte"
)]
fn a_delta_keyer_takes_a_picture_two_mattes_and_a_mask_and_no_other_kind_takes_a_matte() {
    let mut project = keyed();
    // An Ellipse, n6, is the garbage matte of the keyer.
    let (_, inverse) = applied(
        &mut project,
        add(
            Node::new("", Kind::Ellipse),
            vec![Link::new("", "n5", Input::GarbageMatte)],
        ),
    );
    assert_eq!(inverse, remove("n6", Some(vec![])));
    // It is its solid matte and its mask too: one output may feed several inputs.
    for input in [Input::SolidMatte, Input::Mask] {
        let (done, inverse) = applied(&mut project, connect(Link::new("n6", "n5", input)));
        assert_eq!(done, connect(Link::new("n6", "n5", input)));
        assert_eq!(inverse, disconnect("n5", input));
    }
    // Any picture may be a matte, in place of the one that the input had.
    let (_, inverse) = applied(
        &mut project,
        connect(Link::new("n4", "n5", Input::SolidMatte)),
    );
    assert_eq!(inverse, connect(Link::new("n6", "n5", Input::SolidMatte)));
    applied(
        &mut project,
        connect(Link::new(MEDIA_IN, "n5", Input::GarbageMatte)),
    );
    applied(
        &mut project,
        connect(Link::new("n6", "n5", Input::GarbageMatte)),
    );
    // A keyer has no background and no foreground; and no other kind has a matte.
    for input in [Input::Background, Input::Foreground] {
        assert_eq!(
            refused(&project, connect(Link::new(MEDIA_IN, "n5", input))),
            OpError::NoInput {
                node: "n5".into(),
                input
            }
        );
    }
    for node in [MEDIA_IN, MEDIA_OUT, "n3", "n4", "n6"] {
        for input in [Input::GarbageMatte, Input::SolidMatte] {
            assert_eq!(
                refused(&project, connect(Link::new("n6", node, input))),
                OpError::NoInput {
                    node: node.into(),
                    input
                },
                "{node}"
            );
        }
    }
    let filters = filtered();
    for node in ["n5", "n6"] {
        assert_eq!(
            refused(
                &filters,
                connect(Link::new(MEDIA_IN, node, Input::GarbageMatte))
            ),
            OpError::NoInput {
                node: node.into(),
                input: Input::GarbageMatte
            }
        );
    }
    // A node that is added with a link into a matte of a kind that has none is refused too.
    let wrong = add(
        Node::new("", Kind::Ellipse),
        vec![Link::new("", "n5", Input::SolidMatte)],
    );
    assert_eq!(
        refused(&filters, wrong),
        OpError::NoInput {
            node: "n5".into(),
            input: Input::SolidMatte
        }
    );
    // A loop through a matte is a loop: of one node, and of several.
    assert_eq!(
        refused(
            &project,
            connect(Link::new("n5", "n5", Input::GarbageMatte))
        ),
        OpError::Loop { node: "n5".into() }
    );
    for (from, to, input) in [
        ("n3", "n5", Input::GarbageMatte),
        ("n3", "n5", Input::SolidMatte),
        ("n5", "n6", Input::Mask),
        ("n3", "n6", Input::Mask),
    ] {
        assert!(
            matches!(
                refused(&project, connect(Link::new(from, to, input))),
                OpError::Loop { .. }
            ),
            "{from} into {to}"
        );
    }
    // A matte is unlinked as another input is.
    let (_, inverse) = applied(&mut project, disconnect("n5", Input::GarbageMatte));
    assert_eq!(inverse, connect(Link::new("n6", "n5", Input::GarbageMatte)));
    assert_eq!(
        refused(&project, disconnect("n5", Input::GarbageMatte)),
        missing("link", "n5")
    );
    // The links of a keyer are kept by the names of its inputs.
    applied(&mut project, inverse);
    let into: Vec<&str> = composition(&project, "c1")
        .links
        .iter()
        .filter(|link| link.to == "n5")
        .map(|link| link.input.name())
        .collect();
    assert_eq!(into, ["garbage_matte", "input", "mask", "solid_matte"]);
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "a keyer as a layer has it, after what it reads, and as it is passed through"
)]
fn a_delta_keyer_of_a_layer_is_its_numbers_at_the_frame_after_what_it_reads() {
    let at = |id: &str, mask: Option<usize>| (id.to_owned(), mask);
    assert_eq!(
        drawn(&keyed()),
        [
            at("n4", None),
            at(MEDIA_IN, None),
            at("n5", None),
            at("n3", None)
        ]
    );
    // An Ellipse, n6, that is read as its garbage matte alone is among the nodes, after its
    // picture; and a Rectangle, n7, its solid matte, after that.
    let mut project = matted();
    assert_eq!(
        drawn(&project),
        [
            at("n4", None),
            at(MEDIA_IN, None),
            at("n6", None),
            at("n7", None),
            at("n5", None),
            at("n3", None)
        ]
    );
    // The Ellipse is its mask too: it is there once.
    applied(&mut project, connect(Link::new("n6", "n5", Input::Mask)));
    assert_eq!(drawn(&project)[4], at("n5", Some(2)));
    // c1 shows m1 from its start at the rate of the timeline: frame 20 shows picture 20.
    applied(
        &mut project,
        set_keys("c1", "n5.gain", keys(&[(10, 1.0), (30, 3.0)])),
    );
    applied(
        &mut project,
        set_keys("c1", "n5.screen.b", keys(&[(10, 0.0), (30, 0.5)])),
    );
    applied(
        &mut project,
        set_params(
            "n5",
            json!({
                "low": 0.125, "high": 0.875, "pre_blur": 4, "erode_dilate": -2,
                "matte_blur": 3, "clean_foreground": 0.5, "clean_background": 0.25,
                "balance": 0.75, "replace": "source", "replace_color": {"r": 0.25},
                "garbage_invert": true, "spill_method": "rare", "spill_suppression": 0.5,
                "view": "matte",
            }),
        ),
    );
    let fused = layer_at(&project, 20).fusion.unwrap();
    assert_eq!(fused.output, Some(5));
    assert_eq!(
        fused.nodes[4].kind,
        FusedKind::DeltaKeyer {
            input: Some(1),
            garbage_matte: Some(2),
            solid_matte: Some(3),
            mask: Some(2),
            settings: DeltaKeyed {
                screen: [0.0, 1.0, 0.25],
                gain: 2.0,
                balance: 0.75,
                pre_blur: 4.0,
                low: 0.125,
                high: 0.875,
                erode_dilate: -2.0,
                matte_blur: 3.0,
                clean_foreground: 0.5,
                clean_background: 0.25,
                replace: ReplaceMode::Source,
                replace_color: [0.25, 0.5, 0.5],
                garbage_invert: true,
                spill_method: SpillMethod::Rare,
                spill_suppression: 0.5,
                view: KeyerView::Matte,
            },
        }
    );
    assert_eq!(fused.nodes[4].kind.first_input(), Some(1));
    assert_eq!(fused.nodes[4].kind.mask(), Some(2));
    assert_eq!(fused.nodes[4].kind.reads(), [1, 2, 3, 2]);
    assert_eq!(fused.nodes[5].kind.reads(), [0, 4]);
    // A matte that is unlinked is not there, and the node that it read is not drawn where
    // nothing else reads it.
    let mut bare = project.clone();
    applied(&mut bare, disconnect("n5", Input::SolidMatte));
    assert_eq!(
        drawn(&bare),
        [
            at("n4", None),
            at(MEDIA_IN, None),
            at("n6", None),
            at("n5", Some(2)),
            at("n3", None)
        ]
    );
    // A keyer that is passed through hands on its picture, and neither its mattes nor its
    // mask: the Merge lays the picture of the clip, as it did before the keyer.
    applied(&mut project, passed_through("n5"));
    assert_eq!(drawn(&project), drawn(&merged()));
    assert_eq!(
        drawn(&project),
        [at("n4", None), at(MEDIA_IN, None), at("n3", None)]
    );
    // One that is taken away with the link to put in its place leaves its picture to what
    // it fed.
    let mut joined = matted();
    let bridge = vec![Link::new(MEDIA_IN, "n3", Input::Foreground)];
    let (_, inverse) = applied(&mut joined, remove("n5", Some(bridge)));
    assert!(matches!(&inverse, Op::AddFusionNode { links, .. } if links.len() == 4));
    assert_eq!(drawn(&joined), drawn(&merged()));
}

#[test]
fn a_change_of_a_delta_keyer_that_changes_nothing_makes_no_step() {
    let mut history = History::new(matted());
    let revision = history.revision();
    for op in [
        set_params(
            "n5",
            json!({
                "screen": {"g": 1.0}, "replace": "soft_color", "spill_method": "medium",
                "garbage_invert": false, "view": "final_result", "high": 1.0,
            }),
        ),
        set_params("n5", json!({"replace_color": {"b": 0.5}})),
        set_keys("c1", "n5.gain", Param::Fixed(1.0)),
        set_keys("c1", "n5.screen.r", Param::Fixed(0.0)),
        connect(Link::new("n6", "n5", Input::GarbageMatte)),
        connect(Link::new("n7", "n5", Input::SolidMatte)),
    ] {
        let nothing = history.change(vec![Change::Op(op.clone())], "desktop", |change, _| change);
        assert_eq!(nothing.err(), Some(TransactionError::Empty), "{op:?}");
    }
    assert_eq!(history.revision(), revision);
    assert!(!history.can_undo());
    // One that changes something is a step, and is undone: the view too, which is a setting
    // of the node.
    let before = history.project().clone();
    history
        .change(
            vec![Change::Op(set_params("n5", json!({"view": "status"})))],
            "desktop",
            |change, _| change,
        )
        .unwrap();
    assert_ne!(history.project(), &before);
    history.undo("desktop").unwrap().unwrap();
    assert_eq!(history.project(), &before);
}

// The Tracker.

/// The sample of `merged` with a Tracker, n5, of the picture of c1, whose picture the Merge
/// n3 lays over the Background n4 in the place of the picture of the clip: `next_id` is 6.
fn tracked() -> Project {
    let mut project = merged();
    applied(
        &mut project,
        add(
            Node::new("", Kind::Tracker),
            vec![
                Link::new(MEDIA_IN, "", Input::Input),
                Link::new("", "n3", Input::Foreground),
            ],
        ),
    );
    assert_eq!(project.next_id, 6);
    project
}

/// A pattern as it is written: named `name`, put at `at`, as large as one is at first, with
/// no offset, and with `path`.
fn written_pattern(name: &str, at: [f64; 2], path: serde_json::Value) -> serde_json::Value {
    let mut pattern = json!({
        "name": name, "enabled": true,
        "pattern": {"x": at[0], "y": at[1], "width": 0.05, "height": 0.05},
        "search": {"width": 0.125, "height": 0.125},
        "offset": {"x": 0.0, "y": 0.0},
    });
    pattern["path"] = path;
    pattern
}

/// Two patterns as they are written, which both go a sixteenth of the width to the right
/// and an eighth of the height up from frame 10 to frame 20.
fn two_patterns() -> serde_json::Value {
    json!([
        written_pattern(
            "Left",
            [0.25, 0.5],
            json!({"10": [0.25, 0.5], "20": [0.3125, 0.625]}),
        ),
        written_pattern(
            "Right",
            [0.75, 0.5],
            json!({"10": [0.75, 0.5], "20": [0.8125, 0.625]}),
        ),
    ])
}

/// The two patterns of `two_patterns` with `change` made to the second, as the `trackers`
/// of a patch.
fn with_second(change: fn(&mut serde_json::Value)) -> serde_json::Value {
    let mut patch = json!({ "trackers": two_patterns() });
    change(&mut patch["trackers"][1]);
    patch
}

/// The Tracker n5 of c1.
fn tracker_of(project: &Project) -> &Tracker {
    match &composition(project, "c1").node("n5").unwrap().kind {
        NodeKind::Tracker(tracker) => tracker,
        other => panic!("{other:?}"),
    }
}

/// What the Tracker that the one layer of the timeline draws at `frame` draws there.
fn tracked_at(project: &Project, frame: i64) -> Tracked {
    let fused = layer_at(project, frame)
        .fusion
        .expect("the layer has its composition");
    fused
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            FusedKind::Tracker { settings, .. } => Some(*settings),
            _ => None,
        })
        .expect("the layer draws its tracker")
}

/// Whether two places are one, but for the rounding of a few floats.
fn near(one: [f64; 2], other: [f64; 2]) -> bool {
    (one[0] - other[0]).abs() < 1e-9 && (one[1] - other[1]).abs() < 1e-9
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "a tracker as it is written, read, written by another program and taken away"
)]
fn a_tracker_is_added_with_every_setting_written_and_is_read_back() {
    let mut project = tracked();
    let written = serde_json::to_value(composition(&project, "c1")).unwrap();
    assert_eq!(
        written["nodes"][4],
        json!({
            "id": "n5", "kind": "tracker", "label": "", "pass_through": false, "pos": null,
            "params": {
                "operation": "none", "merge": "bg_only", "position": true, "rotation": true,
                "scaling": true, "pivot": "tracker_average", "pivot_tracker": 0,
                "pivot_point": {"x": 0.5, "y": 0.5}, "reference": 0, "trackers": [],
            },
        })
    );
    // The picture of the clip is its first input, and it is the foreground of the Merge.
    assert_eq!(
        written["links"],
        json!([
            {"from": "n3", "to": "MediaOut1", "input": "input"},
            {"from": "n4", "to": "n3", "input": "background"},
            {"from": "n5", "to": "n3", "input": "foreground"},
            {"from": "MediaIn1", "to": "n5", "input": "input"},
        ])
    );
    // To the byte, in the order of its settings.
    let nodes = &composition(&project, "c1").nodes;
    assert_eq!(
        serde_json::to_string(&nodes[4]).unwrap(),
        concat!(
            r#"{"id":"n5","kind":"tracker","label":"","pass_through":false,"pos":null,"#,
            r#""params":{"operation":"none","merge":"bg_only","position":true,"#,
            r#""rotation":true,"scaling":true,"pivot":"tracker_average","pivot_tracker":0,"#,
            r#""pivot_point":{"x":0.5,"y":0.5},"reference":0,"trackers":[]}}"#,
        )
    );
    // A project with it is read back as it was written, and written the same again.
    let text = serde_json::to_string(&project).unwrap();
    let read: Project = serde_json::from_str(&text).unwrap();
    assert_eq!(read, project);
    assert_eq!(serde_json::to_string(&read).unwrap(), text);
    // A Tracker as another program writes it, with every setting and two patterns with
    // their paths: the Background its foreground, and the Tracker before it its mask.
    let op: Op = serde_json::from_value(json!({
        "op": "add_fusion_node", "timeline": "t1", "clip": "c1",
        "node": {"id": "", "kind": "tracker", "label": "Sign", "params": {
            "operation": "match_move", "merge": "fg_over_bg", "position": true,
            "rotation": false, "scaling": true, "pivot": "selected_tracker",
            "pivot_tracker": 1, "pivot_point": {"x": 0.25, "y": 0.75}, "reference": 10,
            "trackers": two_patterns(),
        }},
        "links": [
            {"from": "MediaIn1", "to": "", "input": "input"},
            {"from": "n4", "to": "", "input": "foreground"},
            {"from": "n5", "to": "", "input": "mask"},
        ],
    }))
    .unwrap();
    let (done, inverse) = applied(&mut project, op);
    assert_eq!(inverse, remove("n6", Some(vec![])));
    let Op::AddFusionNode { node, links, .. } = done else {
        panic!("a node was added")
    };
    assert_eq!(node.id, "n6");
    assert_eq!(
        links,
        [
            Link::new("n4", "n6", Input::Foreground),
            Link::new(MEDIA_IN, "n6", Input::Input),
            Link::new("n5", "n6", Input::Mask),
        ]
    );
    let NodeKind::Tracker(sign) = &node.kind else {
        panic!("n6 is a tracker")
    };
    assert_eq!(sign.operation, TrackerOperation::MatchMove);
    assert_eq!(sign.merge, TrackerMerge::FgOverBg);
    assert_eq!(sign.pivot, TrackerPivot::SelectedTracker);
    assert_eq!((sign.pivot_tracker, sign.reference), (1, 10));
    assert_eq!(sign.pivot_point, Place { x: 0.25, y: 0.75 });
    assert!(sign.position && !sign.rotation && sign.scaling);
    assert_eq!(sign.trackers.len(), 2);
    assert_eq!(sign.trackers[1].name, "Right");
    let path: Vec<(i64, [f64; 2])> = sign.trackers[0]
        .path
        .iter()
        .map(|(&frame, &at)| (frame, at))
        .collect();
    assert_eq!(path, [(10, [0.25, 0.5]), (20, [0.3125, 0.625])]);
    assert_eq!(
        serde_json::to_string(&sign.trackers[0]).unwrap(),
        concat!(
            r#"{"name":"Left","enabled":true,"pattern":{"x":0.25,"y":0.5,"width":0.05,"#,
            r#""height":0.05},"search":{"width":0.125,"height":0.125},"#,
            r#""offset":{"x":0.0,"y":0.0},"path":{"10":[0.25,0.5],"20":[0.3125,0.625]}}"#,
        )
    );
    let text = serde_json::to_string(&project).unwrap();
    let read: Project = serde_json::from_str(&text).unwrap();
    assert_eq!(read, project);
    assert_eq!(serde_json::to_string(&read).unwrap(), text);
    let value = serde_json::to_value(&project).unwrap();
    assert_eq!(Project::from_value(value).unwrap(), project);
    // One whose settings are not all there, or that has a setting that a Tracker has not,
    // cannot be read; and so it is with a pattern.
    let whole = serde_json::to_value(&node).unwrap()["params"].clone();
    for setting in [
        "operation",
        "merge",
        "position",
        "rotation",
        "scaling",
        "pivot",
        "pivot_tracker",
        "pivot_point",
        "reference",
        "trackers",
    ] {
        let mut params = whole.clone();
        params.as_object_mut().unwrap().remove(setting);
        let node = json!({"id": "", "kind": "tracker", "label": "", "params": params});
        assert!(serde_json::from_value::<Node>(node).is_err(), "{setting}");
    }
    for setting in ["name", "enabled", "pattern", "search", "offset", "path"] {
        let mut params = whole.clone();
        params["trackers"][0]
            .as_object_mut()
            .unwrap()
            .remove(setting);
        let node = json!({"id": "", "kind": "tracker", "label": "", "params": params});
        assert!(serde_json::from_value::<Node>(node).is_err(), "{setting}");
    }
    let mut more = whole.clone();
    more["adaptive_mode"] = json!("none");
    let node = json!({"id": "", "kind": "tracker", "label": "", "params": more});
    assert!(serde_json::from_value::<Node>(node).is_err());
    let node = json!({"id": "", "kind": "tracker", "label": "", "params": whole});
    assert!(serde_json::from_value::<Node>(node).is_ok());
    // Taken away again, the first with the link that takes its place, they leave the
    // composition as it was before them, and written as it was: one without a Tracker is
    // written as it always was.
    applied(&mut project, remove("n6", None));
    let bridge = vec![Link::new(MEDIA_IN, "n3", Input::Foreground)];
    applied(&mut project, remove("n5", Some(bridge)));
    let before = merged();
    assert_eq!(composition(&project, "c1"), composition(&before, "c1"));
    assert_eq!(
        serde_json::to_string(composition(&project, "c1")).unwrap(),
        serde_json::to_string(composition(&before, "c1")).unwrap()
    );
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "a refusal for each number of a tracker and of its patterns"
)]
fn the_settings_of_a_tracker_are_those_of_its_kind_within_their_ranges() {
    let mut project = tracked();
    let many: Vec<serde_json::Value> = (0..=MOST_TRACKERS)
        .map(|_| written_pattern("", [0.5, 0.5], json!({})))
        .collect();
    for (patch, what) in [
        (json!({ "trackers": many }), "tracker.trackers"),
        (
            with_second(|pattern| pattern["name"] = json!("x".repeat(LONGEST_LABEL + 1))),
            "tracker.trackers.name",
        ),
        (
            with_second(|pattern| pattern["pattern"]["x"] = json!(17)),
            "tracker.trackers.pattern",
        ),
        (
            with_second(|pattern| pattern["pattern"]["y"] = json!(-16)),
            "tracker.trackers.pattern",
        ),
        (
            with_second(|pattern| pattern["pattern"]["width"] = json!(1.5)),
            "tracker.trackers.pattern",
        ),
        (
            with_second(|pattern| pattern["pattern"]["height"] = json!(-0.1)),
            "tracker.trackers.pattern",
        ),
        (
            with_second(|pattern| pattern["search"]["width"] = json!(-0.1)),
            "tracker.trackers.search",
        ),
        (
            with_second(|pattern| pattern["search"]["height"] = json!(1.5)),
            "tracker.trackers.search",
        ),
        (
            with_second(|pattern| pattern["offset"]["x"] = json!(16.5)),
            "tracker.trackers.offset",
        ),
        (
            with_second(|pattern| pattern["offset"]["y"] = json!(-17)),
            "tracker.trackers.offset",
        ),
        // A place of a path beyond its range, and a frame beyond the largest.
        (
            with_second(|pattern| pattern["path"]["30"] = json!([0.5, 17])),
            "tracker.trackers.path",
        ),
        (
            with_second(|pattern| pattern["path"]["9007199254740992"] = json!([0.5, 0.5])),
            "tracker.trackers.path",
        ),
        (
            with_second(|pattern| pattern["path"]["-9007199254740992"] = json!([0.5, 0.5])),
            "tracker.trackers.path",
        ),
        (
            json!({ "pivot_tracker": MOST_TRACKERS }),
            "tracker.pivot_tracker",
        ),
        (json!({"pivot_point": {"x": 17}}), "tracker.pivot_point"),
        (json!({"pivot_point": {"y": -15.6}}), "tracker.pivot_point"),
        (
            json!({"reference": 9_007_199_254_740_992_i64}),
            "tracker.reference",
        ),
    ] {
        assert_eq!(
            refused(&project, set_params("n5", patch)),
            range(what),
            "{what}"
        );
    }
    // A number that is no number cannot be written: put in with a composition, whole, it is
    // refused.
    let mut broken = composition(&project, "c1").clone();
    if let NodeKind::Tracker(tracker) = &mut broken.nodes[4].kind {
        tracker.trackers.push(Pattern {
            path: [(3, [f64::NAN, 0.5])].into_iter().collect(),
            ..Pattern::default()
        });
    }
    assert_eq!(
        refused(&project, set("c1", Some(broken))),
        range("tracker.trackers.path")
    );
    // What is not a setting of the kind, or not written as one, is refused: a frame of a
    // path that is no whole number, a place that is not two numbers, and keys, which no
    // number of a Tracker takes.
    let keyed = json!({"keys": [{"frame": 0, "value": 0.5, "interp": "linear"}]});
    for patch in [
        json!({"operation": "stabilize"}),
        json!({"merge": "fg_under_bg"}),
        json!({"pivot": "tracker_center"}),
        json!({"position": 1}),
        json!({"pivot_tracker": -1}),
        json!({"pivot_tracker": 1.5}),
        json!({"pivot_point": 0.5}),
        json!({"pivot_point": {"x": keyed}}),
        json!({"reference": 1.5}),
        json!({ "reference": keyed }),
        json!({"trackers": {"0": {}}}),
        json!({"trackers": [{"name": "", "enabled": true}]}),
        with_second(|pattern| pattern["path"]["1.5"] = json!([0.5, 0.5])),
        with_second(|pattern| pattern["path"]["first"] = json!([0.5, 0.5])),
        with_second(|pattern| pattern["path"]["30"] = json!([0.5])),
        with_second(|pattern| pattern["path"] = json!([[0.5, 0.5]])),
        with_second(|pattern| pattern["pattern"]["x"] = json!({"keys": []})),
        with_second(|pattern| pattern["confidence"] = json!(1.0)),
        json!({"center": {"x": 0.5, "y": 0.5}}),
        json!({"adaptive_mode": "none"}),
    ] {
        let error = refused(&project, set_params("n5", patch.clone()));
        assert!(
            matches!(&error, OpError::Params { node, .. } if node == "n5"),
            "{patch}: {error:?}"
        );
    }
    // What is given is set, and set back by the inverse; the list of patterns whole.
    let (_, inverse) = applied(
        &mut project,
        set_params(
            "n5",
            json!({
                "operation": "match_move", "merge": "fg_over_bg", "rotation": false,
                "pivot": "manual", "pivot_point": {"x": 0.25}, "pivot_tracker": 1,
                "reference": 10, "trackers": two_patterns(),
            }),
        ),
    );
    assert_eq!(
        patch_of(&inverse),
        Some(json!({
            "operation": "none", "merge": "bg_only", "rotation": true,
            "pivot": "tracker_average", "pivot_point": {"x": 0.5}, "pivot_tracker": 0,
            "reference": 0, "trackers": [],
        }))
    );
    let tracker = tracker_of(&project);
    assert_eq!(tracker.operation, TrackerOperation::MatchMove);
    assert_eq!(tracker.merge, TrackerMerge::FgOverBg);
    assert!(tracker.position && !tracker.rotation && tracker.scaling);
    assert_eq!(tracker.pivot, TrackerPivot::Manual);
    assert_eq!(tracker.pivot_point, Place { x: 0.25, y: 0.5 });
    assert_eq!((tracker.pivot_tracker, tracker.reference), (1, 10));
    assert_eq!(tracker.trackers.len(), 2);
    let single = json!([written_pattern(
        "Only",
        [0.5, 0.5],
        json!({"0": [0.5, 0.5]})
    )]);
    let (done, inverse) = applied(
        &mut project,
        set_params("n5", json!({ "trackers": single })),
    );
    assert_eq!(patch_of(&done), Some(json!({ "trackers": single })));
    assert_eq!(
        patch_of(&inverse),
        Some(json!({ "trackers": two_patterns() }))
    );
    assert_eq!(tracker_of(&project).trackers[0].name, "Only");
    // The ends of each range are within it, and each way of each menu.
    for patch in [
        json!({"pivot_tracker": MOST_TRACKERS - 1, "pivot_point": {"x": -15.5, "y": 16.5}}),
        json!({"reference": -9_007_199_254_740_991_i64}),
        json!({"reference": 9_007_199_254_740_991_i64}),
        json!({"operation": "none", "merge": "bg_over_fg", "pivot": "selected_tracker"}),
        json!({"merge": "fg_only", "position": false, "scaling": false}),
        with_second(|pattern| {
            pattern["pattern"] = json!({"x": -15.5, "y": 16.5, "width": 0.0, "height": 1.0});
            pattern["search"] = json!({"width": 1.0, "height": 0.0});
            pattern["offset"] = json!({"x": -16.0, "y": 16.0});
            pattern["path"] = json!({"-9007199254740991": [-15.5, 16.5], "0": [16.5, -15.5]});
        }),
        json!({"trackers": []}),
    ] {
        applied(&mut project, set_params("n5", patch));
    }
    // No number of a Tracker takes keys.
    for path in [
        "n5.reference",
        "n5.pivot_tracker",
        "n5.pivot_point.x",
        "n5.operation",
        "n5.trackers",
        "n5.trackers.0.pattern.x",
        "n5.trackers.0.offset.y",
        "n5.trackers.0.path.10.0",
    ] {
        assert_eq!(
            refused(&project, set_keys("c1", path, Param::Fixed(0.5))),
            missing("setting", path),
            "{path}"
        );
        assert_eq!(
            refused(&project, splice(path, 0, 0, vec![key(0, 0.5)])),
            missing("setting", path),
            "{path}"
        );
    }
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "each input of a tracker, what it refuses, and what a layer draws of it"
)]
fn a_tracker_takes_a_picture_a_foreground_and_a_mask() {
    let mut project = tracked();
    // The Background, n4, is its foreground, and an Ellipse, n6, its mask.
    let (done, inverse) = applied(
        &mut project,
        connect(Link::new("n4", "n5", Input::Foreground)),
    );
    assert_eq!(done, connect(Link::new("n4", "n5", Input::Foreground)));
    assert_eq!(inverse, disconnect("n5", Input::Foreground));
    let (_, inverse) = applied(
        &mut project,
        add(
            Node::new("", Kind::Ellipse),
            vec![Link::new("", "n5", Input::Mask)],
        ),
    );
    assert_eq!(inverse, remove("n6", Some(vec![])));
    // It has no background and no matte; and no kind but a Merge and a Tracker has a
    // foreground.
    for input in [Input::Background, Input::GarbageMatte, Input::SolidMatte] {
        assert_eq!(
            refused(&project, connect(Link::new(MEDIA_IN, "n5", input))),
            OpError::NoInput {
                node: "n5".into(),
                input
            }
        );
    }
    for node in [MEDIA_OUT, "n4", "n6"] {
        assert_eq!(
            refused(
                &project,
                connect(Link::new(MEDIA_IN, node, Input::Foreground))
            ),
            OpError::NoInput {
                node: node.into(),
                input: Input::Foreground
            },
            "{node}"
        );
    }
    // A loop through its foreground or its mask is a loop.
    for input in [Input::Input, Input::Foreground, Input::Mask] {
        assert!(
            matches!(
                refused(&project, connect(Link::new("n3", "n5", input))),
                OpError::Loop { .. }
            ),
            "{input:?}"
        );
    }
    assert_eq!(
        refused(&project, connect(Link::new("n5", "n5", Input::Foreground))),
        OpError::Loop { node: "n5".into() }
    );
    // Its links are kept by the names of its inputs.
    let into: Vec<&str> = composition(&project, "c1")
        .links
        .iter()
        .filter(|link| link.to == "n5")
        .map(|link| link.input.name())
        .collect();
    assert_eq!(into, ["foreground", "input", "mask"]);
    // The layer draws it after its picture, its foreground and its mask, which it names in
    // the order of its inputs.
    let at = |id: &str, mask: Option<usize>| (id.to_owned(), mask);
    assert_eq!(
        drawn(&project),
        [
            at("n4", None),
            at(MEDIA_IN, None),
            at("n6", None),
            at("n5", Some(2)),
            at("n3", None)
        ]
    );
    let fused = layer_at(&project, 20).fusion.unwrap();
    assert_eq!(fused.nodes[3].kind.reads(), [1, 0, 2]);
    assert_eq!(fused.nodes[3].kind.first_input(), Some(1));
    assert!(matches!(
        fused.nodes[3].kind,
        FusedKind::Tracker {
            input: Some(1),
            foreground: Some(0),
            mask: Some(2),
            ..
        }
    ));
    // Passed through, it hands on its picture, and neither its foreground nor its mask: the
    // Merge lays the picture of the clip, as it did before the Tracker.
    applied(&mut project, passed_through("n5"));
    assert_eq!(drawn(&project), drawn(&merged()));
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "what a tracker draws at a frame, each way and with each control"
)]
fn a_tracker_of_a_layer_is_what_it_draws_at_the_frame() {
    let mut project = tracked();
    let left = || {
        written_pattern(
            "Left",
            [0.25, 0.5],
            json!({"10": [0.25, 0.5], "20": [0.3125, 0.625]}),
        )
    };
    applied(
        &mut project,
        set_params(
            "n5",
            json!({"operation": "match_move", "reference": 10, "trackers": [left()]}),
        ),
    );
    assert!(composition(&project, "c1").is_animated());
    // c1 shows m1 from its start at the rate of the timeline: frame 20 shows picture 20.
    // Held still, the picture is put as far to the left and down as the pattern has gone to
    // the right and up since the reference frame.
    let held = tracked_at(&project, 20);
    assert_eq!(
        (held.operation, held.merge),
        (TrackerOperation::MatchMove, TrackerMerge::BgOnly)
    );
    assert!(near(held.center, [0.4375, 0.375]), "{held:?}");
    assert_eq!((held.size, held.angle), (1.0, 0.0));
    // Between two frames of the path, on the line between them.
    assert!(near(tracked_at(&project, 15).center, [0.46875, 0.4375]));
    // At the reference frame, and before the path, where the pattern is where it was first,
    // the picture lies as it is; after the last frame of the path, as at that one.
    for frame in [10, 5, 0] {
        assert_eq!(tracked_at(&project, frame).center, [0.5, 0.5], "{frame}");
    }
    assert_eq!(tracked_at(&project, 30), held);
    // With each of the other three the foreground follows the pattern.
    for (merge, written) in [
        (TrackerMerge::FgOnly, "fg_only"),
        (TrackerMerge::FgOverBg, "fg_over_bg"),
        (TrackerMerge::BgOverFg, "bg_over_fg"),
    ] {
        applied(&mut project, set_params("n5", json!({ "merge": written })));
        let follows = tracked_at(&project, 20);
        assert_eq!(follows.merge, merge);
        assert!(near(follows.center, [0.5625, 0.625]), "{follows:?}");
        assert_eq!((follows.size, follows.angle), (1.0, 0.0));
    }
    // The reference frame is where nothing moves: with the last frame of the path for it,
    // the first is as far the other way.
    applied(
        &mut project,
        set_params("n5", json!({"merge": "bg_only", "reference": 20})),
    );
    assert_eq!(tracked_at(&project, 20).center, [0.5, 0.5]);
    assert_eq!(tracked_at(&project, 40).center, [0.5, 0.5]);
    assert!(near(tracked_at(&project, 10).center, [0.5625, 0.625]));
    // Without Position the shift of one pattern is no motion.
    applied(
        &mut project,
        set_params("n5", json!({"reference": 10, "position": false})),
    );
    assert_eq!(tracked_at(&project, 20).center, [0.5, 0.5]);
    applied(&mut project, set_params("n5", json!({"position": true})));
    // With an Operation of None, with its pattern off, and with no pattern, the picture
    // lies as it is; and one with no path is the same at every frame.
    let mut idle = project.clone();
    applied(&mut idle, set_params("n5", json!({"operation": "none"})));
    let still = tracked_at(&idle, 20);
    assert_eq!(still.operation, TrackerOperation::None);
    assert_eq!(
        (still.center, still.size, still.angle),
        ([0.5, 0.5], 1.0, 0.0)
    );
    let mut off = project.clone();
    let mut pattern = left();
    pattern["enabled"] = json!(false);
    applied(&mut off, set_params("n5", json!({ "trackers": [pattern] })));
    assert_eq!(tracked_at(&off, 20).center, [0.5, 0.5]);
    let mut none = project.clone();
    applied(&mut none, set_params("n5", json!({"trackers": []})));
    assert_eq!(tracked_at(&none, 20).center, [0.5, 0.5]);
    assert!(!composition(&none, "c1").is_animated());
    // Two patterns: a shift, a turn and a size, as the mathematics has them for pictures of
    // the size of the composition.
    let ends = [[0.3, 0.6], [0.7, 0.3]];
    let patterns = json!([
        written_pattern(
            "Left",
            [0.25, 0.5],
            json!({"10": [0.25, 0.5], "20": ends[0]})
        ),
        written_pattern(
            "Right",
            [0.75, 0.5],
            json!({"10": [0.75, 0.5], "20": ends[1]})
        ),
    ]);
    applied(
        &mut project,
        set_params("n5", json!({ "trackers": patterns })),
    );
    let size = size_of(clip(&project, "c1"), &project, &project.timelines[0]);
    let extent = (f64::from(size.0), f64::from(size.1));
    let pixels = |at: [f64; 2]| matchmove::from_fusion(at, extent);
    let paths = [
        [pixels([0.25, 0.5]), pixels(ends[0])],
        [pixels([0.75, 0.5]), pixels(ends[1])],
    ];
    let trackers = [
        matchmove::Tracker::new(&paths[0]),
        matchmove::Tracker::new(&paths[1]),
    ];
    let settings = matchmove::Settings {
        operation: matchmove::Operation::MatchMove,
        ..matchmove::Settings::default()
    };
    let solved = matchmove::solve(&trackers, &settings)[1].expect("the patterns have moved");
    let middle = [extent.0 / 2.0, extent.1 / 2.0];
    let held = tracked_at(&project, 20);
    assert!(
        (held.size - 1.0 / solved.motion.zoom).abs() < 1e-9,
        "{held:?}"
    );
    assert!(
        (held.angle - solved.motion.rotate.to_degrees()).abs() < 1e-9,
        "{held:?}"
    );
    let put = matchmove::to_fusion(solved.steady().apply(middle), extent).unwrap();
    assert!(near(held.center, put), "{held:?}");
    assert!((held.size - 1.0).abs() > 0.01 && held.angle.abs() > 1.0);
    applied(&mut project, set_params("n5", json!({"merge": "fg_only"})));
    let follows = tracked_at(&project, 20);
    assert!((follows.size - solved.motion.zoom).abs() < 1e-9);
    assert!((follows.angle + solved.motion.rotate.to_degrees()).abs() < 1e-9);
    let put = matchmove::to_fusion(solved.unsteady().apply(middle), extent).unwrap();
    assert!(near(follows.center, put), "{follows:?}");
}

/// `tracked` with its two patterns on their paths, the second with an offset, frame 10 its
/// reference; and an Ellipse, n6, as the effect mask of the Merge n3: `next_id` is 7.
fn followed() -> Project {
    let mut project = tracked();
    let patterns = with_second(|pattern| pattern["offset"] = json!({"x": 0.125, "y": -0.25}));
    applied(&mut project, set_params("n5", patterns));
    applied(&mut project, set_params("n5", json!({"reference": 10})));
    applied(
        &mut project,
        add(
            Node::new("", Kind::Ellipse),
            vec![Link::new("", "n3", Input::Mask)],
        ),
    );
    assert_eq!(project.next_id, 7);
    project
}

/// `set_fusion_node` that makes the centre of node `id` of c1 follow `follow`, as it is
/// written, or changes what it follows by it; `null` lets go.
fn following(id: &str, follow: serde_json::Value) -> Op {
    let mut patch = json!({"center": {}});
    patch["center"]["follow"] = follow;
    set_params(id, patch)
}

/// The centre of the Merge or of the Ellipse `id` as the layer of c1 draws it at `frame`.
fn centre_at(project: &Project, id: &str, frame: i64) -> [f64; 2] {
    let fused = layer_at(project, frame)
        .fusion
        .expect("the layer has its composition");
    let node = fused
        .nodes
        .iter()
        .find(|node| node.id == id)
        .expect("the node is drawn");
    match &node.kind {
        FusedKind::Merge { settings, .. } => settings.center,
        FusedKind::Ellipse { shape, .. } => shape.center,
        other => panic!("{other:?}"),
    }
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "a centre that follows each place, what is refused of it, and letting go"
)]
fn a_centre_follows_what_a_tracker_publishes_and_lets_go_of_it() {
    let mut project = followed();
    let before = serde_json::to_string(composition(&project, "c1")).unwrap();
    assert!(!before.contains("follow"));
    // The Ellipse follows the second pattern, with its offset: it is set with the centre,
    // and what the centre had is set back by the inverse.
    let second = json!({"node": "n5", "output": "tracker", "tracker": 1});
    let (done, inverse) = applied(&mut project, following("n6", second.clone()));
    assert_eq!(done, following("n6", second.clone()));
    assert_eq!(inverse, following("n6", json!(null)));
    let written = serde_json::to_value(composition(&project, "c1")).unwrap();
    assert_eq!(
        written["nodes"][5]["params"]["center"],
        json!({"x": 0.5, "y": 0.5, "follow": second})
    );
    let NodeKind::Ellipse(ellipse) = &composition(&project, "c1").node("n6").unwrap().kind else {
        panic!("n6 is an ellipse")
    };
    assert_eq!(ellipse.center.follow, Some(Follow::tracker("n5", 1)));
    // c1 shows m1 from its start at the rate of the timeline: frame 20 shows picture 20. The
    // centre is where the pattern is, and its offset, right and up: on its path, between
    // two frames of it, and before it.
    assert_eq!(centre_at(&project, "n6", 20), [0.9375, 0.375]);
    assert_eq!(centre_at(&project, "n6", 15), [0.90625, 0.3125]);
    assert_eq!(centre_at(&project, "n6", 0), [0.875, 0.25]);
    let size = size_of(clip(&project, "c1"), &project, &project.timelines[0]);
    assert_eq!(
        tracker_of(&project).published(FollowOutput::Tracker, 1, 20.0, size),
        Some([0.9375, 0.375])
    );
    // The Merge follows the Unsteady Position. A pattern need not be named for it: it is
    // then written as the first.
    let mut short = project.clone();
    let unnamed = json!({"node": "n5", "output": "unsteady_position"});
    apply(&mut short, following("n3", unnamed)).unwrap();
    let unsteady = json!({"node": "n5", "output": "unsteady_position", "tracker": 0});
    let (done, _) = applied(&mut project, following("n3", unsteady.clone()));
    assert_eq!(done, following("n3", unsteady));
    assert_eq!(short, project);
    let wanted = tracker_of(&project)
        .published(FollowOutput::UnsteadyPosition, 0, 20.0, size)
        .unwrap();
    assert_eq!(centre_at(&project, "n3", 20), wanted);
    assert!(near(wanted, [0.5625, 0.625]), "{wanted:?}");
    let follow = Follow::of("n5", FollowOutput::UnsteadyPosition);
    assert_eq!(
        composition(&project, "c1").published(&follow, 20.0, size),
        Some(wanted)
    );
    assert_eq!(centre_at(&project, "n3", 10), [0.5, 0.5]);
    // What is followed is changed by a patch, which leaves the node as it is: the Steady
    // Position is as far from the middle the other way.
    let (_, inverse) = applied(
        &mut project,
        following("n3", json!({"output": "steady_position"})),
    );
    assert_eq!(
        inverse,
        following("n3", json!({"output": "unsteady_position"}))
    );
    let steady = centre_at(&project, "n3", 20);
    assert!(near(steady, [0.4375, 0.375]), "{steady:?}");
    // The own numbers of a centre that follows are kept, and not read; where nothing is
    // published they are the centre: a pattern that the Tracker has not, and a Tracker that
    // is passed through.
    applied(
        &mut project,
        set_params("n6", json!({"center": {"x": 0.25, "y": 0.75}})),
    );
    assert_eq!(centre_at(&project, "n6", 20), [0.9375, 0.375]);
    applied(&mut project, following("n6", json!({"tracker": 5})));
    assert_eq!(centre_at(&project, "n6", 20), [0.25, 0.75]);
    applied(&mut project, following("n6", json!({"tracker": 1})));
    applied(&mut project, passed_through("n5"));
    assert_eq!(centre_at(&project, "n6", 20), [0.25, 0.75]);
    assert_eq!(centre_at(&project, "n3", 20), [0.5, 0.5]);
    let mut shown = set_node("n5");
    if let Op::SetFusionNode { pass_through, .. } = &mut shown {
        *pass_through = Some(false);
    }
    applied(&mut project, shown);
    assert_eq!(centre_at(&project, "n6", 20), [0.9375, 0.375]);
    // The node that is followed is a Tracker of the composition, and the pattern one that a
    // Tracker may have.
    assert_eq!(
        refused(&project, following("n6", json!({"node": "n404"}))),
        missing("node", "n404")
    );
    let wrong = OpError::WrongNode {
        node: "n3".into(),
        kind: "merge",
    };
    assert_eq!(
        refused(&project, following("n6", json!({"node": "n3"}))),
        wrong
    );
    assert_eq!(serde_json::to_value(wrong).unwrap()["reason"], "wrong_node");
    assert_eq!(
        refused(
            &project,
            following("n6", json!({ "tracker": MOST_TRACKERS }))
        ),
        range("center.follow.tracker")
    );
    for patch in [
        json!({"output": "steady_angle"}),
        json!({"output": 1}),
        json!({"tracker": -1}),
        json!({"input": "mask"}),
        json!("n5"),
    ] {
        let error = refused(&project, following("n6", patch.clone()));
        assert!(
            matches!(&error, OpError::Params { node, .. } if node == "n6"),
            "{patch}: {error:?}"
        );
    }
    // A kind that has no centre follows nothing; and a node that is added, or a composition
    // that is put in whole, is held to the same.
    let error = refused(&project, following("n4", second.clone()));
    assert!(matches!(&error, OpError::Params { node, .. } if node == "n4"));
    let mut stray = Node::new("", Kind::Rectangle);
    if let NodeKind::Rectangle(rectangle) = &mut stray.kind {
        rectangle.center.follow = Some(Follow::tracker("n4", 0));
    }
    assert_eq!(
        refused(&project, add(stray, vec![])),
        OpError::WrongNode {
            node: "n4".into(),
            kind: "background"
        }
    );
    let mut alone = composition(&project, "c1").clone();
    alone.nodes.retain(|node| node.id != "n5");
    alone.links.retain(|link| !link.touches("n5"));
    assert_eq!(
        refused(&project, set("c1", Some(alone))),
        missing("node", "n5")
    );
    // A Tracker that is followed is not taken away.
    let used = OpError::InUse {
        what: "node",
        id: "n5".into(),
        by: "n3".into(),
    };
    assert_eq!(refused(&project, remove("n5", None)), used);
    assert_eq!(serde_json::to_value(used).unwrap()["reason"], "in_use");
    // Let go of, by a `follow` of `null`, the centre is its own numbers again, and is
    // written as it was.
    let (done, inverse) = applied(&mut project, following("n3", json!(null)));
    assert_eq!(done, following("n3", json!(null)));
    assert_eq!(
        inverse,
        following(
            "n3",
            json!({"node": "n5", "output": "steady_position", "tracker": 0})
        )
    );
    assert_eq!(centre_at(&project, "n3", 20), [0.5, 0.5]);
    assert_eq!(
        refused(&project, remove("n5", None)),
        OpError::InUse {
            what: "node",
            id: "n5".into(),
            by: "n6".into(),
        }
    );
    applied(&mut project, following("n6", json!(null)));
    assert_eq!(centre_at(&project, "n6", 20), [0.25, 0.75]);
    applied(
        &mut project,
        set_params("n6", json!({"center": {"x": 0.5, "y": 0.5}})),
    );
    assert_eq!(
        serde_json::to_string(composition(&project, "c1")).unwrap(),
        before
    );
    // And the Tracker may go.
    let bridge = vec![Link::new(MEDIA_IN, "n3", Input::Foreground)];
    applied(&mut project, remove("n5", Some(bridge)));
}

/// A composition that holds the picture of its clip still: `in` through the Tracker
/// `track`, whose one pattern has `path` and whose reference is `reference`, into `out`.
fn holding(path: &[(i64, [f64; 2])], reference: i64) -> Composition {
    let mut track = Node::new("track", Kind::Tracker);
    track.kind = NodeKind::Tracker(Tracker {
        operation: TrackerOperation::MatchMove,
        reference,
        trackers: vec![Pattern {
            path: path.iter().copied().collect(),
            ..Pattern::default()
        }],
        ..Tracker::default()
    });
    let mut composition = Composition {
        nodes: vec![
            Node::new("in", Kind::MediaIn),
            track,
            Node::new("out", Kind::MediaOut),
        ],
        links: vec![
            Link::new("in", "track", Input::Input),
            Link::new("track", "out", Input::Input),
        ],
    };
    composition.sort_links();
    composition
}

/// The frames of the path of the pattern of the Tracker `track` of clip `id`, and its
/// reference.
fn path_of(project: &Project, id: &str) -> (Vec<i64>, i64) {
    let node = composition(project, id)
        .node("track")
        .expect("the composition has the tracker");
    let NodeKind::Tracker(tracker) = &node.kind else {
        panic!("the node is a tracker")
    };
    (
        tracker.trackers[0].path.keys().copied().collect(),
        tracker.reference,
    )
}

#[test]
fn the_paths_of_a_tracker_are_moved_with_the_keys_of_its_clip() {
    // A colour counts its own frames: where its head is trimmed, the frames of the path and
    // the reference are moved as its keys are, so that each frame of the timeline shows what
    // it showed.
    let frames = [210, 215, 220, 230, 245];
    let mut project = with_a_colour_that_moves();
    let colour = project.timelines[0].tracks[0]
        .clips
        .iter_mut()
        .find(|clip| clip.id == "k1")
        .expect("the colour is there");
    colour.fusion = Some(holding(&[(0, [0.25, 0.5]), (40, [0.75, 0.25])], 10));
    project.validate().unwrap();
    let seen = |project: &Project| frames.map(|frame| tracked_at(project, frame));
    let before = seen(&project);
    assert_ne!(before[0], before[4]);
    edited(
        &mut project,
        edit(json!({"edit": "trim", "timeline": "t1", "clip": "k1", "edge": "in", "delta": 10})),
    );
    assert_eq!(path_of(&project, "k1"), (vec![-10, 30], 0));
    assert_eq!(seen(&project), before);
    // Cut in two by the blade, both halves keep the Tracker, the second its path on its own
    // frames.
    edited(
        &mut project,
        edit(json!({"edit": "blade", "timeline": "t1", "at": 225, "tracks": ["V1"]})),
    );
    let second = project.timelines[0].tracks[0]
        .clips
        .iter()
        .find(|clip| clip.start == 225)
        .expect("the second half is there")
        .id
        .clone();
    assert_eq!(path_of(&project, "k1"), (vec![-10, 30], 0));
    assert_eq!(path_of(&project, &second), (vec![-25, 15], -15));
    assert_eq!(seen(&project), before);
    // Of media, a path is on its pictures: a trim of the head leaves it where it was.
    let mut media = Project::sample();
    applied(
        &mut media,
        set(
            "c1",
            Some(holding(&[(20, [0.25, 0.5]), (60, [0.75, 0.25])], 30)),
        ),
    );
    let before = [30, 40, 70].map(|frame| tracked_at(&media, frame));
    edited(
        &mut media,
        edit(
            json!({"edit": "trim", "timeline": "t1", "clip": "c1", "edge": "in", "delta": 10,
            "linked": false}),
        ),
    );
    assert_eq!(path_of(&media, "c1"), (vec![20, 60], 30));
    assert_eq!([30, 40, 70].map(|frame| tracked_at(&media, frame)), before);
}

#[test]
fn a_change_of_a_tracker_that_changes_nothing_makes_no_step() {
    let mut project = followed();
    let second = json!({"node": "n5", "output": "tracker", "tracker": 1});
    applied(&mut project, following("n6", second.clone()));
    let mut history = History::new(project);
    let revision = history.revision();
    for op in [
        set_params(
            "n5",
            json!({"operation": "none", "merge": "bg_only", "reference": 10}),
        ),
        set_params(
            "n5",
            with_second(|pattern| pattern["offset"] = json!({"x": 0.125, "y": -0.25})),
        ),
        following("n6", second),
        following("n6", json!({"output": "tracker"})),
        following("n3", json!(null)),
    ] {
        let nothing = history.change(vec![Change::Op(op.clone())], "desktop", |change, _| change);
        assert_eq!(nothing.err(), Some(TransactionError::Empty), "{op:?}");
    }
    assert_eq!(history.revision(), revision);
    assert!(!history.can_undo());
    // One that changes something is a step, and is undone: a place of a path, and what a
    // centre follows.
    let before = history.project().clone();
    for op in [
        set_params(
            "n5",
            with_second(|pattern| pattern["path"]["20"] = json!([0.875, 0.625])),
        ),
        following("n6", json!({"tracker": 0})),
    ] {
        history
            .change(vec![Change::Op(op)], "desktop", |change, _| change)
            .unwrap();
        assert_ne!(history.project(), &before);
        history.undo("desktop").unwrap().unwrap();
        assert_eq!(history.project(), &before);
    }
}

// The Planar Tracker.

/// The sample of `merged` with a Planar Tracker, n5, of the picture of c1, which lays the
/// Background n4 on its plane, and whose picture the Merge n3 lays over n4 in the place of the
/// picture of the clip: `next_id` is 6.
fn planar_tracked() -> Project {
    let mut project = merged();
    applied(
        &mut project,
        add(
            Node::new("", Kind::PlanarTracker),
            vec![
                Link::new(MEDIA_IN, "", Input::Input),
                Link::new("n4", "", Input::Foreground),
                Link::new("", "n3", Input::Foreground),
            ],
        ),
    );
    assert_eq!(project.next_id, 6);
    project
}

/// The Planar Tracker n5 of c1.
fn planar_of(project: &Project) -> &PlanarTracker {
    match &composition(project, "c1").node("n5").unwrap().kind {
        NodeKind::PlanarTracker(planar) => planar,
        other => panic!("{other:?}"),
    }
}

/// What the Planar Tracker that the one layer of the timeline draws at `frame` draws there.
fn planar_at(project: &Project, frame: i64) -> PlanarTracked {
    let fused = layer_at(project, frame)
        .fusion
        .expect("the layer has its composition");
    fused
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            FusedKind::PlanarTracker { settings, .. } => Some(*settings),
            _ => None,
        })
        .expect("the layer draws its planar tracker")
}

/// A shift of `x` across and `y` up, in shares of the picture, as a path writes it.
fn shift_of(x: f64, y: f64) -> [f64; 8] {
    [1.0, 0.0, x, 0.0, 1.0, y, 0.0, 0.0]
}

/// A place as it is written.
fn place_of(x: f64, y: f64) -> serde_json::Value {
    json!({"x": x, "y": y})
}

/// Where `matrix`, of shares from the top left corner of the picture, puts `[x, y]`.
fn put_by(matrix: &[[f64; 3]; 3], [x, y]: [f64; 2]) -> [f64; 2] {
    let w = matrix[2][0] * x + matrix[2][1] * y + matrix[2][2];
    [
        (matrix[0][0] * x + matrix[0][1] * y + matrix[0][2]) / w,
        (matrix[1][0] * x + matrix[1][1] * y + matrix[1][2]) / w,
    ]
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "a planar tracker as it is written, read, written by another program and taken away"
)]
fn a_planar_tracker_is_added_with_every_setting_written_and_is_read_back() {
    let mut project = planar_tracked();
    let written = serde_json::to_value(composition(&project, "c1")).unwrap();
    assert_eq!(
        written["nodes"][4],
        json!({
            "id": "n5", "kind": "planar_tracker", "label": "", "pass_through": false,
            "pos": null,
            "params": {
                "operation": "none", "motion": "perspective", "reference": 0,
                "region": [place_of(0.25, 0.25), place_of(0.75, 0.25), place_of(0.75, 0.75),
                           place_of(0.25, 0.75)],
                "corners": {"tl": place_of(0.25, 0.75), "tr": place_of(0.75, 0.75),
                            "br": place_of(0.75, 0.25), "bl": place_of(0.25, 0.25)},
                "path": {},
            },
        })
    );
    // The picture of the clip is its first input, the Background its foreground, and it is
    // the foreground of the Merge.
    assert_eq!(
        written["links"],
        json!([
            {"from": "n3", "to": "MediaOut1", "input": "input"},
            {"from": "n4", "to": "n3", "input": "background"},
            {"from": "n5", "to": "n3", "input": "foreground"},
            {"from": "n4", "to": "n5", "input": "foreground"},
            {"from": "MediaIn1", "to": "n5", "input": "input"},
        ])
    );
    let text = serde_json::to_string(&project).unwrap();
    let read: Project = serde_json::from_str(&text).unwrap();
    assert_eq!(read, project);
    assert_eq!(serde_json::to_string(&read).unwrap(), text);
    // As another program writes one, with every setting, a region of three places and a path;
    // the Planar Tracker before it its mask.
    let op: Op = serde_json::from_value(json!({
        "op": "add_fusion_node", "timeline": "t1", "clip": "c1",
        "node": {"id": "", "kind": "planar_tracker", "label": "Screen", "params": {
            "operation": "corner_pin", "motion": "translation_rotation", "reference": 10,
            "region": [place_of(0.2, 0.2), place_of(0.8, 0.3), place_of(0.5, 0.9)],
            "corners": {"tl": place_of(0.3, 0.7), "tr": place_of(0.7, 0.7),
                        "br": place_of(0.7, 0.3), "bl": place_of(0.3, 0.3)},
            "path": {"10": STILL, "20": shift_of(0.125, 0.25), "30": null},
        }},
        "links": [
            {"from": "MediaIn1", "to": "", "input": "input"},
            {"from": "n4", "to": "", "input": "foreground"},
            {"from": "n5", "to": "", "input": "mask"},
        ],
    }))
    .unwrap();
    let (done, inverse) = applied(&mut project, op);
    assert_eq!(inverse, remove("n6", Some(vec![])));
    let Op::AddFusionNode { node, .. } = done else {
        panic!("a node was added")
    };
    assert_eq!(node.id, "n6");
    let NodeKind::PlanarTracker(screen) = &node.kind else {
        panic!("n6 is a planar tracker")
    };
    assert_eq!(
        (screen.operation, screen.motion, screen.reference),
        (
            PlanarOperation::CornerPin,
            PlanarMotion::TranslationRotation,
            10
        )
    );
    assert_eq!(screen.region.len(), 3);
    assert_eq!(screen.corners.tl, Place { x: 0.3, y: 0.7 });
    // A frame written null is not there.
    assert_eq!(screen.path.keys().copied().collect::<Vec<_>>(), [10, 20]);
    assert_eq!(screen.path[&20], shift_of(0.125, 0.25));
    assert_eq!(
        serde_json::to_string(&serde_json::to_value(screen).unwrap()["path"]).unwrap(),
        r#"{"10":[1.0,0.0,0.0,0.0,1.0,0.0,0.0,0.0],"20":[1.0,0.0,0.125,0.0,1.0,0.25,0.0,0.0]}"#
    );
    let text = serde_json::to_string(&project).unwrap();
    let read: Project = serde_json::from_str(&text).unwrap();
    assert_eq!(read, project);
    let value = serde_json::to_value(&project).unwrap();
    assert_eq!(Project::from_value(value).unwrap(), project);
    // One whose settings are not all there, or that has one that a Planar Tracker has not,
    // cannot be read.
    let whole = serde_json::to_value(&node).unwrap()["params"].clone();
    for setting in [
        "operation",
        "motion",
        "reference",
        "region",
        "corners",
        "path",
    ] {
        let mut params = whole.clone();
        params.as_object_mut().unwrap().remove(setting);
        let node = json!({"id": "", "kind": "planar_tracker", "label": "", "params": params});
        assert!(serde_json::from_value::<Node>(node).is_err(), "{setting}");
    }
    let mut more = whole.clone();
    more["trackers"] = json!([]);
    let node = json!({"id": "", "kind": "planar_tracker", "label": "", "params": more});
    assert!(serde_json::from_value::<Node>(node).is_err());
    // Taken away again, the first with the link that takes its place, they leave the
    // composition as it was before them.
    applied(&mut project, remove("n6", None));
    let bridge = vec![Link::new(MEDIA_IN, "n3", Input::Foreground)];
    applied(&mut project, remove("n5", Some(bridge)));
    let before = merged();
    assert_eq!(composition(&project, "c1"), composition(&before, "c1"));
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "a refusal for each number of a planar tracker, and the ends of their ranges"
)]
fn the_settings_of_a_planar_tracker_are_those_of_its_kind_within_their_ranges() {
    let mut project = planar_tracked();
    let many: Vec<serde_json::Value> = (0..=MOST_REGION_POINTS)
        .map(|_| place_of(0.5, 0.5))
        .collect();
    for (patch, what) in [
        (
            json!({"region": [place_of(0.1, 0.1), place_of(0.9, 0.9)]}),
            "planar_tracker.region",
        ),
        (json!({ "region": many }), "planar_tracker.region"),
        (
            json!({"region": [place_of(0.1, 0.1), place_of(17.0, 0.9), place_of(0.5, 0.5)]}),
            "planar_tracker.region",
        ),
        (
            json!({"corners": {"tl": {"x": -16}}}),
            "planar_tracker.corners",
        ),
        (
            json!({"corners": {"br": {"y": 16.6}}}),
            "planar_tracker.corners",
        ),
        (
            json!({"path": {"9007199254740992": STILL}}),
            "planar_tracker.path",
        ),
        (
            json!({"path": {"-9007199254740992": STILL}}),
            "planar_tracker.path",
        ),
        (
            json!({"reference": 9_007_199_254_740_992_i64}),
            "planar_tracker.reference",
        ),
    ] {
        assert_eq!(
            refused(&project, set_params("n5", patch)),
            range(what),
            "{what}"
        );
    }
    // A number that is no number cannot be written: put in with a composition, whole, it is
    // refused.
    let mut broken = composition(&project, "c1").clone();
    if let NodeKind::PlanarTracker(planar) = &mut broken.nodes[4].kind {
        planar
            .path
            .insert(3, [f64::NAN, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]);
    }
    assert_eq!(
        refused(&project, set("c1", Some(broken))),
        range("planar_tracker.path")
    );
    // What is not a setting of the kind, or not written as one, is refused: an operation and
    // a motion that it has not, a frame that is no whole number, a homography that is not
    // eight numbers, and keys, which no number of a Planar Tracker takes.
    let keyed = json!({"keys": [{"frame": 0, "value": 0.5, "interp": "linear"}]});
    for patch in [
        json!({"operation": "track"}),
        json!({"operation": "stabilize"}),
        json!({"motion": "affine"}),
        json!({"reference": 1.5}),
        json!({ "reference": keyed }),
        json!({"region": {"0": {"x": 0.5}}}),
        json!({"region": [{"x": 0.5}, place_of(0.5, 0.5), place_of(0.5, 0.5)]}),
        json!({"corners": {"tl": {"x": keyed}}}),
        json!({"corners": {"middle": place_of(0.5, 0.5)}}),
        json!({"path": {"1.5": STILL}}),
        json!({"path": {"first": STILL}}),
        json!({"path": {"3": [1.0, 0.0, 0.0]}}),
        json!({"path": [STILL]}),
        json!({"trackers": []}),
    ] {
        let error = refused(&project, set_params("n5", patch.clone()));
        assert!(
            matches!(&error, OpError::Params { node, .. } if node == "n5"),
            "{patch}: {error:?}"
        );
    }
    // What is given is set, and set back by the inverse; the region whole.
    let triangle = json!([place_of(0.1, 0.2), place_of(0.9, 0.2), place_of(0.5, 0.8)]);
    let (_, inverse) = applied(
        &mut project,
        set_params(
            "n5",
            json!({
                "operation": "steady", "motion": "translation", "reference": 12,
                "region": triangle, "corners": {"tl": {"x": 0.1}},
            }),
        ),
    );
    assert_eq!(
        patch_of(&inverse),
        Some(json!({
            "operation": "none", "motion": "perspective", "reference": 0,
            "region": [place_of(0.25, 0.25), place_of(0.75, 0.25), place_of(0.75, 0.75),
                       place_of(0.25, 0.75)],
            "corners": {"tl": {"x": 0.25}},
        }))
    );
    let planar = planar_of(&project);
    assert_eq!(
        (planar.operation, planar.motion, planar.reference),
        (PlanarOperation::Steady, PlanarMotion::Translation, 12)
    );
    assert_eq!(planar.region.len(), 3);
    assert_eq!(planar.corners.tl, Place { x: 0.1, y: 0.75 });
    // The ends of each range are within it, and each way of each menu.
    let edge = json!([
        place_of(-15.5, 16.5),
        place_of(16.5, -15.5),
        place_of(0.5, 0.5)
    ]);
    let far = [1e12; 8];
    for patch in [
        json!({ "region": edge }),
        json!({ "region": vec![place_of(0.5, 0.5); MOST_REGION_POINTS] }),
        json!({"corners": {"tl": place_of(-15.5, 16.5), "br": place_of(16.5, -15.5)}}),
        json!({"reference": -9_007_199_254_740_991_i64}),
        json!({"path": {"9007199254740991": STILL, "-9007199254740991": far}}),
        json!({"operation": "corner_pin", "motion": "translation_rotation_scale"}),
        json!({"operation": "none", "motion": "translation_rotation"}),
    ] {
        applied(&mut project, set_params("n5", patch));
    }
    // No number of a Planar Tracker takes keys.
    for path in [
        "n5.reference",
        "n5.operation",
        "n5.region",
        "n5.region.0.x",
        "n5.corners.tl.x",
        "n5.path",
        "n5.path.9007199254740991.0",
    ] {
        assert_eq!(
            refused(&project, set_keys("c1", path, Param::Fixed(0.5))),
            missing("setting", path),
            "{path}"
        );
        assert_eq!(
            refused(&project, splice(path, 0, 0, vec![key(0, 0.5)])),
            missing("setting", path),
            "{path}"
        );
    }
}

#[test]
fn a_patch_of_the_path_of_a_planar_tracker_sets_its_frames_and_its_inverse_takes_them_away() {
    let mut project = planar_tracked();
    // Frames put into a path that has none: the inverse gives each null, which takes it away.
    let first = json!({"path": {"10": STILL, "11": shift_of(0.01, 0.0)}});
    let (done, inverse) = applied(&mut project, set_params("n5", first.clone()));
    assert_eq!(patch_of(&done), Some(first));
    assert_eq!(
        patch_of(&inverse),
        Some(json!({"path": {"10": null, "11": null}}))
    );
    // More frames, one of them over one that was there: the others are kept, and the inverse
    // gives that one back as it was.
    let second = json!({"path": {"11": shift_of(0.02, 0.0), "12": shift_of(0.03, 0.0)}});
    let (_, inverse) = applied(&mut project, set_params("n5", second));
    assert_eq!(
        patch_of(&inverse),
        Some(json!({"path": {"11": shift_of(0.01, 0.0), "12": null}}))
    );
    assert_eq!(
        planar_of(&project).path.keys().copied().collect::<Vec<_>>(),
        [10, 11, 12]
    );
    assert_eq!(planar_of(&project).path[&11], shift_of(0.02, 0.0));
    // A frame given null by a client is taken away, and its inverse puts it back.
    let (_, inverse) = applied(
        &mut project,
        set_params("n5", json!({"path": {"10": null}})),
    );
    assert_eq!(patch_of(&inverse), Some(json!({"path": {"10": STILL}})));
    assert_eq!(
        planar_of(&project).path.keys().copied().collect::<Vec<_>>(),
        [11, 12]
    );
    // As two changes of one gesture, they are undone in one step, last first.
    let mut history = History::new(planar_tracked());
    let before = history.project().clone();
    for patch in [
        json!({"path": {"10": STILL, "11": shift_of(0.01, 0.0)}}),
        json!({"path": {"11": shift_of(0.02, 0.0), "12": shift_of(0.03, 0.0)}}),
    ] {
        history
            .change_in_gesture(
                vec![Change::Op(set_params("n5", patch))],
                "agent",
                &mut |change, _: &[Op]| change,
                "track-1",
                false,
            )
            .unwrap();
    }
    history.end_gesture();
    assert_eq!(
        planar_of(history.project())
            .path
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        [10, 11, 12]
    );
    history.undo("agent").unwrap().unwrap();
    assert_eq!(history.project(), &before);
    // Set to what it is, it changes nothing and makes no step.
    let nothing = history.change(
        vec![Change::Op(set_params("n5", json!({"path": {}})))],
        "agent",
        |change, _| change,
    );
    assert_eq!(nothing.err(), Some(TransactionError::Empty));
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "what a planar tracker draws at a frame, with each operation"
)]
fn a_planar_tracker_of_a_layer_is_the_motion_of_its_path_at_the_frame() {
    let mut project = planar_tracked();
    applied(
        &mut project,
        set_params(
            "n5",
            json!({
                "operation": "steady", "reference": 10,
                "path": {"10": STILL, "20": shift_of(0.25, 0.125)},
            }),
        ),
    );
    assert!(composition(&project, "c1").is_animated());
    // c1 shows m1 from its start at the rate of the timeline: frame 20 shows picture 20. Held
    // still, the picture is put back a quarter of the width to the left and an eighth of the
    // height down, from the top left.
    let held = planar_at(&project, 20);
    assert_eq!(held.operation, PlanarOperation::Steady);
    assert_eq!(
        held.matrix,
        [[1.0, 0.0, -0.25], [0.0, 1.0, 0.125], [0.0, 0.0, 1.0]]
    );
    // Between two frames of the path, half as far.
    let half = planar_at(&project, 15).matrix;
    let middle = put_by(&half, [0.5, 0.5]);
    assert!(
        (middle[0] - 0.375).abs() < 1e-9 && (middle[1] - 0.5625).abs() < 1e-9,
        "{half:?}"
    );
    // At the reference frame, and before the path, where the plane is where it was first,
    // nothing moves; after the last frame of the path, as at that one.
    for frame in [10, 5, 0] {
        assert_eq!(
            planar_at(&project, frame).matrix,
            columbina_model::stabilize::IDENTITY,
            "{frame}"
        );
    }
    assert_eq!(planar_at(&project, 30), held);
    // Laid on the plane: the corners of the foreground, from the top left of the picture, go
    // where the corners are put, moved as the plane moved.
    applied(
        &mut project,
        set_params("n5", json!({"operation": "corner_pin"})),
    );
    let pinned = planar_at(&project, 20);
    assert_eq!(pinned.operation, PlanarOperation::CornerPin);
    for (corner, wanted) in [
        ([0.0, 0.0], [0.5, 0.125]),
        ([1.0, 0.0], [1.0, 0.125]),
        ([1.0, 1.0], [1.0, 0.625]),
        ([0.0, 1.0], [0.5, 0.625]),
    ] {
        let at = put_by(&pinned.matrix, corner);
        assert!(
            (at[0] - wanted[0]).abs() < 1e-9 && (at[1] - wanted[1]).abs() < 1e-9,
            "{corner:?}: {at:?}"
        );
    }
    // At the reference frame, where they are put.
    let resting = planar_at(&project, 10);
    let at = put_by(&resting.matrix, [0.0, 0.0]);
    assert!((at[0] - 0.25).abs() < 1e-9 && (at[1] - 0.25).abs() < 1e-9);
    // Corners that make no quadrilateral pin nothing: the node is its picture.
    let one = place_of(0.5, 0.5);
    let mut flat = project.clone();
    applied(
        &mut flat,
        set_params(
            "n5",
            json!({"corners": {"tl": one, "tr": one, "br": one, "bl": one}}),
        ),
    );
    assert_eq!(planar_at(&flat, 20).operation, PlanarOperation::None);
    // With an operation of None it is its picture, whatever the path.
    applied(&mut project, set_params("n5", json!({"operation": "none"})));
    assert_eq!(
        planar_at(&project, 20),
        PlanarTracked {
            operation: PlanarOperation::None,
            matrix: columbina_model::stabilize::IDENTITY,
        }
    );
    // With no path it is the same at every frame, and the composition does not change with
    // the frame.
    applied(
        &mut project,
        set_params(
            "n5",
            json!({"operation": "steady", "path": {"10": null, "20": null}}),
        ),
    );
    assert!(!composition(&project, "c1").is_animated());
    assert_eq!(
        planar_at(&project, 20).matrix,
        columbina_model::stabilize::IDENTITY
    );
}

#[test]
fn a_planar_tracker_takes_a_picture_a_foreground_and_a_mask() {
    let mut project = planar_tracked();
    applied(
        &mut project,
        add(
            Node::new("", Kind::Ellipse),
            vec![Link::new("", "n5", Input::Mask)],
        ),
    );
    // It has no background and no matte.
    for input in [Input::Background, Input::GarbageMatte, Input::SolidMatte] {
        assert_eq!(
            refused(&project, connect(Link::new(MEDIA_IN, "n5", input))),
            OpError::NoInput {
                node: "n5".into(),
                input
            }
        );
    }
    // A loop through any of its inputs is a loop.
    for input in [Input::Input, Input::Foreground, Input::Mask] {
        assert!(
            matches!(
                refused(&project, connect(Link::new("n3", "n5", input))),
                OpError::Loop { .. }
            ),
            "{input:?}"
        );
    }
    // A centre does not follow it: it publishes nothing.
    let follow = json!({"center": {"follow": {"node": "n5", "output": "tracker"}}});
    assert_eq!(
        refused(&project, set_params("n3", follow)),
        OpError::WrongNode {
            node: "n5".into(),
            kind: "planar_tracker"
        }
    );
    // The layer draws it after its picture, its foreground and its mask, which it names in
    // the order of its inputs.
    let fused = layer_at(&project, 20).fusion.unwrap();
    let at = fused.nodes.iter().position(|node| node.id == "n5").unwrap();
    let ids: Vec<&str> = fused.nodes[at]
        .kind
        .reads()
        .iter()
        .map(|&read| fused.nodes[read].id.as_str())
        .collect();
    assert_eq!(ids, [MEDIA_IN, "n4", "n6"]);
    let first = fused.nodes[at].kind.first_input();
    assert_eq!(
        first.map(|read| fused.nodes[read].id.as_str()),
        Some(MEDIA_IN)
    );
    assert!(matches!(
        fused.nodes[at].kind,
        FusedKind::PlanarTracker { mask: Some(_), .. }
    ));
    // Passed through, it hands on its picture, and neither its foreground nor its mask: the
    // Merge lays the picture of the clip, as it did before the Planar Tracker.
    applied(&mut project, passed_through("n5"));
    assert_eq!(drawn(&project), drawn(&merged()));
}

/// A composition that holds the picture of its clip still: `in` through the Planar Tracker
/// `plane`, whose path is `path` and whose reference is `reference`, into `out`.
fn planar_holding(path: &[(i64, [f64; 8])], reference: i64) -> Composition {
    let mut plane = Node::new("plane", Kind::PlanarTracker);
    plane.kind = NodeKind::PlanarTracker(PlanarTracker {
        operation: PlanarOperation::Steady,
        reference,
        path: path.iter().copied().collect(),
        ..PlanarTracker::default()
    });
    let mut composition = Composition {
        nodes: vec![
            Node::new("in", Kind::MediaIn),
            plane,
            Node::new("out", Kind::MediaOut),
        ],
        links: vec![
            Link::new("in", "plane", Input::Input),
            Link::new("plane", "out", Input::Input),
        ],
    };
    composition.sort_links();
    composition
}

/// The frames of the path of the Planar Tracker `plane` of clip `id`, and its reference.
fn planar_path_of(project: &Project, id: &str) -> (Vec<i64>, i64) {
    let node = composition(project, id)
        .node("plane")
        .expect("the composition has the planar tracker");
    let NodeKind::PlanarTracker(planar) = &node.kind else {
        panic!("the node is a planar tracker")
    };
    (planar.path.keys().copied().collect(), planar.reference)
}

#[test]
fn the_path_of_a_planar_tracker_is_moved_with_the_keys_of_its_clip() {
    // A colour counts its own frames: where its head is trimmed, the frames of the path and
    // the reference are moved as its keys are, so that each frame of the timeline shows what
    // it showed.
    let frames = [210, 215, 220, 230, 245];
    let mut project = with_a_colour_that_moves();
    let colour = project.timelines[0].tracks[0]
        .clips
        .iter_mut()
        .find(|clip| clip.id == "k1")
        .expect("the colour is there");
    colour.fusion = Some(planar_holding(&[(0, STILL), (40, shift_of(0.5, 0.25))], 10));
    project.validate().unwrap();
    let seen = |project: &Project| frames.map(|frame| planar_at(project, frame));
    let before = seen(&project);
    assert_ne!(before[0], before[4]);
    // At the reference frame, nothing moves.
    assert_eq!(before[0].matrix, columbina_model::stabilize::IDENTITY);
    edited(
        &mut project,
        edit(json!({"edit": "trim", "timeline": "t1", "clip": "k1", "edge": "in", "delta": 10})),
    );
    assert_eq!(planar_path_of(&project, "k1"), (vec![-10, 30], 0));
    assert_eq!(seen(&project), before);
    // Of media, a path is on its pictures: a trim of the head leaves it where it was.
    let mut media = Project::sample();
    applied(
        &mut media,
        set(
            "c1",
            Some(planar_holding(
                &[(20, STILL), (60, shift_of(0.25, 0.0))],
                20,
            )),
        ),
    );
    let before = [30, 40, 70].map(|frame| planar_at(&media, frame));
    edited(
        &mut media,
        edit(
            json!({"edit": "trim", "timeline": "t1", "clip": "c1", "edge": "in", "delta": 10,
            "linked": false}),
        ),
    );
    assert_eq!(planar_path_of(&media, "c1"), (vec![20, 60], 20));
    assert_eq!([30, 40, 70].map(|frame| planar_at(&media, frame)), before);
}
