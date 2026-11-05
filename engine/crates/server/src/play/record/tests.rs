//! Passes recorded from scripted events, each at a sample of the timeline heard, as a simulated
//! clock gives them: what is written, as the edit applied to the project, and what is heard.

#![allow(
    clippy::float_cmp,
    reason = "values set by the test, compared to the bit"
)]

use columbina_audio::graph::{Live, Owner};
use columbina_model::audio::{ActiveRange, OnStop, RecordMode, Touch};
use columbina_model::{Edit, Lane, LaneKind, LanePoint, Project, Time};

use super::{Heard, Recorder};

const RATE: i64 = 48_000;

/// Sample of `seconds` at 48 kHz.
fn s(seconds: f64) -> i64 {
    #[allow(clippy::cast_possible_truncation, reason = "a time of the test")]
    let at = (seconds * 48_000.0).round() as i64;
    at
}

fn a1() -> Owner {
    Owner::Track("A1".into())
}

/// The sample project, the fader of A1 on a lane of `points` (seconds, dB) where there are any,
/// recorded with `touch`, `mode` and `on_stop`.
fn project(points: &[(f64, f64)], touch: Touch, mode: RecordMode, on_stop: OnStop) -> Project {
    let mut project = Project::sample();
    let t = &mut project.timelines[0];
    assert_eq!(i64::from(t.sample_rate), RATE);
    t.audio.record.touch = touch;
    t.audio.record.mode = mode;
    t.audio.record.on_stop = on_stop;
    if !points.is_empty() {
        let track = t.tracks.iter_mut().find(|track| track.id == "A1").unwrap();
        track.audio.lanes.insert(
            "fader_db".into(),
            Lane {
                points: points
                    .iter()
                    .map(|&(at, value)| LanePoint {
                        at: Time {
                            ticks: s(at),
                            scale: 48_000,
                        },
                        value,
                    })
                    .collect(),
                kind: LaneKind::Continuous,
                scale: None,
            },
        );
    }
    project.validate().unwrap();
    project
}

/// A recorder on, its glide 250 ms, a pass begun at 0.
fn recording(project: &Project) -> Recorder {
    let mut recorder = Recorder::default();
    recorder.set(true, 0.250);
    recorder.begin(project, 0, 0);
    recorder
}

/// The control of A1 at `path` touched at `value` (`touching`) at `seconds`.
fn touch(
    recorder: &mut Recorder,
    project: &Project,
    seconds: f64,
    path: &str,
    value: f64,
    touching: bool,
) -> Vec<Heard> {
    recorder.touch(
        project,
        0,
        (true, Some(s(seconds))),
        (&a1(), path),
        value,
        touching,
    )
}

/// The project with `edit` applied.
fn written(project: &Project, edit: Edit) -> Project {
    let mut after = project.clone();
    columbina_model::expand(&mut after, edit).unwrap();
    after
}

/// The value of the lane of A1 at `path` at `seconds`.
fn at(project: &Project, path: &str, seconds: f64) -> f64 {
    let track = project.timelines[0]
        .tracks
        .iter()
        .find(|track| track.id == "A1")
        .unwrap();
    track.audio.lanes[path].value_at(Time {
        ticks: s(seconds),
        scale: 48_000,
    })
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-4
}

#[test]
fn latch_writes_from_the_touch_to_the_stop_and_returns_over_the_glide() {
    let project = project(
        &[(0.0, -10.0)],
        Touch::Latch,
        RecordMode::Write,
        OnStop::Return,
    );
    let mut recorder = recording(&project);
    touch(&mut recorder, &project, 1.0, "fader_db", -6.0, true);
    let heard = touch(&mut recorder, &project, 2.0, "fader_db", -6.0, false);
    // Let go under Latch, it holds its value.
    assert!(matches!(heard[..], [Heard::Hold { live: Live::Hold(v), .. }] if v == -6.0));
    let edit = recorder.end(&project, s(3.0)).expect("a pass written");
    let after = written(&project, edit);
    for (seconds, want) in [
        (0.5, -10.0),
        (0.9999, -10.0),
        (1.01, -6.0),
        (2.5, -6.0),
        (2.9999, -6.0),
        (3.125, -8.0),
        (3.25, -10.0),
        (8.0, -10.0),
    ] {
        let got = at(&after, "fader_db", seconds);
        assert!((got - want).abs() < 1e-3, "{seconds} s: {got}, not {want}");
    }
    assert!(!recorder.recording());
}

