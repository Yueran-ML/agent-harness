//! A project of the studio: the media that it is cut from, kept in bins, and its timelines,
//! each a stack of tracks of clips, transitions and subtitles.
//!
//! This is the form in which a project is written and read, by the engine and by every client
//! of it, so the names of the fields are kept as they are. A field that is not known is refused,
//! and so is one that is left out, but for the proxy of media and what schema 3 adds; and so is
//! a project of another schema. A project of schema 1 or 2 is read into this one by
//! [`crate::upgrade`].
//!
//! Schema 3 added the grades of the Color page (see [`crate::grade`]): a grade on each clip and
//! on each timeline, and the groups that clips are graded in. They are always written, and may
//! be left out where a clip, a timeline or a project is read, as by a client that builds one
//! without knowing grades: a clip or a timeline is then read with a grade that changes nothing
//! and in no group, and a project with no groups. Settings that came later within schema 3 are
//! fields that may be left out so too: the colour management of the project (`color`, see
//! [`crate::color_management`]), the colour of media set by hand (`color_override`), and the
//! Blur palette of a corrector (`blur`, see [`crate::grade::blur`]). A project written before
//! them is read with the values that leave it as it was: not managed, no media set, and no
//! blur.
//!
//! Schema 4 added the sound of the Fairlight page (see [`crate::audio`]): on a clip, keyframes of
//! its gain and pan, fades of any length and curve, an equalizer, effects, a pitch and a map of
//! its channels; on a track of sound, a format, a mute and a solo, a strip and lanes of
//! automation; on a timeline, buses, VCAs, groups of tracks and the settings of its sound; on a
//! transition, the levels of a crossfade; and in the facts of media, its streams of sound. They
//! are always written, and may each be left out where a project is read, as its default. A
//! project of schema 3 is read into schema 4 so that it sounds as it did (see
//! [`crate::upgrade`]).

use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::audio::{
    self, AudioFade, AudioParam, Bus, ChannelMap, ChannelOrder, ClipPitch, CrossfadeLevel,
    Equalizer, Insert, Lanes, Strip, TimelineAudio, TrackAudio, TrackFormat, TrackGroup, Vca,
};
use crate::color::ColorFacts;
use crate::color_management::{ColorOverride, ColorSettings};
use crate::grade::{Grade, Group};
use crate::ops::Invalid;
use crate::time::{Rate, Speed, Time};

/// The schema of a project that this engine reads and writes.
pub const SCHEMA: u32 = 4;

/// What a thing of a project is known by. It never changes.
///
/// The id that the engine makes for a bin, media, a timeline, a clip, a transition, a subtitle,
/// a group of linked clips, a group of clips graded together or a node of a grade is never made
/// again, so it is never given to another thing by the engine. The id of a track is not kept so: it names the kind of the track and its place among
/// the tracks of that kind (`V1`, `A2`, `S1`), and a track that is added after `V2` was taken
/// away may be `V2` again.
pub type Id = String;

/// A project of the studio.
///
/// A project is read with [`Project::from_value`], which refuses one of another schema before
/// anything else of it is looked at, and so names that schema rather than a field it does not
/// know. A project that is deserialized directly, as inside a larger value, is checked too: it
/// is refused if it is of another schema or if [`Project::validate`] refuses it, and is put
/// right as `validate` puts it right. So no project is read that the operations could not have
/// made.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawProject")]
pub struct Project {
    /// The schema that the project is written in.
    pub schema: u32,
    /// The number that the next id that the engine makes ends in, for a bin, media, a timeline,
    /// a clip, a transition, a subtitle or a group of linked clips. Never goes back. Tracks are
    /// named otherwise, and do not use it.
    ///
    /// An undo does not take it back: such an id is never made twice, so one that a client has
    /// seen is never given to another thing after an undo. Two projects compared across an undo
    /// are the same if they differ only in this. When a project is read, it is raised above the
    /// number of every id of the project that the engine could have made.
    pub next_id: u64,
    /// The bins that the media is kept in, each in another one or at the root.
    pub bins: Vec<Bin>,
    /// The files that the project is cut from.
    pub media: Vec<Media>,
    /// The timelines of the project.
    pub timelines: Vec<Timeline>,
    /// The groups that clips are graded in. It may be left out where a project is read.
    pub groups: Vec<Group>,
    /// How its colours are managed (see [`crate::color_management`]). It may be left out where
    /// a project is read, and so may any of its fields: the project is then not managed.
    pub color: ColorSettings,
}

/// A project as it is written, before it is checked.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProject {
    schema: u32,
    next_id: u64,
    bins: Vec<Bin>,
    media: Vec<Media>,
    timelines: Vec<Timeline>,
    #[serde(default)]
    groups: Vec<Group>,
    #[serde(default)]
    color: ColorSettings,
}

impl TryFrom<RawProject> for Project {
    type Error = ReadError;

    fn try_from(raw: RawProject) -> Result<Self, Self::Error> {
        if raw.schema != SCHEMA {
            return Err(ReadError::Schema(raw.schema.into()));
        }
        let mut project = Self {
            schema: raw.schema,
            next_id: raw.next_id,
            bins: raw.bins,
            media: raw.media,
            timelines: raw.timelines,
            groups: raw.groups,
            color: raw.color,
        };
        project.validate()?;
        Ok(project)
    }
}

/// Read a field that may be `null` but may not be left out, as serde would read an `Option`
/// that is left out as none.
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(deserializer)
}

/// A bin of the media pool, as an editor shows it: a folder that media and other bins are kept
/// in. It is only a name; the files stay where they are.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bin {
    /// What it is known by.
    pub id: Id,
    /// The name that it is shown by.
    pub name: String,
    /// The bin that it is in, or none if it is at the root.
    #[serde(deserialize_with = "present")]
    pub parent: Option<Id>,
}

