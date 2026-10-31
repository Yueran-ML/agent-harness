//! The operations and edits of grades, one by one: what each does, what it gives back, and what
//! it refuses.

use columbina_model::grade::{Composite, Interp, Key, Kind, Link, Node, NodeKind, Param, Port};
use columbina_model::{
    apply, apply_done, expand, Edit, EditError, Grade, Group, Op, OpError, Project, Target,
};
use serde_json::json;

fn c1() -> Target {
    Target::Clip {
        timeline: "t1".into(),
        clip: "c1".into(),
    }
}

fn grade(project: &Project, target: &Target) -> Grade {
    let found = match target {
        Target::Clip { clip, .. } => project.timelines[0]
            .tracks
            .iter()
            .flat_map(|r| &r.clips)
            .find(|c| c.id == *clip)
            .map(|c| c.grade.clone()),
        Target::Timeline { .. } => Some(project.timelines[0].grade.clone()),
        Target::PreClip { group } => project
            .groups
            .iter()
            .find(|k| k.id == *group)
            .map(|k| k.pre.clone()),
        Target::PostClip { group } => project
            .groups
            .iter()
            .find(|k| k.id == *group)
            .map(|k| k.post.clone()),
    };
    found.expect("the grade is there")
}

fn rgb(from: &str, to: &str, input: u32) -> Link {
    Link {
        from: from.into(),
        to: to.into(),
        kind: Port::Rgb,
        input,
    }
}

fn key(from: &str, to: &str, input: u32) -> Link {
    Link {
        from: from.into(),
        to: to.into(),
        kind: Port::Key,
        input,
    }
}