#[test]
fn snap_writes_to_the_release_then_glides_back_over_250_ms_as_it_is_heard() {
    let project = project(
        &[(0.0, -10.0)],
        Touch::Snap,
        RecordMode::Write,
        OnStop::Return,
    );
    let mut recorder = recording(&project);
    let held = touch(&mut recorder, &project, 1.0, "fader_db", -6.0, true);
    assert_eq!(
        held,
        vec![Heard::Hold {
            owner: a1(),
            path: "fader_db".into(),
            live: Live::Hold(-6.0),
            plain: -6.0,
            rest: 0.0,
        }]
    );
    let let_go = touch(&mut recorder, &project, 2.0, "fader_db", -6.0, false);
    // The lane back over the glide; with no lane, the fader as it is set (0 dB in the sample).
    assert_eq!(
        let_go,
        vec![Heard::Release {
            owner: a1(),
            path: "fader_db".into(),
            samples: 12_000,
            plain: 0.0,
        }]
    );
    let after = written(&project, recorder.end(&project, s(3.0)).unwrap());
    for (seconds, want) in [
        (0.9999, -10.0),
        (1.01, -6.0),
        (1.9999, -6.0),
        (2.0, -6.0),
        (2.125, -8.0),
        (2.25, -10.0),
        (2.9, -10.0),
        (5.0, -10.0),
    ] {
        let got = at(&after, "fader_db", seconds);
        assert!((got - want).abs() < 1e-3, "{seconds} s: {got}, not {want}");
    }
}

#[test]
fn trim_moves_the_lane_by_the_offset_recorded_and_is_heard_as_an_offset() {
    let lane = |seconds: f64| -10.0 + 10.0 * seconds / 4.0;
    let project = project(
        &[(0.0, -10.0), (4.0, 0.0)],
        Touch::Latch,
        RecordMode::Trim,
        OnStop::Return,
    );
    let mut recorder = recording(&project);
    let heard = touch(&mut recorder, &project, 1.0, "fader_db", 3.0, true);
    // Where the graph has no lane of it, the fader as set (0 dB) moved by 3 dB.
    assert!(matches!(
        heard[..],
        [Heard::Hold { live: Live::Offset(v), plain, .. }] if v == 3.0 && plain == 3.0
    ));
    for (seconds, value) in [(1.5, 3.0), (2.0, 3.0), (2.999, 3.0)] {
        touch(&mut recorder, &project, seconds, "fader_db", value, true);
    }
    let after = written(&project, recorder.end(&project, s(3.0)).unwrap());
    for seconds in [1.01, 1.5, 2.25, 2.9] {
        let got = at(&after, "fader_db", seconds);
        assert!(near(got, lane(seconds) + 3.0), "{seconds} s: {got}");
    }
    for seconds in [0.5, 3.25, 3.5, 4.0] {
        let got = at(&after, "fader_db", seconds);
        assert!(near(got, lane(seconds)), "{seconds} s: {got}");
    }
    // Trim moves a fader only: a pan under Trim is written as Write writes it.
    let mut recorder = recording(&project);
    touch(&mut recorder, &project, 1.0, "pan.x", 0.5, true);
    let Some(Edit::RecordPass { lanes, .. }) = recorder.end(&project, s(2.0)) else {
        panic!("a pass");
    };
    assert!(!lanes[0].trim);
}

#[test]
fn preview_is_heard_and_kept_and_written_only_when_it_is_punched_in() {
    let mut project = project(
        &[(0.0, -10.0)],
        Touch::Latch,
        RecordMode::Write,
        OnStop::Return,
    );
    project.timelines[0].audio.record.preview = true;
    let mut recorder = recording(&project);
    let heard = touch(&mut recorder, &project, 1.0, "fader_db", -6.0, true);
    assert_eq!(heard.len(), 1);
    touch(&mut recorder, &project, 2.0, "fader_db", -6.0, false);
    assert_eq!(
        recorder.end(&project, s(3.0)),
        None,
        "nothing written at the stop"
    );
    assert_eq!(recorder.pending(), 1);
    // Stopped, a control is still heard as it is moved (it is kept for the next pass).
    assert_eq!(
        recorder
            .touch(&project, 0, (false, None), (&a1(), "fader_db"), -5.0, true)
            .len(),
        1
    );
    // Punched in: the same as a pass of Latch.
    let punched = written(&project, recorder.punch("t1").expect("what was kept"));
    let mut latch = project.clone();
    latch.timelines[0].audio.record.preview = false;
    let mut again = recording(&latch);
    touch(&mut again, &latch, 1.0, "fader_db", -6.0, true);
    touch(&mut again, &latch, 2.0, "fader_db", -6.0, false);
    let latched = written(&latch, again.end(&latch, s(3.0)).unwrap());
    assert_eq!(
        punched.timelines[0].tracks[1].audio.lanes,
        latched.timelines[0].tracks[1].audio.lanes
    );
    assert_eq!(recorder.punch("t1"), None, "punched in once");
    // Recording turned off lets go of what Preview kept.
    let mut kept = recording(&project);
    touch(&mut kept, &project, 1.0, "fader_db", -6.0, true);
    kept.end(&project, s(2.0));
    kept.set(false, 0.25);
    assert_eq!(kept.punch("t1"), None);
}

