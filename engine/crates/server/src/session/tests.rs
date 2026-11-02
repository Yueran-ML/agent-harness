//! Locked transactions and owned gestures, without a view, saver, or background workers.

use super::*;

pub(super) fn state() -> State {
    State {
        history: History::new(Project::sample()),
        window: window::WindowState::default(),
        tracks: super::tracker::Tracks::default(),
        saver: None,
        peers: HashMap::new(),
        kicked: vec![],
        transport: None,
        proxies: Proxies::new(
            &proxy::Setup {
                make: false,
                ..proxy::Setup::default()
            },
            Path::new("."),
        ),
        gesture_last: None,
        next_owned_gesture: 0,
        held: None,
        told_at: None,
        tell_at: None,
    }
}

fn rename(name: &str) -> Value {
    json!({"op":"set_track", "timeline":"t1", "id":"V1", "name":name})
}

fn apply(
    state: &mut State,
    gesture: Option<(u64, &mut OwnedGesture)>,
    now: Instant,
    ops: Vec<Value>,
) -> Result<PreparedResult, WorkedOutError> {
    state.apply_worked_out_locked(Kind::Desktop, None, gesture, now, |_| Ok(ops))
}

#[test]
fn consecutive_worked_out_changes_are_one_owned_undo_step() {
    let mut state = state();
    let before = state.history.project().clone();
    let now = Instant::now();
    let mut gesture = state.begin_owned_gesture(7, 0).unwrap();
    assert_eq!(gesture.last_own, None);
    for suffix in ["a", "b", "c"] {
        let result = state
            .apply_worked_out_locked(
                Kind::Desktop,
                None,
                Some((7, &mut gesture)),
                now,
                |project| {
                    Ok(vec![rename(&format!(
                        "{}{suffix}",
                        project.timelines[0].tracks[0].name
                    ))])
                },
            )
            .unwrap();
        assert_eq!(gesture.last_own, Some(result.revision));
    }
    assert_eq!(state.history.project().timelines[0].tracks[0].name, "V1abc");
    assert_eq!(gesture.base, 0);
    state.finish_owned_gesture(7, gesture).unwrap();
    assert_eq!(
        state.history.revision(),
        3,
        "up adds no synthetic operation"
    );
    state.history.undo("desktop").unwrap().unwrap();
    assert_eq!(state.history.project(), &before);
    assert!(!state.history.can_undo());
    state.history.redo("desktop").unwrap().unwrap();
    assert_eq!(state.history.project().timelines[0].tracks[0].name, "V1abc");
}

#[test]
fn an_unwritten_finish_preserves_another_gesture_and_undo_redo() {
    let mut state = state();
    let now = Instant::now();
    apply(&mut state, None, now, vec![rename("redo")]).unwrap();
    state.history.undo("desktop").unwrap().unwrap();
    let before = state.history.project().clone();
    let revision = state.history.revision();
    let gesture = state.begin_owned_gesture(7, revision).unwrap();
    state.finish_owned_gesture(7, gesture).unwrap();
    assert_eq!(state.history.project(), &before);
    assert_eq!(state.history.revision(), revision);
    assert!(!state.history.can_undo());
    assert!(state.history.can_redo());

    let mut other = state.begin_owned_gesture(8, revision).unwrap();
    apply(
        &mut state,
        Some((8, &mut other)),
        now,
        vec![rename("other")],
    )
    .unwrap();
    let unused = state
        .begin_owned_gesture(7, state.history.revision())
        .unwrap();
    state.finish_owned_gesture(7, unused).unwrap();
    apply(
        &mut state,
        Some((8, &mut other)),
        now,
        vec![rename("still other")],
    )
    .unwrap();
    state.finish_owned_gesture(8, other).unwrap();
    state.history.undo("desktop").unwrap().unwrap();
    assert_eq!(state.history.project(), &before);
    assert!(!state.history.can_undo());
}

