//! Taking clips away, cutting frames out, cutting clips in two and moving clips: lift, ripple
//! delete, ripple delete of a range, close gap, blade, move and insert move.

use std::collections::HashSet;

use super::work::{union, Work};
use super::EditError;
use crate::ops::OpError;
use crate::project::{Clip, Id, Track, TrackKind};

pub(super) fn lift(work: &mut Work, clips: &[Id], linked: bool) -> Result<(), EditError> {
    let all = work.group(clips, linked)?;
    if all.is_empty() {
        return Err(EditError::EmptyRange);
    }
    for id in &all {
        work.remove(id)?;
    }
    Ok(())
}

pub(super) fn ripple_delete(work: &mut Work, clips: &[Id], linked: bool) -> Result<(), EditError> {
    let all = work.group(clips, linked)?;
    if all.is_empty() {
        return Err(EditError::EmptyRange);
    }
    let spans = spans_of(work, &all)?;
    for id in &all {
        work.remove(id)?;
    }
    work.close(&spans)
}

/// Where each of `clips` lies: its start and end.
fn spans_of(work: &Work, clips: &[Id]) -> Result<Vec<(i64, i64)>, EditError> {
    clips
        .iter()
        .map(|id| work.clip(id).map(|(_, c)| (c.start, c.end())))
        .collect()
}

/// The tracks that an edit names, each once, which must not be locked; or, if it names none,
/// those of `kinds` that are not locked.
fn open_tracks(
    work: &Work,
    tracks: Option<&[Id]>,
    kinds: &[TrackKind],
) -> Result<Vec<Id>, EditError> {
    let Some(tracks) = tracks else {
        return Ok(work.unlocked(kinds));
    };
    let mut open: Vec<Id> = vec![];
    for id in tracks {
        work.open(id)?;
        if !open.contains(id) {
            open.push(id.clone());
        }
    }
    Ok(open)
}

pub(super) fn ripple_delete_range(
    work: &mut Work,
    from: i64,
    to: i64,
    tracks: Option<&[Id]>,
) -> Result<(), EditError> {
    if from < 0 {
        return Err(EditError::Op(OpError::OutOfTimeline { start: from }));
    }
    if to <= from {
        return Err(EditError::EmptyRange);
    }
    let all = [TrackKind::Video, TrackKind::Audio, TrackKind::Subtitle];
    let tracks = open_tracks(work, tracks, &all)?;
    work.clear_clips(&tracks, from, to)?;
    work.shift(&tracks, to, from - to, &HashSet::new())
}

pub(super) fn close_gap(work: &mut Work, track: &str, at: i64) -> Result<(), EditError> {
    let (from, to) = gap_of(work.open(track)?, at).ok_or(EditError::EmptyRange)?;
    work.close(&[(from, to)])
}

/// The gap of a track that `at` is in: from the end of what is before it, or the start of the
/// timeline, to the start of what is after it. None if something covers `at`, or nothing is
/// after it.
fn gap_of(track: &Track, at: i64) -> Option<(i64, i64)> {
    let clips = track.clips.iter().map(|c| (c.start, c.end()));
    let cues = track.cues.iter().map(|q| (q.start, q.start + q.length));
    let spans: Vec<(i64, i64)> = clips.chain(cues).collect();
    if at < 0 || spans.iter().any(|&(start, end)| start <= at && at < end) {
        return None;
    }
    let from = spans
        .iter()
        .map(|&(_, end)| end)
        .filter(|&end| end <= at)
        .max()
        .unwrap_or(0);
    let to = spans
        .iter()
        .map(|&(start, _)| start)
        .filter(|&start| start > at)
        .min()?;
    Some((from, to))
}