#[test]
fn the_active_range_is_written_only_and_its_end_punches_out() {
    let lane = [(0.0, -10.0), (4.0, 0.0)];
    let value = |seconds: f64| -10.0 + 10.0 * seconds / 4.0;
    let mut project = project(&lane, Touch::Latch, RecordMode::Write, OnStop::Return);
    project.timelines[0].audio.record.active_range = Some(ActiveRange {
        start: Time {
            ticks: s(1.5),
            scale: 48_000,
        },
        end: Time {
            ticks: s(2.5),
            scale: 48_000,
        },
        on: true,
    });
    let mut recorder = recording(&project);
    touch(&mut recorder, &project, 1.0, "fader_db", -3.0, true);
    touch(&mut recorder, &project, 2.0, "fader_db", -2.0, true);
    assert!(!recorder.tick(s(2.4)));
    assert!(recorder.tick(s(2.5)), "the end of the range punches out");
    assert!(!recorder.tick(s(2.6)));
    // After the range, a touch is heard and not written.
    touch(&mut recorder, &project, 2.8, "pan.x", 0.5, true);
    let after = written(&project, recorder.end(&project, s(3.5)).unwrap());
    for seconds in [0.5, 1.0, 1.4999, 1.5, 2.51, 3.0, 3.6] {
        let got = at(&after, "fader_db", seconds);
        assert!(near(got, value(seconds)), "{seconds} s: {got}");
    }
    // Within it, what was recorded: −3 from its start (ramped in over 10 ms), held to 2 s, where
    // −2 was touched a second after −3 and so heard as a step ramped over 10 ms; held to its end.
    assert!(near(at(&after, "fader_db", 1.51), -3.0));
    assert!(near(at(&after, "fader_db", 2.0), -3.0));
    assert!(near(at(&after, "fader_db", 2.005), -2.5));
    assert!(near(at(&after, "fader_db", 2.4999), -2.0));
    let a1 = &after.timelines[0].tracks[1].audio.lanes;
    assert!(!a1.contains_key("pan.x"));
}

#[test]
fn hold_and_event_at_the_stop() {
    let lane = [(0.0, -10.0), (5.0, -10.0), (6.0, -20.0)];
    let held = project(&lane, Touch::Latch, RecordMode::Write, OnStop::Hold);
    let mut recorder = recording(&held);
    touch(&mut recorder, &held, 1.0, "fader_db", -6.0, true);
    let after = written(&held, recorder.end(&held, s(3.0)).unwrap());
    assert!(near(at(&after, "fader_db", 7.0), -6.0));
    let event = project(&lane, Touch::Latch, RecordMode::Write, OnStop::Event);
    let mut recorder = recording(&event);
    touch(&mut recorder, &event, 1.0, "fader_db", -6.0, true);
    let after = written(&event, recorder.end(&event, s(3.0)).unwrap());
    assert!(near(at(&after, "fader_db", 4.9899), -6.0));
    assert!(near(at(&after, "fader_db", 5.5), -15.0));
}

