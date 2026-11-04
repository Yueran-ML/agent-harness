//! The sound of a timeline as Fairlight mixes it (Reference Manual 20, Ch. 170–182): what schema 4
//! adds to a project.
//!
//! - A clip of sound has keyframes of its volume and pan ([`param::AudioParam`]), fades of a
//!   curve and of any length ([`AudioFade`]), its own equalizer and effects, a pitch, whether a
//!   change of its speed keeps its pitch, and which channels of its media it plays
//!   ([`ChannelMap`]).
//! - A track of sound has a format of channels ([`TrackFormat`]), a mute and a solo apart from
//!   whether it is on, and a strip ([`Strip`]): its trim, Ducker, inserts, Dynamics, equalizer,
//!   fader, pan, sends, outputs and VCAs; and lanes of automation ([`param::Lane`]).
//! - A timeline has buses, VCAs and groups of tracks ([`routing`]), and the settings of its
//!   meters, its loudness, its solo and its recording of automation ([`TimelineAudio`]).
//! - A crossfade has a level at each side ([`CrossfadeLevel`]).
//!
//! Every field of schema 4 is written by the engine, and may be left out where a project is
//! read, as its default; the defaults are those of a new timeline in `DaVinci`, and those of the
//! interaction reference of the studio where the manual gives none. A project of schema 3 is
//! read with values that leave its sound as it was (see [`crate::upgrade`]).
//!
//! # Levels in decibels
//!
//! A fader, a send, a VCA and a boom are written in decibels, at most +10; -100 or below is -∞
//! (see [`gain_of_db`]).

pub mod automation;
pub mod fx;
pub mod param;
pub mod routing;

use serde::{Deserialize, Serialize};

pub use fx::{Effect, Equalizer, Insert, SourceRef};
pub use param::{AudioKey, AudioParam, KeyInterp, Lane, LaneKind, LanePoint, LaneScale, Lanes};
pub use routing::{Bus, BusFormat, GroupControl, Output, Send, TrackGroup, Vca, FIRST_BUS};

use crate::ops::OpError;
use crate::project::{Clip, Id, Timeline, Track};
use crate::time::{Rate, Time};

use fx::within;

/// A level of -100 dB or below, which is -∞: silence.
pub const SILENT_DB: f64 = -100.0;

/// The factor of a level in decibels: 0 for [`SILENT_DB`] and below.
#[must_use]
pub fn gain_of_db(db: f64) -> f64 {
    if db <= SILENT_DB {
        0.0
    } else {
        10f64.powf(db / 20.0)
    }
}

/// Whether a level in decibels is one that a fader, a send or a boom takes: finite, at most 10.
fn level(db: f64) -> bool {
    db.is_finite() && db <= 10.0
}

const fn yes() -> bool {
    true
}

fn out(what: &'static str) -> OpError {
    OpError::OutOfRange { what }
}

/// Where a strip, or a send, puts its sound among the channels of a bus (pp. 3872–3874). On a
/// bus of two channels only `x` counts; on one of more, the rest too.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Pan {
    /// Whether it pans: off, a mono strip goes to the middle and one of more channels to the
    /// channels of the bus one by one.
    pub on: bool,
    /// Left (-1) to right (1), shown from -100 to 100.
    pub x: f64,
    /// Back (-1) to front (1), shown from -100 to 100.
    pub y: f64,
    /// How far it is turned about the listener, in degrees, from -180 to 180.
    pub rotate: f64,
    /// How far apart the channels of a strip of more than one are, from 1 (a point) to 99 (full,
    /// as a new pan is).
    pub spread: f64,
    /// How much spills to the speakers beside, in percent, from 0 to 100.
    pub divergence: f64,
    /// How much goes to the LFE (Boom), in decibels, at most 10: -∞ (-100) for a new pan (C-F11).
    pub lfe_db: f64,
    /// Whether the Boom is on.
    pub lfe_on: bool,
    /// Whether the Boom is taken before the fader.
    pub lfe_pre: bool,
}

impl Default for Pan {
    fn default() -> Self {
        Self {
            on: true,
            x: 0.0,
            y: 0.0,
            rotate: 0.0,
            spread: 99.0,
            divergence: 0.0,
            lfe_db: SILENT_DB,
            lfe_on: false,
            lfe_pre: false,
        }
    }
}

impl Pan {
    fn check(&self) -> Result<(), OpError> {
        let fine = within(self.x, -1.0, 1.0)
            && within(self.y, -1.0, 1.0)
            && within(self.rotate, -180.0, 180.0)
            && within(self.spread, 1.0, 99.0)
            && within(self.divergence, 0.0, 100.0)
            && level(self.lfe_db);
        if fine {
            Ok(())
        } else {
            Err(out("pan"))
        }
    }
}

/// A stage of a strip that its Order puts in place (p. 3859).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Its inserts.
    Fx,
    /// Its Dynamics.
    Dynamics,
    /// Its equalizer.
    Eq,
}

/// The order of a new strip: inserts, Dynamics, equalizer.
fn first_order() -> Vec<Stage> {
    vec![Stage::Fx, Stage::Dynamics, Stage::Eq]
}

