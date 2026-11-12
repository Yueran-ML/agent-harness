//! The rest of the automation of a timeline (T16b; pp. 3906–3914, the interaction reference §11.4,
//! §11.6): the Automation Arm's Safe and Off besides armed; Fill Range and Glide Range; Copy,
//! Paste and Erase of automation over a range, within the categories of recording and leaving a
//! Safe arm alone; and the lanes of the tracks of a group that links Automation edited together.

#![allow(
    clippy::needless_pass_by_value,
    reason = "edits are given as a client gives them"
)]

use columbina_model::edit::{arm_of, copy_lanes};
use columbina_model::{
    apply, AutomationArm, Edit, Lane, LanesOf, Project, Time, Track, TrackGroup, TrackKind,
};
use serde_json::{json, Value};

/// A time of the timeline, in milliseconds, at 48 kHz.
fn ms(ms: i64) -> Value {
    json!({"ticks": ms * 48, "scale": 48000})
}

fn time(ms: i64) -> Time {
    Time {
        ticks: ms * 48,
        scale: 48000,
    }
}

fn lane(points: &[(i64, f64)], kind: &str) -> Lane {
    serde_json::from_value(json!({
        "points": points.iter().map(|&(t, v)| json!({"at": ms(t), "value": v}))
            .collect::<Vec<_>>(),
        "kind": kind,
        "scale": "linear",
    }))
    .unwrap()
}

/// The sample project with a second track of sound, A2, its fader at -6 dB; the fader of A1 on
/// a lane of `points` (milliseconds, dB), where there are any.
fn project(points: &[(i64, f64)]) -> Project {
    let mut project = Project::sample();
    let mut a2 = Track::new("A2", TrackKind::Audio);
    a2.audio.strip.fader_db = -6.0;
    project.timelines[0].tracks.push(a2);
    if !points.is_empty() {
        track(&mut project, "A1")
            .audio
            .lanes
            .insert("fader_db".into(), lane(points, "continuous"));
    }
    project.validate().unwrap();
    project
}

fn track<'p>(project: &'p mut Project, id: &str) -> &'p mut Track {
    project.timelines[0]
        .tracks
        .iter_mut()
        .find(|track| track.id == id)
        .unwrap()
}

/// A1 and A2 in a group linking `linked`.
fn grouped(project: &mut Project, linked: &[&str]) {
    let group: TrackGroup = serde_json::from_value(json!({
        "id": "TG1", "name": "Group 1", "members": ["A1", "A2"], "linked": linked,
    }))
    .unwrap();
    project.timelines[0].track_groups.push(group);
    project.validate().unwrap();
}

/// The edit applied as one transaction, taken back by its inverse in one step.
fn edit(project: &mut Project, edit: Value) {
    let before = project.clone();
    let edit: Edit = serde_json::from_value(edit).unwrap();
    let expansion =
        columbina_model::expand(project, edit.clone()).unwrap_or_else(|e| panic!("{edit:?}: {e}"));
    project.validate().unwrap();
    let mut undone = project.clone();
    for inverse in expansion.inverse.iter().rev() {
        apply(&mut undone, inverse.clone()).unwrap();
    }
    assert_eq!(undone, before, "taken back in one step");
}

/// The reason that the edit is refused, the project left as it was.
fn refused(project: &mut Project, edit: Value) -> Value {
    let before = project.clone();
    let edit: Edit = serde_json::from_value(edit).unwrap();
    let error = columbina_model::expand(project, edit).unwrap_err();
    assert_eq!(*project, before);
    serde_json::to_value(&error).unwrap()["reason"].clone()
}