#[test]
fn touch_off_writes_armed_tracks_only_and_categories_off_are_not_written() {
    let mut project = project(&[], Touch::Off, RecordMode::Write, OnStop::Hold);
    let mut recorder = recording(&project);
    let heard = touch(&mut recorder, &project, 1.0, "fader_db", -6.0, true);
    assert_eq!(heard.len(), 1, "heard all the same");
    assert_eq!(recorder.end(&project, s(2.0)), None, "A1 not armed");
    project.timelines[0].tracks[1].audio.automation_arm = columbina_model::AutomationArm::Armed;
    project.timelines[0].audio.record.categories.pan = false;
    let mut recorder = recording(&project);
    touch(&mut recorder, &project, 1.0, "fader_db", -6.0, true);
    touch(&mut recorder, &project, 1.0, "pan.x", 0.5, true);
    let Some(Edit::RecordPass { lanes, .. }) = recorder.end(&project, s(2.0)) else {
        panic!("a pass");
    };
    let paths: Vec<_> = lanes.iter().map(|lane| lane.path.as_str()).collect();
    assert_eq!(paths, ["fader_db"]);
}

#[test]
fn snap_latch_snaps_a_fader_and_latches_the_rest() {
    let project = project(&[], Touch::SnapLatch, RecordMode::Write, OnStop::Hold);
    let mut recorder = recording(&project);
    touch(&mut recorder, &project, 1.0, "fader_db", -6.0, true);
    let fader = touch(&mut recorder, &project, 1.5, "fader_db", -6.0, false);
    assert!(matches!(fader[..], [Heard::Release { .. }]));
    touch(&mut recorder, &project, 1.0, "pan.x", 0.5, true);
    let pan = touch(&mut recorder, &project, 1.5, "pan.x", 0.5, false);
    assert!(matches!(pan[..], [Heard::Hold { .. }]));
    let after = written(&project, recorder.end(&project, s(3.0)).unwrap());
    assert!(
        near(at(&after, "fader_db", 2.5), 0.0),
        "back to 0 dB after the glide"
    );
    assert!(near(at(&after, "pan.x", 2.5), 0.5), "held to the stop");
    assert!(near(at(&after, "pan.x", 9.0), 0.5), "and on (Hold)");
}

#[test]
fn the_same_events_record_the_same_pass_and_points_are_a_millisecond_apart() {
    let project = project(
        &[(0.0, -10.0)],
        Touch::Latch,
        RecordMode::Write,
        OnStop::Return,
    );
    let run = || {
        let mut recorder = recording(&project);
        // Every 10 ms from 1 s, with two a quarter of a millisecond apart.
        for n in 0..50 {
            let seconds = 1.0 + f64::from(n) * 0.01;
            touch(
                &mut recorder,
                &project,
                seconds,
                "fader_db",
                -6.0 + f64::from(n) * 0.1,
                true,
            );
        }
        touch(&mut recorder, &project, 1.49025, "fader_db", -1.0, true);
        recorder.end(&project, s(2.0)).unwrap()
    };
    let one = run();
    assert_eq!(one, run());
    let Edit::RecordPass { lanes, .. } = &one else {
        panic!("a pass");
    };
    let points = &lanes[0].runs[0].points;
    assert_eq!(points.len(), 50);
    assert!(points
        .windows(2)
        .all(|w| w[1].at.ticks - w[0].at.ticks >= 48));
    assert_eq!(points[49].value, -1.0);
}

#[test]
fn nothing_is_recorded_while_recording_is_off() {
    let project = project(&[], Touch::Latch, RecordMode::Write, OnStop::Return);
    let mut recorder = Recorder::default();
    recorder.begin(&project, 0, 0);
    assert!(!recorder.recording());
    assert_eq!(
        touch(&mut recorder, &project, 1.0, "fader_db", -6.0, true),
        vec![]
    );
    assert_eq!(recorder.end(&project, s(2.0)), None);
}

#[test]
fn a_value_typed_after_a_release_under_latch_steps_over_10_ms_as_it_was_heard() {
    let project = project(
        &[(0.0, -10.0)],
        Touch::Latch,
        RecordMode::Write,
        OnStop::Hold,
    );
    let mut recorder = recording(&project);
    touch(&mut recorder, &project, 1.0, "fader_db", -6.0, true);
    touch(&mut recorder, &project, 2.0, "fader_db", -6.0, false);
    // Typed: touched and let go at once, 3 s later.
    touch(&mut recorder, &project, 5.0, "fader_db", -3.0, true);
    touch(&mut recorder, &project, 5.0, "fader_db", -3.0, false);
    let after = written(&project, recorder.end(&project, s(6.0)).unwrap());
    for (seconds, want) in [
        (3.0, -6.0),
        (4.999, -6.0),
        (5.0, -6.0),
        (5.005, -4.5),
        (5.01, -3.0),
        (5.9, -3.0),
    ] {
        let got = at(&after, "fader_db", seconds);
        assert!((got - want).abs() < 1e-3, "{seconds} s: {got}, not {want}");
    }
}