/// The strip of a track or a bus (pp. 3685–3688, 3855): Trim and polarity, the Ducker of a track,
/// its inserts, Dynamics and equalizer in their order, the mute, the fader, the pan, and the
/// buses it outputs and sends to. A new one is at unity and changes nothing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Strip {
    /// The gain before everything, in decibels, from -20 to 20 (Path Settings, p. 3857).
    pub trim_db: f64,
    /// Whether its polarity is turned over.
    pub invert: bool,
    /// The Ducker of a track (its one Track FX), or none. A bus has none.
    pub track_fx: Option<fx::Ducker>,
    /// The order of its inserts, Dynamics and equalizer: each once.
    pub order: Vec<Stage>,
    /// Its inserts, at most [`routing::MOST_INSERTS`], in order.
    pub inserts: Vec<Insert>,
    /// Whether its inserts are on (Effects In, p. 3860): on for a new strip.
    pub inserts_on: bool,
    /// Its Dynamics.
    pub dynamics: fx::Dynamics,
    /// Its equalizer, of 6 bands.
    pub eq: Equalizer,
    /// Its fader, in decibels, at most 10; -100 or below is -∞.
    pub fader_db: f64,
    /// Its pan.
    pub pan: Pan,
    /// Its sends, at most [`routing::MOST_SENDS`].
    pub sends: Vec<Send>,
    /// The buses that it outputs to, at most [`routing::MOST_OUTPUTS`], each once.
    pub outputs: Vec<Output>,
    /// The VCAs that it is a member of.
    pub vcas: Vec<Id>,
}

impl Default for Strip {
    fn default() -> Self {
        Self {
            trim_db: 0.0,
            invert: false,
            track_fx: None,
            order: first_order(),
            inserts: vec![],
            inserts_on: true,
            dynamics: fx::Dynamics::default(),
            eq: Equalizer::strip(),
            fader_db: 0.0,
            pan: Pan::default(),
            sends: vec![],
            outputs: vec![],
            vcas: vec![],
        }
    }
}

impl Strip {
    /// The strip of a new track: at unity, out to the first bus of a timeline (p. 3721).
    #[must_use]
    pub fn to_first_bus() -> Self {
        Self {
            outputs: vec![Output {
                bus: FIRST_BUS.into(),
                on: true,
            }],
            ..Self::default()
        }
    }

    /// The strip of a track that is added without one: at unity, out to the first bus of its
    /// timeline where it has one, and to none where it has not. The output is named by an empty
    /// id until the track is in its timeline (see [`place_outputs`]).
    #[must_use]
    pub fn to_first_bus_if_any() -> Self {
        Self {
            outputs: vec![Output {
                bus: String::new(),
                on: true,
            }],
            ..Self::default()
        }
    }

    /// Check its settings, as the strip of a bus where `bus`, but not what it names, which
    /// [`routing::check_routing`] checks.
    ///
    /// # Errors
    /// Its order is not one of its three stages each once (`order`); it has too many inserts,
    /// sends or outputs; it outputs twice to one bus (`outputs`); it is of a bus and has a
    /// Ducker (`track_fx`); or a setting is out of its range.
    pub fn check(&self, bus: bool) -> Result<(), OpError> {
        if !within(self.trim_db, -20.0, 20.0) {
            return Err(out("trim_db"));
        }
        match &self.track_fx {
            Some(_) if bus => return Err(out("track_fx")),
            Some(ducker) => ducker.check()?,
            None => {}
        }
        let mut order = self.order.clone();
        order.sort_by_key(|stage| *stage as u8);
        if order != first_order() {
            return Err(OpError::Order);
        }
        if self.inserts.len() > routing::MOST_INSERTS {
            return Err(OpError::TooManyInserts);
        }
        for insert in &self.inserts {
            insert.effect.check()?;
        }
        let mut ids: Vec<&Id> = self.inserts.iter().map(|i| &i.id).collect();
        ids.sort();
        if ids.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(out("inserts"));
        }
        self.dynamics.check()?;
        self.eq.check(true)?;
        if !level(self.fader_db) {
            return Err(out("fader_db"));
        }
        self.pan.check()?;
        if self.sends.len() > routing::MOST_SENDS {
            return Err(OpError::TooManySends);
        }
        for send in &self.sends {
            if !level(send.level_db) {
                return Err(out("send"));
            }
            send.pan.check()?;
        }
        let mut sent: Vec<&Id> = self.sends.iter().map(|s| &s.bus).collect();
        sent.sort();
        sent.dedup();
        if sent.len() != self.sends.len() {
            return Err(out("sends"));
        }
        if self.outputs.len() > routing::MOST_OUTPUTS {
            return Err(OpError::TooManyOutputs);
        }
        let mut buses: Vec<&Id> = self.outputs.iter().map(|o| &o.bus).collect();
        buses.sort();
        buses.dedup();
        if buses.len() != self.outputs.len() {
            return Err(out("outputs"));
        }
        Ok(())
    }
}

/// The format of a track of sound (pp. 3723–3724): its channels, and so its lanes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrackFormat {
    /// One channel.
    #[serde(rename = "mono")]
    Mono,
    /// Two: L R.
    #[serde(rename = "stereo")]
    Stereo,
    /// Six, in the order of the track (see [`ChannelOrder`]).
    #[serde(rename = "5.1")]
    FiveOne,
    /// From 1 to 24, as many as the track was made with.
    #[serde(rename = "adaptive")]
    Adaptive,
}

impl TrackFormat {
    /// How many channels a track of the format has, or none for an adaptive one, which has as
    /// many as it was made with.
    #[must_use]
    pub fn channels(self) -> Option<u8> {
        match self {
            Self::Mono => Some(1),
            Self::Stereo => Some(2),
            Self::FiveOne => Some(6),
            Self::Adaptive => None,
        }
    }
}

/// The order of the channels of a 5.1 track (p. 3730). Within the engine they are always taken
/// in the order of SMPTE (C-F8).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelOrder {
    /// L R C LFE Ls Rs.
    #[default]
    Smpte,
    /// L C R Ls Rs LFE.
    Film,
}

/// The most channels of an adaptive track (p. 3724).
pub const MOST_CHANNELS: u8 = 24;