fn wire(
    state: &mut State,
    key: &str,
    end: bool,
    now: Instant,
    ops: Vec<Value>,
) -> Result<PreparedResult, Fault> {
    let mut inputs: Vec<names::Input> = ops
        .into_iter()
        .map(|op| serde_json::from_value(op).unwrap())
        .collect();
    let names = names::prepare(&mut inputs).unwrap();
    state.apply_prepared(
        inputs.into_iter().map(|input| input.change).collect(),
        names,
        Kind::Desktop,
        Some((key, end)),
        now,
    )
}

fn fault(error: WorkedOutError) -> Fault {
    match error {
        WorkedOutError::Input(fault) | WorkedOutError::Apply(fault) => fault,
    }
}

fn failures() -> Vec<(&'static str, Result<Vec<Value>, Fault>)> {
    vec![
        ("work", Err(Fault::params("cannot calculate this edit"))),
        ("parse", Ok(vec![json!({"op":"not_an_operation"})])),
        (
            "prepare",
            Ok(vec![
                json!({"op":"set_track", "timeline":"t1", "id":"$unknown", "name":"bad"}),
            ]),
        ),
        (
            "model",
            Ok(vec![
                json!({"edit":"add_track", "timeline":"t1", "kind":"subtitle"}),
                json!({"op":"set_track", "timeline":"t1", "id":"missing", "name":"bad"}),
            ]),
        ),
        (
            "named output",
            Ok(vec![
                rename("changed before failure"),
                json!({"edit":"insert", "timeline":"t1", "at":10, "source":{"kind":"color", "color":"#000000ff", "length":5}, "name":{"audio":"$a"}}),
            ]),
        ),
    ]
}

#[test]
fn every_failed_worked_out_stage_preserves_project_ids_history_and_owned_continuation() {
    for (stage, work) in failures() {
        let mut state = state();
        let now = Instant::now();
        let mut gesture = state.begin_owned_gesture(7, 0).unwrap();
        apply(
            &mut state,
            Some((7, &mut gesture)),
            now,
            vec![rename("first")],
        )
        .unwrap();
        apply(
            &mut state,
            Some((7, &mut gesture)),
            now,
            vec![rename("second")],
        )
        .unwrap();
        let before = state.history.project().clone();
        let held = state.held.as_ref().unwrap().done.clone();
        let last = state.gesture_last;
        let due = state.tell_at;
        let result = state.apply_worked_out_locked(
            Kind::Desktop,
            None,
            Some((7, &mut gesture)),
            now,
            |_| work,
        );
        let error = result.unwrap_err();
        let early = matches!(&error, WorkedOutError::Input(_));
        let error = fault(error);
        assert_eq!(early, matches!(stage, "work" | "parse" | "prepare"));
        match stage {
            "work" => assert_eq!(error, Fault::params("cannot calculate this edit")),
            "parse" => assert_eq!(error.data, Some(json!({"index":0,"reason":"invalid_op"}))),
            "prepare" => assert_eq!(error.data, Some(json!({"reason":"unknown_name"}))),
            "model" | "named output" => {
                assert_eq!(error.code, rpc::REFUSED);
                let data = error.data.unwrap();
                assert_eq!(data["index"], 1);
                assert_eq!(data["reason"], "missing");
                assert_eq!(
                    data["what"],
                    if stage == "model" {
                        "track"
                    } else {
                        "named output"
                    }
                );
            }
            _ => unreachable!(),
        }
        assert_eq!(
            state.history.project(),
            &before,
            "{stage}: includes next_id"
        );
        assert_eq!(state.history.revision(), 2, "{stage}");
        assert_eq!(gesture.last_own, Some(2), "{stage}");
        assert_eq!(state.gesture_last, last, "{stage}");
        assert_eq!(state.tell_at, due, "{stage}");
        assert_eq!(state.held.as_ref().unwrap().done, held, "{stage}");
        apply(
            &mut state,
            Some((7, &mut gesture)),
            now,
            vec![rename("continued")],
        )
        .unwrap();
        state.finish_owned_gesture(7, gesture).unwrap();
        state.history.undo("desktop").unwrap().unwrap();
        assert_eq!(state.history.project().timelines[0].tracks[0].name, "V1");
        assert!(!state.history.can_undo(), "{stage}");
    }
}