/// The lane of `path` of track `id` at a time in milliseconds.
fn at(project: &mut Project, id: &str, path: &str, ms: f64) -> f64 {
    #[allow(clippy::cast_possible_truncation, reason = "a time of the test")]
    let ticks = (ms * 48.0).round() as i64;
    track(project, id).audio.lanes[path].value_at(Time {
        ticks,
        scale: 48000,
    })
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn check(project: &mut Project, id: &str, path: &str, want: &[(f64, f64)]) {
    for &(ms, want) in want {
        let got = at(project, id, path, ms);
        assert!(near(got, want), "{id} {path} at {ms} ms: {got}, not {want}");
    }
}

// The Automation Arm.

#[test]
fn an_arm_written_true_or_false_before_reads_as_armed_or_not_and_safe_and_off_are_kept() {
    let mut written = serde_json::to_value(project(&[])).unwrap();
    let tracks = written["timelines"][0]["tracks"].as_array_mut().unwrap();
    tracks[1]["automation_arm"] = json!(true);
    tracks[2]["automation_arm"] = json!(false);
    written["timelines"][0]["buses"][0]["automation_arm"] = json!("armed");
    let read: Project = serde_json::from_value(written).unwrap();
    let t = &read.timelines[0];
    assert_eq!(t.tracks[1].audio.automation_arm, AutomationArm::Armed);
    assert_eq!(t.tracks[2].audio.automation_arm, AutomationArm::Unarmed);
    assert_eq!(t.buses[0].automation_arm, AutomationArm::Armed);
    let mut project = read;
    edit(
        &mut project,
        json!({"edit": "set_track_audio", "timeline": "t1", "track": "A1",
            "patch": {"automation_arm": "safe"}}),
    );
    edit(
        &mut project,
        json!({"edit": "set_bus", "timeline": "t1", "id": "B1",
            "patch": {"automation_arm": "off"}}),
    );
    let again = serde_json::to_value(&project).unwrap();
    assert_eq!(again["timelines"][0]["tracks"][1]["automation_arm"], "safe");
    assert_eq!(again["timelines"][0]["tracks"][2]["automation_arm"], false);
    assert_eq!(again["timelines"][0]["buses"][0]["automation_arm"], "off");
    let back: Project = serde_json::from_value(again).unwrap();
    assert_eq!(back, project);
    let t = &project.timelines[0];
    assert_eq!(
        arm_of(t, &LanesOf::Track("A1".into())).unwrap(),
        AutomationArm::Safe
    );
    assert!(!AutomationArm::Safe.records() && !AutomationArm::Off.records());
    assert!(AutomationArm::Safe.plays() && !AutomationArm::Off.plays());
    assert!(!AutomationArm::Safe.pastes() && AutomationArm::Off.pastes());
    // A bus not armed writes none.
    let mut plain = serde_json::to_value(self::project(&[])).unwrap();
    assert!(plain["timelines"][0]["buses"][0]
        .as_object_mut()
        .unwrap()
        .remove("automation_arm")
        .is_none());
}

#[test]
fn the_arm_of_a_bus_or_of_a_vca_is_its_own_among_several() {
    let mut project = project(&[]);
    for op in [
        json!({"op": "add_bus", "timeline": "t1", "bus": {"id": "B2", "name": "Bus 2",
            "color": "#336699ff", "format": "stereo", "strip": {"outputs": [{"bus": "B1"}]}}}),
        json!({"op": "add_vca", "timeline": "t1", "vca": {"id": "VCA1", "name": "VCA 1"}}),
        json!({"op": "add_vca", "timeline": "t1", "vca": {"id": "VCA2", "name": "VCA 2"}}),
    ] {
        apply(&mut project, serde_json::from_value(op).unwrap()).unwrap();
    }
    let t = &mut project.timelines[0];
    t.buses
        .iter_mut()
        .find(|b| b.id == "B2")
        .unwrap()
        .automation_arm = AutomationArm::Safe;
    t.vcas[0].automation_arm = AutomationArm::Safe;
    t.vcas[1].automation_arm = AutomationArm::Off;
    let t = &project.timelines[0];
    for (of, arm) in [
        (LanesOf::Bus("B1".into()), AutomationArm::default()),
        (LanesOf::Bus("B2".into()), AutomationArm::Safe),
        (LanesOf::Vca("VCA1".into()), AutomationArm::Safe),
        (LanesOf::Vca("VCA2".into()), AutomationArm::Off),
    ] {
        assert_eq!(arm_of(t, &of).unwrap(), arm, "{of:?}");
    }
    assert!(arm_of(t, &LanesOf::Bus("B9".into())).is_err());
}

// Fill Range and Glide Range.

fn fill(lanes: Value, from: i64, to: i64, glide: bool) -> Value {
    json!({"edit": "lane_fill", "timeline": "t1", "lanes": lanes, "from": ms(from),
        "to": ms(to), "glide": glide})
}

#[test]
fn fill_range_writes_the_value_over_the_range_ramping_in_and_out_over_ten_milliseconds() {
    let mut project = project(&[(0, -10.0)]);
    edit(
        &mut project,
        fill(
            json!([{"of": {"track": "A1"}, "path": "fader_db", "value": -4.0}]),
            1000,
            2000,
            false,
        ),
    );
    check(
        &mut project,
        "A1",
        "fader_db",
        &[
            (999.0, -10.0),
            (1000.0, -10.0),
            (1005.0, -7.0),
            (1010.0, -4.0),
            (1500.0, -4.0),
            (2000.0, -4.0),
            (2005.0, -7.0),
            (2010.0, -10.0),
            (3000.0, -10.0),
        ],
    );
}

#[test]
fn glide_range_goes_from_the_value_at_the_start_on_a_straight_line_to_the_value_at_the_end() {
    let mut project = project(&[(0, -10.0)]);
    edit(
        &mut project,
        fill(
            json!([{"of": {"track": "A1"}, "path": "fader_db", "value": -4.0}]),
            1000,
            3000,
            true,
        ),
    );
    check(
        &mut project,
        "A1",
        "fader_db",
        &[
            (1000.0, -10.0),
            (1010.0, -10.0),
            (2005.0, -7.0),
            (3000.0, -4.0),
            (3005.0, -7.0),
            (3010.0, -10.0),
        ],
    );
    // A lane that is not there is made from the setting's value: A2 at -6.
    edit(
        &mut project,
        fill(
            json!([{"of": {"track": "A2"}, "path": "fader_db", "value": 0.0}]),
            1000,
            2000,
            true,
        ),
    );
    check(
        &mut project,
        "A2",
        "fader_db",
        &[
            (500.0, -6.0),
            (1010.0, -6.0),
            (1505.0, -3.0),
            (2000.0, 0.0),
            (2500.0, -6.0),
        ],
    );
}

#[test]
fn fill_range_leaves_a_safe_track_alone() {
    let mut project = project(&[(0, -10.0)]);
    track(&mut project, "A1").audio.automation_arm = AutomationArm::Safe;
    let lanes = json!([
        {"of": {"track": "A1"}, "path": "fader_db", "value": -4.0},
        {"of": {"track": "A2"}, "path": "fader_db", "value": -1.0},
    ]);
    edit(&mut project, fill(lanes, 1000, 2000, false));
    check(&mut project, "A1", "fader_db", &[(1500.0, -10.0)]);
    check(&mut project, "A2", "fader_db", &[(1500.0, -1.0)]);
    let only_safe = json!([{"of": {"track": "A1"}, "path": "fader_db", "value": -4.0}]);
    assert_eq!(
        refused(&mut project, fill(only_safe, 1000, 2000, false)),
        "empty_range"
    );
}

#[test]
fn a_range_filled_again_with_the_value_that_it_has_changes_nothing() {
    let mut project = project(&[(0, -10.0)]);
    let lanes = json!([{"of": {"track": "A1"}, "path": "fader_db", "value": -4.0}]);
    edit(&mut project, fill(lanes.clone(), 1000, 2000, false));
    assert_eq!(
        refused(&mut project, fill(lanes, 1000, 2000, false)),
        "empty_range"
    );
}

// Copy, Paste and Erase.

#[test]
fn copy_takes_the_points_of_a_range_from_its_start_with_its_values_at_both_ends() {
    let mut project = project(&[(0, -10.0), (1500, -4.0), (2500, -8.0)]);
    track(&mut project, "A1")
        .audio
        .lanes
        .insert("mute".into(), lane(&[(0, 0.0), (1200, 1.0)], "switch"));
    let t = &project.timelines[0];
    let of = [LanesOf::Track("A1".into()), LanesOf::Track("A2".into())];
    let copied = copy_lanes(t, &of, Some((time(1000), time(2000))), None).unwrap();
    let copied = serde_json::to_value(&copied).unwrap();
    assert_eq!(copied.as_array().unwrap().len(), 2, "{copied}");
    let fader = &copied[0];
    assert_eq!(fader["path"], "fader_db");
    let points: Vec<(i64, f64)> = fader["points"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["at"]["ticks"].as_i64().unwrap() / 48,
                p["value"].as_f64().unwrap(),
            )
        })
        .collect();
    assert_eq!(points.len(), 3);
    assert_eq!(points[0].0, 0);
    assert!(near(points[0].1, -6.0));
    assert_eq!(points[1], (500, -4.0));
    assert_eq!(points[2].0, 1000);
    assert!(near(points[2].1, -6.0));
    assert_eq!(copied[1]["path"], "mute");
    // A snapshot: the value at the playhead.
    let snapshot = copy_lanes(t, &of, None, Some(time(1500))).unwrap();
    assert_eq!(snapshot[0].value, Some(-4.0));
    assert_eq!(snapshot[1].value, Some(1.0));
    assert!(snapshot[0].points.is_none());
    // Only the categories of recording that are on.
    // A static mix too: A2's fader, with no lane, at -6, its mute off, its pan centred.
    let a2 = |path: &str| {
        snapshot
            .iter()
            .find(|c| c.of == LanesOf::Track("A2".into()) && c.path == path)
            .and_then(|c| c.value)
    };
    assert_eq!(a2("fader_db"), Some(-6.0));
    assert_eq!(a2("mute"), Some(0.0));
    assert_eq!(a2("pan.x"), Some(0.0));
    // Nothing of A2 that is not of the mix (its equalizer), nor A1's fader again.
    assert_eq!(a2("eq.1.gain_db"), None);
    assert_eq!(
        snapshot
            .iter()
            .filter(|c| c.of == LanesOf::Track("A1".into()) && c.path == "fader_db")
            .count(),
        1
    );
    let mut faders_off = project.clone();
    faders_off.timelines[0].audio.record.categories.fader = false;
    let some = copy_lanes(&faders_off.timelines[0], &of, None, Some(time(1500))).unwrap();
    assert!(some.iter().all(|c| c.path != "fader_db"));
    assert!(some.iter().any(|c| c.path == "mute"));
    let reason = |e: columbina_model::OpError| serde_json::to_value(e).unwrap()["what"].clone();
    assert_eq!(reason(copy_lanes(t, &of, None, None).unwrap_err()), "at");
    let range = Some((time(1000), time(2000)));
    assert_eq!(
        reason(copy_lanes(t, &of, range, Some(time(1500))).unwrap_err()),
        "at"
    );
    let far = Time {
        ticks: i64::MAX,
        scale: 1,
    };
    assert_eq!(
        reason(copy_lanes(t, &of, Some((far, time(2000))), None).unwrap_err()),
        "from"
    );
}

