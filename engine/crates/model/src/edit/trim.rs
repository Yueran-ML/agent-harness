//! The four trims, as `DaVinci` makes them: moving an edge of a clip (with or without a ripple),
//! rolling the cut between two clips, slipping a clip in its source and sliding it between its
//! neighbours.

use std::collections::HashSet;

use super::work::{beyond, covered, edged, reach, set_source_in, Limit, Work};
use super::{Edge, EditError};
use crate::ops::OpError;
use crate::project::{Clip, Id, Track};
use crate::span::ticks_of_scaled;
use crate::time::Time;

/// The fewest frames that a clip of a track can last, with the transitions that stay at its
/// start and its end.
fn least(track: &Track, clip: &str, start: bool, end: bool) -> i64 {
    let (at_start, at_end) = covered(track, clip);
    (i64::from(start) * at_start + i64::from(end) * at_end).max(1)
}

/// The frames beyond its ends that the transitions that stay at a clip need.
fn kept_beyond(track: &Track, clip: &str, start: bool, end: bool) -> (i64, i64) {
    let (before, after) = beyond(track, clip);
    (i64::from(start) * before, i64::from(end) * after)
}

/// The end of the clip of a track before `clip`, or the start of the timeline; and the start of
/// the one after it, if any.
fn neighbours(track: &Track, clip: &Clip) -> (i64, Option<i64>) {
    let others = track.clips.iter().filter(|c| c.id != clip.id);
    let before = others
        .clone()
        .map(Clip::end)
        .filter(|&end| end <= clip.start)
        .max()
        .unwrap_or(0);
    let after = others
        .map(|c| c.start)
        .filter(|&start| start >= clip.end())
        .min();
    (before, after)
}

fn no_room(track: &Track) -> impl FnOnce() -> EditError + '_ {
    || EditError::NoRoom {
        track: track.id.clone(),
    }
}

/// A clip that is changed, and the frames that the transitions that stay at it need beyond its
/// ends.
struct Changed {
    clip: Clip,
    before: i64,
    after: i64,
}

/// Whether each of `changed`, as `make` makes it for a delta, keeps within its media.
fn all_fit(work: &Work, changed: &[Changed], make: impl Fn(&Clip) -> Clip) -> Result<(), Id> {
    for c in changed {
        if !work.fits(&make(&c.clip), c.before, c.after) {
            return Err(c.clip.id.clone());
        }
    }
    Ok(())
}

pub(super) fn trim(
    work: &mut Work,
    clip: &str,
    edge: Edge,
    delta: i64,
    ripple: bool,
    linked: bool,
) -> Result<(), EditError> {
    let all = work.group(&[clip.into()], linked)?;
    let (_, given) = work.owned(clip)?;
    // Where the ripple is: the end of the clip, which is where what follows starts.
    let place = given.end();
    let rate = work.rate();
    if ripple && delta != 0 {
        let group: Vec<(Id, Clip)> = all
            .iter()
            .map(|id| work.owned(id))
            .collect::<Result<_, _>>()?;
        work.split_ripple(&group, place)?;
    }
    let mut limit = Limit::new();
    let mut changed = vec![];
    for id in &all {
        let (track, c) = work.clip(id)?;
        // Without a ripple, the transition at the edge that moves is left behind.
        let start = ripple || edge == Edge::Out;
        let end = ripple || edge == Edge::In;
        let least = least(track, id, start, end);
        match edge {
            Edge::In => limit.at_most(c.length - least, || EditError::EmptyRange),
            Edge::Out => limit.at_least(least - c.length, || EditError::EmptyRange),
        }
        let (before, after) = neighbours(track, c);
        if !ripple {
            match (edge, after) {
                (Edge::In, _) => limit.at_least(before - c.start, no_room(track)),
                (Edge::Out, Some(after)) => limit.at_most(after - c.end(), no_room(track)),
                (Edge::Out, None) => {}
            }
        } else if let Some(after) = after.filter(|&after| after < place) {
            // What is after the clip but before the ripple stays where it is.
            match edge {
                Edge::Out => limit.at_most(after - c.end(), no_room(track)),
                Edge::In => limit.at_least(c.end() - after, no_room(track)),
            }
        }
        let (before, after) = kept_beyond(track, id, start, end);
        changed.push(Changed {
            clip: c.clone(),
            before,
            after,
        });
    }
    let (delta, clamped) = reach(delta, &limit, |d| {
        all_fit(work, &changed, |c| edged(c, edge, d, ripple, rate))
    })?;
    if clamped {
        work.clamp();
    }
    let shift = match edge {
        Edge::Out => delta,
        Edge::In => -delta,
    };
    let skip: HashSet<Id> = all.iter().cloned().collect();
    let tracks = work.all_unlocked();
    for id in &all {
        work.detach(id)?;
    }
    if ripple && shift > 0 {
        work.shift(&tracks, place, shift, &skip)?;
    }
    for c in &changed {
        work.reshape(&edged(&c.clip, edge, delta, ripple, rate))?;
    }
    if ripple && shift < 0 {
        work.shift(&tracks, place, shift, &skip)?;
    }
    Ok(())
}

