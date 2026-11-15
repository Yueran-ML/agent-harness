//! Transitions, speed, subtitles and tracks.

use std::collections::HashSet;

use super::work::{fit_fades, most, Work};
use super::{Created, CueItem, Edge, EditError};
use crate::audio::CrossfadeLevel;
use crate::ops::{Op, OpError};
use crate::project::{Alignment, Clip, Cue, Id, Track, TrackKind, Transition, TransitionKind};
use crate::span::{frames_for_span, source_span};
use crate::time::{Rate, Speed};

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
            fade_in_level_db: CrossfadeLevel::default(),
            fade_out_level_db: CrossfadeLevel::default(),
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

/// The cuts of the clips of sound that are linked to `left` and `right`, where two of them meet
/// on a track of sound that is not locked and has no transition at their cut: the track and the
/// two clips of each; and whether a cut was passed over because a clip fades there, as a
/// transition does not go where a fade is (`fade_at_transition`).
fn linked_cuts(work: &Work, left: &Clip, right: &Clip) -> (Vec<(Id, Id, Id)>, bool) {
    let (Some(l), Some(r)) = (&left.link, &right.link) else {
        return (vec![], false);
    };
    let mut cuts = vec![];
    let mut faded = false;
    for track in &work.timeline().tracks {
        if track.kind != TrackKind::Audio || track.locked {
            continue;
        }
        for a in track.clips.iter().filter(|c| c.link.as_ref() == Some(l)) {
            let Some(b) = track
                .clips
                .iter()
                .find(|c| c.link.as_ref() == Some(r) && c.start == a.end())
            else {
                continue;
            };
            let taken = track
                .transitions
                .iter()
                .any(|x| x.left == a.id || x.right == b.id);
            let sounds =
                |fade: &Option<crate::audio::AudioFade>| fade.is_some_and(|f| f.length.ticks != 0);
            let fading = a.fade_out != 0
                || sounds(&a.audio_fade_out)
                || b.fade_in != 0
                || sounds(&b.audio_fade_in);
            if fading && !taken {
                faded = true;
            } else if !taken {
                cuts.push((track.id.clone(), a.id.clone(), b.id.clone()));
            }
        }
    }
    (cuts, faded)
}

/// Add a crossfade of `length` and `align` at the cut of `left` and `right` on track `track`, or
/// one as long as fits where that is too long, saying that it was clamped; none where none fits.
fn add_crossfade(
    work: &mut Work,
    (track, left, right): (Id, Id, Id),
    length: i64,
    align: Alignment,
) -> Result<(), EditError> {
    let at = work.track(&track)?;
    let (Some(l), Some(r)) = (
        at.clips.iter().find(|c| c.id == left),
        at.clips.iter().find(|c| c.id == right),
    ) else {
        return Ok(());
    };
    let length = match check_fits(work, at, (l, r), length, align, None) {
        Ok(()) => length,
        Err(EditError::NoHandles {
            longest: Some(longest),
            ..
        }) => {
            work.clamp();
            longest
        }
        Err(error) => return Err(error),
    };
    if length < 1 {
        return Ok(());
    }
    let timeline = work.timeline_id();
    work.op(Op::AddTransition {
        timeline,
        track,
        transition: Transition {
            id: String::new(),
            kind: TransitionKind::Crossfade,
            left,
            right,
            length,
            align,
            fade_in_level_db: CrossfadeLevel::default(),
            fade_out_level_db: CrossfadeLevel::default(),
        },
    })?;
    Ok(())
}