#[test]
fn a_range_copied_from_one_point_to_another_has_each_of_them_once() {
    let project = project(&[(0, -10.0), (1500, -4.0), (2500, -8.0)]);
    let of = [LanesOf::Track("A1".into())];
    let range = Some((time(1500), time(2500)));
    let copied = copy_lanes(&project.timelines[0], &of, range, None).unwrap();
    let points: Vec<(i64, f64)> = copied[0]
        .points
        .as_ref()
        .unwrap()
        .iter()
        .map(|p| (p.at.ticks / 48, p.value))
        .collect();
    assert_eq!(points, vec![(0, -4.0), (1000, -8.0)]);
}

fn paste(lanes: Value, at: i64, to: Option<i64>) -> Value {
    let mut edit = json!({"edit": "automation_paste", "timeline": "t1", "lanes": lanes,
        "at": ms(at)});
    if let Some(to) = to {
        edit["to"] = ms(to);
    }
    edit
}

#[test]
fn paste_puts_the_range_copied_on_another_track_filling_the_range_with_its_last_value() {
    let mut project = project(&[(0, -10.0), (1500, -4.0), (2500, -8.0)]);
    let of = [LanesOf::Track("A1".into())];
    let copied = copy_lanes(
        &project.timelines[0],
        &of,
        Some((time(1000), time(2000))),
        None,
    )
    .unwrap();
    let mut lanes = serde_json::to_value(&copied).unwrap();
    lanes[0]["of"] = json!({"track": "A2"});
    // Over a range longer than what was copied: its last value held to the end.
    edit(&mut project, paste(lanes.clone(), 5000, Some(6500)));
    check(
        &mut project,
        "A2",
        "fader_db",
        &[
            (4000.0, -6.0),
            (5000.0, -6.0),
            (5250.0, -5.0),
            (5500.0, -4.0),
            (6000.0, -6.0),
            (6400.0, -6.0),
            (7000.0, -6.0),
        ],
    );
    // With no range, whole from the time; over a shorter range, cut at its end.
    edit(&mut project, paste(lanes.clone(), 8000, None));
    check(
        &mut project,
        "A2",
        "fader_db",
        &[(8250.0, -5.0), (8500.0, -4.0), (9000.0, -6.0)],
    );
    edit(&mut project, paste(lanes, 10_000, Some(10_250)));
    check(
        &mut project,
        "A2",
        "fader_db",
        &[
            (10_000.0, -6.0),
            (10_200.0, -5.2),
            (10_240.0, -5.04),
            (10_250.0, -6.0),
        ],
    );
    // A snapshot: its value over a range, or one point where there is none.
    let snapshot = json!([{"of": {"track": "A1"}, "path": "fader_db", "value": -3.0}]);
    edit(&mut project, paste(snapshot.clone(), 1000, Some(2000)));
    check(
        &mut project,
        "A1",
        "fader_db",
        &[
            (999.0, -10.0 + 6.0 * 999.0 / 1500.0),
            (1500.0, -3.0),
            (1990.0, -3.0),
            (1999.0, -5.7),
            (2500.0, -8.0),
        ],
    );
    // With no range, a snapshot is refused rather than reshaping the lane around a point.
    let refusal = serde_json::to_value(
        columbina_model::expand(
            &mut project.clone(),
            serde_json::from_value(paste(snapshot, 3000, None)).unwrap(),
        )
        .unwrap_err(),
    )
    .unwrap();
    assert_eq!(refusal["what"], "to", "{refusal}");
    // A lane of no length among others is passed over.
    let lanes = json!([
        {"of": {"track": "A2"}, "path": "fader_db", "points": [{"at": ms(0), "value": -1.0}]},
        {"of": {"track": "A1"}, "path": "fader_db",
            "points": [{"at": ms(0), "value": -2.0}, {"at": ms(100), "value": -2.0}]},
    ]);
    edit(&mut project, paste(lanes, 20_000, None));
    check(&mut project, "A1", "fader_db", &[(20_050.0, -2.0)]);
}

