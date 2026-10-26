//! The operations on a project, each of which gives back the operation that takes it back.
//!
//! An operation is applied whole or not at all: one that is refused leaves the project exactly as
//! it was, `next_id` too. One that is applied gives back its inverse, which carries every id that
//! the engine gave and the place that each thing was taken from, so that applying the inverse of
//! the inverse makes the same things again, with the same ids, in the same places.
//!
//! An operation is written in JSON as an object that names it in `op`, such as
//! `{"op": "remove_clip", "timeline": "t1", "id": "c1"}`, and is read and written so by the
//! engine and by its clients in Python and JavaScript. A field that is not known is refused, and
//! an `index` that is left out is read as none.
//!
//! # Ids
//!
//! A thing that is added with an empty id is given one by the engine.
//!
//! - Media, a timeline and a clip are given a letter for their kind (`m`, `t` or `c`) and the
//!   number in `next_id`, which then goes up by one. A number whose id is held already by a thing
//!   of the same kind is passed over, as it may be in a project that was written elsewhere.
//! - A track is given `V` if it holds picture and `A` if it holds sound, and a number one above
//!   the highest number of the ids of that letter among the tracks of its timeline, whatever
//!   they hold: beside `V1` and `V3`, a new track of picture is `V4`, and in a timeline with no
//!   such id it is `V1`. Tracks do not use `next_id`, so a track that is taken away and added
//!   again without an id may be given the id that it had.
//! - The tracks and clips inside a timeline or a track that is added are given theirs after it,
//!   in the order of their lists as they are given: the first track, then its clips, then the
//!   next track. The clips of each track are then put in the order of their start.
//!
//! A thing that is added with an id keeps it, unless a thing of its kind holds it already: media
//! and timelines among those of the project, a track among those of its timeline, and a clip
//! among all the clips of the project. Then the operation is refused. An id that the engine could
//! have made itself, one of the letters `m`, `t` and `c` and a number written without a leading
//! 0, moves `next_id` past that number, so that the engine never makes that id for another thing.
//!
//! `next_id` never goes back: not when an operation is refused, and not when one is taken back.
//!
//! # What is checked
//!
//! A clip names media that is in the project and has a stream for the kind of its track. It lasts
//! at least one frame, from a frame of the timeline no earlier than 0, and on its track it covers
//! no frame that another clip covers. What it shows of its media lies inside the media: from
//! `source_in`, for the ticks that its frames last at the rate of the timeline, and no further
//! than the duration of the media. `source_in` counts ticks of the scale of that duration, which
//! is never 0. A clip moves only between tracks of the same kind, within its timeline. A thing is
//! not put past the end of its list, and a timeline that is added has no 0 in its rate.
//!
//! Every integer that an operation writes into a project is within [`LARGEST`] of 0, since
//! JavaScript reads the project too. Media that clips use cannot be taken away; a track or a
//! timeline can, with everything on it, and its inverse adds all of it back.

use std::collections::HashSet;

use serde::{Deserialize, Serialize, Serializer};

use crate::project::{Clip, Id, Media, Project, Timeline, Track, TrackKind};
use crate::time::{Rate, Time};

/// The largest integer that JavaScript holds exactly, 2^53 - 1. No integer that an operation
/// writes into a project is further than this from 0.
pub const LARGEST: i64 = (1 << 53) - 1;

/// [`LARGEST`] as a count.
const LARGEST_COUNT: u64 = LARGEST.unsigned_abs();

/// The letters of the ids that the engine makes with `next_id`.
const NUMBERED: [char; 3] = ['m', 't', 'c'];

/// A change to a project.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Op {
    /// Add media to the project, at `index` in its list, or at the end if there is none.
    AddMedia {
        /// The media, with an empty id for the engine to give it one.
        media: Media,
        /// Where it goes in the list of media.
        #[serde(default)]
        index: Option<usize>,
    },
    /// Take media that no clip uses away from the project.
    RemoveMedia {
        /// The media.
        id: Id,
    },
    /// Add a timeline to the project, with its tracks and their clips.
    AddTimeline {
        /// The timeline. It and the things in it may have empty ids.
        timeline: Timeline,
        /// Where it goes in the list of timelines.
        #[serde(default)]
        index: Option<usize>,
    },
    /// Take a timeline away, with everything on it.
    RemoveTimeline {
        /// The timeline.
        id: Id,
    },
    /// Add a track to a timeline, with its clips.
    AddTrack {
        /// The timeline.
        timeline: Id,
        /// The track. It and its clips may have empty ids.
        track: Track,
        /// Where it goes in the list of tracks of the timeline.
        #[serde(default)]
        index: Option<usize>,
    },
    /// Take a track away from a timeline, with its clips.
    RemoveTrack {
        /// The timeline.
        timeline: Id,
        /// The track.
        id: Id,
    },
    /// Put a clip on a track, in the order of the starts of the clips there.
    InsertClip {
        /// The timeline.
        timeline: Id,
        /// The track.
        track: Id,
        /// The clip, with an empty id for the engine to give it one.
        clip: Clip,
    },
    /// Take a clip away from a timeline.
    RemoveClip {
        /// The timeline.
        timeline: Id,
        /// The clip.
        id: Id,
    },
    /// Move a clip to another frame, on its track or on another track of the same kind.
    MoveClip {
        /// The timeline of the clip.
        timeline: Id,
        /// The clip.
        id: Id,
        /// The track that it goes to.
        track: Id,
        /// The frame that it starts at there.
        start: i64,
    },
    /// Set where a clip begins in its media, where it starts on its track, and how long it lasts.
    TrimClip {
        /// The timeline of the clip.
        timeline: Id,
        /// The clip.
        id: Id,
        /// Where it begins in its media.
        source_in: Time,
        /// The frame that it starts at.
        start: i64,
        /// How many frames it lasts.
        length: i64,
    },
}