pub(super) fn blade(
    work: &mut Work,
    at: i64,
    tracks: Option<&[Id]>,
    linked: bool,
) -> Result<(), EditError> {
    let kinds = [TrackKind::Video, TrackKind::Audio];
    let named = tracks.and_then(|t| (t.len() == 1).then(|| t[0].clone()));
    let tracks = open_tracks(work, tracks, &kinds)?;
    let mut across = vec![];
    for track in &tracks {
        across.extend(work.across(track, at)?);
    }
    let mut all = work.group(&across, linked)?;
    all.retain(|id| {
        work.clip(id)
            .is_ok_and(|(_, c)| c.start < at && at < c.end())
    });
    if all.is_empty() {
        return Err(EditError::NothingAt {
            track: named,
            frame: at,
        });
    }
    work.blade(&all, at)
}

pub(super) fn move_clips(
    work: &mut Work,
    clips: &[Id],
    delta: i64,
    to_track: Option<&str>,
    overwrite: bool,
    linked: bool,
) -> Result<(), EditError> {
    let all = work.group(clips, linked)?;
    let Some(first) = all.first() else {
        return Err(EditError::EmptyRange);
    };
    let (first_track, _) = work.owned(first)?;
    let (kind, offset) = match to_track {
        Some(to) => {
            let kind = work.track(to)?.kind;
            if work.track(&first_track)?.kind != kind {
                return Err(EditError::Op(OpError::WrongKind {
                    track: to.into(),
                    kind,
                }));
            }
            let place = |id: &str| place_among(work, kind, id);
            (Some(kind), place(to) - place(&first_track))
        }
        None => (None, 0),
    };
    let mut moved: Vec<(Id, Clip)> = vec![];
    for id in &all {
        let (track, clip) = work.owned(id)?;
        let track = match kind {
            Some(kind) if work.track(&track)?.kind == kind => {
                let to = place_among(work, kind, &track) + offset;
                let to = usize::try_from(to)
                    .ok()
                    .and_then(|to| of_kind(work, kind).get(to).cloned());
                to.ok_or_else(|| EditError::NoRoom {
                    track: to_track.unwrap_or_default().into(),
                })?
            }
            _ => track,
        };
        work.open(&track)?;
        moved.push((track, clip));
    }
    let earliest = moved.iter().map(|(_, c)| c.start).min().unwrap_or(0);
    let delta = if earliest + delta < 0 {
        work.clamp();
        -earliest
    } else {
        delta
    };
    if delta == 0 && offset == 0 {
        return Err(EditError::EmptyRange);
    }
    for (_, clip) in &mut moved {
        clip.start += delta;
    }
    for id in &all {
        work.remove(id)?;
    }
    for (track, clip) in &moved {
        if overwrite {
            work.clear_clips(std::slice::from_ref(track), clip.start, clip.end())?;
        } else if work.track(track)?.clips.iter().any(|c| overlap(c, clip)) {
            return Err(EditError::NoRoom {
                track: track.clone(),
            });
        }
    }
    for (track, clip) in moved {
        work.insert(&track, clip)?;
    }
    Ok(())
}

fn overlap(a: &Clip, b: &Clip) -> bool {
    a.start < b.end() && b.start < a.end()
}

/// The ids of the tracks of `kind`, the first at the bottom.
fn of_kind(work: &Work, kind: TrackKind) -> Vec<Id> {
    work.timeline()
        .tracks
        .iter()
        .filter(|r| r.kind == kind)
        .map(|r| r.id.clone())
        .collect()
}

/// The place of a track among those of its kind.
fn place_among(work: &Work, kind: TrackKind, track: &str) -> i64 {
    let at = of_kind(work, kind).iter().position(|id| id == track);
    at.and_then(|at| i64::try_from(at).ok()).unwrap_or(0)
}

