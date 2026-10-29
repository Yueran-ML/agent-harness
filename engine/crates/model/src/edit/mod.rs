//! Edits: what an editor does to a timeline, as `DaVinci` names it (insert, overwrite, ripple
//! delete, blade, roll, slip …), each worked out into the operations of [`crate::ops`] at the
//! moment that it is applied.
//!
//! An edit is not an operation: it is worked out against the project as it is when it is
//! applied, after the edits and operations before it in the same transaction, into operations
//! that are applied one by one. What is undone is those operations, so taking an edit back needs
//! nothing that the operations do not have, and a client's copy of a project needs to know the
//! operations only. An edit is applied whole or not at all.
//!
//! An edit is written in JSON as an object that names it in `edit`, beside the operations of a
//! transaction: `{"edit": "ripple_delete", "timeline": "t1", "clips": ["c4"]}`. A field that is
//! not known is refused.
//!
//! # Links and locks
//!
//! An edit of clips works on the clips that it names and on the clips linked to them, in the same
//! group, unless it is given `"linked": false`. No edit changes a track that is locked: an edit
//! that names a locked track, or a clip on one, is refused with `locked`; a clip linked to one
//! that it names is left alone if its track is locked; and a ripple moves what is on the tracks
//! that are not locked only.
//!
//! # Ripples
//!
//! An edit that ripples moves what comes after its place, on every track that is not locked, by
//! the frames that it adds or takes away, so that what was in step stays in step. Where it takes
//! frames away, and another track has a clip in them that the edit does not take away, the
//! edit is refused with `ripple_blocked`, naming the track and the thing: lock that track, or cut
//! the frames out of every track with `ripple_delete_range`.
//! Subtitles are cut with the frames taken away, and those after them move with the picture.
//! An insert through a subtitle cuts it into two with the same text, leaving the new frames
//! between them. Locked subtitle tracks stay as they were.
//!
//! # As far as it goes
//!
//! An edit that moves an edge or a clip by `delta` frames, and cannot go so far, for the end of
//! the media, a clip in the way or a clip that would be left with no frame, goes as far as it can
//! and says that it was clamped. One that cannot go at all is refused, and so is a `delta` of 0.

mod cut;
mod input;
mod misc;
mod place;
mod trim;
mod work;

use serde::{Deserialize, Serialize, Serializer};

use crate::ops::{apply_done, Op, OpError};
use crate::project::{Alignment, Id, Project, Rgba, TitleStyle, TrackKind, TransitionKind};
use crate::time::{Speed, Time};

use work::Work;

