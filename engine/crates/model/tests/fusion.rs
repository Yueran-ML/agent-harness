//! The composition of a clip (the Fusion page): how a project with one is written and read,
//! the operations on it, one by one, with what each gives back and what it refuses, the keys
//! of its settings by their paths, what the edits of a clip do to it, and what a frame of the
//! timeline is given of it.

use std::collections::BTreeMap;

use columbina_model::edit::copy_clips;
use columbina_model::fusion::{
    Composition, FillMethod, Fused, FusedKind, FusedNode, Input, Kind, Link, Node, NodeKind,
    PaintMode, Point, MEDIA_IN, MEDIA_OUT, MOST_NODES, MOST_POINTS,
};
use columbina_model::grade::{Composite, Interp, Key, Param, Pos, LONGEST_LABEL};
use columbina_model::resolve::{video_at, Layer};
use columbina_model::{
    apply, apply_done, expand, Change, Clip, Edit, History, Invalid, Material, Op, OpError,
    Project, ReadError, Rgba, Source, Time, TrackKind, TransactionError,
};
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