pub(super) fn roll(
    work: &mut Work,
    left: &str,
    right: &str,
    delta: i64,
    linked: bool,
) -> Result<(), EditError> {
    let pairs = cuts(work, left, right, linked)?;
    let rate = work.rate();
    let mut limit = Limit::new();
    let (mut lefts, mut rights) = (vec![], vec![]);
    for (l, r) in &pairs {
        let (track, l) = work.clip(l)?;
        let r = track
            .clips
            .iter()
            .find(|c| c.id == *r)
            .expect("a cut is on one track");
        limit.at_least(least(track, &l.id, true, true) - l.length, || {
            EditError::EmptyRange
        });
        limit.at_most(r.length - least(track, &r.id, true, true), || {
            EditError::EmptyRange
        });
        for (clip, side) in [(l, &mut lefts), (r, &mut rights)] {
            let (before, after) = beyond(track, &clip.id);
            side.push(Changed {
                clip: clip.clone(),
                before,
                after,
            });
        }
    }
    let (delta, clamped) = reach(delta, &limit, |d| {
        all_fit(work, &lefts, |c| edged(c, Edge::Out, d, false, rate))?;
        all_fit(work, &rights, |c| edged(c, Edge::In, d, false, rate))
    })?;
    if clamped {
        work.clamp();
    }
    for (l, r) in lefts.iter().zip(&rights) {
        let l = edged(&l.clip, Edge::Out, delta, false, rate);
        let r = edged(&r.clip, Edge::In, delta, false, rate);
        // The one that gives frames goes first.
        if delta > 0 {
            work.reshape(&r)?;
            work.reshape(&l)?;
        } else {
            work.reshape(&l)?;
            work.reshape(&r)?;
        }
    }
    Ok(())
}

/// The cuts that a roll of the cut between `left` and `right` moves: that one, and with
/// `linked` each cut on a track that is not locked between a clip linked to `left` and one
/// linked to `right`.
fn cuts(work: &Work, left: &str, right: &str, linked: bool) -> Result<Vec<(Id, Id)>, EditError> {
    let (track, l) = work.clip(left)?;
    let r = track.clips.iter().find(|c| c.id == right);
    if r.is_none_or(|r| r.start != l.end()) {
        work.clip(right)?;
        return Err(EditError::Op(OpError::NotAdjacent {
            left: left.into(),
            right: right.into(),
        }));
    }
    work.open(&track.id)?;
    let mut pairs = vec![(Id::from(left), Id::from(right))];
    if !linked {
        return Ok(pairs);
    }
    let lefts = work.group(&[left.into()], true)?;
    let rights = work.group(&[right.into()], true)?;
    for l in &lefts[1..] {
        let (track, l) = work.clip(l)?;
        let meets = track
            .clips
            .iter()
            .find(|r| r.start == l.end() && rights[1..].contains(&r.id));
        if let Some(r) = meets {
            let pair = (l.id.clone(), r.id.clone());
            let used = pairs
                .iter()
                .any(|(a, b)| [a, b].contains(&&pair.0) || [a, b].contains(&&pair.1));
            if !used {
                pairs.push(pair);
            }
        }
    }
    Ok(pairs)
}

