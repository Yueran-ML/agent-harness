//! Transitions, speed, subtitles and tracks.

use std::collections::HashSet;

use super::work::{fit_fades, most, set_fades, Work};
use super::{CueItem, Edge, EditError};
use crate::ops::{Op, OpError};
use crate::project::{Alignment, Clip, Cue, Id, Track, TrackKind, Transition, TransitionKind};
use crate::span::{frames_for_span, source_span};
use crate::time::Speed;

/// Check that a transition of `length` and `align` fits at the cut between `left` and `right` of
/// a track, beside the other transitions at them, but for `replacing`: each clip has the frames
/// of its media that it shows beyond the cut, and has room for what it covers. Refused with
/// `no_handles` for the clip that has too little, with the longest that would fit.
pub(super) fn check_fits(
    work: &Work,
    track: &Track,
    (left, right): (&Clip, &Clip),
    length: i64,
    align: Alignment,
    replacing: Option<&str>,
) -> Result<(), EditError> {
    let mut left_covered = 0;
    let mut left_before = 0;
    let mut right_covered = 0;
    let mut right_after = 0;
    for other in track
        .transitions
        .iter()
        .filter(|x| Some(x.id.as_str()) != replacing)
    {
        if other.right == left.id {
            (left_covered, left_before) = other.handles();
        }
        if other.left == right.id {
            (right_after, right_covered) = other.handles();
        }
    }
    let handles = |length| {
        Transition {
            id: String::new(),
            kind: TransitionKind::Dissolve,
            left: String::new(),
            right: String::new(),
            length,
            align,
        }
        .handles()
    };
    // What each side lacks at a length, if anything: the clip, the frames it needs, and those
    // it has.
    let lacks = |length| {
        let (after_left, before_right) = handles(length);
        let sides = [
            (
                left,
                left_before,
                after_left,
                left_covered,
                before_right,
                true,
            ),
            (
                right,
                before_right,
                right_after,
                right_covered,
                after_left,
                false,
            ),
        ];
        for (clip, before, after, other, cover, is_left) in sides {
            let source = |frames| {
                if is_left {
                    work.fits(clip, before, frames)
                } else {
                    work.fits(clip, frames, after)
                }
            };
            let need = if is_left { after } else { before };
            if !source(need) {
                return Some((clip.id.clone(), need, most(need, source)));
            }
            if other + cover > clip.length {
                return Some((clip.id.clone(), cover, clip.length - other));
            }
        }
        None
    };
    match lacks(length) {
        None => Ok(()),
        Some((clip, need, have)) => Err(EditError::NoHandles {
            clip,
            need,
            have,
            longest: Some(most(length, |l| lacks(l).is_none())),
        }),
    }
}

pub(super) fn add_transition(
    work: &mut Work,
    left: &str,
    right: &str,
    kind: Option<TransitionKind>,
    length: i64,
    align: Alignment,
) -> Result<(), EditError> {
    let (track, l) = work.clip(left)?;
    let r = track.clips.iter().find(|c| c.id == right);
    let Some(r) = r.filter(|r| r.start == l.end()) else {
        work.clip(right)?;
        return Err(EditError::Op(OpError::NotAdjacent {
            left: left.into(),
            right: right.into(),
        }));
    };
    let track = work.open(&track.id)?;
    if length < 1 {
        return Err(EditError::Op(OpError::OutOfRange { what: "length" }));
    }
    check_fits(work, track, (l, r), length, align, None)?;
    let kind = kind.unwrap_or(match track.kind {
        TrackKind::Audio => TransitionKind::Crossfade,
        _ => TransitionKind::Dissolve,
    });
    let (track, l, r) = (track.id.clone(), l.clone(), r.clone());
    // The transition does what a fade at the cut would.
    let timeline = work.timeline_id();
    if l.fade_out != 0 {
        work.op(set_fades(&timeline, left, l.fade_in, 0))?;
    }
    if r.fade_in != 0 {
        work.op(set_fades(&timeline, right, 0, r.fade_out))?;
    }
    work.op(Op::AddTransition {
        timeline,
        track,
        transition: Transition {
            id: String::new(),
            kind,
            left: left.into(),
            right: right.into(),
            length,
            align,
        },
    })?;
    Ok(())
}

