//! The operations that set and splice the keys of one setting of a node, by its path
//! ([`Op::SetKeys`], [`Op::SpliceKeys`]): what they change, what takes them back, and what they
//! refuse.

use columbina_model::grade::{
    Interp, Key, Kind, Link, Node, Param, Qualifier, Window, WindowShape,
};
use columbina_model::{apply, apply_done, Change, Grade, History, Op, OpError, Project, Target};

fn c1() -> Target {
    Target::Clip {
        timeline: "t1".into(),
        clip: "c1".into(),
    }
}

fn grade(project: &Project) -> &Grade {
    &project.timelines[0]
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
        .find(|clip| clip.id == "c1")
        .expect("the clip is there")
        .grade
}

/// The sample with a node of a kind in the grade of c1, and the id of the node.
fn with_node(kind: Kind) -> (Project, String) {
    let mut project = Project::sample();
    let links = if kind == Kind::Corrector {
        vec![Link::rgb("source", ""), Link::rgb("", "output")]
    } else {
        Vec::new()
    };
    let added = Op::AddNode {
        grade: c1(),
        node: Node::new("", kind),
        index: None,
        links,
    };
    apply(&mut project, added).unwrap();
    let id = grade(&project).nodes[0].id.clone();
    (project, id)
}

/// The setting at a path of the first node of the grade of c1.
fn setting(project: &Project, path: &str) -> Param {
    let mut at = &grade(project).nodes[0].kind.params();
    for part in path.split('.') {
        at = match part.parse::<usize>() {
            Ok(place) => &at[place],
            Err(_) => &at[part],
        };
    }
    serde_json::from_value(at.clone()).unwrap()
}

fn key(frame: i64, value: f64) -> Key {
    Key {
        frame,
        value,
        interp: Interp::Linear,
    }
}

fn set(id: &str, path: &str, param: Param) -> Op {
    Op::SetKeys {
        grade: c1(),
        id: id.into(),
        path: path.into(),
        param,
    }
}

fn splice(id: &str, path: &str, at: usize, remove: usize, keys: Vec<Key>) -> Op {
    Op::SpliceKeys {
        grade: c1(),
        id: id.into(),
        path: path.into(),
        at,
        remove,
        keys,
    }
}

const SATURATION: &str = "primaries.saturation";

#[test]
fn a_setting_is_set_to_keys_or_to_a_number_and_what_it_was_is_put_back() {
    let (mut project, id) = with_node(Kind::Corrector);
    let before = project.clone();
    let keys = Param::Keys(vec![key(0, 50.0), key(24, 80.0)]);

    let inverse = apply(&mut project, set(&id, SATURATION, keys.clone())).unwrap();
    assert_eq!(setting(&project, SATURATION), keys);
    assert_eq!(inverse, set(&id, SATURATION, Param::Fixed(50.0)));
    // Nothing else of the node has changed.
    let mut expected = before.clone();
    let done = set(&id, SATURATION, keys.clone());
    apply(&mut expected, done).unwrap();
    assert_eq!(project, expected);

    let keyed = project.clone();
    let again = apply(&mut project, inverse).unwrap();
    assert_eq!(project, before);
    assert_eq!(again, set(&id, SATURATION, keys));
    apply(&mut project, again).unwrap();
    assert_eq!(project, keyed);
}

#[test]
fn keys_are_spliced_one_for_another_and_the_ones_taken_away_are_put_back() {
    let (mut project, id) = with_node(Kind::Corrector);
    let three = vec![key(0, 50.0), key(24, 80.0), key(48, 20.0)];
    apply(
        &mut project,
        set(&id, SATURATION, Param::Keys(three.clone())),
    )
    .unwrap();
    let before = project.clone();

    // The middle key becomes two.
    let two = vec![key(12, 60.0), key(36, 70.0)];
    let inverse = apply(&mut project, splice(&id, SATURATION, 1, 1, two.clone())).unwrap();
    assert_eq!(
        setting(&project, SATURATION),
        Param::Keys(vec![
            key(0, 50.0),
            key(12, 60.0),
            key(36, 70.0),
            key(48, 20.0)
        ])
    );
    assert_eq!(inverse, splice(&id, SATURATION, 1, 2, vec![key(24, 80.0)]));
    let again = apply(&mut project, inverse).unwrap();
    assert_eq!(project, before);
    assert_eq!(again, splice(&id, SATURATION, 1, 1, two));

    // A key is added at the end, and one is taken away from the beginning.
    apply(
        &mut project,
        splice(&id, SATURATION, 3, 0, vec![key(60, 0.0)]),
    )
    .unwrap();
    let inverse = apply(&mut project, splice(&id, SATURATION, 0, 1, Vec::new())).unwrap();
    assert_eq!(
        setting(&project, SATURATION),
        Param::Keys(vec![key(24, 80.0), key(48, 20.0), key(60, 0.0)])
    );
    assert_eq!(inverse, splice(&id, SATURATION, 0, 0, vec![key(0, 50.0)]));
}