fn add(node: Node, links: Vec<Link>) -> Op {
    Op::AddNode {
        grade: c1(),
        node,
        index: None,
        links,
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

/// The sample with c1 graded by n3 then n4, in a row.
fn serial() -> Project {
    let mut project = Project::sample();
    applied(
        &mut project,
        add(
            Node::corrector(""),
            vec![rgb("source", "", 0), rgb("", "output", 0)],
        ),
    );
    applied(
        &mut project,
        add(
            Node::corrector(""),
            vec![rgb("n3", "", 0), rgb("", "output", 0)],
        ),
    );
    project
}

#[test]
fn a_node_added_with_no_id_is_given_one_and_takes_the_place_of_the_link_it_overwrites() {
    let mut project = Project::sample();
    assert_eq!(project.next_id, 3);
    let (done, inverse) = applied(
        &mut project,
        add(
            Node::corrector(""),
            vec![rgb("", "output", 0), rgb("source", "", 0)],
        ),
    );
    assert_eq!(project.next_id, 4);
    let g = grade(&project, &c1());
    assert_eq!(g.nodes, vec![Node::corrector("n3")]);
    assert_eq!(
        g.links,
        vec![rgb("source", "n3", 0), rgb("n3", "output", 0)]
    );
    assert!(g.is_complete());
    assert_eq!(
        done,
        Op::AddNode {
            grade: c1(),
            node: Node::corrector("n3"),
            index: Some(0),
            links: vec![rgb("source", "n3", 0), rgb("n3", "output", 0)],
        }
    );
    assert_eq!(
        inverse,
        Op::RemoveNode {
            grade: c1(),
            id: "n3".into(),
            links: Some(vec![rgb("source", "output", 0)]),
        }
    );
    // The clip's other copies keep their grades: c2 is ungraded.
    let c2 = Target::Clip {
        timeline: "t1".into(),
        clip: "c2".into(),
    };
    assert!(grade(&project, &c2).is_empty());
}

#[test]
fn an_operation_on_a_grade_is_written_as_the_protocol_names_it() {
    let op = Op::Disconnect {
        grade: Target::PreClip { group: "k3".into() },
        to: "output".into(),
        kind: Port::Rgb,
        input: 0,
    };
    assert_eq!(
        serde_json::to_value(&op).unwrap(),
        json!({"op": "disconnect", "grade": {"level": "pre_clip", "group": "k3"}, "to": "output", "kind": "rgb", "input": 0})
    );
    let read: Op = serde_json::from_value(json!({
        "op": "add_node",
        "grade": {"level": "clip", "timeline": "t1", "clip": "c1"},
        "node": {"id": "", "kind": "parallel_mixer", "label": "", "enabled": true, "params": {"inputs": 3}},
    }))
    .unwrap();
    let Op::AddNode {
        node, index, links, ..
    } = read
    else {
        panic!()
    };
    assert_eq!((index, links), (None, vec![]));
    assert_eq!(node.kind.inputs(Port::Rgb), 3);
    let wrong = json!({"op": "remove_node", "grade": {"level": "timeline", "timeline": "t1", "clip": "c1"}, "id": "n1"});
    assert!(serde_json::from_value::<Op>(wrong).is_err());
}

#[test]
fn a_node_keeps_the_id_it_is_given_unless_it_is_held_reserved_or_an_end() {
    let project = serial();
    let mut mine = project.clone();
    applied(&mut mine, add(Node::corrector("skin"), vec![]));
    assert_eq!(grade(&mine, &c1()).nodes[2].id, "skin");
    // An id that the engine made for a node of another grade is the node's alone in its grade.
    let mut other = project.clone();
    let c2 = Target::Clip {
        timeline: "t1".into(),
        clip: "c2".into(),
    };
    let op = Op::AddNode {
        grade: c2.clone(),
        node: Node::corrector("n3"),
        index: None,
        links: vec![],
    };
    applied(&mut other, op);
    assert_eq!(grade(&other, &c2).nodes[0].id, "n3");

    let taken = refused(&project, add(Node::corrector("n4"), vec![]));
    assert_eq!(
        taken,
        OpError::Taken {
            what: "node",
            id: "n4".into()
        }
    );
    for end in ["source", "output"] {
        let error = refused(&project, add(Node::corrector(end), vec![]));
        assert!(
            matches!(error, OpError::Taken { what: "node", .. }),
            "{error}"
        );
    }
    let error = refused(&project, add(Node::corrector("n99"), vec![]));
    assert!(matches!(error, OpError::Reserved { .. }), "{error}");
    let error = refused(&project, add(Node::corrector("$a"), vec![]));
    assert!(matches!(error, OpError::Dollar { .. }), "{error}");
    let mut long = Node::corrector("");
    long.label = "x".repeat(101);
    assert_eq!(
        refused(&project, add(long, vec![])),
        OpError::OutOfRange { what: "label" }
    );
    let op = Op::AddNode {
        grade: c1(),
        node: Node::corrector(""),
        index: Some(3),
        links: vec![],
    };
    assert_eq!(refused(&project, op), OpError::OutOfList { at: 3, len: 2 });
}

#[test]
fn a_node_is_added_with_links_only_of_its_own_and_none_that_go_round() {
    let project = serial();
    let error = refused(&project, add(Node::corrector(""), vec![rgb("n3", "n4", 0)]));
    assert_eq!(error, OpError::OutOfRange { what: "links" });
    let error = refused(
        &project,
        add(
            Node::corrector(""),
            vec![rgb("source", "", 0), rgb("n3", "", 0)],
        ),
    );
    assert_eq!(error, OpError::OutOfRange { what: "links" });
    let error = refused(&project, add(Node::corrector(""), vec![rgb("", "", 0)]));
    assert!(matches!(error, OpError::Loop { .. }), "{error}");
    let error = refused(
        &project,
        add(
            Node::corrector(""),
            vec![rgb("n4", "", 0), rgb("", "n3", 0)],
        ),
    );
    assert!(matches!(error, OpError::Loop { .. }), "{error}");
    let error = refused(
        &project,
        add(Node::new("", Kind::KeyMixer), vec![rgb("", "output", 0)]),
    );
    assert!(
        matches!(error, OpError::NoPort { input: None, .. }),
        "{error}"
    );
    let error = refused(&project, add(Node::corrector(""), vec![rgb("", "n3", 1)]));
    assert!(
        matches!(error, OpError::NoPort { input: Some(1), .. }),
        "{error}"
    );
    let error = refused(&project, add(Node::corrector(""), vec![rgb("n9", "", 0)]));
    assert!(
        matches!(error, OpError::Missing { what: "node", .. }),
        "{error}"
    );
}

#[test]
fn a_node_taken_away_is_bridged_and_the_ones_after_it_move_up_a_number() {
    let mut project = serial();
    applied(&mut project, add(Node::corrector(""), vec![]));
    let (done, inverse) = applied(
        &mut project,
        Op::RemoveNode {
            grade: c1(),
            id: "n3".into(),
            links: None,
        },
    );
    let g = grade(&project, &c1());
    // n4 is number 1 now, and n5 number 2 (p. 3318).
    let ids: Vec<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, ["n4", "n5"]);
    assert_eq!(
        g.links,
        vec![rgb("source", "n4", 0), rgb("n4", "output", 0)]
    );
    assert_eq!(
        done,
        Op::RemoveNode {
            grade: c1(),
            id: "n3".into(),
            links: Some(vec![rgb("source", "n4", 0)]),
        }
    );
    assert_eq!(
        inverse,
        Op::AddNode {
            grade: c1(),
            node: Node::corrector("n3"),
            index: Some(0),
            links: vec![rgb("source", "n3", 0), rgb("n3", "n4", 0)],
        }
    );
}

#[test]
fn a_node_that_feeds_many_inputs_is_bridged_into_each_and_its_key_too() {
    let mut project = serial();
    // n3 also gives its key to n4, and its picture to input 1 of a parallel mixer n5.
    applied(
        &mut project,
        Op::Connect {
            grade: c1(),
            link: key("n3", "n4", 0),
        },
    );
    applied(
        &mut project,
        add(
            Node::new("", Kind::ParallelMixer),
            vec![rgb("n4", "", 0), rgb("n3", "", 1), rgb("", "output", 0)],
        ),
    );
    applied(
        &mut project,
        Op::RemoveNode {
            grade: c1(),
            id: "n3".into(),
            links: None,
        },
    );
    let g = grade(&project, &c1());
    // Nothing came into the key input of n3, so nothing goes into that of n4 now.
    assert_eq!(
        g.links,
        vec![
            rgb("source", "n4", 0),
            rgb("n4", "n5", 0),
            rgb("source", "n5", 1),
            rgb("n5", "output", 0),
        ]
    );
}

#[test]
fn a_node_taken_away_may_be_replaced_by_links_into_the_inputs_it_fed_only() {
    let mut project = serial();
    let only = |links| Op::RemoveNode {
        grade: c1(),
        id: "n4".into(),
        links: Some(links),
    };
    assert_eq!(
        refused(&project, only(vec![rgb("source", "n3", 0)])),
        OpError::OutOfRange { what: "links" }
    );
    assert_eq!(
        refused(&project, only(vec![rgb("n4", "output", 0)])),
        OpError::OutOfRange { what: "links" }
    );
    // Nothing in its place: the output is left without a link, and the grade does not colour.
    applied(&mut project, only(vec![]));
    let g = grade(&project, &c1());
    assert_eq!(g.links, vec![rgb("source", "n3", 0)]);
    assert!(!g.is_complete());
    assert_eq!(
        refused(
            &project,
            Op::RemoveNode {
                grade: c1(),
                id: "n4".into(),
                links: None,
            }
        ),
        OpError::Missing {
            what: "node",
            id: "n4".into()
        }
    );
}

#[test]
fn a_link_takes_the_place_of_the_one_its_input_had_and_is_taken_back_to_it() {
    let mut project = serial();
    let (_, inverse) = applied(
        &mut project,
        Op::Connect {
            grade: c1(),
            link: rgb("source", "n4", 0),
        },
    );
    assert_eq!(
        inverse,
        Op::Connect {
            grade: c1(),
            link: rgb("n3", "n4", 0),
        }
    );
    // n3 goes nowhere now: the grade is kept, but does not colour.
    assert!(!grade(&project, &c1()).is_complete());
    let (_, inverse) = applied(
        &mut project,
        Op::Connect {
            grade: c1(),
            link: key("n3", "n4", 0),
        },
    );
    assert_eq!(
        inverse,
        Op::Disconnect {
            grade: c1(),
            to: "n4".into(),
            kind: Port::Key,
            input: 0,
        }
    );
    let (_, inverse) = applied(
        &mut project,
        Op::Disconnect {
            grade: c1(),
            to: "n4".into(),
            kind: Port::Key,
            input: 0,
        },
    );
    assert_eq!(
        inverse,
        Op::Connect {
            grade: c1(),
            link: key("n3", "n4", 0),
        }
    );
    let error = refused(
        &project,
        Op::Disconnect {
            grade: c1(),
            to: "n4".into(),
            kind: Port::Key,
            input: 0,
        },
    );
    assert_eq!(
        error,
        OpError::Missing {
            what: "link",
            id: "n4".into()
        }
    );
    let error = refused(
        &serial(),
        Op::Connect {
            grade: c1(),
            link: rgb("n4", "n3", 0),
        },
    );
    assert!(matches!(error, OpError::Loop { .. }), "{error}");
    let error = refused(
        &project,
        Op::Connect {
            grade: c1(),
            link: key("source", "n3", 0),
        },
    );
    assert!(matches!(error, OpError::NoPort { .. }), "{error}");
    let error = refused(
        &project,
        Op::Connect {
            grade: c1(),
            link: rgb("output", "n3", 0),
        },
    );
    assert!(matches!(error, OpError::NoPort { .. }), "{error}");
}

#[test]
fn the_settings_of_a_node_are_changed_where_they_are_given_and_taken_back_exactly() {
    let mut project = serial();
    let patch = json!({
        "primaries": {"saturation": 60, "lift": {"r": -0.05}},
        "lut": "LUTs/film.cube",
        "curves": {"y": {"points": [[0, 0], [0.5, 0.6], [1, 1]]}},
    });
    let (done, inverse) = applied(
        &mut project,
        Op::SetNode {
            grade: c1(),
            id: "n3".into(),
            label: Some("Skin".into()),
            enabled: Some(false),
            params: Some(patch),
        },
    );
    let g = grade(&project, &c1());
    let n3 = &g.nodes[0];
    assert_eq!((n3.label.as_str(), n3.enabled), ("Skin", false));
    let corrector = n3.as_corrector().unwrap();
    assert_eq!(corrector.primaries.saturation, Param::Fixed(60.0));
    assert_eq!(corrector.primaries.lift.r, Param::Fixed(-0.05));
    assert_eq!(corrector.primaries.lift.g, Param::Fixed(0.0));
    assert_eq!(corrector.lut.as_deref(), Some("LUTs/film.cube"));
    assert_eq!(corrector.curves.y.points[1], [0.5, 0.6]);
    assert_eq!(corrector.curves.r.points.len(), 2);
    // What was done names the values as they were read, and the inverse as they were.
    assert_eq!(
        done,
        Op::SetNode {
            grade: c1(),
            id: "n3".into(),
            label: Some("Skin".into()),
            enabled: Some(false),
            params: Some(json!({
                "primaries": {"saturation": 60.0, "lift": {"r": -0.05}},
                "lut": "LUTs/film.cube",
                "curves": {"y": {"points": [[0.0, 0.0], [0.5, 0.6], [1.0, 1.0]]}},
            })),
        }
    );
    assert_eq!(
        inverse,
        Op::SetNode {
            grade: c1(),
            id: "n3".into(),
            label: Some(String::new()),
            enabled: Some(true),
            params: Some(json!({
                "primaries": {"saturation": 50.0, "lift": {"r": 0.0}},
                "lut": null,
                "curves": {"y": {"points": [[0.0, 0.0], [1.0, 1.0]]}},
            })),
        }
    );
}

#[test]
fn a_setting_is_keyframed_and_set_back_to_one_value() {
    let mut project = serial();
    let keys = vec![
        Key {
            frame: 0,
            value: 1.0,
            interp: Interp::Linear,
        },
        Key {
            frame: 50,
            value: 1.5,
            interp: Interp::Hold,
        },
    ];
    let set = |value: serde_json::Value| Op::SetNode {
        grade: c1(),
        id: "n4".into(),
        label: None,
        enabled: None,
        params: Some(json!({"primaries": {"contrast": value}})),
    };
    let (_, inverse) = applied(&mut project, set(json!({ "keys": keys })));
    let contrast = |project: &Project| {
        grade(project, &c1()).nodes[1]
            .as_corrector()
            .unwrap()
            .primaries
            .contrast
            .clone()
    };
    assert_eq!(contrast(&project), Param::Keys(keys.clone()));
    assert_eq!(contrast(&project).value_at(25.0), 1.25);
    assert_eq!(inverse, set(json!(1.0)));
    applied(&mut project, set(json!(1.2)));
    assert_eq!(contrast(&project), Param::Fixed(1.2));
}

#[test]
fn settings_that_are_not_of_the_node_or_out_of_bounds_are_refused() {
    let project = serial();
    let set = |id: &str, params: serde_json::Value| Op::SetNode {
        grade: c1(),
        id: id.into(),
        label: None,
        enabled: None,
        params: Some(params),
    };
    let error = refused(
        &project,
        set("n3", json!({"primaries": {"saturation": 101}})),
    );
    assert_eq!(
        error,
        OpError::OutOfRange {
            what: "primaries.saturation"
        }
    );
    let error = refused(
        &project,
        set("n3", json!({"primaries": {"temperature": -4001}})),
    );
    assert_eq!(
        error,
        OpError::OutOfRange {
            what: "primaries.temperature"
        }
    );
    let error = refused(&project, set("n3", json!({"primaries": {"brightness": 1}})));
    assert!(
        matches!(&error, OpError::Params { why, .. } if why.contains("brightness")),
        "{error}"
    );
    let error = refused(&project, set("n3", json!({"inputs": 3})));
    assert!(matches!(error, OpError::Params { .. }), "{error}");
    let error = refused(&project, set("n3", json!(5)));
    assert!(matches!(error, OpError::Params { .. }), "{error}");
    let error = refused(
        &project,
        set(
            "n3",
            json!({"curves": {"y": {"points": [[0.5, 0], [0.2, 1]]}}}),
        ),
    );
    assert_eq!(
        error,
        OpError::OutOfRange {
            what: "curves.y.points"
        }
    );
    let error = refused(
        &project,
        set("n3", json!({"hsl": {"hue_vs_hue": [[0.2, 1.5]]}})),
    );
    assert_eq!(
        error,
        OpError::OutOfRange {
            what: "hsl.hue_vs_hue"
        }
    );
    let error = refused(&project, set("n3", json!({"lut": ""})));
    assert_eq!(error, OpError::OutOfRange { what: "lut" });
    let error = refused(
        &project,
        set("n3", json!({"key": {"output": {"gain": 2.5}}})),
    );
    assert_eq!(
        error,
        OpError::OutOfRange {
            what: "key.output.gain"
        }
    );
    let error = refused(&project, set("n9", json!({})));
    assert!(
        matches!(error, OpError::Missing { what: "node", .. }),
        "{error}"
    );
}

#[test]
fn a_mixer_keeps_the_inputs_that_links_go_into() {
    let mut project = serial();
    applied(
        &mut project,
        add(
            Node::new("", Kind::LayerMixer),
            vec![rgb("n3", "", 0), rgb("n4", "", 1)],
        ),
    );
    let set = |inputs: u32| Op::SetNode {
        grade: c1(),
        id: "n5".into(),
        label: None,
        enabled: None,
        params: Some(json!({"inputs": inputs})),
    };
    let error = refused(&project, add(Node::corrector(""), vec![rgb("", "n5", 2)]));
    assert!(
        matches!(error, OpError::NoPort { .. }),
        "two inputs at first: {error}"
    );
    applied(&mut project, set(3));
    applied(
        &mut project,
        Op::Connect {
            grade: c1(),
            link: rgb("source", "n5", 2),
        },
    );
    let error = refused(&project, set(2));
    assert_eq!(
        error,
        OpError::NoPort {
            node: "n5".into(),
            kind: Port::Rgb,
            input: Some(2)
        }
    );
    assert_eq!(
        refused(&project, set(0)),
        OpError::OutOfRange { what: "inputs" }
    );
    applied(
        &mut project,
        Op::SetNode {
            grade: c1(),
            id: "n5".into(),
            label: None,
            enabled: None,
            params: Some(json!({"composite": "screen"})),
        },
    );
    let NodeKind::LayerMixer(mixer) = &grade(&project, &c1()).nodes[2].kind else {
        panic!()
    };
    assert_eq!((mixer.inputs, mixer.composite), (3, Composite::Screen));
}

#[test]
fn a_grade_is_put_in_whole_with_its_ids_and_taken_back_whole() {
    let mut project = serial();
    let mut value = grade(&project, &c1());
    value.links.reverse();
    let timeline = Target::Timeline {
        timeline: "t1".into(),
    };
    let (done, inverse) = applied(
        &mut project,
        Op::SetGrade {
            grade: timeline.clone(),
            value,
        },
    );
    let Op::SetGrade { value: put, .. } = done else {
        panic!()
    };
    assert_eq!(put, grade(&project, &c1()), "its links are put in order");
    assert_eq!(
        inverse,
        Op::SetGrade {
            grade: timeline.clone(),
            value: Grade::default()
        }
    );
    let set = |value| Op::SetGrade {
        grade: timeline.clone(),
        value,
    };
    let with = |id: &str| Grade {
        nodes: vec![Node::corrector(id)],
        links: vec![],
    };
    assert_eq!(refused(&project, set(with(""))), OpError::NoId);
    assert!(matches!(
        refused(&project, set(with("n50"))),
        OpError::Reserved { .. }
    ));
    let twice = Grade {
        nodes: vec![Node::corrector("a"), Node::corrector("a")],
        links: vec![],
    };
    assert!(matches!(
        refused(&project, set(twice)),
        OpError::Taken { what: "node", .. }
    ));
    let round = Grade {
        nodes: vec![Node::corrector("a")],
        links: vec![rgb("a", "a", 0)],
    };
    assert!(matches!(
        refused(&project, set(round)),
        OpError::Loop { .. }
    ));
}

#[test]
fn a_group_is_added_named_and_taken_away_once_no_clip_is_in_it() {
    let mut project = serial();
    let (done, inverse) = applied(
        &mut project,
        Op::AddGroup {
            group: Group::new("", "Interview"),
            index: None,
        },
    );
    assert_eq!(project.groups, vec![Group::new("k5", "Interview")]);
    assert_eq!(
        done,
        Op::AddGroup {
            group: Group::new("k5", "Interview"),
            index: Some(0)
        }
    );
    assert_eq!(inverse, Op::RemoveGroup { id: "k5".into() });
    let (_, inverse) = applied(
        &mut project,
        Op::SetGroup {
            id: "k5".into(),
            name: Some("Exteriors".into()),
        },
    );
    assert_eq!(
        inverse,
        Op::SetGroup {
            id: "k5".into(),
            name: Some("Interview".into())
        }
    );
    let into = |group: Option<&str>| Op::SetClipGroup {
        timeline: "t1".into(),
        clip: "c1".into(),
        group: group.map(Into::into),
    };
    let (_, inverse) = applied(&mut project, into(Some("k5")));
    assert_eq!(inverse, into(None));
    assert_eq!(
        refused(&project, Op::RemoveGroup { id: "k5".into() }),
        OpError::Grouped {
            group: "k5".into(),
            clip: "c1".into()
        }
    );
    assert_eq!(
        refused(&project, into(Some("k9"))),
        OpError::Missing {
            what: "group",
            id: "k9".into()
        }
    );
    // A pre-clip grade of the group is graded as any other.
    let pre = Target::PreClip { group: "k5".into() };
    applied(
        &mut project,
        Op::AddNode {
            grade: pre.clone(),
            node: Node::corrector(""),
            index: None,
            links: vec![rgb("source", "", 0), rgb("", "output", 0)],
        },
    );
    assert_eq!(grade(&project, &pre).nodes[0].id, "n6");
    applied(&mut project, into(None));
    let (_, inverse) = applied(&mut project, Op::RemoveGroup { id: "k5".into() });
    let Op::AddGroup { group, index } = inverse else {
        panic!()
    };
    assert_eq!((group.name.as_str(), index), ("Exteriors", Some(0)));
    assert_eq!(group.pre.nodes.len(), 1);
    assert_eq!(
        refused(
            &project,
            Op::AddGroup {
                group: Group::new("k99", ""),
                index: None
            }
        ),
        OpError::Reserved { id: "k99".into() }
    );
}

#[test]
fn a_clip_is_put_in_only_a_group_that_is_there_and_a_grade_with_its_clip() {
    let mut project = serial();
    applied(
        &mut project,
        Op::AddGroup {
            group: Group::new("", "A"),
            index: None,
        },
    );
    applied(
        &mut project,
        Op::SetClipGroup {
            timeline: "t1".into(),
            clip: "c1".into(),
            group: Some("k5".into()),
        },
    );
    let (_, inverse) = applied(
        &mut project,
        Op::RemoveClip {
            timeline: "t1".into(),
            id: "c1".into(),
        },
    );
    let Op::InsertClip { clip, .. } = &inverse else {
        panic!()
    };
    assert_eq!(clip.group.as_deref(), Some("k5"));
    assert_eq!(clip.grade.nodes.len(), 2);
    // With c1 gone the group may be taken away, and then c1 cannot come back into it.
    applied(&mut project, Op::RemoveGroup { id: "k5".into() });
    assert_eq!(
        refused(&project, inverse),
        OpError::Missing {
            what: "group",
            id: "k5".into()
        }
    );
}

#[test]
fn a_project_read_with_grades_is_put_right_or_refused_as_the_operations_would() {
    let mut project = serial();
    applied(
        &mut project,
        Op::AddGroup {
            group: Group::new("", "A"),
            index: None,
        },
    );
    let mut value = serde_json::to_value(&project).unwrap();
    // Links out of order are put in order, and next_id goes past the ids of nodes and groups.
    let links = &mut value["timelines"][0]["tracks"][0]["clips"][0]["grade"]["links"];
    links.as_array_mut().unwrap().reverse();
    value["next_id"] = 1.into();
    assert_eq!(Project::from_value(value.clone()).unwrap(), project);

    let mut twice = value.clone();
    twice["timelines"][0]["tracks"][0]["clips"][0]["grade"]["nodes"][1]["id"] = "n3".into();
    let error = Project::from_value(twice).unwrap_err().to_string();
    assert!(error.contains("clip c1") && error.contains("n3"), "{error}");
    let mut stray = value.clone();
    stray["timelines"][0]["tracks"][0]["clips"][0]["group"] = "k9".into();
    let error = Project::from_value(stray).unwrap_err().to_string();
    assert!(error.contains("there is no group k9"), "{error}");
    let mut round = value.clone();
    round["timelines"][0]["grade"] = json!({
        "nodes": [serde_json::to_value(Node::corrector("a")).unwrap()],
        "links": [{"from": "a", "to": "a", "kind": "rgb", "input": 0}],
    });
    let error = Project::from_value(round).unwrap_err().to_string();
    assert!(
        error.contains("timeline t1") && error.contains("loop"),
        "{error}"
    );
    let mut named = value;
    named["groups"][0]["name"] = "x".repeat(101).into();
    let error = Project::from_value(named).unwrap_err().to_string();
    assert!(error.contains("group k5"), "{error}");
}

// ---------------------------------------------------------------------------------------------
// Edits
// ---------------------------------------------------------------------------------------------

/// Expand an edit, check that it is undone and redone exactly, and give back what was done.
fn edited(project: &mut Project, edit: Edit) -> Vec<Op> {
    let before = project.clone();
    let edit_text = format!("{edit:?}");
    let expansion = expand(project, edit).unwrap_or_else(|e| panic!("{edit_text}: {e}"));
    let after = project.clone();
    let mut undone = after.clone();
    for inverse in expansion.inverse.iter().rev() {
        apply(&mut undone, inverse.clone()).unwrap();
    }
    let mut expected = before.clone();
    expected.next_id = after.next_id;
    assert_eq!(undone, expected, "{edit_text} undone");
    let mut redone = expected;
    for done in &expansion.done {
        apply(&mut redone, done.clone()).unwrap();
    }
    assert_eq!(redone, after, "{edit_text} redone");
    expansion.done
}

fn refused_edit(project: &Project, edit: Edit) -> EditError {
    let mut copy = project.clone();
    let error = expand(&mut copy, edit).unwrap_err();
    assert_eq!(&copy, project);
    error
}

fn ids(project: &Project) -> Vec<String> {
    grade(project, &c1())
        .nodes
        .iter()
        .map(|n| n.id.clone())
        .collect()
}

#[test]
fn add_serial_puts_a_node_after_the_one_chosen_and_numbers_it_last() {
    let mut project = serial();
    let done = edited(
        &mut project,
        Edit::AddSerial {
            grade: c1(),
            node: "n3".into(),
        },
    );
    assert_eq!(done.len(), 1);
    // In between n3 and n4, and number 3 (p. 3318).
    assert_eq!(ids(&project), ["n3", "n4", "n5"]);
    assert_eq!(
        grade(&project, &c1()).links,
        vec![
            rgb("source", "n3", 0),
            rgb("n5", "n4", 0),
            rgb("n3", "n5", 0),
            rgb("n4", "output", 0)
        ]
    );
    edited(
        &mut project,
        Edit::AddSerial {
            grade: c1(),
            node: "source".into(),
        },
    );
    let g = grade(&project, &c1());
    assert_eq!(g.link_into("n3", Port::Rgb, 0), Some(&rgb("n6", "n3", 0)));
    assert!(g.is_complete());
}

#[test]
fn add_serial_before_and_append_node_put_a_node_before_the_one_chosen_and_the_output() {
    let mut project = serial();
    edited(
        &mut project,
        Edit::AddSerialBefore {
            grade: c1(),
            node: "n4".into(),
        },
    );
    edited(&mut project, Edit::AppendNode { grade: c1() });
    let g = grade(&project, &c1());
    assert_eq!(
        g.links,
        vec![
            rgb("source", "n3", 0),
            rgb("n5", "n4", 0),
            rgb("n3", "n5", 0),
            rgb("n4", "n6", 0),
            rgb("n6", "output", 0),
        ]
    );
    // Into an ungraded clip, Append Node makes its first node.
    let mut fresh = Project::sample();
    edited(&mut fresh, Edit::AppendNode { grade: c1() });
    assert_eq!(
        grade(&fresh, &c1()).links,
        vec![rgb("source", "n3", 0), rgb("n3", "output", 0)]
    );
    let error = refused_edit(
        &project,
        Edit::AddSerialBefore {
            grade: c1(),
            node: "source".into(),
        },
    );
    assert!(
        matches!(error, EditError::Op(OpError::WrongNode { .. })),
        "{error}"
    );
    let error = refused_edit(
        &project,
        Edit::AddSerial {
            grade: c1(),
            node: "output".into(),
        },
    );
    assert!(
        matches!(error, EditError::Op(OpError::WrongNode { .. })),
        "{error}"
    );
}

#[test]
fn add_parallel_mixes_a_new_node_beside_the_chosen_one_and_adds_inputs_after() {
    let mut project = serial();
    let done = edited(
        &mut project,
        Edit::AddParallel {
            grade: c1(),
            node: "n4".into(),
        },
    );
    assert_eq!(done.len(), 2);
    let g = grade(&project, &c1());
    assert_eq!(ids(&project), ["n3", "n4", "n5", "n6"]);
    assert!(matches!(g.nodes[3].kind, NodeKind::ParallelMixer(_)));
    assert_eq!(
        g.links,
        vec![
            rgb("source", "n3", 0),
            rgb("n3", "n4", 0),
            rgb("n3", "n5", 0),
            rgb("n4", "n6", 0),
            rgb("n5", "n6", 1),
            rgb("n6", "output", 0),
        ]
    );
    assert!(g.is_complete());
    // Again on n4, whose picture goes into that mixer alone: a third input (p. 3335).
    edited(
        &mut project,
        Edit::AddParallel {
            grade: c1(),
            node: "n4".into(),
        },
    );
    let g = grade(&project, &c1());
    assert_eq!(g.nodes.len(), 5);
    assert_eq!(g.nodes[3].kind.inputs(Port::Rgb), 3);
    assert_eq!(g.link_into("n6", Port::Rgb, 2), Some(&rgb("n7", "n6", 2)));
    assert_eq!(g.link_into("n7", Port::Rgb, 0), Some(&rgb("n3", "n7", 0)));
    let error = refused_edit(
        &project,
        Edit::AddParallel {
            grade: c1(),
            node: "n6".into(),
        },
    );
    assert_eq!(
        error,
        EditError::Op(OpError::WrongNode {
            node: "n6".into(),
            kind: "parallel_mixer"
        })
    );
}

#[test]
fn add_layer_lays_the_new_node_on_the_lowest_input_of_a_layer_mixer() {
    let mut project = serial();
    edited(
        &mut project,
        Edit::AddLayer {
            grade: c1(),
            node: "n3".into(),
        },
    );
    let g = grade(&project, &c1());
    let NodeKind::LayerMixer(mixer) = &g.nodes[3].kind else {
        panic!("{:?}", g.nodes)
    };
    assert_eq!((mixer.inputs, mixer.composite), (2, Composite::Normal));
    assert_eq!(g.link_into("n6", Port::Rgb, 0), Some(&rgb("n3", "n6", 0)));
    assert_eq!(g.link_into("n6", Port::Rgb, 1), Some(&rgb("n5", "n6", 1)));
    assert_eq!(g.link_into("n4", Port::Rgb, 0), Some(&rgb("n6", "n4", 0)));
    assert!(g.is_complete());
}

#[test]
fn extract_node_keeps_the_node_out_of_the_tree_and_reset_node_sets_it_anew() {
    let mut project = serial();
    applied(
        &mut project,
        Op::SetNode {
            grade: c1(),
            id: "n3".into(),
            label: Some("Base".into()),
            enabled: None,
            params: Some(json!({"primaries": {"gain": {"y": 1.4}}})),
        },
    );
    edited(
        &mut project,
        Edit::ResetNode {
            grade: c1(),
            node: "n3".into(),
        },
    );
    let g = grade(&project, &c1());
    assert_eq!(g.nodes[0].label, "Base");
    assert_eq!(g.nodes[0].kind, NodeKind::new(Kind::Corrector));
    edited(
        &mut project,
        Edit::ExtractNode {
            grade: c1(),
            node: "n3".into(),
        },
    );
    let g = grade(&project, &c1());
    assert_eq!(ids(&project), ["n3", "n4"]);
    assert_eq!(
        g.links,
        vec![rgb("source", "n4", 0), rgb("n4", "output", 0)]
    );
    assert!(!g.is_complete());
}

#[test]
fn copy_grade_puts_a_grade_in_place_of_others_and_append_grade_after_them() {
    let mut project = serial();
    let c2 = Target::Clip {
        timeline: "t1".into(),
        clip: "c2".into(),
    };
    let timeline = Target::Timeline {
        timeline: "t1".into(),
    };
    let done = edited(
        &mut project,
        Edit::CopyGrade {
            from: c1(),
            to: vec![c2.clone(), timeline.clone()],
        },
    );
    assert_eq!(done.len(), 2);
    assert_eq!(grade(&project, &c2), grade(&project, &c1()));
    assert_eq!(grade(&project, &timeline), grade(&project, &c1()));
    edited(
        &mut project,
        Edit::AppendGrade {
            from: c1(),
            to: vec![c2.clone()],
        },
    );
    let g = grade(&project, &c2);
    let ids: Vec<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, ["n3", "n4", "n5", "n6"]);
    assert_eq!(
        g.links,
        vec![
            rgb("source", "n3", 0),
            rgb("n3", "n4", 0),
            rgb("n4", "n5", 0),
            rgb("n5", "n6", 0),
            rgb("n6", "output", 0),
        ]
    );
    let error = refused_edit(
        &project,
        Edit::CopyGrade {
            from: c1(),
            to: vec![],
        },
    );
    assert_eq!(error, EditError::Op(OpError::OutOfRange { what: "to" }));
}