/// What a track of sound has that the other tracks do not: its format, its mute and solo, its
/// strip and its automation. A track of picture or of subtitles has the default, and writes
/// none of it.
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::struct_excessive_bools, reason = "the buttons of a track")]
pub struct TrackAudio {
    /// Its format: stereo for a track made without one.
    pub format: TrackFormat,
    /// How many channels it has: as many as its format has, or from 1 to [`MOST_CHANNELS`] for
    /// an adaptive one.
    pub channels: u8,
    /// The order of the channels of a 5.1 track.
    pub order: ChannelOrder,
    /// Whether it is muted (p. 3885): it is not heard, nor rendered. Unlike a track that is not
    /// `enabled` (Disable Track, p. 3726), it is still read and metered.
    pub mute: bool,
    /// Whether it is soloed (p. 3885): while any track is, those that are not, and are not
    /// solo-safe, are not heard, nor rendered.
    pub solo: bool,
    /// Whether it is heard while others are soloed (Solo Safe).
    pub solo_safe: bool,
    /// Whether its delay is made up with the others' (ADC, p. 3706): on for a new track.
    pub adc: bool,
    /// Whether its automation is armed to record (p. 3906).
    pub automation_arm: bool,
    /// Its strip: out to the first bus, for a track made without one.
    pub strip: Strip,
    /// The automation of its settings.
    pub lanes: Lanes,
}

impl Default for TrackAudio {
    fn default() -> Self {
        Self {
            format: TrackFormat::Stereo,
            channels: 2,
            order: ChannelOrder::Smpte,
            mute: false,
            solo: false,
            solo_safe: false,
            adc: true,
            automation_arm: false,
            strip: Strip::to_first_bus(),
            lanes: Lanes::new(),
        }
    }
}

/// A fade of the sound of a clip (pp. 972–973, 3794, 3809): over `length` from its start, or to
/// its end, along a curve.
///
/// The gain of a fade in at a share `t` of the way through it, from 0 to 1, is
/// `g(t') = (1 - |c|)·t' + |c|·e(t')`, where `t' = t^(2^-skew)` moves the middle of the curve
/// earlier (skew > 0) or later, and `e` is `sin(πt'/2)` for a curve `c` above 0 and
/// `1 - cos(πt'/2)` below it; a fade out is the same curve the other way, `g(1 - t)`. A curve
/// and a skew of 0 are a straight line, as fades were before schema 4.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioFade {
    /// How long it lasts: at least nothing.
    pub length: Time,
    /// From -1 to 1: its shape, 0 a straight line (dragged up and down, "angle").
    #[serde(default)]
    pub curve: f64,
    /// From -1 to 1: where its middle is (dragged across, "shape").
    #[serde(default)]
    pub skew: f64,
}

impl AudioFade {
    /// The gain of a fade in a share `t` of the way through it.
    #[must_use]
    pub fn rise(&self, t: f64) -> f64 {
        fade_curve(t, self.curve, self.skew)
    }
}

/// The gain of a fade in of `curve` and `skew` a share `t` of the way through it (see
/// [`AudioFade`]); exactly `t` for a straight line.
#[must_use]
pub fn fade_curve(t: f64, curve: f64, skew: f64) -> f64 {
    let t = if skew == 0.0 {
        t
    } else {
        t.powf((-skew).exp2())
    };
    if curve == 0.0 {
        return t;
    }
    let turn = t * std::f64::consts::FRAC_PI_2;
    let end = if curve > 0.0 {
        turn.sin()
    } else {
        1.0 - turn.cos()
    };
    (1.0 - curve.abs()) * t + curve.abs() * end
}

/// The level of a side of a crossfade (p. 3831): how the clip that comes in rises, or the one
/// that goes out falls, across it. 0 dB is a straight line, as a new crossfade has (p. 3808);
/// +3 dB rises by the sine, at equal power, as every crossfade did before schema 4; -3 dB by
/// `t^1.5`, 3 dB below a straight line at the middle. It is written as the number of decibels:
/// `-3`, `0` or `3`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CrossfadeLevel {
    /// -3 dB: `t^1.5`.
    Minus3,
    /// 0 dB: `t`.
    #[default]
    Zero,
    /// +3 dB: `sin(πt/2)`.
    Plus3,
}

impl Serialize for CrossfadeLevel {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_i8(match self {
            Self::Minus3 => -3,
            Self::Zero => 0,
            Self::Plus3 => 3,
        })
    }
}

#[allow(clippy::float_cmp, reason = "the three levels are whole numbers")]
impl<'de> Deserialize<'de> for CrossfadeLevel {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = f64::deserialize(deserializer)?;
        if value == -3.0 {
            Ok(Self::Minus3)
        } else if value == 0.0 {
            Ok(Self::Zero)
        } else if value == 3.0 {
            Ok(Self::Plus3)
        } else {
            Err(serde::de::Error::custom(
                "the level of a crossfade is -3, 0 or 3 decibels",
            ))
        }
    }
}

impl CrossfadeLevel {
    /// The gain of the clip that comes in, a share `t` of the way through the crossfade.
    #[must_use]
    pub fn rise(self, t: f64) -> f64 {
        let turn = t * std::f64::consts::FRAC_PI_2;
        match self {
            Self::Plus3 => turn.sin(),
            Self::Zero => t,
            Self::Minus3 => t.powf(1.5),
        }
    }

    /// The gain of the clip that goes out, a share `t` of the way through the crossfade: the
    /// rise the other way, `rise(1 - t)`.
    #[must_use]
    pub fn fall(self, t: f64) -> f64 {
        let turn = t * std::f64::consts::FRAC_PI_2;
        match self {
            Self::Plus3 => turn.cos(),
            Self::Zero => 1.0 - t,
            Self::Minus3 => (1.0 - t).powf(1.5),
        }
    }
}

/// A channel of the media of a clip: its stream, as libav (and ffprobe) number the streams of
/// the file, and the channel in it, from 0, in the stream's own order of channels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelSource {
    /// The stream.
    pub stream: u32,
    /// The channel in it.
    pub channel: u32,
}