#[test]
fn failed_first_writes_and_failed_external_requests_do_not_interrupt_gestures() {
    let now = Instant::now();
    for (stage, work) in failures() {
        let mut state = state();
        apply(&mut state, None, now, vec![rename("redo")]).unwrap();
        state.history.undo("desktop").unwrap().unwrap();
        let mut gesture = state.begin_owned_gesture(7, 2).unwrap();
        let before = state.history.project().clone();
        assert!(
            state
                .apply_worked_out_locked(Kind::Desktop, None, Some((7, &mut gesture)), now, |_| {
                    work.clone()
                })
                .is_err(),
            "{stage}"
        );
        assert_eq!(gesture.last_own, None);
        assert_eq!(state.history.project(), &before);
        assert_eq!(state.history.revision(), 2);
        assert!(state.history.can_redo());
        assert!(!state.history.can_undo());
        apply(
            &mut state,
            Some((7, &mut gesture)),
            now,
            vec![rename("first")],
        )
        .unwrap();
        assert!(
            state
                .apply_worked_out_locked(Kind::Agent, None, None, now, |_| work)
                .is_err(),
            "{stage}"
        );
        assert!(wire(&mut state, "8:other", true, now, vec![]).is_err());
        apply(
            &mut state,
            Some((7, &mut gesture)),
            now,
            vec![rename("continued")],
        )
        .unwrap();
        state.finish_owned_gesture(7, gesture).unwrap();
        state.history.undo("desktop").unwrap().unwrap();
        assert_eq!(state.history.project(), &before);
        assert!(!state.history.can_undo());
    }
}

#[test]
fn wire_end_on_model_failure_keeps_its_existing_end_semantics() {
    let mut state = state();
    let now = Instant::now();
    wire(&mut state, "7:drag", false, now, vec![rename("first")]).unwrap();
    wire(&mut state, "7:drag", false, now, vec![rename("second")]).unwrap();
    assert!(state.held.is_some());
    let before = state.history.project().clone();
    assert!(wire(&mut state, "7:drag", true, now, vec![]).is_err());
    assert_eq!(state.history.project(), &before);
    assert_eq!(state.history.revision(), 2);
    assert!(state.history.gesture().is_none());
    assert!(state.gesture_last.is_none());
    assert!(state.held.is_none());
    wire(&mut state, "7:drag", false, now, vec![rename("reopened")]).unwrap();
    assert_eq!(state.history.gesture(), Some(("7:drag", 2)));
    state.history.undo("desktop").unwrap().unwrap();
    assert_eq!(state.history.project(), &before);
    assert!(state.history.can_undo());
}

#[test]
fn owner_key_and_base_are_checked_before_work_and_finish() {
    for change in ["owner", "key", "base", "last"] {
        let mut state = state();
        let now = Instant::now();
        let mut gesture = state.begin_owned_gesture(7, 0).unwrap();
        apply(
            &mut state,
            Some((7, &mut gesture)),
            now,
            vec![rename("first")],
        )
        .unwrap();
        apply(
            &mut state,
            Some((7, &mut gesture)),
            now,
            vec![rename("second")],
        )
        .unwrap();
        let owner = if change == "owner" { 8 } else { 7 };
        match change {
            "key" => gesture.key.push_str("-old"),
            "base" => gesture.base += 1,
            "last" => gesture.last_own = Some(1),
            _ => {}
        }
        let before = state.history.project().clone();
        let original = state
            .history
            .gesture()
            .map(|(key, base)| (key.to_owned(), base));
        let error = state
            .apply_worked_out_locked(
                Kind::Desktop,
                None,
                Some((owner, &mut gesture)),
                now,
                |_| panic!("work must not run for {change}"),
            )
            .unwrap_err();
        assert_eq!(fault(error).code, rpc::REFUSED);
        assert!(state.finish_owned_gesture(owner, gesture).is_err());
        assert_eq!(state.history.project(), &before);
        assert_eq!(state.history.revision(), 2);
        assert_eq!(
            state
                .history
                .gesture()
                .map(|(key, base)| (key.to_owned(), base)),
            original
        );
        assert!(state.held.is_some());
    }
}

