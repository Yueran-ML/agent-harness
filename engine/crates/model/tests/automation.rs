//! The automation of a timeline (T14): the operations on lanes, each taken back by its inverse;
//! the edits of lanes (points added, taken away and moved, a range taken away, raised, pasted
//! over or thinned, a lane drawn, lanes cleared), each one step of undo; and the edits of inserts
//! that leave no lane naming an insert that is not there.

#![allow(
    clippy::needless_pass_by_value,
    reason = "settings are given as a client gives them"
)]

use columbina_model::{apply, apply_done, Edit, Lane, LaneKind, Op, Project, Time};
use serde_json::{json, Value};

/// The sample, with a bus B2 that A1 sends to, a Gain on A1 (`fx1`) and a VCA.
fn project() -> Project {
    let mut project = Project::sample();
    project.validate().unwrap();
    for change in [
        json!({"op": "add_bus", "timeline": "t1", "bus": {"id": "B2", "name": "Bus 2",
            "color": "#336699ff", "format": "stereo", "strip": {"outputs": [{"bus": "B1"}]}}}),
        json!({"op": "set_track_audio", "timeline": "t1", "track": "A1", "patch": {"strip": {
            "sends": [{"bus": "B2", "level_db": -6}], "vcas": ["VCA1"]}}}),
        json!({"op": "add_insert", "timeline": "t1", "of": {"track": "A1"},
            "insert": {"effect": {"kind": "gain", "gain_db": -3}}}),
    ] {
        if change["op"] == "set_track_audio" {
            op(
                &mut project,
                json!({"op": "add_vca", "timeline": "t1", "vca": {"id": "VCA1", "name": "VCA"}}),
            );
        }
        op(&mut project, change);
    }
    project
}

fn op(project: &mut Project, op: Value) -> Op {
    let op: Op = serde_json::from_value(op).unwrap();
    apply(project, op.clone()).unwrap_or_else(|e| panic!("{op:?}: {e}"))
}

/// An operation refused, as it is written without its message; the project left as it was.
fn refused_op(project: &mut Project, op: Value) -> Value {
    let before = project.clone();
    let op: Op = serde_json::from_value(op).unwrap();
    let error = apply(project, op).unwrap_err();
    assert_eq!(*project, before);
    let mut value = serde_json::to_value(&error).unwrap();
    value.as_object_mut().unwrap().remove("message");
    value
}

/// An edit applied: the operations as they were done, and whether it was clamped. Each is taken
/// back by the inverse, in one step.
fn edit(project: &mut Project, edit: Value) -> (Vec<Op>, bool) {
    let before = project.clone();
    let edit: Edit = serde_json::from_value(edit).unwrap();
    let expansion =
        columbina_model::expand(project, edit.clone()).unwrap_or_else(|e| panic!("{edit:?}: {e}"));
    let mut undone = project.clone();
    for inverse in expansion.inverse.iter().rev() {
        apply(&mut undone, inverse.clone()).unwrap();
    }
    assert_eq!(undone, before, "{edit:?} taken back");
    let mut redone = before;
    for done in &expansion.done {
        apply(&mut redone, done.clone()).unwrap();
    }
    assert_eq!(&redone, project, "{edit:?} done again");
    (expansion.done, expansion.clamped)
}

fn refused(project: &mut Project, edit: Value) -> Value {
    let before = project.clone();
    let edit: Edit = serde_json::from_value(edit).unwrap();
    let error = columbina_model::expand(project, edit).unwrap_err();
    assert_eq!(*project, before);
    let mut value = serde_json::to_value(&error).unwrap();
    value.as_object_mut().unwrap().remove("message");
    value
}

/// A time of the timeline, in milliseconds, at 48 kHz.
fn ms(ms: i64) -> Value {
    json!({"ticks": ms * 48, "scale": 48000})
}

fn at(ms: i64) -> Time {
    Time {
        ticks: ms * 48,
        scale: 48000,
    }
}

/// A lane of points at milliseconds.
fn lane(points: &[(i64, f64)], kind: &str) -> Value {
    let points: Vec<Value> = points
        .iter()
        .map(|&(t, v)| json!({"at": ms(t), "value": v}))
        .collect();
    json!({"points": points, "kind": kind})
}

fn lane_of<'p>(project: &'p Project, track: &str, path: &str) -> Option<&'p Lane> {
    let t = &project.timelines[0];
    match track {
        "B2" => t.buses.iter().find(|b| b.id == "B2")?.lanes.get(path),
        "VCA1" => t.vcas[0].lanes.get(path),
        _ => t
            .tracks
            .iter()
            .find(|r| r.id == track)?
            .audio
            .lanes
            .get(path),
    }
}

/// The values of a lane at milliseconds.
fn value(project: &Project, track: &str, path: &str, at_ms: f64) -> f64 {
    #[allow(clippy::cast_possible_truncation, reason = "a time in the test")]
    let ticks = (at_ms * 48.0).round() as i64;
    lane_of(project, track, path).unwrap().value_at(Time {
        ticks,
        scale: 48000,
    })
}

