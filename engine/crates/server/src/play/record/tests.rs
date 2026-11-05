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
    recorder.touch(project, 0, Some(s(seconds)), (&a1(), path), value, touching)
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
        (1.0, -6.0),
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
        (1.0, -6.0),
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
    for seconds in [1.0, 1.5, 2.25, 2.9] {
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
            .touch(&project, 0, None, (&a1(), "fader_db"), -5.0, true)
            .len(),
        1
    );
    // Punched in: the same as a pass of Latch.
    let punched = written(&project, recorder.punch().expect("what was kept"));
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
    assert_eq!(recorder.punch(), None, "punched in once");
    // Recording turned off lets go of what Preview kept.
    let mut kept = recording(&project);
    touch(&mut kept, &project, 1.0, "fader_db", -6.0, true);
    kept.end(&project, s(2.0));
    kept.set(false, 0.25);
    assert_eq!(kept.punch(), None);
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
    for seconds in [0.5, 1.0, 1.4999, 2.5, 3.0, 3.6] {
        let got = at(&after, "fader_db", seconds);
        assert!(near(got, value(seconds)), "{seconds} s: {got}");
    }
    // Within it, what was recorded: the value at its start, on to −2 at 2 s, held.
    assert!(near(at(&after, "fader_db", 1.5), -2.5));
    assert!(near(at(&after, "fader_db", 2.0), -2.0));
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
    assert!(near(at(&after, "fader_db", 4.9999), -6.0));
    assert!(near(at(&after, "fader_db", 5.5), -15.0));
}

#[test]
fn touch_off_writes_armed_tracks_only_and_categories_off_are_not_written() {
    let mut project = project(&[], Touch::Off, RecordMode::Write, OnStop::Hold);
    let mut recorder = recording(&project);
    let heard = touch(&mut recorder, &project, 1.0, "fader_db", -6.0, true);
    assert_eq!(heard.len(), 1, "heard all the same");
    assert_eq!(recorder.end(&project, s(2.0)), None, "A1 not armed");
    project.timelines[0].tracks[1].audio.automation_arm = true;
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