/// A file that the project is cut from. It is listed by its path and never copied.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Media {
    /// What the clips name it by.
    pub id: Id,
    /// Where the file is. A path that is not UTF-8 is refused when the file is taken in.
    pub path: String,
    /// What the file was when it was taken in: its size, the time it was written, and a digest.
    pub identity: Identity,
    /// What was found in the file when it was taken in.
    pub facts: Facts,
    /// A copy of its picture that is quick to seek in, made by the engine, if one was made. It
    /// may be left out, and is not written when there is none, so that a project written
    /// before proxies were made is read as one without them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<Proxy>,
    /// The name that it is shown by: the name of its file when it is taken in.
    pub name: String,
    /// The bin that it is kept in, or none if it is at the root.
    #[serde(deserialize_with = "present")]
    pub bin: Option<Id>,
    /// Where it is marked to begin, for an edit of three points, in ticks of the scale of its
    /// duration.
    #[serde(deserialize_with = "present")]
    pub mark_in: Option<Time>,
    /// Where it is marked to end, in ticks of the scale of its duration; after `mark_in`. The
    /// range marked is half open: this is the start of the frame after the Out frame, which is in
    /// it, as the Out point of `DaVinci Resolve` is the last frame of the range (Reference Manual
    /// 20, pp. 760, 824). A client that marks frame `n` as the Out keeps the start of `n + 1`.
    #[serde(deserialize_with = "present")]
    pub mark_out: Option<Time>,
    /// Its input colour space and data levels set by hand (see
    /// [`crate::color_management::ColorOverride`]). It may be left out where it is read, as
    /// none set.
    #[serde(default)]
    pub color_override: ColorOverride,
}

/// A copy of the picture of media whose every frame is a key frame, smaller where the picture is
/// larger than 1920 pixels on its long edge, made by the engine so that its frames are sought,
/// scrubbed and played backward at once. It has the frames of the media, at the same rate,
/// numbered alike; the sound is still read from the media itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proxy {
    /// Where the file of the copy is: an absolute path.
    pub path: String,
    /// Its width in pixels, as it is stored.
    pub width: u32,
    /// Its height in pixels, as it is stored.
    pub height: u32,
    /// The name of its codec, as ffprobe names it (`hevc`, `prores`).
    pub codec: String,
    /// The digest of the identity of the media that it was made from (see [`Identity`]): a
    /// copy whose digest is not that of its media is of another file, and is made again.
    pub digest_of_source: String,
}

/// What tells a file from another one at the same path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    /// Its size in bytes.
    pub size: u64,
    /// When it was written last, in microseconds since the start of 1970 (UTC).
    pub modified_us: i64,
    /// The SHA-256, in lowercase hexadecimal, of its size written in decimal, its first 64 KiB,
    /// and its last 64 KiB where it is longer than that (from no earlier than byte 65536), as
    /// `columbina.video.project.identity_of` makes it, so that a project of the earlier editing
    /// keeps the identities of its sources.
    pub digest: String,
}

/// What a file holds.
///
/// A file without a picture has no `video`, and one without sound has no `audio`. Each may be
/// written as `null` or left out; either is read as none.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Facts {
    /// How long it lasts.
    pub duration: Time,
    /// Its picture, if it has one.
    pub video: Option<VideoFacts>,
    /// Its sound, if it has any.
    pub audio: Option<AudioFacts>,
}

/// The picture of a file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VideoFacts {
    /// A single still image, held for the length of its timeline clip.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub still: bool,
    /// Its width in pixels, as it is stored.
    pub width: u32,
    /// Its height in pixels, as it is stored.
    pub height: u32,
    /// Its rate of frames.
    pub rate: Rate,
    /// The name of its codec.
    pub codec: String,
    /// The bits of each sample of a pixel.
    pub bits: u8,
    /// How far the picture is turned clockwise to be shown upright, in degrees: 0, 90, 180 or 270.
    pub rotation: i16,
    /// What the file says of its colours. None where they were not read, as in a project whose
    /// media were probed before colours were: left out of the JSON, and read so where it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<ColorFacts>,
}

/// The sound of a file: that of the stream of sound that libav ranks best, and the streams of
/// sound that it has.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioFacts {
    /// Its samples a second.
    pub rate: u32,
    /// How many channels it has.
    pub channels: u16,
    /// The name of its codec.
    pub codec: String,
    /// The name of the layout of its channels, as libav names it (`stereo`, `5.1(side)`), if
    /// one was read. None where it was not, as in a project whose media was probed before
    /// schema 4: left out of the JSON, and read so where it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
    /// Each stream of sound of the file, in the order of the file, if they were read. None where
    /// they were not, as for `layout`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streams: Option<Vec<AudioStream>>,
}

/// A stream of sound of a file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioStream {
    /// Its number among all the streams of the file, as libav numbers them.
    pub index: u32,
    /// How many channels it has.
    pub channels: u16,
    /// The name of the layout of its channels, if it has one.
    #[serde(default)]
    pub layout: Option<String>,
}

/// A timeline: tracks of clips and subtitles, laid on frames of one rate, and the picture and
/// the sound that it is played and rendered in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Timeline {
    /// What it is known by.
    pub id: Id,
    /// The name that it is shown by.
    pub name: String,
    /// The rate of its frames.
    pub rate: Rate,
    /// The width of its picture in pixels: an even number from 16 to 16384.
    pub width: u32,
    /// The height of its picture in pixels: an even number from 16 to 16384.
    pub height: u32,
    /// The samples a second of its sound: 44100, 48000 or 96000.
    pub sample_rate: u32,
    /// Its tracks. Those of picture are stacked in their order, the first at the bottom.
    pub tracks: Vec<Track>,
    /// Its grade, which colours the picture once its tracks are put together. It may be left out
    /// where a timeline is read.
    #[serde(default)]
    pub grade: Grade,
    /// Its buses (see [`crate::audio::routing`]). It may be left out where a timeline is read, as
    /// the one bus of a new timeline, `B1`.
    #[serde(default = "audio::first_buses")]
    pub buses: Vec<Bus>,
    /// Its VCAs. It may be left out, as none.
    #[serde(default)]
    pub vcas: Vec<Vca>,
    /// Its groups of tracks of sound. It may be left out, as none.
    #[serde(default)]
    pub track_groups: Vec<TrackGroup>,
    /// The settings of its sound. It may be left out, as those of a new timeline.
    #[serde(default)]
    pub audio: TimelineAudio,
}

/// What a track holds: picture, sound or subtitles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    /// Picture.
    Video,
    /// Sound.
    Audio,
    /// Subtitles.
    Subtitle,
}

