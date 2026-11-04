//! Random edits of random timelines: each is refused and changes nothing, or is applied whole and
//! leaves a project that the operations could have made; it is undone and redone exactly; it
//! leaves locked tracks alone; a ripple keeps what follows it in step; a roll and an interior
//! slide keep how long the timeline lasts, and a slip where its clips are; a blade leaves two halves
//! that meet on the timeline and in their source; and a move to another track takes every clip as
//! many tracks along its own kind.

#[path = "common/edits.rs"]
mod gen;

use std::collections::{BTreeMap, HashSet};

use columbina_model::resolve::duration;
use columbina_model::span::source_span;
use columbina_model::{
    expand, Clip, Edit, EditError, History, Project, Straddling, Track, TrackKind,
};
use gen::{check, clips_of, edit_of, making, making_of, project_of, recipe, Making, Recipe};
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;

/// Apply the edits that `makings` make, one after another, each against the project that the
/// one before left, and look at each with `look`: the project before, the edit, and the project
/// after or why it was refused.
fn each_edit(
    recipe: &Recipe,
    makings: &[Making],
    mut look: impl FnMut(&Project, &Edit, Result<&Project, &EditError>) -> Result<(), TestCaseError>,
) -> Result<(), TestCaseError> {
    let mut project = project_of(recipe);
    check(&project);
    for making in makings {
        let edit = edit_of(&project, making);
        let before = project.clone();
        match expand(&mut project, edit.clone()) {
            Ok(_) => look(&before, &edit, Ok(&project))?,
            Err(error) => {
                prop_assert_eq!(&project, &before, "{:?} was refused: {}", edit, error);
                look(&before, &edit, Err(&error))?;
            }
        }
    }
    Ok(())
}

fn track_in<'p>(project: &'p Project, id: &str) -> Option<&'p Track> {
    project.timelines[0].tracks.iter().find(|r| r.id == id)
}

/// The ids of the tracks of `kind`, the first at the bottom.
fn of_kind(project: &Project, kind: TrackKind) -> Vec<&str> {
    let tracks = project.timelines[0].tracks.iter();
    tracks
        .filter(|r| r.kind == kind)
        .map(|r| r.id.as_str())
        .collect()
}

/// The track that a clip on `track` goes to, `offset` tracks along its kind, if there is one.
fn along(project: &Project, track: &Track, offset: i64) -> Option<String> {
    let tracks = of_kind(project, track.kind);
    let here = tracks.iter().position(|id| *id == track.id)?;
    let to = i64::try_from(here).ok()? + offset;
    let to = tracks.get(usize::try_from(to).ok()?)?;
    Some((*to).to_string())
}

fn clip<'p>(project: &'p Project, id: &str) -> Option<&'p Clip> {
    clips_of(project).into_iter().find(|c| c.id == id)
}

/// The clips that an edit of `clips` works on with the clips linked to them: a clip linked to one
/// of them on a locked track is left alone.
fn group(project: &Project, clips: &[String], linked: bool) -> HashSet<String> {
    let given: Vec<&Clip> = clips.iter().filter_map(|id| clip(project, id)).collect();
    let groups: HashSet<&String> = given.iter().filter_map(|c| c.link.as_ref()).collect();
    let tracks = project.timelines[0].tracks.iter();
    tracks
        .flat_map(|r| r.clips.iter().map(move |c| (r, c)))
        .filter(|(r, c)| {
            clips.contains(&c.id)
                || (linked && !r.locked && c.link.as_ref().is_some_and(|g| groups.contains(g)))
        })
        .map(|(_, c)| c.id.clone())
        .collect()
}