/// A transition of the timeline, and the track that it is on.
fn transition<'w>(work: &'w Work, id: &str) -> Result<(&'w Track, &'w Transition), EditError> {
    work.timeline()
        .tracks
        .iter()
        .find_map(|r| r.transitions.iter().find(|x| x.id == id).map(|x| (r, x)))
        .ok_or_else(|| super::work::missing("transition", id))
}

pub(super) fn remove_transition(work: &mut Work, id: &str) -> Result<(), EditError> {
    let (track, _) = transition(work, id)?;
    work.open(&track.id)?;
    work.op(Op::RemoveTransition {
        timeline: work.timeline_id(),
        id: id.into(),
    })?;
    Ok(())
}

pub(super) fn set_transition(
    work: &mut Work,
    id: &str,
    kind: Option<TransitionKind>,
    length: Option<i64>,
    align: Option<Alignment>,
) -> Result<(), EditError> {
    let (track, x) = transition(work, id)?;
    let track = work.open(&track.id)?;
    if length.is_some_and(|length| length < 1) {
        return Err(EditError::Op(OpError::OutOfRange { what: "length" }));
    }
    if length.is_some() || align.is_some() {
        let clip = |id: &str| track.clips.iter().find(|c| c.id == id);
        if let (Some(l), Some(r)) = (clip(&x.left), clip(&x.right)) {
            let length = length.unwrap_or(x.length);
            check_fits(
                work,
                track,
                (l, r),
                length,
                align.unwrap_or(x.align),
                Some(id),
            )?;
        }
    }
    work.op(Op::SetTransition {
        timeline: work.timeline_id(),
        id: id.into(),
        kind,
        length,
        align,
    })?;
    Ok(())
}

pub(super) fn set_speed(
    work: &mut Work,
    clip: &str,
    speed: Speed,
    reverse: Option<bool>,
    ripple: bool,
    linked: bool,
) -> Result<(), EditError> {
    let (num, den) = (u64::from(speed.num), u64::from(speed.den));
    if num * 100 < den || num > den * 100 {
        return Err(EditError::Op(OpError::OutOfRange { what: "speed" }));
    }
    let (_, given) = work.owned(clip)?;
    if given.source_in().is_none() {
        return Err(EditError::Op(OpError::Source { clip: clip.into() }));
    }
    let rate = work.rate();
    // As many frames at the new speed as show the same stretch of the source, the last of them
    // before its end.
    let length_at = |c: &Clip| {
        let scale = c.source_in().map_or(1, |t| t.scale);
        let span = source_span(c, rate, scale);
        let frames = |span| frames_for_span(span, speed, rate, scale);
        frames(span).min(frames(span - 1) + 1).max(1)
    };
    let place = given.end();
    let shift = length_at(&given) - given.length;
    let mut changed = vec![];
    let mut clamped = false;
    for id in work.group(&[clip.into()], linked)? {
        let (track, c) = work.clip(&id)?;
        if c.source_in().is_none() {
            continue;
        }
        let mut length = length_at(c);
        let after = track
            .clips
            .iter()
            .filter(|o| o.id != c.id && o.start >= c.end())
            .map(|o| o.start)
            .min();
        // What stays where it is bounds the clip.
        if let Some(after) = after.filter(|&after| !ripple || after < place) {
            if c.start + length > after {
                length = after - c.start;
                clamped = true;
            }
        }
        let mut new = Clip {
            speed,
            reverse: reverse.unwrap_or(c.reverse),
            length,
            ..c.clone()
        };
        fit_fades(&mut new, Edge::Out);
        changed.push((c.clone(), new));
    }
    if clamped {
        work.clamp();
    }
    let skip: HashSet<Id> = changed.iter().map(|(c, _)| c.id.clone()).collect();
    let tracks = work.all_unlocked();
    for (old, _) in &changed {
        work.detach(&old.id)?;
    }
    if ripple && shift > 0 {
        work.shift(&tracks, place, shift, &skip)?;
    }
    let timeline = work.timeline_id();
    for (old, new) in changed {
        let set = Op::SetClip {
            timeline: timeline.clone(),
            id: new.id.clone(),
            enabled: None,
            speed: Some(new.speed),
            reverse: Some(new.reverse),
            link: None,
            transform: None,
            gain_db: None,
            pan: None,
            fade_in: None,
            fade_out: None,
            source: None,
        };
        // A clip that gets shorter is shortened at its old speed, and one that gets longer is
        // lengthened at its new one, so that it never shows more than its media has.
        if new.length < old.length {
            work.reshape(&new)?;
            work.op(set)?;
        } else {
            work.op(set)?;
            work.reshape(&new)?;
        }
    }
    if ripple && shift < 0 {
        work.shift(&tracks, place, shift, &skip)?;
    }
    Ok(())
}

