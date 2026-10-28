//! The operations on a project, each of which gives back the operation that takes it back.
//!
//! An operation is applied whole or not at all: one that is refused leaves the project exactly as
//! it was, `next_id` too. One that is applied gives back its inverse, which carries every id that
//! the engine gave and the place that each thing was taken from, so that applying the inverse of
//! the inverse makes the same things again, with the same ids, in the same places.
//! [`apply_done`] gives back that inverse of the inverse too, as the operation as it was done.
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
//! - A bin, media, a timeline, a clip, a transition and a subtitle are given a letter for their
//!   kind (`b`, `m`, `t`, `c`, `x` or `q`) and the number in `next_id`, which then goes up by
//!   one. A number whose id is held already by a thing of the same kind is passed over, as it may
//!   be in a project that was written elsewhere. A clip that is added with an empty `link` is put
//!   in a new group of linked clips, whose id is made so with `g`.
//! - A track is given `V` if it holds picture, `A` if it holds sound and `S` if it holds
//!   subtitles, and a number one above the highest number of the ids of that letter among the
//!   tracks of its timeline, whatever they hold: beside `V1` and `V3`, a new track of picture is
//!   `V4`, and in a timeline with no such id it is `V1`. The id of a track names its place among
//!   the tracks of its kind, as an editor shows it, and is not kept from reuse as the others are:
//!   tracks do not use `next_id`, so a track that is taken away and added again without an id may
//!   be given the id that it had, and so may a new track after `V2` is taken away.
//! - The tracks and the things on them inside a timeline or a track that is added are given
//!   theirs after it, in the order of their lists as they are given: the first track, then its
//!   clips (each clip, then its group), its transitions and its subtitles, then the next track.
//!   The things of each track are then put in order.
//!
//! A thing that is added with an id keeps it, unless a thing of its kind holds it already: bins,
//! media and timelines among those of the project, a track among those of its timeline, and a
//! clip, a transition or a subtitle among all those of the project. Then the operation is
//! refused. An id of the shape of those that the engine makes, one of the letters `b`, `m`, `t`,
//! `c`, `x`, `q` and `g` and a number below [`LARGEST`] written without a leading 0, is refused
//! too if its number is `next_id` or above: the engine may make it later. One with a lower number
//! is kept if no thing of its kind holds it: the engine never makes that number again, so the id
//! is the thing's alone. It may be one that the engine made and the inverse of an operation
//! carries back, or one that a client chose itself. The group of a clip may be one that other
//! clips are in. An id that starts with `$` is refused, in an operation and in a project that is
//! read: such ids are the names that live for one transaction, which the engine puts real ids in
//! place of.
//!
//! `next_id` never goes back: not when an operation is refused, and not when one is taken back.
//! So an id that the engine makes is never made again, and is never given to another thing,
//! unless a client gives it to one itself.
//!
//! # What is checked
//!
//! A timeline has no 0 in its rate, a picture of an even number of pixels from 16 to 16384 each
//! way, and sound of 44100, 48000 or 96000 samples a second. A track of picture or of sound holds
//! clips and transitions and no language; a track of subtitles holds subtitles, and a language
//! that is empty or a short tag of BCP 47.
//!
//! A clip lasts at least one frame, from a frame of the timeline no earlier than 0, and on its
//! track it covers no frame that another clip covers. It plays at a speed from 1/100 to 100. Its
//! transform is of finite numbers, with a scale above 0 and at most 16 and an opacity from 0 to
//! 1; its gain is from -96 to 24 decibels and its pan from -1 to 1; and it fades in and out over
//! no more frames than it lasts. A clip of a title or a colour is on a track of picture, and its
//! title has at most 1000 characters, a size from 1 to 4096 and a finite place. A clip of media
//! names media that is in the project and has a stream for the kind of its track, which is not
//! one of subtitles. What it shows of its media lies inside the media: from where it begins in
//! it, for the ticks that its frames last at the rate of the timeline and its speed (see
//! [`crate::span`]), and no further than the duration of the media. Where it begins counts ticks
//! of the scale of that duration, which is never 0. A clip moves only between tracks of the same
//! kind, within its timeline.
//!
//! A transition is between two clips of its track that meet, the one ending where the other
//! starts, and a cut has at most one. It lasts at least a frame, and is a crossfade on a track of
//! sound and one of the others on a track of picture. It covers no more of a clip than the clip
//! has beside the transition at its other end. The frames that it shows of its clips beyond their
//! ends are in their media: of a transition of `length` frames whose middle is at the cut, the
//! left clip goes on for `ceil(length / 2)` frames after its end and the right one begins
//! `floor(length / 2)` frames before its start; one that starts or ends at the cut takes them all
//! from the left or the right clip. A title and a colour go on as long as they are wanted. A clip
//! that a transition is at is not moved, trimmed or taken away: the transition is taken away
//! first, so that taking the operation back never loses it.
//!
//! A subtitle lasts at least one frame, from a frame no earlier than 0, says something in at most
//! 1000 characters, and on its track covers no frame that another one covers.
//!
//! A bin is in a bin that is there, or at the root, and is never inside itself. Media is in a bin
//! that is there, or at the root. Media does not last less than no time, and its picture has no 0
//! in its rate. Its marks count ticks of the scale of its duration, are within it, and the mark in
//! is before the mark out. The path of a proxy is absolute, on Windows or on Unix, and the media
//! it is set on is there. A thing is not put past the end of its list.
//!
//! The ticks that the frames of a clip last are rounded down, as [`Rate::ticks_of`] counts
//! them. So at a rate whose frames do not last a whole number of ticks, the last frame of a clip
//! that reaches the end of its media may end less than one tick past that end; whoever reads the
//! media for it stops at the end.
//!
//! A project that is read is checked as the operations check what they add, by
//! [`Project::validate`], so that the inverse of an operation on it is never refused.
//!
//! Every integer that an operation writes into a project is within [`LARGEST`] of 0, since
//! JavaScript reads the project too. Media that clips use cannot be taken away; a track or a
//! timeline can, with everything on it, and its inverse adds all of it back.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize, Serializer};

use crate::project::{
    Bin, Clip, Cue, Id, Media, Project, Proxy, Source, Timeline, Track, TrackKind, Transition,
    TransitionKind,
};
use crate::span;
use crate::time::{Rate, Time};

/// The largest integer that JavaScript holds exactly, 2^53 - 1. No integer that an operation
/// writes into a project is further than this from 0.
pub const LARGEST: i64 = (1 << 53) - 1;

/// [`LARGEST`] as a count.
const LARGEST_COUNT: u64 = LARGEST.unsigned_abs();

/// The letters of the ids that the engine makes with `next_id`.
const NUMBERED: [char; 7] = ['b', 'm', 't', 'c', 'x', 'q', 'g'];

/// The most characters of the text of a title or a subtitle.
const LONGEST_TEXT: usize = 1000;

/// A change to a project.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    /// Add a timeline to the project, with its tracks and what is on them.
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
    /// Add a track to a timeline, with what is on it.
    AddTrack {
        /// The timeline.
        timeline: Id,
        /// The track. It and the things on it may have empty ids.
        track: Track,
        /// Where it goes in the list of tracks of the timeline.
        #[serde(default)]
        index: Option<usize>,
    },
    /// Take a track away from a timeline, with what is on it.
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
        /// The clip, with an empty id for the engine to give it one, and an empty `link` for the
        /// engine to put it in a new group.
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
        /// Where it begins in its media, or none to leave it where it is. A clip of a title or
        /// a colour has none.
        #[serde(default)]
        source_in: Option<Time>,
        /// The frame that it starts at.
        start: i64,
        /// How many frames it lasts.
        length: i64,
    },
    /// Set the proxy of media, or take it away (`None`). The engine sets it once it has made
    /// one; a client may too.
    SetProxy {
        /// The media.
        media: Id,
        /// Its proxy, or none.
        proxy: Option<Proxy>,
    },
}