pub(super) fn add_transition(
    work: &mut Work,
    left: &str,
    right: &str,
    kind: Option<TransitionKind>,
    length: i64,
    align: Alignment,
    linked: bool,
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
    // A transition of picture brings crossfades at the cuts of the sound linked to its clips,
    // where they meet as the picture does (p. 973).
    let (cuts, faded) = if linked && track.kind == TrackKind::Video {
        linked_cuts(work, l, r)
    } else {
        (vec![], false)
    };

    let track = track.id.clone();
    let timeline = work.timeline_id();
    let done = work.op(Op::AddTransition {
        timeline,
        track,
        transition: Transition {
            id: String::new(),
            kind,
            left: left.into(),
            right: right.into(),
            length,
            align,
            fade_in_level_db: CrossfadeLevel::default(),
            fade_out_level_db: CrossfadeLevel::default(),
        },
    })?;
    if let Op::AddTransition { transition, .. } = done {
        work.created(Created::Transition(transition.id));
    }
    if faded {
        work.clamp();
    }
    for cut in cuts {
        add_crossfade(work, cut, length, align)?;
    }
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
    (kind, length, align): (Option<TransitionKind>, Option<i64>, Option<Alignment>),
    (fade_in_level_db, fade_out_level_db): (Option<CrossfadeLevel>, Option<CrossfadeLevel>),
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
        fade_in_level_db,
        fade_out_level_db,
    })?;
    Ok(())
}

/// As many frames at the new speed as show the same source span, the last before its end.
fn length_at_speed(clip: &Clip, speed: Speed, rate: Rate) -> i64 {
    if clip.speed == speed {
        return clip.length;
    }
    let scale = clip.source_in().map_or(1, |t| t.scale);
    let span = source_span(clip, rate, scale);
    let frames = |span| frames_for_span(span, speed, rate, scale);
    frames(span).min(frames(span - 1) + 1).max(1)
}

/// Where the last of a group of clips ends at a speed.
fn ends_at(group: &[(Id, Clip)], speed: Speed, rate: Rate) -> i64 {
    group
        .iter()
        .map(|(_, c)| c.start + length_at_speed(c, speed, rate))
        .max()
        .unwrap_or(0)
}