#[test]
fn pasted_snapshot_ramps_at_both_edges_of_the_range() {
    let mut project = project(&[(0, -10.0), (3000, -10.0)]);
    edit(
        &mut project,
        paste(
            json!([{"of": {"track": "A1"}, "path": "fader_db", "value": -2.0}]),
            1000,
            Some(2000),
        ),
    );
    check(
        &mut project,
        "A1",
        "fader_db",
        &[
            (999.0, -10.0),
            (1000.0, -10.0),
            (1005.0, -6.0),
            (1010.0, -2.0),
            (1990.0, -2.0),
            (1995.0, -6.0),
            (2000.0, -10.0),
            (2001.0, -10.0),
        ],
    );
}

#[test]
fn a_truncated_paste_keeps_logarithmic_and_switch_interpolation() {
    for (path, values, want) in [
        (
            "eq.0.freq",
            [100.0, 10000.0],
            100.0_f64 * 100.0_f64.powf(0.49),
        ),
        ("mute", [0.0, 1.0], 0.0),
    ] {
        let mut project = project(&[]);
        let points = json!([
            {"at": ms(0), "value": values[0]}, {"at": ms(1000), "value": values[1]}
        ]);
        edit(
            &mut project,
            paste(
                json!([{"of": {"track": "A1"}, "path": path, "points": points}]),
                1000,
                Some(1500),
            ),
        );
        let got = at(&mut project, "A1", path, 1490.0);
        assert!((got - want).abs() < 1e-6, "{path}: {got}, not {want}");
    }
}