/// The channels of the media that a channel of a track plays: none, for silence; one, as the
/// Channel Source of `DaVinci` picks it (p. 3835); or several, whose mean it plays, so that a
/// stereo pair is made mono as `0.5·(L + R)` (C-F16). It is written `null`, as one channel
/// (`{"stream": 1, "channel": 0}`), or as a list of them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ChannelSources(pub Vec<ChannelSource>);

impl Serialize for ChannelSources {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match &self.0[..] {
            [] => serializer.serialize_none(),
            [one] => one.serialize(serializer),
            many => many.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for ChannelSources {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Written {
            One(ChannelSource),
            Many(Vec<ChannelSource>),
        }
        Ok(Self(match Option::<Written>::deserialize(deserializer)? {
            None => vec![],
            Some(Written::One(one)) => vec![one],
            Some(Written::Many(many)) => many,
        }))
    }
}

/// What a channel of the track of a clip plays (pp. 3833–3836): channels of its media, or
/// nothing; whether it is on and muted; and its level. A channel that names a stream or a
/// channel that its media has not plays silence, as one that is off or muted does.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelMap {
    /// The channels of the media that it plays.
    #[serde(default)]
    pub source: ChannelSources,
    /// Whether it is on.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Whether it is muted.
    #[serde(default)]
    pub mute: bool,
    /// Its level in decibels, at most 24; -100 or below is -∞.
    #[serde(default)]
    pub level_db: f64,
}

/// The pitch of a clip (p. 3826), apart from its speed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ClipPitch {
    /// Whole semitones from -12 to 12.
    pub semitones: i32,
    /// Cents from -100 to 100.
    pub cents: f64,
}

/// The kind of the meters of a timeline (pp. 3887–3888).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeterKind {
    /// IEC 60268-18, a digital PPM.
    #[default]
    Ppm,
    /// Digital VU.
    Vu,
    /// As [`CustomMeter`] sets.
    Custom,
}

/// What a custom meter reads (p. 162).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Detector {
    /// Sample program peaks.
    #[default]
    Sample,
    /// VU.
    Vu,
    /// RMS.
    Rms,
}

/// The scale of a custom meter (p. 162).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeterScale {
    /// IEC 60268-18.
    #[default]
    Iec,
    /// Quasi linear.
    Linear,
}

/// How a custom meter holds its peaks (p. 162).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeakHold {
    /// Not at all.
    Off,
    /// Short hold and fall.
    ShortHoldFall,
    /// Medium hold and fall.
    #[default]
    MediumHoldFall,
    /// Medium hold.
    MediumHold,
    /// Long hold.
    LongHold,
    /// Until it is reset.
    InfiniteHold,
}

/// A custom meter (p. 162).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CustomMeter {
    /// What it reads.
    pub detector: Detector,
    /// Its scale.
    pub scale: MeterScale,
    /// The seconds that it takes to fall 20 dB, from 0.1 to 5: 1.7 (C-L2).
    pub decay_s: f64,
    /// How it holds its peaks.
    pub peak: PeakHold,
}

impl Default for CustomMeter {
    fn default() -> Self {
        Self {
            detector: Detector::Sample,
            scale: MeterScale::Iec,
            decay_s: 1.7,
            peak: PeakHold::MediumHoldFall,
        }
    }
}

/// A standard of loudness that the loudness meter measures against (p. 3984), and the presets of
/// platforms that the studio adds (marked unofficial where they are).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LoudnessStandard {
    /// ITU-R BS.1770-1.
    #[serde(rename = "bs1770_1")]
    Bs1770_1,
    /// ITU-R BS.1770-4.
    #[serde(rename = "bs1770_4")]
    Bs1770_4,
    /// ATSC A/85.
    #[serde(rename = "atsc_a85")]
    AtscA85,
    /// EBU R128.
    #[default]
    #[serde(rename = "ebu_r128")]
    EbuR128,
    /// OP-59.
    #[serde(rename = "op_59")]
    Op59,
    /// TR-B32.
    #[serde(rename = "tr_b32")]
    TrB32,
    /// AGCOM 219.
    #[serde(rename = "agcom_219")]
    Agcom219,
    /// Netflix.
    #[serde(rename = "netflix")]
    Netflix,
    /// `YouTube`.
    #[serde(rename = "youtube")]
    Youtube,
    /// Disney 2.0.
    #[serde(rename = "disney_2_0")]
    Disney2_0,
    /// Disney 5.1.
    #[serde(rename = "disney_5_1")]
    Disney5_1,
    /// Apple Podcasts.
    #[serde(rename = "apple_podcasts")]
    ApplePodcasts,
    /// Spotify.
    #[serde(rename = "spotify")]
    Spotify,
    /// AES streaming, for dialogue.
    #[serde(rename = "aes_streaming")]
    AesStreaming,
    /// Video on the web in general (unofficial).
    #[serde(rename = "web_video")]
    WebVideo,
}

/// The scale of the loudness meter (p. 3984).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoudnessScale {
    /// EBU +9: from -18 to +9.
    #[default]
    Ebu9,
    /// EBU +18: from -36 to +18.
    Ebu18,
}

/// The settings of the loudness meter of a timeline (pp. 3984–3986, 162).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Loudness {
    /// The standard: EBU R128 (Q15).
    pub standard: LoudnessStandard,
    /// The target, in LUFS, from -36 to -10: -23.
    pub target: f64,
    /// The scale.
    pub scale: LoudnessScale,
    /// Whether it reads relative to the target: so it does.
    pub relative: bool,
    /// Whether it starts again when the playhead moves (Lock Metering to Transport): so it does.
    pub lock_to_transport: bool,
}

impl Default for Loudness {
    fn default() -> Self {
        Self {
            standard: LoudnessStandard::EbuR128,
            target: -23.0,
            scale: LoudnessScale::Ebu9,
            relative: true,
            lock_to_transport: true,
        }
    }
}