/// A track of a timeline, and what is on it.
///
/// A track of picture or of sound holds clips and the transitions between them, and is written
/// with `clips` and `transitions`; a track of subtitles holds subtitles in a language, and is
/// written with `language` and `cues`. A track of sound is written with what it has of sound too
/// (see [`TrackAudio`]): `format`, `channels`, `order`, `mute`, `solo`, `solo_safe`, `adc`,
/// `automation_arm`, `strip` and `lanes`, each of which may be left out where it is read (the
/// `channels` of an adaptive track excepted). A track is written with the fields of its kind
/// only, and one that is read with the fields of another kind is refused. The fields of the other
/// kinds are empty here, and `audio` is the default.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(try_from = "RawTrack")]
pub struct Track {
    /// What it is known by.
    pub id: Id,
    /// Whether it holds picture, sound or subtitles.
    pub kind: TrackKind,
    /// The name that it is shown by.
    pub name: String,
    /// Whether it is seen or heard. A track that is not is left out when the timeline is
    /// played or rendered.
    pub enabled: bool,
    /// Whether it is locked, so that no edit changes it. An operation changes it all the same,
    /// so that every change can be undone.
    pub locked: bool,
    /// Its clips, in the order of their start.
    pub clips: Vec<Clip>,
    /// The transitions between its clips, in the order of the cuts that they are at.
    pub transitions: Vec<Transition>,
    /// The language of its subtitles, as a short tag of BCP 47 (`zh`, `en-GB`), or empty.
    pub language: String,
    /// Its subtitles, in the order of their start.
    pub cues: Vec<Cue>,
    /// What it has of sound, if it is a track of sound.
    pub audio: TrackAudio,
}

/// A track as it is written, before the fields of its kind are looked at.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTrack {
    id: Id,
    kind: TrackKind,
    name: String,
    enabled: bool,
    locked: bool,
    clips: Option<Vec<Clip>>,
    transitions: Option<Vec<Transition>>,
    language: Option<String>,
    cues: Option<Vec<Cue>>,
    format: Option<TrackFormat>,
    channels: Option<u8>,
    order: Option<ChannelOrder>,
    mute: Option<bool>,
    solo: Option<bool>,
    solo_safe: Option<bool>,
    adc: Option<bool>,
    automation_arm: Option<crate::audio::AutomationArm>,
    strip: Option<Strip>,
    lanes: Option<Lanes>,
}

impl TryFrom<RawTrack> for Track {
    type Error = String;

    fn try_from(raw: RawTrack) -> Result<Self, Self::Error> {
        let kind = raw.kind;
        let name = match kind {
            TrackKind::Video => "video",
            TrackKind::Audio => "audio",
            TrackKind::Subtitle => "subtitle",
        };
        let subtitles = kind == TrackKind::Subtitle;
        let fields = [
            ("clips", raw.clips.is_some(), !subtitles),
            ("transitions", raw.transitions.is_some(), !subtitles),
            ("language", raw.language.is_some(), subtitles),
            ("cues", raw.cues.is_some(), subtitles),
        ];
        for (field, there, wanted) in fields {
            if there && !wanted {
                return Err(format!("a {name} track has no `{field}`"));
            }
            if wanted && !there {
                return Err(format!("missing field `{field}` of a {name} track"));
            }
        }
        let sound = [
            ("format", raw.format.is_some()),
            ("channels", raw.channels.is_some()),
            ("order", raw.order.is_some()),
            ("mute", raw.mute.is_some()),
            ("solo", raw.solo.is_some()),
            ("solo_safe", raw.solo_safe.is_some()),
            ("adc", raw.adc.is_some()),
            ("automation_arm", raw.automation_arm.is_some()),
            ("strip", raw.strip.is_some()),
            ("lanes", raw.lanes.is_some()),
        ];
        if kind != TrackKind::Audio {
            if let Some((field, _)) = sound.iter().find(|(_, there)| *there) {
                return Err(format!("a {name} track has no `{field}`"));
            }
        }
        let made = TrackAudio::default();
        let format = raw.format.unwrap_or(made.format);
        let Some(channels) = raw.channels.or(format.channels()) else {
            return Err("missing field `channels` of an adaptive track".into());
        };
        let audio = TrackAudio {
            format,
            channels,
            order: raw.order.unwrap_or(made.order),
            mute: raw.mute.unwrap_or(made.mute),
            solo: raw.solo.unwrap_or(made.solo),
            solo_safe: raw.solo_safe.unwrap_or(made.solo_safe),
            adc: raw.adc.unwrap_or(made.adc),
            automation_arm: raw.automation_arm.unwrap_or(made.automation_arm),
            strip: match (raw.strip, kind) {
                (Some(strip), _) => strip,
                (None, TrackKind::Audio) => Strip::to_first_bus_if_any(),
                (None, _) => made.strip,
            },
            lanes: raw.lanes.unwrap_or(made.lanes),
        };
        Ok(Self {
            id: raw.id,
            kind,
            name: raw.name,
            enabled: raw.enabled,
            locked: raw.locked,
            clips: raw.clips.unwrap_or_default(),
            transitions: raw.transitions.unwrap_or_default(),
            language: raw.language.unwrap_or_default(),
            cues: raw.cues.unwrap_or_default(),
            audio,
        })
    }
}

impl Serialize for Track {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut track = serializer.serialize_struct("Track", 17)?;
        track.serialize_field("id", &self.id)?;
        track.serialize_field("kind", &self.kind)?;
        track.serialize_field("name", &self.name)?;
        track.serialize_field("enabled", &self.enabled)?;
        track.serialize_field("locked", &self.locked)?;
        if self.kind == TrackKind::Subtitle {
            track.serialize_field("language", &self.language)?;
            track.serialize_field("cues", &self.cues)?;
        } else {
            track.serialize_field("clips", &self.clips)?;
            track.serialize_field("transitions", &self.transitions)?;
        }
        if self.kind == TrackKind::Audio {
            let audio = &self.audio;
            track.serialize_field("format", &audio.format)?;
            track.serialize_field("channels", &audio.channels)?;
            track.serialize_field("order", &audio.order)?;
            track.serialize_field("mute", &audio.mute)?;
            track.serialize_field("solo", &audio.solo)?;
            track.serialize_field("solo_safe", &audio.solo_safe)?;
            track.serialize_field("adc", &audio.adc)?;
            track.serialize_field("automation_arm", &audio.automation_arm)?;
            track.serialize_field("strip", &audio.strip)?;
            track.serialize_field("lanes", &audio.lanes)?;
        }
        track.end()
    }
}