#[test]
fn paste_preserves_source_keys_in_the_destination_lanes_timebase() {
    let mut project = project(&[(0, -10.0)]);
    track(&mut project, "A1")
        .audio
        .lanes
        .get_mut("fader_db")
        .unwrap()
        .points[0]
        .at
        .scale = 96000;
    let key = Time {
        ticks: 4801,
        scale: 96000,
    };
    let points = json!([
        {"at": ms(0), "value": -6.0}, {"at": key, "value": -12.0},
        {"at": ms(100), "value": -6.0}
    ]);
    edit(
        &mut project,
        paste(
            json!([{"of": {"track": "A1"}, "path": "fader_db", "points": points}]),
            1000,
            None,
        ),
    );
    let lane = &track(&mut project, "A1").audio.lanes["fader_db"];
    assert!(lane.points.iter().all(|p| p.at.scale == 96000));
    let pasted = lane
        .points
        .iter()
        .find(|p| p.at.ticks == 96000 + key.ticks)
        .unwrap();
    assert_eq!(pasted.value, -12.0);
    assert!(near(
        lane.value_at(Time {
            ticks: 96480,
            scale: 96000
        }),
        (-10.0 - 6.0 - 6.0 * 960.0 / 4801.0) / 2.0
    ));
}

#[test]
fn pasted_switches_step_and_continuous_values_are_clamped_before_ramping() {
    let mut project = project(&[]);
    edit(
        &mut project,
        paste(
            json!([
                {"of": {"track": "A1"}, "path": "mute", "points": [
                    {"at": ms(0), "value": 0.8}, {"at": ms(50), "value": 0.2}, {"at": ms(100), "value": 0.8}]},
                {"of": {"track": "A1"}, "path": "fader_db", "value": 50.0}
            ]),
            1000,
            Some(1100),
        ),
    );
    check(
        &mut project,
        "A1",
        "mute",
        &[
            (999.0, 0.0),
            (1000.0, 1.0),
            (1049.0, 1.0),
            (1050.0, 0.0),
            (1099.0, 0.0),
            (1100.0, 0.0),
        ],
    );
    check(
        &mut project,
        "A1",
        "fader_db",
        &[
            (1000.0, 0.0),
            (1005.0, 5.0),
            (1010.0, 10.0),
            (1095.0, 5.0),
            (1100.0, 0.0),
        ],
    );
}

