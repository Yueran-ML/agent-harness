//! Subtitles and tracks.

use super::work::Work;
use super::{CueItem, EditError};
use crate::ops::{Op, OpError};
use crate::project::{Cue, Id, Track, TrackKind};

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
    use crate::project::TrackKind;

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