/// A stretch of a source on a track: at frame `start` of the timeline, for `length` frames. The
/// clips of a track are kept in the order of their start.
///
/// The source is media, from a place in it, or a title or a colour that the engine makes, which
/// go on tracks of picture only. A clip of media shows its frames at its speed, forward or
/// backward (see [`crate::span`]). Its transform is used on a track of picture, and its settings
/// of sound on a track of sound; they are kept on the other kind too, as they were set.
///
/// The ticks that its frames last are counted rounded down, so its last frame may end less than
/// one tick past the end of its source; whoever reads the source for it stops at the end.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Clip {
    /// What it is known by.
    pub id: Id,
    /// The frame of the timeline that it begins at.
    pub start: i64,
    /// How many frames it lasts.
    pub length: i64,
    /// What it shows or plays.
    pub source: Source,
    /// Whether it is seen or heard.
    pub enabled: bool,
    /// How fast it plays its source.
    pub speed: Speed,
    /// Whether it plays its source backward.
    pub reverse: bool,
    /// The group of linked clips that it is in, if any: the clips of a group, picture and sound
    /// of one source, are selected and moved together.
    #[serde(deserialize_with = "present")]
    pub link: Option<Id>,
    /// Where its picture is put on the picture of the timeline.
    pub transform: Transform,
    /// How much louder its sound is played, in decibels: from -96 to 24. One number, or keyframes
    /// at times of its source (see [`AudioParam`]).
    pub gain_db: AudioParam,
    /// Where its sound is put between the left and the right: from -1, left, to 1, right. One
    /// number, or keyframes.
    pub pan: AudioParam,
    /// How many frames its picture fades in over, and its sound where `audio_fade_in` is none,
    /// from its start.
    pub fade_in: i64,
    /// How many frames its picture fades out over, and its sound where `audio_fade_out` is none,
    /// to its end.
    pub fade_out: i64,
    /// How its sound fades in, if not over `fade_in` frames on a straight line. It may be left
    /// out, as none.
    #[serde(default)]
    pub audio_fade_in: Option<AudioFade>,
    /// How its sound fades out, if not over `fade_out` frames on a straight line. It may be left
    /// out, as none.
    #[serde(default)]
    pub audio_fade_out: Option<AudioFade>,
    /// Its equalizer, of 4 bands (p. 3827). It may be left out, as that of a new clip: off.
    #[serde(default = "Equalizer::clip")]
    pub eq: Equalizer,
    /// Its effects, in order (p. 986). It may be left out, as none.
    #[serde(default)]
    pub effects: Vec<Insert>,
    /// Whether a change of its speed keeps the pitch of its sound (p. 3827). It may be left out,
    /// as not: its pitch goes with its speed, as it did before schema 4.
    #[serde(default)]
    pub pitch_correction: bool,
    /// How far its pitch is moved, apart from its speed (p. 3826). It may be left out, as not.
    #[serde(default)]
    pub pitch: ClipPitch,
    /// What each channel of its track plays (pp. 3833–3836), as many as its track has; or none,
    /// for the channels of its media mixed to those of its track as libav mixes them. It may be
    /// left out, as none.
    #[serde(default)]
    pub channels: Option<Vec<ChannelMap>>,
    /// Its grade, which colours its picture before it is put on the tracks below it. It is kept
    /// on a clip of sound too, as it was set. It may be left out where a clip is read.
    #[serde(default)]
    pub grade: Grade,
    /// The group that it is graded in, if any. It may be left out where a clip is read.
    #[serde(default)]
    pub group: Option<Id>,
}

/// What a clip shows or plays.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Source {
    /// Media, from a place in it.
    Media {
        /// The media.
        media: Id,
        /// Where the clip begins in it, in ticks of the scale of its duration.
        #[serde(rename = "in")]
        source_in: Time,
    },
    /// A title: text that the engine lays out and draws.
    Title {
        /// What it says.
        text: String,
        /// How it is drawn.
        style: TitleStyle,
    },
    /// A colour that fills the picture.
    Color {
        /// The colour.
        color: Rgba,
    },
}

/// How the text of a title is drawn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TitleStyle {
    /// The family of the font, or empty for the one that the engine chooses.
    pub font: String,
    /// The height of its letters in pixels of the timeline: from 1 to 4096.
    pub size: u32,
    /// The colour of its letters.
    pub color: Rgba,
    /// Where its anchor is across the picture: 0 at the left edge, 1 at the right.
    pub x: f64,
    /// Where its anchor is down the picture: 0 at the top edge, 1 at the bottom.
    pub y: f64,
    /// Which side of the text the anchor is on.
    pub align: TextAlign,
}

/// Which side of a text its anchor is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    /// The left.
    Left,
    /// The middle.
    Center,
    /// The right.
    Right,
}

/// A colour and how opaque it is, each from 0 to 255: red, green, blue and alpha. It is written
/// as `#rrggbbaa`, in hexadecimal, lowercase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Rgba(pub [u8; 4]);

impl TryFrom<String> for Rgba {
    type Error = String;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        let wrong = || format!("the colour {text:?} is not of the form #rrggbbaa");
        let digits = text.strip_prefix('#').ok_or_else(wrong)?;
        if digits.len() != 8 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(wrong());
        }
        let mut rgba = [0; 4];
        for (at, channel) in rgba.iter_mut().enumerate() {
            *channel = u8::from_str_radix(&digits[at * 2..at * 2 + 2], 16).map_err(|_| wrong())?;
        }
        Ok(Self(rgba))
    }
}

impl From<Rgba> for String {
    fn from(colour: Rgba) -> Self {
        let [r, g, b, a] = colour.0;
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}

/// Where the picture of a clip is put on the picture of its timeline, once it is fitted to it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transform {
    /// How far it is moved right, as a share of the width of the timeline.
    pub x: f64,
    /// How far it is moved down, as a share of the height of the timeline.
    pub y: f64,
    /// How much larger it is made, about the middle of the picture: above 0, and at most 16.
    pub scale: f64,
    /// How far it is turned clockwise, in degrees.
    pub rotation: f64,
    /// How opaque it is: from 0, not seen, to 1.
    pub opacity: f64,
}