#[test]
fn a_one_tick_paste_range_is_judged_in_the_destination_lane_timebase() {
    let mut project = project(&[(0, -10.0)]);
    track(&mut project, "A1")
        .audio
        .lanes
        .get_mut("fader_db")
        .unwrap()
        .points[0]
        .at
        .scale = 96000;
    let start = Time {
        ticks: 96001,
        scale: 96000,
    };
    let end = Time {
        ticks: 96002,
        scale: 96000,
    };
    let mut expected = project.clone();
    edit(
        &mut expected,
        json!({"edit": "lane_range", "timeline": "t1", "of": {"track": "A1"},
        "path": "fader_db", "action": "paste", "from": start, "to": end,
        "points": [{"at": {"ticks": 0, "scale": 96000}, "value": -2.0},
                   {"at": {"ticks": 1, "scale": 96000}, "value": -2.0}]}),
    );
    edit(
        &mut project,
        json!({"edit": "automation_paste", "timeline": "t1", "at": start, "to": end,
        "lanes": [{"of": {"track": "A1"}, "path": "fader_db", "value": -2.0}]}),
    );
    assert_eq!(project, expected);
    let lane = &track(&mut project, "A1").audio.lanes["fader_db"];
    assert!(near(lane.value_at(start), -2.0));
    assert!(near(lane.value_at(end), -10.0));
}

#[test]
fn a_paste_clipped_at_a_source_jump_preserves_the_curve_approaching_it() {
    let mut project = project(&[(0, 0.0)]);
    let points = json!([
        {"at": ms(0), "value": -6.0},
        {"at": ms(500), "value": -6.0},
        {"at": ms(500), "value": 0.0},
        {"at": ms(1000), "value": 0.0}
    ]);
    edit(
        &mut project,
        paste(
            json!([{"of": {"track": "A1"}, "path": "fader_db", "points": points}]),
            1000,
            Some(1500),
        ),
    );
    check(
        &mut project,
        "A1",
        "fader_db",
        &[
            (1010.0, -6.0),
            (1250.0, -6.0),
            (1490.0, -6.0),
            (1495.0, -3.0),
            (1500.0, 0.0),
        ],
    );
}

#[test]
fn paste_leaves_a_safe_track_and_a_category_that_is_off_alone() {
    let mut project = project(&[(0, -10.0)]);
    let snapshot = json!([{"of": {"track": "A1"}, "path": "fader_db", "value": -3.0}]);
    track(&mut project, "A1").audio.automation_arm = AutomationArm::Safe;
    assert_eq!(
        refused(&mut project, paste(snapshot.clone(), 1000, Some(2000))),
        "empty_range"
    );
    track(&mut project, "A1").audio.automation_arm = AutomationArm::Off;
    project.timelines[0].audio.record.categories.fader = false;
    assert_eq!(
        refused(&mut project, paste(snapshot.clone(), 1000, Some(2000))),
        "empty_range"
    );
    project.timelines[0].audio.record.categories.fader = true;
    edit(&mut project, paste(snapshot, 1000, Some(2000)));
    check(&mut project, "A1", "fader_db", &[(1500.0, -3.0)]);
}

#[test]
fn a_paste_is_at_a_time_from_0_on_to_a_later_one_and_of_points_from_0_on_in_order() {
    let mut project = project(&[(0, -10.0), (3000, -10.0)]);
    let snapshot = json!([{"of": {"track": "A1"}, "path": "fader_db", "value": -3.0}]);
    let odd = |at: Value, to: Value| {
        json!({"edit": "automation_paste", "timeline": "t1", "lanes": snapshot.clone(),
            "at": at, "to": to})
    };
    // Before the start of the timeline, by however little.
    let before = json!({"ticks": -1, "scale": 144_000});
    assert_eq!(refused(&mut project, odd(before, ms(2000))), "out_of_range");
    let before = json!({"ticks": -1, "scale": 48000});
    assert_eq!(refused(&mut project, odd(ms(0), before)), "out_of_range");
    // To the time that it is at, or to an earlier one: no range, said before a lane is looked at.
    assert_eq!(refused(&mut project, odd(ms(0), ms(0))), "empty_range");
    let nowhere = json!([{"of": {"track": "A9"}, "path": "fader_db", "value": -3.0}]);
    assert_eq!(
        refused(&mut project, paste(nowhere, 2000, Some(1000))),
        "empty_range"
    );
    // Points before 0, or out of order.
    let points = |a: i64, b: i64| {
        json!([{"of": {"track": "A1"}, "path": "fader_db", "points": [
            {"at": ms(a), "value": -3.0}, {"at": ms(b), "value": -6.0}]}])
    };
    assert_eq!(
        refused(&mut project, paste(points(-100, 500), 1000, Some(2000))),
        "out_of_range"
    );
    assert_eq!(
        refused(&mut project, paste(points(500, 200), 1000, None)),
        "out_of_range"
    );
    // From 0.
    edit(&mut project, paste(snapshot.clone(), 0, Some(1000)));
    check(&mut project, "A1", "fader_db", &[(500.0, -3.0)]);
}