#[test]
fn paste_value_sets_one_setting_and_paste_node_all_of_them() {
    let mut project = serial();
    applied(
        &mut project,
        Op::SetNode {
            grade: c1(),
            id: "n3".into(),
            label: None,
            enabled: None,
            params: Some(json!({"primaries": {"saturation": 70, "contrast": 1.3}})),
        },
    );
    edited(
        &mut project,
        Edit::PasteValue {
            from: c1(),
            from_node: "n3".into(),
            to: c1(),
            to_node: "n4".into(),
            param: "primaries.saturation".into(),
        },
    );
    let g = grade(&project, &c1());
    let n4 = g.nodes[1].as_corrector().unwrap();
    assert_eq!(n4.primaries.saturation, Param::Fixed(70.0));
    assert_eq!(n4.primaries.contrast, Param::Fixed(1.0));
    edited(
        &mut project,
        Edit::PasteNode {
            from: c1(),
            from_node: "n3".into(),
            to: c1(),
            to_node: "n4".into(),
        },
    );
    let g = grade(&project, &c1());
    assert_eq!(g.nodes[1].kind, g.nodes[0].kind);
    let error = refused_edit(
        &project,
        Edit::PasteValue {
            from: c1(),
            from_node: "n3".into(),
            to: c1(),
            to_node: "n4".into(),
            param: "primaries.brightness".into(),
        },
    );
    assert_eq!(error, EditError::Op(OpError::OutOfRange { what: "param" }));
    applied(&mut project, add(Node::new("", Kind::KeyMixer), vec![]));
    let error = refused_edit(
        &project,
        Edit::PasteNode {
            from: c1(),
            from_node: "n3".into(),
            to: c1(),
            to_node: "n5".into(),
        },
    );
    assert_eq!(
        error,
        EditError::Op(OpError::WrongNode {
            node: "n5".into(),
            kind: "key_mixer"
        })
    );
}