/// Why an operation is refused.
///
/// It is written in JSON as an object that names the reason in `reason`, gives the message in
/// `message`, and has the fields of the reason beside them:
/// `{"reason": "overlap", "with": "c1", "message": "the clip covers frames that clip c1 covers"}`.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum OpError {
    /// A thing that the operation names is not there. `what` is `media`, `timeline`, `track` or
    /// `clip`.
    #[error("there is no {what} {id}")]
    Missing {
        /// The kind of the thing.
        what: &'static str,
        /// Its id.
        id: Id,
    },
    /// The id of a thing that is added is held by another thing of its kind.
    #[error("the id {id} is held by another {what}")]
    Taken {
        /// The kind of the thing.
        what: &'static str,
        /// The id.
        id: Id,
    },
    /// A thing is to be put at a place past the end of its list.
    #[error("there is no place {at} in a list of {len}")]
    OutOfList {
        /// The place.
        at: usize,
        /// How long the list is.
        len: usize,
    },
    /// A clip is cut from media without the stream that its track shows.
    #[error("media {media} has no {} stream, which a {} track needs", name_of(*.kind), name_of(*.kind))]
    NoStream {
        /// The media.
        media: Id,
        /// What the track holds.
        kind: TrackKind,
    },
    /// A clip is to move to a track of the other kind.
    #[error("track {track} holds {}, and a clip moves only between tracks of one kind", name_of(*.kind))]
    WrongKind {
        /// The track.
        track: Id,
        /// What it holds.
        kind: TrackKind,
    },
    /// A clip lasts no frame, or shows what is not in its media.
    #[error("the clip lasts no frame, or shows what is not in media {media}")]
    OutOfSource {
        /// The media.
        media: Id,
    },
    /// A time counts ticks of a scale of 0, or not of the scale of its media.
    #[error("{}", scale_message(*.scale, *.media_scale))]
    Scale {
        /// The scale of the time.
        scale: u32,
        /// The scale of the duration of the media.
        media_scale: u32,
    },
    /// A clip covers a frame that another clip on its track covers.
    #[error("the clip covers frames that clip {with} covers")]
    Overlap {
        /// The other clip.
        with: Id,
    },
    /// A clip starts before the first frame of its timeline.
    #[error("the clip starts at frame {start}, before the timeline does")]
    OutOfTimeline {
        /// The frame.
        start: i64,
    },
    /// Media that a clip uses is to be taken away.
    #[error("media {id} is used by clip {by}")]
    InUse {
        /// The media.
        id: Id,
        /// A clip that uses it.
        by: Id,
    },
    /// An integer to be written into the project is further than [`LARGEST`] from 0, and
    /// JavaScript would not read it exactly.
    #[error("the {what} is further than 2^53 - 1 from 0")]
    TooLarge {
        /// What the integer is.
        what: &'static str,
    },
    /// A rate of frames has a 0 in it.
    #[error("a rate of frames has a 0 in it")]
    ZeroRate,
}

impl Serialize for OpError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut value = Fields::serialize(self, serde_json::value::Serializer)
            .map_err(serde::ser::Error::custom)?;
        if let Some(object) = value.as_object_mut() {
            object.insert("message".into(), self.to_string().into());
        }
        value.serialize(serializer)
    }
}

