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
//! the frames out of every track with `ripple_delete_range`. A `close_gap` given
//! `"straddling": "stay"` leaves such a clip where it is instead (see [`Straddling`]).
//! Subtitles are cut with the frames taken away, and those after them move with the picture.
//! An insert through a subtitle cuts it into two with the same text, leaving the new frames
//! between them. Locked subtitle tracks stay as they were.
//!
//! # Speed points
//!
//! A clip whose speed changes on the way has speed points ([`crate::retime`]), and the edits
//! take it along them. Moved, copied and pasted it keeps them. Trimmed at its head, rolled or
//! slid into, it keeps those from its new first frame on, and trimmed at its tail those before
//! its new end, its last segment going on where it gets longer; cut in two by the blade, or by
//! an edit that clears frames through it, each half has its own. A half or a trimmed clip that
//! is left with one speed is a clip of that speed. Every frame that is kept shows what it
//! showed, as nearly as a place of the media is a whole tick (see [`crate::retime`]). Slipped,
//! it begins as many frames of its source later.
//!
//! What sets one speed is refused for a clip that has speed points, and for the clips linked
//! to it, with `out_of_range` (`retimed`): `set_speed`, the volume of `paste_attributes` where
//! it has keyframes, and a paste into a timeline of another rate. The operation `set_retime`
//! with no `retime` puts the clip back to one speed first, and so does the edit `reset_speed`.
//!
//! The speed points themselves are changed by the edits of the Retime Controls (Reference
//! Manual 20, pp. 1070–1082), which a clip of one speed takes too: [`Edit::AddSpeedPoint`],
//! [`Edit::SetSegmentSpeed`], [`Edit::FreezeFrame`], [`Edit::ReverseSegment`],
//! [`Edit::ClearSpeedPoint`] and [`Edit::ResetSpeed`]. Each is made at a frame of the clip that
//! it names, counted from its first, and the clips linked to it take it at the same frame of
//! the timeline, each counted from its own start; a linked clip that the frame is not in is
//! left as it is. [`Edit::SetSpeedPoints`] sets them whole, as a drag of a speed point does at
//! each move, with one end of the path kept at its tick of the media. A title, a colour and a
//! still are refused with `out_of_range`
//! (`retime_source`). The frames of the clip before the segment that is changed show the
//! pictures that they showed: where the lowest place of the path is then another, the clip
//! begins that much later in its media, or earlier. Those that make the clip longer or shorter
//! take `ripple` as `set_speed` does: with it, what follows moves with the end of the linked
//! clips together; without, a clip that would cover the next one on its track is cut at its
//! end where that one starts, and the edit says that it was clamped. The sound of a clip with
//! speed points is not heard yet.
//!
//! # As far as it goes
//!
//! An edit that moves an edge or a clip by `delta` frames, and cannot go so far, for the end of
//! the media, a clip in the way or a clip that would be left with no frame, goes as far as it can
//! and says that it was clamped. One that cannot go at all is refused, and so is a `delta` of 0.

mod audio;
mod automation;
mod clipboard;
mod cut;
mod follow;
mod grade;
mod input;
mod misc;
mod place;
mod retiming;
mod source_tracks;
mod trim;
mod work;

use serde::{Deserialize, Serialize, Serializer};

use crate::audio::KeyInterp;
use crate::audio::{Insert, LanePoint};
use crate::grade::{Target, Window};
use crate::keyframes::Align;
use crate::ops::{apply_done, InsertsOf, LanesOf, Op, OpError};
use crate::project::{Alignment, Id, Project, Rgba, TitleStyle, TrackKind, TransitionKind};
use crate::retime::Retime;
use crate::time::{Speed, Time};

pub use automation::{
    arm_of, copy_lanes, group_control, grouped_with, CopiedLane, FilledLane, NewPoint, PointAction,
    RangeAction, RecordedLane, RecordedRun, RunEnd, Then,
};
pub use clipboard::{copy_clips, ClipClipboard, CopiedClip, CopiedTransition};
pub use follow::Piece as ClipLane;
pub use grade::Given;
use retiming::Retiming;
use work::Work;