impl Transform {
    /// The picture as it is fitted to the timeline: not moved, not made larger, not turned, and
    /// opaque.
    pub const NONE: Transform = Transform {
        x: 0.0,
        y: 0.0,
        scale: 1.0,
        rotation: 0.0,
        opacity: 1.0,
    };
}

impl Default for Transform {
    fn default() -> Self {
        Self::NONE
    }
}

/// A transition between two clips of a track that meet, the one that ends where the other
/// starts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transition {
    /// What it is known by.
    pub id: Id,
    /// How the one clip gives way to the other.
    pub kind: TransitionKind,
    /// The clip that ends at the cut.
    pub left: Id,
    /// The clip that starts at the cut.
    pub right: Id,
    /// How many frames it lasts.
    pub length: i64,
    /// Where it lies about the cut.
    pub align: Alignment,
    /// How the clip that starts at the cut rises, on a crossfade (p. 3831). It may be left out,
    /// as 0 dB, a straight line.
    #[serde(default)]
    pub fade_in_level_db: CrossfadeLevel,
    /// How the clip that ends at the cut falls, on a crossfade. It may be left out, as 0 dB.
    #[serde(default)]
    pub fade_out_level_db: CrossfadeLevel,
}

impl Transition {
    /// The frames that it shows of its left clip after the cut, and of its right clip before
    /// the cut: of one whose middle is at the cut, `ceil(length / 2)` and `floor(length / 2)`;
    /// of one that starts at the cut, all of them of the left clip, and of one that ends at it,
    /// all of them of the right clip. It lies over the frames from the first of those of the
    /// right clip to the last of those of the left.
    #[must_use]
    pub fn handles(&self) -> (i64, i64) {
        let length = self.length;
        match self.align {
            Alignment::Center => (length - length / 2, length / 2),
            Alignment::Start => (length, 0),
            Alignment::End => (0, length),
        }
    }
}

/// How one clip gives way to another.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionKind {
    /// The pictures are mixed, the one fading as the other comes up. On tracks of picture.
    Dissolve,
    /// The picture fades to black and up from it. On tracks of picture.
    DipBlack,
    /// The picture fades to white and up from it. On tracks of picture.
    DipWhite,
    /// The sounds are mixed, the one falling as the other rises, at the levels of the transition.
    /// On tracks of sound.
    Crossfade,
}

/// Where a transition lies about the cut that it is at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Alignment {
    /// It starts at the cut.
    Start,
    /// Its middle is at the cut.
    Center,
    /// It ends at the cut.
    End,
}

/// A subtitle: text shown on frames `start` to `start + length` of the timeline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cue {
    /// What it is known by.
    pub id: Id,
    /// The frame of the timeline that it is first shown at.
    pub start: i64,
    /// How many frames it is shown for.
    pub length: i64,
    /// What it says: from 1 to 1000 characters.
    pub text: String,
}

/// Why a project cannot be read.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    /// It is not an object of JSON.
    #[error("the project is not an object")]
    NotAnObject,
    /// It names no schema, or one that is not a whole number of at least 0.
    #[error("the project names no schema")]
    NoSchema,
    /// It is of another schema.
    #[error("the project is of schema {0}, and this engine reads schema {SCHEMA}")]
    Schema(u64),
    /// It is not a project.
    #[error("the project cannot be read: {0}")]
    Json(#[from] serde_json::Error),
    /// It is a project, but not one that the operations could have made.
    #[error("the project is not valid: {0}")]
    Invalid(#[from] Invalid),
}

impl Project {
    /// A project with nothing in it.
    #[must_use]
    pub fn new() -> Self {
        Self {
            schema: SCHEMA,
            next_id: 1,
            bins: vec![],
            media: vec![],
            timelines: vec![],
            groups: vec![],
            color: ColorSettings::default(),
        }
    }

    /// Read a project, refusing one of another schema before anything else is looked at, and
    /// then one that the operations could not have made (see [`Project::validate`], which also
    /// puts the things of each track in order and raises `next_id` where it is too low).
    ///
    /// # Errors
    /// It is not an object, it names no schema or another one, it is not a project, or it is
    /// not a valid one.
    pub fn from_value(value: serde_json::Value) -> Result<Self, ReadError> {
        let schema = schema_of(&value)?;
        if schema != u64::from(SCHEMA) {
            return Err(ReadError::Schema(schema));
        }
        let raw: RawProject = serde_json::from_value(value)?;
        Self::try_from(raw)
    }

    /// Read a project of this schema, as [`Project::from_value`] does, or one of an earlier
    /// schema, as [`crate::upgrade::from_schema_1`], [`crate::upgrade::from_schema_2`] and
    /// [`crate::upgrade::from_schema_3`] do.
    /// Gives back the schema that it was upgraded from, if it was.
    ///
    /// # Errors
    /// It is not an object, it names no schema or one that is none of these, it is not a project
    /// of its schema, or it is not a valid one.
    pub fn from_value_upgrading(
        value: serde_json::Value,
    ) -> Result<(Self, Option<u32>), ReadError> {
        match schema_of(&value)? {
            1 => Ok((crate::upgrade::from_schema_1(value)?, Some(1))),
            2 => Ok((crate::upgrade::from_schema_2(value)?, Some(2))),
            3 => Ok((crate::upgrade::from_schema_3(value)?, Some(3))),
            _ => Ok((Self::from_value(value)?, None)),
        }
    }
}

/// The schema that a project names.
pub(crate) fn schema_of(value: &serde_json::Value) -> Result<u64, ReadError> {
    value
        .as_object()
        .ok_or(ReadError::NotAnObject)?
        .get("schema")
        .and_then(serde_json::Value::as_u64)
        .ok_or(ReadError::NoSchema)
}

impl Default for Project {
    fn default() -> Self {
        Self::new()
    }
}

impl Timeline {
    /// A timeline with no tracks, at `rate`, of 1920 by 1080 pixels and 48000 samples a second.
    #[must_use]
    pub fn new(id: impl Into<Id>, name: impl Into<String>, rate: Rate) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            rate,
            width: 1920,
            height: 1080,
            sample_rate: 48000,
            tracks: vec![],
            grade: Grade::default(),
            buses: audio::first_buses(),
            vcas: vec![],
            track_groups: vec![],
            audio: TimelineAudio::default(),
        }
    }
}