#[test]
fn a_value_out_of_its_range_is_heard_and_written_at_its_end() {
    let trimming = project(&[], Touch::Latch, RecordMode::Trim, OnStop::Hold);
    let project = project(&[], Touch::Latch, RecordMode::Write, OnStop::Hold);
    let mut recorder = recording(&project);
    for value in [120.0, 1e30] {
        let heard = touch(&mut recorder, &project, 1.0, "fader_db", value, true);
        assert!(
            matches!(heard[..], [Heard::Hold { live: Live::Hold(v), plain, .. }]
                if v == 10.0 && plain == 10.0),
            "{heard:?}"
        );
    }
    assert_eq!(
        touch(&mut recorder, &project, 1.5, "fader_db", f64::NAN, true),
        vec![]
    );
    // A frequency at 0 Hz or below: the least of the band.
    let heard = touch(&mut recorder, &project, 1.0, "eq.1.freq", -5.0, true);
    let [Heard::Hold {
        live: Live::Hold(least),
        ..
    }] = heard[..]
    else {
        panic!("{heard:?}");
    };
    assert!(least > 0.0, "{least}");
    let after = written(&project, recorder.end(&project, s(2.0)).unwrap());
    assert!(near(at(&after, "fader_db", 1.5), 10.0));
    // A Trim moves a fader by no more than its span.
    let mut recorder = recording(&trimming);
    let heard = touch(&mut recorder, &trimming, 1.0, "fader_db", -1e30, true);
    assert!(
        matches!(heard[..], [Heard::Hold { live: Live::Offset(v), .. }] if v == -110.0),
        "{heard:?}"
    );
}

#[test]
fn a_touch_at_another_speed_is_heard_and_not_recorded() {
    let project = project(&[], Touch::Latch, RecordMode::Write, OnStop::Hold);
    let mut recorder = recording(&project);
    let heard = recorder.touch(&project, 0, (true, None), (&a1(), "fader_db"), -6.0, true);
    assert_eq!(heard.len(), 1, "heard");
    assert_eq!(recorder.end(&project, s(2.0)), None, "not recorded");
}

#[test]
fn a_punch_in_is_of_its_timeline_and_what_it_could_not_write_is_given_back() {
    let mut project = project(&[], Touch::Latch, RecordMode::Write, OnStop::Hold);
    project.timelines[0].audio.record.preview = true;
    let mut recorder = recording(&project);
    touch(&mut recorder, &project, 1.0, "fader_db", -6.0, true);
    assert_eq!(recorder.end(&project, s(2.0)), None);
    assert_eq!(recorder.punch("t2"), None, "of another timeline");
    assert_eq!(recorder.pending(), 1);
    let Some(Edit::RecordPass { lanes, .. }) = recorder.punch("t1") else {
        panic!("what was kept");
    };
    assert_eq!(recorder.pending(), 0);
    recorder.give_back("t1", lanes);
    assert_eq!(recorder.pending(), 1, "given back");
    recorder.reset();
    assert_eq!(recorder.pending(), 0);
    assert!(!recorder.is_on());
}

#[test]
fn no_pass_begins_where_no_sound_session_plays() {
    let project = project(&[], Touch::Latch, RecordMode::Write, OnStop::Hold);
    let mut automation = super::Automation::default();
    automation.recorder.set(true, 0.25);
    let mut playback = super::super::sound::Playback::new();
    automation.touch(
        (&project, 0),
        (true, Some(s(1.0))),
        (&a1(), "fader_db"),
        -6.0,
        true,
        &mut playback,
    );
    assert!(!automation.recorder.recording());
}

#[test]
fn touch_off_writes_a_bus_or_a_vca_where_it_is_armed() {
    let mut project = project(&[], Touch::Off, RecordMode::Write, OnStop::Hold);
    let b1 = Owner::Bus("B1".into());
    let pass = |project: &Project| {
        let mut recorder = recording(project);
        recorder.touch(
            project,
            0,
            (true, Some(s(1.0))),
            (&b1, "fader_db"),
            -6.0,
            true,
        );
        recorder.end(project, s(2.0))
    };
    assert_eq!(pass(&project), None, "not armed");
    project.timelines[0].buses[0].automation_arm = columbina_model::AutomationArm::Armed;
    let Some(Edit::RecordPass { lanes, .. }) = pass(&project) else {
        panic!("armed, written");
    };
    assert_eq!(lanes[0].of, columbina_model::LanesOf::Bus("B1".into()));
}