/// The most nodes that Copy Grade keeps of the grades it is put in place of: "0–10" (Reference
/// Manual 20, p. 3285).
pub const MOST_PRESERVED_NODES: u32 = 10;

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
    ///
    /// Given `grouped`, as an editor's own action is, an edit of clips (`lift`, `ripple_delete`,
    /// `move` without `to_track`, `insert_move`) does on each other track, not locked, of the
    /// active groups of tracks that link Editing (Ch. 179, p. 3917) and that the track of one of
    /// its clips is in, what it does over the frames of that clip: the clips there are cut at
    /// its ends and what is between them is taken too. A `blade` or a `ripple_delete_range` given
    /// `tracks` cuts the other tracks of their groups too. Edits that a client makes of several,
    /// such as clearing where copies go, leave it out.
    Lift {
        /// The timeline.
        timeline: Id,
        /// The clips.
        clips: Vec<Id>,
        /// Whether the clips linked to them go too.
        #[serde(default = "yes")]
        linked: bool,
        /// Whether the active groups of tracks that link Editing take the frames of the clips on
        /// their other tracks along (see [`Edit::Lift`]); not where it is left out.
        #[serde(default, skip_serializing_if = "is_false")]
        grouped: bool,
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
        /// Whether the active groups of tracks that link Editing take the frames of the clips on
        /// their other tracks along (see [`Edit::Lift`]); not where it is left out.
        #[serde(default, skip_serializing_if = "is_false")]
        grouped: bool,
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
        /// Whether the active groups of tracks that link Editing take the frames of the clips on
        /// their other tracks along (see [`Edit::Lift`]); not where it is left out.
        #[serde(default, skip_serializing_if = "is_false")]
        grouped: bool,
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
        /// What to do with what lies across the gap on the other tracks.
        #[serde(default, skip_serializing_if = "Straddling::is_refuse")]
        straddling: Straddling,
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
        /// Whether the active groups of tracks that link Editing take the frames of the clips on
        /// their other tracks along (see [`Edit::Lift`]); not where it is left out.
        #[serde(default, skip_serializing_if = "is_false")]
        grouped: bool,
    },
    /// Move clips by `delta` frames and, given `to_track`, each to the track as far from its own,
    /// among the tracks of its kind, as `to_track` is from the track of the first clip: the
    /// sound linked to a picture moved from V1 to V2 goes from A1 to A2 with it, as the Linked
    /// Move Across Tracks of `DaVinci Resolve` does (Reference Manual 20, p. 769). No track is
    /// made: a clip with no track to go to is refused with `no_track`, and one whose track to go
    /// to is locked with `locked`. Refused with `no_room` where there is something in the way,
    /// unless `overwrite`, which overwrites it.
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
        /// Whether the active groups of tracks that link Editing take the frames of the clips on
        /// their other tracks along (see [`Edit::Lift`]); not where it is left out.
        #[serde(default, skip_serializing_if = "is_false")]
        grouped: bool,
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
        /// Whether the active groups of tracks that link Editing take the frames of the clips on
        /// their other tracks along (see [`Edit::Lift`]); not where it is left out.
        #[serde(default, skip_serializing_if = "is_false")]
        grouped: bool,
    },
    /// Put copies of clips where they are given to go, as an Alt-drag duplicates clips or a
    /// paste puts copied ones (pp. 811, 3814): what is there is cleared as an overwrite clears it,
    /// each copy has all the settings of its clip but its transitions, the copies of clips linked
    /// together are linked together in a new group, and with Automation Follows Edit each copy
    /// takes a copy of the lanes over its clip (p. 3910).
    PlaceCopies {
        /// The timeline.
        timeline: Id,
        /// The copies.
        copies: Vec<CopyOf>,
        /// Copy the source spans on active Editing group peers, when staying on the same tracks.
        #[serde(default, skip_serializing_if = "is_false")]
        grouped: bool,
    },
    /// Lift whole clips or selected spans, leaving the surrounding material and time in place.
    CutClips {
        /// The timeline.
        timeline: Id,
        /// The selected source clips.
        clips: Vec<Id>,
        /// First frame of the copied range, if it is not the whole clips.
        #[serde(default)]
        from: Option<i64>,
        /// End-exclusive last frame; given together with `from`.
        #[serde(default)]
        to: Option<i64>,
        /// Include unlocked clips linked to the selection.
        #[serde(default = "yes")]
        linked: bool,
        /// Include the selected spans on direct active Editing-group peers.
        #[serde(default)]
        grouped: bool,
    },
    /// Paste independent clip snapshots at the playhead, overwriting their destinations.
    PasteClips {
        /// The destination timeline.
        timeline: Id,
        /// The clips and automation as they were when copied.
        clipboard: ClipClipboard,
        /// The frame aligned with the playhead used when copying.
        at: i64,
        /// Destination track for each source track; omitted entries keep the original id.
        #[serde(default)]
        tracks: std::collections::BTreeMap<Id, Id>,
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
        /// Whether a transition of picture brings a crossfade of the same length at the cut of
        /// the clips of sound linked to its clips, where they meet on a track that is not locked
        /// and have no transition there (p. 973). It is off where it is left out, so that a
        /// client that adds both itself is not refused.
        #[serde(default, skip_serializing_if = "is_false")]
        linked: bool,
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
        /// How the clip that starts at the cut rises across a crossfade: -3, 0 or 3 (p. 3831).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fade_in_level_db: Option<crate::audio::CrossfadeLevel>,
        /// How the clip that ends at the cut falls across a crossfade.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fade_out_level_db: Option<crate::audio::CrossfadeLevel>,
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
    /// Put a speed point at a frame of a clip (Add Speed Point of the Retime Controls,
    /// Reference Manual 20, p. 1076): the segment that the frame is in becomes two of its
    /// speed, and the clip shows what it showed. A clip of one speed has speed points from
    /// then on. Where a speed point is at the frame already, nothing is done.
    AddSpeedPoint {
        /// The timeline.
        timeline: Id,
        /// The clip.
        clip: Id,
        /// The frame of the clip, counted from its first, that the speed point is put at.
        at: i64,
        /// Whether the clips linked to it get one at the same frame of the timeline.
        #[serde(default = "yes")]
        linked: bool,
    },
    /// Play the segment of a clip that a frame is in at another speed, the way that it went,
    /// showing what it showed (Change Speed of the Retime Controls, pp. 1074 and 1076): so the
    /// segment lasts more frames or fewer, rounded down but one at least, the speed points
    /// after it come later or sooner, and so does the end of the clip. A freeze that is given
    /// a speed plays forward over the frames that it had. The whole of a clip of one speed is
    /// its one segment. With `ripple`, what is after the clip moves with its end; without,
    /// a clip that would cover the next one is cut at its end, and the edit is clamped.
    SetSegmentSpeed {
        /// The timeline.
        timeline: Id,
        /// The clip.
        clip: Id,
        /// A frame of the segment, counted from the first frame of the clip.
        at: i64,
        /// The speed, from 1/100 to 100: it is taken as it is given, and not brought to a
        /// round number.
        speed: Speed,
        /// Whether what follows moves with the end of the clip.
        #[serde(default)]
        ripple: bool,
        /// Whether the clips linked to it change speed at the same frame of the timeline.
        #[serde(default = "yes")]
        linked: bool,
    },
    /// Hold the picture of a frame of a clip for `hold` frames more (Freeze Frame of the
    /// Retime Controls, pp. 1075–1076): two speed points, one where the freeze begins and one
    /// where the clip plays on at the speed that it had. The clip lasts `hold` frames more, and
    /// shows all that it showed. With `ripple`, what is after the clip moves with its end;
    /// without, a clip that would cover the next one is cut at its end, and the edit is
    /// clamped.
    FreezeFrame {
        /// The timeline.
        timeline: Id,
        /// The clip.
        clip: Id,
        /// The frame of the clip, counted from its first, whose picture is held.
        at: i64,
        /// How many frames more the picture is held: one at least.
        hold: i64,
        /// Whether what follows moves with the end of the clip.
        #[serde(default)]
        ripple: bool,
        /// Whether the clips linked to it are held at the same frame of the timeline.
        #[serde(default = "yes")]
        linked: bool,
    },
    /// Play the segment of a clip that a frame is in the other way, at its speed (Reverse
    /// Segment of the Retime Controls, pp. 1074 and 1076). The path of the clip has no jump: the
    /// first segment shows what it showed, the last first, and so does the whole of a clip of
    /// one speed; any other segment goes back from where the one before it stopped; and what
    /// follows goes on from where the segment then stops. The clip lasts as long as it did. A
    /// freeze stays as it is. Where the clip would then show what its media has not, the edit
    /// is refused with `out_of_source`.
    ReverseSegment {
        /// The timeline.
        timeline: Id,
        /// The clip.
        clip: Id,
        /// A frame of the segment, counted from the first frame of the clip.
        at: i64,
        /// Whether the clips linked to it are turned at the same frame of the timeline.
        #[serde(default = "yes")]
        linked: bool,
    },
    /// Clear the speed point that begins the segment of a clip that a frame is in (Clear Speed
    /// Point of the Retime Controls, p. 1076, which clears the speed point at the left of a
    /// segment). The segment and the one before it become one, which goes at one speed from
    /// where the one before it began to where the segment stopped, as the Retime Frame curve
    /// goes in a straight line where a control point of it is taken away (pp. 1078 and 1081).
    /// The clip lasts as long as it did, and what is before the two segments and after them
    /// shows what it showed. The first segment has no speed point to clear (`out_of_range`,
    /// `speed_point`), and a speed that no segment may have is refused (`speed`,
    /// `retime_unit`).
    ClearSpeedPoint {
        /// The timeline.
        timeline: Id,
        /// The clip.
        clip: Id,
        /// A frame of the segment, counted from the first frame of the clip.
        at: i64,
        /// Whether the clips linked to it lose theirs at the same frame of the timeline.
        #[serde(default = "yes")]
        linked: bool,
    },
    /// Put a clip back to the speed of its source, forward, with no speed points (Reset to
    /// 100% of the Retime Controls, p. 1074). It begins where it did in its media, and lasts
    /// as many frames as show at that speed, once, what it showed from the lowest place of its
    /// path to the highest. With `ripple`, what is after the clip moves with its end; without,
    /// a clip that would cover the next one is cut at its end, and the edit is clamped.
    ResetSpeed {
        /// The timeline.
        timeline: Id,
        /// The clip.
        clip: Id,
        /// Whether what follows moves with the end of the clip.
        #[serde(default)]
        ripple: bool,
        /// Whether the clips linked to it are put back too.
        #[serde(default = "yes")]
        linked: bool,
    },
    /// Set the speed points of a clip whole, the clip lasting as long as it did: what a drag
    /// of a speed point of the Retime Controls, or of a control point of the Retime Frame curve,
    /// asks for at each of its moves (pp. 1075 and 1078–1081). One end of the path of the clip
    /// ([`crate::retime`]) stays at the tick of the media that it is at, and the clip begins
    /// that much later in its media, or earlier, where the lowest place of the path is then
    /// another: where the path begins is kept, the frames of a first segment that plays as it
    /// did show the pictures that they showed. Speed points of one speed are kept as the speed
    /// of the clip. The edit is the same whatever went before it: made again, it sets the
    /// speed points of the clip that it names once more, and changes nothing.
    SetSpeedPoints {
        /// The timeline.
        timeline: Id,
        /// The clip.
        clip: Id,
        /// The speed points, as [`crate::ops::Op::SetRetime`] takes them: those of a clip of
        /// the length that the clip has.
        retime: Retime,
        /// The end of the path that stays where it is in the media: where it begins, if this
        /// is left out.
        #[serde(default, skip_serializing_if = "PathEnd::is_start")]
        keep: PathEnd,
        /// Whether the clips linked to it that start with it, last as long and play as it did
        /// take the same speed points.
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
    /// Add a track with nothing on it. A track of subtitles is added disabled (hidden) while
    /// another track of subtitles of the timeline is enabled: a timeline shows one at a time.
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
    /// Add a corrector after a node of a grade, or after its source: it takes what the node
    /// gave its picture to (Add Serial, Option-S; Reference Manual 20, p. 3318). With
    /// `windows`, the corrector has those Power Windows from the start, in the one operation
    /// that adds it (Add Serial Node + CPW, LPW, PPW or PCW, Option-C, -Q, -G or -B; p. 3187).
    AddSerial {
        /// The grade.
        grade: Target,
        /// The node, or `source`.
        node: Id,
        /// The windows of the new corrector, checked as `set_node` checks a node's windows.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        windows: Vec<Window>,
    },
    /// Add a corrector before a corrector of a grade, or before its output: it takes what came
    /// into the node (Add Before Current, Shift-S).
    AddSerialBefore {
        /// The grade.
        grade: Target,
        /// The corrector, or `output`.
        node: Id,
    },
    /// Add a corrector at the end of the tree of a grade, before its output (Append Node,
    /// Option-K).
    AppendNode {
        /// The grade.
        grade: Target,
    },
    /// Add a corrector beside a corrector of a grade, taking what the corrector takes, and mix
    /// the two in a parallel mixer: the one that the corrector alone goes into, or a new one in
    /// its place (Add Parallel, Option-P; p. 3334).
    AddParallel {
        /// The grade.
        grade: Target,
        /// The corrector.
        node: Id,
    },
    /// Add a corrector beside a corrector of a grade, as `add_parallel` does, into a layer
    /// mixer, at its lowest input, which is on top (Add Layer, Option-L; p. 3336).
    AddLayer {
        /// The grade.
        grade: Target,
        /// The corrector.
        node: Id,
    },
    /// Add an Outside node after a corrector of a grade: a corrector that takes what the
    /// corrector gave its picture to, with the corrector's picture linked into its RGB input and
    /// the corrector's key into its key input, inverted, so that it works on what the corrector
    /// did not (Add Outside Node, Option-O; pp. 3318, 3341–3342).
    AddOutside {
        /// The grade.
        grade: Target,
        /// The corrector.
        node: Id,
    },
    /// Take a node out of the tree of its grade, keeping it, and link what was on either side
    /// of it (Extract, E; p. 3319).
    ExtractNode {
        /// The grade.
        grade: Target,
        /// The node.
        node: Id,
    },
    /// Set a node as a new node of its kind is set, keeping its label and its inputs.
    ResetNode {
        /// The grade.
        grade: Target,
        /// The node.
        node: Id,
    },
    /// Put a grade in place of other grades, whole, or but for the first nodes of each (Copy
    /// Grade; pp. 3285–3286, 3290).
    CopyGrade {
        /// The grade that is copied.
        from: Target,
        /// The grades that it is put in place of.
        to: Vec<Target>,
        /// How many of the first nodes of each grade of `to` are kept, from 0 to
        /// [`MOST_PRESERVED_NODES`] (Copy Grade: Preserve number of nodes, p. 3285); none if it
        /// is left out, as in `DaVinci` at first. A grade of `to` with fewer nodes is put in
        /// place whole, as the manual keeps them only "as long as there are at least" that
        /// many. Of one with as many, its first nodes, ids and all, take the places of the
        /// first nodes of the copy, which keeps its links: each pair is of one kind, with as
        /// many inputs, and the copy has that many nodes, or the edit is refused.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        preserve_nodes: Option<u32>,
        /// How the keys of the copy are put on the frames of each clip of `to` (Apply Grades
        /// Using, pp. 3264, 3287; see [`crate::keyframes`]): aligning source timecode if it is
        /// left out, as `DaVinci` does at first. The nodes that are kept keep their keys.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        keyframes: Option<Align>,
    },
    /// Add the nodes of a grade to the end of other grades (Append Node Graph; p. 3290). It is
    /// read by its old name, `append_grade`, too.
    #[serde(alias = "append_grade")]
    AppendNodeGraph {
        /// The grade whose nodes are added.
        from: Target,
        /// The grades that they are added to.
        to: Vec<Target>,
        /// How the keys of the nodes added are put on the frames of each clip of `to`, as for
        /// [`Edit::CopyGrade`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        keyframes: Option<Align>,
    },
    /// [`Edit::CopyGrade`] of a grade given whole rather than named in the project: Apply Grade
    /// of a still of the Gallery (p. 3290), worked out exactly as Copy Grade is. The engine makes
    /// it of a still; it is not read or written.
    #[serde(skip)]
    CopyGivenGrade {
        /// The grade, the frames its keys count, and how they go.
        given: Box<Given>,
        /// The grades that it is put in place of.
        to: Vec<Target>,
        /// As of [`Edit::CopyGrade`].
        preserve_nodes: u32,
    },
    /// [`Edit::AppendNodeGraph`] of a grade given whole: Append Node Graph of a still (p. 3290),
    /// and Append Node to Selected Clips (p. 3297) of one node. The engine makes it; it is not
    /// read or written.
    #[serde(skip)]
    AppendGivenGraph {
        /// The grade, the frames its keys count, and how they go.
        given: Box<Given>,
        /// The grades that its nodes are added to.
        to: Vec<Target>,
    },
    /// Put the nodes of the pre-clip and the post-clip grades of each clip's group into its own
    /// grade, before and after its nodes, and take the clip out of the group (Collapse Group
    /// Grades; pp. 3306–3307). The group keeps its grades and its other clips.
    CollapseGroupGrades {
        /// The timeline of the clips.
        timeline: Id,
        /// The clips, each in a group.
        clips: Vec<Id>,
    },
    /// Set one setting of a node to what it is in another node (Paste Value; p. 3286).
    PasteValue {
        /// The grade of the node that it is copied from.
        from: Target,
        /// The node that it is copied from.
        from_node: Id,
        /// The grade of the node that it is set in.
        to: Target,
        /// The node that it is set in.
        to_node: Id,
        /// The setting, as a path of the fields of `params` written with dots, such as
        /// `primaries.saturation` or `primaries.lift`.
        param: String,
    },
    /// Set a node as another node of its kind is set (Copy and Paste of a node; p. 3320).
    PasteNode {
        /// The grade of the node that is copied.
        from: Target,
        /// The node that is copied.
        from_node: Id,
        /// The grade of the node that is set.
        to: Target,
        /// The node that is set.
        to_node: Id,
    },
    /// Set what a track of sound has of sound by a patch, as the operation `set_track_audio`
    /// does; and with `follow_groups`, what the active groups of tracks that it is in link of
    /// what changed, on their other tracks (Ch. 179, p. 3917): a fader, a pan and the level of a
    /// send by the same change, a mute, a solo, the switch of a send and an automation arm to the
    /// same value. While the solo of the timeline is exclusive (p. 3885), a track that is soloed
    /// takes the solo off every other track and bus. A track that is locked is not changed.
    SetTrackAudio {
        /// The timeline.
        timeline: Id,
        /// The track of sound.
        track: Id,
        /// The settings that change.
        patch: serde_json::Value,
        /// Whether the tracks grouped with it follow it: so they do where it is left out
        /// (`false` is the Ctrl+Shift clutch of p. 3920).
        #[serde(default = "yes")]
        follow_groups: bool,
    },
    /// Add, take away, move or ease a keyframe of the volume or the pan of a clip (pp. 3838–3840).
    ClipGainKeys {
        /// The timeline.
        timeline: Id,
        /// The clip.
        clip: Id,
        /// Its volume (`gain_db`, where it is left out) or its pan.
        #[serde(default)]
        param: Keyed,
        /// What is done.
        action: KeyAction,
        /// The time of the key, a time of the media of the clip.
        at: Time,
        /// Where a key is moved to.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<Time>,
        /// The value of a key that is added or moved: that of the setting there, where it is left
        /// out of one added.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<f64>,
        /// How a key that is added or eased comes in and goes out.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        interp: Option<KeyInterp>,
    },
    /// Raise or lower the volume of a clip between two times of its media (p. 3840).
    ClipGainRange {
        /// The timeline.
        timeline: Id,
        /// The clip.
        clip: Id,
        /// Where it starts, a time of the media of the clip.
        from: Time,
        /// Where it ends.
        to: Time,
        /// By how many decibels.
        delta_db: f64,
    },
    /// Raise or lower the volume of clips of sound, keys and all (p. 969, p. 3837).
    NudgeClipGain {
        /// The timeline.
        timeline: Id,
        /// The clips.
        clips: Vec<Id>,
        /// By how many decibels.
        delta_db: f64,
        /// Whether the clips linked to them change too.
        #[serde(default = "yes")]
        linked: bool,
    },
    /// Paste attributes of a clip onto others (pp. 3803, 3842).
    PasteAttributes {
        /// The timeline.
        timeline: Id,
        /// The clip that they are copied from.
        from: Id,
        /// The clips that they are pasted onto.
        to: Vec<Id>,
        /// What is pasted.
        items: Vec<Attribute>,
    },
    /// Put attributes of clips back as a new clip has them (pp. 3804, 3842).
    RemoveAttributes {
        /// The timeline.
        timeline: Id,
        /// The clips.
        clips: Vec<Id>,
        /// What is put back.
        items: Vec<Attribute>,
    },
    /// Fade clips in to the playhead, or out from it (p. 3812), in one step.
    FadeToPlayhead {
        /// The timeline.
        timeline: Id,
        /// The clips.
        clips: Vec<Id>,
        /// Which end fades.
        side: Edge,
        /// The frame of the playhead.
        at: i64,
    },
    /// Set a bus by a patch, as the operation `set_bus` does; while the solo of the timeline is
    /// exclusive (p. 3885), a bus that is soloed takes the solo off every track, other bus and VCA.
    SetBus {
        /// The timeline.
        timeline: Id,
        /// The bus.
        id: Id,
        /// The settings that change.
        patch: serde_json::Value,
    },
    /// Copy the inserts of one chain onto others, in place of theirs (pp. 3855–3860); the lanes
    /// of the inserts of a strip that they replace are taken away with them.
    CopyEffects {
        /// The timeline.
        timeline: Id,
        /// The chain that they are copied from.
        from: InsertsOf,
        /// The chains that they are pasted onto.
        to: Vec<InsertsOf>,
    },
    /// Put an insert in place of the one at `slot` of a chain, or take that one away (`null`), as
    /// the operation `set_insert` does, the lanes of the one that goes taken away with it; an
    /// insert of a strip put in place of another is given a new id.
    SetInsert {
        /// The timeline.
        timeline: Id,
        /// The chain.
        of: InsertsOf,
        /// The place in the chain, from 0.
        slot: usize,
        /// The insert, or `null`, which must be written, to take the one there away.
        #[serde(deserialize_with = "nullable_required")]
        insert: Option<Insert>,
    },
    /// Move the insert at `from` of a chain to `to`, within it (its lanes going with it), or to
    /// the chain `to_of`, where it is a new insert (its lanes taken away).
    MoveInsert {
        /// The timeline.
        timeline: Id,
        /// The chain.
        of: InsertsOf,
        /// Where it is.
        from: usize,
        /// Where it goes.
        to: usize,
        /// The chain that it goes to, where it is not `of`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to_of: Option<InsertsOf>,
    },
    /// Add, take away or move points of a lane of automation (pp. 3898–3900, 3911–3912).
    LanePoints {
        /// The timeline.
        timeline: Id,
        /// The track of sound, bus or VCA.
        of: LanesOf,
        /// The path of the setting of the lane.
        path: String,
        /// What is done.
        action: PointAction,
        /// The points that are added.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        points: Vec<NewPoint>,
        /// The times of the points that are taken away or moved.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        at: Vec<Time>,
        /// How far in time the points move (back where it is below 0).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        by: Option<Time>,
        /// How far in value the points move.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delta: Option<f64>,
        /// Whether the tracks of the active groups that link Automation and the control of the
        /// setting follow (§11.6, [`grouped_with`]); not where it is left out.
        #[serde(default, skip_serializing_if = "is_false")]
        grouped: bool,
    },
    /// Take away, paste over, raise or lower, or thin a range of a lane of automation (pp.
    /// 3898–3899).
    LaneRange {
        /// The timeline.
        timeline: Id,
        /// The track of sound, bus or VCA.
        of: LanesOf,
        /// The path of the setting of the lane.
        path: String,
        /// Where the range starts, a time of the timeline.
        from: Time,
        /// Where it ends.
        to: Time,
        /// What is done.
        action: RangeAction,
        /// The points pasted, at times from `from`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        points: Option<Vec<LanePoint>>,
        /// How far the values move.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delta: Option<f64>,
        /// How far a thinned lane may go from the points that it had.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tolerance: Option<f64>,
        /// Whether the tracks of the active groups that link Automation and the control of the
        /// setting follow (§11.6, [`grouped_with`]); not where it is left out.
        #[serde(default, skip_serializing_if = "is_false")]
        grouped: bool,
    },
    /// Draw a lane of automation with the Pencil from `from` to `to` (pp. 3900, 3911).
    LaneDraw {
        /// The timeline.
        timeline: Id,
        /// The track of sound, bus or VCA.
        of: LanesOf,
        /// The path of the setting of the lane.
        path: String,
        /// Where the drawing starts, a time of the timeline.
        from: Time,
        /// Where it ends.
        to: Time,
        /// The points drawn, at times of the timeline, in order.
        points: Vec<LanePoint>,
        /// How far the lane may go from them, as they are thinned.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tolerance: Option<f64>,
        /// Whether the tracks of the active groups that link Automation and the control of the
        /// setting follow (§11.6, [`grouped_with`]); not where it is left out.
        #[serde(default, skip_serializing_if = "is_false")]
        grouped: bool,
    },
    /// Fill Range, or Glide Range (pp. 3908, 3913; T16b): lanes written over a range with the
    /// values that Preview holds, or glided to them from the values at its start.
    LaneFill {
        /// The timeline.
        timeline: Id,
        /// The lanes and their values.
        lanes: Vec<FilledLane>,
        /// Where the range starts, a time of the timeline.
        from: Time,
        /// Where it ends.
        to: Time,
        /// Whether it glides (Glide Range), not fills.
        #[serde(default, skip_serializing_if = "is_false")]
        glide: bool,
    },
    /// Paste of automation (pp. 3913–3914; T16b): what Copy took, put at a time, or over a range.
    AutomationPaste {
        /// The timeline.
        timeline: Id,
        /// Where it goes, a time of the timeline.
        at: Time,
        /// The end of the range that it fills, where there is one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<Time>,
        /// The lanes, each to the track, bus or VCA and the setting that it names.
        lanes: Vec<CopiedLane>,
    },
    /// Erase of automation (p. 3914; T16b): the points of a range of the lanes of tracks, buses
    /// and VCAs taken away.
    AutomationErase {
        /// The timeline.
        timeline: Id,
        /// The tracks of sound, buses and VCAs.
        of: Vec<LanesOf>,
        /// Where the range starts, a time of the timeline.
        from: Time,
        /// Where it ends.
        to: Time,
    },
    /// Write what a pass of the recording of automation recorded (T16, pp. 3905–3909): the runs
    /// of each lane in place of what was there, as one step of undo.
    RecordPass {
        /// The timeline.
        timeline: Id,
        /// What each lane recorded.
        lanes: Vec<RecordedLane>,
    },
    /// Take lanes of automation away (Clear Track Automation, p. 3912).
    ClearLanes {
        /// The timeline.
        timeline: Id,
        /// The track of sound, bus or VCA.
        of: LanesOf,
        /// The paths of the lanes; all of them where it is left out.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        paths: Option<Vec<String>>,
        /// What each setting is left at.
        #[serde(default)]
        then: Then,
        /// The time whose value a setting holds, for `hold`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<Time>,
    },
}

/// What an edit puts on a timeline.
///
/// Media is put from `from` to `to`, in ticks of the scale of its duration, `to` not included:
/// from its mark in, or its start, to its mark out (the end of the range marked, after its Out
/// frame), or its end, where they are left out; or for `length` frames from `from`, as for an
/// edit of three points that takes its length from the timeline. Its picture goes on
/// `video_track` and its sound on `audio_track`, linked to each other if both go: a track that
/// is left out is the first track of its kind, and one that is `null` is not used. A title or a
/// colour lasts `length` frames, and goes on a track of picture.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[allow(
    clippy::large_enum_variant,
    reason = "material is read, placed and let go, as an operation is"
)]
pub enum Material {
    /// Media.
    Media {
        /// The media.
        media: Id,
        /// Where in it the clip begins.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<Time>,
        /// Where in it the clip ends: the start of the frame after its last.
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

/// An end of the path of a clip through its source ([`crate::retime`]): the one that stays at
/// the tick of the media that it is at where [`Edit::SetSpeedPoints`] sets the speed points of
/// the clip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathEnd {
    /// Where the path begins: the place at which the first frame of the clip begins.
    #[default]
    Start,
    /// Where the path stops: the place at which the last frame of the clip ends.
    End,
}

impl PathEnd {
    #[allow(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde passes a reference"
    )]
    fn is_start(&self) -> bool {
        *self == Self::Start
    }
}