#[test]
fn the_first_keys_of_a_number_take_its_place_and_the_number_comes_back() {
    let (mut project, id) = with_node(Kind::Corrector);
    let before = project.clone();
    let keys = vec![key(10, 0.25), key(20, -0.25)];

    let inverse = apply(
        &mut project,
        splice(&id, "primaries.lift.r", 0, 0, keys.clone()),
    )
    .unwrap();

    assert_eq!(setting(&project, "primaries.lift.r"), Param::Keys(keys));
    // There are no keys to put back: the setting was a number.
    assert_eq!(inverse, set(&id, "primaries.lift.r", Param::Fixed(0.0)));
    apply(&mut project, inverse).unwrap();
    assert_eq!(project, before);
}

#[test]
fn the_settings_of_a_window_a_qualifier_and_a_key_mixer_are_set_by_their_paths() {
    let (mut project, id) = with_node(Kind::Corrector);
    let whole = Op::SetNode {
        grade: c1(),
        id: id.clone(),
        label: None,
        enabled: None,
        locked: None,
        pos: None,
        params: Some(serde_json::json!({
            "qualifier": Qualifier::default(),
            "windows": [Window::new("w1", WindowShape::circular())],
        })),
    };
    apply(&mut project, whole).unwrap();

    let pan = Param::Keys(vec![key(0, 50.0), key(48, 75.0)]);
    apply(
        &mut project,
        set(&id, "windows.0.transform.pan", pan.clone()),
    )
    .unwrap();
    assert_eq!(setting(&project, "windows.0.transform.pan"), pan);
    apply(
        &mut project,
        set(&id, "qualifier.hue.center", Param::Fixed(12.5)),
    )
    .unwrap();
    assert_eq!(
        setting(&project, "qualifier.hue.center"),
        Param::Fixed(12.5)
    );
    // There is one window, and no second one to set.
    assert_eq!(
        apply(
            &mut project,
            set(&id, "windows.1.transform.pan", Param::Fixed(50.0))
        ),
        Err(OpError::Missing {
            what: "setting",
            id: "windows.1.transform.pan".into()
        })
    );

    let (mut project, id) = with_node(Kind::KeyMixer);
    let gain = Param::Keys(vec![key(0, 1.0), key(10, 0.5)]);
    apply(&mut project, set(&id, "inputs.1.gain", gain.clone())).unwrap();
    assert_eq!(setting(&project, "inputs.1.gain"), gain);
    apply(
        &mut project,
        splice(&id, "output.offset", 0, 0, vec![key(3, 0.5)]),
    )
    .unwrap();
    assert_eq!(
        setting(&project, "output.offset"),
        Param::Keys(vec![key(3, 0.5)])
    );
}

#[test]
fn what_is_no_setting_that_takes_keys_is_refused_and_nothing_changes() {
    let (mut project, id) = with_node(Kind::Corrector);
    let before = project.clone();
    let no_setting = |path: &str| OpError::Missing {
        what: "setting",
        id: path.into(),
    };
    for path in [
        // A number that takes no keys, a list, a palette, a word, and nothing.
        "curves.y.points.0.0",
        "curves.y.points",
        "primaries",
        "lut",
        "primaries.lift.w",
        "",
        // A setting of a palette that the node has not: it has no qualifier and no window.
        "qualifier.hue.center",
        "windows.0.opacity",
        // A setting of another kind of node.
        "inputs.0.gain",
    ] {
        let refused = apply(&mut project, set(&id, path, Param::Fixed(0.5)));
        assert_eq!(refused, Err(no_setting(path)), "{path}");
        let refused = apply(&mut project, splice(&id, path, 0, 0, vec![key(0, 0.5)]));
        assert_eq!(refused, Err(no_setting(path)), "{path}");
    }
    assert_eq!(
        apply(&mut project, set("n999", SATURATION, Param::Fixed(50.0))),
        Err(OpError::Missing {
            what: "node",
            id: "n999".into()
        })
    );
    // A mixer of pictures has inputs, and no setting that takes keys.
    let (mut mixer, mixer_id) = with_node(Kind::LayerMixer);
    assert_eq!(
        apply(&mut mixer, set(&mixer_id, "inputs", Param::Fixed(3.0))),
        Err(no_setting("inputs"))
    );
    assert_eq!(project, before);
}