impl Track {
    /// A track of `kind` with nothing on it, named by its id, seen or heard, and not locked;
    /// with no language if it is a track of subtitles.
    #[must_use]
    pub fn new(id: impl Into<Id>, kind: TrackKind) -> Self {
        let id = id.into();
        Self {
            name: id.clone(),
            id,
            kind,
            enabled: true,
            locked: false,
            clips: vec![],
            transitions: vec![],
            language: String::new(),
            cues: vec![],
            audio: TrackAudio::default(),
        }
    }

    /// The track with `clips` on it.
    #[must_use]
    pub fn with_clips(mut self, clips: Vec<Clip>) -> Self {
        self.clips = clips;
        self
    }
}

impl Clip {
    /// A clip of `source` at `start` for `length` frames, played as it is: seen and heard, at its
    /// speed, forward, in no group, not moved, at its loudness, in the middle, not faded and not
    /// graded.
    #[must_use]
    pub fn new(id: impl Into<Id>, source: Source, start: i64, length: i64) -> Self {
        Self {
            id: id.into(),
            start,
            length,
            source,
            enabled: true,
            speed: Speed::ONE,
            reverse: false,
            link: None,
            transform: Transform::NONE,
            gain_db: AudioParam::Fixed(0.0),
            pan: AudioParam::Fixed(0.0),
            fade_in: 0,
            fade_out: 0,
            audio_fade_in: None,
            audio_fade_out: None,
            eq: Equalizer::clip(),
            effects: vec![],
            pitch_correction: false,
            pitch: ClipPitch::default(),
            channels: None,
            grade: Grade::default(),
            group: None,
        }
    }

    /// A clip of `media` from `source_in`, as [`Clip::new`] makes it.
    #[must_use]
    pub fn of_media(
        id: impl Into<Id>,
        media: impl Into<Id>,
        source_in: Time,
        start: i64,
        length: i64,
    ) -> Self {
        let source = Source::Media {
            media: media.into(),
            source_in,
        };
        Self::new(id, source, start, length)
    }

    /// The media that it is cut from, if it is.
    #[must_use]
    pub fn media(&self) -> Option<&Id> {
        match &self.source {
            Source::Media { media, .. } => Some(media),
            _ => None,
        }
    }

    /// Where it begins in its media, if it is cut from media.
    #[must_use]
    pub fn source_in(&self) -> Option<Time> {
        match &self.source {
            Source::Media { source_in, .. } => Some(*source_in),
            _ => None,
        }
    }

    /// Where it begins in its media, to be changed, if it is cut from media.
    pub fn source_in_mut(&mut self) -> Option<&mut Time> {
        match &mut self.source {
            Source::Media { source_in, .. } => Some(source_in),
            _ => None,
        }
    }

    /// The frame after its last one.
    #[must_use]
    pub fn end(&self) -> i64 {
        self.start.saturating_add(self.length)
    }
}