/// An edit of a timeline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "edit", rename_all = "snake_case", deny_unknown_fields)]
pub enum Edit {
    /// Put a source in at `at`: every track that is not locked is cut at `at`, and what is
    /// after it moves on by the length of the source.
    Insert {
        /// The timeline.
        timeline: Id,
        /// What is put in.
        source: Material,
        /// The frame that it goes in at.
        at: i64,
        /// The track that its picture goes on (see [`Material`]).
        #[serde(
            default,
            deserialize_with = "nullable",
            skip_serializing_if = "Option::is_none"
        )]
        video_track: Option<Option<Id>>,
        /// The track that its sound goes on.
        #[serde(
            default,
            deserialize_with = "nullable",
            skip_serializing_if = "Option::is_none"
        )]
        audio_track: Option<Option<Id>>,
    },
    /// Put a source on its tracks at `at`, over what is there; other tracks are left as they are.
    Overwrite {
        /// The timeline.
        timeline: Id,
        /// What is put on.
        source: Material,
        /// The frame that it starts at.
        at: i64,
        /// The track that its picture goes on.
        #[serde(
            default,
            deserialize_with = "nullable",
            skip_serializing_if = "Option::is_none"
        )]
        video_track: Option<Option<Id>>,
        /// The track that its sound goes on.
        #[serde(
            default,
            deserialize_with = "nullable",
            skip_serializing_if = "Option::is_none"
        )]
        audio_track: Option<Option<Id>>,
    },
    /// Show a source in place of the clip at `at` on each of its tracks, keeping where the clip
    /// is, how long it lasts and how it is set. The source begins at the clip's start from its
    /// mark in, or from its start; or, if `from` is given, `from` is shown at `at`.
    Replace {
        /// The timeline.
        timeline: Id,
        /// What is shown instead.
        source: Material,
        /// A frame of the clips to replace.
        at: i64,
        /// The track of picture to replace on.
        #[serde(
            default,
            deserialize_with = "nullable",
            skip_serializing_if = "Option::is_none"
        )]
        video_track: Option<Option<Id>>,
        /// The track of sound to replace on.
        #[serde(
            default,
            deserialize_with = "nullable",
            skip_serializing_if = "Option::is_none"
        )]
        audio_track: Option<Option<Id>>,
    },
    /// Put a source at `at` above everything there: on the first track of its kind, above the
    /// highest that has something in its frames, that is not locked, or on a new one.
    PlaceOnTop {
        /// The timeline.
        timeline: Id,
        /// What is put on.
        source: Material,
        /// The frame that it starts at.
        at: i64,
    },
    /// Put a source after the last clip of its tracks.
    Append {
        /// The timeline.
        timeline: Id,
        /// What is put on.
        source: Material,
        /// The track that its picture goes on.
        #[serde(
            default,
            deserialize_with = "nullable",
            skip_serializing_if = "Option::is_none"
        )]
        video_track: Option<Option<Id>>,
        /// The track that its sound goes on.
        #[serde(
            default,
            deserialize_with = "nullable",
            skip_serializing_if = "Option::is_none"
        )]
        audio_track: Option<Option<Id>>,
    },
    /// Insert at the cut nearest to `at`, on the tracks that are not locked.
    SmartInsert {
        /// The timeline.
        timeline: Id,
        /// What is put in.
        source: Material,
        /// A frame near the cut.
        at: i64,
        /// The track that its picture goes on.
        #[serde(
            default,
            deserialize_with = "nullable",
            skip_serializing_if = "Option::is_none"
        )]
        video_track: Option<Option<Id>>,
        /// The track that its sound goes on.
        #[serde(
            default,
            deserialize_with = "nullable",
            skip_serializing_if = "Option::is_none"
        )]
        audio_track: Option<Option<Id>>,
    },
    /// Put a source in place of the clip at `at`, for as long as the source lasts, and ripple
    /// what follows by the difference.
    RippleOverwrite {
        /// The timeline.
        timeline: Id,
        /// What is put on.
        source: Material,
        /// A frame of the clip to replace.
        at: i64,
        /// The track that its picture goes on.
        #[serde(
            default,
            deserialize_with = "nullable",
            skip_serializing_if = "Option::is_none"
        )]
        video_track: Option<Option<Id>>,
        /// The track that its sound goes on.
        #[serde(
            default,
            deserialize_with = "nullable",
            skip_serializing_if = "Option::is_none"
        )]
        audio_track: Option<Option<Id>>,
    },
    /// Take clips away, leaving gaps.
    Lift {
        /// The timeline.
        timeline: Id,
        /// The clips.
        clips: Vec<Id>,
        /// Whether the clips linked to them go too.
        #[serde(default = "yes")]
        linked: bool,
    },
    /// Take clips away and close the gaps that they leave, on every track that is not locked.
    RippleDelete {
        /// The timeline.
        timeline: Id,
        /// The clips.
        clips: Vec<Id>,
        /// Whether the clips linked to them go too.
        #[serde(default = "yes")]
        linked: bool,
    },
    /// Cut frames `from` to `to` out of every track that is not locked, or of the tracks given,
    /// and close the gap.
    RippleDeleteRange {
        /// The timeline.
        timeline: Id,
        /// The first frame cut out.
        from: i64,
        /// The frame after the last cut out.
        to: i64,
        /// The tracks to cut, or all those that are not locked.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tracks: Option<Vec<Id>>,
    },
    /// Close the gap that `at` is in on `track`, moving what is after it on every track that is
    /// not locked.
    CloseGap {
        /// The timeline.
        timeline: Id,
        /// The track of the gap.
        track: Id,
        /// A frame of the gap.
        at: i64,
    },
    /// Cut the clips across `at` in two, on the tracks given or on every track that is not
    /// locked, and the clips linked to them.
    Blade {
        /// The timeline.
        timeline: Id,
        /// The frame that the second halves start at.
        at: i64,
        /// The tracks to cut, or all those that are not locked.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tracks: Option<Vec<Id>>,
        /// Whether the clips linked to those cut are cut too.
        #[serde(default = "yes")]
        linked: bool,
    },
    /// Move clips by `delta` frames, and those of the kind of `to_track` to tracks as far from
    /// theirs as `to_track` is from the track of the first clip. Refused with `no_room` where
    /// there is something in the way, unless `overwrite`, which overwrites it.
    Move {
        /// The timeline.
        timeline: Id,
        /// The clips.
        clips: Vec<Id>,
        /// How many frames later they go.
        delta: i64,
        /// The track that the first clip goes to.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to_track: Option<Id>,
        /// Whether the clips overwrite what is in their way.
        #[serde(default)]
        overwrite: bool,
        /// Whether the clips linked to them move too.
        #[serde(default = "yes")]
        linked: bool,
    },
    /// Take clips away, close the gaps, and insert them at `at`, a frame of the timeline as it
    /// was before.
    InsertMove {
        /// The timeline.
        timeline: Id,
        /// The clips.
        clips: Vec<Id>,
        /// Where the first of them goes in.
        at: i64,
        /// Whether the clips linked to them move too.
        #[serde(default = "yes")]
        linked: bool,
    },
    /// Move an edge of a clip by `delta` frames, later if it is above 0. Without `ripple`, the
    /// clip's start or end moves and nothing else does; with it, the clip's start stays where
    /// it is and what is after the clip moves on every track that is not locked.
    Trim {
        /// The timeline.
        timeline: Id,
        /// The clip.
        clip: Id,
        /// The edge.
        edge: Edge,
        /// How many frames later the edge goes.
        delta: i64,
        /// Whether what follows moves with it.
        #[serde(default)]
        ripple: bool,
        /// Whether the clips linked to it are trimmed too.
        #[serde(default = "yes")]
        linked: bool,
    },
    /// Move the cut between two clips that meet by `delta` frames: the end of the one and the
    /// start of the other.
    Roll {
        /// The timeline.
        timeline: Id,
        /// The clip that ends at the cut.
        left: Id,
        /// The clip that starts at it.
        right: Id,
        /// How many frames later the cut goes.
        delta: i64,
        /// Whether the cuts between the clips linked to them roll too.
        #[serde(default = "yes")]
        linked: bool,
    },
    /// Show `delta` frames later of the source of a clip, which stays where it is.
    Slip {
        /// The timeline.
        timeline: Id,
        /// The clip.
        clip: Id,
        /// How many frames later in its source it begins.
        delta: i64,
        /// Whether the clips linked to it slip too.
        #[serde(default = "yes")]
        linked: bool,
    },
    /// Move a clip by `delta` frames between its neighbours, which give and take the frames.
    Slide {
        /// The timeline.
        timeline: Id,
        /// The clip.
        clip: Id,
        /// How many frames later it goes.
        delta: i64,
        /// Whether the clips linked to it slide too.
        #[serde(default = "yes")]
        linked: bool,
    },
    /// Put a transition at the cut between two clips that meet.
    AddTransition {
        /// The timeline.
        timeline: Id,
        /// The clip that ends at the cut.
        left: Id,
        /// The clip that starts at it.
        right: Id,
        /// How the one gives way to the other: a dissolve on a track of picture and a crossfade
        /// on one of sound if it is left out.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<TransitionKind>,
        /// How many frames it lasts.
        length: i64,
        /// Where it lies about the cut.
        #[serde(default = "center")]
        align: Alignment,
    },
    /// Take a transition away.
    RemoveTransition {
        /// The timeline.
        timeline: Id,
        /// The transition.
        id: Id,
    },
    /// Set what is given of a transition.
    SetTransition {
        /// The timeline.
        timeline: Id,
        /// The transition.
        id: Id,
        /// How the one clip gives way to the other.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<TransitionKind>,
        /// How many frames it lasts.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        length: Option<i64>,
        /// Where it lies about the cut.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        align: Option<Alignment>,
    },
    /// Play a clip at another speed, showing the same stretch of its source: so it lasts more or
    /// fewer frames. With `ripple`, what is after it moves with its end.
    SetSpeed {
        /// The timeline.
        timeline: Id,
        /// The clip.
        clip: Id,
        /// The speed.
        speed: Speed,
        /// Whether it plays backward, or as it does if this is left out.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reverse: Option<bool>,
        /// Whether what follows moves with its end.
        #[serde(default)]
        ripple: bool,
        /// Whether the clips linked to it change speed too.
        #[serde(default = "yes")]
        linked: bool,
    },
    /// Put a subtitle on a track of subtitles.
    AddCue {
        /// The timeline.
        timeline: Id,
        /// The track.
        track: Id,
        /// The frame that it is first shown at.
        start: i64,
        /// How many frames it is shown for.
        length: i64,
        /// What it says.
        text: String,
    },
    /// Set what is given of a subtitle.
    SetCue {
        /// The timeline.
        timeline: Id,
        /// The subtitle.
        id: Id,
        /// The frame that it is first shown at.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start: Option<i64>,
        /// How many frames it is shown for.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        length: Option<i64>,
        /// What it says.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    /// Take a subtitle away.
    RemoveCue {
        /// The timeline.
        timeline: Id,
        /// The subtitle.
        id: Id,
    },
    /// Put these subtitles on a track of subtitles in place of all that are on it.
    CuesFrom {
        /// The timeline.
        timeline: Id,
        /// The track.
        track: Id,
        /// The subtitles.
        items: Vec<CueItem>,
    },
    /// Add a track with nothing on it.
    AddTrack {
        /// The timeline.
        timeline: Id,
        /// What it holds.
        kind: TrackKind,
        /// Its place among the tracks of its kind, the first at the bottom; after the last of
        /// them if it is left out.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
        /// The name it is shown by, or its id if it is left out.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        /// The language of its subtitles, for a track of subtitles.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        language: Option<String>,
    },
    /// Take a track away with everything on it.
    RemoveTrack {
        /// The timeline.
        timeline: Id,
        /// The track.
        track: Id,
    },
}