/// For an edit that ripples, the frame from which on what is on the tracks that are not locked
/// moves by one number of frames, and the clips that the edit changes itself.
fn ripple_of(project: &Project, edit: &Edit) -> Option<(i64, HashSet<String>)> {
    let end_of = |ids: &HashSet<String>| {
        ids.iter()
            .filter_map(|id| clip(project, id))
            .map(Clip::end)
            .max()
    };
    match edit {
        Edit::Insert { at, .. } => Some((*at, HashSet::new())),
        Edit::CloseGap { track, at, .. } => {
            let track = track_in(project, track)?;
            // Subtitles on other tracks inside the gap are cut with it. Only what starts
            // beyond the gap must keep its length and move by the same amount.
            let end = track
                .clips
                .iter()
                .map(|c| c.start)
                .chain(track.cues.iter().map(|q| q.start))
                .filter(|start| start > at)
                .min()?;
            Some((end, HashSet::new()))
        }
        Edit::RippleDelete { clips, linked, .. } => {
            let changed = group(project, clips, *linked);
            Some((end_of(&changed)?, changed))
        }
        Edit::RippleDeleteRange {
            to, tracks: None, ..
        } => Some((*to, HashSet::new())),
        Edit::Trim {
            clip: id,
            ripple: true,
            linked,
            ..
        } => {
            let changed = group(project, std::slice::from_ref(id), *linked);
            Some((clip(project, id)?.end(), changed))
        }
        Edit::SetSpeed {
            clip: id,
            ripple: true,
            linked,
            ..
        } => {
            let changed = group(project, std::slice::from_ref(id), *linked);
            Some((end_of(&changed)?, changed))
        }
        Edit::InsertMove {
            clips, at, linked, ..
        } => {
            let changed = group(project, clips, *linked);
            Some((end_of(&changed)?.max(*at), changed))
        }
        _ => None,
    }
}

/// For an edit that ripples, how far what follows it must move: by the length of what an insert
/// puts in, by minus the frames that a ripple delete or a close takes out, by the change in the
/// end of what a ripple trim or a change of speed changes, and not at all for an insert move,
/// which takes out as many frames as it puts in.
fn shift_of(before: &Project, edit: &Edit, after: &Project) -> Option<i64> {
    // Clips on locked tracks are left where they are, linked or not.
    let open: HashSet<&String> = before.timelines[0]
        .tracks
        .iter()
        .filter(|r| !r.locked)
        .flat_map(|r| r.clips.iter().map(|c| &c.id))
        .collect();
    let group = |clips: &[String], linked: bool| -> HashSet<String> {
        let mut all = group(before, clips, linked);
        all.retain(|id| open.contains(id));
        all
    };
    let ends = |project: &Project, ids: &HashSet<String>| {
        ids.iter()
            .filter_map(|id| clip(project, id))
            .map(Clip::end)
            .max()
    };
    let taken = |spans: Vec<(i64, i64)>| -> i64 {
        let mut spans = spans;
        spans.sort_unstable();
        let mut total = 0;
        let mut reached = i64::MIN;
        for (from, to) in spans {
            let from = from.max(reached);
            if to > from {
                total += to - from;
            }
            reached = reached.max(to);
        }
        total
    };
    match edit {
        Edit::Insert { at, .. } => clips_of(after)
            .into_iter()
            .filter(|c| c.start == *at && clip(before, &c.id).is_none())
            .map(|c| c.length)
            .max(),
        Edit::CloseGap { track, at, .. } => {
            let track = track_in(before, track)?;
            let spans = track
                .clips
                .iter()
                .map(|c| (c.start, c.end()))
                .chain(track.cues.iter().map(|q| (q.start, q.start + q.length)));
            let spans: Vec<(i64, i64)> = spans.collect();
            let from = spans
                .iter()
                .map(|&(_, end)| end)
                .filter(|end| end <= at)
                .max()
                .unwrap_or(0);
            let to = spans
                .iter()
                .map(|&(start, _)| start)
                .filter(|start| start > at)
                .min()?;
            Some(from - to)
        }
        Edit::RippleDelete { clips, linked, .. } => {
            let changed = group(clips, *linked);
            let spans = changed
                .iter()
                .filter_map(|id| clip(before, id))
                .map(|c| (c.start, c.end()))
                .collect();
            Some(-taken(spans))
        }
        Edit::RippleDeleteRange {
            from,
            to,
            tracks: None,
            ..
        } => Some(from - to),
        Edit::Trim {
            clip: id,
            ripple: true,
            ..
        } => Some(clip(after, id)?.end() - clip(before, id)?.end()),
        Edit::SetSpeed {
            clip: id,
            ripple: true,
            linked,
            ..
        } => {
            let changed = group(std::slice::from_ref(id), *linked);
            Some(ends(after, &changed)? - ends(before, &changed)?)
        }
        Edit::InsertMove { .. } => Some(0),
        _ => None,
    }
}