/// A track of subtitles that the edit may change.
fn subtitles<'w>(work: &'w Work, track: &str) -> Result<&'w Track, EditError> {
    let track = work.open(track)?;
    if track.kind != TrackKind::Subtitle {
        return Err(EditError::Op(OpError::WrongTrack {
            what: "cue",
            kind: track.kind,
        }));
    }
    Ok(track)
}

/// The track of a subtitle of the timeline, which the edit may change.
fn track_of_cue(work: &Work, id: &str) -> Result<Id, EditError> {
    let track = work
        .timeline()
        .tracks
        .iter()
        .find(|r| r.cues.iter().any(|q| q.id == id))
        .ok_or_else(|| super::work::missing("cue", id))?;
    Ok(work.open(&track.id)?.id.clone())
}

pub(super) fn add_cue(
    work: &mut Work,
    track: &str,
    start: i64,
    length: i64,
    text: String,
) -> Result<(), EditError> {
    subtitles(work, track)?;
    work.op(Op::AddCue {
        timeline: work.timeline_id(),
        track: track.into(),
        cue: Cue {
            id: String::new(),
            start,
            length,
            text,
        },
    })?;
    Ok(())
}

pub(super) fn set_cue(
    work: &mut Work,
    id: &str,
    start: Option<i64>,
    length: Option<i64>,
    text: Option<String>,
) -> Result<(), EditError> {
    track_of_cue(work, id)?;
    work.op(Op::SetCue {
        timeline: work.timeline_id(),
        id: id.into(),
        start,
        length,
        text,
    })?;
    Ok(())
}

pub(super) fn remove_cue(work: &mut Work, id: &str) -> Result<(), EditError> {
    track_of_cue(work, id)?;
    work.op(Op::RemoveCue {
        timeline: work.timeline_id(),
        id: id.into(),
    })?;
    Ok(())
}

pub(super) fn cues_from(
    work: &mut Work,
    track: &str,
    mut items: Vec<CueItem>,
) -> Result<(), EditError> {
    let old: Vec<Id> = subtitles(work, track)?
        .cues
        .iter()
        .map(|q| q.id.clone())
        .collect();
    let timeline = work.timeline_id();
    for id in old {
        work.op(Op::RemoveCue {
            timeline: timeline.clone(),
            id,
        })?;
    }
    items.sort_by_key(|item| item.start);
    for item in items {
        work.op(Op::AddCue {
            timeline: timeline.clone(),
            track: track.into(),
            cue: Cue {
                id: String::new(),
                start: item.start,
                length: item.length,
                text: item.text,
            },
        })?;
    }
    Ok(())
}