// T16b: the arm's Safe and Off, All Read, Join Mix, Fill Range and Glide Range, and groups that
// link Automation.

#[test]
fn a_safe_or_off_arm_is_heard_and_never_written_whatever_the_touch() {
    for arm in [
        columbina_model::AutomationArm::Safe,
        columbina_model::AutomationArm::Off,
    ] {
        for touch_mode in [Touch::Off, Touch::Latch] {
            let mut project = project(
                &[(0.0, -10.0)],
                touch_mode,
                RecordMode::Write,
                OnStop::Return,
            );
            project.timelines[0].tracks[1].audio.automation_arm = arm;
            let mut recorder = recording(&project);
            let heard = touch(&mut recorder, &project, 1.0, "fader_db", -6.0, true);
            assert_eq!(heard.len(), 1, "heard");
            touch(&mut recorder, &project, 2.0, "fader_db", -6.0, false);
            assert_eq!(
                recorder.end(&project, s(3.0)),
                None,
                "{arm:?} {touch_mode:?}"
            );
        }
    }
    // Armed, under Touch Off, it is.
    let mut project = project(
        &[(0.0, -10.0)],
        Touch::Off,
        RecordMode::Write,
        OnStop::Return,
    );
    project.timelines[0].tracks[1].audio.automation_arm = columbina_model::AutomationArm::Armed;
    let mut recorder = recording(&project);
    touch(&mut recorder, &project, 1.0, "fader_db", -6.0, true);
    assert!(recorder.end(&project, s(3.0)).is_some());
}

#[test]
fn all_read_stops_what_is_written_where_it_is_asked_and_nothing_touched_after_is_written() {
    let project = project(
        &[(0.0, -10.0)],
        Touch::Latch,
        RecordMode::Write,
        OnStop::Return,
    );
    let mut recorder = recording(&project);
    touch(&mut recorder, &project, 1.0, "fader_db", -6.0, true);
    touch(&mut recorder, &project, 1.5, "fader_db", -6.0, false);
    assert!(recorder.all_read(Some(s(2.0))));
    // Touched again: heard, not written.
    let heard = touch(&mut recorder, &project, 3.0, "fader_db", 0.0, true);
    assert_eq!(heard.len(), 1);
    let after = written(
        &project,
        recorder.end(&project, s(4.0)).expect("the first run"),
    );
    for (seconds, want) in [
        (0.5, -10.0),
        (1.5, -6.0),
        (1.999, -6.0),
        (2.005, -8.0),
        (2.01, -10.0),
        (3.5, -10.0),
    ] {
        let got = at(&after, "fader_db", seconds);
        assert!((got - want).abs() < 1e-3, "{seconds} s: {got}, not {want}");
    }
    // Recording turned on again writes again.
    recorder.set(true, 0.25);
    recorder.begin(&project, 0, 0);
    touch(&mut recorder, &project, 1.0, "fader_db", -3.0, true);
    assert!(recorder.end(&project, s(2.0)).is_some());
    // Off, there is nothing to read.
    let mut off = Recorder::default();
    assert!(!off.all_read(None));
}

#[test]
fn join_mix_writes_on_from_the_last_values_where_the_next_pass_reaches_the_stop() {
    let project = project(
        &[(0.0, -10.0)],
        Touch::Latch,
        RecordMode::Write,
        OnStop::Return,
    );
    let mut recorder = recording(&project);
    touch(&mut recorder, &project, 1.0, "fader_db", -6.0, true);
    touch(&mut recorder, &project, 1.5, "fader_db", -4.0, false);
    let first = written(&project, recorder.end(&project, s(3.0)).unwrap());
    assert_eq!(recorder.join("t1"), (1, Some(s(3.0))));
    // From 2 s again: nothing before the stop, the last value joined there and held to the stop.
    recorder.begin(&first, 0, s(2.0));
    assert!(recorder.joined(&first, 0, s(2.5)).is_empty(), "not yet");
    let heard = recorder.joined(&first, 0, s(3.0));
    assert!(matches!(heard[..], [Heard::Hold { live: Live::Hold(v), .. }] if v == -4.0));
    assert!(recorder.joined(&first, 0, s(3.5)).is_empty(), "joined once");
    let after = written(&first, recorder.end(&first, s(5.0)).unwrap());
    for (seconds, want) in [
        (2.0, -4.0),
        (2.999, -4.0),
        (3.1, -4.0),
        (4.0, -4.0),
        (4.999, -4.0),
        (5.25, -10.0),
    ] {
        let got = at(&after, "fader_db", seconds);
        assert!((got - want).abs() < 1e-3, "{seconds} s: {got}, not {want}");
    }
    // Of another timeline, or with no stop, nothing.
    assert_eq!(recorder.join("t2"), (0, None));
}