/// A setting of a clip that has keyframes (see [`Edit::ClipGainKeys`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Keyed {
    /// Its volume.
    #[default]
    GainDb,
    /// Its pan.
    Pan,
}

/// What [`Edit::ClipGainKeys`] does to a keyframe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyAction {
    /// Add one, or set the one there.
    Add,
    /// Take one away.
    Remove,
    /// Move one to another time, and to another value where one is given.
    Move,
    /// Set how one comes in and goes out.
    SetInterp,
}

/// An attribute of a clip that is pasted or taken away (pp. 3842, 3803–3804).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Attribute {
    /// Its volume, keys and all.
    Volume,
    /// Its effects.
    Plugins,
    /// Its equalizer.
    Equalizer,
}

#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde passes a reference"
)]
fn is_false(value: &bool) -> bool {
    !*value
}

/// What [`Edit::CloseGap`] does with a clip or subtitle of another track that lies across the
/// gap, in some of its frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Straddling {
    /// Refuse the edit with `ripple_blocked`, as every ripple does.
    #[default]
    Refuse,
    /// Leave the clip where it is, as the Cut page of `DaVinci` Resolve closes the gaps of its
    /// first track: what starts after the gap still moves back with the gap, so that
    /// superimposed clips stay in step with what is under them, and subtitles in the gap are cut
    /// out with it as every ripple cuts them. Where that would run a clip into one that stays,
    /// on its track, the edit is refused with `ripple_blocked` naming the one that stays.
    Stay,
}