#[test]
fn an_edit_of_grades_is_written_as_the_protocol_names_it() {
    let edit = Edit::PasteValue {
        from: c1(),
        from_node: "n3".into(),
        to: Target::PostClip { group: "k5".into() },
        to_node: "n1".into(),
        param: "primaries.lift".into(),
    };
    assert_eq!(
        serde_json::to_value(&edit).unwrap(),
        json!({
            "edit": "paste_value",
            "from": {"level": "clip", "timeline": "t1", "clip": "c1"},
            "from_node": "n3",
            "to": {"level": "post_clip", "group": "k5"},
            "to_node": "n1",
            "param": "primaries.lift",
        })
    );
    let read: Edit = serde_json::from_value(json!({
        "edit": "add_serial", "grade": {"level": "timeline", "timeline": "t1"}, "node": "source",
    }))
    .unwrap();
    assert_eq!(
        read,
        Edit::AddSerial {
            grade: Target::Timeline {
                timeline: "t1".into()
            },
            node: "source".into()
        }
    );
}

#[test]
fn a_grade_is_kept_on_both_halves_of_a_clip_that_is_bladed() {
    let mut project = serial();
    let edit = Edit::Blade {
        timeline: "t1".into(),
        at: 40,
        tracks: Some(vec!["V1".into()]),
        linked: false,
    };
    expand(&mut project, edit).unwrap();
    let clips = &project.timelines[0].tracks[0].clips;
    assert_eq!(clips.len(), 2);
    assert_eq!(clips[0].grade, clips[1].grade);
    assert_eq!(clips[1].grade.nodes.len(), 2);
}