#[test]
fn an_external_revision_or_ended_gesture_never_reopens_an_owned_token() {
    for written in [false, true] {
        for external in ["ordinary", "gesture", "owned", "undo", "redo", "unrecorded"] {
            let mut state = state();
            let now = Instant::now();
            apply(&mut state, None, now, vec![rename("seed")]).unwrap();
            let mut gesture = state.begin_owned_gesture(7, 1).unwrap();
            if written {
                apply(
                    &mut state,
                    Some((7, &mut gesture)),
                    now,
                    vec![rename("own")],
                )
                .unwrap();
            }
            match external {
                "ordinary" => {
                    apply(&mut state, None, now, vec![rename("external")]).unwrap();
                }
                "gesture" => {
                    wire(&mut state, "7:other", false, now, vec![rename("external")]).unwrap();
                }
                "owned" => {
                    let mut other = state
                        .begin_owned_gesture(7, state.history.revision())
                        .unwrap();
                    assert_ne!(gesture.key, other.key);
                    apply(
                        &mut state,
                        Some((7, &mut other)),
                        now,
                        vec![rename("external")],
                    )
                    .unwrap();
                }
                "undo" => {
                    state.history.undo("agent").unwrap().unwrap();
                }
                "redo" => {
                    state.history.undo("agent").unwrap().unwrap();
                    state.history.redo("agent").unwrap().unwrap();
                }
                "unrecorded" => {
                    state
                        .history
                        .apply_unrecorded(
                            vec![Op::SetProxy {
                                media: "m1".into(),
                                proxy: None,
                            }],
                            "engine",
                        )
                        .unwrap();
                }
                _ => unreachable!(),
            }
            let before = state.history.project().clone();
            let revision = state.history.revision();
            let last_own = gesture.last_own;
            // Supplying the latest revision must not refresh the owned token's immutable base.
            let error = state
                .apply_worked_out_locked(
                    Kind::Desktop,
                    Some(revision),
                    Some((7, &mut gesture)),
                    now,
                    |_| panic!("stale work: {external}"),
                )
                .unwrap_err();
            assert_eq!(fault(error).code, rpc::REFUSED, "{external}");
            assert_eq!(gesture.last_own, last_own);
            assert!(
                state.finish_owned_gesture(7, gesture).is_err(),
                "{external}"
            );
            assert_eq!(state.history.project(), &before);
            assert_eq!(state.history.revision(), revision);
        }
    }
}

fn windows() -> Value {
    json!([
        {"id":"w1", "label":"Subject", "enabled":true, "invert":true, "combine":"add", "opacity":75.0,
         "transform":{"size":40.0,"aspect":60.0,"pan":{"keys":[{"frame":0,"value":50.0,"interp":"linear"},{"frame":20,"value":65.0,"interp":"hold"}]},"tilt":50.0,"rotate":20.0},
         "shape":{"kind":"circular","softness":4.0}},
        {"id":"w2", "label":"Exclude", "enabled":false, "invert":false, "combine":"subtract", "opacity":25.0,
         "transform":{"size":30.0,"aspect":40.0,"pan":30.0,"tilt":60.0,"rotate":0.0},
         "shape":{"kind":"gradient","softness":2.0}}
    ])
}

fn set_windows(windows: &Value) -> Value {
    json!({"op":"set_node", "grade":{"level":"timeline","timeline":"t1"}, "id":"n3", "params":{"windows":windows}})
}