fn set_lane(of: Value, path: &str, lane: Value) -> Value {
    json!({"op": "set_lane", "timeline": "t1", "of": of, "path": path, "lane": lane})
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

// The operations.

#[test]
fn a_lane_is_set_replaced_and_taken_away_and_each_is_taken_back() {
    let mut project = project();
    let before = project.clone();
    let fader = lane(&[(0, -10.0), (1000, 0.0)], "continuous");
    let set: Op =
        serde_json::from_value(set_lane(json!({"track": "A1"}), "fader_db", fader)).unwrap();
    let (done, inverse) = apply_done(&mut project, set.clone()).unwrap();
    // As it was done, it has the scale of its path (linear for a fader).
    let mut given = serde_json::to_value(&set).unwrap();
    given["lane"]["scale"] = json!("linear");
    assert_eq!(serde_json::to_value(&done).unwrap(), given);
    assert_eq!(
        serde_json::to_value(&inverse).unwrap(),
        set_lane(json!({"track": "A1"}), "fader_db", Value::Null)
    );
    assert!(near(value(&project, "A1", "fader_db", 250.0), -7.5));
    // Replaced: the inverse puts the first back.
    let other = set_lane(
        json!({"track": "A1"}),
        "fader_db",
        lane(&[(0, 3.0)], "continuous"),
    );
    let inverse = op(&mut project, other);
    assert_eq!(value(&project, "A1", "fader_db", 250.0), 3.0);
    apply(&mut project, inverse).unwrap();
    assert!(near(value(&project, "A1", "fader_db", 250.0), -7.5));
    // Taken away, and put back by the inverse.
    let inverse = op(
        &mut project,
        set_lane(json!({"track": "A1"}), "fader_db", Value::Null),
    );
    assert!(lane_of(&project, "A1", "fader_db").is_none());
    apply(&mut project, inverse).unwrap();
    assert!(lane_of(&project, "A1", "fader_db").is_some());
    let inverse = op(
        &mut project,
        set_lane(json!({"track": "A1"}), "fader_db", Value::Null),
    );
    apply(&mut project, inverse).unwrap();
    op(
        &mut project,
        set_lane(json!({"track": "A1"}), "fader_db", Value::Null),
    );
    assert_eq!(project, before);
}

#[test]
fn lanes_go_on_tracks_buses_vcas_sends_and_inserts() {
    let mut project = project();
    for (of, path, kind, value) in [
        (json!({"track": "A1"}), "mute", "switch", 1.0),
        (
            json!({"track": "A1"}),
            "send.B2.level_db",
            "continuous",
            -12.0,
        ),
        (
            json!({"track": "A1"}),
            "insert.fx1.gain_db",
            "continuous",
            6.0,
        ),
        (json!({"track": "A1"}), "insert.fx1.on", "switch", 0.0),
        (json!({"track": "A1"}), "eq.2.gain_db", "continuous", 4.0),
        (
            json!({"track": "A1"}),
            "dynamics.compressor.threshold_db",
            "continuous",
            -30.0,
        ),
        (json!({"bus": "B2"}), "fader_db", "continuous", -3.0),
        (json!({"bus": "B2"}), "mute", "switch", 1.0),
        (json!({"vca": "VCA1"}), "fader_db", "continuous", -4.0),
        (json!({"vca": "VCA1"}), "mute", "switch", 0.0),
    ] {
        op(
            &mut project,
            set_lane(of, path, lane(&[(500, value)], kind)),
        );
    }
    assert_eq!(
        lane_of(&project, "B2", "mute").unwrap().kind,
        LaneKind::Switch
    );
    assert_eq!(
        lane_of(&project, "VCA1", "fader_db").unwrap().points[0].value,
        -4.0
    );
    project.validate().unwrap();
}

#[test]
#[allow(clippy::too_many_lines, reason = "a refusal for each way to be wrong")]
fn a_lane_out_of_its_setting_or_of_another_kind_is_refused() {
    let mut project = project();
    let wrong = |what: &str| json!({"reason": "out_of_range", "what": what});
    for (of, path, lane_given) in [
        // A switch of a value that is neither 0 nor 1, and a continuous lane of a switch.
        (json!({"track": "A1"}), "mute", lane(&[(0, 0.5)], "switch")),
        (
            json!({"track": "A1"}),
            "mute",
            lane(&[(0, 1.0)], "continuous"),
        ),
        // Values out of the range of the setting.
        (
            json!({"track": "A1"}),
            "fader_db",
            lane(&[(0, 0.0), (10, 11.0)], "continuous"),
        ),
        (
            json!({"track": "A1"}),
            "pan.x",
            lane(&[(0, -1.5)], "continuous"),
        ),
        (
            json!({"track": "A1"}),
            "insert.fx1.gain_db",
            lane(&[(0, 99.0)], "continuous"),
        ),
        (
            json!({"vca": "VCA1"}),
            "fader_db",
            lane(&[(0, 12.0)], "continuous"),
        ),
        // Settings that are not there, or that no lane is of.
        (
            json!({"track": "A1"}),
            "send.B9.level_db",
            lane(&[(0, 0.0)], "continuous"),
        ),
        (
            json!({"track": "A1"}),
            "insert.fx7.gain_db",
            lane(&[(0, 0.0)], "continuous"),
        ),
        (
            json!({"track": "A1"}),
            "order",
            lane(&[(0, 0.0)], "continuous"),
        ),
        (
            json!({"vca": "VCA1"}),
            "pan.x",
            lane(&[(0, 0.0)], "continuous"),
        ),
        // No point.
        (
            json!({"track": "A1"}),
            "fader_db",
            json!({"points": [], "kind": "continuous"}),
        ),
    ] {
        let path_given = path.to_owned();
        assert_eq!(
            refused_op(&mut project, set_lane(of, path, lane_given)),
            wrong("lane"),
            "{path_given}"
        );
    }
    assert_eq!(
        refused_op(
            &mut project,
            set_lane(
                json!({"track": "V1"}),
                "fader_db",
                lane(&[(0, 0.0)], "continuous")
            )
        ),
        json!({"reason": "wrong_track", "what": "lanes", "kind": "video"})
    );
    assert_eq!(
        refused_op(
            &mut project,
            set_lane(
                json!({"bus": "B7"}),
                "fader_db",
                lane(&[(0, 0.0)], "continuous")
            )
        ),
        json!({"reason": "unknown_bus", "id": "B7"})
    );
    assert_eq!(
        refused_op(
            &mut project,
            set_lane(
                json!({"vca": "VCA7"}),
                "fader_db",
                lane(&[(0, 0.0)], "continuous")
            )
        ),
        json!({"reason": "unknown_vca", "id": "VCA7"})
    );
    assert_eq!(
        refused_op(
            &mut project,
            set_lane(json!({"track": "A1"}), "fader_db", Value::Null)
        ),
        json!({"reason": "missing", "what": "lane", "id": "fader_db"})
    );
}

#[test]
fn a_splice_puts_points_in_place_of_others_and_its_inverse_holds_only_those_it_took() {
    let mut project = project();
    op(
        &mut project,
        set_lane(
            json!({"track": "A1"}),
            "fader_db",
            lane(
                &[(0, -10.0), (100, -5.0), (200, 0.0), (300, -20.0)],
                "continuous",
            ),
        ),
    );
    let before = project.clone();
    let splice = json!({"op": "splice_lane", "timeline": "t1", "of": {"track": "A1"},
        "path": "fader_db", "at": 1, "remove": 2,
        "points": [{"at": ms(150), "value": -1.0}]});
    let (done, inverse) = apply_done(
        &mut project,
        serde_json::from_value(splice.clone()).unwrap(),
    )
    .unwrap();
    assert_eq!(serde_json::to_value(&done).unwrap(), splice);
    assert_eq!(
        serde_json::to_value(&inverse).unwrap(),
        json!({"op": "splice_lane", "timeline": "t1", "of": {"track": "A1"}, "path": "fader_db",
            "at": 1, "remove": 1, "points": [{"at": ms(100), "value": -5.0}, {"at": ms(200), "value": 0.0}]})
    );
    assert_eq!(lane_of(&project, "A1", "fader_db").unwrap().points.len(), 3);
    apply(&mut project, inverse).unwrap();
    assert_eq!(project, before);
    // Out of the list, out of order, of no point left, and of a lane that is not there.
    let splice = |at: usize, remove: usize, points: Value| {
        json!({"op": "splice_lane", "timeline": "t1", "of": {"track": "A1"}, "path": "fader_db",
            "at": at, "remove": remove, "points": points})
    };
    assert_eq!(
        refused_op(&mut project, splice(3, 2, json!([]))),
        json!({"reason": "out_of_list", "at": 5, "len": 4})
    );
    assert_eq!(
        refused_op(
            &mut project,
            splice(0, 1, json!([{"at": ms(250), "value": 0.0}]))
        ),
        json!({"reason": "out_of_range", "what": "lane"})
    );
    assert_eq!(
        refused_op(&mut project, splice(0, 4, json!([]))),
        json!({"reason": "out_of_range", "what": "lane"})
    );
    let mut other = splice(0, 0, json!([]));
    other["path"] = json!("pan.x");
    assert_eq!(
        refused_op(&mut project, other),
        json!({"reason": "missing", "what": "lane", "id": "pan.x"})
    );
}

// The edits of points.

fn points_edit(path: &str, action: &str, extra: Value) -> Value {
    let mut edit = json!({"edit": "lane_points", "timeline": "t1", "of": {"track": "A1"},
        "path": path, "action": action});
    for (key, value) in extra.as_object().unwrap() {
        edit[key] = value.clone();
    }
    edit
}

#[test]
fn a_point_added_takes_the_value_of_the_lane_there_and_makes_a_lane_where_there_is_none() {
    let mut project = project();
    // No lane yet: one is made at the fader as it is set, of the kind of the setting.
    op(
        &mut project,
        json!({"op": "set_track_audio", "timeline": "t1", "track": "A1", "patch": {"strip": {"fader_db": -6}}}),
    );
    let (done, _) = edit(
        &mut project,
        points_edit("fader_db", "add", json!({"points": [{"at": ms(500)}]})),
    );
    assert_eq!(done.len(), 1);
    assert_eq!(
        lane_of(&project, "A1", "fader_db").unwrap().points,
        vec![columbina_model::LanePoint {
            at: at(500),
            value: -6.0
        }]
    );
    edit(
        &mut project,
        points_edit(
            "mute",
            "add",
            json!({"points": [{"at": ms(100), "value": 0.7}]}),
        ),
    );
    let mute = lane_of(&project, "A1", "mute").unwrap();
    assert_eq!((mute.kind, mute.points[0].value), (LaneKind::Switch, 1.0));
    // On a line: the new point keeps the value there, and nothing is heard to change.
    edit(
        &mut project,
        points_edit(
            "fader_db",
            "add",
            json!({"points": [{"at": ms(1500), "value": 0.0}]}),
        ),
    );
    edit(
        &mut project,
        points_edit("fader_db", "add", json!({"points": [{"at": ms(1000)}]})),
    );
    assert!(near(value(&project, "A1", "fader_db", 1000.0), -3.0));
    assert!(near(value(&project, "A1", "fader_db", 750.0), -4.5));
    // One at the time of a point sets its value; one past +10 dB is kept at it.
    let (_, clamped) = edit(
        &mut project,
        points_edit(
            "fader_db",
            "add",
            json!({"points": [{"at": ms(1000), "value": 30.0}]}),
        ),
    );
    assert!(clamped);
    assert_eq!(lane_of(&project, "A1", "fader_db").unwrap().points.len(), 3);
    assert_eq!(value(&project, "A1", "fader_db", 1000.0), 10.0);
}

#[test]
fn points_are_taken_away_by_their_times_and_the_last_takes_the_lane_away() {
    let mut project = project();
    op(
        &mut project,
        set_lane(
            json!({"track": "A1"}),
            "fader_db",
            lane(
                &[(0, -10.0), (500, -5.0), (500, -1.0), (1000, 0.0)],
                "continuous",
            ),
        ),
    );
    edit(
        &mut project,
        points_edit("fader_db", "remove", json!({"at": [ms(500)]})),
    );
    assert_eq!(lane_of(&project, "A1", "fader_db").unwrap().points.len(), 2);
    assert_eq!(
        refused(
            &mut project,
            points_edit("fader_db", "remove", json!({"at": [ms(400)]}))
        ),
        json!({"reason": "missing", "what": "point", "id": "19200/48000"})
    );
    edit(
        &mut project,
        points_edit("fader_db", "remove", json!({"at": [ms(0), ms(1000)]})),
    );
    assert!(lane_of(&project, "A1", "fader_db").is_none());
    assert_eq!(
        refused(
            &mut project,
            points_edit("fader_db", "remove", json!({"at": [ms(0)]}))
        ),
        json!({"reason": "missing", "what": "lane", "id": "fader_db"})
    );
}

#[test]
fn points_move_as_far_as_their_neighbours_and_the_range_of_their_setting_let_them() {
    let mut project = project();
    op(
        &mut project,
        set_lane(
            json!({"track": "A1"}),
            "fader_db",
            lane(
                &[(0, -10.0), (500, -5.0), (600, -4.0), (1000, 0.0)],
                "continuous",
            ),
        ),
    );
    // The two in the middle move together, 500 ms later: as far as the last point lets them.
    let (_, clamped) = edit(
        &mut project,
        points_edit(
            "fader_db",
            "move",
            json!({"at": [ms(500), ms(600)], "by": ms(500), "delta": 2.0}),
        ),
    );
    assert!(clamped);
    let points: Vec<(i64, f64)> = lane_of(&project, "A1", "fader_db")
        .unwrap()
        .points
        .iter()
        .map(|p| (p.at.ticks / 48, p.value))
        .collect();
    assert_eq!(
        points,
        vec![(0, -10.0), (900, -3.0), (1000, -2.0), (1000, 0.0)]
    );
    // Back, and up past +10 dB: kept at it.
    let (_, clamped) = edit(
        &mut project,
        points_edit(
            "fader_db",
            "move",
            json!({"at": [ms(0)], "by": {"ticks": -480, "scale": 48000}, "delta": 25.0}),
        ),
    );
    assert!(clamped);
    assert_eq!(
        lane_of(&project, "A1", "fader_db").unwrap().points[0].value,
        10.0
    );
    assert_eq!(
        lane_of(&project, "A1", "fader_db").unwrap().points[0]
            .at
            .ticks,
        0
    );
    assert_eq!(
        refused(
            &mut project,
            points_edit("fader_db", "move", json!({"at": [ms(0)]}))
        ),
        json!({"reason": "empty_range"})
    );
}

// The edits of ranges.

fn range_edit(path: &str, action: &str, from: i64, to: i64, extra: Value) -> Value {
    let mut edit = json!({"edit": "lane_range", "timeline": "t1", "of": {"track": "A1"},
        "path": path, "action": action, "from": ms(from), "to": ms(to)});
    for (key, value) in extra.as_object().unwrap() {
        edit[key] = value.clone();
    }
    edit
}

#[test]
fn a_range_taken_away_keeps_the_values_at_its_ends() {
    let mut project = project();
    op(
        &mut project,
        set_lane(
            json!({"track": "A1"}),
            "fader_db",
            lane(
                &[(0, -10.0), (1200, -2.0), (1600, -8.0), (2500, 0.0)],
                "continuous",
            ),
        ),
    );
    let before = project.clone();
    edit(
        &mut project,
        range_edit("fader_db", "delete", 1000, 2000, json!({})),
    );
    for at_ms in [0.0, 500.0, 999.0, 2000.0, 2200.0, 3000.0] {
        assert!(
            near(
                value(&project, "A1", "fader_db", at_ms),
                value(&before, "A1", "fader_db", at_ms)
            ),
            "{at_ms}"
        );
    }
    let points = &lane_of(&project, "A1", "fader_db").unwrap().points;
    assert!(points
        .iter()
        .all(|p| p.at.ticks <= 48 * 1000 || p.at.ticks >= 48 * 2000));
    // Between its ends, a straight line.
    let (left, right) = (
        value(&before, "A1", "fader_db", 1000.0),
        value(&before, "A1", "fader_db", 2000.0),
    );
    assert!(near(
        value(&project, "A1", "fader_db", 1500.0),
        f64::midpoint(left, right)
    ));
    // A cut is the same.
    let mut cut = before.clone();
    edit(
        &mut cut,
        range_edit("fader_db", "cut", 1000, 2000, json!({})),
    );
    assert_eq!(cut, project);
    assert_eq!(
        refused(
            &mut project,
            range_edit("fader_db", "delete", 2000, 2000, json!({}))
        ),
        json!({"reason": "empty_range"})
    );
    assert_eq!(
        refused(
            &mut project,
            range_edit("pan.x", "delete", 0, 2000, json!({}))
        ),
        json!({"reason": "missing", "what": "lane", "id": "pan.x"})
    );
}

#[test]
fn a_range_raised_ramps_in_and_out_over_10_ms_and_leaves_the_rest_as_it_was() {
    let mut project = project();
    edit(
        &mut project,
        range_edit(
            "fader_db",
            "shift_value",
            1000,
            2000,
            json!({"delta": -6.0}),
        ),
    );
    let points: Vec<(i64, f64)> = lane_of(&project, "A1", "fader_db")
        .unwrap()
        .points
        .iter()
        .map(|p| (p.at.ticks, p.value))
        .collect();
    assert_eq!(
        points,
        vec![(48000, 0.0), (48480, -6.0), (95520, -6.0), (96000, 0.0)]
    );
    // Again, over a part of it: what is in it moves; past +10 dB it is kept, and clamped.
    let (_, clamped) = edit(
        &mut project,
        range_edit(
            "fader_db",
            "shift_value",
            1500,
            3000,
            json!({"delta": 20.0}),
        ),
    );
    assert!(clamped);
    assert!(near(value(&project, "A1", "fader_db", 1200.0), -6.0));
    assert!(near(value(&project, "A1", "fader_db", 1800.0), 10.0));
    assert!(near(value(&project, "A1", "fader_db", 2500.0), 10.0));
    assert!(near(value(&project, "A1", "fader_db", 3000.0), 0.0));
    // A switch jumps at the ends of its range.
    edit(
        &mut project,
        range_edit("mute", "shift_value", 100, 200, json!({"delta": 1.0})),
    );
    for (at_ms, muted) in [(99.9, 0.0), (100.0, 1.0), (199.9, 1.0), (200.0, 0.0)] {
        assert_eq!(value(&project, "A1", "mute", at_ms), muted, "{at_ms}");
    }
}

#[test]
fn points_pasted_take_the_place_of_a_range_from_its_start() {
    let mut project = project();
    op(
        &mut project,
        set_lane(
            json!({"track": "A1"}),
            "pan.x",
            lane(&[(0, -1.0), (4000, 1.0)], "continuous"),
        ),
    );
    let before = project.clone();
    let pasted = json!([{"at": ms(0), "value": 0.0}, {"at": ms(500), "value": 0.5}]);
    edit(
        &mut project,
        range_edit("pan.x", "paste", 1000, 1500, json!({"points": pasted})),
    );
    assert!(near(
        value(&project, "A1", "pan.x", 999.0),
        value(&before, "A1", "pan.x", 999.0)
    ));
    assert!(near(value(&project, "A1", "pan.x", 1250.0), 0.25));
    assert!(near(
        value(&project, "A1", "pan.x", 1500.0),
        value(&before, "A1", "pan.x", 1500.0)
    ));
    assert!(near(value(&project, "A1", "pan.x", 3000.0), 0.5));
    // Points past the end of the range are refused.
    let long = json!([{"at": ms(600), "value": 0.0}]);
    assert_eq!(
        refused(
            &mut project,
            range_edit("pan.x", "paste", 1000, 1500, json!({"points": long}))
        ),
        json!({"reason": "out_of_range", "what": "points"})
    );
}

#[test]
fn a_lane_is_thinned_within_its_tolerance() {
    let mut project = project();
    let ramp: Vec<(i64, f64)> = (0..=100)
        .map(|k| (k * 10, -20.0 + f64::from(i32::try_from(k).unwrap()) * 0.2))
        .collect();
    op(
        &mut project,
        set_lane(
            json!({"track": "A1"}),
            "fader_db",
            lane(&ramp, "continuous"),
        ),
    );
    let (done, _) = edit(
        &mut project,
        range_edit("fader_db", "thin", 0, 1000, json!({})),
    );
    assert_eq!(lane_of(&project, "A1", "fader_db").unwrap().points.len(), 2);
    // One splice of the points between, whose inverse holds the 99 it took away.
    assert_eq!(done.len(), 1);
    let Op::SpliceLane {
        at,
        remove,
        ref points,
        ..
    } = done[0]
    else {
        panic!("{done:?}");
    };
    assert_eq!((at, remove, points.len()), (1, 99, 0));
}

#[test]
fn a_lane_drawn_is_thinned_and_takes_the_place_of_what_was_there() {
    let mut project = project();
    op(
        &mut project,
        set_lane(
            json!({"track": "A1"}),
            "fader_db",
            lane(&[(0, -6.0), (3000, -6.0)], "continuous"),
        ),
    );
    // A fade drawn from 1 s to 2 s, a point every millisecond, wobbling by less than 0.1 dB.
    let drawn: Vec<Value> = (1000..=2000)
        .map(|t| {
            let wobble = if t % 2 == 0 { 0.04 } else { -0.04 };
            json!({"at": ms(t), "value": -6.0 - f64::from(i32::try_from(t - 1000).unwrap()) * 0.01 + wobble})
        })
        .collect();
    let draw = json!({"edit": "lane_draw", "timeline": "t1", "of": {"track": "A1"},
        "path": "fader_db", "from": ms(1000), "to": ms(2000), "points": drawn});
    edit(&mut project, draw);
    let lane = lane_of(&project, "A1", "fader_db").unwrap();
    assert!(lane.points.len() <= 8, "{}", lane.points.len());
    for (t, expected) in [
        (500.0, -6.0),
        (1500.0, -11.0),
        (1999.0, -15.99),
        (2000.0, -6.0),
        (2500.0, -6.0),
    ] {
        assert!(
            (value(&project, "A1", "fader_db", t) - expected).abs() < 0.1,
            "{t}"
        );
    }
    // Out of order, or out of the range drawn.
    let back = json!({"edit": "lane_draw", "timeline": "t1", "of": {"track": "A1"}, "path": "fader_db",
        "from": ms(0), "to": ms(100), "points": [{"at": ms(50), "value": 0}, {"at": ms(40), "value": 0}]});
    assert_eq!(
        refused(&mut project, back),
        json!({"reason": "out_of_range", "what": "points"})
    );
}

// Clearing.

#[test]
fn lanes_cleared_leave_their_settings_as_set_reset_or_held() {
    let mut project = project();
    op(
        &mut project,
        set_lane(
            json!({"track": "A1"}),
            "fader_db",
            lane(&[(0, -10.0), (1000, 0.0)], "continuous"),
        ),
    );
    op(
        &mut project,
        set_lane(
            json!({"track": "A1"}),
            "send.B2.level_db",
            lane(&[(0, -20.0)], "continuous"),
        ),
    );
    op(
        &mut project,
        set_lane(json!({"track": "A1"}), "mute", lane(&[(0, 1.0)], "switch")),
    );
    let track = |p: &Project| p.timelines[0].tracks[1].audio.clone();
    let clear = |then: &str| json!({"edit": "clear_lanes", "timeline": "t1", "of": {"track": "A1"}, "then": then, "at": ms(250)});
    let mut kept = project.clone();
    edit(&mut kept, clear("keep"));
    assert!(track(&kept).lanes.is_empty());
    assert_eq!(track(&kept).strip.sends[0].level_db, -6.0);
    let mut held = project.clone();
    edit(&mut held, clear("hold"));
    assert!(track(&held).lanes.is_empty());
    assert!(near(track(&held).strip.fader_db, -7.5));
    assert_eq!(track(&held).strip.sends[0].level_db, -20.0);
    assert!(track(&held).mute);
    let mut reset = project.clone();
    op(
        &mut reset,
        json!({"op": "set_track_audio", "timeline": "t1", "track": "A1", "patch": {"strip": {"fader_db": -3}}}),
    );
    edit(
        &mut reset,
        json!({"edit": "clear_lanes", "timeline": "t1", "of": {"track": "A1"},
        "paths": ["fader_db", "send.B2.level_db"], "then": "reset"}),
    );
    assert_eq!(track(&reset).strip.fader_db, 0.0);
    assert_eq!(track(&reset).strip.sends[0].level_db, 0.0);
    assert_eq!(track(&reset).strip.sends[0].bus, "B2");
    assert_eq!(track(&reset).lanes.keys().collect::<Vec<_>>(), ["mute"]);
    // A VCA held.
    op(
        &mut project,
        set_lane(
            json!({"vca": "VCA1"}),
            "fader_db",
            lane(&[(0, -4.0)], "continuous"),
        ),
    );
    edit(
        &mut project,
        json!({"edit": "clear_lanes", "timeline": "t1", "of": {"vca": "VCA1"}, "then": "hold", "at": ms(0)}),
    );
    assert_eq!(project.timelines[0].vcas[0].fader_db, -4.0);
    assert_eq!(
        refused(
            &mut project,
            json!({"edit": "clear_lanes", "timeline": "t1", "of": {"track": "A1"}, "then": "hold"})
        ),
        json!({"reason": "out_of_range", "what": "at"})
    );
}

#[test]
fn a_lane_of_a_locked_track_is_not_edited() {
    let mut project = project();
    op(
        &mut project,
        json!({"op": "set_track", "timeline": "t1", "id": "A1", "locked": true}),
    );
    assert_eq!(
        refused(
            &mut project,
            points_edit("fader_db", "add", json!({"points": [{"at": ms(0)}]}))
        ),
        json!({"reason": "locked", "track": "A1"})
    );
}

// Inserts, and the lanes that name them.

fn with_insert_lanes() -> Project {
    let mut project = project();
    op(
        &mut project,
        json!({"op": "add_insert", "timeline": "t1", "of": {"track": "A1"},
            "insert": {"effect": {"kind": "gain", "gain_db": 0}}}),
    );
    op(
        &mut project,
        set_lane(
            json!({"track": "A1"}),
            "insert.fx1.gain_db",
            lane(&[(0, 6.0)], "continuous"),
        ),
    );
    op(
        &mut project,
        set_lane(
            json!({"track": "A1"}),
            "insert.fx2.on",
            lane(&[(0, 0.0)], "switch"),
        ),
    );
    project
}

fn paths(project: &Project) -> Vec<String> {
    project.timelines[0].tracks[1]
        .audio
        .lanes
        .keys()
        .cloned()
        .collect()
}

fn ids(project: &Project) -> Vec<String> {
    project.timelines[0].tracks[1]
        .audio
        .strip
        .inserts
        .iter()
        .map(|i| i.id.clone())
        .collect()
}

#[test]
fn an_insert_taken_away_takes_its_lanes_with_it_where_the_operation_alone_is_refused() {
    let mut project = with_insert_lanes();
    let take = json!({"op": "set_insert", "timeline": "t1", "of": {"track": "A1"}, "slot": 0, "insert": null});
    assert_eq!(
        refused_op(&mut project, take),
        json!({"reason": "out_of_range", "what": "lane"})
    );
    edit(
        &mut project,
        json!({"edit": "set_insert", "timeline": "t1", "of": {"track": "A1"}, "slot": 0, "insert": null}),
    );
    assert_eq!(paths(&project), ["insert.fx2.on"]);
    assert_eq!(ids(&project), ["fx2"]);
}

#[test]
fn an_insert_put_in_place_of_another_is_new_and_the_lanes_of_the_old_one_go() {
    let mut project = with_insert_lanes();
    // fx2 is replaced by a Gain, which would have been given fx2 again: it is given fx3.
    edit(
        &mut project,
        json!({"edit": "set_insert", "timeline": "t1", "of": {"track": "A1"}, "slot": 1,
        "insert": {"id": "fx2", "effect": {"kind": "gain", "gain_db": 1}}}),
    );
    assert_eq!(ids(&project), ["fx1", "fx3"]);
    assert_eq!(paths(&project), ["insert.fx1.gain_db"]);
}

#[test]
fn an_insert_moved_keeps_its_lanes_within_its_strip_and_leaves_them_for_another() {
    let mut project = with_insert_lanes();
    edit(
        &mut project,
        json!({"edit": "move_insert", "timeline": "t1", "of": {"track": "A1"}, "from": 0, "to": 1}),
    );
    assert_eq!(ids(&project), ["fx2", "fx1"]);
    assert_eq!(paths(&project), ["insert.fx1.gain_db", "insert.fx2.on"]);
    edit(
        &mut project,
        json!({"edit": "move_insert", "timeline": "t1", "of": {"track": "A1"}, "from": 1, "to": 0,
        "to_of": {"bus": "B2"}}),
    );
    assert_eq!(ids(&project), ["fx2"]);
    assert_eq!(paths(&project), ["insert.fx2.on"]);
    let bus = &project.timelines[0].buses[1];
    assert_eq!(bus.strip.inserts[0].id, "fx1");
    assert!(bus.lanes.is_empty());
}

#[test]
fn effects_copied_onto_a_strip_take_the_lanes_of_its_inserts_away() {
    let mut project = with_insert_lanes();
    op(
        &mut project,
        json!({"op": "add_insert", "timeline": "t1", "of": {"bus": "B2"},
            "insert": {"effect": {"kind": "soft_clipper"}}}),
    );
    // The copy is a new insert, fx3, above the ids of those that it replaces: no lane stays.
    edit(
        &mut project,
        json!({"edit": "copy_effects", "timeline": "t1", "from": {"bus": "B2"}, "to": [{"track": "A1"}]}),
    );
    assert_eq!(ids(&project), ["fx3"]);
    assert_eq!(paths(&project), Vec::<String>::new());
}

#[test]
fn a_send_taken_away_by_an_edit_takes_its_lanes_with_it() {
    let mut project = project();
    op(
        &mut project,
        set_lane(
            json!({"track": "A1"}),
            "send.B2.level_db",
            lane(&[(0, -20.0)], "continuous"),
        ),
    );
    op(
        &mut project,
        set_lane(
            json!({"track": "A1"}),
            "fader_db",
            lane(&[(0, -2.0)], "continuous"),
        ),
    );
    let take = json!({"op": "set_track_audio", "timeline": "t1", "track": "A1", "patch": {"strip": {"sends": []}}});
    assert_eq!(
        refused_op(&mut project, take),
        json!({"reason": "out_of_range", "what": "lane"})
    );
    edit(
        &mut project,
        json!({"edit": "set_track_audio", "timeline": "t1", "track": "A1", "patch": {"strip": {"sends": []}}}),
    );
    assert_eq!(paths(&project), ["fader_db"]);
}

// What an edit of a range leaves outside it.

mod outside {
    use super::{lane_of, project, set_lane};
    use columbina_model::{Edit, Project, Time};
    use proptest::prelude::*;
    use serde_json::{json, Value};

    fn value_at(project: &Project, path: &str, ticks: i64) -> f64 {
        lane_of(project, "A1", path).unwrap().value_at(Time {
            ticks,
            scale: 48000,
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        /// Taken away, pasted over, raised, drawn or thinned, a range of a lane leaves the lane
        /// before it and from its end on as it was (pp. 3898–3899), to the rounding of a line cut
        /// in two: of a fader or a mute, of points of a scale of their own (the range is put on
        /// the nearest ticks of it).
        #[test]
        fn an_edit_of_a_range_leaves_the_lane_outside_it_as_it_was(
            mut points in prop::collection::vec((0i64..20_000, -60.0f64..10.0), 1..12),
            jumps in any::<bool>(),
            switch in any::<bool>(),
            scale in prop::sample::select(vec![48_000i64, 44_100, 1000, 7]),
            (from, length) in (0i64..22_000, 1i64..8_000),
            action in 0usize..5,
            delta in -30.0f64..30.0,
        ) {
            let path = if switch { "mute" } else { "fader_db" };
            // The points at ticks of their scale, near the samples drawn.
            let mut points: Vec<(i64, f64)> = points
                .drain(..)
                .map(|(t, v)| (t * scale / 48_000, if switch { f64::from(u8::from(v > -25.0)) } else { v }))
                .collect();
            points.sort_by_key(|p| p.0);
            if jumps && points.len() > 2 {
                points[2].0 = points[1].0;
            }
            let lane: Vec<Value> = points
                .iter()
                .map(|&(t, v)| json!({"at": {"ticks": t, "scale": scale}, "value": v}))
                .collect();
            let kind = if switch { "switch" } else { "continuous" };
            let mut project = project();
            super::op(&mut project, set_lane(json!({"track": "A1"}), path,
                json!({"points": lane, "kind": kind})));
            let before = project.clone();
            let to = from + length;
            let time = |t: i64| json!({"ticks": t, "scale": 48000});
            let change = match action {
                0 => json!({"edit": "lane_range", "action": "delete"}),
                1 => json!({"edit": "lane_range", "action": "paste",
                    "points": [{"at": time(0), "value": 0.0}, {"at": time(length / 2), "value": 1.0}]}),
                2 => json!({"edit": "lane_range", "action": "shift_value", "delta": delta}),
                3 => json!({"edit": "lane_range", "action": "thin", "tolerance": 3.0}),
                _ => json!({"edit": "lane_draw",
                    "points": [{"at": time(from), "value": 0.0}, {"at": time(to), "value": 1.0}]}),
            };
            let mut change = change;
            for (key, value) in [("timeline", json!("t1")), ("of", json!({"track": "A1"})),
                ("path", json!(path)), ("from", time(from)), ("to", time(to))] {
                change[key] = value;
            }
            let change: Edit = serde_json::from_value(change).unwrap();
            if columbina_model::expand(&mut project, change).is_err() {
                // Nothing to change (a range of no tick of a coarse scale, or a lane as thin as
                // it was): nothing changed.
                prop_assert_eq!(&project, &before);
                return Ok(());
            }
            // The range as ticks of the scale of the lane, on the nearest.
            let tick = |t: i64| (2 * t * scale + 48_000) / 96_000;
            let (low, high) = (tick(from), tick(to));
            for t in (0..30_000).step_by(37).chain([from - 1, to, to + 1]) {
                // Within the range where the sample is in it as the lane counts.
                let place = i128::from(t) * i128::from(scale);
                if place >= i128::from(low) * 48_000 && place < i128::from(high) * 48_000 {
                    continue;
                }
                let (old, new) = (value_at(&before, path, t), value_at(&project, path, t));
                prop_assert!((old - new).abs() <= 1e-9 * (1.0 + old.abs()), "{t}: {old} {new}");
            }
        }
    }
}

#[test]
fn the_lookahead_of_a_ducker_has_no_lane_and_its_other_settings_have() {
    let mut project = project();
    op(
        &mut project,
        json!({"op": "set_track_audio", "timeline": "t1", "track": "A1",
            "patch": {"strip": {"track_fx": {"on": true}}}}),
    );
    op(
        &mut project,
        set_lane(
            json!({"track": "A1"}),
            "track_fx.amount_db",
            lane(&[(0, 12.0)], "continuous"),
        ),
    );
    op(
        &mut project,
        set_lane(
            json!({"track": "A1"}),
            "track_fx.on",
            lane(&[(0, 0.0)], "switch"),
        ),
    );
    assert_eq!(
        refused_op(
            &mut project,
            set_lane(
                json!({"track": "A1"}),
                "track_fx.lookahead_ms",
                lane(&[(0, 15.0)], "continuous")
            ),
        ),
        json!({"reason": "out_of_range", "what": "lane_not_wired"})
    );
    assert_eq!(
        refused(
            &mut project,
            points_edit(
                "track_fx.lookahead_ms",
                "add",
                json!({"points": [{"at": ms(0)}]})
            ),
        ),
        json!({"reason": "out_of_range", "what": "lane_not_wired"})
    );
}

#[test]
fn effects_copied_of_the_kind_of_those_they_replace_still_take_their_lanes_away() {
    // The copy of B2's Gain on A1, a Gain where a Gain had a lane: that lane is of the old
    // insert, and goes.
    let mut project = with_insert_lanes();
    op(
        &mut project,
        json!({"op": "add_insert", "timeline": "t1", "of": {"bus": "B2"},
            "insert": {"effect": {"kind": "gain", "gain_db": -9}}}),
    );
    edit(
        &mut project,
        json!({"edit": "copy_effects", "timeline": "t1", "from": {"bus": "B2"}, "to": [{"track": "A1"}]}),
    );
    assert_eq!(ids(&project), ["fx3"]);
    assert_eq!(paths(&project), Vec::<String>::new());
}

/// What a drawing of 1000 points costs, as the edit works it out. Run with
/// `cargo test --release -p columbina-model --test automation cost_of_a_drawing -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement, not a check"]
fn cost_of_a_drawing_of_1000_points() {
    let mut project = project();
    let drawn: Vec<Value> = (0..1000)
        .map(|k| {
            let wave = f64::from(i32::try_from(k % 40).unwrap()) * 0.5 - 10.0;
            json!({"at": ms(k), "value": wave})
        })
        .collect();
    let draw = json!({"edit": "lane_draw", "timeline": "t1", "of": {"track": "A1"},
        "path": "insert.fx1.gain_db", "from": ms(0), "to": ms(999), "points": drawn});
    let edit: Edit = serde_json::from_value(draw).unwrap();
    let begun = std::time::Instant::now();
    for _ in 0..10 {
        let mut copy = project.clone();
        columbina_model::expand(&mut copy, edit.clone()).unwrap();
    }
    println!(
        "a drawing of 1000 points: {:.2} ms",
        begun.elapsed().as_secs_f64() * 100.0
    );
    columbina_model::expand(&mut project, edit).unwrap();
}

// The review of T14.

/// A1 with two Gains, fx1 and fx2, and a lane of the second's gain at +6 dB.
fn two_gains_with_a_lane() -> Project {
    let mut project = Project::sample();
    for db in [-3, -6] {
        op(
            &mut project,
            json!({"op": "add_insert", "timeline": "t1", "of": {"track": "A1"},
                "insert": {"effect": {"kind": "gain", "gain_db": db}}}),
        );
    }
    op(
        &mut project,
        set_lane(
            json!({"track": "A1"}),
            "insert.fx2.gain_db",
            lane(&[(0, 6.0)], "continuous"),
        ),
    );
    project
}

#[test]
fn inserts_given_no_id_are_new_and_never_take_the_lanes_of_those_they_replace() {
    // The whole chain put in place of by a patch of two Gains of no id: they are fx3 and fx4, and
    // the lane of the old fx2 is refused by the operation, and taken away by the edit.
    let gains = json!([{"effect": {"kind": "gain", "gain_db": 0}},
        {"effect": {"kind": "gain", "gain_db": -20}}]);
    let patch = json!({"strip": {"inserts": gains}});
    let mut project = two_gains_with_a_lane();
    assert_eq!(
        refused_op(
            &mut project,
            json!({"op": "set_track_audio", "timeline": "t1", "track": "A1", "patch": patch}),
        ),
        json!({"reason": "out_of_range", "what": "lane"})
    );
    edit(
        &mut project,
        json!({"edit": "set_track_audio", "timeline": "t1", "track": "A1", "patch": patch}),
    );
    assert_eq!(ids(&project), ["fx3", "fx4"]);
    assert_eq!(paths(&project), Vec::<String>::new());
    // The second put in place of by the operation, with a Gain of no id: it is fx3, and the lane
    // of fx2 names nothing, so the operation is refused; the edit takes the lane away.
    let mut project = two_gains_with_a_lane();
    let put = json!({"timeline": "t1", "of": {"track": "A1"}, "slot": 1,
        "insert": {"effect": {"kind": "gain", "gain_db": -20}}});
    let mut set = put.clone();
    set["op"] = json!("set_insert");
    assert_eq!(
        refused_op(&mut project, set),
        json!({"reason": "out_of_range", "what": "lane"})
    );
    let mut set = put;
    set["edit"] = json!("set_insert");
    edit(&mut project, set);
    assert_eq!(ids(&project), ["fx1", "fx3"]);
    assert_eq!(paths(&project), Vec::<String>::new());
}

#[test]
fn an_insert_that_keeps_its_id_with_another_effect_leaves_no_lane_to_it() {
    // A patch that names fx2 again, now a Limiter, which has no gain_db; and again, now a Soft
    // Clipper whose lane of `on` would carry over: the operation refuses both, the edit takes
    // the lanes away.
    let mut project = two_gains_with_a_lane();
    op(
        &mut project,
        set_lane(
            json!({"track": "A1"}),
            "insert.fx2.on",
            lane(&[(0, 0.0)], "switch"),
        ),
    );
    let patch = json!({"strip": {"inserts": [
        {"id": "fx1", "effect": {"kind": "gain", "gain_db": -3}},
        {"id": "fx2", "effect": {"kind": "soft_clipper"}}]}});
    assert_eq!(
        refused_op(
            &mut project,
            json!({"op": "set_track_audio", "timeline": "t1", "track": "A1", "patch": patch}),
        ),
        json!({"reason": "out_of_range", "what": "lane"})
    );
    edit(
        &mut project,
        json!({"edit": "set_track_audio", "timeline": "t1", "track": "A1", "patch": patch}),
    );
    assert_eq!(ids(&project), ["fx1", "fx2"]);
    assert_eq!(paths(&project), Vec::<String>::new());
    // Only a lane of `on`, which a Soft Clipper has too: the operation still refuses it.
    let mut project = two_gains_with_a_lane();
    op(
        &mut project,
        set_lane(json!({"track": "A1"}), "insert.fx2.gain_db", Value::Null),
    );
    op(
        &mut project,
        set_lane(
            json!({"track": "A1"}),
            "insert.fx2.on",
            lane(&[(0, 0.0)], "switch"),
        ),
    );
    assert_eq!(
        refused_op(
            &mut project,
            json!({"op": "set_track_audio", "timeline": "t1", "track": "A1", "patch": patch}),
        ),
        json!({"reason": "out_of_range", "what": "lane"})
    );
}

#[test]
fn an_edit_that_changes_nothing_is_an_empty_range() {
    let mut project = project();
    op(
        &mut project,
        set_lane(
            json!({"track": "A1"}),
            "fader_db",
            lane(&[(0, -6.0), (1000, 0.0)], "continuous"),
        ),
    );
    assert_eq!(
        refused(
            &mut project,
            points_edit(
                "fader_db",
                "add",
                json!({"points": [{"at": ms(1000), "value": 0.0}]})
            ),
        ),
        json!({"reason": "empty_range"})
    );
    assert_eq!(
        refused(
            &mut project,
            range_edit("fader_db", "thin", 0, 1000, json!({}))
        ),
        json!({"reason": "empty_range"})
    );
}

#[test]
fn a_lane_of_a_frequency_is_made_on_a_scale_of_logarithms_and_goes_through_1_khz_half_way() {
    let mut project = project();
    let band = json!({"strip": {"eq": {"on": true}}});
    op(
        &mut project,
        json!({"op": "set_track_audio", "timeline": "t1", "track": "A1", "patch": band}),
    );
    let points = json!([{"at": ms(0), "value": 100.0}, {"at": ms(1000), "value": 10_000.0}]);
    edit(
        &mut project,
        json!({"edit": "lane_draw", "timeline": "t1", "of": {"track": "A1"}, "path": "eq.0.freq",
            "from": ms(0), "to": ms(1000), "points": points}),
    );
    let made = lane_of(&project, "A1", "eq.0.freq").unwrap();
    assert_eq!(made.scale, Some(columbina_model::LaneScale::Log));
    assert!((value(&project, "A1", "eq.0.freq", 500.0) - 1000.0).abs() < 1e-6);
    // A lane given with another scale than that of its path is refused.
    let mut wrong = lane(&[(0, 100.0)], "continuous");
    wrong["scale"] = json!("linear");
    assert_eq!(
        refused_op(
            &mut project,
            set_lane(json!({"track": "A1"}), "eq.1.freq", wrong)
        ),
        json!({"reason": "out_of_range", "what": "lane"})
    );
    // Whole numbers have no lane, and say so.
    assert_eq!(
        refused_op(
            &mut project,
            set_lane(
                json!({"track": "A1"}),
                "eq.0.slope",
                lane(&[(0, 12.0)], "continuous")
            )
        ),
        json!({"reason": "out_of_range", "what": "lane_integer"})
    );
}

#[test]
fn the_fade_that_the_agent_is_shown_fades_out_and_stays_silent() {
    // `video_edit`'s example: points added at 0 dB (2 s) and -inf (4 s) of a fader with no lane.
    let mut project = project();
    edit(
        &mut project,
        json!({"edit": "lane_points", "timeline": "t1", "of": {"track": "A1"}, "path": "fader_db",
            "action": "add", "points": [{"at": {"ticks": 96000, "scale": 48000}, "value": 0},
            {"at": {"ticks": 192_000, "scale": 48000}, "value": -100}]}),
    );
    assert_eq!(value(&project, "A1", "fader_db", 1000.0), 0.0);
    assert!(near(value(&project, "A1", "fader_db", 3000.0), -50.0));
    assert_eq!(value(&project, "A1", "fader_db", 5000.0), -100.0);
}