#[cfg(any(test, feature = "sample"))]
impl Project {
    /// A small project to test with: ten seconds of a talk, on one track of picture and one
    /// of sound.
    ///
    /// The engine makes an id of a letter for the kind of the thing and `next_id`, and then adds
    /// one to `next_id`; so an id that it makes is new, whatever its letter, as long as `next_id`
    /// is above the number that every id of the project ends in. The ids here were written by
    /// hand, as those of a project made elsewhere may be, and the last of them ends in 2: so
    /// `next_id` is 3.
    #[must_use]
    pub fn sample() -> Self {
        const SCALE: u32 = 15360;
        let clip = |id: &str| {
            let start = Time {
                ticks: 0,
                scale: SCALE,
            };
            Clip::of_media(id, "m1", start, 0, 100)
        };
        let mut timeline = Timeline::new("t1", "Timeline 1", Rate { num: 25, den: 1 });
        timeline.tracks = vec![
            Track::new("V1", TrackKind::Video).with_clips(vec![clip("c1")]),
            Track::new("A1", TrackKind::Audio).with_clips(vec![clip("c2")]),
        ];
        Self {
            schema: SCHEMA,
            next_id: 3,
            bins: vec![],
            media: vec![Media {
                id: "m1".into(),
                path: "C:/media/talk.mp4".into(),
                identity: Identity {
                    size: 12_500_000,
                    modified_us: 1_790_000_000_000_000,
                    digest: "0".repeat(64),
                },
                facts: Facts {
                    duration: Time {
                        ticks: 10 * i64::from(SCALE),
                        scale: SCALE,
                    },
                    video: Some(VideoFacts {
                        still: false,
                        width: 1920,
                        height: 1080,
                        rate: Rate { num: 25, den: 1 },
                        codec: "h264".into(),
                        bits: 8,
                        rotation: 0,
                        color: None,
                    }),
                    audio: Some(AudioFacts {
                        rate: 48000,
                        channels: 2,
                        codec: "aac".into(),
                        layout: None,
                        streams: None,
                    }),
                },
                proxy: None,
                name: "talk.mp4".into(),
                bin: None,
                mark_in: None,
                mark_out: None,
                color_override: ColorOverride::default(),
            }],
            timelines: vec![timeline],
            groups: vec![],
            color: ColorSettings::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_is_written_with_the_names_that_other_languages_read() {
        let text = serde_json::to_string_pretty(&Project::sample()).unwrap();
        let written: serde_json::Value = serde_json::from_str(&text).unwrap();
        let clip = |id: &str| {
            serde_json::json!({
                "id": id,
                "start": 0,
                "length": 100,
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
                "grade": { "nodes": [], "links": [{ "from": "source", "to": "output", "kind": "rgb", "input": 0 }] },
                "group": null,
            })
        };
        let expected = serde_json::json!({
            "schema": 4,
            "next_id": 3,
            "bins": [],
            "media": [{
                "id": "m1",
                "path": "C:/media/talk.mp4",
                "identity": {
                    "size": 12_500_000,
                    "modified_us": 1_790_000_000_000_000_i64,
                    "digest": "0".repeat(64),
                },
                "facts": {
                    "duration": { "ticks": 153_600, "scale": 15360 },
                    "video": {
                        "width": 1920,
                        "height": 1080,
                        "rate": { "num": 25, "den": 1 },
                        "codec": "h264",
                        "bits": 8,
                        "rotation": 0,
                    },
                    "audio": { "rate": 48000, "channels": 2, "codec": "aac" },
                },
                "name": "talk.mp4",
                "bin": null,
                "mark_in": null,
                "mark_out": null,
                "color_override": {
                    "input": null, "levels": "auto", "lut": { "1d": null, "3d": null }, "inverse_drt": true,
                },
            }],
            "timelines": [{
                "id": "t1",
                "name": "Timeline 1",
                "rate": { "num": 25, "den": 1 },
                "width": 1920,
                "height": 1080,
                "sample_rate": 48000,
                "tracks": [
                    {
                        "id": "V1",
                        "kind": "video",
                        "name": "V1",
                        "enabled": true,
                        "locked": false,
                        "clips": [clip("c1")],
                        "transitions": [],
                    },
                    {
                        "id": "A1",
                        "kind": "audio",
                        "name": "A1",
                        "enabled": true,
                        "locked": false,
                        "clips": [clip("c2")],
                        "transitions": [],
                    },
                ],
                "grade": { "nodes": [], "links": [{ "from": "source", "to": "output", "kind": "rgb", "input": 0 }] },
            }],
            "groups": [],
            "color": serde_json::to_value(ColorSettings::default()).unwrap(),
        });
        let mut expected = expected;
        let timeline = &mut expected["timelines"][0];
        crate::audio::written::with(timeline, crate::audio::written::timeline());
        crate::audio::written::with(&mut timeline["tracks"][1], crate::audio::written::track());
        for track in [0, 1] {
            let clip = &mut timeline["tracks"][track]["clips"][0];
            crate::audio::written::with(clip, crate::audio::written::clip());
        }
        assert_eq!(written, expected);
    }

    #[test]
    fn a_project_is_written_and_read_back_as_it_was() {
        let project = Project::sample();
        let text = serde_json::to_string_pretty(&project).unwrap();
        assert_eq!(serde_json::from_str::<Project>(&text).unwrap(), project);
    }

    #[test]
    fn a_project_of_another_schema_is_refused() {
        let mut value = serde_json::to_value(Project::sample()).unwrap();
        value["schema"] = 99.into();
        let error = Project::from_value(value).unwrap_err();
        assert!(error.to_string().contains("schema 99"), "{error}");
    }

    #[test]
    fn a_field_that_is_not_known_is_refused() {
        let mut value = serde_json::to_value(Project::sample()).unwrap();
        value["colour"] = "red".into();
        let error = Project::from_value(value).unwrap_err();
        assert!(matches!(error, ReadError::Json(_)), "{error}");
    }

    #[test]
    fn a_field_that_is_not_known_is_refused_in_a_clip_too() {
        let mut value = serde_json::to_value(Project::sample()).unwrap();
        value["timelines"][0]["tracks"][0]["clips"][0]["colour"] = "red".into();
        let error = Project::from_value(value).unwrap_err();
        assert!(matches!(error, ReadError::Json(_)), "{error}");
        assert!(error.to_string().contains("colour"), "{error}");
    }

    #[test]
    fn a_project_that_is_deserialized_directly_is_checked_too() {
        // One that validate refuses: c1 cut from media that is not there.
        let mut value = serde_json::to_value(Project::sample()).unwrap();
        value["timelines"][0]["tracks"][0]["clips"][0]["source"]["media"] = "m9".into();
        let error = serde_json::from_value::<Project>(value).unwrap_err();
        assert!(
            error.to_string().contains("there is no media m9"),
            "{error}"
        );

        // One of another schema.
        let mut value = serde_json::to_value(Project::sample()).unwrap();
        value["schema"] = 5.into();
        let error = serde_json::from_value::<Project>(value).unwrap_err();
        assert!(error.to_string().contains("schema 5"), "{error}");

        // One that validate puts right.
        let mut value = serde_json::to_value(Project::sample()).unwrap();
        value["next_id"] = 1.into();
        let project: Project = serde_json::from_value(value).unwrap();
        assert_eq!(project, Project::sample());
    }

    #[test]
    fn a_project_that_validate_refuses_is_named_so_by_from_value() {
        let mut value = serde_json::to_value(Project::sample()).unwrap();
        value["timelines"][0]["tracks"][0]["clips"][0]["source"]["media"] = "m9".into();
        let error = Project::from_value(value).unwrap_err();
        assert!(matches!(error, ReadError::Invalid(_)), "{error}");
    }

    #[test]
    fn a_project_is_read_from_a_value_as_it_was() {
        let value = serde_json::to_value(Project::sample()).unwrap();
        assert_eq!(Project::from_value(value).unwrap(), Project::sample());
    }

    #[test]
    fn a_new_project_holds_nothing_and_makes_its_first_id_with_1() {
        let project = Project::new();
        assert_eq!(project.schema, SCHEMA);
        assert_eq!(project.next_id, 1);
        assert_eq!(project.media, []);
        assert_eq!(project.timelines, []);
        assert_eq!(Project::default(), project);
    }

    #[test]
    fn what_is_not_an_object_is_not_a_project() {
        let error = Project::from_value(serde_json::json!([1])).unwrap_err();
        assert!(matches!(error, ReadError::NotAnObject), "{error}");
        assert_eq!(error.to_string(), "the project is not an object");
    }

    #[test]
    fn a_project_that_names_no_schema_is_refused() {
        let mut value = serde_json::to_value(Project::sample()).unwrap();
        value.as_object_mut().unwrap().remove("schema");
        let error = Project::from_value(value).unwrap_err();
        assert!(matches!(error, ReadError::NoSchema), "{error}");
        assert_eq!(error.to_string(), "the project names no schema");
        for schema in [
            serde_json::json!(-1),
            serde_json::json!(1.5),
            serde_json::json!("1"),
        ] {
            let mut value = serde_json::to_value(Project::sample()).unwrap();
            value["schema"] = schema;
            let error = Project::from_value(value).unwrap_err();
            assert!(matches!(error, ReadError::NoSchema), "{error}");
        }
    }

    #[test]
    fn a_schema_too_large_for_the_field_is_named_as_it_is() {
        let mut value = serde_json::to_value(Project::sample()).unwrap();
        value["schema"] = 5_000_000_000_u64.into();
        let error = Project::from_value(value).unwrap_err();
        assert!(error.to_string().contains("schema 5000000000"), "{error}");
    }

    #[test]
    fn a_rate_of_no_frames_is_refused() {
        let mut value = serde_json::to_value(Project::sample()).unwrap();
        value["timelines"][0]["rate"]["num"] = 0.into();
        let error = Project::from_value(value).unwrap_err();
        assert!(matches!(error, ReadError::Json(_)), "{error}");
        assert!(error.to_string().contains("`num` of 0"), "{error}");
    }

    #[test]
    fn a_rate_over_no_time_is_refused() {
        let mut value = serde_json::to_value(Project::sample()).unwrap();
        value["timelines"][0]["rate"]["den"] = 0.into();
        let error = Project::from_value(value).unwrap_err();
        assert!(matches!(error, ReadError::Json(_)), "{error}");
        assert!(error.to_string().contains("`den` of 0"), "{error}");
    }

    #[test]
    fn a_rate_of_a_file_with_a_zero_in_it_is_refused_too() {
        let mut value = serde_json::to_value(Project::sample()).unwrap();
        value["media"][0]["facts"]["video"]["rate"]["num"] = 0.into();
        assert!(Project::from_value(value).is_err());
    }

    #[test]
    fn a_track_is_written_and_read_with_the_fields_of_its_kind_only() {
        let mut subtitles = Track::new("S1", TrackKind::Subtitle);
        subtitles.language = "zh".into();
        subtitles.cues = vec![Cue {
            id: "q9".into(),
            start: 100,
            length: 50,
            text: "你好".into(),
        }];
        let written = serde_json::to_value(&subtitles).unwrap();
        assert_eq!(
            written,
            serde_json::json!({
                "id": "S1", "kind": "subtitle", "name": "S1", "enabled": true, "locked": false,
                "language": "zh", "cues": [{ "id": "q9", "start": 100, "length": 50, "text": "你好" }],
            })
        );
        assert_eq!(serde_json::from_value::<Track>(written).unwrap(), subtitles);

        let read = |value: serde_json::Value| serde_json::from_value::<Track>(value);
        let video = serde_json::to_value(Track::new("V1", TrackKind::Video)).unwrap();
        let mut with_cues = video.clone();
        with_cues["cues"] = serde_json::json!([]);
        let error = read(with_cues).unwrap_err().to_string();
        assert!(error.contains("a video track has no `cues`"), "{error}");
        let mut without_transitions = video;
        without_transitions
            .as_object_mut()
            .unwrap()
            .remove("transitions");
        let error = read(without_transitions).unwrap_err().to_string();
        assert!(error.contains("missing field `transitions`"), "{error}");
        let error = read(serde_json::json!({
            "id": "S1", "kind": "subtitle", "name": "S1", "enabled": true, "locked": false,
            "language": "", "cues": [], "clips": [],
        }))
        .unwrap_err()
        .to_string();
        assert!(error.contains("a subtitle track has no `clips`"), "{error}");
    }

    #[test]
    fn a_field_that_may_be_null_may_not_be_left_out() {
        let mut value = serde_json::to_value(Project::sample()).unwrap();
        value["media"][0].as_object_mut().unwrap().remove("bin");
        let error = Project::from_value(value).unwrap_err();
        assert!(error.to_string().contains("missing field `bin`"), "{error}");
        let mut value = serde_json::to_value(Project::sample()).unwrap();
        value["timelines"][0]["tracks"][0]["clips"][0]
            .as_object_mut()
            .unwrap()
            .remove("link");
        let error = Project::from_value(value).unwrap_err();
        assert!(
            error.to_string().contains("missing field `link`"),
            "{error}"
        );
    }

    #[test]
    fn what_schema_3_adds_may_be_left_out_and_is_read_as_changing_nothing() {
        let mut value = serde_json::to_value(Project::sample()).unwrap();
        value.as_object_mut().unwrap().remove("groups");
        let timeline = value["timelines"][0].as_object_mut().unwrap();
        timeline.remove("grade");
        for track in timeline["tracks"].as_array_mut().unwrap() {
            for clip in track["clips"].as_array_mut().unwrap() {
                let clip = clip.as_object_mut().unwrap();
                clip.remove("grade");
                clip.remove("group");
            }
        }
        assert_eq!(Project::from_value(value).unwrap(), Project::sample());
    }

    #[test]
    fn a_clip_names_its_media_in_its_source_only() {
        let mut value = serde_json::to_value(Project::sample()).unwrap();
        value["timelines"][0]["tracks"][0]["clips"][0]["media"] = "m1".into();
        let error = Project::from_value(value).unwrap_err();
        assert!(
            error.to_string().contains("unknown field `media`"),
            "{error}"
        );
    }

    #[test]
    fn a_title_and_a_colour_are_written_as_their_kinds() {
        let title = Source::Title {
            text: "标题".into(),
            style: TitleStyle {
                font: String::new(),
                size: 96,
                color: Rgba([255, 255, 255, 255]),
                x: 0.5,
                y: 0.5,
                align: TextAlign::Center,
            },
        };
        let written = serde_json::to_value(&title).unwrap();
        assert_eq!(
            written,
            serde_json::json!({
                "kind": "title", "text": "标题",
                "style": { "font": "", "size": 96, "color": "#ffffffff", "x": 0.5, "y": 0.5, "align": "center" },
            })
        );
        assert_eq!(serde_json::from_value::<Source>(written).unwrap(), title);
        let colour = serde_json::json!({ "kind": "color", "color": "#0A0b0C80" });
        let read: Source = serde_json::from_value(colour).unwrap();
        assert_eq!(
            read,
            Source::Color {
                color: Rgba([10, 11, 12, 128])
            }
        );
        assert_eq!(
            serde_json::to_value(&read).unwrap()["color"],
            serde_json::json!("#0a0b0c80")
        );
        for wrong in ["#fff", "0a0b0c80", "#0a0b0c8g", "#0a0b0c80ff"] {
            let value = serde_json::json!({ "kind": "color", "color": wrong });
            let error = serde_json::from_value::<Source>(value).unwrap_err();
            assert!(error.to_string().contains("#rrggbbaa"), "{error}");
        }
    }

    #[test]
    fn a_speed_with_a_zero_in_it_is_refused() {
        let mut value = serde_json::to_value(Project::sample()).unwrap();
        value["timelines"][0]["tracks"][0]["clips"][0]["speed"]["den"] = 0.into();
        let error = Project::from_value(value).unwrap_err();
        assert!(
            error.to_string().contains("a speed has a `den` of 0"),
            "{error}"
        );
    }
}
