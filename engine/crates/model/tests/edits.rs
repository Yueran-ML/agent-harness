//! Random edits of random timelines: each is refused and changes nothing, or is applied whole and
//! leaves a project that the operations could have made; it is undone and redone exactly; it
//! leaves locked tracks alone; a ripple keeps what follows it in step; a roll and a slide keep
//! how long the timeline lasts, and a slip where its clips are; and a blade leaves two halves
//! that meet on the timeline and in their source.

#[path = "common/edits.rs"]
mod gen;

use std::collections::HashSet;

use columbina_model::resolve::duration;
use columbina_model::span::source_span;
use columbina_model::{expand, Clip, Edit, EditError, History, Project, Track};
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

fn clip<'p>(project: &'p Project, id: &str) -> Option<&'p Clip> {
    clips_of(project).into_iter().find(|c| c.id == id)
}

/// The clips that an edit of `clips` works on with the clips linked to them, wherever they are.
fn group(project: &Project, clips: &[String], linked: bool) -> HashSet<String> {
    let given: Vec<&Clip> = clips.iter().filter_map(|id| clip(project, id)).collect();
    let groups: HashSet<&String> = given.iter().filter_map(|c| c.link.as_ref()).collect();
    clips_of(project)
        .into_iter()
        .filter(|c| {
            clips.contains(&c.id) || (linked && c.link.as_ref().is_some_and(|g| groups.contains(g)))
        })
        .map(|c| c.id.clone())
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
                Edit::Roll { .. } | Edit::Slide { .. } => {
                    prop_assert_eq!(
                        duration(&after.timelines[0]), duration(&before.timelines[0]),
                        "{:?}", edit
                    );
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
}