impl Straddling {
    #[allow(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde passes a reference"
    )]
    fn is_refuse(&self) -> bool {
        *self == Self::Refuse
    }
}

/// A copy of [`Edit::PlaceCopies`]: of which clip, on which track and from which frame.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CopyOf {
    /// The clip copied.
    pub clip: Id,
    /// The track that the copy goes on, of the clip's kind.
    pub track: Id,
    /// The frame that it starts at.
    pub start: i64,
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

/// Read a field that must be there and may be `null`.
fn nullable_required<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(deserializer)
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
    let edit = match edit {
        Edit::AddSerial {
            grade,
            node,
            windows,
        } => return grade::add(project, &grade, &node, grade::Beside::After(windows)),
        Edit::AddSerialBefore { grade, node } => {
            return grade::add(project, &grade, &node, grade::Beside::Before)
        }
        Edit::AppendNode { grade } => return grade::append_node(project, &grade),
        Edit::AddParallel { grade, node } => {
            return grade::add(project, &grade, &node, grade::Beside::Parallel)
        }
        Edit::AddLayer { grade, node } => {
            return grade::add(project, &grade, &node, grade::Beside::Layer)
        }
        Edit::AddOutside { grade, node } => return grade::add_outside(project, &grade, &node),
        Edit::ExtractNode { grade, node } => return grade::extract(project, &grade, &node),
        Edit::ResetNode { grade, node } => return grade::reset(project, &grade, &node),
        Edit::CopyGrade {
            from,
            to,
            preserve_nodes,
            keyframes,
        } => {
            let (preserve, align) = (preserve_nodes.unwrap_or(0), keyframes.unwrap_or_default());
            return grade::copy_grade(project, &from, &to, preserve, align);
        }
        Edit::AppendNodeGraph {
            from,
            to,
            keyframes,
        } => return grade::append_node_graph(project, &from, &to, keyframes.unwrap_or_default()),
        Edit::CopyGivenGrade {
            given,
            to,
            preserve_nodes,
        } => return grade::copy_given(project, &given, &to, preserve_nodes),
        Edit::AppendGivenGraph { given, to } => return grade::append_given(project, &given, &to),
        Edit::CollapseGroupGrades { timeline, clips } => {
            return grade::collapse_group_grades(project, &timeline, &clips)
        }
        Edit::PasteValue {
            from,
            from_node,
            to,
            to_node,
            param,
        } => return grade::paste_value(project, (&from, &from_node), (&to, &to_node), &param),
        Edit::PasteNode {
            from,
            from_node,
            to,
            to_node,
        } => return grade::paste_node(project, (&from, &from_node), (&to, &to_node)),
        edit => edit,
    };
    input::check(&edit)?;
    let timeline = timeline_of(&edit)
        .expect("an edit of a timeline names its timeline")
        .clone();
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
        Edit::Lift {
            clips,
            linked,
            grouped,
            ..
        } => cut::lift(&mut work, &clips, linked, grouped),
        Edit::RippleDelete {
            clips,
            linked,
            grouped,
            ..
        } => cut::ripple_delete(&mut work, &clips, linked, grouped),
        Edit::RippleDeleteRange {
            from,
            to,
            tracks,
            grouped,
            ..
        } => cut::ripple_delete_range(&mut work, (from, to), tracks.as_deref(), grouped),
        Edit::CloseGap {
            track,
            at,
            straddling,
            ..
        } => cut::close_gap(&mut work, &track, at, straddling),
        Edit::Blade {
            at,
            tracks,
            linked,
            grouped,
            ..
        } => cut::blade(&mut work, at, tracks.as_deref(), (linked, grouped)),
        Edit::Move {
            clips,
            delta,
            to_track,
            overwrite,
            linked,
            grouped,
            ..
        } => cut::move_clips(
            &mut work,
            &clips,
            (delta, to_track.as_deref()),
            overwrite,
            (linked, grouped),
        ),
        Edit::InsertMove {
            clips,
            at,
            linked,
            grouped,
            ..
        } => cut::insert_move(&mut work, &clips, at, (linked, grouped)),
        Edit::PlaceCopies {
            copies, grouped, ..
        } => cut::place_copies(&mut work, &copies, grouped),
        Edit::CutClips {
            clips,
            from,
            to,
            linked,
            grouped,
            ..
        } => clipboard::cut(&mut work, &clips, (from, to), linked, grouped),
        Edit::PasteClips {
            clipboard,
            at,
            tracks,
            ..
        } => clipboard::paste(&mut work, &clipboard, at, &tracks),
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
            linked,
            ..
        } => misc::add_transition(&mut work, &left, &right, kind, length, align, linked),
        Edit::RemoveTransition { id, .. } => misc::remove_transition(&mut work, &id),
        Edit::SetTransition {
            id,
            kind,
            length,
            align,
            fade_in_level_db,
            fade_out_level_db,
            ..
        } => misc::set_transition(
            &mut work,
            &id,
            (kind, length, align),
            (fade_in_level_db, fade_out_level_db),
        ),
        Edit::SetSpeed {
            clip,
            speed,
            reverse,
            ripple,
            linked,
            ..
        } => misc::set_speed(&mut work, &clip, speed, reverse, ripple, linked),
        Edit::AddSpeedPoint {
            clip, at, linked, ..
        } => retiming::retime(&mut work, &clip, Some(at), Retiming::Point, false, linked),
        Edit::SetSegmentSpeed {
            clip,
            at,
            speed,
            ripple,
            linked,
            ..
        } => {
            let change = Retiming::Speed(speed);
            retiming::retime(&mut work, &clip, Some(at), change, ripple, linked)
        }
        Edit::FreezeFrame {
            clip,
            at,
            hold,
            ripple,
            linked,
            ..
        } => {
            let change = Retiming::Freeze(hold);
            retiming::retime(&mut work, &clip, Some(at), change, ripple, linked)
        }
        Edit::ReverseSegment {
            clip, at, linked, ..
        } => retiming::retime(&mut work, &clip, Some(at), Retiming::Reverse, false, linked),
        Edit::ClearSpeedPoint {
            clip, at, linked, ..
        } => retiming::retime(&mut work, &clip, Some(at), Retiming::Clear, false, linked),
        Edit::ResetSpeed {
            clip,
            ripple,
            linked,
            ..
        } => retiming::retime(&mut work, &clip, None, Retiming::Reset, ripple, linked),
        Edit::SetSpeedPoints {
            clip,
            retime,
            keep,
            linked,
            ..
        } => retiming::set_points(&mut work, &clip, &retime, keep, linked),
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
        Edit::SetTrackAudio {
            track,
            patch,
            follow_groups,
            ..
        } => audio::set_track_audio(&mut work, &track, &patch, follow_groups),
        Edit::ClipGainKeys {
            clip,
            param,
            action,
            at,
            to,
            value,
            interp,
            ..
        } => audio::clip_gain_keys(&mut work, &clip, param, action, at, to, value, interp),
        Edit::ClipGainRange {
            clip,
            from,
            to,
            delta_db,
            ..
        } => audio::clip_gain_range(&mut work, &clip, from, to, delta_db),
        Edit::NudgeClipGain {
            clips,
            delta_db,
            linked,
            ..
        } => audio::nudge_clip_gain(&mut work, &clips, delta_db, linked),
        Edit::PasteAttributes {
            from, to, items, ..
        } => audio::paste_attributes(&mut work, &from, &to, &items),
        Edit::RemoveAttributes { clips, items, .. } => {
            audio::remove_attributes(&mut work, &clips, &items)
        }
        Edit::FadeToPlayhead {
            clips, side, at, ..
        } => audio::fade_to_playhead(&mut work, &clips, side, at),
        Edit::SetBus { id, patch, .. } => audio::set_bus(&mut work, &id, &patch),
        Edit::CopyEffects { from, to, .. } => audio::copy_effects(&mut work, &from, &to),
        Edit::SetInsert {
            of, slot, insert, ..
        } => automation::set_insert(&mut work, &of, slot, insert.as_ref()),
        Edit::MoveInsert {
            of,
            from,
            to,
            to_of,
            ..
        } => automation::move_insert(&mut work, &of, (from, to), to_of.as_ref()),
        Edit::LanePoints {
            of,
            path,
            action,
            points,
            at,
            by,
            delta,
            grouped,
            ..
        } => {
            let when = points.first().map(|p| p.at);
            automation::with_group(
                &mut work,
                (&of, &path),
                (grouped, when),
                |work, of, follow| {
                    let points: Vec<NewPoint> = points
                        .iter()
                        .map(|p| NewPoint {
                            at: p.at,
                            value: p.value.map(|v| follow.apply(v)),
                        })
                        .collect();
                    automation::lane_points(work, of, &path, action, &points, &at, (by, delta))
                },
            )
        }
        Edit::LaneRange {
            of,
            path,
            from,
            to,
            action,
            points,
            delta,
            tolerance,
            grouped,
            ..
        } => automation::with_group(
            &mut work,
            (&of, &path),
            (grouped, Some(from)),
            |work, of, follow| {
                let points: Option<Vec<LanePoint>> = points.as_ref().map(|points| {
                    points
                        .iter()
                        .map(|p| LanePoint {
                            at: p.at,
                            value: follow.apply(p.value),
                        })
                        .collect()
                });
                automation::lane_range(
                    work,
                    of,
                    &path,
                    (from, to),
                    action,
                    points.as_deref(),
                    (delta, tolerance),
                )
            },
        ),
        Edit::LaneDraw {
            of,
            path,
            from,
            to,
            points,
            tolerance,
            grouped,
            ..
        } => automation::with_group(
            &mut work,
            (&of, &path),
            (grouped, Some(from)),
            |work, of, follow| {
                let points: Vec<LanePoint> = points
                    .iter()
                    .map(|p| LanePoint {
                        at: p.at,
                        value: follow.apply(p.value),
                    })
                    .collect();
                automation::lane_draw(work, of, &path, (from, to), &points, tolerance)
            },
        ),
        Edit::LaneFill {
            lanes,
            from,
            to,
            glide,
            ..
        } => automation::lane_fill(&mut work, &lanes, (from, to), glide),
        Edit::AutomationPaste { at, to, lanes, .. } => {
            automation::automation_paste(&mut work, &lanes, at, to)
        }
        Edit::AutomationErase { of, from, to, .. } => {
            automation::automation_erase(&mut work, &of, (from, to))
        }
        Edit::ClearLanes {
            of,
            paths,
            then,
            at,
            ..
        } => automation::clear_lanes(&mut work, &of, paths.as_deref(), then, at),
        Edit::RecordPass { lanes, .. } => automation::record_pass(&mut work, &lanes),
        _ => unreachable!("the edits of grades are worked out above"),
    }?;
    work.finish()
}