#[test]
fn fill_range_writes_what_preview_holds_and_glide_range_glides_to_it() {
    let mut project = project(
        &[(0.0, -10.0)],
        Touch::Latch,
        RecordMode::Write,
        OnStop::Return,
    );
    project.timelines[0].audio.record.preview = true;
    for glide in [false, true] {
        let mut recorder = Recorder::default();
        recorder.set(true, 0.25);
        // Stopped, under Preview: held, not recorded.
        recorder.touch(&project, 0, (false, None), (&a1(), "fader_db"), -4.0, true);
        let (edit, gone) = recorder
            .fill(
                &project,
                0,
                (
                    Time {
                        ticks: s(1.0),
                        scale: 48_000,
                    },
                    Time {
                        ticks: s(3.0),
                        scale: 48_000,
                    },
                ),
                glide,
            )
            .expect("what Preview holds");
        assert_eq!(gone, vec![(a1(), "fader_db".to_string())]);
        let after = written(&project, edit);
        let middle = if glide { -7.0 } else { -4.0 };
        for (seconds, want) in [(0.5, -10.0), (2.005, middle), (2.999, -4.0), (3.5, -10.0)] {
            let got = at(&after, "fader_db", seconds);
            assert!(
                (got - want).abs() < 1e-2,
                "{glide} {seconds} s: {got}, not {want}"
            );
        }
        assert!(
            recorder
                .fill(
                    &project,
                    0,
                    (Time { ticks: 0, scale: 1 }, Time { ticks: 1, scale: 1 }),
                    glide
                )
                .is_none(),
            "filled once"
        );
    }
}

#[test]
fn a_fader_of_a_group_linking_automation_moves_and_writes_the_others_by_the_same_change() {
    let mut project = project(
        &[(0.0, -10.0)],
        Touch::Latch,
        RecordMode::Write,
        OnStop::Return,
    );
    let mut a2 = columbina_model::Track::new("A2", columbina_model::TrackKind::Audio);
    a2.audio.strip.fader_db = -20.0;
    project.timelines[0].tracks.push(a2);
    let group: columbina_model::TrackGroup = serde_json::from_value(serde_json::json!({
        "id": "TG1", "name": "Group 1", "members": ["A1", "A2"], "linked": ["fader", "automation"],
    }))
    .unwrap();
    project.timelines[0].track_groups.push(group);
    project.validate().unwrap();
    let mut recorder = recording(&project);
    // A1's lane is at -10 there and A2 at -20: A1 to -6 moves A2 to -16.
    let heard = touch(&mut recorder, &project, 1.0, "fader_db", -6.0, true);
    assert_eq!(heard.len(), 2);
    assert!(
        matches!(&heard[1], Heard::Hold { owner: Owner::Track(id), live: Live::Hold(v), .. }
        if id == "A2" && *v == -16.0)
    );
    let after = written(&project, recorder.end(&project, s(3.0)).unwrap());
    let a2 = after.timelines[0].tracks[2].audio.lanes["fader_db"].value_at(Time {
        ticks: s(2.0),
        scale: 48_000,
    });
    assert!(near(a2, -16.0), "{a2}");
    // Not where the group does not link Automation.
    let mut unlinked = project.clone();
    unlinked.timelines[0].track_groups[0].linked = vec![columbina_model::GroupControl::Fader];
    let mut recorder = recording(&unlinked);
    assert_eq!(
        touch(&mut recorder, &unlinked, 1.0, "fader_db", -6.0, true).len(),
        1
    );
}

// T16b's review.