/// How automation is written (p. 3905).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordMode {
    /// The value of the control.
    #[default]
    Write,
    /// A change of the value that was there.
    Trim,
}

/// When automation is written (p. 3905).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Touch {
    /// Only on armed tracks, as Latch.
    #[default]
    Off,
    /// From a touch, on to the stop.
    Latch,
    /// From a touch, back to what was there on release.
    Snap,
    /// The fader as Snap, the rest as Latch.
    SnapLatch,
}

/// What is written at the stop (p. 3905).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnStop {
    /// The last value, to the start of what was written next.
    Event,
    /// The last value, to the end.
    Hold,
    /// Back to what was there.
    #[default]
    Return,
}

/// What is written by a pass of recording (pp. 3905–3906): all of it, where it is left out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools, reason = "the switches of the manual")]
pub struct Categories {
    /// Faders.
    pub fader: bool,
    /// Mutes.
    pub mute: bool,
    /// Pans.
    pub pan: bool,
    /// Equalizers.
    pub eq: bool,
    /// Compressors.
    pub comp: bool,
    /// Expanders and gates.
    pub gate: bool,
    /// Limiters.
    pub lim: bool,
    /// Sends.
    pub sends: bool,
    /// Plugins.
    pub plugins: bool,
    /// The rest: outputs and inserts on and off.
    pub misc: bool,
}

impl Default for Categories {
    fn default() -> Self {
        Self {
            fader: true,
            mute: true,
            pan: true,
            eq: true,
            comp: true,
            gate: true,
            lim: true,
            sends: true,
            plugins: true,
            misc: true,
        }
    }
}

/// The stretch of the timeline that automation is written in only (Active Range, p. 3908).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveRange {
    /// Its start, a time of the timeline.
    pub start: Time,
    /// Its end, after its start.
    pub end: Time,
    /// Whether it is used.
    #[serde(default)]
    pub on: bool,
}

/// The settings of the recording of automation of a timeline (pp. 3905–3909).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Record {
    /// Write or Trim.
    pub mode: RecordMode,
    /// When it is written.
    pub touch: Touch,
    /// What is written at the stop.
    pub on_stop: OnStop,
    /// What is written.
    pub categories: Categories,
    /// Whether values are only heard, until they are punched in.
    pub preview: bool,
    /// Where only it is written, if anywhere.
    pub active_range: Option<ActiveRange>,
}

/// The settings of the sound of a timeline that are not of a track or a bus.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TimelineAudio {
    /// The bus that is heard, and that the loudness meter measures; or none.
    pub monitor: Option<Id>,
    /// The kind of the meters: IEC 60268-18 (Q15).
    pub meter: MeterKind,
    /// The custom meter.
    pub custom_meter: CustomMeter,
    /// Whether the meters of the tracks read before their faders: they read after (Q15).
    pub prefader_metering: bool,
    /// The loudness meter.
    pub loudness: Loudness,
    /// Whether a solo takes the place of the others (Exclusive Solo, p. 3885).
    pub exclusive_solo: bool,
    /// Whether automation moves with the clips that it is under (p. 3910): so it does.
    pub follows_edit: bool,
    /// The recording of automation.
    pub record: Record,
}

impl Default for TimelineAudio {
    fn default() -> Self {
        Self {
            monitor: Some(FIRST_BUS.into()),
            meter: MeterKind::Ppm,
            custom_meter: CustomMeter::default(),
            prefader_metering: false,
            loudness: Loudness::default(),
            exclusive_solo: false,
            follows_edit: true,
            record: Record::default(),
        }
    }
}

/// The buses of a new timeline: one, `B1`.
#[must_use]
pub fn first_buses() -> Vec<Bus> {
    vec![Bus::first()]
}

/// The settings of a strip that lanes of automation may be of, as an object whose paths are
/// the paths of the lanes: those of its fields (`fader_db`, `trim_db`, `pan.x`,
/// `eq.on`, `eq.gain_db`, `dynamics.compressor.threshold_db`, `track_fx.amount_db`), its bands of
/// equalizer by their place from 0 (`eq.2.gain_db`), its sends and outputs by their bus
/// (`send.B3.level_db`, `send.B3.pan.x`, `output.B2.on`), and its inserts by their id
/// (`insert.fx1.on`, `insert.fx1.ceiling_db`); and `mute`, which a track and a bus have beside
/// it. A number is a setting that changes smoothly, and a lane of it is continuous; a switch is
/// a lane of a setting that is on or off. What is neither (a kind, a mode, a source) has none,
/// nor has a whole number (`eq.<band>.slope`, the semitones of a Pitch), nor what the graph does
/// not move as a lane would (see [`automation::drop_unwired`]: Effects In, the pan's switch,
/// Boom, rotation and spread, whether a send is before the fader, the Lookahead of a Ducker, a
/// Pitch).
#[must_use]
pub fn automatable(strip: &Strip) -> serde_json::Value {
    let mut view = settings_of_strip(strip);
    automation::drop_unwired(&mut view);
    view
}