/// What an edit puts on a timeline.
///
/// Media is put from `from` to `to`, in ticks of the scale of its duration: from its mark in, or
/// its start, to its mark out, or its end, where they are left out; or for `length` frames from
/// `from`, as for an edit of three points that takes its length from the timeline. Its picture
/// goes on `video_track` and its sound on `audio_track`, linked to each other if both go: a
/// track that is left out is the first track of its kind, and one that is `null` is not used. A
/// title or a colour lasts `length` frames, and goes on a track of picture.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Material {
    /// Media.
    Media {
        /// The media.
        media: Id,
        /// Where in it the clip begins.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<Time>,
        /// Where in it the clip ends.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<Time>,
        /// How many frames the clip lasts, if not as far as `to`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        length: Option<i64>,
    },
    /// A title.
    Title {
        /// What it says.
        text: String,
        /// How it is drawn.
        style: TitleStyle,
        /// How many frames it lasts.
        length: i64,
    },
    /// A colour.
    Color {
        /// The colour.
        color: Rgba,
        /// How many frames it lasts.
        length: i64,
    },
}

/// An edge of a clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Edge {
    /// Its start.
    In,
    /// Its end.
    Out,
}

/// A subtitle of [`Edit::CuesFrom`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CueItem {
    /// The frame that it is first shown at.
    pub start: i64,
    /// How many frames it is shown for.
    pub length: i64,
    /// What it says.
    pub text: String,
}

