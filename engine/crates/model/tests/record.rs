//! A pass of the recording of automation written into lanes (T16, pp. 3905–3909): each run of a
//! control touched puts what it recorded in place of the lane from where it started, and then
//! returns to the lane over a glide (Snap's release, Return), holds its last value to the end
//! (Hold) or to the next point of the lane (Event), or gives the lane back at once (the end of the
//! Active Range); a Trim moves the lane by what it recorded, in decibels; and the whole pass is
//! one step of undo.

#![allow(
    clippy::needless_pass_by_value,
    reason = "runs are given as a client gives them"
)]

use columbina_model::{apply, Edit, Lane, Project, Time};
use proptest::prelude::*;
use serde_json::{json, Value};

/// A time of the timeline, in milliseconds, at 48 kHz.
fn ms(ms: i64) -> Value {
    json!({"ticks": ms * 48, "scale": 48000})
}

/// The sample project, the fader of A1 on a lane of `points` (milliseconds, dB), or none.
fn project(points: &[(i64, f64)]) -> Project {
    let mut project = Project::sample();
    if !points.is_empty() {
        let lane: Lane = serde_json::from_value(json!({
            "points": points.iter().map(|&(t, v)| json!({"at": ms(t), "value": v}))
                .collect::<Vec<_>>(),
            "kind": "continuous",
        }))
        .unwrap();
        a1(&mut project).audio.lanes.insert("fader_db".into(), lane);
    }
    project.validate().unwrap();
    project
}

fn a1(project: &mut Project) -> &mut columbina_model::Track {
    project.timelines[0]
        .tracks
        .iter_mut()
        .find(|track| track.id == "A1")
        .unwrap()
}

/// A run of the fader of A1: points (milliseconds, value), its end, what follows it, and its
/// glide.
fn run(points: &[(i64, f64)], to: i64, then: &str, glide: Option<i64>) -> Value {
    let mut run = json!({
        "points": points.iter().map(|&(t, v)| json!({"at": ms(t), "value": v}))
            .collect::<Vec<_>>(),
        "to": ms(to),
        "then": then,
    });
    if let Some(glide) = glide {
        run["glide"] = ms(glide);
    }
    run
}

fn pass(path: &str, trim: bool, runs: Vec<Value>) -> Value {
    json!({"edit": "record_pass", "timeline": "t1", "lanes": [
        {"of": {"track": "A1"}, "path": path, "trim": trim, "runs": runs}
    ]})
}