fn erase(of: Value, from: i64, to: i64) -> Value {
    json!({"edit": "automation_erase", "timeline": "t1", "of": of, "from": ms(from),
        "to": ms(to)})
}

#[test]
fn erase_takes_the_points_of_a_range_away_keeping_the_values_at_its_ends() {
    let mut project = project(&[(0, -10.0), (1500, -4.0), (2500, -8.0)]);
    track(&mut project, "A1")
        .audio
        .lanes
        .insert("mute".into(), lane(&[(0, 0.0), (1200, 1.0)], "switch"));
    project.timelines[0].audio.record.categories.mute = false;
    edit(
        &mut project,
        erase(json!([{"track": "A1"}, {"track": "A2"}]), 1000, 2000),
    );
    check(
        &mut project,
        "A1",
        "fader_db",
        &[
            (1000.0, -6.0),
            (1500.0, -6.0),
            (2000.0, -6.0),
            (2500.0, -8.0),
        ],
    );
    // The mute, its category off, stays.
    check(&mut project, "A1", "mute", &[(1500.0, 1.0)]);
    track(&mut project, "A1").audio.automation_arm = AutomationArm::Safe;
    assert_eq!(
        refused(&mut project, erase(json!([{"track": "A1"}]), 0, 3000)),
        "empty_range"
    );
}

// Groups that link Automation.

#[test]
fn the_control_of_a_group_that_links_a_setting_is_that_of_its_path() {
    use columbina_model::edit::group_control;
    use columbina_model::GroupControl;
    for (path, control) in [
        ("fader_db", Some(GroupControl::Fader)),
        ("mute", Some(GroupControl::Mute)),
        ("pan.x", Some(GroupControl::Panning)),
        ("send.B2.level_db", Some(GroupControl::Sends)),
        ("send.B2.pan.x", Some(GroupControl::Sends)),
        ("eq.0.freq", None),
        ("trim_db", None),
    ] {
        assert_eq!(group_control(path), control, "{path}");
    }
}

fn draw(grouped: bool) -> Value {
    json!({"edit": "lane_draw", "timeline": "t1", "of": {"track": "A1"}, "path": "fader_db",
        "from": ms(1000), "to": ms(2000),
        "points": [{"at": ms(1000), "value": -3.0}, {"at": ms(2000), "value": -3.0}],
        "tolerance": 0.0, "grouped": grouped})
}

#[test]
fn a_lane_drawn_grouped_is_drawn_on_the_other_tracks_of_a_group_linking_automation_relative() {
    let mut project = project(&[]);
    grouped(&mut project, &["fader", "mute", "automation"]);
    edit(&mut project, draw(true));
    // A1's fader is at 0 dB, A2's at -6: A2 is drawn 6 dB below.
    check(&mut project, "A1", "fader_db", &[(1500.0, -3.0)]);
    check(
        &mut project,
        "A2",
        "fader_db",
        &[(500.0, -6.0), (1500.0, -9.0), (2500.0, -6.0)],
    );
    // Raised together by the same change.
    edit(
        &mut project,
        json!({"edit": "lane_range", "timeline": "t1", "of": {"track": "A1"},
            "path": "fader_db", "from": ms(1200), "to": ms(1800), "action": "shift_value",
            "delta": 2.0, "grouped": true}),
    );
    check(&mut project, "A1", "fader_db", &[(1500.0, -1.0)]);
    check(&mut project, "A2", "fader_db", &[(1500.0, -7.0)]);
    // A switch to the same state.
    edit(
        &mut project,
        json!({"edit": "lane_points", "timeline": "t1", "of": {"track": "A1"}, "path": "mute",
            "action": "add", "points": [{"at": ms(1000), "value": 1.0}], "grouped": true}),
    );
    check(&mut project, "A2", "mute", &[(1000.0, 1.0)]);
}

#[test]
fn a_lane_drawn_is_drawn_alone_without_grouped_a_group_linking_automation_or_its_control() {
    for (linked, flag) in [
        (&["fader", "automation"][..], false),
        (&["fader"][..], true),
        (&["automation", "mute"][..], true),
    ] {
        let mut project = project(&[]);
        grouped(&mut project, linked);
        edit(&mut project, draw(flag));
        assert!(
            !track(&mut project, "A2")
                .audio
                .lanes
                .contains_key("fader_db"),
            "{linked:?} {flag}"
        );
    }
    // Nor on a member that is locked, nor while the group is not active.
    let mut project = project(&[]);
    grouped(&mut project, &["fader", "automation"]);
    track(&mut project, "A2").locked = true;
    edit(&mut project, draw(true));
    assert!(!track(&mut project, "A2")
        .audio
        .lanes
        .contains_key("fader_db"));
    let mut project = self::project(&[]);
    grouped(&mut project, &["fader", "automation"]);
    project.timelines[0].track_groups[0].active = false;
    edit(&mut project, draw(true));
    assert!(!track(&mut project, "A2")
        .audio
        .lanes
        .contains_key("fader_db"));
}