/// [`OpError`] as it is written, without its message: an object with the reason in `reason`
/// and the fields beside it. The compiler holds it to the variants and fields of `OpError`.
#[derive(Serialize)]
#[serde(remote = "OpError", tag = "reason", rename_all = "snake_case")]
enum Fields {
    Missing { what: &'static str, id: Id },
    Taken { what: &'static str, id: Id },
    OutOfList { at: usize, len: usize },
    NoStream { media: Id, kind: TrackKind },
    WrongKind { track: Id, kind: TrackKind },
    OutOfSource { media: Id },
    Scale { scale: u32, media_scale: u32 },
    Overlap { with: Id },
    OutOfTimeline { start: i64 },
    InUse { id: Id, by: Id },
    TooLarge { what: &'static str },
    ZeroRate,
}

/// What a track of a kind is called in a message.
fn name_of(kind: TrackKind) -> &'static str {
    match kind {
        TrackKind::Video => "video",
        TrackKind::Audio => "audio",
    }
}

/// The message of [`OpError::Scale`].
fn scale_message(scale: u32, media_scale: u32) -> String {
    if scale == 0 {
        "a time cannot count ticks of 1/0 of a second".into()
    } else {
        format!(
            "the time counts ticks of 1/{scale} of a second, and its media counts ticks of \
             1/{media_scale}"
        )
    }
}

/// Apply an operation to a project, and give back the operation that takes it back.
///
/// # Errors
/// The operation is refused, and the project is left exactly as it was.
pub fn apply(project: &mut Project, op: Op) -> Result<Op, OpError> {
    match op {
        Op::AddMedia { media, index } => add_media(project, media, index),
        Op::RemoveMedia { id } => remove_media(project, &id),
        Op::AddTimeline { timeline, index } => add_timeline(project, timeline, index),
        Op::RemoveTimeline { id } => remove_timeline(project, &id),
        Op::AddTrack {
            timeline,
            track,
            index,
        } => add_track(project, &timeline, track, index),
        Op::RemoveTrack { timeline, id } => remove_track(project, &timeline, &id),
        Op::InsertClip {
            timeline,
            track,
            clip,
        } => insert_clip(project, &timeline, &track, clip),
        Op::RemoveClip { timeline, id } => remove_clip(project, &timeline, &id),
        Op::MoveClip {
            timeline,
            id,
            track,
            start,
        } => move_clip(project, &timeline, &id, &track, start),
        Op::TrimClip {
            timeline,
            id,
            source_in,
            start,
            length,
        } => trim_clip(project, &timeline, &id, source_in, start, length),
    }
}

fn add_media(project: &mut Project, mut media: Media, index: Option<usize>) -> Result<Op, OpError> {
    let at = place(index, project.media.len())?;
    within("size", media.identity.size.into())?;
    within("modified_us", media.identity.modified_us.into())?;
    within("duration", media.facts.duration.ticks.into())?;
    if media.facts.duration.scale == 0 {
        return Err(OpError::Scale {
            scale: 0,
            media_scale: 0,
        });
    }
    let held = |id: &str| project.media.iter().any(|m| m.id == id);
    let mut counter = Counter::of(project);
    if media.id.is_empty() {
        media.id = counter.make('m', held)?;
    } else if held(&media.id) {
        return Err(taken("media", &media.id));
    } else {
        counter.saw(&media.id);
    }
    let id = media.id.clone();
    project.media.insert(at, media);
    project.next_id = counter.next;
    Ok(Op::RemoveMedia { id })
}

fn remove_media(project: &mut Project, id: &str) -> Result<Op, OpError> {
    let at = project
        .media
        .iter()
        .position(|m| m.id == id)
        .ok_or_else(|| missing("media", id))?;
    if let Some(clip) = clips_of(project).find(|c| c.media == id) {
        return Err(OpError::InUse {
            id: id.into(),
            by: clip.id.clone(),
        });
    }
    let media = project.media.remove(at);
    Ok(Op::AddMedia {
        media,
        index: Some(at),
    })
}

fn add_timeline(
    project: &mut Project,
    mut timeline: Timeline,
    index: Option<usize>,
) -> Result<Op, OpError> {
    let at = place(index, project.timelines.len())?;
    check_rate(timeline.rate)?;
    let held = |id: &str| project.timelines.iter().any(|t| t.id == id);
    let mut counter = Counter::of(project);
    if !timeline.id.is_empty() {
        if held(&timeline.id) {
            return Err(taken("timeline", &timeline.id));
        }
        counter.saw(&timeline.id);
    }
    let mut clip_ids = claim(project, &mut counter, &[], &timeline.tracks)?;
    if timeline.id.is_empty() {
        timeline.id = counter.make('t', held)?;
    }
    name(&mut counter, &mut clip_ids, &[], &mut timeline.tracks)?;
    check_tracks(project, timeline.rate, &mut timeline.tracks)?;
    let id = timeline.id.clone();
    project.timelines.insert(at, timeline);
    project.next_id = counter.next;
    Ok(Op::RemoveTimeline { id })
}

fn remove_timeline(project: &mut Project, id: &str) -> Result<Op, OpError> {
    let at = timeline_at(project, id)?;
    let timeline = project.timelines.remove(at);
    Ok(Op::AddTimeline {
        timeline,
        index: Some(at),
    })
}

fn add_track(
    project: &mut Project,
    timeline_id: &str,
    track: Track,
    index: Option<usize>,
) -> Result<Op, OpError> {
    let t = timeline_at(project, timeline_id)?;
    let timeline = &project.timelines[t];
    let at = place(index, timeline.tracks.len())?;
    let mut counter = Counter::of(project);
    let mut added = [track];
    let mut clip_ids = claim(project, &mut counter, &timeline.tracks, &added)?;
    name(&mut counter, &mut clip_ids, &timeline.tracks, &mut added)?;
    check_tracks(project, timeline.rate, &mut added)?;
    let [track] = added;
    let id = track.id.clone();
    project.timelines[t].tracks.insert(at, track);
    project.next_id = counter.next;
    Ok(Op::RemoveTrack {
        timeline: timeline_id.into(),
        id,
    })
}

fn remove_track(project: &mut Project, timeline_id: &str, id: &str) -> Result<Op, OpError> {
    let t = timeline_at(project, timeline_id)?;
    let tracks = &mut project.timelines[t].tracks;
    let at = tracks
        .iter()
        .position(|r| r.id == id)
        .ok_or_else(|| missing("track", id))?;
    let track = tracks.remove(at);
    Ok(Op::AddTrack {
        timeline: timeline_id.into(),
        track,
        index: Some(at),
    })
}

fn insert_clip(
    project: &mut Project,
    timeline_id: &str,
    track_id: &str,
    mut clip: Clip,
) -> Result<Op, OpError> {
    let t = timeline_at(project, timeline_id)?;
    let timeline = &project.timelines[t];
    let r = track_at(timeline, track_id)?;
    let track = &timeline.tracks[r];
    let held = |id: &str| clips_of(project).any(|c| c.id == id);
    let mut counter = Counter::of(project);
    if clip.id.is_empty() {
        clip.id = counter.make('c', held)?;
    } else if held(&clip.id) {
        return Err(taken("clip", &clip.id));
    } else {
        counter.saw(&clip.id);
    }
    check_clip(project, timeline.rate, track.kind, &clip)?;
    check_room(&track.clips, &clip)?;
    let id = clip.id.clone();
    put(&mut project.timelines[t].tracks[r].clips, clip);
    project.next_id = counter.next;
    Ok(Op::RemoveClip {
        timeline: timeline_id.into(),
        id,
    })
}

fn remove_clip(project: &mut Project, timeline_id: &str, id: &str) -> Result<Op, OpError> {
    let t = timeline_at(project, timeline_id)?;
    let (r, at) = clip_at(&project.timelines[t], id)?;
    let track = &mut project.timelines[t].tracks[r];
    let clip = track.clips.remove(at);
    Ok(Op::InsertClip {
        timeline: timeline_id.into(),
        track: track.id.clone(),
        clip,
    })
}

fn move_clip(
    project: &mut Project,
    timeline_id: &str,
    id: &str,
    track_id: &str,
    start: i64,
) -> Result<Op, OpError> {
    let t = timeline_at(project, timeline_id)?;
    let timeline = &project.timelines[t];
    let (from, at) = clip_at(timeline, id)?;
    let to = track_at(timeline, track_id)?;
    let kind = timeline.tracks[to].kind;
    if kind != timeline.tracks[from].kind {
        return Err(OpError::WrongKind {
            track: track_id.into(),
            kind,
        });
    }
    let old = &timeline.tracks[from].clips[at];
    let moved = Clip {
        start,
        ..old.clone()
    };
    check_clip(project, timeline.rate, kind, &moved)?;
    check_room(&timeline.tracks[to].clips, &moved)?;
    let inverse = Op::MoveClip {
        timeline: timeline_id.into(),
        id: id.into(),
        track: timeline.tracks[from].id.clone(),
        start: old.start,
    };
    let tracks = &mut project.timelines[t].tracks;
    tracks[from].clips.remove(at);
    put(&mut tracks[to].clips, moved);
    Ok(inverse)
}

fn trim_clip(
    project: &mut Project,
    timeline_id: &str,
    id: &str,
    source_in: Time,
    start: i64,
    length: i64,
) -> Result<Op, OpError> {
    let t = timeline_at(project, timeline_id)?;
    let timeline = &project.timelines[t];
    let (r, at) = clip_at(timeline, id)?;
    let track = &timeline.tracks[r];
    let old = &track.clips[at];
    let trimmed = Clip {
        source_in,
        start,
        length,
        ..old.clone()
    };
    check_clip(project, timeline.rate, track.kind, &trimmed)?;
    check_room(&track.clips, &trimmed)?;
    let inverse = Op::TrimClip {
        timeline: timeline_id.into(),
        id: id.into(),
        source_in: old.source_in,
        start: old.start,
        length: old.length,
    };
    let clips = &mut project.timelines[t].tracks[r].clips;
    clips.remove(at);
    put(clips, trimmed);
    Ok(inverse)
}

/// The numbers that the engine makes ids with, while an operation is checked. They are written
/// back to the project only once the operation is applied.
struct Counter {
    /// What `next_id` will be.
    next: u64,
}

impl Counter {
    fn of(project: &Project) -> Self {
        Self {
            next: project.next_id,
        }
    }

    /// Note an id that a thing is given, so that the engine never makes it.
    fn saw(&mut self, id: &str) {
        for letter in NUMBERED {
            if let Some(number) = number_in(id, letter) {
                if number < LARGEST_COUNT {
                    self.next = self.next.max(number + 1);
                }
            }
        }
    }

    /// Make an id of `letter` and the next number with which it is not `held`.
    fn make(&mut self, letter: char, held: impl Fn(&str) -> bool) -> Result<Id, OpError> {
        loop {
            let number = self.next;
            if number >= LARGEST_COUNT {
                return Err(OpError::TooLarge { what: "next_id" });
            }
            self.next = number + 1;
            let id = format!("{letter}{number}");
            if !held(&id) {
                return Ok(id);
            }
        }
    }
}

/// The number in an id that is `letter` and then a number written without a leading 0.
fn number_in(id: &str, letter: char) -> Option<u64> {
    let digits = id.strip_prefix(letter)?;
    let plain = !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'));
    if plain {
        digits.parse().ok()
    } else {
        None
    }
}

/// Check the ids that tracks to be added to a timeline, which has `existing` tracks, are given,
/// and those of their clips, and note them in `counter`. Gives the ids of the clips of the
/// project with those of the clips of `tracks`.
fn claim(
    project: &Project,
    counter: &mut Counter,
    existing: &[Track],
    tracks: &[Track],
) -> Result<HashSet<Id>, OpError> {
    let mut track_ids: HashSet<&str> = existing.iter().map(|r| r.id.as_str()).collect();
    let mut clip_ids: HashSet<Id> = clips_of(project).map(|c| c.id.clone()).collect();
    for track in tracks {
        if !track.id.is_empty() {
            if !track_ids.insert(&track.id) {
                return Err(taken("track", &track.id));
            }
            counter.saw(&track.id);
        }
        for clip in &track.clips {
            if !clip.id.is_empty() {
                if !clip_ids.insert(clip.id.clone()) {
                    return Err(taken("clip", &clip.id));
                }
                counter.saw(&clip.id);
            }
        }
    }
    Ok(clip_ids)
}

/// Give an id to each track of `tracks` and each clip of theirs that has none, in the order of
/// the lists: a track, then its clips, then the next track. `clip_ids` holds the ids of the
/// clips that there are.
fn name(
    counter: &mut Counter,
    clip_ids: &mut HashSet<Id>,
    existing: &[Track],
    tracks: &mut [Track],
) -> Result<(), OpError> {
    for at in 0..tracks.len() {
        if tracks[at].id.is_empty() {
            let ids = existing.iter().chain(tracks.iter()).map(|r| r.id.as_str());
            tracks[at].id = track_id(tracks[at].kind, ids)?;
        }
        for clip in &mut tracks[at].clips {
            if clip.id.is_empty() {
                clip.id = counter.make('c', |id| clip_ids.contains(id))?;
                clip_ids.insert(clip.id.clone());
            }
        }
    }
    Ok(())
}

/// The id of a new track of `kind` in a timeline whose tracks have `ids`: its letter, and one
/// above the highest number of an id of that letter.
fn track_id<'a>(kind: TrackKind, ids: impl Iterator<Item = &'a str>) -> Result<Id, OpError> {
    let letter = match kind {
        TrackKind::Video => 'V',
        TrackKind::Audio => 'A',
    };
    let highest = ids
        .filter_map(|id| number_in(id, letter))
        .max()
        .unwrap_or(0);
    let number = highest.checked_add(1).ok_or(OpError::TooLarge {
        what: "number of the track",
    })?;
    Ok(format!("{letter}{number}"))
}

/// Check the clips of tracks to be added to a timeline of `rate`, and put each track's in the
/// order of their start.
fn check_tracks(project: &Project, rate: Rate, tracks: &mut [Track]) -> Result<(), OpError> {
    for track in tracks {
        for clip in &track.clips {
            check_clip(project, rate, track.kind, clip)?;
        }
        track.clips.sort_by_key(|c| c.start);
        if let Some(pair) = track.clips.windows(2).find(|p| overlap(&p[0], &p[1])) {
            return Err(OpError::Overlap {
                with: pair[0].id.clone(),
            });
        }
    }
    Ok(())
}

/// Check a clip as it would be on a track of `kind`, in a timeline of `rate`.
fn check_clip(project: &Project, rate: Rate, kind: TrackKind, clip: &Clip) -> Result<(), OpError> {
    check_rate(rate)?;
    let media = project
        .media
        .iter()
        .find(|m| m.id == clip.media)
        .ok_or_else(|| missing("media", &clip.media))?;
    within("start", clip.start.into())?;
    within("length", clip.length.into())?;
    within("end", end_of(clip))?;
    within("source_in", clip.source_in.ticks.into())?;
    if clip.start < 0 {
        return Err(OpError::OutOfTimeline { start: clip.start });
    }
    let duration = media.facts.duration;
    let scale = clip.source_in.scale;
    if scale == 0 || scale != duration.scale {
        return Err(OpError::Scale {
            scale,
            media_scale: duration.scale,
        });
    }
    let source_end =
        i128::from(clip.source_in.ticks) + i128::from(rate.ticks_of(clip.length, scale));
    if clip.length <= 0 || clip.source_in.ticks < 0 || source_end > i128::from(duration.ticks) {
        return Err(OpError::OutOfSource {
            media: media.id.clone(),
        });
    }
    let has = match kind {
        TrackKind::Video => media.facts.video.is_some(),
        TrackKind::Audio => media.facts.audio.is_some(),
    };
    if !has {
        return Err(OpError::NoStream {
            media: media.id.clone(),
            kind,
        });
    }
    Ok(())
}

/// Check that a clip covers no frame that one of `clips`, other than itself, covers.
fn check_room(clips: &[Clip], clip: &Clip) -> Result<(), OpError> {
    match clips.iter().find(|c| c.id != clip.id && overlap(c, clip)) {
        Some(other) => Err(OpError::Overlap {
            with: other.id.clone(),
        }),
        None => Ok(()),
    }
}

/// Whether two clips cover a frame in common.
fn overlap(a: &Clip, b: &Clip) -> bool {
    i128::from(a.start) < end_of(b) && i128::from(b.start) < end_of(a)
}

/// The frame after the last one of a clip.
fn end_of(clip: &Clip) -> i128 {
    i128::from(clip.start) + i128::from(clip.length)
}

/// Put a clip among the clips of a track, in the order of their start.
fn put(clips: &mut Vec<Clip>, clip: Clip) {
    let at = clips.partition_point(|c| c.start < clip.start);
    clips.insert(at, clip);
}

fn check_rate(rate: Rate) -> Result<(), OpError> {
    if rate.num == 0 || rate.den == 0 {
        Err(OpError::ZeroRate)
    } else {
        Ok(())
    }
}

/// Check that an integer to be written is one that JavaScript reads exactly.
fn within(what: &'static str, value: i128) -> Result<(), OpError> {
    if value.unsigned_abs() > u128::from(LARGEST_COUNT) {
        Err(OpError::TooLarge { what })
    } else {
        Ok(())
    }
}

/// Where a thing goes in a list of `len`: at `index`, or at the end if there is none.
fn place(index: Option<usize>, len: usize) -> Result<usize, OpError> {
    match index {
        None => Ok(len),
        Some(at) if at <= len => Ok(at),
        Some(at) => Err(OpError::OutOfList { at, len }),
    }
}

fn missing(what: &'static str, id: &str) -> OpError {
    OpError::Missing {
        what,
        id: id.into(),
    }
}

fn taken(what: &'static str, id: &str) -> OpError {
    OpError::Taken {
        what,
        id: id.into(),
    }
}

fn timeline_at(project: &Project, id: &str) -> Result<usize, OpError> {
    project
        .timelines
        .iter()
        .position(|t| t.id == id)
        .ok_or_else(|| missing("timeline", id))
}

fn track_at(timeline: &Timeline, id: &str) -> Result<usize, OpError> {
    timeline
        .tracks
        .iter()
        .position(|r| r.id == id)
        .ok_or_else(|| missing("track", id))
}

/// Where a clip is in a timeline: its track, and its place on it.
fn clip_at(timeline: &Timeline, id: &str) -> Result<(usize, usize), OpError> {
    timeline
        .tracks
        .iter()
        .enumerate()
        .find_map(|(r, track)| {
            track
                .clips
                .iter()
                .position(|c| c.id == id)
                .map(|at| (r, at))
        })
        .ok_or_else(|| missing("clip", id))
}

/// Every clip of a project.
fn clips_of(project: &Project) -> impl Iterator<Item = &Clip> {
    project
        .timelines
        .iter()
        .flat_map(|t| &t.tracks)
        .flat_map(|r| &r.clips)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The scale of the media of the sample: m1 lasts 153600 ticks of it, 250 frames at 25/1.
    const SCALE: u32 = 15360;

    // The sample: timeline t1 at 25/1, with c1 on V1 and c2 on A1, each on frames [0, 100) and
    // cut from m1, which lasts 10 seconds; next_id is 3.

    fn clip(id: &str, media: &str, start: i64, length: i64) -> Clip {
        Clip {
            id: id.into(),
            media: media.into(),
            source_in: Time {
                ticks: 0,
                scale: SCALE,
            },
            start,
            length,
        }
    }

    fn insert(track: &str, clip: Clip) -> Op {
        Op::InsertClip {
            timeline: "t1".into(),
            track: track.into(),
            clip,
        }
    }

    /// Media like m1, with an id of `id`, and picture or sound only if asked for.
    fn media(id: &str, video: bool, audio: bool) -> Media {
        let mut media = Project::sample().media[0].clone();
        media.id = id.into();
        if !video {
            media.facts.video = None;
        }
        if !audio {
            media.facts.audio = None;
        }
        media
    }

    fn track(id: &str, kind: TrackKind, clips: Vec<Clip>) -> Track {
        Track {
            id: id.into(),
            kind,
            clips,
        }
    }

    fn timeline(id: &str, tracks: Vec<Track>) -> Timeline {
        Timeline {
            id: id.into(),
            name: "Timeline 2".into(),
            rate: Rate { num: 25, den: 1 },
            tracks,
        }
    }

    /// Apply operations to the sample, each of which must be applied.
    fn sample_with(ops: Vec<Op>) -> Project {
        let mut project = Project::sample();
        for op in ops {
            apply(&mut project, op).unwrap();
        }
        project
    }

    /// The error that `op` is refused with, once it is checked that the project is unchanged.
    fn refused_in(mut project: Project, op: Op) -> OpError {
        let before = project.clone();
        let error = apply(&mut project, op).unwrap_err();
        assert_eq!(project, before);
        error
    }

    fn refused(op: Op) -> OpError {
        refused_in(Project::sample(), op)
    }

    fn starts(project: &Project, track: usize) -> Vec<i64> {
        project.timelines[0].tracks[track]
            .clips
            .iter()
            .map(|c| c.start)
            .collect()
    }

    #[test]
    fn an_added_clip_gets_an_id_and_its_inverse_removes_it() {
        let mut project = Project::sample();
        let op = insert("V1", clip("", "m1", 150, 50));
        let inverse = apply(&mut project, op).unwrap();
        let Op::RemoveClip { id, .. } = &inverse else {
            panic!("{inverse:?}")
        };
        assert!(id.starts_with('c'));
        assert_eq!(id, "c3");
        assert_eq!(project.next_id, 4);
        apply(&mut project, inverse).unwrap();
        assert_eq!(project.timelines[0].tracks[0].clips.len(), 1);
        assert_eq!(project.next_id, 4, "next_id never goes back");
    }

    #[test]
    fn the_clips_of_a_track_stay_in_the_order_of_their_start() {
        let mut project = sample_with(vec![
            insert("V1", clip("", "m1", 300, 50)),
            insert("V1", clip("", "m1", 150, 50)),
        ]);
        assert_eq!(starts(&project, 0), [0, 150, 300]);
        let id = project.timelines[0].tracks[0].clips[2].id.clone();
        let op = Op::MoveClip {
            timeline: "t1".into(),
            id: id.clone(),
            track: "V1".into(),
            start: 100,
        };
        apply(&mut project, op).unwrap();
        assert_eq!(starts(&project, 0), [0, 100, 150]);
        assert_eq!(project.timelines[0].tracks[0].clips[1].id, id);
        let op = Op::TrimClip {
            timeline: "t1".into(),
            id: "c1".into(),
            source_in: Time {
                ticks: 0,
                scale: SCALE,
            },
            start: 400,
            length: 10,
        };
        apply(&mut project, op).unwrap();
        assert_eq!(starts(&project, 0), [100, 150, 400]);
    }

    #[test]
    fn the_inverse_of_the_inverse_makes_the_same_things_with_the_same_ids() {
        let mut project = Project::sample();
        let added = Op::AddTimeline {
            timeline: timeline(
                "",
                vec![
                    track(
                        "",
                        TrackKind::Video,
                        vec![clip("", "m1", 50, 10), clip("", "m1", 0, 10)],
                    ),
                    track("", TrackKind::Audio, vec![clip("", "m1", 0, 10)]),
                ],
            ),
            index: None,
        };
        let inverse = apply(&mut project, added).unwrap();
        let done = project.clone();
        // Ids are given depth first, in the order of the lists, before the clips are sorted.
        let made = &project.timelines[1];
        assert_eq!(made.id, "t3");
        assert_eq!(made.tracks[0].id, "V1");
        assert_eq!(made.tracks[1].id, "A1");
        let ids: Vec<&str> = made.tracks[0].clips.iter().map(|c| &*c.id).collect();
        assert_eq!(ids, ["c5", "c4"]);
        assert_eq!(made.tracks[1].clips[0].id, "c6");
        assert_eq!(project.next_id, 7);

        let again = apply(&mut project, inverse).unwrap();
        assert_eq!(project.timelines.len(), 1);
        let Op::AddTimeline { index, .. } = &again else {
            panic!("{again:?}")
        };
        assert_eq!(*index, Some(1));
        apply(&mut project, again).unwrap();
        assert_eq!(project, done);

        // And so for a clip that the engine named.
        let inverse = apply(&mut project, insert("V1", clip("", "m1", 100, 10))).unwrap();
        let done = project.clone();
        let again = apply(&mut project, inverse).unwrap();
        let Op::InsertClip { clip, .. } = &again else {
            panic!("{again:?}")
        };
        assert_eq!(clip.id, "c7");
        apply(&mut project, again).unwrap();
        assert_eq!(project, done);
    }

    #[test]
    fn a_removed_track_comes_back_whole_at_its_place() {
        let mut project = Project::sample();
        let op = Op::RemoveTrack {
            timeline: "t1".into(),
            id: "V1".into(),
        };
        let inverse = apply(&mut project, op).unwrap();
        assert_eq!(project.timelines[0].tracks.len(), 1);
        assert_eq!(
            inverse,
            Op::AddTrack {
                timeline: "t1".into(),
                track: Project::sample().timelines[0].tracks[0].clone(),
                index: Some(0),
            }
        );
        apply(&mut project, inverse).unwrap();
        assert_eq!(project, Project::sample());
    }

    #[test]
    fn a_removed_timeline_comes_back_whole_at_its_place() {
        let mut project = Project::sample();
        let inverse = apply(&mut project, Op::RemoveTimeline { id: "t1".into() }).unwrap();
        assert_eq!(project.timelines, []);
        apply(&mut project, inverse).unwrap();
        assert_eq!(project, Project::sample());
    }

    #[test]
    fn media_comes_back_at_its_place() {
        let mut project = sample_with(vec![Op::AddMedia {
            media: media("", true, true),
            index: Some(0),
        }]);
        assert_eq!(project.media[0].id, "m3");
        let inverse = apply(&mut project, Op::RemoveMedia { id: "m3".into() }).unwrap();
        assert_eq!(
            inverse,
            Op::AddMedia {
                media: media("m3", true, true),
                index: Some(0),
            }
        );
    }

    #[test]
    fn a_move_and_a_trim_are_taken_back() {
        let mut project = sample_with(vec![Op::AddTrack {
            timeline: "t1".into(),
            track: track("", TrackKind::Video, vec![]),
            index: None,
        }]);
        let before = project.clone();
        let op = Op::MoveClip {
            timeline: "t1".into(),
            id: "c1".into(),
            track: "V2".into(),
            start: 30,
        };
        let inverse = apply(&mut project, op).unwrap();
        assert_eq!(project.timelines[0].tracks[0].clips, []);
        assert_eq!(project.timelines[0].tracks[2].clips[0].start, 30);
        assert_eq!(
            inverse,
            Op::MoveClip {
                timeline: "t1".into(),
                id: "c1".into(),
                track: "V1".into(),
                start: 0,
            }
        );
        apply(&mut project, inverse).unwrap();
        assert_eq!(project, before);

        let op = Op::TrimClip {
            timeline: "t1".into(),
            id: "c2".into(),
            source_in: Time {
                ticks: 614,
                scale: SCALE,
            },
            start: 10,
            length: 200,
        };
        let inverse = apply(&mut project, op).unwrap();
        let trimmed = &project.timelines[0].tracks[1].clips[0];
        assert_eq!(
            (trimmed.source_in.ticks, trimmed.start, trimmed.length),
            (614, 10, 200)
        );
        apply(&mut project, inverse).unwrap();
        assert_eq!(project, before);
    }

    #[test]
    fn a_new_track_is_numbered_one_above_the_highest_of_its_letter() {
        let add = |id: &str, kind| Op::AddTrack {
            timeline: "t1".into(),
            track: track(id, kind, vec![]),
            index: None,
        };
        let mut project = Project::sample();
        apply(&mut project, add("", TrackKind::Video)).unwrap();
        apply(&mut project, add("", TrackKind::Audio)).unwrap();
        apply(&mut project, add("V7", TrackKind::Video)).unwrap();
        apply(&mut project, add("", TrackKind::Video)).unwrap();
        let ids: Vec<&str> = project.timelines[0].tracks.iter().map(|r| &*r.id).collect();
        assert_eq!(ids, ["V1", "A1", "V2", "A2", "V7", "V8"]);
        assert_eq!(project.next_id, 3, "tracks do not use next_id");

        // Without V2, the highest is still V8; without anything above V1, V2 is made again.
        let mut project = sample_with(vec![add("", TrackKind::Video)]);
        let remove = Op::RemoveTrack {
            timeline: "t1".into(),
            id: "V2".into(),
        };
        apply(&mut project, remove).unwrap();
        let inverse = apply(&mut project, add("", TrackKind::Video)).unwrap();
        assert_eq!(
            inverse,
            Op::RemoveTrack {
                timeline: "t1".into(),
                id: "V2".into(),
            }
        );
    }

    #[test]
    fn an_id_that_the_engine_could_make_moves_next_id_past_it() {
        let mut project = sample_with(vec![insert("V1", clip("c9", "m1", 100, 10))]);
        assert_eq!(project.next_id, 10);
        apply(&mut project, insert("V1", clip("", "m1", 110, 10))).unwrap();
        assert_eq!(project.timelines[0].tracks[0].clips[2].id, "c10");
        // An id of another form leaves it be.
        apply(&mut project, insert("V1", clip("c099", "m1", 120, 10))).unwrap();
        apply(&mut project, insert("V1", clip("x500", "m1", 130, 10))).unwrap();
        assert_eq!(project.next_id, 11);
    }

    #[test]
    fn a_number_whose_id_is_held_is_passed_over() {
        let mut project = Project::sample();
        project.next_id = 1;
        apply(&mut project, insert("V1", clip("", "m1", 100, 10))).unwrap();
        assert_eq!(project.timelines[0].tracks[0].clips[1].id, "c3");
        assert_eq!(project.next_id, 4);
    }

    #[test]
    fn a_thing_that_is_not_there_is_missing() {
        let cases = [
            (insert("V1", clip("", "m9", 100, 10)), "media", "m9"),
            (Op::RemoveMedia { id: "m9".into() }, "media", "m9"),
            (Op::RemoveTimeline { id: "t9".into() }, "timeline", "t9"),
            (
                Op::InsertClip {
                    timeline: "t9".into(),
                    track: "V1".into(),
                    clip: clip("", "m1", 100, 10),
                },
                "timeline",
                "t9",
            ),
            (insert("V9", clip("", "m1", 100, 10)), "track", "V9"),
            (
                Op::RemoveTrack {
                    timeline: "t1".into(),
                    id: "V9".into(),
                },
                "track",
                "V9",
            ),
            (
                Op::RemoveClip {
                    timeline: "t1".into(),
                    id: "c9".into(),
                },
                "clip",
                "c9",
            ),
            (
                Op::MoveClip {
                    timeline: "t1".into(),
                    id: "c1".into(),
                    track: "V9".into(),
                    start: 0,
                },
                "track",
                "V9",
            ),
        ];
        for (op, what, id) in cases {
            let error = refused(op);
            assert_eq!(
                error,
                OpError::Missing {
                    what,
                    id: id.into()
                }
            );
        }
    }

    #[test]
    fn a_clip_needs_the_stream_of_its_track() {
        let project = sample_with(vec![
            Op::AddMedia {
                media: media("sound", false, true),
                index: None,
            },
            Op::AddMedia {
                media: media("picture", true, false),
                index: None,
            },
        ]);
        let error = refused_in(project.clone(), insert("V1", clip("", "sound", 100, 10)));
        assert_eq!(
            error,
            OpError::NoStream {
                media: "sound".into(),
                kind: TrackKind::Video
            }
        );
        assert_eq!(
            error.to_string(),
            "media sound has no video stream, which a video track needs"
        );
        let error = refused_in(project, insert("A1", clip("", "picture", 100, 10)));
        assert!(
            matches!(
                error,
                OpError::NoStream {
                    kind: TrackKind::Audio,
                    ..
                }
            ),
            "{error}"
        );
    }

    #[test]
    fn a_clip_moves_only_between_tracks_of_one_kind() {
        let error = refused(Op::MoveClip {
            timeline: "t1".into(),
            id: "c1".into(),
            track: "A1".into(),
            start: 200,
        });
        assert_eq!(
            error,
            OpError::WrongKind {
                track: "A1".into(),
                kind: TrackKind::Audio
            }
        );
    }

    #[test]
    fn a_clip_lasts_at_least_a_frame() {
        for length in [0, -1] {
            let error = refused(insert("V1", clip("", "m1", 100, length)));
            assert!(matches!(error, OpError::OutOfSource { .. }), "{error}");
        }
    }

    #[test]
    fn a_clip_shows_only_what_is_in_its_media() {
        // m1 lasts 250 frames: all of them fit, and one more does not.
        let mut project = Project::sample();
        apply(&mut project, insert("V1", clip("", "m1", 100, 250))).unwrap();
        let error = refused(insert("V1", clip("", "m1", 100, 251)));
        assert_eq!(error, OpError::OutOfSource { media: "m1".into() });
        // From the last frame, one frame fits and two do not.
        let mut late = clip("", "m1", 100, 1);
        late.source_in.ticks = Rate { num: 25, den: 1 }.ticks_of(249, SCALE);
        apply(&mut Project::sample(), insert("V1", late.clone())).unwrap();
        late.length = 2;
        assert!(matches!(
            refused(insert("V1", late)),
            OpError::OutOfSource { .. }
        ));
        let mut early = clip("", "m1", 100, 10);
        early.source_in.ticks = -1;
        assert!(matches!(
            refused(insert("V1", early)),
            OpError::OutOfSource { .. }
        ));
        // A trim is checked so too.
        let error = refused(Op::TrimClip {
            timeline: "t1".into(),
            id: "c1".into(),
            source_in: Time {
                ticks: 15360,
                scale: SCALE,
            },
            start: 0,
            length: 226,
        });
        assert!(matches!(error, OpError::OutOfSource { .. }), "{error}");
    }

    #[test]
    fn a_clip_counts_its_source_in_the_scale_of_its_media() {
        let mut other = clip("", "m1", 100, 10);
        other.source_in.scale = 90000;
        assert_eq!(
            refused(insert("V1", other)),
            OpError::Scale {
                scale: 90000,
                media_scale: SCALE
            }
        );
        let mut none = clip("", "m1", 100, 10);
        none.source_in.scale = 0;
        let error = refused(insert("V1", none));
        assert_eq!(
            error,
            OpError::Scale {
                scale: 0,
                media_scale: SCALE
            }
        );
        assert_eq!(
            error.to_string(),
            "a time cannot count ticks of 1/0 of a second"
        );
        let mut endless = media("", true, true);
        endless.facts.duration.scale = 0;
        let error = refused(Op::AddMedia {
            media: endless,
            index: None,
        });
        assert!(matches!(error, OpError::Scale { scale: 0, .. }), "{error}");
    }

    #[test]
    fn two_clips_of_a_track_cover_no_frame_in_common() {
        assert_eq!(
            refused(insert("V1", clip("", "m1", 99, 10))),
            OpError::Overlap { with: "c1".into() }
        );
        // A clip may start on the frame after another ends.
        apply(
            &mut Project::sample(),
            insert("V1", clip("", "m1", 100, 10)),
        )
        .unwrap();
        // A clip on another track does not count, nor does the clip itself when it moves.
        apply(
            &mut Project::sample(),
            insert("A1", clip("", "m1", 100, 10)),
        )
        .unwrap();
        let project = sample_with(vec![insert("V1", clip("", "m1", 100, 50))]);
        let shift = Op::MoveClip {
            timeline: "t1".into(),
            id: "c1".into(),
            track: "V1".into(),
            start: 50,
        };
        assert_eq!(
            refused_in(project.clone(), shift),
            OpError::Overlap { with: "c3".into() }
        );
        let nudge = Op::MoveClip {
            timeline: "t1".into(),
            id: "c3".into(),
            track: "V1".into(),
            start: 120,
        };
        apply(&mut project.clone(), nudge).unwrap();
        // Two clips of a track that is added are checked against each other.
        let error = refused(Op::AddTrack {
            timeline: "t1".into(),
            track: track(
                "",
                TrackKind::Video,
                vec![clip("", "m1", 5, 10), clip("x", "m1", 0, 10)],
            ),
            index: None,
        });
        assert_eq!(error, OpError::Overlap { with: "x".into() });
    }

    #[test]
    fn a_clip_starts_no_earlier_than_the_timeline() {
        assert_eq!(
            refused(insert("V1", clip("", "m1", -1, 10))),
            OpError::OutOfTimeline { start: -1 }
        );
        let error = refused(Op::MoveClip {
            timeline: "t1".into(),
            id: "c1".into(),
            track: "V1".into(),
            start: -5,
        });
        assert_eq!(error, OpError::OutOfTimeline { start: -5 });
    }

    #[test]
    fn media_that_a_clip_uses_stays() {
        let error = refused(Op::RemoveMedia { id: "m1".into() });
        assert_eq!(
            error,
            OpError::InUse {
                id: "m1".into(),
                by: "c1".into()
            }
        );
    }

    #[test]
    fn an_id_that_is_held_is_not_given_again() {
        let cases = [
            (insert("V1", clip("c2", "m1", 100, 10)), "clip", "c2"),
            (
                Op::AddMedia {
                    media: media("m1", true, true),
                    index: None,
                },
                "media",
                "m1",
            ),
            (
                Op::AddTimeline {
                    timeline: timeline("t1", vec![]),
                    index: None,
                },
                "timeline",
                "t1",
            ),
            (
                Op::AddTrack {
                    timeline: "t1".into(),
                    track: track("A1", TrackKind::Audio, vec![]),
                    index: None,
                },
                "track",
                "A1",
            ),
            (
                Op::AddTimeline {
                    timeline: timeline(
                        "",
                        vec![
                            track("V1", TrackKind::Video, vec![]),
                            track("V1", TrackKind::Video, vec![]),
                        ],
                    ),
                    index: None,
                },
                "track",
                "V1",
            ),
            (
                Op::AddTimeline {
                    timeline: timeline(
                        "",
                        vec![track("", TrackKind::Video, vec![clip("c1", "m1", 0, 10)])],
                    ),
                    index: None,
                },
                "clip",
                "c1",
            ),
        ];
        for (op, what, id) in cases {
            assert_eq!(
                refused(op),
                OpError::Taken {
                    what,
                    id: id.into()
                }
            );
        }
    }

    #[test]
    fn a_thing_goes_no_further_than_the_end_of_its_list() {
        let error = refused(Op::AddMedia {
            media: media("", true, true),
            index: Some(2),
        });
        assert_eq!(error, OpError::OutOfList { at: 2, len: 1 });
        let error = refused(Op::AddTrack {
            timeline: "t1".into(),
            track: track("", TrackKind::Audio, vec![]),
            index: Some(3),
        });
        assert_eq!(error, OpError::OutOfList { at: 3, len: 2 });
    }

    #[test]
    fn a_timeline_has_a_whole_rate() {
        let mut stopped = timeline("", vec![]);
        stopped.rate.num = 0;
        let error = refused(Op::AddTimeline {
            timeline: stopped,
            index: None,
        });
        assert_eq!(error, OpError::ZeroRate);
    }

    #[test]
    fn an_integer_that_javascript_would_not_read_exactly_is_refused() {
        let too_large = |what| OpError::TooLarge { what };
        assert_eq!(
            refused(insert("V1", clip("", "m1", LARGEST + 1, 10))),
            too_large("start")
        );
        assert_eq!(
            refused(insert("V1", clip("", "m1", LARGEST - 5, 10))),
            too_large("end")
        );
        assert_eq!(
            refused(insert("V1", clip("", "m1", 100, i64::MAX))),
            too_large("length")
        );
        let mut far = clip("", "m1", 100, 10);
        far.source_in.ticks = -LARGEST - 1;
        assert_eq!(refused(insert("V1", far)), too_large("source_in"));
        let mut huge = media("", true, true);
        huge.identity.size = 1 << 53;
        assert_eq!(
            refused(Op::AddMedia {
                media: huge,
                index: None,
            }),
            too_large("size")
        );
        let mut old = media("", true, true);
        old.identity.modified_us = i64::MIN;
        assert_eq!(
            refused(Op::AddMedia {
                media: old,
                index: None,
            }),
            too_large("modified_us")
        );
        let mut long = media("", true, true);
        long.facts.duration.ticks = LARGEST + 1;
        assert_eq!(
            refused(Op::AddMedia {
                media: long,
                index: None,
            }),
            too_large("duration")
        );

        // The last number that next_id can be is 2^53 - 1, so the last id it makes ends in one
        // below that.
        let mut project = Project::sample();
        project.next_id = LARGEST.unsigned_abs() - 1;
        apply(&mut project, insert("V1", clip("", "m1", 100, 10))).unwrap();
        assert_eq!(project.next_id, LARGEST.unsigned_abs());
        let error = refused_in(project.clone(), insert("V1", clip("", "m1", 110, 10)));
        assert_eq!(error, too_large("next_id"));
        // An id of the engine's form that it could never make leaves next_id be.
        let mut project = Project::sample();
        apply(
            &mut project,
            insert("V1", clip("c9007199254740991", "m1", 100, 10)),
        )
        .unwrap();
        assert_eq!(project.next_id, 3);
    }

    #[test]
    fn a_refused_operation_leaves_the_project_and_next_id_as_they_were() {
        // The timeline and its first clip are named before the second clip is refused.
        let error = refused(Op::AddTimeline {
            timeline: timeline(
                "",
                vec![track(
                    "",
                    TrackKind::Video,
                    vec![clip("", "m1", 0, 10), clip("", "m9", 20, 10)],
                )],
            ),
            index: None,
        });
        assert_eq!(
            error,
            OpError::Missing {
                what: "media",
                id: "m9".into()
            }
        );
    }

    #[test]
    fn every_operation_is_written_with_the_names_that_other_languages_read() {
        let source_in = Time {
            ticks: 0,
            scale: SCALE,
        };
        let ops = [
            (
                Op::AddMedia {
                    media: media("m1", true, false),
                    index: Some(0),
                },
                "add_media",
            ),
            (Op::RemoveMedia { id: "m1".into() }, "remove_media"),
            (
                Op::AddTimeline {
                    timeline: timeline("t2", vec![track("V1", TrackKind::Video, vec![])]),
                    index: None,
                },
                "add_timeline",
            ),
            (Op::RemoveTimeline { id: "t1".into() }, "remove_timeline"),
            (
                Op::AddTrack {
                    timeline: "t1".into(),
                    track: track("A2", TrackKind::Audio, vec![clip("c5", "m1", 0, 1)]),
                    index: Some(1),
                },
                "add_track",
            ),
            (
                Op::RemoveTrack {
                    timeline: "t1".into(),
                    id: "A1".into(),
                },
                "remove_track",
            ),
            (insert("V1", clip("", "m1", 100, 10)), "insert_clip"),
            (
                Op::RemoveClip {
                    timeline: "t1".into(),
                    id: "c1".into(),
                },
                "remove_clip",
            ),
            (
                Op::MoveClip {
                    timeline: "t1".into(),
                    id: "c1".into(),
                    track: "V1".into(),
                    start: 5,
                },
                "move_clip",
            ),
            (
                Op::TrimClip {
                    timeline: "t1".into(),
                    id: "c1".into(),
                    source_in,
                    start: 5,
                    length: 7,
                },
                "trim_clip",
            ),
        ];
        for (op, tag) in ops {
            let value = serde_json::to_value(&op).unwrap();
            assert_eq!(value["op"], tag, "{value}");
            assert_eq!(serde_json::from_value::<Op>(value).unwrap(), op);
        }
        let value = serde_json::to_value(Op::TrimClip {
            timeline: "t1".into(),
            id: "c1".into(),
            source_in,
            start: 5,
            length: 7,
        })
        .unwrap();
        let expected = serde_json::json!({
            "op": "trim_clip",
            "timeline": "t1",
            "id": "c1",
            "source_in": { "ticks": 0, "scale": 15360 },
            "start": 5,
            "length": 7,
        });
        assert_eq!(value, expected);
    }

    #[test]
    fn an_index_that_is_left_out_is_none() {
        let value = serde_json::json!({
            "op": "add_track",
            "timeline": "t1",
            "track": { "id": "", "kind": "video", "clips": [] },
        });
        let op: Op = serde_json::from_value(value).unwrap();
        let Op::AddTrack { index, .. } = op else {
            panic!("{op:?}")
        };
        assert_eq!(index, None);
    }

    #[test]
    fn a_field_that_is_not_known_is_refused_in_an_operation() {
        let value = serde_json::json!({
            "op": "remove_clip", "timeline": "t1", "id": "c1", "colour": "red",
        });
        let error = serde_json::from_value::<Op>(value).unwrap_err();
        assert!(error.to_string().contains("colour"), "{error}");
        let value = serde_json::json!({
            "op": "insert_clip", "timeline": "t1", "track": "V1",
            "clip": {
                "id": "", "media": "m1", "source_in": { "ticks": 0, "scale": 15360 },
                "start": 0, "length": 1, "colour": "red",
            },
        });
        assert!(serde_json::from_value::<Op>(value).is_err());
        let value = serde_json::json!({ "op": "paint_clip", "timeline": "t1", "id": "c1" });
        assert!(serde_json::from_value::<Op>(value).is_err());
    }

    #[test]
    fn an_error_is_written_with_its_reason_its_message_and_its_fields() {
        let value = serde_json::to_value(OpError::Overlap { with: "c1".into() }).unwrap();
        let expected = serde_json::json!({
            "reason": "overlap",
            "with": "c1",
            "message": "the clip covers frames that clip c1 covers",
        });
        assert_eq!(value, expected);
        let value = serde_json::to_value(OpError::ZeroRate).unwrap();
        assert_eq!(value["reason"], "zero_rate");
        assert_eq!(value["message"], "a rate of frames has a 0 in it");
        let value = serde_json::to_value(OpError::NoStream {
            media: "m2".into(),
            kind: TrackKind::Audio,
        })
        .unwrap();
        assert_eq!(value["reason"], "no_stream");
        assert_eq!(value["kind"], "audio");
        let value = serde_json::to_value(OpError::OutOfTimeline { start: -1 }).unwrap();
        assert_eq!(value["reason"], "out_of_timeline");
        assert_eq!(value["start"], -1);

        let id = || Id::from("x");
        let kind = TrackKind::Video;
        let every = [
            (
                OpError::Missing {
                    what: "clip",
                    id: id(),
                },
                "missing",
            ),
            (
                OpError::Taken {
                    what: "clip",
                    id: id(),
                },
                "taken",
            ),
            (OpError::OutOfList { at: 1, len: 0 }, "out_of_list"),
            (OpError::NoStream { media: id(), kind }, "no_stream"),
            (OpError::WrongKind { track: id(), kind }, "wrong_kind"),
            (OpError::OutOfSource { media: id() }, "out_of_source"),
            (
                OpError::Scale {
                    scale: 1,
                    media_scale: 2,
                },
                "scale",
            ),
            (OpError::Overlap { with: id() }, "overlap"),
            (OpError::OutOfTimeline { start: -1 }, "out_of_timeline"),
            (OpError::InUse { id: id(), by: id() }, "in_use"),
            (OpError::TooLarge { what: "start" }, "too_large"),
            (OpError::ZeroRate, "zero_rate"),
        ];
        for (error, reason) in every {
            let value = serde_json::to_value(&error).unwrap();
            assert_eq!(value["reason"], reason);
            assert_eq!(value["message"], error.to_string());
        }
    }

    #[test]
    fn media_with_one_stream_goes_on_a_track_of_its_kind() {
        let mut project = sample_with(vec![
            Op::AddMedia {
                media: media("", false, true),
                index: None,
            },
            Op::AddMedia {
                media: media("", true, false),
                index: None,
            },
        ]);
        apply(&mut project, insert("A1", clip("", "m3", 100, 10))).unwrap();
        apply(&mut project, insert("V1", clip("", "m4", 100, 10))).unwrap();
    }
}
