//! Long timelines keep their internal transitions when their clips ripple together.

use std::time::Instant;

use columbina_model::{
    apply, Alignment, Clip, Edit, History, Material, Op, OpError, Project, Rate, Rgba, Source,
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
