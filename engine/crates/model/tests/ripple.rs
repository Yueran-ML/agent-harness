//! Long timelines keep their internal transitions when their clips ripple together.

use std::time::Instant;

use columbina_model::{
    apply, Alignment, Clip, Cue, Edit, History, Material, Op, OpError, Project, Rate, Rgba, Source,
    Timeline, Track, TrackKind, Transition, TransitionKind, LARGEST,
};

fn sequence(count: i64) -> Project {
    let mut track = Track::new("V1", TrackKind::Video);
    for i in 0..count {
        track.clips.push(Clip::new(
            format!("clip-{i}"),
            Source::Color {
                color: Rgba([0, 0, 0, 255]),
            },
            i * 50,
            50,
        ));
        if i > 0 {
            track.transitions.push(Transition {
                id: format!("transition-{i}"),
                left: format!("clip-{}", i - 1),
                right: format!("clip-{i}"),
                kind: TransitionKind::Dissolve,
                length: 10,
                align: Alignment::Center,
            });
        }
    }
    let mut timeline = Timeline::new("t1", "Sequence", Rate { num: 25, den: 1 });
    timeline.tracks = vec![track];
    let mut project = Project {
        timelines: vec![timeline],
        ..Project::new()
    };
    project.validate().unwrap();
    project
}

fn insert() -> Edit {
    Edit::Insert {
        timeline: "t1".into(),
        video_track: Some(Some("V1".into())),
        audio_track: None,
        at: 0,
        source: Material::Color {
            color: Rgba([255, 255, 255, 255]),
            length: 10,
        },
    }
}

fn shift(ids: &[&str], by: i64) -> Op {
    Op::ShiftClips {
        timeline: "t1".into(),
        track: "V1".into(),
        clips: ids.iter().map(|id| (*id).into()).collect(),
        by,
    }
}

#[test]
fn shifting_a_group_reorders_its_transitions_and_undo_restores_both_lists() {
    let mut before = sequence(4);
    before.timelines[0].tracks[0].transitions.remove(1);
    before.validate().unwrap();
    let mut project = before.clone();
    let op = shift(&["clip-0", "clip-1", "clip-0"], 250);
    let inverse = apply(&mut project, op.clone()).unwrap();
    project.validate().unwrap();
    let track = &project.timelines[0].tracks[0];
    assert_eq!(
        track
            .clips
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>(),
        ["clip-2", "clip-3", "clip-0", "clip-1"]
    );
    assert_eq!(
        track
            .transitions
            .iter()
            .map(|x| x.id.as_str())
            .collect::<Vec<_>>(),
        ["transition-3", "transition-1"]
    );
    let redo = apply(&mut project, inverse).unwrap();
    assert_eq!(project, before);
    assert_eq!(redo, op);
}

#[test]
fn a_shift_refuses_missing_clips_partial_transitions_and_invalid_positions_atomically() {
    let before = sequence(4);
    for (op, error) in [
        (
            shift(&["clip-0", "absent"], 10),
            OpError::Missing {
                what: "clip",
                id: "absent".into(),
            },
        ),
        (
            shift(&["clip-0"], 10),
            OpError::HasTransition {
                transition: "transition-1".into(),
            },
        ),
        (
            shift(&["clip-0", "clip-1", "clip-2", "clip-3"], -1),
            OpError::OutOfTimeline { start: -1 },
        ),
        (
            shift(&["clip-0", "clip-1", "clip-2", "clip-3"], LARGEST),
            OpError::TooLarge { what: "end" },
        ),
        (shift(&[], i64::MIN), OpError::TooLarge { what: "by" }),
        (shift(&[], i64::MAX), OpError::TooLarge { what: "by" }),
        // Neither would change anything, and would make a step that undoes nothing.
        (shift(&[], 10), OpError::OutOfRange { what: "clips" }),
        (shift(&["clip-0"], 0), OpError::OutOfRange { what: "by" }),
    ] {
        let mut project = before.clone();
        assert_eq!(apply(&mut project, op), Err(error));
        assert_eq!(project, before);
    }
    let mut before = before;
    before.timelines[0].tracks[0].transitions.clear();
    let mut project = before.clone();
    assert!(matches!(
        apply(&mut project, shift(&["clip-0", "clip-1"], 10)),
        Err(OpError::Overlap { .. })
    ));
    assert_eq!(project, before);
}

fn with_cues(count: i64) -> Project {
    let mut project = sequence(count);
    let mut track = Track::new("S1", TrackKind::Subtitle);
    for i in 0..count {
        track.cues.push(Cue {
            id: format!("cue-{i}"),
            start: i * 50 + 5,
            length: 40,
            text: format!("line {i}"),
        });
    }
    project.timelines[0].tracks.push(track);
    project.validate().unwrap();
    project
}