/// The pass applied as one transaction, taken back by its inverse in one step.
fn record(project: &mut Project, edit: Value) {
    let before = project.clone();
    let edit: Edit = serde_json::from_value(edit).unwrap();
    let expansion =
        columbina_model::expand(project, edit.clone()).unwrap_or_else(|e| panic!("{edit:?}: {e}"));
    let mut undone = project.clone();
    for inverse in expansion.inverse.iter().rev() {
        apply(&mut undone, inverse.clone()).unwrap();
    }
    assert_eq!(undone, before, "taken back in one step");
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

/// The lane of `path` of A1 at a time in milliseconds.
fn at(project: &mut Project, path: &str, ms: f64) -> f64 {
    #[allow(clippy::cast_possible_truncation, reason = "a time of the test")]
    let ticks = (ms * 48.0).round() as i64;
    a1(project).audio.lanes[path].value_at(Time {
        ticks,
        scale: 48000,
    })
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn a_latch_from_one_second_to_the_stop_at_three_writes_its_value_then_returns_over_the_glide() {
    let mut project = project(&[(0, -10.0)]);
    record(
        &mut project,
        pass(
            "fader_db",
            false,
            vec![run(&[(1000, -6.0)], 3000, "return", Some(250))],
        ),
    );
    for (ms, want) in [
        (0.0, -10.0),
        (999.98, -10.0),
        (1000.0, -10.0),
        (1005.0, -8.0),
        (1010.0, -6.0),
        (2000.0, -6.0),
        (2999.98, -6.0),
        (3000.0, -6.0),
        (3125.0, -8.0),
        (3250.0, -10.0),
        (9000.0, -10.0),
    ] {
        let got = at(&mut project, "fader_db", ms);
        assert!(near(got, want), "{ms} ms: {got}, not {want}");
    }
}

#[test]
fn a_snap_released_at_two_seconds_glides_back_over_250_ms() {
    let mut project = project(&[(0, -10.0)]);
    record(
        &mut project,
        pass(
            "fader_db",
            false,
            vec![run(
                &[(1000, -6.0), (1500, -7.0), (1999, -6.0)],
                2000,
                "return",
                Some(250),
            )],
        ),
    );
    for (ms, want) in [
        (999.98, -10.0),
        (1000.0, -10.0),
        (1010.0, -6.0),
        (1500.0, -7.0),
        (2000.0, -6.0),
        (2125.0, -8.0),
        (2250.0, -10.0),
        (5000.0, -10.0),
    ] {
        let got = at(&mut project, "fader_db", ms);
        assert!(near(got, want), "{ms} ms: {got}, not {want}");
    }
}

#[test]
fn hold_keeps_the_last_value_to_the_end_and_event_to_the_next_point_of_the_lane() {
    let lane = [(0, -10.0), (5000, -10.0), (6000, -20.0), (8000, 0.0)];
    let mut held = project(&lane);
    record(
        &mut held,
        pass(
            "fader_db",
            false,
            vec![run(&[(1000, -6.0)], 3000, "hold", None)],
        ),
    );
    for ms in [3000.0, 5500.0, 6000.0, 9000.0] {
        assert!(near(at(&mut held, "fader_db", ms), -6.0), "{ms}");
    }
    let points = &a1(&mut held).audio.lanes["fader_db"].points;
    assert!(points.iter().all(|p| p.at.ticks <= 3000 * 48), "{points:?}");

    let mut event = project(&lane);
    record(
        &mut event,
        pass(
            "fader_db",
            false,
            vec![run(&[(1000, -6.0)], 3000, "event", None)],
        ),
    );
    // The last value to the next point after the stop (5 s), the lane as it was from there.
    for (ms, want) in [
        (3000.0, -6.0),
        (4990.0, -6.0),
        (4995.0, -8.0),
        (5000.0, -10.0),
        (5500.0, -15.0),
        (7000.0, -10.0),
    ] {
        let got = at(&mut event, "fader_db", ms);
        assert!(near(got, want), "{ms} ms: {got}, not {want}");
    }
    // With no point after the stop, Event holds as Hold does.
    let mut last = project(&[(0, -10.0)]);
    record(
        &mut last,
        pass(
            "fader_db",
            false,
            vec![run(&[(1000, -6.0)], 3000, "event", None)],
        ),
    );
    assert!(near(at(&mut last, "fader_db", 9000.0), -6.0));
}

#[test]
fn a_cut_gives_the_lane_back_over_10_ms_at_the_end_of_the_run() {
    let mut project = project(&[(0, -10.0), (4000, 0.0)]);
    record(
        &mut project,
        pass(
            "fader_db",
            false,
            vec![run(&[(1500, -3.0), (2000, -4.0)], 2500, "cut", None)],
        ),
    );
    for (ms, want) in [
        (1499.98, -10.0 + 10.0 * 1499.98 / 4000.0),
        (1500.0, -10.0 + 10.0 * 1500.0 / 4000.0),
        (1510.0, -3.0),
        (2000.0, -4.0),
        (2500.0, -4.0),
        (2505.0, (-4.0 + (-10.0 + 10.0 * 2510.0 / 4000.0)) / 2.0),
        (2510.0, -10.0 + 10.0 * 2510.0 / 4000.0),
        (3000.0, -2.5),
    ] {
        let got = at(&mut project, "fader_db", ms);
        assert!((got - want).abs() < 1e-4, "{ms} ms: {got}, not {want}");
    }
}

#[test]
fn a_trim_adds_its_offset_to_the_lane_as_it_was_and_returns_to_it() {
    // −10 dB to 0 dB over 4 s; +3 dB from 1 s to 3 s, back over 250 ms.
    let lane = |ms: f64| -10.0 + 10.0 * ms / 4000.0;
    let mut project = project(&[(0, -10.0), (4000, 0.0)]);
    record(
        &mut project,
        pass(
            "fader_db",
            true,
            vec![run(&[(1000, 3.0)], 3000, "return", Some(250))],
        ),
    );
    for ms in [1010.0, 1234.5, 2000.0, 2999.98, 3000.0] {
        let got = at(&mut project, "fader_db", ms);
        assert!((got - (lane(ms) + 3.0)).abs() < 1e-4, "{ms} ms: {got}");
    }
    let got = at(&mut project, "fader_db", 1005.0);
    assert!((got - (lane(1005.0) + 1.5)).abs() < 1e-4, "{got}");
    // Half way back, half the offset; then the lane as it was.
    let got = at(&mut project, "fader_db", 3125.0);
    assert!((got - (lane(3125.0) + 1.5)).abs() < 1e-6, "{got}");
    for ms in [0.0, 999.98, 1000.0, 3250.0, 3500.0, 4000.0] {
        let got = at(&mut project, "fader_db", ms);
        assert!((got - lane(ms)).abs() < 1e-4, "{ms} ms: {got}");
    }
}

#[test]
fn a_trim_leaves_minus_infinity_and_keeps_to_plus_10_db() {
    let mut project = project(&[(0, -100.0), (1000, -100.0), (1001, 8.0)]);
    record(
        &mut project,
        pass(
            "fader_db",
            true,
            vec![run(&[(500, 6.0)], 2000, "hold", None)],
        ),
    );
    assert!(near(at(&mut project, "fader_db", 700.0), -100.0));
    assert!(near(at(&mut project, "fader_db", 1500.0), 10.0));
}

#[test]
fn a_run_with_no_lane_before_it_starts_from_the_setting() {
    let mut project = project(&[]);
    a1(&mut project).audio.strip.fader_db = -12.0;
    record(
        &mut project,
        pass(
            "fader_db",
            false,
            vec![run(&[(1000, -6.0)], 2000, "return", Some(250))],
        ),
    );
    assert!(near(at(&mut project, "fader_db", 500.0), -12.0));
    assert!(near(at(&mut project, "fader_db", 1500.0), -6.0));
    assert!(near(at(&mut project, "fader_db", 3000.0), -12.0));
}

#[test]
fn runs_of_a_pass_follow_one_another_a_later_one_taking_over_a_glide_under_way() {
    let mut project = project(&[(0, -10.0)]);
    record(
        &mut project,
        pass(
            "fader_db",
            false,
            vec![
                run(&[(1000, -6.0)], 2000, "return", Some(250)),
                // Touched again 100 ms into the glide.
                run(&[(2100, -2.0)], 2500, "return", Some(250)),
            ],
        ),
    );
    for (ms, want) in [
        (2000.0, -6.0),
        (2099.98, -6.0 - 4.0 * 99.98 / 250.0),
        (2100.0, -6.0 - 4.0 * 100.0 / 250.0),
        (2110.0, -2.0),
        (2500.0, -2.0),
        (2625.0, -6.0),
        (2750.0, -10.0),
    ] {
        let got = at(&mut project, "fader_db", ms);
        assert!((got - want).abs() < 1e-4, "{ms} ms: {got}, not {want}");
    }
}

#[test]
fn a_switch_is_written_on_or_off_and_returns_at_once() {
    let mut project = project(&[]);
    record(
        &mut project,
        pass(
            "mute",
            false,
            vec![run(&[(1000, 0.7), (1500, 0.2)], 2000, "return", Some(250))],
        ),
    );
    assert!(near(at(&mut project, "mute", 999.0), 0.0));
    assert!(near(at(&mut project, "mute", 1200.0), 1.0));
    assert!(near(at(&mut project, "mute", 1600.0), 0.0));
    assert!(near(at(&mut project, "mute", 2100.0), 0.0));
}

#[test]
fn what_is_recorded_is_thinned_within_a_tenth_of_a_decibel() {
    let mut project = project(&[(0, -10.0)]);
    // A slow line of 100 points a second, with a wobble of 0.01 dB.
    let points: Vec<(i64, f64)> = (0..100)
        .map(|n| {
            let wobble = if n % 2 == 0 { 0.01 } else { -0.01 };
            (
                1000 + n * 10,
                -6.0 + f64::from(u32::try_from(n).unwrap()) * 0.02 + wobble,
            )
        })
        .collect();
    record(
        &mut project,
        pass("fader_db", false, vec![run(&points, 2000, "cut", None)]),
    );
    let kept = a1(&mut project).audio.lanes["fader_db"].points.len();
    assert!(kept <= 8, "{kept} points kept");
    for &(t, v) in points.iter().filter(|&&(t, _)| t >= 1010) {
        #[allow(clippy::cast_precision_loss, reason = "a time of the test")]
        let got = at(&mut project, "fader_db", t as f64);
        assert!((got - v).abs() <= 0.1 + 1e-9, "{t} ms: {got}, not {v}");
    }
}

#[test]
fn a_pass_that_is_not_what_it_should_be_is_refused() {
    let mut project = project(&[(0, -10.0)]);
    let bad = |runs: Vec<Value>| pass("fader_db", false, runs);
    // No runs, no points, points out of order or past the end, runs out of order.
    assert_eq!(
        refused(&mut project, bad(vec![])),
        json!({"reason": "out_of_range", "what": "runs"})
    );
    assert_eq!(
        refused(&mut project, bad(vec![run(&[], 2000, "cut", None)])),
        json!({"reason": "out_of_range", "what": "points"})
    );
    assert_eq!(
        refused(
            &mut project,
            bad(vec![run(&[(1500, 0.0), (1000, 0.0)], 2000, "cut", None)])
        ),
        json!({"reason": "out_of_range", "what": "points"})
    );
    assert_eq!(
        refused(
            &mut project,
            bad(vec![run(&[(2500, 0.0)], 2000, "cut", None)])
        ),
        json!({"reason": "out_of_range", "what": "points"})
    );
    assert_eq!(
        refused(
            &mut project,
            bad(vec![
                run(&[(2500, 0.0)], 3000, "cut", None),
                run(&[(1000, 0.0)], 2000, "cut", None)
            ])
        ),
        json!({"reason": "out_of_range", "what": "runs"})
    );
    // A Trim of what is not in decibels; a lane of no setting; no lanes.
    assert_eq!(
        refused(
            &mut project,
            pass("mute", true, vec![run(&[(1000, 1.0)], 2000, "cut", None)])
        ),
        json!({"reason": "out_of_range", "what": "trim"})
    );
    assert_eq!(
        refused(
            &mut project,
            pass("solo", false, vec![run(&[(1000, 1.0)], 2000, "cut", None)])
        )["reason"],
        "out_of_range"
    );
    assert_eq!(
        refused(
            &mut project,
            json!({"edit": "record_pass", "timeline": "t1", "lanes": []})
        ),
        json!({"reason": "out_of_range", "what": "lanes"})
    );
}

#[test]
fn a_switch_jumps_where_a_number_ramps() {
    let mut project = project(&[]);
    record(
        &mut project,
        pass("mute", false, vec![run(&[(1000, 1.0)], 2000, "cut", None)]),
    );
    assert!(near(at(&mut project, "mute", 1000.0), 1.0));
    assert!(near(at(&mut project, "mute", 2000.0), 0.0));
}

#[test]
fn trims_of_one_lane_in_one_pass_each_move_the_lane_as_it_was() {
    // Two passes of Preview punched in together: each offset is of the lane before both, and
    // the later takes the place of the earlier where it writes (as was heard: neither was
    // written while the other played).
    let lane = |ms: f64| -10.0 + 10.0 * ms / 4000.0;
    let mut project = project(&[(0, -10.0), (4000, 0.0)]);
    let edit = json!({"edit": "record_pass", "timeline": "t1", "lanes": [
        {"of": {"track": "A1"}, "path": "fader_db", "trim": true,
            "runs": [run(&[(1000, 3.0)], 3000, "cut", None)]},
        {"of": {"track": "A1"}, "path": "fader_db", "trim": true,
            "runs": [run(&[(2000, -2.0)], 3500, "cut", None)]},
    ]});
    record(&mut project, edit);
    for (ms, by) in [(1500.0, 3.0), (2500.0, -2.0), (3200.0, -2.0), (3700.0, 0.0)] {
        let got = at(&mut project, "fader_db", ms);
        assert!((got - (lane(ms) + by)).abs() < 1e-4, "{ms} ms: {got}");
    }
}

#[test]
fn two_lanes_of_a_pass_are_one_step_of_undo() {
    let mut project = project(&[(0, -10.0)]);
    let edit = json!({"edit": "record_pass", "timeline": "t1", "lanes": [
        {"of": {"track": "A1"}, "path": "fader_db",
            "runs": [run(&[(1000, -6.0)], 2000, "return", Some(250))]},
        {"of": {"track": "A1"}, "path": "pan.x",
            "runs": [run(&[(1000, 0.5)], 2000, "hold", None)]},
    ]});
    record(&mut project, edit);
    assert!(near(at(&mut project, "fader_db", 1500.0), -6.0));
    assert!(near(at(&mut project, "pan.x", 2500.0), 0.5));
}

#[test]
fn thinning_does_not_stretch_the_first_recorded_segment_after_the_entry_ramp() {
    for (start, values, sample) in [
        (36, [-6.48, -9.54, -12.47], 74),
        (
            5585,
            [-23.98281613536591, -7.304378735987619, 9.527919654749956],
            5624,
        ),
    ] {
        for trim in [false, true] {
            let mut project = project(&[(0, 0.0)]);
            let points = [
                (start, values[0]),
                (start + 20, values[1]),
                (start + 40, values[2]),
            ];
            record(
                &mut project,
                pass(
                    "fader_db",
                    trim,
                    vec![run(&points, start + 50, "return", Some(0))],
                ),
            );
            let share = (sample - start - 20) as f64 / 20.0;
            let want = values[1] + (values[2] - values[1]) * share;
            let got = at(&mut project, "fader_db", sample as f64);
            assert!(
                (got - want).abs() <= 0.1 + 1e-9,
                "trim={trim}, {sample} ms: {got}, not {want}"
            );
        }
    }
}

#[test]
fn thinning_keeps_the_last_recorded_value_held_until_the_stop() {
    let mut project = project(&[(0, 0.0)]);
    record(
        &mut project,
        pass(
            "fader_db",
            false,
            vec![run(&[(1000, 0.0), (1020, 0.05)], 1100, "return", Some(0))],
        ),
    );
    for t in [1020.0, 1040.0, 1099.0, 1100.0] {
        assert!(
            near(at(&mut project, "fader_db", t), 0.05),
            "{t} ms must hold the last value"
        );
    }
}

#[test]
fn a_small_trim_keeps_its_ten_millisecond_entry_endpoint_when_thinned() {
    let mut project = project(&[(0, 0.0)]);
    record(
        &mut project,
        pass(
            "fader_db",
            true,
            vec![run(&[(1000, 0.05), (1100, 0.05)], 1200, "return", Some(0))],
        ),
    );
    for (t, want) in [
        (1000.0, 0.0),
        (1005.0, 0.025),
        (1010.0, 0.05),
        (1100.0, 0.05),
        (1200.0, 0.05),
    ] {
        let got = at(&mut project, "fader_db", t);
        assert!(near(got, want), "{t} ms: {got}, not {want}");
    }
}

proptest! {
    /// Whatever the lane and the run, the lane outside the run and its return is as it was, and
    /// within the run it is what was recorded, within the thinning's tenth of a decibel.
    #[test]
    fn a_write_changes_only_its_run_and_its_return(
        lane in prop::collection::vec((0_i64..10_000, -60.0_f64..10.0), 1..8),
        start in 0_i64..8_000,
        values in prop::collection::vec(-60.0_f64..10.0, 1..20),
        glide in 0_i64..400,
    ) {
        let mut lane = lane;
        lane.sort_by_key(|&(t, _)| t);
        let mut project = project(&lane);
        let before = project.clone();
        let points: Vec<(i64, f64)> = values
            .iter()
            .enumerate()
            .map(|(n, &v)| (start + i64::try_from(n).unwrap() * 20, v))
            .collect();
        let to = points.last().unwrap().0 + 10;
        record(
            &mut project,
            pass("fader_db", false, vec![run(&points, to, "return", Some(glide))]),
        );
        let mut before = before;
        for ms in (0..12_000).step_by(37) {
            #[allow(clippy::cast_precision_loss, reason = "a time of the test")]
            let t = ms as f64;
            let got = at(&mut project, "fader_db", t);
            if ms < start || ms > to + glide.max(10) {
                let was = at(&mut before, "fader_db", t);
                prop_assert!((got - was).abs() < 1e-9, "{ms} ms: {got}, was {was}");
            } else if ms < to && ms >= points.get(1).map_or(start + 10, |p| p.0) {
                // The points recorded, joined by straight lines, held from the last.
                let i = points.partition_point(|&(p, _)| p <= ms) - 1;
                let want = match points.get(i + 1) {
                    Some(&(next, v)) => {
                        let (p, u) = points[i];
                        #[allow(clippy::cast_precision_loss, reason = "a time of the test")]
                        let share = (ms - p) as f64 / (next - p) as f64;
                        u + (v - u) * share
                    }
                    None => points[i].1,
                };
                prop_assert!((got - want).abs() <= 0.1 + 1e-9, "{ms} ms: {got}, not {want}");
            }
        }
    }
}

#[test]
fn a_bus_and_a_vca_are_armed_by_a_patch_and_written_only_when_armed() {
    let mut project = project(&[]);
    project.timelines[0]
        .vcas
        .push(serde_json::from_value(json!({"id": "VCA1", "name": "VCA 1"})).unwrap());
    let written = serde_json::to_value(&project.timelines[0].buses[0]).unwrap();
    assert!(
        written.get("automation_arm").is_none(),
        "left out while off"
    );
    for op in [
        json!({"op": "set_bus", "timeline": "t1", "id": "B1",
            "patch": {"automation_arm": true}}),
        json!({"op": "set_vca", "timeline": "t1", "id": "VCA1",
            "patch": {"automation_arm": true}}),
    ] {
        let op: columbina_model::Op = serde_json::from_value(op).unwrap();
        apply(&mut project, op).unwrap();
    }
    let t = &project.timelines[0];
    assert!(t.buses[0].automation_arm.armed() && t.vcas[0].automation_arm.armed());
    let written = serde_json::to_value(&t.buses[0]).unwrap();
    assert_eq!(written["automation_arm"], true);
}