/// The timeline that an edit is of, or none for an edit of grades, which may be of none.
#[must_use]
pub fn timeline_of(edit: &Edit) -> Option<&Id> {
    let timeline = match edit {
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
        | Edit::PlaceCopies { timeline, .. }
        | Edit::PasteClips { timeline, .. }
        | Edit::CutClips { timeline, .. }
        | Edit::Trim { timeline, .. }
        | Edit::Roll { timeline, .. }
        | Edit::Slip { timeline, .. }
        | Edit::Slide { timeline, .. }
        | Edit::AddTransition { timeline, .. }
        | Edit::RemoveTransition { timeline, .. }
        | Edit::SetTransition { timeline, .. }
        | Edit::SetSpeed { timeline, .. }
        | Edit::AddSpeedPoint { timeline, .. }
        | Edit::SetSegmentSpeed { timeline, .. }
        | Edit::FreezeFrame { timeline, .. }
        | Edit::ReverseSegment { timeline, .. }
        | Edit::ClearSpeedPoint { timeline, .. }
        | Edit::ResetSpeed { timeline, .. }
        | Edit::SetSpeedPoints { timeline, .. }
        | Edit::AddCue { timeline, .. }
        | Edit::SetCue { timeline, .. }
        | Edit::RemoveCue { timeline, .. }
        | Edit::CuesFrom { timeline, .. }
        | Edit::AddTrack { timeline, .. }
        | Edit::RemoveTrack { timeline, .. }
        | Edit::SetTrackAudio { timeline, .. }
        | Edit::ClipGainKeys { timeline, .. }
        | Edit::ClipGainRange { timeline, .. }
        | Edit::NudgeClipGain { timeline, .. }
        | Edit::PasteAttributes { timeline, .. }
        | Edit::RemoveAttributes { timeline, .. }
        | Edit::FadeToPlayhead { timeline, .. }
        | Edit::SetBus { timeline, .. }
        | Edit::CopyEffects { timeline, .. }
        | Edit::SetInsert { timeline, .. }
        | Edit::MoveInsert { timeline, .. }
        | Edit::LanePoints { timeline, .. }
        | Edit::LaneRange { timeline, .. }
        | Edit::LaneDraw { timeline, .. }
        | Edit::LaneFill { timeline, .. }
        | Edit::AutomationPaste { timeline, .. }
        | Edit::AutomationErase { timeline, .. }
        | Edit::RecordPass { timeline, .. }
        | Edit::ClearLanes { timeline, .. } => timeline,
        Edit::AddSerial { .. }
        | Edit::AddSerialBefore { .. }
        | Edit::AppendNode { .. }
        | Edit::AddParallel { .. }
        | Edit::AddLayer { .. }
        | Edit::AddOutside { .. }
        | Edit::ExtractNode { .. }
        | Edit::ResetNode { .. }
        | Edit::CopyGrade { .. }
        | Edit::AppendNodeGraph { .. }
        | Edit::CopyGivenGrade { .. }
        | Edit::AppendGivenGraph { .. }
        | Edit::CollapseGroupGrades { .. }
        | Edit::PasteValue { .. }
        | Edit::PasteNode { .. } => return None,
    };
    Some(timeline)
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
            // Setting media, the colour settings, the composition of a clip, a link, a node or
            // the keys of a setting of it, or the stabilizer of a clip or some of its controls
            // (none among them), to what they are already changes nothing, and makes no step.
            if matches!(
                done,
                Op::SetMedia { .. }
                    | Op::SetMediaColor { .. }
                    | Op::SetMediaStreams { .. }
                    | Op::SetColorSettings { .. }
                    | Op::SetFusion { .. }
                    | Op::ConnectFusion { .. }
                    | Op::SetFusionNode { .. }
                    | Op::SetFusionKeys { .. }
                    | Op::SpliceFusionKeys { .. }
                    | Op::SetStabilize { .. }
                    | Op::SetStabilizeSettings { .. }
            ) && done == inverse
            {
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

    #[test]
    fn an_edit_of_clips_takes_the_linked_clips_too_unless_it_is_told_not_to() {
        let read = |json| serde_json::from_value::<Edit>(json).unwrap();
        let edit = read(serde_json::json!({ "edit": "lift", "timeline": "t1", "clips": ["c1"] }));
        assert!(matches!(edit, Edit::Lift { linked: true, .. }));
        let edit = read(serde_json::json!({
            "edit": "lift", "timeline": "t1", "clips": ["c1"], "linked": false,
        }));
        assert!(matches!(edit, Edit::Lift { linked: false, .. }));
    }

    #[test]
    fn a_track_of_an_edit_is_told_apart_as_left_out_null_or_named() {
        let read = |track: Option<serde_json::Value>| {
            let mut json = serde_json::json!({
                "edit": "overwrite", "timeline": "t1", "at": 0,
                "source": { "kind": "media", "media": "m1" },
            });
            if let Some(track) = track {
                json["video_track"] = track;
            }
            match serde_json::from_value::<Edit>(json).unwrap() {
                Edit::Overwrite { video_track, .. } => video_track,
                edit => panic!("{edit:?}"),
            }
        };
        assert_eq!(read(None), None);
        assert_eq!(read(Some(serde_json::Value::Null)), Some(None));
        assert_eq!(read(Some("V2".into())), Some(Some("V2".into())));
    }

    #[test]
    fn an_edit_with_no_track_to_go_on_names_the_kind_of_track() {
        for (kind, name) in [
            (TrackKind::Video, "video"),
            (TrackKind::Audio, "audio"),
            (TrackKind::Subtitle, "subtitle"),
        ] {
            let written = serde_json::to_value(EditError::NoTrack { kind }).unwrap();
            assert_eq!(
                written["message"],
                format!("there is no {name} track to put it on")
            );
        }
    }
}