/// A clip that shows `delta` frames later of its source.
fn slipped(clip: &Clip, delta: i64, work: &Work) -> Clip {
    let mut new = clip.clone();
    if let Some(begins) = clip.source_in() {
        let ticks = ticks_of_scaled(delta, clip.speed, work.rate(), begins.scale);
        set_source_in(
            &mut new,
            Time {
                ticks: begins.ticks.saturating_add(ticks),
                ..begins
            },
        );
    }
    new
}

pub(super) fn slip(work: &mut Work, clip: &str, delta: i64, linked: bool) -> Result<(), EditError> {
    if work.clip(clip)?.1.source_in().is_none() {
        return Err(EditError::Op(OpError::Source { clip: clip.into() }));
    }
    let mut changed = vec![];
    for id in work.group(&[clip.into()], linked)? {
        let (track, c) = work.clip(&id)?;
        if c.source_in().is_some() {
            let (before, after) = beyond(track, &id);
            changed.push(Changed {
                clip: c.clone(),
                before,
                after,
            });
        }
    }
    let (delta, clamped) = reach(delta, &Limit::new(), |d| {
        all_fit(work, &changed, |c| slipped(c, d, work))
    })?;
    if clamped {
        work.clamp();
    }
    for c in &changed {
        let new = slipped(&c.clip, delta, work);
        work.reshape(&new)?;
    }
    Ok(())
}

/// A clip that slides, and its neighbours that meet it, which give and take its frames.
struct Slid {
    track: Id,
    clip: Clip,
    left: Option<Changed>,
    right: Option<Changed>,
}