#[allow(
    clippy::too_many_lines,
    reason = "a step for each part of a change of speed"
)]
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
    let mut group = vec![];
    for id in work.group(&[clip.into()], linked)? {
        let (track, c) = work.owned(&id)?;
        if c.source_in().is_some() {
            // The speed points of a clip say how it plays: it is given no one speed beside
            // them, nor is a clip linked to it while it keeps them.
            if c.retime.is_some() {
                return Err(EditError::Op(OpError::OutOfRange { what: "retimed" }));
            }
            group.push((track, c));
        }
    }
    // The audio and picture of a split edit can end at different frames. Everything after
    // the group moves by the change in its overall end, whichever member was selected.
    let place = group
        .iter()
        .map(|(_, c)| c.end())
        .max()
        .expect("the selected clip has media");
    if ripple && ends_at(&group, speed, rate) != place {
        work.split_ripple(&group, place)?;
    }
    let mut changed = vec![];
    let mut clamped = false;
    for (track, c) in group {
        let track = work.track(&track)?;
        let mut length = length_at_speed(&c, speed, rate);
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
        fit_fades(&mut new, Edge::Out, rate);
        changed.push((c, new));
    }
    let shift = changed
        .iter()
        .map(|(_, c)| c.end())
        .max()
        .expect("the selected clip has media")
        - place;
    changed.retain(|(old, new)| old != new);
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
            crop: None,
            composite: None,
            retime_process: None,
            gain_db: None,
            pan: None,
            fade_in: None,
            fade_out: None,
            audio_fade_in: None,
            audio_fade_out: None,
            eq: None,
            effects: None,
            pitch_correction: None,
            pitch: None,
            channels: None,
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
    let done = work.op(Op::AddCue {
        timeline: work.timeline_id(),
        track: track.into(),
        cue: Cue {
            id: String::new(),
            start,
            length,
            text,
        },
    })?;
    if let Op::AddCue { cue, .. } = done {
        work.created(Created::Cue(cue.id));
    }
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
/// letter. A track of subtitles is added disabled while another of the timeline is enabled.
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
    // A timeline shows one track of subtitles at a time: a new one is hidden while another is
    // shown.
    track.enabled = !(kind == TrackKind::Subtitle
        && tracks
            .iter()
            .any(|r| r.kind == TrackKind::Subtitle && r.enabled));
    if let Some(name) = name {
        track.name = name;
    }
    if let Some(language) = language {
        track.language = language;
    }
    // Its sound goes to the first bus of the timeline where there is one (p. 3721).
    track.audio.strip = crate::audio::Strip::to_first_bus_if_any();
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
    // It leaves the groups of tracks that it is in, as DaVinci takes a deleted track out of
    // them; a sidechain or a Ducker that it drives still stands in the way.
    let groups: Vec<(Id, Vec<Id>)> = work
        .timeline()
        .track_groups
        .iter()
        .filter(|g| g.members.contains(&track.id))
        .map(|g| {
            let members = g.members.iter().filter(|m| **m != track.id).cloned();
            (g.id.clone(), members.collect())
        })
        .collect();
    for (id, members) in groups {
        work.op(Op::SetTrackGroup {
            timeline: timeline.clone(),
            id,
            patch: serde_json::json!({ "members": members }),
        })?;
    }
    // The clips go one by one, each with the transitions at it.
    for c in &track.clips {
        work.remove_alone(&c.id)?;
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
    use crate::ops::{Op, OpError};
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
            linked: false,
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
    fn a_transition_does_not_silently_replace_an_existing_fade() {
        for (id, fade_in, fade_out, edge) in [("c1", 10, 20, "out"), ("c3", 10, 0, "in")] {
            let project = with(
                project(),
                vec![super::super::work::set_fades("t1", id, fade_in, fade_out)],
            );
            let error = refused(project, add("c1", "c3", 10, Alignment::Start));
            assert_eq!(
                error,
                EditError::Op(crate::OpError::FadeAtTransition {
                    clip: id.into(),
                    edge
                })
            );
        }
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
            fade_in_level_db: None,
            fade_out_level_db: None,
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

    fn split_edit() -> crate::Project {
        let mut project = project();
        let tracks = &mut project.timelines[0].tracks;
        tracks[0].clips = vec![
            clip("c1", 20, 80, 20, Some("g1")),
            clip("c3", 150, 50, 150, Some("g2")),
        ];
        tracks[1].clips[0].start = 150;
        tracks[2].clips = vec![
            clip("c2", 10, 120, 10, Some("g1")),
            clip("c4", 150, 50, 150, Some("g2")),
        ];
        tracks[4].cues[1].start = 150;
        project.validate().unwrap();
        project
    }

    #[test]
    fn a_split_edits_speed_ripples_from_the_whole_groups_end() {
        for (num, den, video_length, audio_length, shift) in
            [(2, 1, 40, 60, -60), (1, 2, 160, 240, 120)]
        {
            for selected in ["c1", "c2"] {
                let before = split_edit();
                let mut history = crate::History::new(before.clone());
                let done = history
                    .edit(vec![speed(selected, num, den, true)], "test")
                    .unwrap();
                assert_eq!(done.clamped, [] as [usize; 0]);
                let after = history.project().clone();
                assert_eq!(
                    clips(&after, "V1"),
                    rows(&[("c1", 20, video_length, 20), ("c3", 150 + shift, 50, 150),])
                );
                assert_eq!(
                    clips(&after, "A1"),
                    rows(&[("c2", 10, audio_length, 10), ("c4", 150 + shift, 50, 150),])
                );
                assert_eq!(the(&after, "c5").start, 150 + shift);
                assert_eq!(cues(&after, "S1")[1], ("q7".into(), 150 + shift, 30));
                assert_eq!(the(&after, "c1").link, Some("g1".into()));
                assert_eq!(the(&after, "c2").link, Some("g1".into()));
                history.undo("test").unwrap().unwrap();
                assert_eq!(history.project(), &before);
                history.redo("test").unwrap().unwrap();
                assert_eq!(history.project(), &after);
            }
        }
    }

    #[test]
    fn a_split_edits_speed_keeps_locked_linked_partners_and_tracks_in_place() {
        let mut project = with(split_edit(), vec![lock("A1"), lock("V2")]);
        let audio = project.timelines[0].tracks[2].clone();
        edit(&mut project, speed("c1", 2, 1, true));
        assert_eq!(project.timelines[0].tracks[2], audio);
        assert_eq!(the(&project, "c5").start, 150);
        assert_eq!(the(&project, "c3").start, 110);
    }

    proptest::proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(1000))]

        #[test]
        fn a_split_edits_ripple_moves_later_content_by_exactly_the_change_in_its_end(
            video_start in 0i64..30, audio_start in 0i64..30,
            video_half in 20i64..90, audio_half in 20i64..90,
            gap in 0i64..40, faster in proptest::bool::ANY, select_audio in proptest::bool::ANY,
        ) {
            let mut project = split_edit();
            let (video_length, audio_length) = (video_half * 2, audio_half * 2);
            let end = (video_start + video_length).max(audio_start + audio_length);
            let after = end + gap;
            let tracks = &mut project.timelines[0].tracks;
            tracks[0].clips[0] = clip("c1", video_start, video_length, 0, Some("g1"));
            tracks[2].clips[0] = clip("c2", audio_start, audio_length, 0, Some("g1"));
            tracks[0].clips[1].start = after;
            tracks[1].clips[0].start = after;
            tracks[2].clips[1].start = after;
            tracks[4].cues[1].start = after;
            project.validate().unwrap();
            let (num, den, v, a) = if faster {
                (2, 1, video_half, audio_half)
            } else {
                (1, 2, video_length * 2, audio_length * 2)
            };
            let delta = (video_start + v).max(audio_start + a) - end;
            let selected = if select_audio { "c2" } else { "c1" };
            edit(&mut project, speed(selected, num, den, true));
            proptest::prop_assert_eq!(the(&project, "c1").length, v);
            proptest::prop_assert_eq!(the(&project, "c2").length, a);
            for id in ["c3", "c4", "c5"] {
                proptest::prop_assert_eq!(the(&project, id).start, after + delta);
            }
            proptest::prop_assert_eq!(cues(&project, "S1").last().unwrap().1, after + delta);
        }
    }

    #[test]
    fn a_split_edits_speed_does_not_close_over_unselected_clips() {
        let mut project = split_edit();
        project.timelines[0].tracks[1].clips[0].start = 100;
        let error = refused(project, speed("c1", 2, 1, true));
        assert_eq!(
            error,
            EditError::RippleBlocked {
                track: "V2".into(),
                clip: "c5".into()
            }
        );
    }

    /// V1 c1 [0, 100) g1, c3 [100, 150) g2; A1 c2 [0, 130) g1, c4 [130, 180) g2: the sound of
    /// g1 runs on under the picture of g2.
    fn l_cut() -> crate::Project {
        let mut project = project();
        let tracks = &mut project.timelines[0].tracks;
        tracks[0].clips = vec![
            clip("c1", 0, 100, 0, Some("g1")),
            clip("c3", 100, 50, 150, Some("g2")),
        ];
        tracks[1].clips.clear();
        tracks[2].clips = vec![
            clip("c2", 0, 130, 0, Some("g1")),
            clip("c4", 130, 50, 180, Some("g2")),
        ];
        tracks[4].cues.clear();
        project.validate().unwrap();
        project
    }

    #[test]
    fn a_ripple_of_an_l_cuts_speed_is_refused_either_way_rather_than_parting_the_next_group() {
        for (num, den) in [(1, 2), (2, 1)] {
            for selected in ["c1", "c2"] {
                let error = refused(l_cut(), speed(selected, num, den, true));
                assert_eq!(
                    error,
                    EditError::RippleBlocked {
                        track: "V1".into(),
                        clip: "c3".into()
                    },
                    "{num}/{den} of {selected}"
                );
            }
        }
        // Shortening the sound by a frame would move c4 and leave c3 just the same.
        let trim = Edit::Trim {
            timeline: "t1".into(),
            clip: "c2".into(),
            edge: super::super::Edge::Out,
            delta: -1,
            ripple: true,
            linked: true,
        };
        assert_eq!(
            refused(l_cut(), trim),
            EditError::RippleBlocked {
                track: "V1".into(),
                clip: "c3".into()
            }
        );
        // Without a ripple, the picture goes only as far as the next picture.
        let mut project = l_cut();
        assert!(edit(&mut project, speed("c1", 1, 2, false)).clamped);
        assert_eq!(
            clips(&project, "V1"),
            rows(&[("c1", 0, 100, 0), ("c3", 100, 50, 150)])
        );
        assert_eq!(clips(&project, "A1")[1], ("c4".into(), 130, 50, 180));
    }

    #[test]
    fn a_ripple_that_lengthens_cuts_the_subtitle_across_its_place_as_an_insert_does() {
        let at_cue = |start: i64, length: i64| {
            with(
                project(),
                vec![Op::SetCue {
                    timeline: "t1".into(),
                    id: "q7".into(),
                    start: Some(start),
                    length: Some(length),
                    text: None,
                }],
            )
        };
        let trim = Edit::Trim {
            timeline: "t1".into(),
            clip: "c1".into(),
            edge: super::super::Edge::Out,
            delta: 10,
            ripple: true,
            linked: true,
        };
        let overwrite = Edit::RippleOverwrite {
            timeline: "t1".into(),
            source: m1(Some(0), Some(80)),
            at: 120,
            video_track: None,
            audio_track: None,
        };
        // q7 lies across the place of each ripple: 100 for the trim and the speed of c1, and
        // the end of c3 (150) for the ripple overwrite, which makes c3's 50 frames 80.
        for (edit, (start, length), first, second) in [
            (trim, (90, 30), (90, 10), (110, 20)),
            (speed("c1", 1, 2, true), (90, 30), (90, 10), (200, 20)),
            (overwrite, (140, 20), (140, 10), (180, 10)),
        ] {
            let before = at_cue(start, length);
            let mut history = crate::History::new(before.clone());
            history.edit(vec![edit.clone()], "test").unwrap();
            let after = history.project();
            let parts: Vec<(i64, i64)> = cues(after, "S1")[1..]
                .iter()
                .map(|(_, start, length)| (*start, *length))
                .collect();
            assert_eq!(parts, [first, second], "{edit:?}");
            assert_eq!(cues(after, "S1")[1].0, "q7");
            let texts: Vec<&str> = after.timelines[0].tracks[4].cues[1..]
                .iter()
                .map(|q| q.text.as_str())
                .collect();
            assert_eq!(texts, ["world", "world"]);
            history.undo("test").unwrap().unwrap();
            assert_eq!(
                history.project(),
                &crate::Project {
                    next_id: history.project().next_id,
                    ..before
                }
            );
        }
    }

    #[test]
    fn setting_the_same_speed_leaves_no_history_entry_or_transition_operations() {
        let before = with(
            project(),
            vec![transition("V1", "c1", "c3", 10, Alignment::Start)],
        );
        let mut history = crate::History::new(before.clone());
        assert_eq!(
            history.edit(vec![speed("c1", 1, 1, true)], "test"),
            Err(crate::TransactionError::Empty)
        );
        assert_eq!(history.project(), &before);
        assert_eq!(history.revision(), 0);
        assert!(!history.can_undo());
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
                fade_in_level_db: None,
                fade_out_level_db: None,
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
        // S1 is shown, so S2 is added hidden: a timeline shows one track of subtitles at a time.
        assert!(!added.enabled);
        let mut alone = with(
            super::super::fixture::project(),
            vec![Op::SetTrack {
                timeline: "t1".into(),
                id: "S1".into(),
                name: None,
                color: None,
                enabled: Some(false),
                locked: None,
                language: None,
                mute: None,
                solo: None,
                solo_safe: None,
            }],
        );
        let subtitles = Edit::AddTrack {
            timeline: "t1".into(),
            kind: TrackKind::Subtitle,
            index: None,
            name: None,
            language: None,
        };
        edit(&mut alone, subtitles.clone());
        assert!(alone.timelines[0].tracks.last().unwrap().enabled);
        edit(&mut alone, subtitles);
        let shown: Vec<_> = alone.timelines[0]
            .tracks
            .iter()
            .filter(|r| r.kind == TrackKind::Subtitle)
            .map(|r| (r.id.as_str(), r.enabled))
            .collect();
        assert_eq!(shown, [("S1", false), ("S2", true), ("S3", false)]);
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

    /// V2 of the fixture with c10 on [170, 180) from frame 100 and c11 on [180, 230) from 120,
    /// right after c5, and a transition of 8 frames between c5 and c10 (`first`) or between c10
    /// and c11.
    fn three_on_v2(first: bool) -> crate::Project {
        let put = |start, length, from| Op::InsertClip {
            timeline: "t1".into(),
            track: "V2".into(),
            clip: clip("", start, length, from, None),
        };
        let between = if first {
            transition("V2", "c5", "c10", 8, Alignment::Center)
        } else {
            transition("V2", "c10", "c11", 8, Alignment::Center)
        };
        with(
            project(),
            vec![put(170, 10, 100), put(180, 50, 120), between],
        )
    }

    #[test]
    fn a_transition_is_refused_where_with_the_one_at_the_other_end_it_would_cover_too_much() {
        // c10 lasts 10 frames, and the transition already at it covers 4 of them: one of 14
        // centred on its other end would cover 7 more. One of 13 covers 6 of the clip after its
        // cut and 7 of the clip before it.
        let too_long = |longest| EditError::NoHandles {
            clip: "c10".into(),
            need: 7,
            have: 6,
            longest: Some(longest),
        };
        let error = refused(three_on_v2(true), add("c10", "c11", 14, Alignment::Center));
        assert_eq!(error, too_long(13));
        let error = refused(three_on_v2(false), add("c5", "c10", 14, Alignment::Center));
        assert_eq!(error, too_long(12));
    }

    #[test]
    fn a_transition_lasts_a_frame_at_least() {
        let mut project = project();
        edit(&mut project, add("c1", "c3", 1, Alignment::Center));
        for length in [0, -1] {
            let error = refused(project.clone(), add("c2", "c4", length, Alignment::Center));
            assert_eq!(error, EditError::Op(OpError::OutOfRange { what: "length" }));
        }
        let set = |length| Edit::SetTransition {
            timeline: "t1".into(),
            id: "x10".into(),
            kind: None,
            length: Some(length),
            align: None,
            fade_in_level_db: None,
            fade_out_level_db: None,
        };
        edit(&mut project.clone(), set(1));
        for length in [0, -1] {
            let error = refused(project.clone(), set(length));
            assert_eq!(error, EditError::Op(OpError::OutOfRange { what: "length" }));
        }
    }

    #[test]
    fn a_speed_out_of_range_is_refused_before_the_clip_is_looked_for() {
        for (num, den) in [(1, 101), (101, 1), (2, 201), (201, 2)] {
            let error = refused(project(), speed("c99", num, den, false));
            assert_eq!(error, EditError::Op(OpError::OutOfRange { what: "speed" }));
        }
        for (num, den) in [(1, 100), (100, 1), (2, 200), (200, 2)] {
            edit(&mut project(), speed("c5", num, den, false));
        }
    }

    #[test]
    fn a_speed_that_leaves_the_end_where_it_is_ripples_nothing() {
        // c1 and c2 played backward end where they did, so c3, between the end of c1 and that of
        // c2, is not in the way.
        let mut project = l_cut();
        let backward = Edit::SetSpeed {
            timeline: "t1".into(),
            clip: "c1".into(),
            speed: Speed::ONE,
            reverse: Some(true),
            ripple: true,
            linked: true,
        };
        edit(&mut project, backward);
        assert!(the(&project, "c1").reverse && the(&project, "c2").reverse);
        assert_eq!(clips(&project, "V1")[1], ("c3".into(), 100, 50, 150));
    }

    #[test]
    fn a_clip_slowed_up_to_the_next_is_not_clamped_and_one_slowed_past_it_is() {
        // c1 trimmed to [0, 50) at half the speed lasts 100 frames, up to c3.
        let trimmed = with(
            project(),
            vec![Op::TrimClip {
                timeline: "t1".into(),
                id: "c1".into(),
                source_in: None,
                start: 0,
                length: 50,
            }],
        );
        let half = Edit::SetSpeed {
            timeline: "t1".into(),
            clip: "c1".into(),
            speed: Speed { num: 1, den: 2 },
            reverse: None,
            ripple: false,
            linked: false,
        };
        let mut project = trimmed;
        assert!(!edit(&mut project, half).clamped);
        assert_eq!(the(&project, "c1").length, 100);
        // c3 at half the speed would last 100 frames, and a clip at 170 stops it at 70.
        let mut project = with(
            super::super::fixture::project(),
            vec![Op::InsertClip {
                timeline: "t1".into(),
                track: "V1".into(),
                clip: clip("", 170, 10, 0, None),
            }],
        );
        let half = Edit::SetSpeed {
            timeline: "t1".into(),
            clip: "c3".into(),
            speed: Speed { num: 1, den: 2 },
            reverse: None,
            ripple: false,
            linked: false,
        };
        assert!(edit(&mut project, half).clamped);
        assert_eq!(the(&project, "c3").length, 70);
    }

    #[test]
    fn a_track_added_at_the_place_after_the_last_of_its_kind_goes_above_them() {
        let mut project = project();
        edit(
            &mut project,
            Edit::AddTrack {
                timeline: "t1".into(),
                kind: TrackKind::Video,
                index: Some(2),
                name: None,
                language: None,
            },
        );
        let ids: Vec<&str> = project.timelines[0]
            .tracks
            .iter()
            .map(|r| r.id.as_str())
            .collect();
        assert_eq!(ids, ["V1", "V2", "V3", "A1", "A2", "S1"]);
        let error = refused(
            project,
            Edit::AddTrack {
                timeline: "t1".into(),
                kind: TrackKind::Video,
                index: Some(4),
                name: None,
                language: None,
            },
        );
        assert_eq!(error, EditError::Op(OpError::OutOfList { at: 4, len: 3 }));
    }

    #[test]
    fn a_clip_with_speed_points_is_given_no_one_speed_beside_them() {
        let retimed = EditError::Op(OpError::OutOfRange { what: "retimed" });
        // c1, on V1, which c2 on A1 is linked to, slows to half its speed halfway through.
        let mut project = project();
        retime(&mut project, "c1", &[(0, 1, 1), (50, 1, 2)], 100);
        assert_eq!(refused(project.clone(), speed("c1", 2, 1, false)), retimed);
        // Nor is the clip linked to it, while the two go together.
        assert_eq!(refused(project.clone(), speed("c2", 2, 1, false)), retimed);
        let alone = Edit::SetSpeed {
            timeline: "t1".into(),
            clip: "c2".into(),
            speed: Speed { num: 2, den: 1 },
            reverse: None,
            ripple: false,
            linked: false,
        };
        edit(&mut project, alone);
        assert_eq!(the(&project, "c2").speed, Speed { num: 2, den: 1 });
        assert_eq!(
            the(&project, "c1").retime,
            Some(speed_points(&[(0, 1, 1), (50, 1, 2)]))
        );
        // Put back to one speed, it is given another.
        let plain = Op::SetRetime {
            timeline: "t1".into(),
            id: "c1".into(),
            retime: None,
            length: None,
        };
        let mut project = with(project, vec![plain]);
        edit(&mut project, speed("c1", 1, 2, false));
        assert_eq!(the(&project, "c1").speed, Speed { num: 1, den: 2 });
    }
}