/// Why an operation is refused.
///
/// It is written in JSON as an object that names the reason in `reason`, gives the message in
/// `message`, and has the fields of the reason beside them:
/// `{"reason": "overlap", "with": "c1", "message": "the clip covers frames that clip c1 covers"}`.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum OpError {
    /// A thing that the operation names is not there. `what` is `bin`, `media`, `timeline`,
    /// `track`, `clip`, `transition` or `cue`.
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
    /// A thing is to be on a track of a kind that does not hold it. `what` is `clip` (on a track
    /// of subtitles), `title` or `color` (on a track of sound), `cue` (on a track of picture or
    /// sound), `language` (of a track of picture or sound), `transition` (on a track of
    /// subtitles), or the kind of a transition (`dissolve`, `dip_black` or `dip_white` on a
    /// track of sound, `crossfade` on one of picture).
    #[error("a {what} cannot be on a {} track", name_of(*.kind))]
    WrongTrack {
        /// What is to be on it.
        what: &'static str,
        /// What the track holds.
        kind: TrackKind,
    },
    /// A clip lasts no frame, or shows what is not in its media. `why` is `length` if it lasts
    /// no frame, `before_start` if it begins before the start of the media, `past_end` if it
    /// goes on past the end of the media, and `handles` if a transition at it would show frames
    /// beyond the media. It is `marks` if a mark of the media is outside it, or the mark in is
    /// not before the mark out.
    #[error("{}", source_message(.media, .why))]
    OutOfSource {
        /// The media.
        media: Id,
        /// Which way the clip is out of it.
        why: &'static str,
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
    /// A subtitle is shown on a frame that another subtitle on its track is shown on.
    #[error("the subtitle is shown on frames that subtitle {with} is shown on")]
    CueOverlap {
        /// The other subtitle.
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
    /// A setting is not one that it may be. `what` is `width`, `height`, `sample_rate`,
    /// `start`, `length`, `speed`, `transform`, `gain_db`, `pan`, `fade`, `text`, `size`, `position` or
    /// `language`.
    #[error("{}", range_message(.what))]
    OutOfRange {
        /// The setting.
        what: &'static str,
    },
    /// A rate of frames has a 0 in it.
    #[error("a rate of frames has a 0 in it")]
    ZeroRate,
    /// Media lasts less than no time.
    #[error("the media lasts {ticks} ticks, less than no time")]
    NegativeDuration {
        /// How many ticks it lasts.
        ticks: i64,
    },
    /// An id that a thing is given is of the shape of the ids that the engine makes, with a
    /// number that the engine has not made yet: the engine may make it later.
    #[error("the id {id} is of the shape of the ids that the engine makes, with a number that it has not made yet")]
    Reserved {
        /// The id.
        id: Id,
    },
    /// The engine has made every id that it can, since `next_id` cannot go past [`LARGEST`].
    #[error("the engine has made all the ids it can")]
    IdsExhausted,
    /// An id that a thing is given starts with `$`. Only the names that live for one transaction
    /// start so (see the engine's protocol): no thing keeps one, since no later transaction
    /// could name it.
    #[error("the id {id} starts with $, as only a name of a transaction does")]
    Dollar {
        /// The id.
        id: Id,
    },
    /// A thing of a project that is read has an empty id. A thing that an operation adds with an
    /// empty id is given one, so an operation never gives this.
    #[error("the thing has no id")]
    NoId,
    /// The path of a proxy is not absolute (`C:\…`, `\\server\…` or `/…`), or holds a NUL.
    #[error("the path {path} of a proxy is not absolute")]
    NotAbsolute {
        /// The path.
        path: String,
    },
    /// The path of a proxy is not one where the engine keeps proxies: a file
    /// `<id>-<12 hex digits>.mov` in the directory of the proxies of the project. Only the engine
    /// gives this, which alone knows that directory; the operations do not.
    #[error("the path {path} is not that of a proxy of the project")]
    NotAProxyPath {
        /// The path.
        path: String,
    },
    /// A clip that a transition is at is to be moved, trimmed or taken away. The transition is
    /// to be taken away first.
    #[error("transition {transition} is at the clip, and has to be taken away first")]
    HasTransition {
        /// The transition.
        transition: Id,
    },
    /// The clips of a transition do not meet: the left one does not end where the right one
    /// starts.
    #[error("clip {left} does not end where clip {right} starts")]
    NotAdjacent {
        /// The clip that is to end at the cut.
        left: Id,
        /// The clip that is to start at the cut.
        right: Id,
    },
    /// The cut that a transition is at has another one.
    #[error("the cut has transition {with} already")]
    CutTaken {
        /// The other transition.
        with: Id,
    },
    /// A transition covers more of a clip than the clip has beside the transition at its other
    /// end.
    #[error("transition {transition} covers more of clip {clip} than there is room for")]
    TooLong {
        /// The transition.
        transition: Id,
        /// The clip.
        clip: Id,
    },
    /// The source of a clip is to be set to one of another kind, or to other media, or where a
    /// clip of a title or a colour begins in its media is to be set. A clip of media is given
    /// other media by taking it away and putting a new one in its place.
    #[error("the source of clip {clip} cannot be changed so")]
    Source {
        /// The clip.
        clip: Id,
    },
    /// A bin is to be put inside itself, or inside a bin that is inside it.
    #[error("bin {bin} would be inside itself")]
    Cycle {
        /// The bin.
        bin: Id,
    },
}

/// Why a project that is read is not one that the operations can work on: the thing of it that
/// is wrong, and what is wrong with it.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{} {}", thing_of(.what, .id), .error)]
pub struct Invalid {
    /// The kind of the thing: `project`, `bin`, `media`, `timeline`, `track`, `clip`,
    /// `transition` or `cue`.
    pub what: &'static str,
    /// Its id, which is empty for the project.
    pub id: Id,
    /// What is wrong with it.
    pub error: OpError,
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
    WrongTrack { what: &'static str, kind: TrackKind },
    OutOfSource { media: Id, why: &'static str },
    Scale { scale: u32, media_scale: u32 },
    Overlap { with: Id },
    CueOverlap { with: Id },
    OutOfTimeline { start: i64 },
    InUse { id: Id, by: Id },
    TooLarge { what: &'static str },
    OutOfRange { what: &'static str },
    ZeroRate,
    NegativeDuration { ticks: i64 },
    Reserved { id: Id },
    IdsExhausted,
    Dollar { id: Id },
    NoId,
    NotAbsolute { path: String },
    NotAProxyPath { path: String },
    HasTransition { transition: Id },
    NotAdjacent { left: Id, right: Id },
    CutTaken { with: Id },
    TooLong { transition: Id, clip: Id },
    Source { clip: Id },
    Cycle { bin: Id },
}

/// How [`Invalid`] names the thing that is wrong.
fn thing_of(what: &str, id: &str) -> String {
    if what == "project" {
        "the project:".into()
    } else if id.is_empty() {
        format!("a {what} of the project:")
    } else {
        format!("{what} {id}:")
    }
}

/// The message of [`OpError::OutOfSource`].
fn source_message(media: &str, why: &str) -> String {
    match why {
        "length" => "the clip lasts no frame".into(),
        "before_start" => format!("the clip begins before the start of media {media}"),
        "handles" => format!("a transition would show frames beyond media {media}"),
        "marks" => format!(
            "the marks of media {media} are not within it, with the mark in before the mark out"
        ),
        _ => format!("the clip goes on past the end of media {media}"),
    }
}

/// The message of [`OpError::OutOfRange`].
fn range_message(what: &str) -> String {
    match what {
        "width" | "height" => format!("the {what} is not an even number from 16 to 16384"),
        "sample_rate" => "the sample rate is not 44100, 48000 or 96000".into(),
        "start" => "it starts before the timeline does".into(),
        "length" => "it lasts no frame".into(),
        "speed" => "the speed is not from 1/100 to 100".into(),
        "transform" => {
            "the transform is not of finite numbers, with a scale above 0 and at most 16 and an \
             opacity from 0 to 1"
                .into()
        }
        "gain_db" => "the gain is not from -96 to 24 decibels".into(),
        "pan" => "the pan is not from -1 to 1".into(),
        "fade" => "the fades are not of frames that the clip has".into(),
        "text" => format!("the text is empty or longer than {LONGEST_TEXT} characters"),
        "size" => "the size of the title is not from 1 to 4096".into(),
        "position" => "the place of the title is not of finite numbers".into(),
        "language" => "the language is not a short tag of BCP 47".into(),
        _ => format!("the {what} is out of its range"),
    }
}

/// What a track of a kind is called in a message.
fn name_of(kind: TrackKind) -> &'static str {
    match kind {
        TrackKind::Video => "video",
        TrackKind::Audio => "audio",
        TrackKind::Subtitle => "subtitle",
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
        Op::SetProxy { media, proxy } => set_proxy(project, &media, proxy),
    }
}

/// Apply an operation to a project, and give back the operation as it was done and the one that
/// takes it back, in that order.
///
/// The operation as it was done is the inverse of the inverse, read from the project rather
/// than made by applying the inverse. An operation that adds a thing is given back with the
/// thing as it was put in, with every id that the engine gave it, and with the place in its list
/// that it was put at. An operation that only names things that are there already is given back
/// as it is. Applied to the project as it was before, the operation as it was done makes the
/// same things again, with the same ids, in the same places, and makes no id of its own.
///
/// # Errors
/// The operation is refused, and the project is left exactly as it was.
///
/// # Panics
/// Only if a thing that an operation added is not in the project after it, which would be a
/// bug of this module.
pub fn apply_done(project: &mut Project, op: Op) -> Result<(Op, Op), OpError> {
    // What an operation adds is read back from the project, so only the others are kept.
    let kept = match op {
        Op::AddMedia { .. }
        | Op::AddTimeline { .. }
        | Op::AddTrack { .. }
        | Op::InsertClip { .. } => None,
        _ => Some(op.clone()),
    };
    let inverse = apply(project, op)?;
    let done = kept.unwrap_or_else(|| {
        added(project, &inverse).expect("an operation that adds a thing leaves it in the project")
    });
    Ok((done, inverse))
}

/// The operation that adds back what `inverse` takes away, as the thing is in the project.
fn added(project: &Project, inverse: &Op) -> Option<Op> {
    match inverse {
        Op::RemoveMedia { id } => {
            let at = project.media.iter().position(|m| m.id == *id)?;
            Some(Op::AddMedia {
                media: project.media[at].clone(),
                index: Some(at),
            })
        }
        Op::RemoveTimeline { id } => {
            let at = timeline_at(project, id).ok()?;
            Some(Op::AddTimeline {
                timeline: project.timelines[at].clone(),
                index: Some(at),
            })
        }
        Op::RemoveTrack { timeline, id } => {
            let tracks = &project.timelines[timeline_at(project, timeline).ok()?].tracks;
            let at = tracks.iter().position(|r| r.id == *id)?;
            Some(Op::AddTrack {
                timeline: timeline.clone(),
                track: tracks[at].clone(),
                index: Some(at),
            })
        }
        Op::RemoveClip { timeline, id } => {
            let t = &project.timelines[timeline_at(project, timeline).ok()?];
            let (r, at) = clip_at(t, id).ok()?;
            Some(Op::InsertClip {
                timeline: timeline.clone(),
                track: t.tracks[r].id.clone(),
                clip: t.tracks[r].clips[at].clone(),
            })
        }
        _ => None,
    }
}

fn add_media(project: &mut Project, mut media: Media, index: Option<usize>) -> Result<Op, OpError> {
    let at = place(index, project.media.len())?;
    check_media(project, &media)?;
    let held = |id: &str| project.media.iter().any(|m| m.id == id);
    let mut counter = Counter::of(project);
    if media.id.is_empty() {
        media.id = counter.make('m', held)?;
    } else if held(&media.id) {
        return Err(taken("media", &media.id));
    } else {
        counter.check(&media.id)?;
    }
    let id = media.id.clone();
    project.media.insert(at, media);
    project.next_id = counter.next;
    Ok(Op::RemoveMedia { id })
}

fn set_proxy(project: &mut Project, id: &str, proxy: Option<Proxy>) -> Result<Op, OpError> {
    let media = project
        .media
        .iter_mut()
        .find(|m| m.id == id)
        .ok_or_else(|| missing("media", id))?;
    if let Some(proxy) = &proxy {
        check_proxy(proxy)?;
    }
    let before = std::mem::replace(&mut media.proxy, proxy);
    Ok(Op::SetProxy {
        media: id.into(),
        proxy: before,
    })
}

fn remove_media(project: &mut Project, id: &str) -> Result<Op, OpError> {
    let at = project
        .media
        .iter()
        .position(|m| m.id == id)
        .ok_or_else(|| missing("media", id))?;
    if let Some(clip) = clips_of(project).find(|c| c.media().is_some_and(|m| m == id)) {
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
    check_settings(&timeline)?;
    let held = |id: &str| project.timelines.iter().any(|t| t.id == id);
    let mut counter = Counter::of(project);
    if !timeline.id.is_empty() {
        if held(&timeline.id) {
            return Err(taken("timeline", &timeline.id));
        }
        counter.check(&timeline.id)?;
    }
    let mut claims = claim(project, &counter, &[], &timeline.tracks)?;
    if timeline.id.is_empty() {
        timeline.id = counter.make('t', held)?;
    }
    name(&mut counter, &mut claims, &[], &mut timeline.tracks)?;
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
    let mut claims = claim(project, &counter, &timeline.tracks, &added)?;
    name(&mut counter, &mut claims, &timeline.tracks, &mut added)?;
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
        counter.check(&clip.id)?;
    }
    link(&mut counter, &mut clip.link, &links_of(project))?;
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
    unattached(track, id)?;
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
    unattached(&timeline.tracks[from], id)?;
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
    source_in: Option<Time>,
    start: i64,
    length: i64,
) -> Result<Op, OpError> {
    let t = timeline_at(project, timeline_id)?;
    let timeline = &project.timelines[t];
    let (r, at) = clip_at(timeline, id)?;
    let track = &timeline.tracks[r];
    unattached(track, id)?;
    let old = &track.clips[at];
    let mut trimmed = Clip {
        start,
        length,
        ..old.clone()
    };
    if let Some(time) = source_in {
        match &mut trimmed.source {
            Source::Media { source_in, .. } => *source_in = time,
            _ => return Err(OpError::Source { clip: id.into() }),
        }
    }
    check_clip(project, timeline.rate, track.kind, &trimmed)?;
    check_room(&track.clips, &trimmed)?;
    let inverse = Op::TrimClip {
        timeline: timeline_id.into(),
        id: id.into(),
        source_in: old.source_in(),
        start: old.start,
        length: old.length,
    };
    let clips = &mut project.timelines[t].tracks[r].clips;
    clips.remove(at);
    put(clips, trimmed);
    Ok(inverse)
}

/// Check that no transition of a track is at one of its clips.
fn unattached(track: &Track, clip: &str) -> Result<(), OpError> {
    match track
        .transitions
        .iter()
        .find(|x| x.left == clip || x.right == clip)
    {
        Some(x) => Err(OpError::HasTransition {
            transition: x.id.clone(),
        }),
        None => Ok(()),
    }
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

    /// Check an id that a thing is given: one that starts with `$` is refused, and one of the
    /// shape of the ids that the engine makes is refused if its number is one that the engine
    /// has not made yet.
    fn check(&self, id: &str) -> Result<(), OpError> {
        if id.starts_with('$') {
            return Err(OpError::Dollar { id: id.into() });
        }
        match engine_number(id) {
            Some(number) if number >= self.next => Err(OpError::Reserved { id: id.into() }),
            _ => Ok(()),
        }
    }

    /// Make an id of `letter` and the next number with which it is not `held`.
    fn make(&mut self, letter: char, held: impl Fn(&str) -> bool) -> Result<Id, OpError> {
        loop {
            let number = self.next;
            if number >= LARGEST_COUNT {
                return Err(OpError::IdsExhausted);
            }
            self.next = number + 1;
            let id = format!("{letter}{number}");
            if !held(&id) {
                return Ok(id);
            }
        }
    }
}

/// Check the group that a clip is put in, or make a new one for it where it is empty. `groups`
/// are the groups that there are.
fn link(counter: &mut Counter, link: &mut Option<Id>, groups: &HashSet<Id>) -> Result<(), OpError> {
    match link {
        Some(group) if group.is_empty() => {
            *group = counter.make('g', |id| groups.contains(id))?;
            Ok(())
        }
        Some(group) => counter.check(group),
        None => Ok(()),
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

/// The number of an id that the engine could make: one of the letters of [`NUMBERED`], and a
/// number below [`LARGEST`] written without a leading 0.
fn engine_number(id: &str) -> Option<u64> {
    NUMBERED
        .iter()
        .find_map(|&letter| number_in(id, letter))
        .filter(|&number| number < LARGEST_COUNT)
}

/// The ids of the things of a project and of the tracks that are added to it, that the things
/// that are added are named apart from.
struct Claims {
    clips: HashSet<Id>,
    transitions: HashSet<Id>,
    cues: HashSet<Id>,
    groups: HashSet<Id>,
}

/// Check the ids that tracks to be added to a timeline, which has `existing` tracks, are given,
/// and those of the things on them. Gives the ids of the things of the project with those of
/// the things of `tracks`.
fn claim(
    project: &Project,
    counter: &Counter,
    existing: &[Track],
    tracks: &[Track],
) -> Result<Claims, OpError> {
    let mut track_ids: HashSet<&str> = existing.iter().map(|r| r.id.as_str()).collect();
    let mut claims = Claims {
        clips: clips_of(project).map(|c| c.id.clone()).collect(),
        transitions: tracks_of(project)
            .flat_map(|r| &r.transitions)
            .map(|x| x.id.clone())
            .collect(),
        cues: tracks_of(project)
            .flat_map(|r| &r.cues)
            .map(|q| q.id.clone())
            .collect(),
        groups: links_of(project),
    };
    let fresh = |what, id: &Id, held: &mut HashSet<Id>| {
        if id.is_empty() {
            return Ok(());
        }
        if !held.insert(id.clone()) {
            return Err(taken(what, id));
        }
        counter.check(id)
    };
    for track in tracks {
        if !track.id.is_empty() {
            if !track_ids.insert(&track.id) {
                return Err(taken("track", &track.id));
            }
            counter.check(&track.id)?;
        }
        for clip in &track.clips {
            fresh("clip", &clip.id, &mut claims.clips)?;
            if let Some(group) = clip.link.as_ref().filter(|group| !group.is_empty()) {
                counter.check(group)?;
                claims.groups.insert(group.clone());
            }
        }
        for transition in &track.transitions {
            fresh("transition", &transition.id, &mut claims.transitions)?;
        }
        for cue in &track.cues {
            fresh("cue", &cue.id, &mut claims.cues)?;
        }
    }
    Ok(claims)
}

/// Give an id to each track of `tracks` and each thing on them that has none, and a new group to
/// each clip with an empty one, in the order of the lists: a track, then its clips, its
/// transitions and its subtitles, then the next track.
fn name(
    counter: &mut Counter,
    claims: &mut Claims,
    existing: &[Track],
    tracks: &mut [Track],
) -> Result<(), OpError> {
    fn give(
        counter: &mut Counter,
        letter: char,
        id: &mut Id,
        held: &mut HashSet<Id>,
    ) -> Result<(), OpError> {
        if id.is_empty() {
            *id = counter.make(letter, |id| held.contains(id))?;
            held.insert(id.clone());
        }
        Ok(())
    }
    for at in 0..tracks.len() {
        if tracks[at].id.is_empty() {
            let ids = existing.iter().chain(tracks.iter()).map(|r| r.id.as_str());
            tracks[at].id = track_id(tracks[at].kind, ids)?;
        }
        let track = &mut tracks[at];
        for clip in &mut track.clips {
            give(counter, 'c', &mut clip.id, &mut claims.clips)?;
            if let Some(group) = &mut clip.link {
                give(counter, 'g', group, &mut claims.groups)?;
            }
        }
        for transition in &mut track.transitions {
            give(counter, 'x', &mut transition.id, &mut claims.transitions)?;
        }
        for cue in &mut track.cues {
            give(counter, 'q', &mut cue.id, &mut claims.cues)?;
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
        TrackKind::Subtitle => 'S',
    };
    let highest = ids
        .filter_map(|id| number_in(id, letter))
        .max()
        .unwrap_or(0);
    let number = highest.checked_add(1).ok_or(OpError::IdsExhausted)?;
    Ok(format!("{letter}{number}"))
}

/// Check tracks to be added to a timeline of `rate`, and put the things of each in order.
fn check_tracks(project: &Project, rate: Rate, tracks: &mut [Track]) -> Result<(), OpError> {
    for track in tracks {
        order(track);
        check_track(project, rate, track)?;
    }
    Ok(())
}

/// Put the clips of a track in the order of their start, its transitions in the order of their
/// cuts, and its subtitles in the order of their start.
fn order(track: &mut Track) {
    track.clips.sort_by_key(|c| c.start);
    let clips = &track.clips;
    track.transitions.sort_by_key(|x| cut_of(clips, x));
    track.cues.sort_by_key(|q| q.start);
}

/// The frame of the cut that a transition is at: the start of its right clip, or the end of
/// the timeline if there is none.
fn cut_of(clips: &[Clip], transition: &Transition) -> i64 {
    clips
        .iter()
        .find(|c| c.id == transition.right)
        .map_or(i64::MAX, |c| c.start)
}

/// Check a track whose things are in order, on a timeline of `rate`: what it holds, its clips,
/// its transitions and its subtitles.
fn check_track(project: &Project, rate: Rate, track: &Track) -> Result<(), OpError> {
    let kind = track.kind;
    let wrong = |what| Err(OpError::WrongTrack { what, kind });
    if kind == TrackKind::Subtitle {
        if !track.clips.is_empty() {
            return wrong("clip");
        }
        if !track.transitions.is_empty() {
            return wrong("transition");
        }
        if !is_language(&track.language) {
            return Err(range("language"));
        }
    } else {
        if !track.cues.is_empty() {
            return wrong("cue");
        }
        if !track.language.is_empty() {
            return wrong("language");
        }
    }
    for clip in &track.clips {
        check_clip(project, rate, kind, clip)?;
    }
    if let Some(pair) = track.clips.windows(2).find(|p| overlap(&p[0], &p[1])) {
        return Err(OpError::Overlap {
            with: pair[0].id.clone(),
        });
    }
    check_transitions(project, rate, track)?;
    check_cues(track)
}

/// Whether a tag is empty or a short tag of BCP 47: a language of 2 or 3 letters, and then
/// subtags of 1 to 8 letters and digits, each after a `-`, in at most 35 characters.
fn is_language(tag: &str) -> bool {
    if tag.is_empty() {
        return true;
    }
    let mut parts = tag.split('-');
    let language = parts.next().unwrap_or_default();
    tag.len() <= 35
        && (2..=3).contains(&language.len())
        && language.bytes().all(|b| b.is_ascii_alphabetic())
        && parts.all(|part| {
            (1..=8).contains(&part.len()) && part.bytes().all(|b| b.is_ascii_alphanumeric())
        })
}

/// Check a clip as it would be on a track of `kind`, in a timeline of `rate`, but for the
/// transitions at it.
fn check_clip(project: &Project, rate: Rate, kind: TrackKind, clip: &Clip) -> Result<(), OpError> {
    check_rate(rate)?;
    let media = match &clip.source {
        Source::Media { media, .. } => Some(
            project
                .media
                .iter()
                .find(|m| m.id == *media)
                .ok_or_else(|| missing("media", media))?,
        ),
        _ => None,
    };
    within("start", clip.start.into())?;
    within("length", clip.length.into())?;
    within("end", end_of(clip))?;
    within("fade_in", clip.fade_in.into())?;
    within("fade_out", clip.fade_out.into())?;
    if let Some(time) = clip.source_in() {
        within("source_in", time.ticks.into())?;
    }
    if clip.start < 0 {
        return Err(OpError::OutOfTimeline { start: clip.start });
    }
    if let (Some(media), Some(time)) = (media, clip.source_in()) {
        let media_scale = media.facts.duration.scale;
        if time.scale == 0 || time.scale != media_scale {
            return Err(OpError::Scale {
                scale: time.scale,
                media_scale,
            });
        }
    }
    if clip.length <= 0 {
        return Err(match media {
            Some(media) => OpError::OutOfSource {
                media: media.id.clone(),
                why: "length",
            },
            None => range("length"),
        });
    }
    check_settings_of(clip)?;
    match (&clip.source, media) {
        (Source::Media { .. }, Some(media)) => {
            if let Err(why) = in_source(media, rate, clip, 0, 0) {
                return Err(OpError::OutOfSource {
                    media: media.id.clone(),
                    why,
                });
            }
            let has = match kind {
                TrackKind::Video => media.facts.video.is_some(),
                TrackKind::Audio => media.facts.audio.is_some(),
                TrackKind::Subtitle => return Err(OpError::WrongTrack { what: "clip", kind }),
            };
            if !has {
                return Err(OpError::NoStream {
                    media: media.id.clone(),
                    kind,
                });
            }
        }
        (Source::Title { text, style }, _) => {
            if kind != TrackKind::Video {
                return Err(OpError::WrongTrack {
                    what: "title",
                    kind,
                });
            }
            if text.chars().count() > LONGEST_TEXT {
                return Err(range("text"));
            }
            if !(1..=4096).contains(&style.size) {
                return Err(range("size"));
            }
            if !style.x.is_finite() || !style.y.is_finite() {
                return Err(range("position"));
            }
        }
        (Source::Color { .. }, _) => {
            if kind != TrackKind::Video {
                return Err(OpError::WrongTrack {
                    what: "color",
                    kind,
                });
            }
        }
        (Source::Media { .. }, None) => unreachable!("the media of a clip was found above"),
    }
    Ok(())
}

/// Check how a clip that lasts at least a frame is played: its speed, transform, gain, pan,
/// fades and group.
fn check_settings_of(clip: &Clip) -> Result<(), OpError> {
    let speed = clip.speed;
    let (num, den) = (u64::from(speed.num), u64::from(speed.den));
    if num * 100 < den || num > den * 100 {
        return Err(range("speed"));
    }
    let t = clip.transform;
    let finite = [t.x, t.y, t.scale, t.rotation, t.opacity]
        .iter()
        .all(|v| v.is_finite());
    if !(finite && t.scale > 0.0 && t.scale <= 16.0 && (0.0..=1.0).contains(&t.opacity)) {
        return Err(range("transform"));
    }
    if !(-96.0..=24.0).contains(&clip.gain_db) {
        return Err(range("gain_db"));
    }
    if !(-1.0..=1.0).contains(&clip.pan) {
        return Err(range("pan"));
    }
    let fades = i128::from(clip.fade_in) + i128::from(clip.fade_out);
    if clip.fade_in < 0 || clip.fade_out < 0 || fades > i128::from(clip.length) {
        return Err(range("fade"));
    }
    match &clip.link {
        Some(group) if group.is_empty() => Err(OpError::NoId),
        Some(group) if group.starts_with('$') => Err(OpError::Dollar { id: group.clone() }),
        _ => Ok(()),
    }
}

/// Check that what a clip of `media` shows lies inside the media, with `before` frames more
/// before its start and `after` more after its end, as a transition shows them; or say which
/// way it is out of it, as [`OpError::OutOfSource`] does.
fn in_source(
    media: &Media,
    rate: Rate,
    clip: &Clip,
    before: i64,
    after: i64,
) -> Result<(), &'static str> {
    let Some(source_in) = clip.source_in() else {
        return Ok(());
    };
    let offset = |frames: i64| {
        i128::from(span::ticks_of_scaled(
            frames,
            clip.speed,
            rate,
            source_in.scale,
        ))
    };
    // A clip that plays backward shows what is before its start at its end.
    let (before, after) = if clip.reverse {
        (after, before)
    } else {
        (before, after)
    };
    let first = i128::from(source_in.ticks) + offset(-before);
    let end = i128::from(source_in.ticks) + offset(clip.length.saturating_add(after));
    if first < 0 {
        Err("before_start")
    } else if end > i128::from(media.facts.duration.ticks) {
        Err("past_end")
    } else {
        Ok(())
    }
}

/// Check the transitions of a track whose clips are in order and checked, on a timeline of
/// `rate`.
fn check_transitions(project: &Project, rate: Rate, track: &Track) -> Result<(), OpError> {
    // For each clip, the frames that the transitions at it need beyond its start and end, and
    // those that they cover of it there.
    let mut beyond: HashMap<&str, (i64, i64)> = HashMap::new();
    let mut covered: HashMap<&str, (i64, i64)> = HashMap::new();
    let mut cuts: HashMap<&str, &str> = HashMap::new();
    for transition in &track.transitions {
        within("length", transition.length.into())?;
        if transition.length < 1 {
            return Err(range("length"));
        }
        let fits = match transition.kind {
            TransitionKind::Crossfade => track.kind == TrackKind::Audio,
            _ => track.kind == TrackKind::Video,
        };
        if !fits {
            return Err(OpError::WrongTrack {
                what: kind_name(transition.kind),
                kind: track.kind,
            });
        }
        let find = |id: &str| {
            track
                .clips
                .iter()
                .find(|c| c.id == id)
                .ok_or_else(|| missing("clip", id))
        };
        let (left, right) = (find(&transition.left)?, find(&transition.right)?);
        if left.end() != right.start {
            return Err(OpError::NotAdjacent {
                left: left.id.clone(),
                right: right.id.clone(),
            });
        }
        if let Some(with) = cuts.insert(&left.id, &transition.id) {
            return Err(OpError::CutTaken { with: with.into() });
        }
        let (after_left, before_right) = transition.handles();
        beyond.entry(&left.id).or_default().1 = after_left;
        beyond.entry(&right.id).or_default().0 = before_right;
        covered.entry(&left.id).or_default().1 = before_right;
        covered.entry(&right.id).or_default().0 = after_left;
    }
    for transition in &track.transitions {
        for id in [&transition.left, &transition.right] {
            let (start, end) = covered[id.as_str()];
            let clip = track.clips.iter().find(|c| c.id == *id);
            if clip.is_some_and(|c| i128::from(start) + i128::from(end) > i128::from(c.length)) {
                return Err(OpError::TooLong {
                    transition: transition.id.clone(),
                    clip: id.clone(),
                });
            }
        }
    }
    for clip in &track.clips {
        let (Some(&(before, after)), Some(media)) = (beyond.get(clip.id.as_str()), clip.media())
        else {
            continue;
        };
        let media = project
            .media
            .iter()
            .find(|m| m.id == *media)
            .ok_or_else(|| missing("media", media))?;
        if in_source(media, rate, clip, before, after).is_err() {
            return Err(OpError::OutOfSource {
                media: media.id.clone(),
                why: "handles",
            });
        }
    }
    Ok(())
}

/// What a kind of transition is called in an error.
fn kind_name(kind: TransitionKind) -> &'static str {
    match kind {
        TransitionKind::Dissolve => "dissolve",
        TransitionKind::DipBlack => "dip_black",
        TransitionKind::DipWhite => "dip_white",
        TransitionKind::Crossfade => "crossfade",
    }
}

/// Check the subtitles of a track, which are in the order of their start.
fn check_cues(track: &Track) -> Result<(), OpError> {
    for cue in &track.cues {
        check_cue(cue)?;
    }
    if let Some(pair) = track
        .cues
        .windows(2)
        .find(|p| p[1].start < p[0].start.saturating_add(p[0].length))
    {
        return Err(OpError::CueOverlap {
            with: pair[0].id.clone(),
        });
    }
    Ok(())
}

/// Check a subtitle as it would be on a track.
fn check_cue(cue: &Cue) -> Result<(), OpError> {
    within("start", cue.start.into())?;
    within("length", cue.length.into())?;
    within("end", i128::from(cue.start) + i128::from(cue.length))?;
    if cue.start < 0 {
        return Err(range("start"));
    }
    if cue.length < 1 {
        return Err(range("length"));
    }
    if cue.text.is_empty() || cue.text.chars().count() > LONGEST_TEXT {
        return Err(range("text"));
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

/// Check media as it would be in a project.
fn check_media(project: &Project, media: &Media) -> Result<(), OpError> {
    within("size", media.identity.size.into())?;
    within("modified_us", media.identity.modified_us.into())?;
    let duration = media.facts.duration;
    within("duration", duration.ticks.into())?;
    if duration.scale == 0 {
        return Err(OpError::Scale {
            scale: 0,
            media_scale: 0,
        });
    }
    if duration.ticks < 0 {
        return Err(OpError::NegativeDuration {
            ticks: duration.ticks,
        });
    }
    if let Some(video) = &media.facts.video {
        check_rate(video.rate)?;
    }
    if let Some(proxy) = &media.proxy {
        check_proxy(proxy)?;
    }
    if let Some(bin) = &media.bin {
        bin_at(project, bin)?;
    }
    check_marks(media)
}

/// Check the marks of media: each counts ticks of the scale of its duration and is within it,
/// and the mark in is before the mark out.
fn check_marks(media: &Media) -> Result<(), OpError> {
    let duration = media.facts.duration;
    let out = || OpError::OutOfSource {
        media: media.id.clone(),
        why: "marks",
    };
    for mark in [media.mark_in, media.mark_out].into_iter().flatten() {
        within("mark", mark.ticks.into())?;
        if mark.scale != duration.scale {
            return Err(OpError::Scale {
                scale: mark.scale,
                media_scale: duration.scale,
            });
        }
        if mark.ticks < 0 || mark.ticks > duration.ticks {
            return Err(out());
        }
    }
    match (media.mark_in, media.mark_out) {
        (Some(mark_in), Some(mark_out)) if mark_in.ticks >= mark_out.ticks => Err(out()),
        _ => Ok(()),
    }
}

/// Check the bins of a project: each is in a bin that is there, or at the root, and none is
/// inside itself.
fn check_bins(bins: &[Bin]) -> Result<(), Invalid> {
    for bin in bins {
        let wrong = |error| Invalid {
            what: "bin",
            id: bin.id.clone(),
            error,
        };
        let mut parent = bin.parent.as_ref();
        // A bin is no deeper than there are bins.
        for _ in 0..=bins.len() {
            let Some(id) = parent else {
                break;
            };
            if *id == bin.id {
                return Err(wrong(OpError::Cycle {
                    bin: bin.id.clone(),
                }));
            }
            parent = bins
                .iter()
                .find(|b| b.id == *id)
                .ok_or_else(|| wrong(missing("bin", id)))?
                .parent
                .as_ref();
        }
    }
    Ok(())
}

/// Check a proxy as it would be in a project: its path is absolute, on any system, so that a
/// project written on one is read on another.
fn check_proxy(proxy: &Proxy) -> Result<(), OpError> {
    if absolute(&proxy.path) {
        Ok(())
    } else {
        Err(OpError::NotAbsolute {
            path: proxy.path.clone(),
        })
    }
}

/// Whether a path is absolute on Windows or on Unix, whichever this is: `C:\…` or `C:/…`,
/// `\\server\…`, or `/…`; and holds no NUL, which no system takes.
fn absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    let drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    let rooted = path.starts_with('/') || path.starts_with(r"\\");
    !path.contains('\0') && (drive || rooted)
}

fn check_rate(rate: Rate) -> Result<(), OpError> {
    if rate.num == 0 || rate.den == 0 {
        Err(OpError::ZeroRate)
    } else {
        Ok(())
    }
}

/// Check the settings of a timeline: its rate, the size of its picture and the rate of its sound.
fn check_settings(timeline: &Timeline) -> Result<(), OpError> {
    check_rate(timeline.rate)?;
    let size = |pixels: u32| (16..=16384).contains(&pixels) && pixels.is_multiple_of(2);
    if !size(timeline.width) {
        return Err(range("width"));
    }
    if !size(timeline.height) {
        return Err(range("height"));
    }
    if ![44100, 48000, 96000].contains(&timeline.sample_rate) {
        return Err(range("sample_rate"));
    }
    Ok(())
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

fn range(what: &'static str) -> OpError {
    OpError::OutOfRange { what }
}

fn bin_at(project: &Project, id: &str) -> Result<usize, OpError> {
    project
        .bins
        .iter()
        .position(|b| b.id == id)
        .ok_or_else(|| missing("bin", id))
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

/// Every track of a project.
fn tracks_of(project: &Project) -> impl Iterator<Item = &Track> {
    project.timelines.iter().flat_map(|t| &t.tracks)
}

/// Every clip of a project.
fn clips_of(project: &Project) -> impl Iterator<Item = &Clip> {
    tracks_of(project).flat_map(|r| &r.clips)
}

/// Every group of linked clips of a project.
fn links_of(project: &Project) -> HashSet<Id> {
    clips_of(project).filter_map(|c| c.link.clone()).collect()
}

impl Project {
    /// Check that a project that was read is one that the operations could have made, so that
    /// they can work on it: the inverse of an operation on it is never refused.
    ///
    /// Everything is checked that an operation checks of what it adds: each clip as
    /// [`Op::InsertClip`] checks it, each media as [`Op::AddMedia`] does, the settings of each
    /// timeline, each track with its transitions and subtitles, the bins, and every integer
    /// against [`LARGEST`]. Every thing has an id, and no two things of a kind share one: bins,
    /// media and timelines in the project, tracks in their timeline, and clips, transitions and
    /// subtitles in the project.
    ///
    /// Two things are put right rather than refused. The things of each track are put in order:
    /// its clips and subtitles in the order of their start, and its transitions in the order of
    /// their cuts. `next_id` is raised above the number of every id of the project that the
    /// engine could have made, so that the engine never makes such an id again.
    ///
    /// # Errors
    /// A thing of the project is not as an operation would have made it. The things of the
    /// tracks may have been put in order by then, and nothing else is changed.
    pub fn validate(&mut self) -> Result<(), Invalid> {
        for timeline in &mut self.timelines {
            for track in &mut timeline.tracks {
                order(track);
            }
        }
        self.check_all()?;
        let bins = self.bins.iter().map(|b| &b.id);
        let media = self.media.iter().map(|m| &m.id);
        let timelines = self.timelines.iter().map(|t| &t.id);
        let tracks = self.timelines.iter().flat_map(|t| &t.tracks);
        let track_ids = tracks.clone().map(|r| &r.id);
        let clips = tracks.clone().flat_map(|r| &r.clips);
        let clip_ids = clips.clone().map(|c| &c.id);
        let groups = clips.filter_map(|c| c.link.as_ref());
        let transitions = tracks.clone().flat_map(|r| &r.transitions).map(|x| &x.id);
        let cues = tracks.flat_map(|r| &r.cues).map(|q| &q.id);
        let highest = bins
            .chain(media)
            .chain(timelines)
            .chain(track_ids)
            .chain(clip_ids)
            .chain(groups)
            .chain(transitions)
            .chain(cues)
            .filter_map(|id| engine_number(id))
            .max();
        if let Some(highest) = highest {
            // Below LARGEST, so that the sum is no further from 0 than LARGEST.
            self.next_id = self.next_id.max(highest + 1);
        }
        Ok(())
    }

    /// Check a project whose things are in order. See [`Project::validate`].
    fn check_all(&self) -> Result<(), Invalid> {
        fn wrong<'a>(what: &'static str, id: &'a str) -> impl Fn(OpError) -> Invalid + 'a {
            move |error| Invalid {
                what,
                id: id.into(),
                error,
            }
        }
        if self.next_id > LARGEST_COUNT {
            return Err(wrong("project", "")(OpError::TooLarge { what: "next_id" }));
        }
        let mut bin_ids = HashSet::new();
        for bin in &self.bins {
            named("bin", &bin.id, &mut bin_ids)?;
        }
        check_bins(&self.bins)?;
        let mut media_ids = HashSet::new();
        for media in &self.media {
            named("media", &media.id, &mut media_ids)?;
            check_media(self, media).map_err(wrong("media", &media.id))?;
        }
        let mut timeline_ids = HashSet::new();
        let mut clip_ids = HashSet::new();
        let mut transition_ids = HashSet::new();
        let mut cue_ids = HashSet::new();
        for timeline in &self.timelines {
            named("timeline", &timeline.id, &mut timeline_ids)?;
            check_settings(timeline).map_err(wrong("timeline", &timeline.id))?;
            let mut track_ids = HashSet::new();
            for track in &timeline.tracks {
                named("track", &track.id, &mut track_ids)?;
                for clip in &track.clips {
                    named("clip", &clip.id, &mut clip_ids)?;
                    check_clip(self, timeline.rate, track.kind, clip)
                        .map_err(wrong("clip", &clip.id))?;
                }
                if let Some(pair) = track.clips.windows(2).find(|p| overlap(&p[0], &p[1])) {
                    let with = pair[0].id.clone();
                    return Err(wrong("clip", &pair[1].id)(OpError::Overlap { with }));
                }
                for transition in &track.transitions {
                    named("transition", &transition.id, &mut transition_ids)?;
                }
                for cue in &track.cues {
                    named("cue", &cue.id, &mut cue_ids)?;
                }
                check_track(self, timeline.rate, track).map_err(wrong("track", &track.id))?;
            }
        }
        Ok(())
    }
}

/// Check that a thing of a project that is read has an id, and one that no thing among `ids`
/// has; and add it to them.
fn named<'a>(what: &'static str, id: &'a str, ids: &mut HashSet<&'a str>) -> Result<(), Invalid> {
    let error = if id.is_empty() {
        OpError::NoId
    } else if id.starts_with('$') {
        OpError::Dollar { id: id.into() }
    } else if !ids.insert(id) {
        taken(what, id)
    } else {
        return Ok(());
    };
    Err(Invalid {
        what,
        id: id.into(),
        error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{Alignment, ReadError, Rgba, TextAlign, TitleStyle, Transform};
    use crate::time::Speed;

    /// The scale of the media of the sample: m1 lasts 153600 ticks of it, 250 frames at 25/1.
    const SCALE: u32 = 15360;

    // The sample: timeline t1 at 25/1, with c1 on V1 and c2 on A1, each on frames [0, 100) and
    // cut from m1, which lasts 10 seconds; next_id is 3.

    fn clip(id: &str, media: &str, start: i64, length: i64) -> Clip {
        let source_in = Time {
            ticks: 0,
            scale: SCALE,
        };
        Clip::of_media(id, media, source_in, start, length)
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
        Track::new(id, kind).with_clips(clips)
    }

    fn timeline(id: &str, tracks: Vec<Track>) -> Timeline {
        Timeline {
            tracks,
            ..Timeline::new(id, "Timeline 2", Rate { num: 25, den: 1 })
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
            source_in: Some(Time {
                ticks: 0,
                scale: SCALE,
            }),
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
            source_in: Some(Time {
                ticks: 614,
                scale: SCALE,
            }),
            start: 10,
            length: 200,
        };
        let inverse = apply(&mut project, op).unwrap();
        let trimmed = &project.timelines[0].tracks[1].clips[0];
        assert_eq!(
            (
                trimmed.source_in().unwrap().ticks,
                trimmed.start,
                trimmed.length
            ),
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
    fn an_id_that_the_engine_may_make_later_is_reserved() {
        let reserved = |id: &str| OpError::Reserved { id: id.into() };
        // next_id is 3: c3 is the next clip that the engine makes, and c9 one after it.
        for id in ["c3", "c9", "c9007199254740990"] {
            assert_eq!(refused(insert("V1", clip(id, "m1", 100, 10))), reserved(id));
        }
        let mut late = media("m3", true, true);
        assert_eq!(
            refused(Op::AddMedia {
                media: late.clone(),
                index: None
            }),
            reserved("m3")
        );
        late.id = "t3".into();
        let error = refused(Op::AddMedia {
            media: late,
            index: None,
        });
        assert_eq!(error, reserved("t3"), "the letter of another kind too");
        let nested = Op::AddTimeline {
            timeline: timeline(
                "",
                vec![track("", TrackKind::Video, vec![clip("c4", "m1", 0, 10)])],
            ),
            index: None,
        };
        assert_eq!(refused(nested), reserved("c4"));

        // An id that the engine made before, which its inverse carries, may be given again, and
        // ids of other shapes, or with numbers that the engine never makes, are free.
        let mut project = sample_with(vec![Op::RemoveClip {
            timeline: "t1".into(),
            id: "c1".into(),
        }]);
        for id in ["c1", "c099", "y500", "C7", "c9007199254740991"] {
            let start = i64::try_from(project.timelines[0].tracks[0].clips.len()).unwrap() * 10;
            apply(&mut project, insert("V1", clip(id, "m1", start, 10))).unwrap();
        }
        assert_eq!(project.next_id, 3);
    }

    #[test]
    fn an_id_that_starts_with_a_dollar_is_never_given_nor_read() {
        let dollar = |id: &str| OpError::Dollar { id: id.into() };
        assert_eq!(
            refused(insert("V1", clip("$c", "m1", 100, 10))),
            dollar("$c")
        );
        assert_eq!(
            refused(Op::AddMedia {
                media: media("$m", true, true),
                index: None,
            }),
            dollar("$m")
        );
        let nested = |timeline_id: &str, track_id: &str, clip_id: &str| Op::AddTimeline {
            timeline: timeline(
                timeline_id,
                vec![track(
                    track_id,
                    TrackKind::Video,
                    vec![clip(clip_id, "m1", 0, 10)],
                )],
            ),
            index: None,
        };
        assert_eq!(refused(nested("$t", "", "")), dollar("$t"));
        assert_eq!(refused(nested("", "$v", "")), dollar("$v"));
        assert_eq!(refused(nested("", "", "$c")), dollar("$c"));
        assert_eq!(
            refused(Op::AddTrack {
                timeline: "t1".into(),
                track: track("$a", TrackKind::Audio, vec![]),
                index: None,
            }),
            dollar("$a")
        );
        assert_eq!(
            dollar("$x").to_string(),
            "the id $x starts with $, as only a name of a transaction does"
        );

        // Nor is a project read that has one.
        for (pointer, what) in [
            ("/media/0/id", "media"),
            ("/timelines/0/id", "timeline"),
            ("/timelines/0/tracks/1/id", "track"),
            ("/timelines/0/tracks/0/clips/0/id", "clip"),
        ] {
            let mut value = serde_json::to_value(Project::sample()).unwrap();
            *value.pointer_mut(pointer).unwrap() = "$x".into();
            let error = Project::from_value(value).unwrap_err();
            let ReadError::Invalid(invalid) = error else {
                panic!("{pointer}: {error}")
            };
            assert_eq!(
                (invalid.what, invalid.error),
                (what, dollar("$x")),
                "{pointer}"
            );
        }
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
            assert_eq!(
                error,
                OpError::OutOfSource {
                    media: "m1".into(),
                    why: "length"
                }
            );
            assert_eq!(error.to_string(), "the clip lasts no frame");
        }
    }

    #[test]
    fn a_clip_shows_only_what_is_in_its_media() {
        // m1 lasts 250 frames: all of them fit, and one more does not.
        let mut project = Project::sample();
        apply(&mut project, insert("V1", clip("", "m1", 100, 250))).unwrap();
        let past_end = OpError::OutOfSource {
            media: "m1".into(),
            why: "past_end",
        };
        let error = refused(insert("V1", clip("", "m1", 100, 251)));
        assert_eq!(error, past_end);
        assert_eq!(
            error.to_string(),
            "the clip goes on past the end of media m1"
        );
        // From the last frame, one frame fits and two do not.
        let mut late = clip("", "m1", 100, 1);
        late.source_in_mut().unwrap().ticks = Rate { num: 25, den: 1 }.ticks_of(249, SCALE);
        apply(&mut Project::sample(), insert("V1", late.clone())).unwrap();
        late.length = 2;
        assert_eq!(refused(insert("V1", late)), past_end);
        let mut early = clip("", "m1", 100, 10);
        early.source_in_mut().unwrap().ticks = -1;
        let error = refused(insert("V1", early));
        assert_eq!(
            error,
            OpError::OutOfSource {
                media: "m1".into(),
                why: "before_start"
            }
        );
        assert_eq!(
            error.to_string(),
            "the clip begins before the start of media m1"
        );
        // A trim is checked so too.
        let error = refused(Op::TrimClip {
            timeline: "t1".into(),
            id: "c1".into(),
            source_in: Some(Time {
                ticks: 15360,
                scale: SCALE,
            }),
            start: 0,
            length: 226,
        });
        assert_eq!(error, past_end);
    }

    #[test]
    fn a_clip_counts_its_source_in_the_scale_of_its_media() {
        let mut other = clip("", "m1", 100, 10);
        other.source_in_mut().unwrap().scale = 90000;
        assert_eq!(
            refused(insert("V1", other)),
            OpError::Scale {
                scale: 90000,
                media_scale: SCALE
            }
        );
        let mut none = clip("", "m1", 100, 10);
        none.source_in_mut().unwrap().scale = 0;
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
        far.source_in_mut().unwrap().ticks = -LARGEST - 1;
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
        // 2^53 - 1 itself is read exactly.
        let mut largest = media("", true, true);
        largest.identity.size = LARGEST.unsigned_abs();
        apply(
            &mut Project::sample(),
            Op::AddMedia {
                media: largest,
                index: None,
            },
        )
        .unwrap();

        // The last number that next_id can be is 2^53 - 1, so the last id it makes ends in one
        // below that.
        let mut project = Project::sample();
        project.next_id = LARGEST.unsigned_abs() - 1;
        apply(&mut project, insert("V1", clip("", "m1", 100, 10))).unwrap();
        assert_eq!(project.next_id, LARGEST.unsigned_abs());
        let error = refused_in(project.clone(), insert("V1", clip("", "m1", 110, 10)));
        assert_eq!(error, OpError::IdsExhausted);
        assert_eq!(error.to_string(), "the engine has made all the ids it can");
        // A project in which the engine has made its last id is read back as it was.
        let made = project.clone();
        project.validate().unwrap();
        assert_eq!(project, made);
    }

    #[test]
    fn media_lasts_no_less_than_no_time_and_its_picture_has_a_whole_rate() {
        let mut backwards = media("", true, true);
        backwards.facts.duration.ticks = -1;
        let error = refused(Op::AddMedia {
            media: backwards,
            index: None,
        });
        assert_eq!(error, OpError::NegativeDuration { ticks: -1 });
        let mut still = media("", true, true);
        still.facts.video.as_mut().unwrap().rate.num = 0;
        let error = refused(Op::AddMedia {
            media: still,
            index: None,
        });
        assert_eq!(error, OpError::ZeroRate);
        // Media of no time at all may be taken in, though no clip can be cut from it.
        let mut empty = media("", true, true);
        empty.facts.duration.ticks = 0;
        let mut project = sample_with(vec![Op::AddMedia {
            media: empty,
            index: None,
        }]);
        let error = apply(&mut project, insert("V1", clip("", "m3", 100, 1))).unwrap_err();
        assert!(matches!(
            error,
            OpError::OutOfSource {
                why: "past_end",
                ..
            }
        ));
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
                    source_in: Some(source_in),
                    start: 5,
                    length: 7,
                },
                "trim_clip",
            ),
            (
                Op::SetProxy {
                    media: "m1".into(),
                    proxy: Some(proxy("C:/cache/m1.mov")),
                },
                "set_proxy",
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
            source_in: Some(source_in),
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
    fn an_addition_is_written_whole_and_an_index_of_none_as_null() {
        let value = serde_json::to_value(Op::AddMedia {
            media: media("m2", false, true),
            index: None,
        })
        .unwrap();
        let expected = serde_json::json!({
            "op": "add_media",
            "media": {
                "id": "m2",
                "path": "C:/media/talk.mp4",
                "identity": {
                    "size": 12_500_000,
                    "modified_us": 1_790_000_000_000_000_i64,
                    "digest": "0".repeat(64),
                },
                "facts": {
                    "duration": { "ticks": 153_600, "scale": 15360 },
                    "video": null,
                    "audio": { "rate": 48000, "channels": 2, "codec": "aac" },
                },
                "name": "talk.mp4",
                "bin": null,
                "mark_in": null,
                "mark_out": null,
            },
            "index": null,
        });
        assert_eq!(value, expected);

        let value = serde_json::to_value(Op::AddTimeline {
            timeline: timeline(
                "t2",
                vec![track("V1", TrackKind::Video, vec![clip("c5", "m1", 0, 1)])],
            ),
            index: Some(1),
        })
        .unwrap();
        let expected = serde_json::json!({
            "op": "add_timeline",
            "timeline": {
                "id": "t2",
                "name": "Timeline 2",
                "rate": { "num": 25, "den": 1 },
                "width": 1920,
                "height": 1080,
                "sample_rate": 48000,
                "tracks": [{
                    "id": "V1",
                    "kind": "video",
                    "name": "V1",
                    "enabled": true,
                    "locked": false,
                    "clips": [{
                        "id": "c5",
                        "start": 0,
                        "length": 1,
                        "source": { "kind": "media", "media": "m1", "in": { "ticks": 0, "scale": 15360 } },
                        "enabled": true,
                        "speed": { "num": 1, "den": 1 },
                        "reverse": false,
                        "link": null,
                        "transform": { "x": 0.0, "y": 0.0, "scale": 1.0, "rotation": 0.0, "opacity": 1.0 },
                        "gain_db": 0.0,
                        "pan": 0.0,
                        "fade_in": 0,
                        "fade_out": 0,
                    }],
                    "transitions": [],
                }],
            },
            "index": 1,
        });
        assert_eq!(value, expected);
    }

    #[test]
    fn an_index_that_is_left_out_is_none() {
        let value = serde_json::json!({
            "op": "add_track",
            "timeline": "t1",
            "track": {
                "id": "", "kind": "video", "name": "", "enabled": true, "locked": false,
                "clips": [], "transitions": [],
            },
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
            (
                OpError::OutOfSource {
                    media: id(),
                    why: "past_end",
                },
                "out_of_source",
            ),
            (OpError::NegativeDuration { ticks: -1 }, "negative_duration"),
            (OpError::Reserved { id: id() }, "reserved"),
            (OpError::IdsExhausted, "ids_exhausted"),
            (OpError::Dollar { id: id() }, "dollar"),
            (OpError::NoId, "no_id"),
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

    /// The sample as it is written, changed by `change`, and read back.
    fn read(change: impl FnOnce(&mut serde_json::Value)) -> Result<Project, ReadError> {
        let mut value = serde_json::to_value(Project::sample()).unwrap();
        change(&mut value);
        Project::from_value(value)
    }

    /// Why the sample, changed by `change`, is not valid.
    fn invalid(change: impl FnOnce(&mut serde_json::Value)) -> Invalid {
        match read(change) {
            Err(ReadError::Invalid(invalid)) => invalid,
            other => panic!("{other:?}"),
        }
    }

    fn wrong(what: &'static str, id: &str, error: OpError) -> Invalid {
        Invalid {
            what,
            id: id.into(),
            error,
        }
    }

    /// The first clip of the sample, as it is written.
    fn c1(value: &mut serde_json::Value) -> &mut serde_json::Value {
        &mut value["timelines"][0]["tracks"][0]["clips"][0]
    }

    /// A project that is read in a mess that can be put right: the clips of V1 out of order,
    /// media that no clip uses, and `next_id` below the numbers of its ids.
    fn messy() -> Project {
        read(|value| {
            value["next_id"] = 1.into();
            let mut later = c1(value).clone();
            later["id"] = "c4".into();
            later["start"] = 150.into();
            let clips = value["timelines"][0]["tracks"][0]["clips"]
                .as_array_mut()
                .unwrap();
            clips.insert(0, later);
            let mut unused = value["media"][0].clone();
            unused["id"] = "m5".into();
            value["media"].as_array_mut().unwrap().push(unused);
        })
        .unwrap()
    }

    #[test]
    fn a_valid_project_is_read_as_it_was() {
        assert_eq!(read(|_| {}).unwrap(), Project::sample());
        let mut project = Project::sample();
        project.validate().unwrap();
        assert_eq!(project, Project::sample());
    }

    #[test]
    fn a_project_is_read_with_the_clips_of_each_track_in_order() {
        let project = messy();
        assert_eq!(starts(&project, 0), [0, 150]);
        assert_eq!(project.timelines[0].tracks[0].clips[1].id, "c4");
    }

    #[test]
    fn a_project_is_read_with_next_id_above_the_ids_that_the_engine_could_have_made() {
        let mut project = messy();
        assert_eq!(project.next_id, 6, "above m5");
        apply(&mut project, insert("V1", clip("", "m1", 100, 10))).unwrap();
        assert_eq!(project.timelines[0].tracks[0].clips[1].id, "c6");
        // The id of a track counts too; one with a number that the engine never makes does not.
        let project = read(|value| {
            value["timelines"][0]["tracks"][1]["id"] = "c7".into();
            value["media"][0]["id"] = "m9007199254740991".into();
            c1(value)["source"]["media"] = "m9007199254740991".into();
            value["timelines"][0]["tracks"][1]["clips"][0]["source"]["media"] =
                "m9007199254740991".into();
        })
        .unwrap();
        assert_eq!(project.next_id, 8);
    }

    #[test]
    fn every_removal_from_a_project_that_was_read_is_taken_back() {
        let project = messy();
        let mut removals = vec![
            Op::RemoveTimeline { id: "t1".into() },
            Op::RemoveMedia { id: "m5".into() },
        ];
        for track in &project.timelines[0].tracks {
            removals.push(Op::RemoveTrack {
                timeline: "t1".into(),
                id: track.id.clone(),
            });
            for clip in &track.clips {
                removals.push(Op::RemoveClip {
                    timeline: "t1".into(),
                    id: clip.id.clone(),
                });
                removals.push(Op::TrimClip {
                    timeline: "t1".into(),
                    id: clip.id.clone(),
                    source_in: clip.source_in(),
                    start: clip.start + 300,
                    length: 1,
                });
            }
        }
        for op in removals {
            let mut changed = project.clone();
            let inverse = apply(&mut changed, op.clone()).unwrap();
            apply(&mut changed, inverse).unwrap_or_else(|e| panic!("{op:?}: {e}"));
            assert_eq!(changed, project, "{op:?}");
        }
    }

    #[test]
    fn a_clip_that_is_read_is_checked_as_an_inserted_one_is() {
        let past_end = OpError::OutOfSource {
            media: "m1".into(),
            why: "past_end",
        };
        let error = invalid(|value| c1(value)["length"] = 251.into());
        assert_eq!(error, wrong("clip", "c1", past_end));
        assert_eq!(
            error.to_string(),
            "clip c1: the clip goes on past the end of media m1"
        );
        let error = invalid(|value| c1(value)["source"]["in"]["scale"] = 90000.into());
        let scale = OpError::Scale {
            scale: 90000,
            media_scale: SCALE,
        };
        assert_eq!(error, wrong("clip", "c1", scale));
        let error = invalid(|value| c1(value)["start"] = (-1).into());
        assert_eq!(
            error,
            wrong("clip", "c1", OpError::OutOfTimeline { start: -1 })
        );
        let error = invalid(|value| c1(value)["start"] = (LARGEST + 1).into());
        let too_large = OpError::TooLarge { what: "start" };
        assert_eq!(error, wrong("clip", "c1", too_large));
        let error = invalid(|value| c1(value)["source"]["media"] = "m9".into());
        assert_eq!(error, wrong("clip", "c1", missing("media", "m9")));
        let error = invalid(|value| value["media"][0]["facts"]["video"] = serde_json::Value::Null);
        let no_stream = OpError::NoStream {
            media: "m1".into(),
            kind: TrackKind::Video,
        };
        assert_eq!(error, wrong("clip", "c1", no_stream));
        let error = invalid(|value| {
            let mut second = c1(value).clone();
            second["id"] = "c3".into();
            second["start"] = 99.into();
            value["timelines"][0]["tracks"][0]["clips"]
                .as_array_mut()
                .unwrap()
                .push(second);
        });
        assert_eq!(
            error,
            wrong("clip", "c3", OpError::Overlap { with: "c1".into() })
        );
    }

    #[test]
    fn media_that_is_read_is_checked_as_added_media_is() {
        let error = invalid(|value| value["media"][0]["facts"]["duration"]["scale"] = 0.into());
        let scale = OpError::Scale {
            scale: 0,
            media_scale: 0,
        };
        assert_eq!(error, wrong("media", "m1", scale));
        let error = invalid(|value| value["media"][0]["facts"]["duration"]["ticks"] = (-1).into());
        assert_eq!(
            error,
            wrong("media", "m1", OpError::NegativeDuration { ticks: -1 })
        );
        let error = invalid(|value| value["media"][0]["identity"]["size"] = (1_u64 << 53).into());
        assert_eq!(
            error,
            wrong("media", "m1", OpError::TooLarge { what: "size" })
        );
        // A rate with a 0 in it is refused as it is read; one made in code, by validate.
        let mut project = Project::sample();
        project.media[0].facts.video.as_mut().unwrap().rate.den = 0;
        assert_eq!(
            project.validate().unwrap_err(),
            wrong("media", "m1", OpError::ZeroRate)
        );
        let mut project = Project::sample();
        project.timelines[0].rate.num = 0;
        assert_eq!(
            project.validate().unwrap_err(),
            wrong("timeline", "t1", OpError::ZeroRate)
        );
    }

    #[test]
    fn every_thing_that_is_read_has_an_id_of_its_own() {
        let error = invalid(|value| c1(value)["id"] = "".into());
        assert_eq!(error, wrong("clip", "", OpError::NoId));
        assert_eq!(
            error.to_string(),
            "a clip of the project: the thing has no id"
        );
        let error = invalid(|value| value["timelines"][0]["id"] = "".into());
        assert_eq!(error, wrong("timeline", "", OpError::NoId));
        let error = invalid(|value| {
            value["timelines"][0]["tracks"][1]["clips"][0]["id"] = "c1".into();
        });
        assert_eq!(error, wrong("clip", "c1", taken("clip", "c1")));
        let error = invalid(|value| value["timelines"][0]["tracks"][1]["id"] = "V1".into());
        assert_eq!(error, wrong("track", "V1", taken("track", "V1")));
        let error = invalid(|value| {
            let media = value["media"][0].clone();
            value["media"].as_array_mut().unwrap().push(media);
        });
        assert_eq!(error, wrong("media", "m1", taken("media", "m1")));
        let error = invalid(|value| value["next_id"] = (1_u64 << 53).into());
        let too_large = OpError::TooLarge { what: "next_id" };
        assert_eq!(error, wrong("project", "", too_large));
        assert_eq!(
            error.to_string(),
            "the project: the next_id is further than 2^53 - 1 from 0"
        );
    }

    fn proxy(path: &str) -> Proxy {
        Proxy {
            path: path.into(),
            width: 1920,
            height: 1080,
            codec: "hevc".into(),
            digest_of_source: "0".repeat(64),
        }
    }

    #[test]
    fn a_proxy_that_is_set_is_taken_back_by_its_inverse() {
        let mut project = Project::sample();
        let set = Op::SetProxy {
            media: "m1".into(),
            proxy: Some(proxy(r"C:\cache\m1.mov")),
        };
        let (done, inverse) = apply_done(&mut project, set.clone()).unwrap();
        assert_eq!(done, set);
        assert_eq!(
            inverse,
            Op::SetProxy {
                media: "m1".into(),
                proxy: None,
            }
        );
        assert_eq!(project.media[0].proxy, Some(proxy(r"C:\cache\m1.mov")));
        // Another in its place: the inverse brings back the first.
        let other = Op::SetProxy {
            media: "m1".into(),
            proxy: Some(proxy("/cache/m1-b.mov")),
        };
        let back = apply(&mut project, other).unwrap();
        assert_eq!(
            back,
            Op::SetProxy {
                media: "m1".into(),
                proxy: Some(proxy(r"C:\cache\m1.mov")),
            }
        );
        apply(&mut project, back).unwrap();
        apply(&mut project, inverse).unwrap();
        assert_eq!(project, Project::sample());
    }

    #[test]
    fn a_proxy_is_set_on_media_that_is_there_with_an_absolute_path() {
        let error = refused(Op::SetProxy {
            media: "m9".into(),
            proxy: None,
        });
        assert_eq!(error, missing("media", "m9"));
        for path in [
            "cache/m1.mov",
            "m1.mov",
            "C:m1.mov",
            "",
            r"\cache\m1.mov",
            "C:/a\0b",
        ] {
            let error = refused(Op::SetProxy {
                media: "m1".into(),
                proxy: Some(proxy(path)),
            });
            assert_eq!(
                error,
                OpError::NotAbsolute { path: path.into() },
                "{path:?}"
            );
        }
        for path in [
            r"C:\a\m1.mov",
            "c:/a/m1.mov",
            r"\\server\share\m1.mov",
            "/a/m1.mov",
        ] {
            let mut project = Project::sample();
            let set = Op::SetProxy {
                media: "m1".into(),
                proxy: Some(proxy(path)),
            };
            apply(&mut project, set).unwrap();
        }
        // Media that is added with a proxy is checked as one that is set.
        let mut media = media("m2", true, true);
        media.proxy = Some(proxy("m2.mov"));
        let error = refused(Op::AddMedia { media, index: None });
        assert_eq!(
            error.to_string(),
            "the path m2.mov of a proxy is not absolute"
        );
        let value = serde_json::to_value(&error).unwrap();
        assert_eq!(value["reason"], "not_absolute");
        assert_eq!(value["path"], "m2.mov");
    }

    #[test]
    fn a_proxy_is_written_where_there_is_one_and_read_where_there_is_none() {
        let value = serde_json::to_value(Op::SetProxy {
            media: "m1".into(),
            proxy: Some(proxy("/p/m1.mov")),
        })
        .unwrap();
        let expected = serde_json::json!({
            "op": "set_proxy",
            "media": "m1",
            "proxy": {
                "path": "/p/m1.mov", "width": 1920, "height": 1080, "codec": "hevc",
                "digest_of_source": "0".repeat(64),
            },
        });
        assert_eq!(value, expected);
        let none = serde_json::json!({ "op": "set_proxy", "media": "m1", "proxy": null });
        assert_eq!(
            serde_json::from_value::<Op>(none).unwrap(),
            Op::SetProxy {
                media: "m1".into(),
                proxy: None,
            }
        );
        // A project read with a proxy whose path is not absolute is refused.
        let error = invalid(|value| {
            value["media"][0]["proxy"] = serde_json::to_value(proxy("p.mov")).unwrap();
        });
        let not_absolute = OpError::NotAbsolute {
            path: "p.mov".into(),
        };
        assert_eq!(error, wrong("media", "m1", not_absolute));
        // A field of a proxy that is not known is refused.
        let mut value = serde_json::to_value(proxy("/p.mov")).unwrap();
        value["colour"] = "red".into();
        assert!(serde_json::from_value::<Proxy>(value).is_err());
    }

    // The rules of schema 2.

    /// A change to a clip.
    type Change = dyn Fn(&mut Clip);

    fn title(text: &str) -> Source {
        Source::Title {
            text: text.into(),
            style: TitleStyle {
                font: String::new(),
                size: 96,
                color: Rgba([255, 255, 255, 255]),
                x: 0.5,
                y: 0.5,
                align: TextAlign::Center,
            },
        }
    }

    fn colour() -> Source {
        Source::Color {
            color: Rgba([0, 0, 0, 255]),
        }
    }

    fn range(what: &'static str) -> OpError {
        OpError::OutOfRange { what }
    }

    fn wrong_track(what: &'static str, kind: TrackKind) -> OpError {
        OpError::WrongTrack { what, kind }
    }

    fn dissolve(id: &str, left: &str, right: &str, length: i64, align: Alignment) -> Transition {
        Transition {
            id: id.into(),
            kind: TransitionKind::Dissolve,
            left: left.into(),
            right: right.into(),
            length,
            align,
        }
    }

    fn cue(id: &str, start: i64, length: i64, text: &str) -> Cue {
        Cue {
            id: id.into(),
            start,
            length,
            text: text.into(),
        }
    }

    /// The sample with c3 on V1 right after c1, on frames [100, 200), cut from m1 from 4 seconds
    /// in, where `change` puts it, and with `transitions` on V1; read as a project is read.
    fn with_transitions(
        change: impl FnOnce(&mut Clip),
        transitions: Vec<Transition>,
    ) -> Result<Project, ReadError> {
        let mut c3 = clip("c3", "m1", 100, 100);
        *c3.source_in_mut().unwrap() = Time {
            ticks: 4 * i64::from(SCALE),
            scale: SCALE,
        };
        change(&mut c3);
        let mut project = Project::sample();
        project.timelines[0].tracks[0].clips.push(c3);
        project.timelines[0].tracks[0].transitions = transitions;
        Project::from_value(serde_json::to_value(project).unwrap())
    }

    fn refused_read(result: Result<Project, ReadError>) -> Invalid {
        match result {
            Err(ReadError::Invalid(invalid)) => invalid,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_timeline_is_of_an_even_size_within_bounds_and_of_a_known_sample_rate() {
        for (field, value) in [
            ("width", 1921),
            ("width", 14),
            ("width", 16386),
            ("height", 1081),
            ("height", 0),
            ("sample_rate", 44000),
        ] {
            let error = invalid(|v| v["timelines"][0][field] = value.into());
            let what = if field == "sample_rate" {
                "sample_rate"
            } else {
                field
            };
            assert_eq!(
                error,
                wrong("timeline", "t1", range(what)),
                "{field} {value}"
            );
        }
        for (width, height, rate) in [(16, 16384, 44100), (720, 1280, 96000)] {
            read(|v| {
                v["timelines"][0]["width"] = width.into();
                v["timelines"][0]["height"] = height.into();
                v["timelines"][0]["sample_rate"] = rate.into();
            })
            .unwrap();
        }
        let mut odd = timeline("", vec![]);
        odd.width = 1919;
        let error = refused(Op::AddTimeline {
            timeline: odd,
            index: None,
        });
        assert_eq!(error, range("width"));
        assert_eq!(
            error.to_string(),
            "the width is not an even number from 16 to 16384"
        );
    }

    #[test]
    fn a_track_holds_only_what_its_kind_holds() {
        let add = |track: Track| Op::AddTrack {
            timeline: "t1".into(),
            track,
            index: None,
        };
        let subtitles = Track::new("", TrackKind::Subtitle);
        let error = refused(add(subtitles
            .clone()
            .with_clips(vec![clip("", "m1", 0, 10)])));
        assert_eq!(error, wrong_track("clip", TrackKind::Subtitle));
        let mut cues = Track::new("", TrackKind::Video);
        cues.cues = vec![cue("", 0, 10, "hello")];
        assert_eq!(refused(add(cues)), wrong_track("cue", TrackKind::Video));
        let mut spoken = Track::new("", TrackKind::Audio);
        spoken.language = "en".into();
        assert_eq!(
            refused(add(spoken)),
            wrong_track("language", TrackKind::Audio)
        );
        for tag in ["english", "e", "zh_CN", "zh-", "en-toolongsubtag"] {
            let mut track = subtitles.clone();
            track.language = tag.into();
            assert_eq!(refused(add(track)), range("language"), "{tag}");
        }
        let mut project = Project::sample();
        for tag in ["", "zh", "en-GB", "zh-Hant-TW", "ja"] {
            let mut track = subtitles.clone();
            track.language = tag.into();
            apply(&mut project, add(track)).unwrap();
        }
        let ids: Vec<&str> = project.timelines[0].tracks[2..]
            .iter()
            .map(|r| &*r.id)
            .collect();
        assert_eq!(ids, ["S1", "S2", "S3", "S4", "S5"]);
        let error = refused_in(project, insert("S1", clip("", "m1", 0, 10)));
        assert_eq!(error, wrong_track("clip", TrackKind::Subtitle));
        assert_eq!(error.to_string(), "a clip cannot be on a subtitle track");
    }

    #[test]
    fn a_clip_is_played_within_its_settings() {
        let at_150 = |change: &Change| {
            let mut clip = clip("", "m1", 150, 10);
            change(&mut clip);
            insert("V1", clip)
        };
        let cases: [(&Change, &str); 14] = [
            (
                &|c: &mut Clip| c.speed = Speed { num: 1, den: 101 },
                "speed",
            ),
            (
                &|c: &mut Clip| c.speed = Speed { num: 101, den: 1 },
                "speed",
            ),
            (&|c: &mut Clip| c.transform.scale = 0.0, "transform"),
            (&|c: &mut Clip| c.transform.scale = 16.5, "transform"),
            (&|c: &mut Clip| c.transform.opacity = 1.5, "transform"),
            (&|c: &mut Clip| c.transform.opacity = -0.1, "transform"),
            (&|c: &mut Clip| c.transform.x = f64::NAN, "transform"),
            (
                &|c: &mut Clip| c.transform.rotation = f64::INFINITY,
                "transform",
            ),
            (&|c: &mut Clip| c.gain_db = -97.0, "gain_db"),
            (&|c: &mut Clip| c.gain_db = 24.5, "gain_db"),
            (&|c: &mut Clip| c.pan = 1.5, "pan"),
            (&|c: &mut Clip| c.pan = f64::NAN, "pan"),
            (&|c: &mut Clip| (c.fade_in, c.fade_out) = (6, 5), "fade"),
            (&|c: &mut Clip| c.fade_in = -1, "fade"),
        ];
        for (change, what) in cases {
            assert_eq!(refused(at_150(change)), range(what), "{what}");
        }
        let fine: [&Change; 6] = [
            &|c: &mut Clip| c.speed = Speed { num: 1, den: 100 },
            &|c: &mut Clip| c.speed = Speed { num: 20, den: 1 },
            &|c: &mut Clip| {
                c.transform = Transform {
                    x: -2.0,
                    y: 0.25,
                    scale: 16.0,
                    rotation: 720.0,
                    opacity: 0.0,
                };
            },
            &|c: &mut Clip| (c.gain_db, c.pan) = (-96.0, -1.0),
            &|c: &mut Clip| (c.gain_db, c.pan) = (24.0, 1.0),
            &|c: &mut Clip| (c.fade_in, c.fade_out) = (5, 5),
        ];
        for change in fine {
            apply(&mut Project::sample(), at_150(change)).unwrap();
        }
    }

    #[test]
    fn a_clip_shows_its_media_at_its_speed_forward_or_backward() {
        let past_end = OpError::OutOfSource {
            media: "m1".into(),
            why: "past_end",
        };
        // 100 frames at 25 a second are 4 seconds; m1 lasts 10.
        let fast = |num, den, reverse| {
            let mut clip = clip("", "m1", 150, 100);
            clip.speed = Speed { num, den };
            clip.reverse = reverse;
            insert("V1", clip)
        };
        for reverse in [false, true] {
            apply(&mut Project::sample(), fast(5, 2, reverse)).unwrap();
            assert_eq!(refused(fast(26, 10, reverse)), past_end);
        }
        // Half as fast, 200 frames are 4 seconds of it.
        let mut slow = clip("", "m1", 150, 200);
        slow.speed = Speed { num: 1, den: 2 };
        apply(&mut Project::sample(), insert("V1", slow)).unwrap();
    }

    #[test]
    fn a_title_or_a_colour_is_on_a_track_of_picture() {
        let made = |source, track: &str| insert(track, Clip::new("", source, 150, 50));
        assert_eq!(
            refused(made(title("Hello"), "A1")),
            wrong_track("title", TrackKind::Audio)
        );
        assert_eq!(
            refused(made(colour(), "A1")),
            wrong_track("color", TrackKind::Audio)
        );
        let mut project = Project::sample();
        apply(&mut project, made(title(""), "V1")).unwrap();
        apply(&mut Project::sample(), made(colour(), "V1")).unwrap();
        let long = "字".repeat(1001);
        assert_eq!(refused(made(title(&long), "V1")), range("text"));
        apply(
            &mut Project::sample(),
            made(title(&"字".repeat(1000)), "V1"),
        )
        .unwrap();
        let styled = |change: &dyn Fn(&mut TitleStyle)| {
            let mut source = title("Hello");
            if let Source::Title { style, .. } = &mut source {
                change(style);
            }
            made(source, "V1")
        };
        assert_eq!(
            refused(styled(&|s: &mut TitleStyle| s.size = 0)),
            range("size")
        );
        assert_eq!(
            refused(styled(&|s: &mut TitleStyle| s.size = 4097)),
            range("size")
        );
        assert_eq!(
            refused(styled(&|s: &mut TitleStyle| s.x = f64::NAN)),
            range("position")
        );
        assert_eq!(
            refused(styled(&|s: &mut TitleStyle| s.y = f64::INFINITY)),
            range("position")
        );
        let short = insert("V1", Clip::new("", colour(), 150, 0));
        assert_eq!(refused(short), range("length"));
    }

    #[test]
    fn a_clip_that_is_put_in_a_group_with_no_id_is_given_a_new_one() {
        let mut project = Project::sample();
        let mut linked = clip("", "m1", 150, 10);
        linked.link = Some(String::new());
        let inverse = apply(&mut project, insert("V1", linked.clone())).unwrap();
        let made = &project.timelines[0].tracks[0].clips[1];
        assert_eq!((&*made.id, made.link.as_deref()), ("c3", Some("g4")));
        assert_eq!(project.next_id, 5);
        let mut beside = clip("", "m1", 150, 10);
        beside.link = Some("g4".into());
        apply(&mut project, insert("A1", beside)).unwrap();
        let mut early = clip("", "m1", 300, 10);
        early.link = Some("g9".into());
        assert_eq!(
            refused_in(project.clone(), insert("V1", early)),
            OpError::Reserved { id: "g9".into() }
        );
        let mut dollar = clip("", "m1", 300, 10);
        dollar.link = Some("$g".into());
        assert_eq!(
            refused_in(project.clone(), insert("V1", dollar)),
            OpError::Dollar { id: "$g".into() }
        );

        // Done again, it is put in the group that it was given.
        let mut project = Project::sample();
        let (done, inverse_too) = apply_done(&mut project, insert("V1", linked)).unwrap();
        assert_eq!(inverse_too, inverse);
        let Op::InsertClip { clip, .. } = &done else {
            panic!("{done:?}")
        };
        assert_eq!(clip.link.as_deref(), Some("g4"));

        // A group with no id in a project that is read is refused.
        let error = invalid(|v| c1(v)["link"] = "".into());
        assert_eq!(error, wrong("clip", "c1", OpError::NoId));
        let project = read(|v| c1(v)["link"] = "g40".into()).unwrap();
        assert_eq!(project.next_id, 41);
    }

    #[test]
    fn a_transition_is_between_two_clips_of_its_track_that_meet() {
        let project = with_transitions(
            |_| {},
            vec![dissolve("x5", "c1", "c3", 10, Alignment::Center)],
        )
        .unwrap();
        assert_eq!(project.next_id, 6);
        let refused = |change: fn(&mut Clip), transitions| {
            refused_read(with_transitions(change, transitions))
        };
        let on_v1 = |error| wrong("track", "V1", error);
        assert_eq!(
            refused(
                |_| {},
                vec![dissolve("x5", "c1", "c2", 10, Alignment::Center)]
            ),
            on_v1(missing("clip", "c2"))
        );
        assert_eq!(
            refused(
                |c| c.start = 101,
                vec![dissolve("x5", "c1", "c3", 10, Alignment::Center)]
            ),
            on_v1(OpError::NotAdjacent {
                left: "c1".into(),
                right: "c3".into()
            })
        );
        assert_eq!(
            refused(
                |_| {},
                vec![
                    dissolve("x5", "c1", "c3", 10, Alignment::Center),
                    dissolve("x6", "c1", "c3", 4, Alignment::Start),
                ]
            ),
            on_v1(OpError::CutTaken { with: "x5".into() })
        );
        let mut crossfade = dissolve("x5", "c1", "c3", 10, Alignment::Center);
        crossfade.kind = TransitionKind::Crossfade;
        assert_eq!(
            refused(|_| {}, vec![crossfade]),
            on_v1(wrong_track("crossfade", TrackKind::Video))
        );
        assert_eq!(
            refused(
                |_| {},
                vec![dissolve("x5", "c1", "c3", 0, Alignment::Center)]
            ),
            on_v1(range("length"))
        );
        // Half of 300 is more than the 100 frames of either clip.
        assert_eq!(
            refused(
                |_| {},
                vec![dissolve("x5", "c1", "c3", 300, Alignment::Center)]
            ),
            on_v1(OpError::TooLong {
                transition: "x5".into(),
                clip: "c1".into()
            })
        );
        // Two transitions at the ends of c3 cover 60 and 50 of its 100 frames.
        let error = refused_read(with_transitions(|_| {}, vec![]).and_then(|mut project| {
            let track = &mut project.timelines[0].tracks[0];
            track.clips.push(clip("c4", "m1", 200, 60));
            track.transitions = vec![
                dissolve("x5", "c1", "c3", 120, Alignment::Center),
                dissolve("x6", "c3", "c4", 100, Alignment::Center),
            ];
            Project::from_value(serde_json::to_value(project).unwrap())
        }));
        assert_eq!(
            error,
            on_v1(OpError::TooLong {
                transition: "x5".into(),
                clip: "c3".into()
            })
        );
    }

    #[test]
    fn a_transition_shows_frames_of_the_media_of_its_clips_beyond_their_ends() {
        let handles = |media: &str| OpError::OutOfSource {
            media: media.into(),
            why: "handles",
        };
        let from_start = |c: &mut Clip| c.source_in_mut().unwrap().ticks = 0;
        // c3 begins at the start of m1: no frame of it is before.
        for align in [Alignment::Center, Alignment::End] {
            let error = refused_read(with_transitions(
                from_start,
                vec![dissolve("x5", "c1", "c3", 10, align)],
            ));
            assert_eq!(error, wrong("track", "V1", handles("m1")), "{align:?}");
        }
        with_transitions(
            from_start,
            vec![dissolve("x5", "c1", "c3", 10, Alignment::Start)],
        )
        .unwrap();
        // c3 played backward shows what is before its start after its end.
        let backward = |c: &mut Clip| {
            c.source_in_mut().unwrap().ticks = 0;
            c.reverse = true;
        };
        with_transitions(
            backward,
            vec![dissolve("x5", "c1", "c3", 10, Alignment::End)],
        )
        .unwrap();
        // c3 ends at the end of m1, at twice its speed: 100 frames are 8 seconds from 2 in.
        let to_end = |c: &mut Clip| {
            c.source_in_mut().unwrap().ticks = 2 * i64::from(SCALE);
            c.speed = Speed { num: 2, den: 1 };
        };
        let mut project = with_transitions(to_end, vec![]).unwrap();
        project.timelines[0].tracks[0]
            .clips
            .push(clip("c4", "m1", 200, 10));
        project.timelines[0].tracks[0].transitions =
            vec![dissolve("x5", "c3", "c4", 2, Alignment::Start)];
        let error = refused_read(Project::from_value(serde_json::to_value(&project).unwrap()));
        assert_eq!(error, wrong("track", "V1", handles("m1")));
        // A title goes on as long as it is wanted.
        let titled = |c: &mut Clip| c.source = title("Hello");
        with_transitions(titled, vec![dissolve("x5", "c1", "c3", 10, Alignment::End)]).unwrap();
    }

    #[test]
    fn a_clip_that_a_transition_is_at_is_not_moved_trimmed_or_taken_away() {
        let mut project = with_transitions(
            |_| {},
            vec![dissolve("x5", "c1", "c3", 10, Alignment::Center)],
        )
        .unwrap();
        let has = OpError::HasTransition {
            transition: "x5".into(),
        };
        for id in ["c1", "c3"] {
            let ops = [
                Op::RemoveClip {
                    timeline: "t1".into(),
                    id: id.into(),
                },
                Op::MoveClip {
                    timeline: "t1".into(),
                    id: id.into(),
                    track: "V1".into(),
                    start: 400,
                },
                Op::TrimClip {
                    timeline: "t1".into(),
                    id: id.into(),
                    source_in: None,
                    start: 400,
                    length: 5,
                },
            ];
            for op in ops {
                assert_eq!(refused_in(project.clone(), op), has);
            }
        }
        // A track is taken away with its transitions, and they come back with it.
        let before = project.clone();
        let remove = Op::RemoveTrack {
            timeline: "t1".into(),
            id: "V1".into(),
        };
        let inverse = apply(&mut project, remove).unwrap();
        apply(&mut project, inverse).unwrap();
        assert_eq!(project, before);
    }

    #[test]
    fn the_transitions_of_a_track_are_read_in_the_order_of_their_cuts() {
        let mut project = with_transitions(|_| {}, vec![]).unwrap();
        let track = &mut project.timelines[0].tracks[0];
        track.clips.push(clip("c4", "m1", 200, 10));
        track.transitions = vec![
            dissolve("x6", "c3", "c4", 2, Alignment::Start),
            dissolve("x5", "c1", "c3", 2, Alignment::Center),
        ];
        let read = Project::from_value(serde_json::to_value(&project).unwrap()).unwrap();
        let ids: Vec<&str> = read.timelines[0].tracks[0]
            .transitions
            .iter()
            .map(|x| &*x.id)
            .collect();
        assert_eq!(ids, ["x5", "x6"]);
        assert_eq!(read.next_id, 7);
    }

    #[test]
    fn a_subtitle_lasts_a_frame_says_something_and_covers_no_other() {
        let add = |cues: Vec<Cue>| {
            let mut track = Track::new("", TrackKind::Subtitle);
            track.cues = cues;
            Op::AddTrack {
                timeline: "t1".into(),
                track,
                index: None,
            }
        };
        let error = refused(add(vec![cue("", 0, 10, "a"), cue("", 9, 10, "b")]));
        assert_eq!(error, OpError::CueOverlap { with: "q3".into() });
        assert_eq!(refused(add(vec![cue("", 0, 10, "")])), range("text"));
        let long = "あ".repeat(1001);
        assert_eq!(refused(add(vec![cue("", 0, 10, &long)])), range("text"));
        assert_eq!(refused(add(vec![cue("", 0, 0, "a")])), range("length"));
        assert_eq!(refused(add(vec![cue("", -1, 5, "a")])), range("start"));
        assert_eq!(
            refused(add(vec![cue("", LARGEST, 5, "a")])),
            OpError::TooLarge { what: "end" }
        );

        let mut project = Project::sample();
        let op = add(vec![cue("", 20, 10, "b"), cue("q1", 0, 10, "a")]);
        apply(&mut project, op).unwrap();
        let track = &project.timelines[0].tracks[2];
        assert_eq!(track.id, "S1");
        let cues: Vec<(&str, i64)> = track.cues.iter().map(|q| (&*q.id, q.start)).collect();
        assert_eq!(cues, [("q1", 0), ("q3", 20)]);
        assert_eq!(project.next_id, 4);
        // A subtitle is named apart from those of every track.
        let error = refused_in(project, add(vec![cue("q1", 50, 10, "c")]));
        assert_eq!(error, taken("cue", "q1"));
    }

    #[test]
    fn a_bin_is_in_a_bin_that_is_there_and_never_inside_itself() {
        let bins = |list: serde_json::Value| move |v: &mut serde_json::Value| v["bins"] = list;
        let project = read(bins(serde_json::json!([
            { "id": "b1", "name": "Rushes", "parent": null },
            { "id": "b7", "name": "Day 1", "parent": "b1" },
        ])))
        .unwrap();
        assert_eq!(project.next_id, 8);
        let error = invalid(bins(serde_json::json!([
            { "id": "b1", "name": "Rushes", "parent": "b9" },
        ])));
        assert_eq!(error, wrong("bin", "b1", missing("bin", "b9")));
        let error = invalid(bins(serde_json::json!([
            { "id": "b1", "name": "A", "parent": "b2" },
            { "id": "b2", "name": "B", "parent": "b1" },
        ])));
        assert_eq!(
            error,
            wrong("bin", "b1", OpError::Cycle { bin: "b1".into() })
        );
        let error = invalid(bins(serde_json::json!([
            { "id": "b1", "name": "A", "parent": "b1" },
        ])));
        assert_eq!(
            error,
            wrong("bin", "b1", OpError::Cycle { bin: "b1".into() })
        );
        let error = invalid(bins(serde_json::json!([
            { "id": "b1", "name": "A", "parent": null },
            { "id": "b1", "name": "B", "parent": null },
        ])));
        assert_eq!(error, wrong("bin", "b1", taken("bin", "b1")));
        let error = invalid(|v| v["media"][0]["bin"] = "b9".into());
        assert_eq!(error, wrong("media", "m1", missing("bin", "b9")));
        let mut media = media("", true, true);
        media.bin = Some("b9".into());
        assert_eq!(
            refused(Op::AddMedia { media, index: None }),
            missing("bin", "b9")
        );
    }

    #[test]
    fn the_marks_of_media_are_within_it_the_mark_in_first() {
        let marks = |mark_in: Option<i64>, mark_out: Option<i64>| {
            move |v: &mut serde_json::Value| {
                let time = |ticks: Option<i64>| {
                    ticks.map_or(
                        serde_json::Value::Null,
                        |ticks| serde_json::json!({ "ticks": ticks, "scale": SCALE }),
                    )
                };
                v["media"][0]["mark_in"] = time(mark_in);
                v["media"][0]["mark_out"] = time(mark_out);
            }
        };
        let out = wrong(
            "media",
            "m1",
            OpError::OutOfSource {
                media: "m1".into(),
                why: "marks",
            },
        );
        for (mark_in, mark_out) in [
            (Some(100), Some(100)),
            (Some(200), Some(100)),
            (Some(-1), None),
            (None, Some(153_601)),
        ] {
            assert_eq!(
                invalid(marks(mark_in, mark_out)),
                out,
                "{mark_in:?} {mark_out:?}"
            );
        }
        for (mark_in, mark_out) in [(Some(0), Some(153_600)), (Some(100), None), (None, Some(1))] {
            read(marks(mark_in, mark_out)).unwrap();
        }
        let error = invalid(|v| {
            v["media"][0]["mark_in"] = serde_json::json!({ "ticks": 0, "scale": 90000 });
        });
        let scale = OpError::Scale {
            scale: 90000,
            media_scale: SCALE,
        };
        assert_eq!(error, wrong("media", "m1", scale));
    }

    #[test]
    fn a_transition_and_a_subtitle_are_named_apart_from_all_of_their_kind() {
        let error = refused_read(
            with_transitions(
                |_| {},
                vec![dissolve("x5", "c1", "c3", 10, Alignment::Center)],
            )
            .map(|mut project| {
                let mut track = Track::new("V2", TrackKind::Video)
                    .with_clips(vec![clip("c8", "m1", 0, 10), clip("c9", "m1", 10, 10)]);
                track.transitions = vec![dissolve("x5", "c8", "c9", 2, Alignment::Start)];
                project.timelines[0].tracks.push(track);
                project
            })
            .and_then(|project| Project::from_value(serde_json::to_value(project).unwrap())),
        );
        assert_eq!(error, wrong("transition", "x5", taken("transition", "x5")));
    }
}