#[test]
fn a_pan_follows_a_group_by_the_same_amount_however_far() {
    // A1's divergence is at 10 %, A2's at 20 %: A1's set to 50 % moves A2's by 40, to 60.
    let mut project = project(&[]);
    grouped(&mut project, &["panning", "automation"]);
    track(&mut project, "A1").audio.strip.pan.divergence = 10.0;
    track(&mut project, "A2").audio.strip.pan.divergence = 20.0;
    edit(
        &mut project,
        json!({"edit": "lane_points", "timeline": "t1", "of": {"track": "A1"},
            "path": "pan.divergence", "action": "add",
            "points": [{"at": ms(1000), "value": 50.0}], "grouped": true}),
    );
    check(&mut project, "A1", "pan.divergence", &[(1000.0, 50.0)]);
    check(&mut project, "A2", "pan.divergence", &[(1000.0, 60.0)]);
}

#[test]
fn the_level_of_a_send_follows_a_group_as_a_fader_does_from_minus_infinity() {
    // A1's send at -120 dB (-inf), A2's at -110: A1's set to -6 moves A2's by 94 dB, to -16.
    let mut project = project(&[]);
    let bus = json!({"op": "add_bus", "timeline": "t1", "bus": {"id": "B2", "name": "Bus 2",
        "color": "#336699ff", "format": "stereo", "strip": {"outputs": [{"bus": "B1"}]}}});
    apply(&mut project, serde_json::from_value(bus).unwrap()).unwrap();
    for (id, level) in [("A1", -120.0), ("A2", -110.0)] {
        let send = json!({"op": "set_track_audio", "timeline": "t1", "track": id,
            "patch": {"strip": {"sends": [{"bus": "B2", "level_db": level}]}}});
        apply(&mut project, serde_json::from_value(send).unwrap()).unwrap();
    }
    grouped(&mut project, &["sends", "automation"]);
    edit(
        &mut project,
        json!({"edit": "lane_points", "timeline": "t1", "of": {"track": "A1"},
            "path": "send.B2.level_db", "action": "add",
            "points": [{"at": ms(1000), "value": -6.0}], "grouped": true}),
    );
    check(&mut project, "A1", "send.B2.level_db", &[(1000.0, -6.0)]);
    check(&mut project, "A2", "send.B2.level_db", &[(1000.0, -16.0)]);
}

#[test]
fn a_level_follows_a_group_as_a_fader_does_from_minus_infinity() {
    // A1 at -120 dB (-inf), A2 at -50: A1 drawn to -60 moves A2 by 40 dB, to -10, not +10.
    let mut project = project(&[]);
    grouped(&mut project, &["fader", "automation"]);
    track(&mut project, "A1").audio.strip.fader_db = -120.0;
    track(&mut project, "A2").audio.strip.fader_db = -50.0;
    let mut drawn = draw(true);
    drawn["points"] = json!([{"at": ms(1000), "value": -60.0}, {"at": ms(2000), "value": -60.0}]);
    edit(&mut project, drawn);
    check(&mut project, "A2", "fader_db", &[(1500.0, -10.0)]);
    // A2 at -inf stays there as A1 goes down.
    let mut project = self::project(&[]);
    grouped(&mut project, &["fader", "automation"]);
    track(&mut project, "A2").audio.strip.fader_db = -100.0;
    edit(&mut project, draw(true));
    check(&mut project, "A2", "fader_db", &[(1500.0, -100.0)]);
}

#[test]
fn fill_range_writes_neither_safe_nor_off_and_an_unknown_arm_is_refused_clearly() {
    let mut project = project(&[(0, -10.0)]);
    track(&mut project, "A2").audio.automation_arm = AutomationArm::Off;
    let lanes = json!([
        {"of": {"track": "A1"}, "path": "fader_db", "value": -4.0},
        {"of": {"track": "A2"}, "path": "fader_db", "value": -1.0},
    ]);
    edit(&mut project, fill(lanes, 1000, 2000, false));
    check(&mut project, "A1", "fader_db", &[(1500.0, -4.0)]);
    assert!(!track(&mut project, "A2")
        .audio
        .lanes
        .contains_key("fader_db"));
    let error = serde_json::from_value::<AutomationArm>(json!("ready")).unwrap_err();
    assert!(error.to_string().contains("\"safe\" or \"off\""), "{error}");
}