/// Add a track of `kind` at `index` among those of its kind, or above them, named `name` or by
/// its id, and give its id: its letter and one above the highest number of the ids of that
/// letter.
pub(super) fn new_track(
    work: &mut Work,
    kind: TrackKind,
    index: Option<usize>,
    name: Option<String>,
    language: Option<String>,
) -> Result<Id, EditError> {
    let tracks = &work.timeline().tracks;
    let places: Vec<usize> = (0..tracks.len())
        .filter(|&at| tracks[at].kind == kind)
        .collect();
    let place = match index {
        Some(index) if index < places.len() => places[index],
        Some(index) if index > places.len() => {
            return Err(EditError::Op(OpError::OutOfList {
                at: index,
                len: places.len(),
            }))
        }
        _ => places.last().map_or(tracks.len(), |&at| at + 1),
    };
    let letter = match kind {
        TrackKind::Video => 'V',
        TrackKind::Audio => 'A',
        TrackKind::Subtitle => 'S',
    };
    let highest = tracks
        .iter()
        .filter_map(|r| r.id.strip_prefix(letter)?.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    let id = format!("{letter}{}", highest + 1);
    let mut track = Track::new(id.clone(), kind);
    if let Some(name) = name {
        track.name = name;
    }
    if let Some(language) = language {
        track.language = language;
    }
    work.op(Op::AddTrack {
        timeline: work.timeline_id(),
        track,
        index: Some(place),
    })?;
    Ok(id)
}

pub(super) fn add_track(
    work: &mut Work,
    kind: TrackKind,
    index: Option<usize>,
    name: Option<String>,
    language: Option<String>,
) -> Result<(), EditError> {
    new_track(work, kind, index, name, language).map(drop)
}

pub(super) fn remove_track(work: &mut Work, track: &str) -> Result<(), EditError> {
    let track = work.open(track)?.clone();
    let timeline = work.timeline_id();
    // The clips go one by one, each with the transitions at it.
    for c in &track.clips {
        work.remove(&c.id)?;
    }
    for q in &track.cues {
        work.op(Op::RemoveCue {
            timeline: timeline.clone(),
            id: q.id.clone(),
        })?;
    }
    work.op(Op::RemoveTrack {
        timeline,
        id: track.id,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::fixture::*;
    use super::super::{CueItem, Edit, EditError};
    use crate::ops::Op;
    use crate::project::{Alignment, TrackKind, TransitionKind};
    use crate::time::Speed;

    fn add(left: &str, right: &str, length: i64, align: Alignment) -> Edit {
        Edit::AddTransition {
            timeline: "t1".into(),
            left: left.into(),
            right: right.into(),
            kind: None,
            length,
            align,
        }
    }

    fn speed(clip: &str, num: u32, den: u32, ripple: bool) -> Edit {
        Edit::SetSpeed {
            timeline: "t1".into(),
            clip: clip.into(),
            speed: Speed { num, den },
            reverse: None,
            ripple,
            linked: true,
        }
    }

    #[test]
    fn a_transition_is_added_of_the_kind_of_its_track() {
        let mut project = project();
        edit(&mut project, add("c1", "c3", 10, Alignment::Center));
        edit(&mut project, add("c2", "c4", 6, Alignment::End));
        let track = |id: &str| {
            project.timelines[0]
                .tracks
                .iter()
                .find(|r| r.id == id)
                .unwrap()
                .transitions[0]
                .clone()
        };
        assert_eq!(track("V1").kind, TransitionKind::Dissolve);
        assert_eq!(track("A1").kind, TransitionKind::Crossfade);
        assert_eq!(track("A1").align, Alignment::End);
    }

    #[test]
    fn a_transition_with_too_few_frames_of_media_is_refused_with_the_longest_that_fits() {
        let project = with(
            project(),
            vec![Op::TrimClip {
                timeline: "t1".into(),
                id: "c3".into(),
                source_in: Some(at(4)),
                start: 100,
                length: 50,
            }],
        );
        // c3 begins at frame 4 of m1: a transition centred on the cut needs 5 before it.
        let error = refused(project, add("c1", "c3", 10, Alignment::Center));
        assert_eq!(
            error,
            EditError::NoHandles {
                clip: "c3".into(),
                need: 5,
                have: 4,
                longest: Some(9)
            }
        );
    }

    #[test]
    fn a_transition_takes_the_place_of_the_fades_at_its_cut() {
        let mut project = with(
            project(),
            vec![super::super::work::set_fades("t1", "c1", 10, 20)],
        );
        edit(&mut project, add("c1", "c3", 10, Alignment::Start));
        assert_eq!(
            (the(&project, "c1").fade_in, the(&project, "c1").fade_out),
            (10, 0)
        );
    }

    #[test]
    fn a_transition_is_set_and_taken_away_and_one_too_long_is_refused() {
        let mut project = with(
            project(),
            vec![transition("V1", "c1", "c3", 10, Alignment::Center)],
        );
        let id = project.timelines[0].tracks[0].transitions[0].id.clone();
        let set = |length| Edit::SetTransition {
            timeline: "t1".into(),
            id: id.clone(),
            kind: Some(TransitionKind::DipBlack),
            length: Some(length),
            align: None,
        };
        edit(&mut project, set(20));
        assert_eq!(
            transitions(&project, "V1"),
            [("c1".into(), "c3".into(), 20)]
        );
        // c3 lasts 50 frames, and a transition of 120 would cover 60 of them.
        let error = refused(project.clone(), set(120));
        assert!(
            matches!(error, EditError::NoHandles { ref clip, longest: Some(100), .. } if clip == "c3"),
            "{error:?}"
        );
        edit(
            &mut project,
            Edit::RemoveTransition {
                timeline: "t1".into(),
                id,
            },
        );
        assert_eq!(transitions(&project, "V1"), []);
    }

    #[test]
    fn a_clip_twice_as_fast_lasts_half_as_long_and_a_ripple_pulls_what_follows() {
        let mut project = with(project(), vec![lock("V2"), lock("S1")]);
        edit(&mut project, speed("c1", 2, 1, true));
        assert_eq!(
            clips(&project, "V1"),
            rows(&[("c1", 0, 50, 0), ("c3", 50, 50, 150)])
        );
        assert_eq!(the(&project, "c2").speed, Speed { num: 2, den: 1 });
        assert_eq!(clips(&project, "A1")[1], ("c4".into(), 50, 50, 150));
    }

    #[test]
    fn a_clip_slower_without_a_ripple_goes_as_far_as_its_neighbour() {
        let mut project = project();
        let expansion = edit(&mut project, speed("c3", 1, 2, false));
        // Twice as long would be 100 frames: there is nothing after c3.
        assert!(!expansion.clamped);
        assert_eq!(clips(&project, "V1")[1], ("c3".into(), 100, 100, 150));
        let expansion = edit(&mut project, speed("c1", 1, 2, false));
        assert!(expansion.clamped);
        assert_eq!(clips(&project, "V1")[0], ("c1".into(), 0, 100, 0));
        assert_eq!(the(&project, "c1").speed, Speed { num: 1, den: 2 });
    }

    #[test]
    fn a_speed_out_of_range_is_refused() {
        let error = refused(project(), speed("c1", 1, 101, false));
        assert_eq!(
            error,
            EditError::Op(crate::OpError::OutOfRange { what: "speed" })
        );
    }

    #[test]
    fn reversing_a_clip_without_transition_handles_removes_the_transition() {
        let mut project = with(
            project(),
            vec![transition("V1", "c1", "c3", 10, Alignment::Start)],
        );
        let done = edit(
            &mut project,
            Edit::SetSpeed {
                timeline: "t1".into(),
                clip: "c1".into(),
                speed: Speed::ONE,
                reverse: Some(true),
                ripple: false,
                linked: false,
            },
        );
        assert!(done.clamped);
        assert_eq!(transitions(&project, "V1"), []);
        assert!(the(&project, "c1").reverse);
    }

    #[test]
    fn subtitles_are_added_set_taken_away_and_put_in_whole() {
        let mut project = project();
        edit(
            &mut project,
            Edit::AddCue {
                timeline: "t1".into(),
                track: "S1".into(),
                start: 50,
                length: 10,
                text: "again".into(),
            },
        );
        edit(
            &mut project,
            Edit::SetCue {
                timeline: "t1".into(),
                id: "q10".into(),
                start: Some(60),
                length: None,
                text: None,
            },
        );
        edit(
            &mut project,
            Edit::RemoveCue {
                timeline: "t1".into(),
                id: "q6".into(),
            },
        );
        assert_eq!(
            cues(&project, "S1"),
            [("q10".into(), 60, 10), ("q7".into(), 110, 30)]
        );
        let item = |start, text: &str| CueItem {
            start,
            length: 5,
            text: text.into(),
        };
        edit(
            &mut project,
            Edit::CuesFrom {
                timeline: "t1".into(),
                track: "S1".into(),
                items: vec![item(20, "two"), item(0, "one")],
            },
        );
        assert_eq!(
            cues(&project, "S1"),
            [("q11".into(), 0, 5), ("q12".into(), 20, 5)]
        );
    }

    #[test]
    fn a_subtitle_on_a_locked_track_is_not_changed() {
        let project = with(project(), vec![lock("S1")]);
        let error = refused(
            project,
            Edit::RemoveCue {
                timeline: "t1".into(),
                id: "q6".into(),
            },
        );
        assert_eq!(error, EditError::Locked { track: "S1".into() });
    }

    #[test]
    fn a_track_is_added_among_those_of_its_kind_and_taken_away_with_what_is_on_it() {
        let mut project = project();
        edit(
            &mut project,
            Edit::AddTrack {
                timeline: "t1".into(),
                kind: TrackKind::Video,
                index: Some(1),
                name: None,
                language: None,
            },
        );
        let ids = |p: &crate::Project| {
            p.timelines[0]
                .tracks
                .iter()
                .map(|r| r.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&project), ["V1", "V3", "V2", "A1", "A2", "S1"]);
        let expansion = edit(
            &mut project,
            Edit::RemoveTrack {
                timeline: "t1".into(),
                track: "V1".into(),
            },
        );
        assert_eq!(ids(&project), ["V3", "V2", "A1", "A2", "S1"]);
        // c1 and c3 went first, one by one.
        assert_eq!(expansion.done.len(), 3);
    }

    #[test]
    fn a_transition_on_a_locked_track_is_not_changed_and_one_that_is_not_there_is_refused() {
        let project = with(
            project(),
            vec![
                transition("V1", "c1", "c3", 10, Alignment::Center),
                lock("V1"),
            ],
        );
        let error = refused(
            project.clone(),
            Edit::RemoveTransition {
                timeline: "t1".into(),
                id: "x10".into(),
            },
        );
        assert_eq!(error, EditError::Locked { track: "V1".into() });
        let error = refused(
            project,
            Edit::SetTransition {
                timeline: "t1".into(),
                id: "x99".into(),
                kind: None,
                length: Some(4),
                align: None,
            },
        );
        assert_eq!(
            error,
            EditError::Op(crate::OpError::Missing {
                what: "transition",
                id: "x99".into()
            })
        );
    }

    #[test]
    fn subtitles_put_in_whole_that_overlap_are_refused() {
        let item = |start| CueItem {
            start,
            length: 10,
            text: "line".into(),
        };
        let edit_ = Edit::CuesFrom {
            timeline: "t1".into(),
            track: "S1".into(),
            items: vec![item(0), item(5)],
        };
        assert!(matches!(
            refused(project(), edit_),
            EditError::Op(crate::OpError::CueOverlap { .. })
        ));
    }

    #[test]
    fn a_track_of_subtitles_is_added_with_its_name_and_language_above_the_others() {
        let mut project = project();
        edit(
            &mut project,
            Edit::AddTrack {
                timeline: "t1".into(),
                kind: TrackKind::Subtitle,
                index: None,
                name: Some("English".into()),
                language: Some("en".into()),
            },
        );
        let added = project.timelines[0].tracks.last().unwrap();
        assert_eq!(
            (
                added.id.as_str(),
                added.name.as_str(),
                added.language.as_str()
            ),
            ("S2", "English", "en")
        );
        let error = refused(
            project,
            Edit::AddTrack {
                timeline: "t1".into(),
                kind: TrackKind::Audio,
                index: Some(3),
                name: None,
                language: None,
            },
        );
        assert_eq!(
            error,
            EditError::Op(crate::OpError::OutOfList { at: 3, len: 2 })
        );
    }

    #[test]
    fn a_locked_track_is_not_taken_away() {
        let error = refused(
            with(project(), vec![lock("A1")]),
            Edit::RemoveTrack {
                timeline: "t1".into(),
                track: "A1".into(),
            },
        );
        assert_eq!(error, EditError::Locked { track: "A1".into() });
    }
}