fn yes() -> bool {
    true
}

fn center() -> Alignment {
    Alignment::Center
}

/// Read a field that may be `null` as well as left out: one that is there is read as `Some`.
#[allow(clippy::option_option, reason = "left out and null are told apart")]
fn nullable<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(deserializer).map(Some)
}

/// Why an edit is refused.
///
/// It is written in JSON as [`OpError`] is: the reason in `reason`, the message in `message`,
/// and the fields of the reason beside them. An operation that the edit was worked out into and
/// that was refused is written as that operation's error.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    /// An operation of the edit was refused, or the edit names what is not there or asks for
    /// what no operation could do.
    #[error(transparent)]
    Op(#[from] OpError),
    /// The edit would change a track that is locked.
    #[error("track {track} is locked")]
    Locked {
        /// The track.
        track: Id,
    },
    /// A ripple would take frames away that something on a track has, which the edit does not
    /// take away.
    #[error("{clip} on track {track} is in the way of the ripple")]
    RippleBlocked {
        /// The track.
        track: Id,
        /// The clip or the subtitle in the way.
        clip: Id,
    },
    /// Something on a track is in the way of a clip.
    #[error("there is no room on track {track}")]
    NoRoom {
        /// The track.
        track: Id,
    },
    /// A clip needs more of its source than there is.
    #[error("clip {clip} needs {need} frames of its source, and has {have}")]
    NoHandles {
        /// The clip, or the media for a clip that is to be made of it.
        clip: Id,
        /// How many frames it needs.
        need: i64,
        /// How many it has.
        have: i64,
        /// For a transition, the longest that it could be.
        longest: Option<i64>,
    },
    /// The edit asks for no frame, or no change: a `delta` of 0, a range that ends where it
    /// starts, or a gap that is not there.
    #[error("the edit covers no frame")]
    EmptyRange,
    /// There is no clip at the frame.
    #[error("there is no clip at frame {frame}")]
    NothingAt {
        /// The track that was looked at, if one was.
        track: Option<Id>,
        /// The frame.
        frame: i64,
    },
    /// There is no track for what is to be put on the timeline.
    #[error("there is no {} track to put it on", kind_name(*.kind))]
    NoTrack {
        /// The kind of track that it needs.
        kind: TrackKind,
    },
}