#[test]
fn a_setting_is_checked_as_any_change_of_its_node_is() {
    let (mut project, id) = with_node(Kind::Corrector);
    let three = vec![key(0, 50.0), key(24, 80.0), key(48, 20.0)];
    apply(&mut project, set(&id, SATURATION, Param::Keys(three))).unwrap();
    let before = project.clone();
    let out_of_range = Err(OpError::OutOfRange {
        what: "primaries.saturation",
    });

    // A value beyond the bounds of the setting, as a number and in a key.
    assert_eq!(
        apply(&mut project, set(&id, SATURATION, Param::Fixed(100.5))),
        out_of_range
    );
    assert_eq!(
        apply(
            &mut project,
            splice(&id, SATURATION, 1, 1, vec![key(24, -1.0)])
        ),
        out_of_range
    );
    // Keys out of the order of their frames, two at one frame, and none at all.
    assert_eq!(
        apply(
            &mut project,
            splice(&id, SATURATION, 1, 1, vec![key(60, 80.0)])
        ),
        out_of_range
    );
    assert_eq!(
        apply(
            &mut project,
            splice(&id, SATURATION, 3, 0, vec![key(48, 80.0)])
        ),
        out_of_range
    );
    assert_eq!(
        apply(&mut project, set(&id, SATURATION, Param::Keys(Vec::new()))),
        out_of_range
    );
    // A handle beyond its share.
    let wide = Key {
        interp: Interp::Bezier {
            out: [1.5, 0.0],
            into: [0.5, 0.0],
        },
        ..key(24, 80.0)
    };
    assert_eq!(
        apply(&mut project, splice(&id, SATURATION, 1, 1, vec![wide])),
        out_of_range
    );
    // Places beyond the keys that there are.
    assert_eq!(
        apply(&mut project, splice(&id, SATURATION, 2, 2, Vec::new())),
        Err(OpError::OutOfList { at: 4, len: 3 })
    );
    assert_eq!(
        apply(
            &mut project,
            splice(&id, SATURATION, usize::MAX, 2, Vec::new())
        ),
        Err(OpError::OutOfList {
            at: usize::MAX,
            len: 3
        })
    );
    // A setting keeps a key: its number is put in the place of its keys by set_keys.
    let emptied = apply(&mut project, splice(&id, SATURATION, 0, 3, Vec::new()));
    assert!(
        matches!(&emptied, Err(OpError::Params { node, why })
            if *node == id && why.starts_with("primaries.saturation would have no key left")),
        "{emptied:?}"
    );
    assert_eq!(project, before);
}

#[test]
fn a_node_that_is_locked_is_not_set() {
    let (mut project, id) = with_node(Kind::Corrector);
    let lock = |locked: bool| Op::SetNode {
        grade: c1(),
        id: id.clone(),
        label: None,
        enabled: None,
        locked: Some(locked),
        pos: None,
        params: None,
    };
    apply(&mut project, lock(true)).unwrap();
    let before = project.clone();
    let locked = Err(OpError::NodeLocked { node: id.clone() });

    assert_eq!(
        apply(&mut project, set(&id, SATURATION, Param::Fixed(60.0))),
        locked
    );
    assert_eq!(
        apply(
            &mut project,
            splice(&id, SATURATION, 0, 0, vec![key(0, 60.0)])
        ),
        locked
    );
    assert_eq!(project, before);
    apply(&mut project, lock(false)).unwrap();
    apply(&mut project, set(&id, SATURATION, Param::Fixed(60.0))).unwrap();
    assert_eq!(setting(&project, SATURATION), Param::Fixed(60.0));
}