#[test]
fn restoring_complete_windows_or_a_virtual_nodes_grade_leaves_no_empty_undo_step() {
    for virtual_node in [false, true] {
        let mut state = state();
        let now = Instant::now();
        let add = json!({"op":"add_node", "grade":{"level":"timeline","timeline":"t1"}, "node":columbina_model::Node::corrector(""), "links":[
            {"from":"source","to":"","kind":"rgb","input":0}, {"from":"","to":"output","kind":"rgb","input":0}
        ]});
        if !virtual_node {
            let node: Op = serde_json::from_value(add.clone()).unwrap();
            let mut project = Project::sample();
            columbina_model::apply(&mut project, node).unwrap();
            columbina_model::apply(
                &mut project,
                serde_json::from_value(set_windows(&windows())).unwrap(),
            )
            .unwrap();
            state.history = History::new(project);
        }
        apply(&mut state, None, now, vec![rename("redo cleared by drag")]).unwrap();
        state.history.undo("desktop").unwrap().unwrap();
        let before = state.history.project().clone();
        let mut gesture = state.begin_owned_gesture(7, 2).unwrap();
        let original_grade = before.timelines[0].grade.clone();
        if virtual_node {
            apply(&mut state, Some((7, &mut gesture)), now, vec![add]).unwrap();
        }
        let mut moved = windows();
        moved[0]["transform"]["pan"] = json!(70.0);
        apply(
            &mut state,
            Some((7, &mut gesture)),
            now,
            vec![set_windows(&moved)],
        )
        .unwrap();
        // A failed attempted restore must leave the same token available to retry.
        let invalid = json!({"op":"set_node", "grade":{"level":"timeline","timeline":"t1"}, "id":"missing", "params":{"windows":[]}});
        assert!(apply(&mut state, Some((7, &mut gesture)), now, vec![invalid]).is_err());
        let restore = if virtual_node {
            json!({"op":"set_grade", "grade":{"level":"timeline","timeline":"t1"}, "value":original_grade})
        } else {
            set_windows(&windows())
        };
        apply(&mut state, Some((7, &mut gesture)), now, vec![restore]).unwrap();
        let revision = state.history.revision();
        state.finish_owned_gesture(7, gesture).unwrap();
        let mut expected = before;
        expected.next_id = state.history.project().next_id;
        assert_eq!(state.history.project(), &expected);
        assert_eq!(state.history.revision(), revision);
        assert!(!state.history.can_undo());
        assert!(
            !state.history.can_redo(),
            "cancel does not resurrect cleared redo"
        );
    }
}

fn listen(state: &mut State) -> queue::Receiver {
    let (queue, receiver) = queue::queue(Limits {
        messages: 100,
        bytes: 1_000_000,
    });
    state.peers.insert(
        99,
        Peer {
            kind: Kind::Agent,
            queue,
            stop: watch::channel(false).0,
        },
    );
    receiver
}

fn replay(
    state: &State,
    receiver: &mut queue::Receiver,
    project: &mut Project,
    revision: &mut u64,
) -> Vec<Value> {
    let mut changes = vec![];
    while let Some(line) = receiver.try_recv() {
        let note: Value = serde_json::from_slice(&line).unwrap();
        assert_eq!(note["method"], "project.changed");
        let change = &note["params"];
        let next = change["revision"].as_u64().unwrap();
        assert_eq!(change["base"].as_u64().unwrap_or(next - 1), *revision);
        project.next_id = change["next_id"].as_u64().unwrap();
        for op in serde_json::from_value::<Vec<Op>>(change["done"].clone()).unwrap() {
            columbina_model::apply(project, op).unwrap();
        }
        *revision = next;
        changes.push(change.clone());
    }
    assert_eq!(*revision, state.history.revision());
    assert_eq!(project, state.history.project());
    changes
}