/// The settings of a strip laid out as [`automatable`] lays them out, but with those that no lane
/// is of yet: what a path names, to say why a lane of it is refused.
pub(crate) fn settings_of_strip(strip: &Strip) -> serde_json::Value {
    let mut view = serde_json::to_value(strip).unwrap_or_default();
    let Some(object) = view.as_object_mut() else {
        return serde_json::Value::Null;
    };
    for name in ["order", "vcas", "sends", "outputs", "inserts"] {
        object.remove(name);
    }
    if let Some(serde_json::Value::Object(eq)) = object.get_mut("eq") {
        if let Some(serde_json::Value::Array(bands)) = eq.remove("bands") {
            for (at, band) in bands.into_iter().enumerate() {
                eq.insert(at.to_string(), band);
            }
        }
    }
    let keyed = |pairs: Vec<(String, serde_json::Value)>| {
        serde_json::Value::Object(pairs.into_iter().collect())
    };
    let send = strip
        .sends
        .iter()
        .map(|s| (s.bus.clone(), serde_json::to_value(s).unwrap_or_default()))
        .collect();
    let output = strip
        .outputs
        .iter()
        .map(|o| (o.bus.clone(), serde_json::json!({ "on": o.on })))
        .collect();
    let insert = strip
        .inserts
        .iter()
        .map(|i| {
            let mut effect = serde_json::to_value(&i.effect).unwrap_or_default();
            if let Some(effect) = effect.as_object_mut() {
                effect.insert("on".into(), i.on.into());
            }
            (i.id.clone(), effect)
        })
        .collect();
    object.insert("send".into(), keyed(send));
    object.insert("output".into(), keyed(output));
    object.insert("insert".into(), keyed(insert));
    object.insert("mute".into(), false.into());
    // Listening (to a sidechain, to what a De-Esser or a Noise Reduction takes away) is only
    // heard while one listens, and is never written: it has no lane.
    unlisten(&mut view);
    view
}

/// Take every `listen` out of a view of settings, at any depth.
fn unlisten(view: &mut serde_json::Value) {
    if let serde_json::Value::Object(object) = view {
        object.remove("listen");
        object.values_mut().for_each(unlisten);
    }
}

/// Give each insert of a strip that has no id the next `fx<n>` of the strip.
pub fn name_inserts(strip: &mut Strip) {
    name_inserts_after(strip, &[]);
}