#[test]
fn fill_writes_what_it_can_and_keeps_held_what_a_locked_or_deleted_track_could_not_take() {
    let mut project = project(
        &[(0.0, -10.0)],
        Touch::Latch,
        RecordMode::Write,
        OnStop::Return,
    );
    project.timelines[0].audio.record.preview = true;
    for id in ["A2", "A3"] {
        project.timelines[0]
            .tracks
            .push(columbina_model::Track::new(
                id,
                columbina_model::TrackKind::Audio,
            ));
    }
    project.validate().unwrap();
    let mut recorder = Recorder::default();
    recorder.set(true, 0.25);
    for (id, value) in [("A1", -4.0), ("A2", -8.0), ("A3", -2.0)] {
        let owner = Owner::Track(id.into());
        recorder.touch(
            &project,
            0,
            (false, None),
            (&owner, "fader_db"),
            value,
            true,
        );
    }
    // A2 locked, A3 taken away since.
    let mut now = project.clone();
    now.timelines[0].tracks[2].locked = true;
    now.timelines[0].tracks.retain(|track| track.id != "A3");
    let range = (
        Time {
            ticks: s(1.0),
            scale: 48_000,
        },
        Time {
            ticks: s(2.0),
            scale: 48_000,
        },
    );
    let (edit, gone) = recorder.fill(&now, 0, range, false).expect("A1");
    assert_eq!(gone, vec![(a1(), "fader_db".to_string())]);
    let after = written(&now, edit);
    assert!(near(at(&after, "fader_db", 1.5), -4.0));
    // A2, unlocked, is filled next.
    now.timelines[0].tracks[2].locked = false;
    let (edit, gone) = recorder.fill(&now, 0, range, false).expect("A2 still held");
    assert_eq!(
        gone,
        vec![(Owner::Track("A2".into()), "fader_db".to_string())]
    );
    let after = written(&now, edit);
    let a2 = after.timelines[0].tracks[2].audio.lanes["fader_db"].value_at(Time {
        ticks: s(1.5),
        scale: 48_000,
    });
    assert!(near(a2, -8.0), "{a2}");
    // With Preview off, nothing held is filled.
    let mut plain = now.clone();
    plain.timelines[0].audio.record.preview = false;
    let owner = Owner::Track("A2".into());
    recorder.touch(&now, 0, (false, None), (&owner, "fader_db"), -3.0, true);
    assert!(recorder.fill(&plain, 0, range, false).is_none());
}

#[test]
fn a_level_of_a_group_follows_from_minus_infinity_as_a_fader_does_while_recording() {
    let mut project = project(&[], Touch::Latch, RecordMode::Write, OnStop::Return);
    project.timelines[0].tracks[1].audio.strip.fader_db = -120.0;
    let mut a2 = columbina_model::Track::new("A2", columbina_model::TrackKind::Audio);
    a2.audio.strip.fader_db = -50.0;
    project.timelines[0].tracks.push(a2);
    let group: columbina_model::TrackGroup = serde_json::from_value(serde_json::json!({
        "id": "TG1", "name": "Group 1", "members": ["A1", "A2"], "linked": ["fader", "automation"],
    }))
    .unwrap();
    project.timelines[0].track_groups.push(group);
    project.validate().unwrap();
    let mut recorder = recording(&project);
    let heard = touch(&mut recorder, &project, 1.0, "fader_db", -60.0, true);
    assert!(
        matches!(&heard[1], Heard::Hold { live: Live::Hold(v), .. } if *v == -10.0),
        "{heard:?}"
    );
    // A2 at -inf stays there as A1 goes down.
    project.timelines[0].tracks[2].audio.strip.fader_db = -100.0;
    project.timelines[0].tracks[1].audio.strip.fader_db = 0.0;
    let mut recorder = recording(&project);
    let heard = touch(&mut recorder, &project, 1.0, "fader_db", -6.0, true);
    assert!(
        matches!(&heard[1], Heard::Hold { live: Live::Hold(v), .. } if *v == -100.0),
        "{heard:?}"
    );
}

#[test]
fn all_read_is_in_the_state_of_recording() {
    let project = project(&[], Touch::Latch, RecordMode::Write, OnStop::Return);
    let mut automation = super::Automation::default();
    automation.recorder.set(true, 0.25);
    assert_eq!(automation.state()["read"], false);
    automation.recorder.all_read(None);
    assert_eq!(automation.state()["read"], true);
    assert!(automation.reads());
    automation.recorder.begin(&project, 0, 0);
    automation.recorder.set(true, 0.25);
    assert_eq!(automation.state()["read"], false);
}