#[test]
fn held_notifications_replay_and_flush_before_finish_or_an_external_success() {
    for finish in [false, true] {
        let mut state = state();
        let mut project = state.history.project().clone();
        let mut revision = 0;
        let mut receiver = listen(&mut state);
        let now = Instant::now();
        let mut gesture = state.begin_owned_gesture(7, 0).unwrap();
        let first = apply(
            &mut state,
            Some((7, &mut gesture)),
            now,
            vec![rename("first")],
        )
        .unwrap();
        assert!(first.due.is_none());
        assert_eq!(
            replay(&state, &mut receiver, &mut project, &mut revision).len(),
            1
        );
        let second = apply(
            &mut state,
            Some((7, &mut gesture)),
            now,
            vec![rename("second")],
        )
        .unwrap();
        assert!(second.due.is_some());
        let third = apply(
            &mut state,
            Some((7, &mut gesture)),
            now,
            vec![rename("third")],
        )
        .unwrap();
        assert!(third.due.is_none(), "one outstanding due timer");
        assert!(receiver.try_recv().is_none());
        assert!(apply(&mut state, None, now, vec![]).is_err());
        assert!(
            receiver.try_recv().is_none(),
            "external failure does not flush"
        );
        if finish {
            state.finish_owned_gesture(7, gesture).unwrap();
        } else {
            apply(&mut state, None, now, vec![rename("external")]).unwrap();
        }
        let changes = replay(&state, &mut receiver, &mut project, &mut revision);
        assert_eq!(changes.len(), if finish { 1 } else { 2 });
        assert_eq!(changes[0]["base"], 1);
        assert_eq!(changes[0]["revision"], 3);
        assert_eq!(changes[0]["done"].as_array().unwrap().len(), 2);
        assert!(state.held.is_none());
        assert!(state.tell_at.is_none());
    }
}

#[test]
fn idle_ends_at_ten_seconds_and_an_owned_token_stays_stale() {
    let mut state = state();
    let now = Instant::now();
    let mut gesture = state.begin_owned_gesture(7, 0).unwrap();
    apply(
        &mut state,
        Some((7, &mut gesture)),
        now,
        vec![rename("first")],
    )
    .unwrap();
    apply(
        &mut state,
        Some((7, &mut gesture)),
        now,
        vec![rename("second")],
    )
    .unwrap();
    state.end_idle_gesture(now + GESTURE_IDLE.checked_sub(Duration::from_nanos(1)).unwrap());
    assert!(state.history.gesture().is_some());
    let error = state
        .apply_worked_out_locked(
            Kind::Desktop,
            None,
            Some((7, &mut gesture)),
            now + GESTURE_IDLE,
            |_| panic!("idle token cannot resume"),
        )
        .unwrap_err();
    assert_eq!(fault(error).code, rpc::REFUSED);
    assert!(state.history.gesture().is_none());
    assert!(state.gesture_last.is_none());
    assert!(state.held.is_none());
    assert_eq!(state.history.revision(), 2);
    assert!(apply(
        &mut state,
        Some((7, &mut gesture)),
        now + GESTURE_IDLE,
        vec![rename("reopen")]
    )
    .is_err());
    // Ordinary requests still end idle before rejecting their stale revision.
    wire(&mut state, "7:wire", false, now, vec![rename("wire")]).unwrap();
    let error = state
        .apply_worked_out_locked(Kind::Agent, Some(0), None, now + GESTURE_IDLE, |_| {
            panic!("stale revision")
        })
        .unwrap_err();
    assert_eq!(fault(error).data.unwrap()["reason"], "stale_revision");
    assert!(state.history.gesture().is_none());
    wire(
        &mut state,
        "7:wire",
        false,
        now + GESTURE_IDLE,
        vec![rename("wire reopens")],
    )
    .unwrap();
}

#[test]
fn failed_worked_out_requests_preserve_an_existing_wire_gesture_and_input_error_data() {
    let now = Instant::now();
    for (stage, work) in failures() {
        let mut state = state();
        wire(&mut state, "7:wire", false, now, vec![rename("first")]).unwrap();
        let before = state.history.project().clone();
        let error = state
            .apply_worked_out_locked(Kind::Agent, None, None, now, |_| work)
            .unwrap_err();
        if stage == "prepare" {
            assert_eq!(fault(error).data, Some(json!({"reason":"unknown_name"})));
        }
        assert_eq!(state.history.project(), &before);
        assert_eq!(state.history.revision(), 1);
        assert_eq!(state.history.gesture(), Some(("7:wire", 0)));
        wire(&mut state, "7:wire", true, now, vec![rename("second")]).unwrap();
        state.history.undo("desktop").unwrap().unwrap();
        assert!(!state.history.can_undo());
    }
}