/// Give each insert of a strip that has no id the next `fx<n>` above every id of the strip and of
/// `before`, the inserts that it had before it was changed: so an insert given no id is always a
/// new one, and never takes the id, and so the lanes, of one that it replaces (T14's review).
pub fn name_inserts_after(strip: &mut Strip, before: &[Insert]) {
    let mut highest = strip
        .inserts
        .iter()
        .chain(before)
        .filter_map(|insert| insert.id.strip_prefix("fx")?.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    for insert in strip.inserts.iter_mut().filter(|i| i.id.is_empty()) {
        highest += 1;
        insert.id = format!("fx{highest}");
    }
}

/// Put the output of a track that names no bus, as one added without a strip does
/// ([`Strip::to_first_bus_if_any`]), to the first bus of its timeline, `B1`, where `buses` has it,
/// and take it away where it has not; and name its inserts that have no id ([`name_inserts`]).
pub fn place_outputs(buses: &[Bus], track: &mut Track) {
    if track.kind != crate::project::TrackKind::Audio {
        return;
    }
    name_inserts(&mut track.audio.strip);
    automation::fill_scales(&mut track.audio.lanes);
    let outputs = &mut track.audio.strip.outputs;
    let named = outputs.iter().any(|o| o.bus == FIRST_BUS);
    if !named && buses.iter().any(|b| b.id == FIRST_BUS) {
        for output in outputs.iter_mut().filter(|o| o.bus.is_empty()) {
            output.bus = FIRST_BUS.into();
        }
    } else {
        outputs.retain(|o| !o.bus.is_empty());
    }
}

/// Check what a track of sound has of its own: its format and channels, the channels of its
/// clips, its strip and its lanes; and that a track of another kind has the default.
///
/// # Errors
/// Its channels do not fit its format, or a clip's do not fit it ([`OpError::ChannelCount`]);
/// or its strip or a lane is not valid.
pub fn check_track(track: &Track) -> Result<(), OpError> {
    let audio = &track.audio;
    let channels = match audio.format.channels() {
        Some(count) => count == audio.channels,
        None => (1..=MOST_CHANNELS).contains(&audio.channels),
    };
    if !channels {
        return Err(OpError::ChannelCount {
            track: track.id.clone(),
        });
    }
    for clip in &track.clips {
        check_channels(track, clip)?;
    }
    audio.strip.check(false)?;
    automation::check_lanes(
        &audio.lanes,
        automation::Owner::Strip {
            strip: &audio.strip,
            mute: audio.mute,
            bus: false,
        },
    )
}

/// Check that a clip on a track of sound plays as many channels as the track has, where it
/// names its channels.
///
/// # Errors
/// It does not: [`OpError::ChannelCount`].
pub fn check_channels(track: &Track, clip: &Clip) -> Result<(), OpError> {
    let fits = track.kind != crate::project::TrackKind::Audio
        || clip
            .channels
            .as_ref()
            .is_none_or(|map| map.len() == usize::from(track.audio.channels));
    if fits {
        Ok(())
    } else {
        Err(OpError::ChannelCount {
            track: track.id.clone(),
        })
    }
}

/// Check the settings of the sound of a clip that lasts at least a frame, on a timeline of
/// `rate`: its gain and pan, its fades, equalizer, effects, pitch and the levels of its channels.
///
/// # Errors
/// A setting is out of its range: `gain_db`, `pan`, `fade`, `eq`, `pitch`, `channels`,
/// `sidechain` (an effect of a clip has no sidechain) or the kind of an effect.
pub fn check_clip(clip: &Clip, rate: Rate) -> Result<(), OpError> {
    if !clip.gain_db.is_valid(|v| (-96.0..=24.0).contains(&v)) {
        return Err(out("gain_db"));
    }
    if !clip.pan.is_valid(|v| (-1.0..=1.0).contains(&v)) {
        return Err(out("pan"));
    }
    // The fades, in seconds, are no longer together than the clip.
    let fade = |fade: &Option<AudioFade>, frames: i64| -> Option<(i128, i128)> {
        match fade {
            None => Some((
                i128::from(frames) * i128::from(rate.den),
                i128::from(rate.num),
            )),
            Some(f) => (f.length.scale > 0
                && (0..=crate::ops::LARGEST).contains(&f.length.ticks)
                && within(f.curve, -1.0, 1.0)
                && within(f.skew, -1.0, 1.0))
            .then_some((i128::from(f.length.ticks), i128::from(f.length.scale))),
        }
    };
    let (Some((a, b)), Some((c, d))) = (
        fade(&clip.audio_fade_in, clip.fade_in),
        fade(&clip.audio_fade_out, clip.fade_out),
    ) else {
        return Err(out("fade"));
    };
    let (length, seconds) = (
        i128::from(clip.length) * i128::from(rate.den),
        i128::from(rate.num),
    );
    // a/b + c/d <= length/seconds, all of them at least 0 and the denominators above 0. Each
    // product is checked: a length of 2^53 frames times three scales of 32 bits passes i128.
    let fades = (|| {
        a.checked_mul(d)?
            .checked_add(c.checked_mul(b)?)?
            .checked_mul(seconds)
    })();
    let room = (|| length.checked_mul(b)?.checked_mul(d))();
    match (fades, room) {
        (Some(fades), Some(room)) if fades <= room => {}
        _ => return Err(out("fade")),
    }
    clip.eq.check(false)?;
    for insert in &clip.effects {
        insert.effect.check()?;
        if insert
            .effect
            .sidechain()
            .is_some_and(|s| s.source.is_some() || s.on)
        {
            return Err(out("sidechain"));
        }
    }
    if !(-12..=12).contains(&clip.pitch.semitones) || !within(clip.pitch.cents, -100.0, 100.0) {
        return Err(out("pitch"));
    }
    for map in clip.channels.iter().flatten() {
        if !(map.level_db.is_finite() && map.level_db <= 24.0) {
            return Err(out("channels"));
        }
    }
    Ok(())
}

/// Check what a timeline has of sound: the strips of its tracks of sound and of its buses, its
/// VCAs, its settings, and its routing ([`routing::check_routing`]).
///
/// # Errors
/// What is wrong, as [`routing::Wrong`].
pub fn check_timeline(timeline: &Timeline) -> Result<(), routing::Wrong> {
    for bus in &timeline.buses {
        let wrong = |error| ("bus", bus.id.clone(), error);
        if bus.name.chars().count() > crate::grade::LONGEST_LABEL {
            return Err(wrong(out("label")));
        }
        bus.strip.check(true).map_err(wrong)?;
        let owner = automation::Owner::Strip {
            strip: &bus.strip,
            mute: bus.mute,
            bus: true,
        };
        automation::check_lanes(&bus.lanes, owner).map_err(wrong)?;
    }
    for group in &timeline.track_groups {
        if group.name.chars().count() > crate::grade::LONGEST_LABEL {
            return Err(("track_group", group.id.clone(), out("label")));
        }
    }
    for vca in &timeline.vcas {
        let wrong = |error| ("vca", vca.id.clone(), error);
        if vca.name.chars().count() > crate::grade::LONGEST_LABEL {
            return Err(wrong(out("label")));
        }
        if !level(vca.fader_db) {
            return Err(wrong(out("fader_db")));
        }
        automation::check_lanes(&vca.lanes, automation::Owner::Vca(vca)).map_err(wrong)?;
    }
    let settings = &timeline.audio;
    let range = settings.record.active_range.is_none_or(|r| {
        r.start.scale > 0
            && r.end.scale > 0
            && r.start.ticks >= 0
            && r.end.ticks <= crate::ops::LARGEST
            && param::earlier(r.start, r.end)
    });
    let fine = within(settings.loudness.target, -36.0, -10.0)
        && within(settings.custom_meter.decay_s, 0.1, 5.0)
        && range;
    if !fine {
        return Err(("timeline", timeline.id.clone(), out("audio")));
    }
    routing::check_routing(timeline)
}

/// What schema 4 writes of a new clip, a new track of sound and a new timeline, beside what
/// schema 3 wrote, as JSON: for the tests that check what is written.
#[cfg(test)]
pub(crate) mod written {
    use serde_json::{Map, Value};

    use crate::project::{Clip, Source, Timeline, Track, TrackKind};
    use crate::time::Rate;

    fn only(value: Value, names: &[&str]) -> Map<String, Value> {
        let Value::Object(object) = value else {
            unreachable!("an object")
        };
        object
            .into_iter()
            .filter(|(name, _)| names.contains(&name.as_str()))
            .collect()
    }

    /// The fields of schema 4 of a new clip.
    pub(crate) fn clip() -> Map<String, Value> {
        let clip = Clip::new(
            "c",
            Source::Color {
                color: crate::project::Rgba([0; 4]),
            },
            0,
            1,
        );
        only(serde_json::to_value(clip).unwrap(), &crate::upgrade::CLIP_4)
    }

    /// The fields of schema 4 of a new track of sound.
    pub(crate) fn track() -> Map<String, Value> {
        let track = Track::new("A1", TrackKind::Audio);
        only(
            serde_json::to_value(track).unwrap(),
            &crate::upgrade::TRACK_4,
        )
    }

    /// The fields of schema 4 of a new timeline.
    pub(crate) fn timeline() -> Map<String, Value> {
        let timeline = Timeline::new("t", "t", Rate { num: 25, den: 1 });
        only(
            serde_json::to_value(timeline).unwrap(),
            &crate::upgrade::TIMELINE_4,
        )
    }

    /// Add the fields of schema 4 to an object of JSON.
    pub(crate) fn with(value: &mut Value, fields: Map<String, Value>) {
        value.as_object_mut().unwrap().extend(fields);
    }
}

#[cfg(test)]
#[allow(
    clippy::field_reassign_with_default,
    reason = "a test changes one setting"
)]
mod tests {
    use super::*;