fn kind_name(kind: TrackKind) -> &'static str {
    match kind {
        TrackKind::Video => "video",
        TrackKind::Audio => "audio",
        TrackKind::Subtitle => "subtitle",
    }
}

impl Serialize for EditError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut value = match self {
            Self::Op(error) => return error.serialize(serializer),
            Self::Locked { track } => serde_json::json!({ "reason": "locked", "track": track }),
            Self::RippleBlocked { track, clip } => {
                serde_json::json!({ "reason": "ripple_blocked", "track": track, "clip": clip })
            }
            Self::NoRoom { track } => serde_json::json!({ "reason": "no_room", "track": track }),
            Self::NoHandles {
                clip,
                need,
                have,
                longest,
            } => {
                let mut value = serde_json::json!({
                    "reason": "no_handles", "clip": clip, "need": need, "have": have,
                });
                if let Some(longest) = longest {
                    value["longest"] = (*longest).into();
                }
                value
            }
            Self::EmptyRange => serde_json::json!({ "reason": "empty_range" }),
            Self::NothingAt { track, frame } => {
                serde_json::json!({ "reason": "nothing_at", "track": track, "frame": frame })
            }
            Self::NoTrack { kind } => serde_json::json!({ "reason": "no_track", "kind": kind }),
        };
        value["message"] = self.to_string().into();
        value.serialize(serializer)
    }
}

/// An output intentionally created by an edit, excluding fragments made to clear its way.
/// This is transaction metadata, not a project operation or a saved part of history.
#[derive(Clone, Debug, PartialEq)]
pub enum Created {
    /// A source placed on a track; picture precedes sound when both are placed.
    Clip {
        /// Its id.
        id: Id,
        /// The kind of track it was placed on.
        kind: TrackKind,
        /// Its newly created link group, if picture and sound were placed together.
        link: Option<Id>,
    },
    /// A transition added at a cut.
    Transition(Id),
    /// A subtitle added to a track.
    Cue(Id),
}