fn shift_cues(ids: &[&str], by: i64) -> Op {
    Op::ShiftCues {
        timeline: "t1".into(),
        track: "S1".into(),
        cues: ids.iter().map(|id| (*id).into()).collect(),
        by,
    }
}

#[test]
fn subtitles_shift_together_in_one_operation_that_undo_takes_back() {
    let before = with_cues(4);
    let mut project = before.clone();
    let op = shift_cues(&["cue-0", "cue-1", "cue-0"], 250);
    let inverse = apply(&mut project, op.clone()).unwrap();
    project.validate().unwrap();
    let cues = &project.timelines[0].tracks[1].cues;
    let starts: Vec<(&str, i64)> = cues.iter().map(|q| (q.id.as_str(), q.start)).collect();
    assert_eq!(
        starts,
        [
            ("cue-2", 105),
            ("cue-3", 155),
            ("cue-0", 255),
            ("cue-1", 305)
        ]
    );
    let redo = apply(&mut project, inverse).unwrap();
    assert_eq!(project, before);
    assert_eq!(redo, op);
}

#[test]
fn a_shift_of_subtitles_refuses_what_a_shift_of_clips_does_atomically() {
    let before = with_cues(4);
    let wrong_track = Op::ShiftCues {
        timeline: "t1".into(),
        track: "V1".into(),
        cues: vec!["cue-0".into()],
        by: 10,
    };
    for (op, error) in [
        (
            shift_cues(&["cue-0", "absent"], 10),
            OpError::Missing {
                what: "cue",
                id: "absent".into(),
            },
        ),
        (
            shift_cues(&["cue-0"], 20),
            OpError::CueOverlap {
                with: "cue-0".into(),
            },
        ),
        (
            shift_cues(&["cue-0"], -6),
            OpError::OutOfRange { what: "start" },
        ),
        (shift_cues(&[], 10), OpError::OutOfRange { what: "cues" }),
        (
            shift_cues(&["cue-0"], 0),
            OpError::OutOfRange { what: "by" },
        ),
        (
            shift_cues(&["cue-0"], i64::MAX),
            OpError::TooLarge { what: "by" },
        ),
        (
            wrong_track,
            OpError::WrongTrack {
                what: "cue",
                kind: TrackKind::Video,
            },
        ),
    ] {
        let mut project = before.clone();
        assert_eq!(apply(&mut project, op.clone()), Err(error), "{op:?}");
        assert_eq!(project, before);
    }
}

#[test]
fn rippling_many_subtitles_is_one_operation_per_track() {
    // One operation for each of thousands of subtitles made every ripple take time of the
    // square of their number.
    let before = with_cues(6000);
    let mut history = History::new(before);
    let started = Instant::now();
    let done = history.edit(vec![insert()], "agent").unwrap().applied.done;
    assert_eq!(done.len(), 3, "{:?}", &done[..done.len().min(4)]);
    assert!(matches!(&done[1], Op::ShiftCues { cues, by: 10, .. } if cues.len() == 6000));
    history.undo("agent").unwrap().unwrap();
    assert!(started.elapsed().as_secs() < 5, "{:?}", started.elapsed());
}

#[test]
fn rippling_a_chain_moves_its_transitions_without_detaching_them() {
    let before = sequence(24);
    let mut history = History::new(before.clone());
    let applied = history.edit(vec![insert()], "test").unwrap().applied;
    assert!(applied
        .done
        .iter()
        .all(|op| !matches!(op, Op::RemoveTransition { .. } | Op::AddTransition { .. })));
    let mut after = history.project().clone();
    after.validate().unwrap();
    assert_eq!(
        after.timelines[0].tracks[0].transitions,
        before.timelines[0].tracks[0].transitions
    );
    for (old, new) in before.timelines[0].tracks[0]
        .clips
        .iter()
        .zip(&after.timelines[0].tracks[0].clips[1..])
    {
        assert_eq!(new.start, old.start + 10);
        assert_eq!(new.id, old.id);
    }
    history.undo("test").unwrap().unwrap();
    let mut expected = before;
    expected.next_id = after.next_id;
    assert_eq!(history.project(), &expected);
    history.redo("test").unwrap().unwrap();
    assert_eq!(history.project(), &after);
}

#[test]
#[ignore = "manual release timing: cargo test -p columbina-model --release --test ripple measure -- --ignored --nocapture"]
fn measure_inserting_before_a_long_chain_of_transitions() {
    for count in [1998, 6000] {
        let mut history = History::new(sequence(count));
        let start = Instant::now();
        let applied = history.edit(vec![insert()], "test").unwrap().applied;
        eprintln!(
            "{count} clips: {:?}, {} operations",
            start.elapsed(),
            applied.done.len()
        );
        assert_eq!(
            history.project().timelines[0].tracks[0].clips.len(),
            usize::try_from(count + 1).unwrap()
        );
    }
}