    #[test]
    fn a_fade_is_a_straight_line_at_no_curve_and_no_skew_and_bends_otherwise() {
        for t in [0.0, 0.1, 0.25, 0.5, 0.9, 1.0] {
            assert_eq!(fade_curve(t, 0.0, 0.0).to_bits(), f64::to_bits(t));
        }
        let near = |a: f64, b: f64| (a - b).abs() < 1e-12;
        let quarter = std::f64::consts::FRAC_PI_4;
        assert!(near(fade_curve(0.5, 1.0, 0.0), quarter.sin()));
        assert!(near(fade_curve(0.5, -1.0, 0.0), 1.0 - quarter.cos()));
        assert!(near(fade_curve(0.5, 0.5, 0.0), 0.25 + 0.5 * quarter.sin()));
        // A skew of 1 reaches the middle of a straight line at a quarter of the way.
        assert!(near(fade_curve(0.25, 0.0, 1.0), 0.5));
        assert!(near(fade_curve(0.25, 0.0, -1.0), 0.0625));
        for (curve, skew) in [(1.0, 1.0), (-1.0, -1.0), (0.3, -0.7)] {
            assert!(near(fade_curve(0.0, curve, skew), 0.0));
            assert!(near(fade_curve(1.0, curve, skew), 1.0));
        }
    }

    #[test]
    fn the_three_levels_of_a_crossfade_rise_and_fall_as_mirror_images() {
        let near = |a: f64, b: f64| (a - b).abs() < 1e-15;
        for level in [
            CrossfadeLevel::Minus3,
            CrossfadeLevel::Zero,
            CrossfadeLevel::Plus3,
        ] {
            for t in [0.0, 0.2, 0.5, 0.7, 1.0] {
                assert!(near(level.fall(t), level.rise(1.0 - t)), "{level:?} {t}");
            }
            assert!(near(level.rise(0.0), 0.0) && near(level.rise(1.0), 1.0));
        }
        let half = std::f64::consts::FRAC_1_SQRT_2;
        assert!(near(CrossfadeLevel::Plus3.rise(0.5), half));
        assert_eq!(CrossfadeLevel::Zero.rise(0.5), 0.5);
        assert!(near(CrossfadeLevel::Minus3.rise(0.5), 0.5f64.powf(1.5)));
        // At the middle, -3 dB and +3 dB are 3.01 dB below and above a straight line.
        let below = 20.0 * (CrossfadeLevel::Minus3.rise(0.5) / 0.5).log10();
        let above = 20.0 * (CrossfadeLevel::Plus3.rise(0.5) / 0.5).log10();
        assert!(
            (below + 3.0103).abs() < 1e-4 && (above - 3.0103).abs() < 1e-4,
            "{below} {above}"
        );
        // +3 dB falls by the cosine, exactly as crossfades did before schema 4.
        let t: f64 = 0.3;
        assert_eq!(
            CrossfadeLevel::Plus3.fall(t).to_bits(),
            (t * std::f64::consts::FRAC_PI_2).cos().to_bits()
        );
        for (level, written) in [
            (CrossfadeLevel::Minus3, "-3"),
            (CrossfadeLevel::Zero, "0"),
            (CrossfadeLevel::Plus3, "3"),
        ] {
            assert_eq!(serde_json::to_string(&level).unwrap(), written);
            assert_eq!(
                serde_json::from_str::<CrossfadeLevel>(written).unwrap(),
                level
            );
        }
        assert_eq!(
            serde_json::from_str::<CrossfadeLevel>("3.0").unwrap(),
            CrossfadeLevel::Plus3
        );
        assert!(serde_json::from_str::<CrossfadeLevel>("2").is_err());
    }

    #[test]
    fn a_level_of_minus_100_db_or_below_is_silence() {
        assert_eq!(gain_of_db(-100.0), 0.0);
        assert_eq!(gain_of_db(-140.0), 0.0);
        assert!((gain_of_db(-6.0) - 0.501_187_233_627_272_2).abs() < 1e-15);
        assert_eq!(gain_of_db(0.0), 1.0);
    }

    #[test]
    fn a_strip_is_refused_with_the_names_of_the_plan() {
        let strip = Strip::to_first_bus();
        strip.check(false).unwrap();
        let mut seven = Strip::default();
        seven.inserts = vec![
            Insert {
                id: String::new(),
                on: true,
                effect: Effect::Gain(fx::GainFx { gain_db: 0.0 }),
            };
            7
        ];
        assert_eq!(seven.check(false), Err(OpError::TooManyInserts));
        let mut sends = Strip::default();
        sends.sends = vec![
            Send {
                bus: "B2".into(),
                on: true,
                pre: false,
                level_db: 0.0,
                pan: Pan::default(),
            };
            11
        ];
        assert_eq!(sends.check(false), Err(OpError::TooManySends));
        let mut outputs = Strip::default();
        outputs.outputs = (0..11)
            .map(|n| Output {
                bus: format!("B{n}"),
                on: true,
            })
            .collect();
        assert_eq!(outputs.check(false), Err(OpError::TooManyOutputs));
        let mut order = Strip::default();
        order.order = vec![Stage::Fx, Stage::Fx, Stage::Eq];
        assert_eq!(order.check(false), Err(OpError::Order));
        order.order = vec![Stage::Eq, Stage::Fx, Stage::Dynamics];
        order.check(false).unwrap();
        let mut ducked = Strip::default();
        ducked.track_fx = Some(fx::Ducker::default());
        ducked.check(false).unwrap();
        assert_eq!(ducked.check(true), Err(out("track_fx")));
        let mut loud = Strip::default();
        loud.fader_db = 10.5;
        assert_eq!(loud.check(false), Err(out("fader_db")));
        loud.fader_db = -1000.0;
        loud.check(false).unwrap();
    }
}