/// The ripple kinds of edit, as `edit_of` numbers them: insert, ripple delete, ripple delete of a
/// range, close gap, insert move, trim and set speed.
const RIPPLES: [usize; 7] = [0, 8, 9, 10, 13, 14, 21];

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn every_edit_is_refused_and_changes_nothing_or_leaves_a_valid_project(
        recipe in recipe(),
        makings in prop::collection::vec(making(), 1..6)
    ) {
        each_edit(&recipe, &makings, |_, edit, after| {
            if let Ok(after) = after {
                check(after);
                prop_assert_eq!(after.timelines.len(), 1, "{:?}", edit);
            }
            Ok(())
        })?;
    }

    #[test]
    fn an_edit_is_undone_and_redone_exactly(
        recipe in recipe(),
        makings in prop::collection::vec(making(), 1..6)
    ) {
        let mut history = History::new(project_of(&recipe));
        for making in &makings {
            let before = history.project().clone();
            let edit = edit_of(&before, making);
            let Ok(changed) = history.edit(vec![edit.clone()], "agent") else {
                prop_assert_eq!(history.project(), &before);
                continue;
            };
            let after = history.project().clone();
            check(&after);
            // A client that applies what was done to its copy keeps up.
            let mut client = Project { next_id: changed.applied.next_id, ..before.clone() };
            for op in &changed.applied.done {
                columbina_model::apply(&mut client, op.clone())
                    .unwrap_or_else(|e| panic!("the client refused {op:?} of {edit:?}: {e}"));
            }
            prop_assert_eq!(&client, &after, "{:?}", edit);
            history.undo("desktop").unwrap().unwrap();
            let expected = Project { next_id: after.next_id, ..before.clone() };
            prop_assert_eq!(history.project(), &expected, "{:?} undone", edit);
            history.redo("desktop").unwrap().unwrap();
            prop_assert_eq!(history.project(), &after, "{:?} redone", edit);
        }
    }

    #[test]
    fn a_locked_track_is_the_same_after_every_edit(
        recipe in recipe(),
        makings in prop::collection::vec(making(), 1..6)
    ) {
        each_edit(&recipe, &makings, |before, edit, after| {
            let Ok(after) = after else { return Ok(()) };
            for locked in before.timelines[0].tracks.iter().filter(|r| r.locked) {
                let now = track_in(after, &locked.id);
                prop_assert_eq!(
                    serde_json::to_value(now).unwrap(),
                    serde_json::to_value(Some(locked)).unwrap(),
                    "{:?}", edit
                );
            }
            Ok(())
        })?;
    }

    #[test]
    fn what_follows_a_ripple_on_the_open_tracks_stays_in_step(
        recipe in recipe(),
        makings in prop::collection::vec(
            prop_oneof![3 => making_of(prop::sample::select(&RIPPLES[..])), 1 => making()],
            1..6,
        )
    ) {
        each_edit(&recipe, &makings, |before, edit, after| {
            let Ok(after) = after else { return Ok(()) };
            let Some((place, changed)) = ripple_of(before, edit) else { return Ok(()) };
            let mut shifts = HashSet::new();
            for track in before.timelines[0].tracks.iter().filter(|r| !r.locked) {
                let now = track_in(after, &track.id);
                for c in track.clips.iter().filter(|c| c.start >= place && !changed.contains(&c.id)) {
                    let moved = now.and_then(|r| r.clips.iter().find(|n| n.id == c.id));
                    let moved = moved.unwrap_or_else(|| panic!("{} went in {edit:?}", c.id));
                    prop_assert_eq!(
                        (moved.length, moved.source_in()), (c.length, c.source_in()),
                        "{} in {:?}", c.id, edit
                    );
                    shifts.insert(moved.start - c.start);
                }
                for q in track.cues.iter().filter(|q| q.start >= place) {
                    let moved = now.and_then(|r| r.cues.iter().find(|n| n.id == q.id));
                    let moved = moved.unwrap_or_else(|| panic!("{} went in {edit:?}", q.id));
                    prop_assert_eq!(moved.length, q.length);
                    shifts.insert(moved.start - q.start);
                }
            }
            prop_assert!(shifts.len() <= 1, "{:?} moved what follows by {:?}", edit, shifts);
            // After a ripple trim or change of speed of a split edit, what starts under the end of
            // the group, after a clip of the group on its own track, moves with its partners: an
            // L or J cut that follows is not parted.
            let ends_a_group = matches!(edit, Edit::SetSpeed { .. } | Edit::Trim { .. });
            let open: Vec<&Clip> = before.timelines[0].tracks.iter().filter(|r| !r.locked)
                .flat_map(|r| &r.clips).collect();
            let all: Vec<&Clip> = clips_of(before);
            let under = |c: &Clip| {
                before.timelines[0].tracks.iter().any(|r| {
                    r.clips.iter().any(|o| o.id == c.id)
                        && r.clips.iter().any(|m| changed.contains(&m.id) && m.end() <= c.start && c.start < place)
                })
            };
            for c in open.iter().filter(|c| ends_a_group && c.link.is_some() && !changed.contains(&c.id) && under(c)) {
                let members: Vec<&&Clip> = all.iter().filter(|o| o.link == c.link).collect();
                let whole = members.iter().all(|m| open.iter().any(|o| o.id == m.id) && !changed.contains(&m.id));
                let kept = members.iter().map(|m| clip(after, &m.id).filter(|n| (n.length, n.source_in()) == (m.length, m.source_in())));
                let kept: Option<Vec<&Clip>> = kept.collect();
                if let (true, Some(kept)) = (whole, kept) {
                    let moved: HashSet<i64> = members.iter().zip(&kept).map(|(m, n)| n.start - m.start).collect();
                    let rows: Vec<(String, i64, i64)> = members.iter().map(|m| (m.id.clone(), m.start, m.end())).collect();
                    prop_assert!(moved.len() == 1, "{:?} parted group {:?} {:?}: {:?}", edit, c.link, rows, moved);
                }
            }
            if let (Some(&moved), Some(shift)) = (shifts.iter().next(), shift_of(before, edit, after)) {
                prop_assert_eq!(moved, shift, "{:?} moved what follows by the wrong amount", edit);
            }
            Ok(())
        })?;
    }

    #[test]
    fn a_roll_and_a_slide_keep_the_length_of_the_timeline_and_a_slip_keeps_the_clips_in_place(
        recipe in recipe(),
        makings in prop::collection::vec(
            prop_oneof![3 => making_of(15usize..=17), 1 => making()],
            1..6,
        )
    ) {
        each_edit(&recipe, &makings, |before, edit, after| {
            let Ok(after) = after else { return Ok(()) };
            match edit {
                Edit::Roll { .. } => {
                    prop_assert_eq!(
                        duration(&after.timelines[0]), duration(&before.timelines[0]),
                        "{:?}", edit
                    );
                }
                Edit::Slide { clip: id, linked, .. } => {
                    let selected = group(before, std::slice::from_ref(id), *linked);
                    let mut interior = true;
                    for track in before.timelines[0].tracks.iter().filter(|r| !r.locked) {
                        for c in track.clips.iter().filter(|c| selected.contains(&c.id)) {
                            let now = clip(after, &c.id).unwrap();
                            prop_assert_eq!((&now.source, now.length), (&c.source, c.length));
                            if !track.clips.iter().any(|other| other.start >= c.end()) {
                                interior = false;
                            }
                        }
                    }
                    if interior {
                        prop_assert_eq!(duration(&after.timelines[0]), duration(&before.timelines[0]), "{:?}", edit);
                    }
                }
                Edit::Slip { .. } => {
                    for c in clips_of(before) {
                        let now = clip(after, &c.id).unwrap();
                        prop_assert_eq!((now.start, now.length), (c.start, c.length), "{:?}", edit);
                    }
                    prop_assert_eq!(clips_of(after).len(), clips_of(before).len());
                }
                _ => {}
            }
            Ok(())
        })?;
    }

    #[test]
    fn an_insert_on_open_tracks_always_moves_later_content_by_its_length(
        recipe in recipe(), at in 0i64..500, length in 1i64..120,
    ) {
        let mut project = project_of(&recipe);
        for track in &mut project.timelines[0].tracks { track.locked = false; }
        let before = project.clone();
        let at = at.min(duration(&project.timelines[0]));
        let edit = Edit::Insert {
            timeline: "t1".into(), at,
            source: columbina_model::Material::Color { color: columbina_model::Rgba([0, 0, 0, 255]), length },
            video_track: Some(Some("V1".into())), audio_track: None,
        };
        expand(&mut project, edit).unwrap();
        check(&project);
        for old in clips_of(&before).into_iter().filter(|c| c.start >= at) {
            let new = clip(&project, &old.id).unwrap();
            prop_assert_eq!(new.start, old.start + length);
            prop_assert_eq!((&new.source, new.length), (&old.source, old.length));
        }
        for old in before.timelines[0].tracks.iter().flat_map(|r| &r.cues).filter(|q| q.start >= at) {
            let new = project.timelines[0].tracks.iter().flat_map(|r| &r.cues).find(|q| q.id == old.id).unwrap();
            prop_assert_eq!(new.start, old.start + length);
            prop_assert_eq!((&new.text, new.length), (&old.text, old.length));
        }
    }

    #[test]
    fn an_insert_move_keeps_every_subtitle_and_takes_those_over_its_clips_along(
        recipe in recipe(),
        makings in prop::collection::vec(
            prop_oneof![3 => making_of(Just(13usize)), 1 => making()],
            1..6,
        )
    ) {
        each_edit(&recipe, &makings, |before, edit, after| {
            let (Ok(after), Edit::InsertMove { clips, linked, .. }) = (after, edit) else {
                return Ok(());
            };
            // What each subtitle says is shown for as many frames as before: one that is cut at
            // an end of what moves, or where it goes in, is shown in two parts.
            let shown = |track: &Track| {
                let mut frames: BTreeMap<String, i64> = BTreeMap::new();
                for q in &track.cues {
                    *frames.entry(q.text.clone()).or_default() += q.length;
                }
                frames
            };
            for track in &before.timelines[0].tracks {
                let now = track_in(after, &track.id).unwrap();
                prop_assert_eq!(shown(track), shown(now), "{:?} on {}", edit, track.id);
            }
            // A subtitle over a clip that moves keeps its id and its place on the clip.
            let open: HashSet<&String> = before.timelines[0].tracks.iter().filter(|r| !r.locked)
                .flat_map(|r| r.clips.iter().map(|c| &c.id)).collect();
            let mut moved = group(before, clips, *linked);
            moved.retain(|id| open.contains(id));
            for track in before.timelines[0].tracks.iter().filter(|r| !r.locked) {
                let now = track_in(after, &track.id).unwrap();
                for q in &track.cues {
                    let under = moved.iter().filter_map(|id| clip(before, id)).find(|c| {
                        c.start <= q.start && q.start + q.length <= c.end()
                    });
                    let Some(c) = under else { continue };
                    let went = clip(after, &c.id).unwrap();
                    let q_now = now.cues.iter().find(|n| n.id == q.id);
                    let q_now = q_now.unwrap_or_else(|| panic!("{} went in {edit:?}", q.id));
                    prop_assert_eq!(
                        (q_now.start - went.start, q_now.length, &q_now.text),
                        (q.start - c.start, q.length, &q.text),
                        "{} over {} in {:?}", q.id, c.id, edit
                    );
                }
            }
            Ok(())
        })?;
    }

    #[test]
    fn the_halves_of_a_bladed_clip_meet_on_the_timeline_and_in_their_source(
        recipe in recipe(),
        makings in prop::collection::vec(
            prop_oneof![3 => making_of(Just(11usize)), 1 => making()],
            1..6,
        )
    ) {
        each_edit(&recipe, &makings, |before, edit, after| {
            let (Ok(after), Edit::Blade { at, .. }) = (after, edit) else { return Ok(()) };
            let rate = before.timelines[0].rate;
            let mut cut = 0;
            for track in &before.timelines[0].tracks {
                for c in track.clips.iter().filter(|c| c.start < *at && *at < c.end()) {
                    let now = track_in(after, &track.id).unwrap();
                    let first = now.clips.iter().find(|n| n.id == c.id).unwrap();
                    if first.length == c.length {
                        continue;
                    }
                    cut += 1;
                    let second = now.clips.iter().find(|n| n.start == *at).unwrap();
                    prop_assert_eq!(first.end(), second.start);
                    prop_assert_eq!(second.end(), c.end());
                    prop_assert!(clip(before, &second.id).is_none());
                    prop_assert_eq!((second.speed, second.reverse), (c.speed, c.reverse));
                    if let (Some(a), Some(b)) = (first.source_in(), second.source_in()) {
                        let (earlier, later, by) = if c.reverse {
                            (b, a, source_span(second, rate, b.scale))
                        } else {
                            (a, b, source_span(first, rate, a.scale))
                        };
                        prop_assert_eq!(earlier.ticks + by, later.ticks, "{:?} of {:?}", edit, c);
                    }
                }
            }
            prop_assert!(cut > 0, "{:?} cut nothing", edit);
            Ok(())
        })?;
    }

    #[test]
    fn a_gap_closed_with_clips_across_it_staying_closes_the_gap_and_leaves_them_alone(
        recipe in recipe(),
        makings in prop::collection::vec(
            prop_oneof![3 => making_of(Just(10usize)), 1 => making()],
            1..6,
        )
    ) {
        each_edit(&recipe, &makings, |before, edit, after| {
            let Edit::CloseGap { track, at, straddling: Straddling::Stay, .. } = edit else {
                return Ok(());
            };
            let Some(gap) = track_in(before, track) else { return Ok(()) };
            let spans: Vec<(i64, i64)> = gap.clips.iter().map(|c| (c.start, c.end()))
                .chain(gap.cues.iter().map(|q| (q.start, q.start + q.length))).collect();
            let from = spans.iter().map(|&(_, e)| e).filter(|e| e <= at).max().unwrap_or(0);
            let Some(to) = spans.iter().map(|&(s, _)| s).filter(|s| s > at).min() else {
                return Ok(());
            };
            let across = |r: &Track| -> Vec<Clip> {
                r.clips.iter().filter(|c| c.start < to && from < c.end()).cloned().collect()
            };
            match after {
                Ok(after) => {
                    // What lies across the gap is as it was, every clip of it.
                    for r in &before.timelines[0].tracks {
                        let now = track_in(after, &r.id).unwrap();
                        for c in across(r) {
                            prop_assert_eq!(
                                serde_json::to_value(now.clips.iter().find(|n| n.id == c.id)).unwrap(),
                                serde_json::to_value(Some(&c)).unwrap(),
                                "{} in {:?}", c.id, edit
                            );
                        }
                    }
                    // On the track of the gap, what was after it now starts where it began.
                    let now = track_in(after, track).unwrap();
                    let first = now.clips.iter().map(|c| c.start)
                        .chain(now.cues.iter().map(|q| q.start)).filter(|&s| s >= from).min();
                    prop_assert_eq!(first, Some(from), "{:?}", edit);
                }
                Err(error) => {
                    // Refused only where a clip after the gap would run into one across it.
                    let EditError::RippleBlocked { track: blocked, clip: stays } = error else {
                        return Ok(());
                    };
                    let r = track_in(before, blocked).unwrap();
                    prop_assert!(!r.locked);
                    let stays = r.clips.iter().find(|c| &c.id == stays).unwrap();
                    prop_assert!(stays.start < to && from < stays.end());
                    let first = r.clips.iter().map(|c| c.start).filter(|&s| s >= to).min();
                    prop_assert!(first.is_some_and(|f| f - (to - from) < stays.end()), "{:?}", edit);
                }
            }
            Ok(())
        })?;
    }

    #[test]
    fn a_move_to_another_track_takes_every_clip_as_many_tracks_along_its_kind(
        recipe in recipe(),
        makings in prop::collection::vec(
            prop_oneof![3 => making_of(Just(12usize)), 1 => making()],
            1..6,
        )
    ) {
        each_edit(&recipe, &makings, |before, edit, after| {
            let Edit::Move { clips, to_track: Some(to), linked, .. } = edit else {
                return Ok(());
            };
            let tracks = &before.timelines[0].tracks;
            let on = |id: &str| tracks.iter().find(|r| r.clips.iter().any(|c| c.id == id));
            let (Some(first), Some(to)) = (clips.first().and_then(|c| on(c)), track_in(before, to))
            else {
                return Ok(());
            };
            if first.kind != to.kind {
                return Ok(());
            }
            let place = |r: &Track| of_kind(before, r.kind).iter().position(|id| *id == r.id);
            let offset = i64::try_from(place(to).unwrap()).unwrap()
                - i64::try_from(place(first).unwrap()).unwrap();
            // What moves: the clips named, and those linked to them on tracks that are open.
            let moved: Vec<(String, &Track)> = group(before, clips, *linked)
                .into_iter()
                .filter_map(|id| on(&id).map(|r| (id, r)))
                .collect();
            match after {
                Ok(after) => {
                    for (id, from) in &moved {
                        let went = along(before, from, offset).unwrap();
                        let now = after.timelines[0].tracks.iter()
                            .find(|r| r.clips.iter().any(|c| &c.id == id)).unwrap();
                        prop_assert_eq!(&now.id, &went, "{} in {:?}", id, edit);
                    }
                }
                Err(EditError::NoTrack { kind }) => {
                    // Refused only where a clip that moves has no track of its kind to go to.
                    prop_assert!(
                        moved.iter().any(|(_, r)| r.kind == *kind && along(before, r, offset).is_none()),
                        "{:?}", edit
                    );
                }
                Err(EditError::Locked { track }) => {
                    let goes = moved.iter().any(|(_, r)| along(before, r, offset).as_ref() == Some(track));
                    let named = clips.iter().any(|id| on(id).is_some_and(|r| &r.id == track));
                    prop_assert!(goes || named, "{:?}", edit);
                }
                Err(_) => {}
            }
            Ok(())
        })?;
    }
}