#[test]
fn the_operations_are_given_back_as_they_were_done_and_are_undone_and_redone() {
    let (project, id) = with_node(Kind::Corrector);
    let keys = vec![key(0, 50.0), key(24, 80.0)];
    let op = splice(&id, SATURATION, 0, 0, keys.clone());
    let (done, inverse) = apply_done(&mut project.clone(), op.clone()).unwrap();
    // The first keys of a number are given back as the setting whole.
    assert_eq!(done, set(&id, SATURATION, Param::Keys(keys.clone())));
    assert_eq!(inverse, set(&id, SATURATION, Param::Fixed(50.0)));

    let mut history = History::new(project);
    let before = history.project().clone();
    history.apply(vec![op], "a test").unwrap();
    history
        .apply(
            vec![splice(&id, SATURATION, 1, 1, vec![key(30, 90.0)])],
            "a test",
        )
        .unwrap();
    let after = history.project().clone();
    let moved = splice(&id, SATURATION, 1, 1, vec![key(40, 90.0)]);
    let (done, inverse) = apply_done(&mut after.clone(), moved.clone()).unwrap();
    assert_eq!(done, moved);
    assert_eq!(inverse, splice(&id, SATURATION, 1, 1, vec![key(30, 90.0)]));
    assert_eq!(
        setting(&after, SATURATION),
        Param::Keys(vec![key(0, 50.0), key(30, 90.0)])
    );
    history.undo("a test").unwrap().unwrap();
    assert_eq!(setting(history.project(), SATURATION), Param::Keys(keys));
    history.undo("a test").unwrap().unwrap();
    assert_eq!(*history.project(), before);
    history.redo("a test").unwrap().unwrap();
    history.redo("a test").unwrap().unwrap();
    assert_eq!(*history.project(), after);
}

#[test]
fn a_key_that_is_dragged_in_one_gesture_is_one_step() {
    let (project, id) = with_node(Kind::Corrector);
    let mut history = History::new(project);
    let three = vec![key(0, 50.0), key(24, 80.0), key(48, 20.0)];
    history
        .apply(
            vec![set(&id, SATURATION, Param::Keys(three.clone()))],
            "a test",
        )
        .unwrap();
    let before = history.revision();

    // The middle key goes up and to the right, a little with each move of the pointer.
    for (frame, value, last) in [(25, 81.0, false), (26, 82.0, false), (27, 83.0, true)] {
        let moved = key(frame, value);
        history
            .change_in_gesture(
                vec![Change::Op(splice(&id, SATURATION, 1, 1, vec![moved]))],
                "a test",
                &mut |change, _: &[Op]| change,
                "1:drag",
                last,
            )
            .unwrap();
    }
    assert_eq!(
        setting(history.project(), SATURATION),
        Param::Keys(vec![key(0, 50.0), key(27, 83.0), key(48, 20.0)])
    );
    assert_eq!(history.gesture(), None);

    // One undo takes the whole drag back, and one redo does it again.
    history.undo("a test").unwrap().unwrap();
    assert_eq!(setting(history.project(), SATURATION), Param::Keys(three));
    history.redo("a test").unwrap().unwrap();
    assert_eq!(
        setting(history.project(), SATURATION),
        Param::Keys(vec![key(0, 50.0), key(27, 83.0), key(48, 20.0)])
    );
    assert!(history.revision() > before);
}

#[test]
fn the_operations_are_written_as_the_protocol_names_them() {
    let op = splice("n9", SATURATION, 1, 2, vec![key(24, 80.0)]);
    assert_eq!(
        serde_json::to_value(&op).unwrap(),
        serde_json::json!({
            "op": "splice_keys",
            "grade": {"level": "clip", "timeline": "t1", "clip": "c1"},
            "id": "n9",
            "path": "primaries.saturation",
            "at": 1,
            "remove": 2,
            "keys": [{"frame": 24, "value": 80.0, "interp": "linear"}],
        })
    );
    let read: Op = serde_json::from_value(serde_json::json!({
        "op": "set_keys",
        "grade": {"level": "timeline", "timeline": "t1"},
        "id": "n9",
        "path": "primaries.lift.r",
        "param": 0.25,
    }))
    .unwrap();
    assert_eq!(
        read,
        Op::SetKeys {
            grade: Target::Timeline {
                timeline: "t1".into()
            },
            id: "n9".into(),
            path: "primaries.lift.r".into(),
            param: Param::Fixed(0.25),
        }
    );
}