/// An edit as it was worked out and applied.
#[derive(Clone, Debug, PartialEq)]
pub struct Expansion {
    /// The operations, as they were done, in order.
    pub done: Vec<Op>,
    /// Their inverses, in the same order; applied last first, they take the edit back.
    pub inverse: Vec<Op>,
    /// Whether the edit went less far than it was asked to, as far as it could.
    pub clamped: bool,
    /// The edit's outputs, separate from incidental splits and repaired transitions.
    pub created: Vec<Created>,
}

/// Work an edit out against a project as it is and apply it, all of it or none.
///
/// # Errors
/// The edit is refused, and the project is left exactly as it was.
pub fn expand(project: &mut Project, edit: Edit) -> Result<Expansion, EditError> {
    let before = project.clone();
    let expansion = run(project, edit);
    if expansion.is_err() {
        *project = before;
    }
    expansion
}

/// Work an edit out and apply it. A project that it is refused on may be left changed.
#[allow(clippy::too_many_lines, reason = "an arm for each edit")]
pub(crate) fn run(project: &mut Project, edit: Edit) -> Result<Expansion, EditError> {
    input::check(&edit)?;
    let timeline = timeline_of(&edit).clone();
    let mut work = Work::new(project, &timeline)?;
    match edit {
        Edit::Insert {
            source,
            at,
            video_track,
            audio_track,
            ..
        } => place::insert(&mut work, &source, at, &[video_track, audio_track]),
        Edit::Overwrite {
            source,
            at,
            video_track,
            audio_track,
            ..
        } => place::overwrite(&mut work, &source, at, &[video_track, audio_track]),
        Edit::Replace {
            source,
            at,
            video_track,
            audio_track,
            ..
        } => place::replace(&mut work, &source, at, &[video_track, audio_track]),
        Edit::PlaceOnTop { source, at, .. } => place::place_on_top(&mut work, &source, at),
        Edit::Append {
            source,
            video_track,
            audio_track,
            ..
        } => place::append(&mut work, &source, &[video_track, audio_track]),
        Edit::SmartInsert {
            source,
            at,
            video_track,
            audio_track,
            ..
        } => place::smart_insert(&mut work, &source, at, &[video_track, audio_track]),
        Edit::RippleOverwrite {
            source,
            at,
            video_track,
            audio_track,
            ..
        } => place::ripple_overwrite(&mut work, &source, at, &[video_track, audio_track]),
        Edit::Lift { clips, linked, .. } => cut::lift(&mut work, &clips, linked),
        Edit::RippleDelete { clips, linked, .. } => cut::ripple_delete(&mut work, &clips, linked),
        Edit::RippleDeleteRange {
            from, to, tracks, ..
        } => cut::ripple_delete_range(&mut work, from, to, tracks.as_deref()),
        Edit::CloseGap { track, at, .. } => cut::close_gap(&mut work, &track, at),
        Edit::Blade {
            at, tracks, linked, ..
        } => cut::blade(&mut work, at, tracks.as_deref(), linked),
        Edit::Move {
            clips,
            delta,
            to_track,
            overwrite,
            linked,
            ..
        } => cut::move_clips(
            &mut work,
            &clips,
            delta,
            to_track.as_deref(),
            overwrite,
            linked,
        ),
        Edit::InsertMove {
            clips, at, linked, ..
        } => cut::insert_move(&mut work, &clips, at, linked),
        Edit::Trim {
            clip,
            edge,
            delta,
            ripple,
            linked,
            ..
        } => trim::trim(&mut work, &clip, edge, delta, ripple, linked),
        Edit::Roll {
            left,
            right,
            delta,
            linked,
            ..
        } => trim::roll(&mut work, &left, &right, delta, linked),
        Edit::Slip {
            clip,
            delta,
            linked,
            ..
        } => trim::slip(&mut work, &clip, delta, linked),
        Edit::Slide {
            clip,
            delta,
            linked,
            ..
        } => trim::slide(&mut work, &clip, delta, linked),
        Edit::AddTransition {
            left,
            right,
            kind,
            length,
            align,
            ..
        } => misc::add_transition(&mut work, &left, &right, kind, length, align),
        Edit::RemoveTransition { id, .. } => misc::remove_transition(&mut work, &id),
        Edit::SetTransition {
            id,
            kind,
            length,
            align,
            ..
        } => misc::set_transition(&mut work, &id, kind, length, align),
        Edit::SetSpeed {
            clip,
            speed,
            reverse,
            ripple,
            linked,
            ..
        } => misc::set_speed(&mut work, &clip, speed, reverse, ripple, linked),
        Edit::AddCue {
            track,
            start,
            length,
            text,
            ..
        } => misc::add_cue(&mut work, &track, start, length, text),
        Edit::SetCue {
            id,
            start,
            length,
            text,
            ..
        } => misc::set_cue(&mut work, &id, start, length, text),
        Edit::RemoveCue { id, .. } => misc::remove_cue(&mut work, &id),
        Edit::CuesFrom { track, items, .. } => misc::cues_from(&mut work, &track, items),
        Edit::AddTrack {
            kind,
            index,
            name,
            language,
            ..
        } => misc::add_track(&mut work, kind, index, name, language),
        Edit::RemoveTrack { track, .. } => misc::remove_track(&mut work, &track),
    }?;
    work.finish()
}