#[test]
fn the_random_timelines_have_fades_of_sound_and_channels_named() {
    use proptest::strategy::{Strategy, ValueTree};
    let mut runner = proptest::test_runner::TestRunner::deterministic();
    let (mut faded, mut mapped) = (0, 0);
    for _ in 0..200 {
        let recipe = recipe().new_tree(&mut runner).unwrap().current();
        let project = project_of(&recipe);
        for clip in clips_of(&project) {
            faded += usize::from(clip.audio_fade_in.is_some() || clip.audio_fade_out.is_some());
            mapped += usize::from(clip.channels.is_some());
        }
    }
    assert!(
        faded >= 20 && mapped >= 20,
        "{faded} faded, {mapped} mapped"
    );
}

#[test]
fn the_random_edits_of_sound_are_applied_as_well_as_refused() {
    use proptest::strategy::{Strategy, ValueTree};
    let mut runner = proptest::test_runner::TestRunner::deterministic();
    let mut applied = [0usize; 14];
    let mut followed = 0;
    let mut crossfaded = 0;
    for _ in 0..300 {
        let recipe = recipe().new_tree(&mut runner).unwrap().current();
        let mut project = project_of(&recipe);
        // A transition of picture, linked, at a cut of picture whose sound meets too.
        let pairs = linked_cuts_of(&project);
        if let Some((left, right)) = pairs.first() {
            let edit: Edit = serde_json::from_value(serde_json::json!({"edit": "add_transition",
                "timeline": "t1", "left": left, "right": right, "length": 2, "linked": true}))
            .unwrap();
            if let Ok(expansion) = expand(&mut project, edit) {
                crossfaded += usize::from(expansion.done.len() > 1);
            }
        }
        for kind in 28..42 {
            let making = making_of(Just(kind))
                .new_tree(&mut runner)
                .unwrap()
                .current();
            let edit = edit_of(&project, &making);
            if let Ok(expansion) = expand(&mut project, edit) {
                applied[kind - 28] += 1;
                followed += usize::from(kind == 28 && expansion.done.len() > 1);
            }
        }
    }
    assert!(applied.iter().all(|&n| n >= 20), "{applied:?}");
    assert!(followed >= 10, "{followed} followed by a group");
    assert!(
        crossfaded >= 1,
        "no transition of picture brought a crossfade"
    );
}

/// The cuts of picture of t1 whose two clips are linked to clips of sound that meet too.
fn linked_cuts_of(project: &Project) -> Vec<(String, String)> {
    let tracks = &project.timelines[0].tracks;
    let sound: Vec<&Clip> = tracks
        .iter()
        .filter(|r| r.kind == TrackKind::Audio)
        .flat_map(|r| &r.clips)
        .collect();
    let meets = |l: &Clip, r: &Clip| {
        sound.iter().any(|a| {
            a.link == l.link
                && sound
                    .iter()
                    .any(|b| b.link == r.link && b.start == a.end() && b.start > a.start)
        })
    };
    tracks
        .iter()
        .filter(|r| r.kind == TrackKind::Video)
        .flat_map(|r| r.clips.windows(2))
        .filter(|p| p[0].end() == p[1].start && p[0].link.is_some() && p[1].link.is_some())
        .filter(|p| meets(&p[0], &p[1]))
        .map(|p| (p[0].id.clone(), p[1].id.clone()))
        .collect()
}
