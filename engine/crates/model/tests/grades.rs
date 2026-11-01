//! The operations and edits of grades, one by one: what each does, what it gives back, and what
//! it refuses.

use columbina_model::grade::{
    Composite, Interp, Key, Kind, Link, Node, NodeKind, Param, Port, Pos,
};
use columbina_model::{
    apply, apply_done, expand, Edit, EditError, Given, Grade, Group, Op, OpError, Project, Target,
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
            locked: None,
            pos: None,
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
            locked: None,
            pos: None,
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
            locked: None,
            pos: None,
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
        locked: None,
        pos: None,
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
        locked: None,
        pos: None,
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
        locked: None,
        pos: None,
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
            locked: None,
            pos: None,
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
            locked: None,
            pos: None,
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
fn copy_grade_puts_a_grade_in_place_of_others_and_append_node_graph_after_them() {
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
            preserve_nodes: None,
            keyframes: None,
        },
    );
    assert_eq!(done.len(), 2);
    assert_eq!(grade(&project, &c2), grade(&project, &c1()));
    assert_eq!(grade(&project, &timeline), grade(&project, &c1()));
    edited(
        &mut project,
        Edit::AppendNodeGraph {
            from: c1(),
            to: vec![c2.clone()],
            keyframes: None,
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
            preserve_nodes: None,
            keyframes: None,
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
            locked: None,
            pos: None,
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

// ---------------------------------------------------------------------------------------------
// Copy Grade: Preserve number of nodes; Append Node Graph; Collapse Group Grades; Shot Match
// ---------------------------------------------------------------------------------------------

fn c2() -> Target {
    Target::Clip {
        timeline: "t1".into(),
        clip: "c2".into(),
    }
}

/// A corrector of id `id` whose saturation is `saturation`, to be told apart.
fn saturated(id: &str, saturation: f64) -> Node {
    let mut node = Node::corrector(id);
    let NodeKind::Corrector(c) = &mut node.kind else {
        unreachable!()
    };
    c.primaries.saturation = Param::Fixed(saturation);
    node
}

/// A grade of `nodes` in a row from the source to the output.
fn in_a_row(nodes: Vec<Node>) -> Grade {
    let mut ids = vec!["source".to_string()];
    ids.extend(nodes.iter().map(|n| n.id.clone()));
    ids.push("output".into());
    let mut grade = Grade {
        nodes,
        links: ids.windows(2).map(|w| rgb(&w[0], &w[1], 0)).collect(),
    };
    grade.sort_links();
    grade
}

fn copy(to: Vec<Target>, preserve_nodes: Option<u32>) -> Edit {
    Edit::CopyGrade {
        from: c1(),
        to,
        preserve_nodes,
        keyframes: None,
    }
}

/// The project of `serial` (c1: n3 then n4) with c2 graded by `nodes` in a row.
fn with_c2(nodes: Vec<Node>) -> Project {
    let mut project = serial();
    applied(
        &mut project,
        Op::SetGrade {
            grade: c2(),
            value: in_a_row(nodes),
        },
    );
    project
}

#[test]
fn copy_grade_keeps_the_first_nodes_of_the_grades_it_is_put_in_place_of() {
    // c2: three nodes of its own; c1, copied: n3 then n4.
    let mine = || {
        vec![
            saturated("a1", 10.0),
            saturated("a2", 20.0),
            saturated("a3", 30.0),
        ]
    };
    let mut project = with_c2(mine());
    let done = edited(&mut project, copy(vec![c2()], Some(1)));
    assert_eq!(done.len(), 1);
    let g = grade(&project, &c2());
    let c1_grade = grade(&project, &c1());
    // "The first node of the copied grade is ignored, but all other nodes are copied" (p. 3285).
    assert_eq!(g.nodes, vec![mine()[0].clone(), c1_grade.nodes[1].clone()]);
    assert_eq!(
        g.links,
        vec![
            rgb("source", "a1", 0),
            rgb("a1", "n4", 0),
            rgb("n4", "output", 0)
        ]
    );
    assert!(g.is_complete());

    // Two kept, and the copy has no more: the two in the links of the copy.
    let mut project = with_c2(mine());
    edited(&mut project, copy(vec![c2()], Some(2)));
    let g = grade(&project, &c2());
    assert_eq!(g, in_a_row(mine()[..2].to_vec()));

    // A grade with fewer nodes than are kept is put in place whole, ids and all.
    let mut project = with_c2(mine());
    let timeline = Target::Timeline {
        timeline: "t1".into(),
    };
    edited(&mut project, copy(vec![timeline.clone()], Some(3)));
    assert_eq!(grade(&project, &timeline), grade(&project, &c1()));
    // c2 has three, which the copy, of two, has no places for.
    let mut project = with_c2(mine());
    let error = refused_edit(&project, copy(vec![c2()], Some(3)));
    assert_eq!(
        error,
        EditError::Op(OpError::OutOfRange {
            what: "preserve_nodes"
        })
    );
    // 0 is the whole copy, as no number is.
    edited(&mut project, copy(vec![c2()], Some(0)));
    assert_eq!(grade(&project, &c2()), grade(&project, &c1()));
}

#[test]
fn copy_grade_gives_a_node_of_the_copy_a_new_id_where_a_node_kept_holds_its_own() {
    // c2's first node is n4, as the second of c1, copied, is.
    let mut project = with_c2(vec![saturated("n4", 10.0), saturated("x", 20.0)]);
    let next = project.next_id;
    edited(&mut project, copy(vec![c2()], Some(1)));
    let g = grade(&project, &c2());
    let fresh = format!("n{next}");
    let ids: Vec<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, ["n4", fresh.as_str()]);
    assert_eq!(
        g.nodes[0].as_corrector().unwrap().primaries.saturation,
        Param::Fixed(10.0)
    );
    assert_eq!(
        g.links,
        vec![
            rgb("source", "n4", 0),
            rgb("n4", fresh.as_str(), 0),
            rgb(fresh.as_str(), "output", 0),
        ]
    );
    assert!(g.is_complete());
}

#[test]
fn copy_grade_refuses_to_keep_a_node_in_the_place_of_one_of_another_kind_or_too_many() {
    let mut mixer = Node::new("m", Kind::ParallelMixer);
    mixer.label = "Mix".into();
    let mut project = with_c2(vec![]);
    let value = Grade {
        nodes: vec![mixer, Node::corrector("a")],
        links: vec![
            rgb("source", "a", 0),
            rgb("a", "m", 0),
            rgb("source", "m", 1),
            rgb("m", "output", 0),
        ],
    };
    applied(&mut project, Op::SetGrade { grade: c2(), value });
    let error = refused_edit(&project, copy(vec![c2()], Some(1)));
    assert_eq!(
        error,
        EditError::Op(OpError::WrongNode {
            node: "m".into(),
            kind: "parallel_mixer"
        })
    );
    // A mixer of as many inputs is kept, of another number is not.
    let error = refused_edit(&project, copy(vec![c2()], Some(11)));
    assert_eq!(
        error,
        EditError::Op(OpError::OutOfRange {
            what: "preserve_nodes"
        })
    );
    let mut mixers = Project::sample();
    for (target, inputs) in [(c1(), 2), (c2(), 3)] {
        let mut m = Node::new("m", Kind::ParallelMixer);
        m.kind = NodeKind::from_params(Kind::ParallelMixer, json!({ "inputs": inputs })).unwrap();
        let mut links = vec![rgb("m", "output", 0)];
        links.extend((0..inputs).map(|i| rgb("source", "m", i)));
        let mut value = Grade {
            nodes: vec![m],
            links,
        };
        value.sort_links();
        applied(
            &mut mixers,
            Op::SetGrade {
                grade: target,
                value,
            },
        );
    }
    let error = refused_edit(&mixers, copy(vec![c2()], Some(1)));
    assert_eq!(
        error,
        EditError::Op(OpError::WrongNode {
            node: "m".into(),
            kind: "parallel_mixer"
        })
    );
}

#[test]
fn preserve_nodes_is_written_only_where_it_is_given() {
    let edit = copy(vec![c2()], None);
    let written = serde_json::to_value(&edit).unwrap();
    assert!(written.get("preserve_nodes").is_none(), "{written}");
    let read: Edit = serde_json::from_value(json!({
        "edit": "copy_grade",
        "from": {"level": "clip", "timeline": "t1", "clip": "c1"},
        "to": [{"level": "clip", "timeline": "t1", "clip": "c2"}],
        "preserve_nodes": 2,
    }))
    .unwrap();
    assert_eq!(read, copy(vec![c2()], Some(2)));
    assert_eq!(serde_json::to_value(&read).unwrap()["preserve_nodes"], 2);
}

#[test]
fn append_node_graph_is_written_by_its_name_and_read_by_its_old_one_too() {
    let fields = json!({
        "from": {"level": "clip", "timeline": "t1", "clip": "c1"},
        "to": [{"level": "timeline", "timeline": "t1"}],
    });
    let edit = Edit::AppendNodeGraph {
        from: c1(),
        to: vec![Target::Timeline {
            timeline: "t1".into(),
        }],
        keyframes: None,
    };
    for name in ["append_node_graph", "append_grade"] {
        let mut value = fields.clone();
        value["edit"] = name.into();
        let read: Edit = serde_json::from_value(value).unwrap();
        assert_eq!(read, edit);
    }
    assert_eq!(
        serde_json::to_value(&edit).unwrap()["edit"],
        "append_node_graph"
    );
}

/// The grade of c1 given whole, as a still holds it, its keys where they are.
#[allow(clippy::unnecessary_box_returns, reason = "the edits take it boxed")]
fn given(grade: Grade) -> Box<Given> {
    Box::new(Given {
        grade,
        keys: None,
        align: None,
    })
}

#[test]
fn a_grade_given_whole_is_copied_and_appended_as_one_of_the_project_is() {
    // Of the project's own grade: the same operations as Copy Grade and Append Node Graph.
    for preserve in [0, 1] {
        let mut by_target = with_c2(vec![saturated("a1", 10.0), saturated("a2", 20.0)]);
        let mut by_given = by_target.clone();
        let c1_grade = grade(&by_target, &c1());
        let done = edited(&mut by_target, copy(vec![c2()], Some(preserve)));
        let given_done = edited(
            &mut by_given,
            Edit::CopyGivenGrade {
                given: given(c1_grade.clone()),
                to: vec![c2()],
                preserve_nodes: preserve,
            },
        );
        assert_eq!(given_done, done, "preserve {preserve}");
        assert_eq!(by_given, by_target);
    }
    let mut by_target = with_c2(vec![saturated("a1", 10.0)]);
    let mut by_given = by_target.clone();
    let c1_grade = grade(&by_target, &c1());
    let done = edited(
        &mut by_target,
        Edit::AppendNodeGraph {
            from: c1(),
            to: vec![c2()],
            keyframes: None,
        },
    );
    let given_done = edited(
        &mut by_given,
        Edit::AppendGivenGraph {
            given: given(c1_grade),
            to: vec![c2()],
        },
    );
    assert_eq!(given_done, done);
    assert_eq!(by_given, by_target);
}

#[test]
fn a_grade_of_another_project_takes_ids_of_the_engine_and_lies_right_of_the_nodes_before() {
    // Ids that this project has not made yet (n90, n91) and one of no shape of the engine's.
    let mut project = with_c2(vec![saturated("a1", 10.0)]);
    let next = project.next_id;
    assert!(next < 90);
    let mut far = saturated("n90", 30.0);
    far.pos = Some(Pos { x: 0.0, y: 40.0 });
    let mut farther = saturated("n91", 40.0);
    farther.pos = Some(Pos { x: 120.0, y: 40.0 });
    let other = in_a_row(vec![far, saturated("Look", 50.0), farther]);
    edited(
        &mut project,
        Edit::CopyGivenGrade {
            given: given(other.clone()),
            to: vec![c2()],
            preserve_nodes: 0,
        },
    );
    let ids: Vec<String> = grade(&project, &c2())
        .nodes
        .iter()
        .map(|n| n.id.clone())
        .collect();
    assert_eq!(
        ids,
        [format!("n{next}"), "Look".into(), format!("n{}", next + 1)]
    );
    assert!(grade(&project, &c2()).is_complete());

    // Appended after c2's own node, which lies at 100: new ids of the engine, and the leftmost
    // a column (NODE_STEP) right of it, the rest as far apart as they were.
    let mut project = with_c2(vec![{
        let mut own = saturated("a1", 10.0);
        own.pos = Some(Pos { x: 100.0, y: 0.0 });
        own
    }]);
    let next = project.next_id;
    edited(
        &mut project,
        Edit::AppendGivenGraph {
            given: given(other),
            to: vec![c2()],
        },
    );
    let g = grade(&project, &c2());
    let ids: Vec<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
    let fresh: Vec<String> = (next..next + 3).map(|n| format!("n{n}")).collect();
    assert_eq!(ids, ["a1", &fresh[0], &fresh[1], &fresh[2]]);
    let xs: Vec<Option<f64>> = g.nodes.iter().map(|n| n.pos.map(|p| p.x)).collect();
    let step = columbina_model::grade::NODE_STEP;
    assert_eq!(
        xs,
        [Some(100.0), Some(100.0 + step), None, Some(220.0 + step)]
    );
    assert!(g.is_complete());
}

/// The project of `serial` with a group, c1 in it, a node in its
/// pre-clip grade and two in its post-clip grade, all in a row.
fn grouped() -> (Project, String) {
    let mut project = serial();
    applied(
        &mut project,
        Op::AddGroup {
            group: Group::new("", "Interview"),
            index: None,
        },
    );
    let id = project.groups[0].id.clone();
    applied(
        &mut project,
        Op::SetClipGroup {
            timeline: "t1".into(),
            clip: "c1".into(),
            group: Some(id.clone()),
        },
    );
    let pre = Target::PreClip { group: id.clone() };
    let post = Target::PostClip { group: id.clone() };
    applied(
        &mut project,
        Op::SetGrade {
            grade: pre,
            value: in_a_row(vec![saturated("n3", 40.0)]),
        },
    );
    applied(
        &mut project,
        Op::SetGrade {
            grade: post,
            value: in_a_row(vec![saturated("p1", 60.0), saturated("p2", 70.0)]),
        },
    );
    (project, id)
}

#[test]
fn collapse_group_grades_puts_the_grades_of_the_group_about_the_clip_s_own() {
    let (mut project, group) = grouped();
    let own = grade(&project, &c1());
    let next = project.next_id;
    let done = edited(
        &mut project,
        Edit::CollapseGroupGrades {
            timeline: "t1".into(),
            clips: vec!["c1".into()],
        },
    );
    assert_eq!(done.len(), 2);
    let g = grade(&project, &c1());
    let pre = format!("n{next}");
    let (post1, post2) = (format!("n{}", next + 1), format!("n{}", next + 2));
    let ids: Vec<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
    // Pre-clip nodes "added before any pre-existing nodes in the Clip grade, and all
    // Post-Group nodes … after" (p. 3306); the clip's own keep their ids.
    assert_eq!(ids, [pre.as_str(), "n3", "n4", &post1, &post2]);
    assert_eq!(g.nodes[1..3], own.nodes[..]);
    let saturation = |i: usize| {
        g.nodes[i]
            .as_corrector()
            .unwrap()
            .primaries
            .saturation
            .clone()
    };
    assert_eq!(saturation(0), Param::Fixed(40.0));
    assert_eq!(saturation(3), Param::Fixed(60.0));
    assert_eq!(saturation(4), Param::Fixed(70.0));
    let mut expected = in_a_row(g.nodes.clone());
    expected.sort_links();
    assert_eq!(g, expected);
    assert!(g.is_complete());
    // Out of the group, which is still there with its grades.
    let clip = &project.timelines[0].tracks[0].clips[0];
    assert_eq!(clip.group, None);
    let k = project.groups.iter().find(|k| k.id == group).unwrap();
    assert_eq!(k.pre.nodes.len(), 1);
    assert_eq!(k.post.nodes.len(), 2);
    assert_eq!(
        done[1],
        Op::SetClipGroup {
            timeline: "t1".into(),
            clip: "c1".into(),
            group: None
        }
    );
}

#[test]
fn collapse_group_grades_of_a_group_without_nodes_only_takes_the_clip_out() {
    let mut project = serial();
    applied(
        &mut project,
        Op::AddGroup {
            group: Group::new("empty", "Empty"),
            index: None,
        },
    );
    applied(
        &mut project,
        Op::SetClipGroup {
            timeline: "t1".into(),
            clip: "c2".into(),
            group: Some("empty".into()),
        },
    );
    let own = grade(&project, &c2());
    let done = edited(
        &mut project,
        Edit::CollapseGroupGrades {
            timeline: "t1".into(),
            clips: vec!["c2".into()],
        },
    );
    assert_eq!(done.len(), 1);
    assert_eq!(grade(&project, &c2()), own);
}

#[test]
fn collapse_group_grades_refuses_a_clip_in_no_group_or_none() {
    let (project, _) = grouped();
    let collapse = |clips: Vec<&str>| Edit::CollapseGroupGrades {
        timeline: "t1".into(),
        clips: clips.into_iter().map(Into::into).collect(),
    };
    assert_eq!(
        refused_edit(&project, collapse(vec![])),
        EditError::Op(OpError::OutOfRange { what: "clips" })
    );
    assert_eq!(
        refused_edit(&project, collapse(vec!["c1", "c2"])),
        EditError::Op(OpError::Ungrouped { clip: "c2".into() })
    );
    // Twice: the second time it is in no group.
    assert_eq!(
        refused_edit(&project, collapse(vec!["c1", "c1"])),
        EditError::Op(OpError::Ungrouped { clip: "c1".into() })
    );
    assert!(matches!(
        refused_edit(&project, collapse(vec!["c9"])),
        EditError::Op(OpError::Missing { what: "clip", .. })
    ));
    let written = serde_json::to_value(OpError::Ungrouped { clip: "c2".into() }).unwrap();
    assert_eq!(written["reason"], "ungrouped");
    assert_eq!(written["clip"], "c2");
}

fn set_params(params: serde_json::Value) -> Op {
    Op::SetNode {
        grade: c1(),
        id: "n3".into(),
        label: None,
        enabled: None,
        locked: None,
        pos: None,
        params: Some(params),
    }
}

fn shot_match_of(project: &Project) -> Option<columbina_model::ShotMatch> {
    grade(project, &c1()).nodes[0]
        .as_corrector()
        .unwrap()
        .shot_match
        .clone()
}

#[test]
fn a_shot_match_is_set_whole_patched_and_taken_away_by_set_node() {
    let mut project = serial();
    assert_eq!(shot_match_of(&project), None);
    let node = serde_json::to_value(&grade(&project, &c1()).nodes[0]).unwrap();
    assert_eq!(node["params"]["shot_match"], serde_json::Value::Null);
    let whole = json!({"shot_match": {
        "gamma": {"r": 0.9, "g": 1, "b": 1.1},
        "gain": {"r": 1.2, "g": 1, "b": 0.8},
        "offset": {"r": 0.01, "g": 0, "b": -0.02},
        "contrast": 1.1,
        "pivot": 0.4,
    }});
    let (done, inverse) = applied(&mut project, set_params(whole));
    assert_eq!(inverse, set_params(json!({"shot_match": null})));
    let Op::SetNode {
        params: Some(done), ..
    } = done
    else {
        unreachable!()
    };
    assert_eq!(done["shot_match"]["gain"]["r"], 1.2);
    assert_eq!(done["shot_match"]["pivot"], 0.4);
    let set = shot_match_of(&project).unwrap();
    assert_eq!(set.gain.r, Param::Fixed(1.2));
    assert_eq!(set.contrast, Param::Fixed(1.1));
    // In part where it is there.
    let (_, inverse) = applied(
        &mut project,
        set_params(json!({"shot_match": {"gain": {"g": 1.05}}})),
    );
    assert_eq!(
        inverse,
        set_params(json!({"shot_match": {"gain": {"g": 1.0}}}))
    );
    assert_eq!(shot_match_of(&project).unwrap().gain.g, Param::Fixed(1.05));
    // Reset with the node.
    edited(
        &mut project,
        Edit::ResetNode {
            grade: c1(),
            node: "n3".into(),
        },
    );
    assert_eq!(shot_match_of(&project), None);
}

#[test]
fn a_shot_match_is_refused_in_part_where_there_is_none_and_out_of_its_bounds() {
    let project = serial();
    let mut copy = project.clone();
    let error = apply(
        &mut copy,
        set_params(json!({"shot_match": {"gain": {"r": 1.1}}})),
    )
    .unwrap_err();
    assert!(matches!(error, OpError::Params { .. }), "{error:?}");
    assert_eq!(copy, project);
    let mut whole = json!({"shot_match": columbina_model::ShotMatch::default()});
    for (path, value, what) in [
        (
            "gamma",
            json!({"r": 1, "g": 0.05, "b": 1}),
            "shot_match.gamma.g",
        ),
        (
            "gain",
            json!({"r": 1, "g": 1, "b": 17}),
            "shot_match.gain.b",
        ),
        (
            "offset",
            json!({"r": -1.5, "g": 0, "b": 0}),
            "shot_match.offset.r",
        ),
        ("contrast", json!(4.5), "shot_match.contrast"),
        ("pivot", json!(-0.1), "shot_match.pivot"),
    ] {
        let mut bad = whole.clone();
        bad["shot_match"][path] = value;
        let mut copy = project.clone();
        let error = apply(&mut copy, set_params(bad)).unwrap_err();
        assert_eq!(error, OpError::OutOfRange { what }, "{path}");
    }
    whole["shot_match"]["bogus"] = json!(1);
    assert!(matches!(
        apply(&mut project.clone(), set_params(whole)).unwrap_err(),
        OpError::Params { .. }
    ));
}

#[test]
fn a_corrector_written_without_a_shot_match_is_read_as_having_none() {
    let node = Node::corrector("n1");
    let mut written = node.kind.params();
    written.as_object_mut().unwrap().remove("shot_match");
    let read = NodeKind::from_params(Kind::Corrector, written).unwrap();
    assert_eq!(read, node.kind);
    // The neutral settings are those of no shot match.
    let neutral = columbina_model::ShotMatch::default();
    assert_eq!(neutral.gamma.r, Param::Fixed(1.0));
    assert_eq!(neutral.gain.b, Param::Fixed(1.0));
    assert_eq!(neutral.offset.g, Param::Fixed(0.0));
    assert_eq!(neutral.contrast, Param::Fixed(1.0));
}

// ---------------------------------------------------------------------------------------------
// Lock Node and where nodes lie
// ---------------------------------------------------------------------------------------------

/// Set n3 of c1 as given.
#[allow(clippy::option_option, reason = "left out and null are told apart")]
fn set_n3(locked: Option<bool>, pos: Option<Option<Pos>>, params: Option<serde_json::Value>) -> Op {
    Op::SetNode {
        grade: c1(),
        id: "n3".into(),
        label: None,
        enabled: None,
        locked,
        pos,
        params,
    }
}

fn at(x: f64, y: f64) -> Pos {
    Pos { x, y }
}

/// `serial` with n3 locked.
fn locked_n3() -> Project {
    let mut project = serial();
    applied(&mut project, set_n3(Some(true), None, None));
    project
}

#[test]
fn a_locked_node_refuses_its_settings_but_takes_its_label_its_switch_and_its_place() {
    let mut project = locked_n3();
    assert!(grade(&project, &c1()).nodes[0].locked);
    let saturation = json!({"primaries": {"saturation": 20}});
    let error = refused(&project, set_n3(None, None, Some(saturation.clone())));
    assert_eq!(error, OpError::NodeLocked { node: "n3".into() });
    // Its settings left as they are are refused too: a patch is a change asked for.
    assert_eq!(
        refused(&project, set_n3(Some(true), None, Some(json!({})))),
        OpError::NodeLocked { node: "n3".into() }
    );
    // "Lets you prevent any changes from being made to that track's corresponding node"
    // (p. 3382): to its settings. Its label, whether it is enabled and where it lies are not
    // settings of it, as the button that enables it is beside the lock (p. 3382).
    let (done, inverse) = applied(
        &mut project,
        Op::SetNode {
            grade: c1(),
            id: "n3".into(),
            label: Some("Base".into()),
            enabled: Some(false),
            locked: None,
            pos: Some(Some(at(300.0, 80.0))),
            params: None,
        },
    );
    assert_eq!(
        inverse,
        Op::SetNode {
            grade: c1(),
            id: "n3".into(),
            label: Some(String::new()),
            enabled: Some(true),
            locked: None,
            pos: Some(None),
            params: None,
        }
    );
    assert_eq!(
        serde_json::to_value(&done).unwrap(),
        json!({"op": "set_node", "grade": {"level": "clip", "timeline": "t1", "clip": "c1"},
               "id": "n3", "label": "Base", "enabled": false, "pos": {"x": 300.0, "y": 80.0}})
    );
    // Unlocked and set in one change; and its inverse sets it back and locks it, which is not
    // refused, as the node is unlocked before it.
    let (_, inverse) = applied(&mut project, set_n3(Some(false), None, Some(saturation)));
    assert_eq!(
        inverse,
        set_n3(
            Some(true),
            None,
            Some(json!({"primaries": {"saturation": 50.0}}))
        )
    );
    // Set and locked in one change.
    applied(
        &mut project,
        set_n3(Some(true), None, Some(json!({"primaries": {"hue": 60}}))),
    );
    assert!(grade(&project, &c1()).nodes[0].locked);
}

#[test]
fn the_edits_that_set_a_locked_node_are_refused_and_the_others_are_not() {
    let project = locked_n3();
    let locked = EditError::Op(OpError::NodeLocked { node: "n3".into() });
    let reset = Edit::ResetNode {
        grade: c1(),
        node: "n3".into(),
    };
    assert_eq!(refused_edit(&project, reset), locked);
    let paste_value = Edit::PasteValue {
        from: c1(),
        from_node: "n4".into(),
        to: c1(),
        to_node: "n3".into(),
        param: "primaries.saturation".into(),
    };
    assert_eq!(refused_edit(&project, paste_value), locked);
    let paste_node = Edit::PasteNode {
        from: c1(),
        from_node: "n4".into(),
        to: c1(),
        to_node: "n3".into(),
    };
    assert_eq!(refused_edit(&project, paste_node), locked);
    // Its settings are copied from it, and nodes are added beside it.
    let mut project = project;
    edited(
        &mut project,
        Edit::PasteNode {
            from: c1(),
            from_node: "n3".into(),
            to: c1(),
            to_node: "n4".into(),
        },
    );
    edited(
        &mut project,
        Edit::AddSerial {
            grade: c1(),
            node: "n3".into(),
        },
    );
    // It is taken out of its tree, and away, with its lock and its place.
    applied(&mut project, set_n3(None, Some(Some(at(10.0, 20.0))), None));
    edited(
        &mut project,
        Edit::ExtractNode {
            grade: c1(),
            node: "n3".into(),
        },
    );
    let n3 = grade(&project, &c1()).nodes[0].clone();
    assert!(n3.locked);
    assert_eq!(n3.pos, Some(at(10.0, 20.0)));
    applied(
        &mut project,
        Op::RemoveNode {
            grade: c1(),
            id: "n3".into(),
            links: None,
        },
    );
}

#[test]
fn a_locked_mixer_is_given_no_input_more() {
    let mut project = serial();
    edited(
        &mut project,
        Edit::AddParallel {
            grade: c1(),
            node: "n4".into(),
        },
    );
    let mixer = grade(&project, &c1())
        .nodes
        .iter()
        .find(|n| n.kind.kind() == Kind::ParallelMixer)
        .unwrap()
        .id
        .clone();
    applied(
        &mut project,
        Op::SetNode {
            grade: c1(),
            id: mixer.clone(),
            label: None,
            enabled: None,
            locked: Some(true),
            pos: None,
            params: None,
        },
    );
    let error = refused_edit(
        &project,
        Edit::AddParallel {
            grade: c1(),
            node: "n4".into(),
        },
    );
    assert_eq!(error, EditError::Op(OpError::NodeLocked { node: mixer }));
}

#[test]
fn a_node_is_put_further_than_a_million_from_0_by_nothing() {
    let project = serial();
    for pos in [at(1.5e6, 0.0), at(0.0, -2.0e6)] {
        assert_eq!(
            refused(&project, set_n3(None, Some(Some(pos)), None)),
            OpError::OutOfRange { what: "pos" }
        );
        let mut node = Node::corrector("");
        node.pos = Some(pos);
        assert_eq!(
            refused(&project, add(node, vec![])),
            OpError::OutOfRange { what: "pos" }
        );
    }
    let mut node = Node::corrector("");
    node.pos = Some(at(-40.0, 1000.0));
    node.locked = true;
    let mut project = project;
    let (done, _) = applied(&mut project, add(node, vec![]));
    let Op::AddNode { node, .. } = done else {
        unreachable!()
    };
    assert_eq!((node.locked, node.pos), (true, Some(at(-40.0, 1000.0))));
}

#[test]
fn the_refusal_of_a_locked_node_is_written_as_the_protocol_names_it() {
    let written = serde_json::to_value(OpError::NodeLocked { node: "n3".into() }).unwrap();
    assert_eq!(
        written,
        json!({"reason": "node_locked", "node": "n3", "message": "node n3 is locked"})
    );
    let op: Op = serde_json::from_value(json!({
        "op": "set_node", "grade": {"level": "clip", "timeline": "t1", "clip": "c1"}, "id": "n3",
        "locked": true, "pos": null,
    }))
    .unwrap();
    assert_eq!(op, set_n3(Some(true), Some(None), None));
    // Left out is left as it is, and so not written.
    let op: Op = serde_json::from_value(json!({
        "op": "set_node", "grade": {"level": "clip", "timeline": "t1", "clip": "c1"}, "id": "n3",
    }))
    .unwrap();
    assert_eq!(op, set_n3(None, None, None));
    assert_eq!(
        serde_json::to_value(&op).unwrap(),
        json!({"op": "set_node", "grade": {"level": "clip", "timeline": "t1", "clip": "c1"},
               "id": "n3"})
    );
}

/// A node `id` that lies at `x`, `y`.
fn placed(id: &str, x: f64, y: f64) -> Node {
    let mut node = saturated(id, 40.0);
    node.pos = Some(at(x, y));
    node
}

#[test]
fn copy_grade_takes_where_its_nodes_lie_and_their_locks_along() {
    let mut project = with_c2(vec![placed("a1", 500.0, 60.0)]);
    let mut from = in_a_row(vec![placed("b1", 102.0, 28.0), placed("b2", 250.0, 28.0)]);
    from.nodes[1].locked = true;
    applied(
        &mut project,
        Op::SetGrade {
            grade: c1(),
            value: from.clone(),
        },
    );
    edited(&mut project, copy(vec![c2()], None));
    assert_eq!(grade(&project, &c2()), from);
    // A node kept by Preserve number of nodes lies where the node it takes the place of lay,
    // and keeps its own lock.
    let mut project = with_c2(vec![placed("a1", 500.0, 60.0)]);
    applied(
        &mut project,
        Op::SetGrade {
            grade: c1(),
            value: from,
        },
    );
    applied(
        &mut project,
        Op::SetNode {
            grade: c2(),
            id: "a1".into(),
            label: None,
            enabled: None,
            locked: Some(true),
            pos: None,
            params: None,
        },
    );
    edited(&mut project, copy(vec![c2()], Some(1)));
    let g = grade(&project, &c2());
    assert_eq!(g.nodes[0].id, "a1");
    assert_eq!(g.nodes[0].pos, Some(at(102.0, 28.0)));
    assert!(g.nodes[0].locked);
    assert_eq!(g.nodes[1].pos, Some(at(250.0, 28.0)));
}

#[test]
fn append_node_graph_puts_the_nodes_it_adds_right_of_those_there() {
    // Nodes that lie somewhere: the leftmost added a column right of the rightmost there.
    let mut project = with_c2(vec![placed("a1", 102.0, 28.0), placed("a2", 400.0, 120.0)]);
    applied(
        &mut project,
        Op::SetGrade {
            grade: c1(),
            value: in_a_row(vec![placed("b1", 50.0, 10.0), placed("b2", 198.0, 10.0)]),
        },
    );
    let append = |project: &mut Project| {
        edited(
            project,
            Edit::AppendNodeGraph {
                from: c1(),
                to: vec![c2()],
                keyframes: None,
            },
        );
    };
    append(&mut project);
    let g = grade(&project, &c2());
    let lie: Vec<Option<Pos>> = g.nodes.iter().map(|n| n.pos).collect();
    assert_eq!(
        lie,
        [
            Some(at(102.0, 28.0)),
            Some(at(400.0, 120.0)),
            Some(at(548.0, 10.0)),
            Some(at(696.0, 10.0)),
        ]
    );
    // Nodes laid out by the editor are taken to be as far right as it could lay them: three
    // nodes, three columns.
    let mut project = with_c2(vec![
        saturated("a1", 1.0),
        saturated("a2", 2.0),
        placed("a3", 120.0, 300.0),
    ]);
    applied(
        &mut project,
        Op::SetGrade {
            grade: c1(),
            value: in_a_row(vec![placed("b1", 102.0, 28.0), saturated("b2", 3.0)]),
        },
    );
    append(&mut project);
    let g = grade(&project, &c2());
    assert_eq!(g.nodes[3].pos, Some(at(102.0 + 3.0 * 148.0, 28.0)));
    assert_eq!(g.nodes[4].pos, None);
    // Into a grade without nodes, they lie where they lay.
    let mut project = with_c2(vec![]);
    applied(
        &mut project,
        Op::SetGrade {
            grade: c1(),
            value: in_a_row(vec![placed("b1", 70.0, 28.0)]),
        },
    );
    append(&mut project);
    assert_eq!(grade(&project, &c2()).nodes[0].pos, Some(at(70.0, 28.0)));
}

#[test]
fn collapse_group_grades_puts_the_clip_s_nodes_right_of_the_pre_clip_ones() {
    let (mut project, group) = grouped();
    let pre = Target::PreClip { group };
    applied(
        &mut project,
        Op::SetGrade {
            grade: pre,
            value: in_a_row(vec![placed("n3", 102.0, 28.0)]),
        },
    );
    applied(
        &mut project,
        set_n3(None, Some(Some(at(102.0, 28.0))), None),
    );
    edited(
        &mut project,
        Edit::CollapseGroupGrades {
            timeline: "t1".into(),
            clips: vec!["c1".into()],
        },
    );
    let g = grade(&project, &c1());
    assert_eq!(g.nodes[0].pos, Some(at(102.0, 28.0)));
    // The clip's n3, right of the pre-clip node; its n4, laid out by the editor, as it was.
    assert_eq!(g.nodes[1].id, "n3");
    assert_eq!(g.nodes[1].pos, Some(at(250.0, 28.0)));
    assert_eq!(g.nodes[2].pos, None);
}

fn corrector_of(project: &Project, node: usize) -> columbina_model::Corrector {
    grade(project, &c1()).nodes[node]
        .as_corrector()
        .unwrap()
        .clone()
}

#[test]
fn a_qualifier_is_set_whole_patched_keyframed_and_taken_away_by_set_node() {
    use columbina_model::grade::{Morph, Qualifier, QualifierMode};
    let mut project = serial();
    assert_eq!(corrector_of(&project, 0).qualifier, None);
    let node = serde_json::to_value(&grade(&project, &c1()).nodes[0]).unwrap();
    assert_eq!(node["params"]["qualifier"], serde_json::Value::Null);
    // As it is first: every pixel, written in full.
    let first = serde_json::to_value(Qualifier::default()).unwrap();
    assert_eq!(
        first,
        json!({
            "mode": "hsl", "invert": false,
            "hue": {"on": true, "center": 50.0, "width": 100.0, "soft": 0.0, "symmetry": 50.0},
            "sat": {"on": true, "low": 0.0, "high": 100.0, "low_soft": 0.0, "high_soft": 0.0},
            "lum": {"on": true, "low": 0.0, "high": 100.0, "low_soft": 0.0, "high_soft": 0.0},
            "red": {"on": true, "low": 0.0, "high": 100.0, "low_soft": 0.0, "high_soft": 0.0},
            "green": {"on": true, "low": 0.0, "high": 100.0, "low_soft": 0.0, "high_soft": 0.0},
            "blue": {"on": true, "low": 0.0, "high": 100.0, "low_soft": 0.0, "high_soft": 0.0},
            "finesse": {
                "pre_filter": 0.0, "clean_black": 0.0, "black_clip": 0.0, "clean_white": 0.0,
                "white_clip": 100.0, "blur_radius": 0.0, "in_out_ratio": 0.0,
                "morph": "shrink", "morph_radius": 0.0,
            },
        })
    );
    let mut whole = first.clone();
    whole["hue"]["center"] = json!(8.5);
    whole["hue"]["width"] = json!(12);
    whole["finesse"]["blur_radius"] = json!(40);
    let (done, inverse) = applied(&mut project, set_params(json!({ "qualifier": whole })));
    assert_eq!(inverse, set_params(json!({"qualifier": null})));
    let Op::SetNode {
        params: Some(done), ..
    } = done
    else {
        unreachable!()
    };
    assert_eq!(done["qualifier"]["hue"]["center"], 8.5);
    let set = corrector_of(&project, 0).qualifier.unwrap();
    assert_eq!(set.mode, QualifierMode::Hsl);
    assert_eq!(set.hue.width, Param::Fixed(12.0));
    assert_eq!(set.finesse.blur_radius, Param::Fixed(40.0));
    // In part where it is there: a mode, a toggle, a morph and a keyframed range.
    let keyed = json!({"keys": [
        {"frame": 0, "value": 20, "interp": "linear"},
        {"frame": 24, "value": 35, "interp": "hold"},
    ]});
    let (_, inverse) = applied(
        &mut project,
        set_params(json!({"qualifier": {
            "mode": "lum", "invert": true, "sat": {"on": false},
            "lum": {"low": keyed.clone()},
            "finesse": {"morph": "closing", "morph_radius": 3},
        }})),
    );
    assert_eq!(
        inverse,
        set_params(json!({"qualifier": {
            "mode": "hsl", "invert": false, "sat": {"on": true},
            "lum": {"low": 0.0},
            "finesse": {"morph": "shrink", "morph_radius": 0.0},
        }}))
    );
    let set = corrector_of(&project, 0).qualifier.unwrap();
    assert_eq!(set.mode, QualifierMode::Lum);
    assert!(set.invert && !set.sat.on);
    assert_eq!(set.finesse.morph, Morph::Closing);
    assert!(set.lum.low.is_animated());
    assert!(grade(&project, &c1()).has_keys());
    // Reset with the node.
    edited(
        &mut project,
        Edit::ResetNode {
            grade: c1(),
            node: "n3".into(),
        },
    );
    assert_eq!(corrector_of(&project, 0).qualifier, None);
}

#[test]
fn a_qualifier_is_refused_in_part_where_there_is_none_and_out_of_its_bounds() {
    use columbina_model::grade::Qualifier;
    let project = serial();
    assert!(matches!(
        refused(
            &project,
            set_params(json!({"qualifier": {"hue": {"center": 10}}}))
        ),
        OpError::Params { .. }
    ));
    let whole = serde_json::to_value(Qualifier::default()).unwrap();
    for (path, value, what) in [
        ("hue.center", json!(101), "qualifier.hue.center"),
        ("hue.width", json!(-1), "qualifier.hue.width"),
        ("hue.soft", json!(100.5), "qualifier.hue.soft"),
        ("hue.symmetry", json!(-0.1), "qualifier.hue.symmetry"),
        ("sat.low", json!(-2), "qualifier.sat.low"),
        ("lum.high", json!(120), "qualifier.lum.high"),
        ("red.low_soft", json!(101), "qualifier.red.low_soft"),
        ("green.high_soft", json!(-1), "qualifier.green.high_soft"),
        ("blue.high", json!(100.01), "qualifier.blue.high"),
        (
            "finesse.pre_filter",
            json!(101),
            "qualifier.finesse.pre_filter",
        ),
        (
            "finesse.clean_black",
            json!(-1),
            "qualifier.finesse.clean_black",
        ),
        (
            "finesse.black_clip",
            json!(101),
            "qualifier.finesse.black_clip",
        ),
        (
            "finesse.clean_white",
            json!(101),
            "qualifier.finesse.clean_white",
        ),
        (
            "finesse.white_clip",
            json!(-5),
            "qualifier.finesse.white_clip",
        ),
        (
            "finesse.blur_radius",
            json!(2001),
            "qualifier.finesse.blur_radius",
        ),
        (
            "finesse.in_out_ratio",
            json!(-101),
            "qualifier.finesse.in_out_ratio",
        ),
        (
            "finesse.morph_radius",
            json!(101),
            "qualifier.finesse.morph_radius",
        ),
    ] {
        let mut bad = whole.clone();
        let (group, name) = path.split_once('.').unwrap();
        bad[group][name] = value;
        assert_eq!(
            refused(&project, set_params(json!({ "qualifier": bad }))),
            OpError::OutOfRange { what },
            "{path}"
        );
    }
    for (path, value) in [
        ("mode", json!("3d")),
        ("finesse", json!({"morph": "erode"})),
        ("hue", json!({"on": 1})),
    ] {
        let mut bad = whole.clone();
        bad[path] = value;
        assert!(
            matches!(
                refused(&project, set_params(json!({ "qualifier": bad }))),
                OpError::Params { .. }
            ),
            "{path}"
        );
    }
    let mut bogus = whole;
    bogus["denoise"] = json!(1);
    assert!(matches!(
        refused(&project, set_params(json!({ "qualifier": bogus }))),
        OpError::Params { .. }
    ));
}

#[test]
fn the_key_palette_sets_the_key_of_the_qualifier_and_the_blur_of_the_key_coming_in() {
    let mut project = serial();
    let key = corrector_of(&project, 0).key;
    assert!(key.qualifier.matte && !key.qualifier.invert);
    assert_eq!(key.qualifier.gain, Param::Fixed(1.0));
    assert_eq!(key.qualifier.offset, Param::Fixed(0.0));
    assert_eq!(key.input_blur.radius, Param::Fixed(0.0));
    assert_eq!(key.input_blur.ratio, Param::Fixed(0.5));
    let written = serde_json::to_value(&key).unwrap();
    assert_eq!(
        written["qualifier"],
        json!({"invert": false, "matte": true, "gain": 1.0, "offset": 0.0})
    );
    assert_eq!(written["input_blur"], json!({"radius": 0.0, "ratio": 0.5}));
    let (_, inverse) = applied(
        &mut project,
        set_params(json!({"key": {
            "qualifier": {"invert": true, "matte": false, "gain": 0.5, "offset": 0.2},
            "input_blur": {"radius": 120, "ratio": 0.8},
        }})),
    );
    assert_eq!(
        inverse,
        set_params(json!({"key": {
            "qualifier": {"invert": false, "matte": true, "gain": 1.0, "offset": 0.0},
            "input_blur": {"radius": 0.0, "ratio": 0.5},
        }}))
    );
    for (patch, what) in [
        (json!({"qualifier": {"gain": 2.5}}), "key.qualifier.gain"),
        (
            json!({"qualifier": {"offset": -1.5}}),
            "key.qualifier.offset",
        ),
        (
            json!({"input_blur": {"radius": 2500}}),
            "key.input_blur.radius",
        ),
        (
            json!({"input_blur": {"ratio": 1.5}}),
            "key.input_blur.ratio",
        ),
    ] {
        assert_eq!(
            refused(&project, set_params(json!({ "key": patch }))),
            OpError::OutOfRange { what }
        );
    }
}

#[test]
fn a_corrector_written_before_qualifiers_is_read_without_one_and_with_its_key_as_at_first() {
    let node = Node::corrector("n1");
    let mut written = node.kind.params();
    let params = written.as_object_mut().unwrap();
    params.remove("qualifier");
    let key = params["key"].as_object_mut().unwrap();
    key.remove("qualifier");
    key.remove("input_blur");
    let read = NodeKind::from_params(Kind::Corrector, written).unwrap();
    assert_eq!(read, node.kind);
}

#[test]
fn add_outside_puts_a_corrector_after_the_chosen_one_on_its_picture_and_its_key_inverted() {
    let mut project = serial();
    let done = edited(
        &mut project,
        Edit::AddOutside {
            grade: c1(),
            node: "n3".into(),
        },
    );
    assert_eq!(done.len(), 1);
    // After n3, numbered last (p. 3318), on the picture and the key of n3 (p. 3342).
    assert_eq!(ids(&project), ["n3", "n4", "n5"]);
    let g = grade(&project, &c1());
    assert_eq!(
        g.links,
        vec![
            rgb("source", "n3", 0),
            rgb("n5", "n4", 0),
            rgb("n3", "n5", 0),
            key("n3", "n5", 0),
            rgb("n4", "output", 0)
        ]
    );
    assert!(g.is_complete());
    let outside = corrector_of(&project, 2);
    assert!(outside.key.input.invert, "Key Input Invert is on (p. 3341)");
    let mut plain = columbina_model::Corrector::default();
    plain.key.input.invert = true;
    assert_eq!(outside, plain);
    // At the end of the tree: it takes the output.
    edited(
        &mut project,
        Edit::AddOutside {
            grade: c1(),
            node: "n4".into(),
        },
    );
    let g = grade(&project, &c1());
    assert_eq!(
        g.link_into("output", Port::Rgb, 0),
        Some(&rgb("n6", "output", 0))
    );
    assert_eq!(g.link_into("n6", Port::Key, 0), Some(&key("n4", "n6", 0)));
    // Only after a corrector, which has a key to give.
    for (node, kind) in [("source", "source"), ("output", "output")] {
        assert_eq!(
            refused_edit(
                &project,
                Edit::AddOutside {
                    grade: c1(),
                    node: node.into(),
                },
            ),
            EditError::Op(OpError::WrongNode {
                node: node.into(),
                kind,
            })
        );
    }
    assert!(matches!(
        refused_edit(
            &project,
            Edit::AddOutside {
                grade: c1(),
                node: "n9".into(),
            },
        ),
        EditError::Op(OpError::Missing { what: "node", .. })
    ));
    let mut beside = serial();
    edited(
        &mut beside,
        Edit::AddParallel {
            grade: c1(),
            node: "n3".into(),
        },
    );
    let mixer = grade(&beside, &c1())
        .nodes
        .iter()
        .find(|n| n.kind.kind() == Kind::ParallelMixer)
        .unwrap()
        .id
        .clone();
    assert_eq!(
        refused_edit(
            &beside,
            Edit::AddOutside {
                grade: c1(),
                node: mixer.clone(),
            },
        ),
        EditError::Op(OpError::WrongNode {
            node: mixer,
            kind: "parallel_mixer",
        })
    );
    let written = serde_json::to_value(Edit::AddOutside {
        grade: c1(),
        node: "n3".into(),
    })
    .unwrap();
    assert_eq!(written["edit"], "add_outside");
    assert_eq!(written["node"], "n3");
}