/// The timeline that an edit is of.
#[must_use]
pub fn timeline_of(edit: &Edit) -> &Id {
    match edit {
        Edit::Insert { timeline, .. }
        | Edit::Overwrite { timeline, .. }
        | Edit::Replace { timeline, .. }
        | Edit::PlaceOnTop { timeline, .. }
        | Edit::Append { timeline, .. }
        | Edit::SmartInsert { timeline, .. }
        | Edit::RippleOverwrite { timeline, .. }
        | Edit::Lift { timeline, .. }
        | Edit::RippleDelete { timeline, .. }
        | Edit::RippleDeleteRange { timeline, .. }
        | Edit::CloseGap { timeline, .. }
        | Edit::Blade { timeline, .. }
        | Edit::Move { timeline, .. }
        | Edit::InsertMove { timeline, .. }
        | Edit::Trim { timeline, .. }
        | Edit::Roll { timeline, .. }
        | Edit::Slip { timeline, .. }
        | Edit::Slide { timeline, .. }
        | Edit::AddTransition { timeline, .. }
        | Edit::RemoveTransition { timeline, .. }
        | Edit::SetTransition { timeline, .. }
        | Edit::SetSpeed { timeline, .. }
        | Edit::AddCue { timeline, .. }
        | Edit::SetCue { timeline, .. }
        | Edit::RemoveCue { timeline, .. }
        | Edit::CuesFrom { timeline, .. }
        | Edit::AddTrack { timeline, .. }
        | Edit::RemoveTrack { timeline, .. } => timeline,
    }
}

/// A change of a transaction: an operation, or an edit, which is told by its `edit` field.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
#[allow(
    clippy::large_enum_variant,
    reason = "a change is read, applied and let go"
)]
pub enum Change {
    /// An operation.
    Op(Op),
    /// An edit.
    Edit(Edit),
}

impl<'de> Deserialize<'de> for Change {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let edit = value.as_object().is_some_and(|o| o.contains_key("edit"));
        let change = if edit {
            serde_json::from_value(value).map(Change::Edit)
        } else {
            serde_json::from_value(value).map(Change::Op)
        };
        change.map_err(serde::de::Error::custom)
    }
}

impl From<Op> for Change {
    fn from(op: Op) -> Self {
        Change::Op(op)
    }
}

impl From<Edit> for Change {
    fn from(edit: Edit) -> Self {
        Change::Edit(edit)
    }
}