pub(super) fn insert_move(
    work: &mut Work,
    clips: &[Id],
    at: i64,
    linked: bool,
) -> Result<(), EditError> {
    if at < 0 {
        return Err(EditError::Op(OpError::OutOfTimeline { start: at }));
    }
    let all = work.group(clips, linked)?;
    if all.is_empty() {
        return Err(EditError::EmptyRange);
    }
    let mut moved: Vec<(Id, Clip)> = all
        .iter()
        .map(|id| work.owned(id))
        .collect::<Result<_, _>>()?;
    let spans = union(&spans_of(work, &all)?);
    let length = spans.iter().map(|(from, to)| to - from).sum();
    moved.sort_by_key(|(_, c)| c.start);
    for id in &all {
        work.remove(id)?;
    }
    work.close(&spans)?;
    // Where `at` is once the gaps are closed.
    let mut closed = 0;
    for &(from, to) in &spans {
        closed += (at.min(to) - from).max(0);
    }
    let at = at - closed;
    work.make_room(at, length)?;
    // Disjoint selections carry only their occupied spans. Gaps between them remain at the
    // source rather than being inserted a second time at the destination.
    let (mut span, mut offset) = (0, 0);
    for (track, clip) in moved {
        while spans[span].1 <= clip.start {
            offset += spans[span].1 - spans[span].0;
            span += 1;
        }
        let start = at + offset + clip.start - spans[span].0;
        work.insert(&track, Clip { start, ..clip })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::fixture::*;
    use super::super::{Edit, EditError};
    use crate::project::Alignment;

    fn ripple_delete(clips: &[&str]) -> Edit {
        Edit::RippleDelete {
            timeline: "t1".into(),
            clips: clips.iter().map(|&c| c.into()).collect(),
            linked: true,
        }
    }

    fn range(from: i64, to: i64) -> Edit {
        Edit::RippleDeleteRange {
            timeline: "t1".into(),
            from,
            to,
            tracks: None,
        }
    }

    fn blade(at: i64, tracks: Option<&[&str]>, linked: bool) -> Edit {
        Edit::Blade {
            timeline: "t1".into(),
            at,
            tracks: tracks.map(|t| t.iter().map(|&t| t.into()).collect()),
            linked,
        }
    }

    fn move_by(clips: &[&str], delta: i64, to_track: Option<&str>, overwrite: bool) -> Edit {
        Edit::Move {
            timeline: "t1".into(),
            clips: clips.iter().map(|&c| c.into()).collect(),
            delta,
            to_track: to_track.map(Into::into),
            overwrite,
            linked: true,
        }
    }

    #[test]
    fn a_lift_takes_the_clip_and_those_linked_to_it_and_leaves_gaps() {
        let mut project = project();
        edit(
            &mut project,
            Edit::Lift {
                timeline: "t1".into(),
                clips: vec!["c3".into()],
                linked: true,
            },
        );
        assert_eq!(clips(&project, "V1"), rows(&[("c1", 0, 100, 0)]));
        assert_eq!(clips(&project, "A1"), rows(&[("c2", 0, 100, 0)]));
        assert_eq!(clips(&project, "V2"), rows(&[("c5", 120, 50, 50)]));
    }

    #[test]
    fn a_lift_unlinked_takes_the_clip_alone_and_one_on_a_locked_track_is_refused() {
        let mut project = project();
        let lift = |linked| Edit::Lift {
            timeline: "t1".into(),
            clips: vec!["c1".into()],
            linked,
        };
        edit(&mut project, lift(false));
        assert_eq!(clips(&project, "A1").len(), 2);
        let error = refused(
            with(super::super::fixture::project(), vec![lock("V1")]),
            lift(true),
        );
        assert_eq!(error, EditError::Locked { track: "V1".into() });
    }

    #[test]
    fn a_ripple_delete_closes_the_gap_on_every_open_track() {
        let mut project = with(project(), vec![lock("S1"), lock("V2")]);
        edit(&mut project, ripple_delete(&["c1"]));
        assert_eq!(clips(&project, "V1"), rows(&[("c3", 0, 50, 150)]));
        assert_eq!(clips(&project, "A1"), rows(&[("c4", 0, 50, 150)]));
        // Locked tracks stay.
        assert_eq!(clips(&project, "V2"), rows(&[("c5", 120, 50, 50)]));
        assert_eq!(cues(&project, "S1")[0], ("q6".into(), 10, 30));
    }

    #[test]
    fn a_ripple_delete_is_blocked_by_another_track_with_something_in_the_gap() {
        let error = refused(project(), ripple_delete(&["c3"]));
        // V2 has c5 across frames [100, 150).
        assert_eq!(
            error,
            EditError::RippleBlocked {
                track: "V2".into(),
                clip: "c5".into()
            }
        );
    }

    #[test]
    fn a_ripple_delete_of_two_clips_closes_both_gaps() {
        let mut project = with(project(), vec![lock("V2"), lock("S1")]);
        edit(&mut project, ripple_delete(&["c1", "c3"]));
        assert_eq!(clips(&project, "V1"), []);
        assert_eq!(clips(&project, "A1"), []);
    }

    #[test]
    fn a_range_is_cut_out_of_every_open_track() {
        let mut project = project();
        // Frames [90, 130): the ends of c1, c2, the starts of c3, c4, c5 and part of q7.
        edit(&mut project, range(90, 130));
        assert_eq!(
            clips(&project, "V1"),
            rows(&[("c1", 0, 90, 0), ("c13", 90, 20, 180)])
        );
        assert_eq!(
            clips(&project, "A1"),
            rows(&[("c2", 0, 90, 0), ("c16", 90, 20, 180)])
        );
        // The rest of c3 and the rest of c4 are still linked, in a new group.
        assert_eq!(the(&project, "c13").link, the(&project, "c16").link);
        assert_eq!(clips(&project, "V2"), rows(&[("c15", 90, 40, 60)]));
        assert_eq!(
            cues(&project, "S1"),
            [("q6".into(), 10, 30), ("q7".into(), 90, 10)]
        );
    }

    #[test]
    fn a_range_inside_a_subtitle_shortens_it_once_and_moves_later_subtitles() {
        let mut project = project();
        edit(&mut project, range(20, 30));
        assert_eq!(
            cues(&project, "S1"),
            [("q6".into(), 10, 20), ("q7".into(), 100, 30)]
        );
    }

    #[test]
    fn a_ripple_delete_cuts_subtitles_out_with_the_picture() {
        let mut project = project();
        edit(&mut project, ripple_delete(&["c1"]));
        assert_eq!(cues(&project, "S1"), [("q7".into(), 10, 30)]);
        assert_eq!(clips(&project, "V1"), rows(&[("c3", 0, 50, 150)]));
    }

    #[test]
    fn a_gap_is_closed_through_subtitles_that_are_removed_or_trimmed() {
        let mut project = with(project(), vec![lock("V1"), lock("A1")]);
        edit(
            &mut project,
            Edit::CloseGap {
                timeline: "t1".into(),
                track: "V2".into(),
                at: 30,
            },
        );
        assert_eq!(cues(&project, "S1"), [("q7".into(), 0, 20)]);
        assert_eq!(clips(&project, "V2"), rows(&[("c5", 0, 50, 50)]));
    }

    #[test]
    fn a_range_on_named_tracks_leaves_other_subtitles_alone() {
        let mut project = project();
        let before = cues(&project, "S1");
        edit(
            &mut project,
            Edit::RippleDeleteRange {
                timeline: "t1".into(),
                from: 20,
                to: 30,
                tracks: Some(vec!["V1".into(), "A1".into()]),
            },
        );
        assert_eq!(cues(&project, "S1"), before);
    }

    proptest::proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(1000))]

        #[test]
        fn every_range_inside_a_subtitle_is_cut_out_without_blocking(
            start in 11i64..39,
            width in 1i64..30,
        ) {
            let end = (start + width).min(39);
            let mut project = project();
            edit(&mut project, range(start, end));
            proptest::prop_assert_eq!(
                cues(&project, "S1"),
                [("q6".into(), 10, 30 - (end - start)),
                 ("q7".into(), 110 - (end - start), 30)]
            );
        }
    }

    #[test]
    fn a_range_of_no_frames_is_refused() {
        assert_eq!(refused(project(), range(30, 30)), EditError::EmptyRange);
        let edit_ = Edit::RippleDeleteRange {
            timeline: "t1".into(),
            from: 10,
            to: 20,
            tracks: Some(vec!["V1".into(), "A9".into()]),
        };
        assert!(matches!(refused(project(), edit_), EditError::Op(_)));
    }

    #[test]
    fn a_gap_is_closed_and_one_that_is_not_there_is_refused() {
        let mut project = with(project(), vec![lock("V1"), lock("A1"), lock("S1")]);
        let close = |at| Edit::CloseGap {
            timeline: "t1".into(),
            track: "V2".into(),
            at,
        };
        edit(&mut project, close(30));
        assert_eq!(clips(&project, "V2"), rows(&[("c5", 0, 50, 50)]));
        assert_eq!(refused(project.clone(), close(10)), EditError::EmptyRange);
        assert_eq!(refused(project, close(60)), EditError::EmptyRange);
    }

    #[test]
    fn a_blade_cuts_the_clips_across_it_and_those_linked_to_them() {
        let mut project = project();
        edit(&mut project, blade(30, Some(&["V1"]), true));
        assert_eq!(
            clips(&project, "V1"),
            rows(&[("c1", 0, 30, 0), ("c10", 30, 70, 30), ("c3", 100, 50, 150)])
        );
        assert_eq!(clips(&project, "A1")[1], ("c12".into(), 30, 70, 30));
        assert_eq!(the(&project, "c10").link, the(&project, "c12").link);
        assert_eq!(the(&project, "c1").link.as_deref(), Some("g1"));
        assert_eq!(the(&project, "c10").link.as_deref(), Some("g11"));
    }

    #[test]
    fn a_blade_unlinked_cuts_its_track_alone_and_one_that_cuts_nothing_is_refused() {
        let mut project = project();
        edit(&mut project, blade(30, Some(&["V1"]), false));
        assert_eq!(clips(&project, "A1").len(), 2);
        let error = refused(project, blade(100, None, true));
        assert_eq!(
            error,
            EditError::NothingAt {
                track: None,
                frame: 100
            }
        );
    }

    #[test]
    fn a_blade_gives_the_transition_at_the_end_of_the_clip_to_its_second_half() {
        let mut project = with(
            project(),
            vec![transition("V1", "c1", "c3", 10, Alignment::Center)],
        );
        // The transition is x10, and the second half c11.
        edit(&mut project, blade(50, Some(&["V1"]), false));
        assert_eq!(
            transitions(&project, "V1"),
            [("c11".into(), "c3".into(), 10)]
        );
    }

    #[test]
    fn a_blade_inside_a_transition_shortens_it_and_undo_restores_its_length() {
        for (at, length) in [(95, 11), (105, 10)] {
            let before = with(
                project(),
                vec![transition("V1", "c1", "c3", 20, Alignment::Center)],
            );
            let mut history = crate::History::new(before.clone());
            let changed = history
                .edit(vec![blade(at, Some(&["V1"]), false)], "desktop")
                .unwrap();
            assert_eq!(changed.clamped, [0]);
            let after = history.project().clone();
            let transition = &after.timelines[0].tracks[0].transitions[0];
            assert_eq!((transition.id.as_str(), transition.length), ("x10", length));
            let mut checked = after.clone();
            checked.validate().unwrap();
            history.undo("agent").unwrap().unwrap();
            assert_eq!(
                history.project(),
                &crate::Project {
                    next_id: after.next_id,
                    ..before
                }
            );
            history.redo("desktop").unwrap().unwrap();
            assert_eq!(history.project(), &after);
        }
    }

    #[test]
    fn a_range_inside_a_transition_keeps_the_longest_transition_that_fits() {
        let mut project = with(
            project(),
            vec![transition("V1", "c1", "c3", 20, Alignment::Center)],
        );
        let done = edit(&mut project, range(95, 98));
        assert!(done.clamped);
        assert_eq!(project.timelines[0].tracks[0].transitions[0].length, 5);
    }

    #[test]
    fn a_clip_played_backward_is_bladed_so_that_its_halves_meet_in_its_source() {
        let mut project = project();
        let mut reversed = the(&project, "c5");
        reversed.reverse = true;
        project.timelines[0].tracks[1].clips[0] = reversed;
        edit(&mut project, blade(150, Some(&["V2"]), true));
        // c5 shows [50, 100) of m1 backward: its first 30 frames show [70, 100).
        assert_eq!(
            clips(&project, "V2"),
            rows(&[("c5", 120, 30, 70), ("c10", 150, 20, 50)])
        );
    }

    #[test]
    fn a_move_takes_the_linked_clips_along_and_refuses_to_land_on_another() {
        let mut project = with(project(), vec![]);
        edit(&mut project, move_by(&["c3"], 50, None, false));
        assert_eq!(clips(&project, "V1")[1], ("c3".into(), 150, 50, 150));
        assert_eq!(clips(&project, "A1")[1], ("c4".into(), 150, 50, 150));
        let error = refused(project, move_by(&["c3"], -60, None, false));
        assert_eq!(error, EditError::NoRoom { track: "V1".into() });
    }

    #[test]
    fn a_move_to_another_track_overwrites_what_is_there_if_asked() {
        let mut project = project();
        edit(&mut project, move_by(&["c3"], 0, Some("V2"), true));
        // c3 lands on V2 over [100, 150): c5 keeps [150, 170). The sound stays on A1.
        assert_eq!(
            clips(&project, "V2"),
            rows(&[("c3", 100, 50, 150), ("c10", 150, 20, 80)])
        );
        assert_eq!(clips(&project, "V1"), rows(&[("c1", 0, 100, 0)]));
        assert_eq!(clips(&project, "A1")[1], ("c4".into(), 100, 50, 150));
    }

    #[test]
    fn a_move_before_the_start_goes_as_far_as_the_start() {
        let mut project = with(project(), vec![]);
        let expansion = edit(&mut project, move_by(&["c5"], -500, None, false));
        assert!(expansion.clamped);
        assert_eq!(clips(&project, "V2"), rows(&[("c5", 0, 50, 50)]));
        assert_eq!(
            refused(project, move_by(&["c5"], -10, None, false)),
            EditError::EmptyRange
        );
    }

    #[test]
    fn an_insert_move_takes_clips_out_and_puts_them_in_elsewhere() {
        let mut project = with(project(), vec![lock("V2"), lock("S1")]);
        // c3 and c4 go in at the start, before c1 and c2.
        edit(
            &mut project,
            Edit::InsertMove {
                timeline: "t1".into(),
                clips: vec!["c3".into()],
                at: 0,
                linked: true,
            },
        );
        assert_eq!(
            clips(&project, "V1"),
            rows(&[("c3", 0, 50, 150), ("c1", 50, 100, 0)])
        );
        assert_eq!(
            clips(&project, "A1"),
            rows(&[("c4", 0, 50, 150), ("c2", 50, 100, 0)])
        );
    }

    #[test]
    fn an_insert_move_of_separate_clips_inserts_only_the_frames_taken_out() {
        let mut before = project();
        before.timelines[0].tracks.retain(|r| r.id == "V1");
        before.timelines[0].tracks[0].clips = ["c1", "middle", "c3", "tail", "last"]
            .into_iter()
            .zip(0..)
            .map(|(id, n)| clip(id, n * 25, 25, n * 25, None))
            .collect();
        before.validate().unwrap();
        let mut history = crate::History::new(before.clone());
        history
            .edit(
                vec![Edit::InsertMove {
                    timeline: "t1".into(),
                    clips: vec!["c3".into(), "c1".into()],
                    at: 100,
                    linked: false,
                }],
                "test",
            )
            .unwrap();
        let after = history.project().clone();
        assert_eq!(
            clips(&after, "V1"),
            rows(&[
                ("middle", 0, 25, 25),
                ("tail", 25, 25, 75),
                ("c1", 50, 25, 0),
                ("c3", 75, 25, 50),
                ("last", 100, 25, 100),
            ])
        );
        history.undo("test").unwrap().unwrap();
        assert_eq!(history.project(), &before);
        history.redo("test").unwrap().unwrap();
        assert_eq!(history.project(), &after);
    }

    #[test]
    fn a_lift_takes_the_transition_at_the_clip_too() {
        let mut project = with(
            project(),
            vec![transition("V1", "c1", "c3", 10, Alignment::Center)],
        );
        edit(
            &mut project,
            Edit::Lift {
                timeline: "t1".into(),
                clips: vec!["c3".into()],
                linked: true,
            },
        );
        assert_eq!(transitions(&project, "V1"), []);
    }

    #[test]
    fn a_range_cut_out_of_the_tracks_given_leaves_the_others() {
        let mut project = project();
        let edit_ = Edit::RippleDeleteRange {
            timeline: "t1".into(),
            from: 0,
            to: 50,
            tracks: Some(vec!["V1".into(), "A1".into()]),
        };
        edit(&mut project, edit_);
        assert_eq!(
            clips(&project, "V1"),
            rows(&[("c10", 0, 50, 50), ("c3", 50, 50, 150)])
        );
        assert_eq!(clips(&project, "V2"), rows(&[("c5", 120, 50, 50)]));
        assert_eq!(cues(&project, "S1")[0], ("q6".into(), 10, 30));
    }

    #[test]
    fn a_gap_between_subtitles_is_closed_unless_another_track_is_in_it() {
        let close = Edit::CloseGap {
            timeline: "t1".into(),
            track: "S1".into(),
            at: 50,
        };
        // The gap is [40, 110): c1 on V1 is in it.
        let error = refused(project(), close.clone());
        assert_eq!(
            error,
            EditError::RippleBlocked {
                track: "V1".into(),
                clip: "c1".into()
            }
        );
        let mut project = with(project(), vec![lock("V1"), lock("V2"), lock("A1")]);
        edit(&mut project, close.clone());
        assert_eq!(cues(&project, "S1")[1], ("q7".into(), 40, 30));
        let error = refused(with(project, vec![lock("S1")]), close);
        assert_eq!(error, EditError::Locked { track: "S1".into() });
    }

    #[test]
    fn an_insert_move_into_a_clip_cuts_it() {
        let mut project = with(project(), vec![lock("V2"), lock("S1")]);
        edit(
            &mut project,
            Edit::InsertMove {
                timeline: "t1".into(),
                clips: vec!["c3".into()],
                at: 50,
                linked: true,
            },
        );
        assert_eq!(
            clips(&project, "V1"),
            rows(&[("c1", 0, 50, 0), ("c3", 50, 50, 150), ("c10", 100, 50, 50)])
        );
        assert_eq!(
            clips(&project, "A1"),
            rows(&[("c2", 0, 50, 0), ("c4", 50, 50, 150), ("c12", 100, 50, 50)])
        );
    }

    #[test]
    fn an_insert_move_whose_gap_another_track_is_in_is_refused() {
        let project = with(
            project(),
            vec![crate::Op::MoveClip {
                timeline: "t1".into(),
                id: "c5".into(),
                track: "V2".into(),
                start: 25,
            }],
        );
        let edit_ = Edit::InsertMove {
            timeline: "t1".into(),
            clips: vec!["c1".into()],
            at: 150,
            linked: true,
        };
        let error = refused(project, edit_);
        assert_eq!(
            error,
            EditError::RippleBlocked {
                track: "V2".into(),
                clip: "c5".into()
            }
        );
    }
}