pub(super) fn slide(
    work: &mut Work,
    clip: &str,
    delta: i64,
    linked: bool,
) -> Result<(), EditError> {
    let rate = work.rate();
    let mut limit = Limit::new();
    let mut slid = vec![];
    for id in work.group(&[clip.into()], linked)? {
        let (track, c) = work.clip(&id)?;
        let (before, after) = neighbours(track, c);
        let meeting = |at: i64, start: bool| {
            track
                .clips
                .iter()
                .find(|o| o.id != c.id && if start { o.end() == at } else { o.start == at })
        };
        let left = meeting(c.start, true);
        let right = meeting(c.end(), false);
        match left {
            Some(l) => limit.at_least(least(track, &l.id, true, true) - l.length, || {
                EditError::EmptyRange
            }),
            None => limit.at_least(before - c.start, no_room(track)),
        }
        match right {
            Some(r) => limit.at_most(r.length - least(track, &r.id, true, true), || {
                EditError::EmptyRange
            }),
            None => limit.at_most(
                after.unwrap_or(crate::ops::LARGEST) - c.end(),
                no_room(track),
            ),
        }
        let changed = |n: &Clip| {
            let (before, after) = beyond(track, &n.id);
            Changed {
                clip: n.clone(),
                before,
                after,
            }
        };
        slid.push(Slid {
            track: track.id.clone(),
            clip: c.clone(),
            left: left.map(changed),
            right: right.map(changed),
        });
    }
    let (delta, clamped) = reach(delta, &limit, |d| {
        for s in &slid {
            all_fit(work, s.left.as_slice(), |c| {
                edged(c, Edge::Out, d, false, rate)
            })?;
            all_fit(work, s.right.as_slice(), |c| {
                edged(c, Edge::In, d, false, rate)
            })?;
        }
        Ok(())
    })?;
    if clamped {
        work.clamp();
    }
    for s in slid {
        let left = s
            .left
            .map(|l| edged(&l.clip, Edge::Out, delta, false, rate));
        let right = s
            .right
            .map(|r| edged(&r.clip, Edge::In, delta, false, rate));
        let (first, last) = if delta > 0 {
            (right, left)
        } else {
            (left, right)
        };
        if let Some(first) = first {
            work.reshape(&first)?;
        }
        work.move_to(&s.clip.id, &s.track, s.clip.start + delta)?;
        if let Some(last) = last {
            work.reshape(&last)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::fixture::*;
    use super::super::{Edge, Edit, EditError};
    use crate::ops::Op;
    use crate::project::Alignment;
    use crate::time::Speed;

    fn trim(clip: &str, edge: Edge, delta: i64, ripple: bool) -> Edit {
        Edit::Trim {
            timeline: "t1".into(),
            clip: clip.into(),
            edge,
            delta,
            ripple,
            linked: true,
        }
    }

    fn roll(left: &str, right: &str, delta: i64) -> Edit {
        Edit::Roll {
            timeline: "t1".into(),
            left: left.into(),
            right: right.into(),
            delta,
            linked: true,
        }
    }

    fn slip(clip: &str, delta: i64) -> Edit {
        Edit::Slip {
            timeline: "t1".into(),
            clip: clip.into(),
            delta,
            linked: true,
        }
    }

    fn slide(clip: &str, delta: i64) -> Edit {
        Edit::Slide {
            timeline: "t1".into(),
            clip: clip.into(),
            delta,
            linked: true,
        }
    }

    #[test]
    fn a_trim_of_an_end_into_a_gap_moves_nothing_else() {
        let mut project = project();
        let expansion = edit(&mut project, trim("c3", Edge::Out, 20, false));
        assert!(!expansion.clamped);
        assert_eq!(clips(&project, "V1")[1], ("c3".into(), 100, 70, 150));
        assert_eq!(clips(&project, "A1")[1], ("c4".into(), 100, 70, 150));
        assert_eq!(clips(&project, "V2"), rows(&[("c5", 120, 50, 50)]));
    }

    #[test]
    fn a_trim_goes_no_further_than_the_end_of_the_media() {
        let mut project = project();
        // c3 shows [150, 200) of m1, which has 250 frames.
        let expansion = edit(&mut project, trim("c3", Edge::Out, 80, false));
        assert!(expansion.clamped);
        assert_eq!(clips(&project, "V1")[1], ("c3".into(), 100, 100, 150));
        let error = refused(project, trim("c3", Edge::Out, 1, false));
        assert!(matches!(error, EditError::NoHandles { .. }), "{error:?}");
    }

    #[test]
    fn a_trim_of_a_start_against_a_neighbour_is_refused_and_one_of_0_too() {
        let error = refused(project(), trim("c3", Edge::In, -5, false));
        assert_eq!(error, EditError::NoRoom { track: "V1".into() });
        assert_eq!(
            refused(project(), trim("c3", Edge::In, 0, false)),
            EditError::EmptyRange
        );
    }

    #[test]
    fn a_ripple_trim_of_a_start_keeps_the_start_and_pulls_what_follows_back() {
        let mut project = with(project(), vec![lock("S1")]);
        edit(&mut project, trim("c1", Edge::In, 10, true));
        assert_eq!(
            clips(&project, "V1"),
            rows(&[("c1", 0, 90, 10), ("c3", 90, 50, 150)])
        );
        assert_eq!(
            clips(&project, "A1"),
            rows(&[("c2", 0, 90, 10), ("c4", 90, 50, 150)])
        );
        assert_eq!(clips(&project, "V2"), rows(&[("c5", 110, 50, 50)]));
    }

    #[test]
    fn a_ripple_trim_that_takes_frames_away_from_under_another_track_is_refused() {
        // c3 ends at 150, inside c5 on V2.
        let error = refused(project(), trim("c3", Edge::Out, -10, true));
        assert_eq!(
            error,
            EditError::RippleBlocked {
                track: "V2".into(),
                clip: "c5".into()
            }
        );
    }

    #[test]
    fn a_ripple_trim_keeps_the_transition_at_its_edge() {
        let mut project = with(
            project(),
            vec![
                lock("V2"),
                lock("S1"),
                transition("V1", "c1", "c3", 10, Alignment::Center),
            ],
        );
        edit(&mut project, trim("c1", Edge::Out, -20, true));
        assert_eq!(
            clips(&project, "V1"),
            rows(&[("c1", 0, 80, 0), ("c3", 80, 50, 150)])
        );
        assert_eq!(
            transitions(&project, "V1"),
            [("c1".into(), "c3".into(), 10)]
        );
    }

    #[test]
    fn a_roll_moves_the_cut_and_the_linked_one() {
        let mut project = project();
        edit(&mut project, roll("c1", "c3", -30));
        assert_eq!(
            clips(&project, "V1"),
            rows(&[("c1", 0, 70, 0), ("c3", 70, 80, 120)])
        );
        assert_eq!(
            clips(&project, "A1"),
            rows(&[("c2", 0, 70, 0), ("c4", 70, 80, 120)])
        );
    }

    #[test]
    fn a_roll_goes_as_far_as_the_right_clip_has_frames() {
        let mut project = project();
        let expansion = edit(&mut project, roll("c1", "c3", 70));
        assert!(expansion.clamped);
        assert_eq!(
            clips(&project, "V1"),
            rows(&[("c1", 0, 149, 0), ("c3", 149, 1, 199)])
        );
    }

    #[test]
    fn a_roll_of_clips_that_do_not_meet_is_refused() {
        let error = refused(project(), roll("c1", "c5", 5));
        assert!(matches!(
            error,
            EditError::Op(crate::OpError::NotAdjacent { .. })
        ));
    }

    #[test]
    fn a_slip_changes_where_the_clip_begins_in_its_source_only() {
        let mut project = project();
        edit(&mut project, slip("c3", -100));
        assert_eq!(clips(&project, "V1")[1], ("c3".into(), 100, 50, 50));
        assert_eq!(clips(&project, "A1")[1], ("c4".into(), 100, 50, 50));
        let expansion = edit(&mut project, slip("c3", 500));
        assert!(expansion.clamped);
        assert_eq!(clips(&project, "V1")[1], ("c3".into(), 100, 50, 200));
    }

    #[test]
    fn a_slip_of_a_title_is_refused() {
        let mut project = project();
        edit(
            &mut project,
            Edit::Overwrite {
                timeline: "t1".into(),
                source: super::super::Material::Color {
                    color: crate::Rgba([0, 0, 0, 255]),
                    length: 10,
                },
                at: 200,
                video_track: None,
                audio_track: None,
            },
        );
        let error = refused(project, slip("c10", 5));
        assert_eq!(
            error,
            EditError::Op(crate::OpError::Source { clip: "c10".into() })
        );
    }

    #[test]
    fn a_slide_moves_the_clip_and_its_neighbours_give_and_take() {
        // c6 on V2 after c5, so that c5 has a neighbour after it.
        let mut project = with(
            project(),
            vec![Op::InsertClip {
                timeline: "t1".into(),
                track: "V2".into(),
                clip: clip("c8", 170, 30, 0, None),
            }],
        );
        edit(&mut project, slide("c5", 10));
        // Before c5 is a gap, which takes the 10 frames; c8 gives them.
        assert_eq!(
            clips(&project, "V2"),
            rows(&[("c5", 130, 50, 50), ("c8", 180, 20, 10)])
        );
    }

    #[test]
    fn a_last_clip_slides_and_changes_its_neighbour_without_changing_its_source() {
        for delta in [-20, 20] {
            let mut before = project();
            before.timelines[0]
                .tracks
                .retain(|r| r.id == "V1" || r.id == "A1");
            let mut history = crate::History::new(before.clone());
            history.edit(vec![slide("c3", delta)], "test").unwrap();
            let after = history.project().clone();
            assert_eq!(
                clips(&after, "V1"),
                rows(&[("c1", 0, 100 + delta, 0), ("c3", 100 + delta, 50, 150)])
            );
            assert_eq!(
                clips(&after, "A1"),
                rows(&[("c2", 0, 100 + delta, 0), ("c4", 100 + delta, 50, 150)])
            );
            assert_eq!(crate::resolve::duration(&after.timelines[0]), 150 + delta);
            history.undo("test").unwrap().unwrap();
            assert_eq!(history.project(), &before);
            history.redo("test").unwrap().unwrap();
            assert_eq!(history.project(), &after);
        }
    }

    #[test]
    fn a_slide_between_two_neighbours_keeps_their_outer_edges() {
        let mut project = with(
            project(),
            vec![
                lock("V2"),
                Op::InsertClip {
                    timeline: "t1".into(),
                    track: "V1".into(),
                    clip: clip("c8", 150, 50, 100, None),
                },
            ],
        );
        let expansion = edit(
            &mut project,
            Edit::Slide {
                timeline: "t1".into(),
                clip: "c3".into(),
                delta: -200,
                linked: false,
            },
        );
        // c1 can give 99 frames.
        assert!(expansion.clamped);
        assert_eq!(
            clips(&project, "V1"),
            rows(&[("c1", 0, 1, 0), ("c3", 1, 50, 150), ("c8", 51, 149, 1)])
        );
    }

    #[test]
    fn a_ripple_trim_stops_where_the_transition_uses_the_remaining_media_handles() {
        let mut project = with(
            project(),
            vec![
                Op::InsertClip {
                    timeline: "t1".into(),
                    track: "V1".into(),
                    clip: clip("c8", 150, 50, 100, None),
                },
                transition("V1", "c3", "c8", 20, Alignment::Start),
            ],
        );
        let done = edit(&mut project, trim("c3", Edge::Out, 80, true));
        assert!(done.clamped);
        assert_eq!(the(&project, "c3").length, 80);
        assert_eq!(the(&project, "c4").length, 80);
        assert_eq!(the(&project, "c8").start, 180);
        assert_eq!(
            transitions(&project, "V1"),
            [("c3".into(), "c8".into(), 20)]
        );
    }

    #[test]
    fn a_trim_of_a_clip_twice_as_fast_moves_its_source_twice_as_far() {
        let mut project = with(
            project(),
            vec![Op::SetClip {
                timeline: "t1".into(),
                id: "c5".into(),
                enabled: None,
                speed: Some(Speed { num: 2, den: 1 }),
                reverse: None,
                link: None,
                transform: None,
                gain_db: None,
                pan: None,
                fade_in: None,
                fade_out: None,
                source: None,
            }],
        );
        edit(&mut project, trim("c5", Edge::In, 10, false));
        assert_eq!(clips(&project, "V2"), rows(&[("c5", 130, 40, 70)]));
    }

    #[test]
    fn a_slip_keeps_the_frames_that_a_transition_shows_beyond_the_clip() {
        let mut project = with(
            project(),
            vec![transition("V1", "c1", "c3", 10, Alignment::Center)],
        );
        // c3 needs 5 frames before where it begins, and c4 slips as far as c3 does.
        let expansion = edit(&mut project, slip("c3", -200));
        assert!(expansion.clamped);
        assert_eq!(clips(&project, "V1")[1], ("c3".into(), 100, 50, 5));
        assert_eq!(clips(&project, "A1")[1], ("c4".into(), 100, 50, 5));
        assert_eq!(
            transitions(&project, "V1"),
            [("c1".into(), "c3".into(), 10)]
        );
    }

    #[test]
    fn a_slide_keeps_the_transitions_at_the_clip_that_slides() {
        let mut project = with(
            project(),
            vec![
                transition("V1", "c1", "c3", 10, Alignment::Center),
                Op::InsertClip {
                    timeline: "t1".into(),
                    track: "V1".into(),
                    clip: clip("c8", 150, 50, 100, None),
                },
            ],
        );
        edit(
            &mut project,
            Edit::Slide {
                timeline: "t1".into(),
                clip: "c3".into(),
                delta: -10,
                linked: false,
            },
        );
        assert_eq!(
            clips(&project, "V1"),
            rows(&[("c1", 0, 90, 0), ("c3", 90, 50, 150), ("c8", 140, 60, 90)])
        );
        assert_eq!(
            transitions(&project, "V1"),
            [("c1".into(), "c3".into(), 10)]
        );
    }

    fn trim_alone(clip: &str, edge: Edge, delta: i64, ripple: bool) -> Edit {
        Edit::Trim {
            timeline: "t1".into(),
            clip: clip.into(),
            edge,
            delta,
            ripple,
            linked: false,
        }
    }

    /// Put a clip on a track of t1.
    fn put(track: &str, clip: crate::project::Clip) -> Op {
        Op::InsertClip {
            timeline: "t1".into(),
            track: track.into(),
            clip,
        }
    }

    /// The fixture with c10 on V1 on [150, 200) from frame 20, right after c3, and transitions
    /// of 10 frames centred on the cuts of c3 that `at` names.
    fn between_two(at: &[(&str, &str)]) -> crate::Project {
        let mut ops = vec![put("V1", clip("", 150, 50, 20, None))];
        for &(left, right) in at {
            ops.push(transition("V1", left, right, 10, Alignment::Center));
        }
        with(project(), ops)
    }

    #[test]
    fn a_clip_is_trimmed_no_shorter_than_the_transitions_that_stay_at_it_cover() {
        // With a ripple both stay, and each covers 5 frames of c3.
        let both = [("c1", "c3"), ("c3", "c10")];
        let mut project = with(between_two(&both), vec![lock("V2"), lock("A1"), lock("S1")]);
        let expansion = edit(&mut project, trim_alone("c3", Edge::Out, -48, true));
        assert!(expansion.clamped);
        assert_eq!(
            clips(&project, "V1")[1..],
            rows(&[("c3", 100, 10, 150), ("c10", 110, 50, 20)])
        );
        // Without one, the transition at the end that moves is left behind, and the one at the
        // other end stays.
        let mut project = between_two(&[("c1", "c3")]);
        edit(&mut project, trim_alone("c3", Edge::Out, -48, false));
        assert_eq!(clips(&project, "V1")[1], ("c3".into(), 100, 5, 150));
        assert_eq!(
            transitions(&project, "V1"),
            [("c1".into(), "c3".into(), 10)]
        );
        let mut project = between_two(&[("c3", "c10")]);
        edit(&mut project, trim_alone("c3", Edge::In, 48, false));
        assert_eq!(clips(&project, "V1")[1], ("c3".into(), 145, 5, 195));
        assert_eq!(
            transitions(&project, "V1"),
            [("c3".into(), "c10".into(), 10)]
        );
    }

    #[test]
    fn a_start_is_trimmed_to_the_last_frame_of_a_clip_and_no_further() {
        let mut project = project();
        let expansion = edit(&mut project, trim_alone("c5", Edge::In, 60, false));
        assert!(expansion.clamped);
        assert_eq!(clips(&project, "V2"), rows(&[("c5", 169, 1, 99)]));
    }

    #[test]
    fn an_end_is_trimmed_up_to_the_next_clip_across_a_gap() {
        let mut project = with(project(), vec![put("V2", clip("", 190, 10, 0, None))]);
        let expansion = edit(&mut project, trim_alone("c5", Edge::Out, 30, false));
        assert!(expansion.clamped);
        assert_eq!(clips(&project, "V2")[0], ("c5".into(), 120, 70, 50));
    }

    #[test]
    fn the_transitions_at_a_trimmed_clip_are_put_back_beside_others_on_its_track() {
        let both = [("c1", "c3"), ("c3", "c10")];
        let mut project = between_two(&both);
        edit(&mut project, trim_alone("c10", Edge::Out, -10, false));
        assert_eq!(
            transitions(&project, "V1"),
            [
                ("c1".into(), "c3".into(), 10),
                ("c3".into(), "c10".into(), 10)
            ]
        );
    }

    /// The fixture with c4 cut short to [100, 130), and c10 on A1 on [135, 145) from frame 10
    /// in the group of c3, whose sound is so in two pieces. V2 and S1 are locked.
    fn sound_in_two() -> crate::Project {
        let short = Op::TrimClip {
            timeline: "t1".into(),
            id: "c4".into(),
            source_in: None,
            start: 100,
            length: 30,
        };
        let piece = put("A1", clip("", 135, 10, 10, Some("g2")));
        with(project(), vec![short, piece, lock("V2"), lock("S1")])
    }

    #[test]
    fn a_ripple_trim_goes_as_far_as_the_pieces_of_its_group_before_the_ripple_have_room() {
        let mut project = sound_in_two();
        assert!(edit(&mut project, trim("c3", Edge::Out, 10, true)).clamped);
        assert_eq!(clips(&project, "V1")[1], ("c3".into(), 100, 55, 150));
        assert_eq!(
            clips(&project, "A1")[1..],
            rows(&[("c4", 100, 35, 150), ("c10", 135, 15, 10)])
        );
        let mut project = sound_in_two();
        assert!(edit(&mut project, trim("c3", Edge::In, -10, true)).clamped);
        assert_eq!(clips(&project, "V1")[1], ("c3".into(), 100, 55, 145));
        assert_eq!(
            clips(&project, "A1")[1..],
            rows(&[("c4", 100, 35, 145), ("c10", 135, 15, 5)])
        );
    }

    #[test]
    fn a_trim_without_a_ripple_of_an_l_cut_is_not_held_up_by_what_follows_its_sound() {
        // c4 cut short to [100, 130), and c10 on A1 after it, before the end of c3.
        let short = Op::TrimClip {
            timeline: "t1".into(),
            id: "c4".into(),
            source_in: None,
            start: 100,
            length: 30,
        };
        let after = put("A1", clip("", 135, 10, 0, None));
        let mut project = with(project(), vec![short, after]);
        edit(&mut project, trim("c3", Edge::Out, -10, false));
        assert_eq!(clips(&project, "V1")[1], ("c3".into(), 100, 40, 150));
        assert_eq!(clips(&project, "A1")[1], ("c4".into(), 100, 20, 150));
    }

    /// Put a clip of t1 in a group, or in none.
    fn grouped(id: &str, group: Option<&str>) -> Op {
        Op::SetClip {
            timeline: "t1".into(),
            id: id.into(),
            enabled: None,
            speed: None,
            reverse: None,
            link: Some(group.map(Into::into)),
            transform: None,
            gain_db: None,
            pan: None,
            fade_in: None,
            fade_out: None,
            source: None,
        }
    }

    #[test]
    fn a_roll_moves_only_the_cuts_between_clips_of_the_two_groups() {
        // c4 is taken out of g2: the cut between c2 and c4 stays.
        let mut unlinked = with(project(), vec![grouped("c4", None)]);
        edit(&mut unlinked, roll("c1", "c3", 10));
        assert_eq!(clips(&unlinked, "V1")[1], ("c3".into(), 110, 40, 160));
        assert_eq!(
            clips(&unlinked, "A1"),
            rows(&[("c2", 0, 100, 0), ("c4", 100, 50, 150)])
        );
        // c3, and c10 after it, are put in g1 with c1: the cut after c3 is not rolled with the
        // cut before it.
        let after = put("V1", clip("", 150, 50, 0, Some("g1")));
        let mut regrouped = with(project(), vec![grouped("c3", Some("g1")), after]);
        edit(&mut regrouped, roll("c1", "c3", 10));
        assert_eq!(
            clips(&regrouped, "V1"),
            rows(&[("c1", 0, 110, 0), ("c3", 110, 40, 160), ("c10", 150, 50, 0)])
        );
    }

    #[test]
    fn a_slide_goes_as_far_as_its_neighbours_have_frames_and_room() {
        // Back: c1 gives 10 frames.
        let mut back = project();
        edit(&mut back, slide("c3", -10));
        assert_eq!(
            clips(&back, "V1"),
            rows(&[("c1", 0, 90, 0), ("c3", 90, 50, 150)])
        );
        // On: c3 can give all but a frame.
        let mut on = project();
        assert!(edit(&mut on, slide("c1", 60)).clamped);
        assert_eq!(
            clips(&on, "V1"),
            rows(&[("c1", 49, 100, 0), ("c3", 149, 1, 199)])
        );
        // On, up to the next clip across a gap.
        let mut gap = with(project(), vec![put("V2", clip("", 200, 10, 0, None))]);
        assert!(edit(&mut gap, slide("c5", 50)).clamped);
        assert_eq!(clips(&gap, "V2")[0], ("c5".into(), 150, 50, 50));
    }
}