/// Apply a change to a project: an operation as [`apply_done`] does, or an edit as [`run`]
/// does. A project that it is refused on may be left changed by an edit.
pub(crate) fn run_change(project: &mut Project, change: Change) -> Result<Expansion, ChangeError> {
    match change {
        Change::Op(op) => {
            let (done, inverse) = apply_done(project, op).map_err(ChangeError::Op)?;
            // Setting media to what it is already changes nothing, and makes no step.
            if matches!(done, Op::SetMedia { .. }) && done == inverse {
                return Ok(Expansion {
                    done: vec![],
                    inverse: vec![],
                    clamped: false,
                    created: vec![],
                });
            }
            Ok(Expansion {
                done: vec![done],
                inverse: vec![inverse],
                clamped: false,
                created: vec![],
            })
        }
        Change::Edit(edit) => run(project, edit).map_err(ChangeError::Edit),
    }
}

/// Why a change was refused.
pub(crate) enum ChangeError {
    Op(OpError),
    Edit(EditError),
}

#[cfg(test)]
pub(crate) mod fixture;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edit_is_read_by_its_name_beside_operations() {
        let change: Change = serde_json::from_value(serde_json::json!({
            "edit": "remove_cue", "timeline": "t1", "id": "q6",
        }))
        .unwrap();
        assert_eq!(
            change,
            Change::Edit(Edit::RemoveCue {
                timeline: "t1".into(),
                id: "q6".into(),
            })
        );
        let change: Change = serde_json::from_value(serde_json::json!({
            "op": "remove_cue", "timeline": "t1", "id": "q6",
        }))
        .unwrap();
        assert!(matches!(change, Change::Op(Op::RemoveCue { .. })));
        let written = serde_json::to_value(&change).unwrap();
        assert_eq!(written["op"], "remove_cue");
    }

    #[test]
    fn an_edit_with_a_field_that_is_not_known_is_refused_with_its_name() {
        let error = serde_json::from_value::<Change>(serde_json::json!({
            "edit": "remove_cue", "timeline": "t1", "id": "q6", "colour": "red",
        }))
        .unwrap_err();
        assert!(error.to_string().contains("colour"), "{error}");
        let error = serde_json::from_value::<Change>(serde_json::json!({
            "op": "cues_from", "timeline": "t1", "track": "S1", "items": [],
        }))
        .unwrap_err();
        assert!(error.to_string().contains("cues_from"), "{error}");
    }

    #[test]
    fn an_error_of_an_edit_is_written_with_its_reason_its_fields_and_its_message() {
        let error = EditError::RippleBlocked {
            track: "V2".into(),
            clip: "c7".into(),
        };
        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            serde_json::json!({
                "reason": "ripple_blocked", "track": "V2", "clip": "c7",
                "message": "c7 on track V2 is in the way of the ripple",
            })
        );
        let error = EditError::NoHandles {
            clip: "c1".into(),
            need: 12,
            have: 5,
            longest: Some(10),
        };
        let written = serde_json::to_value(&error).unwrap();
        assert_eq!(written["reason"], "no_handles");
        assert_eq!(written["longest"], 10);
        let error = EditError::from(OpError::Overlap { with: "c1".into() });
        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            serde_json::to_value(OpError::Overlap { with: "c1".into() }).unwrap()
        );
        let written = serde_json::to_value(EditError::NothingAt {
            track: None,
            frame: 4,
        })
        .unwrap();
        assert_eq!(written["track"], serde_json::Value::Null);
        assert_eq!(written["reason"], "nothing_at");
    }

    #[test]
    fn a_refused_edit_leaves_the_project_as_it_was() {
        let mut project = fixture::project();
        let before = project.clone();
        let edit = Edit::RemoveCue {
            timeline: "t1".into(),
            id: "q99".into(),
        };
        let error = expand(&mut project, edit).unwrap_err();
        assert!(matches!(error, EditError::Op(OpError::Missing { .. })));
        assert_eq!(project, before);
    }

    #[test]
    fn an_edit_of_a_timeline_that_is_not_there_is_refused() {
        let edit = Edit::RemoveTrack {
            timeline: "t9".into(),
            track: "V1".into(),
        };
        let error = expand(&mut fixture::project(), edit).unwrap_err();
        assert_eq!(
            error,
            EditError::Op(OpError::Missing {
                what: "timeline",
                id: "t9".into()
            })
        );
    }
}
