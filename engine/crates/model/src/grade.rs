//! Grades: the graphs of nodes that colour a picture, as the Color page of `DaVinci Resolve`
//! builds them (Reference Manual 20, Chapters 143–147, pp. 3308–3379), and the parameter that
//! every setting of a node is, a fixed value or keyframes.
//!
//! A grade is a set of nodes and the links between them. The picture comes in at the source,
//! goes through the nodes along their RGB links, and leaves at the output; a node's key, what
//! part of the picture it works on, goes along key links. Each link goes from the output of a
//! node, of the kind of the link, into an input of that kind of another node, numbered from 0;
//! an output may feed any number of inputs, and an input takes at most one link (p. 3319). The
//! source has an RGB output only, and the output an RGB input only. The links never make a
//! loop. A node that is not linked so that the picture runs from the source through it to the
//! output leaves the grade disabled, as in `DaVinci` (pp. 3312, 3319): such a grade is kept, as
//! a colourist leaves one while rebuilding it, and [`Grade::is_complete`] tells it.
//!
//! The nodes of a grade are kept in the order in which they were added, which is how an editor
//! numbers them: added nodes go to the end, so that a node added between nodes 1 and 2 is
//! number 4, and when a node is taken away the ones after it move up a number (p. 3318). A
//! node's id is what it is known by and does not change; it is unique within its grade, and
//! the same node may be in the grades of many clips, as a grade is copied.
//!
//! A project has four levels of grades, in the order that the picture goes through them
//! (p. 3330): the pre-clip grade of a clip's group, the clip's own grade, the post-clip grade of
//! its group, and, once the tracks are put together, the grade of the timeline. [`Target`]
//! names one of them.

use std::collections::{HashMap, HashSet};

use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::ops::{OpError, LARGEST};
use crate::project::Id;

pub mod blur;
pub use blur::{Blur, BlurLinks, BlurMode};
pub mod window;
pub use window::{
    Window, WindowBezierPoint, WindowCombine, WindowPoint, WindowShape, WindowTransform,
};

/// The name of the source of a grade in its links.
pub const SOURCE: &str = "source";

/// The name of the output of a grade in its links.
pub const OUTPUT: &str = "output";

/// The most characters of the label of a node or the name of a group.
pub const LONGEST_LABEL: usize = 100;

/// The most inputs of a mixer.
pub const MOST_INPUTS: u32 = 256;

/// The most characters of the path of a LUT.
pub const LONGEST_PATH: usize = 4096;

// ---------------------------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------------------------

/// A setting that may change over the frames of a clip: one value, or keyframes.
///
/// A fixed value is written as a number, and keyframes as `{"keys": [...]}`, each key
/// `{"frame": 12, "value": 1.2, "interp": "linear"}`. The frame of a key of a clip's grade, or of
/// its group's grades as they apply to it, is a frame of its source, as `DaVinci Resolve` ties
/// the keys of the Color page to the source of a clip: for a clip of media with a picture, the
/// number of the frame of the media, at the media's own rate, whose picture is shown; for a
/// title, a colour or a still, which have no frames of their own, the frame of the clip from its
/// first, 0 (see [`crate::keyframes`]). The frame of a key of a timeline's grade is a frame of the
/// timeline. Keys are in the order of their
/// frames, at most one at a frame, and there is at least one. Before the first key the value is
/// that of the first, and after the last that of the last. How the value goes from a key to the
/// next is told by the key's `interp` (see [`Interp`]).
#[derive(Clone, Debug, PartialEq)]
pub enum Param {
    /// One value, on every frame.
    Fixed(f64),
    /// Keyframes.
    Keys(Vec<Key>),
}

/// A keyframe of a [`Param`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Key {
    /// The frame that it is at.
    pub frame: i64,
    /// The value there.
    pub value: f64,
    /// How the value goes from here to the next key.
    pub interp: Interp,
}

/// How the value of a [`Param`] goes from a key to the next one, as the keyframes of `DaVinci`
/// do: on a straight line, held until the next key and changed there (a static keyframe,
/// p. 3384), or along a curve that eases out of the key and into the next (a dynamic keyframe
/// with an ease, p. 3385).
///
/// It is written `"linear"`, `"hold"` or `{"bezier": {"out": [t, dv], "in": [t, dv]}}`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Interp {
    /// On a straight line.
    Linear,
    /// Held, and changed at the next key.
    Hold,
    /// Along a cubic Bezier curve between the two keys. Each handle is a share of the frames
    /// between the keys, from 0 to 1, and a change of value: `out` from this key, forward, and
    /// `in` from the next key, back. `[0, 0]` for both is a straight line; `[0.33, 0]` for both
    /// eases out and in.
    Bezier {
        /// The handle out of this key.
        out: [f64; 2],
        /// The handle into the next key.
        #[serde(rename = "in")]
        into: [f64; 2],
    },
}

/// A parameter as it is written: an object of keys.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawKeys {
    keys: Vec<Key>,
}

impl Serialize for Param {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Fixed(value) => serializer.serialize_f64(*value),
            Self::Keys(keys) => {
                let mut object = serializer.serialize_struct("Param", 1)?;
                object.serialize_field("keys", keys)?;
                object.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for Param {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::Number(number) => number
                .as_f64()
                .map(Self::Fixed)
                .ok_or_else(|| serde::de::Error::custom("a parameter is not a finite number")),
            serde_json::Value::Object(_) => serde_json::from_value::<RawKeys>(value)
                .map(|raw| Self::Keys(raw.keys))
                .map_err(serde::de::Error::custom),
            _ => Err(serde::de::Error::custom(
                "a parameter is a number or an object of `keys`",
            )),
        }
    }
}

impl From<f64> for Param {
    fn from(value: f64) -> Self {
        Self::Fixed(value)
    }
}

impl Param {
    /// The value at `frame`, which may lie between frames.
    #[must_use]
    pub fn value_at(&self, frame: f64) -> f64 {
        let keys = match self {
            Self::Fixed(value) => return *value,
            Self::Keys(keys) => keys,
        };
        let (Some(first), Some(last)) = (keys.first(), keys.last()) else {
            return 0.0;
        };
        #[allow(clippy::cast_precision_loss, reason = "frames are within 2^53")]
        let at = |key: &Key| key.frame as f64;
        if frame <= at(first) {
            return first.value;
        }
        if frame >= at(last) {
            return last.value;
        }
        // The last key at or before the frame; there is one after it.
        let i = keys.partition_point(|key| at(key) <= frame) - 1;
        let (from, to) = (&keys[i], &keys[i + 1]);
        let share = (frame - at(from)) / (at(to) - at(from));
        match from.interp {
            Interp::Linear => from.value + (to.value - from.value) * share,
            Interp::Hold => from.value,
            Interp::Bezier { out, into } => bezier(
                share,
                [0.0, out[0], 1.0 - into[0], 1.0],
                [
                    from.value,
                    from.value + out[1],
                    to.value + into[1],
                    to.value,
                ],
            ),
        }
    }

    /// Whether it has keyframes.
    #[must_use]
    pub fn is_animated(&self) -> bool {
        matches!(self, Self::Keys(_))
    }

    /// Check that it is a value from `low` to `high`, or keys of such values at frames in order
    /// within [`LARGEST`] of 0, with handles that are finite shares from 0 to 1.
    pub(crate) fn check(&self, what: &'static str, low: f64, high: f64) -> Result<(), OpError> {
        let fits = |value: f64| value.is_finite() && (low..=high).contains(&value);
        let ok = match self {
            Self::Fixed(value) => fits(*value),
            Self::Keys(keys) => {
                !keys.is_empty()
                    && keys.windows(2).all(|pair| pair[0].frame < pair[1].frame)
                    && keys.iter().all(|key| {
                        key.frame.unsigned_abs() <= LARGEST.unsigned_abs()
                            && fits(key.value)
                            && match key.interp {
                                Interp::Bezier { out, into } => [out, into].iter().all(|handle| {
                                    (0.0..=1.0).contains(&handle[0]) && handle[1].is_finite()
                                }),
                                _ => true,
                            }
                    })
            }
        };
        if ok {
            Ok(())
        } else {
            Err(OpError::OutOfRange { what })
        }
    }
}

/// The point at `share` of the time of a cubic Bezier curve whose control points are at times
/// `x` and values `y`, where `x` goes from 0 to 1 and never back, as it does when its two inner
/// points are from 0 to 1.
fn bezier(share: f64, x: [f64; 4], y: [f64; 4]) -> f64 {
    let at = |p: [f64; 4], s: f64| {
        let r = 1.0 - s;
        r * r * r * p[0] + 3.0 * r * r * s * p[1] + 3.0 * r * s * s * p[2] + s * s * s * p[3]
    };
    let (mut low, mut high) = (0.0_f64, 1.0_f64);
    for _ in 0..64 {
        let middle = f64::midpoint(low, high);
        if at(x, middle) < share {
            low = middle;
        } else {
            high = middle;
        }
    }
    at(y, f64::midpoint(low, high))
}

/// The value a parameter is at first, and the values it may take.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    /// The value that changes nothing, which a new node has.
    pub default: f64,
    /// The lowest value.
    pub low: f64,
    /// The highest value.
    pub high: f64,
}

impl Bounds {
    const fn new(default: f64, low: f64, high: f64) -> Self {
        Self { default, low, high }
    }

    fn param(self) -> Param {
        Param::Fixed(self.default)
    }

    fn check(self, what: &'static str, param: &Param) -> Result<(), OpError> {
        param.check(what, self.low, self.high)
    }
}

/// The bounds of the settings of a corrector, in the units that `DaVinci` shows them in.
///
/// Where the manual gives a range or a default, the page is named. The neutral values of the
/// wheels are those that `DaVinci` shows (Lift and Gamma 0, Gain 1, Offset 25) and their ranges
/// are ours: the manual gives neither, and its formulas are not published.
pub mod bounds {
    use super::Bounds;

    /// Lift, Y, R, G and B: 0 changes nothing.
    pub const LIFT: Bounds = Bounds::new(0.0, -1.0, 1.0);
    /// Gamma, Y, R, G and B: 0 changes nothing.
    pub const GAMMA: Bounds = Bounds::new(0.0, -1.0, 1.0);
    /// Gain, Y, R, G and B: 1 changes nothing.
    pub const GAIN: Bounds = Bounds::new(1.0, 0.0, 16.0);
    /// Offset, R, G and B: 25 changes nothing; it has no Y (p. 3067).
    pub const OFFSET: Bounds = Bounds::new(25.0, -75.0, 125.0);
    /// Contrast: 1 changes nothing.
    pub const CONTRAST: Bounds = Bounds::new(1.0, 0.0, 2.0);
    /// Pivot, the level that contrast stretches about.
    pub const PIVOT: Bounds = Bounds::new(0.435, 0.0, 1.0);
    /// Saturation: 50 changes nothing, 0 takes all colour away and 100 doubles it (p. 3064).
    pub const SATURATION: Bounds = Bounds::new(50.0, 0.0, 100.0);
    /// Hue: 50 changes nothing (p. 3064); a turn of the hues each way.
    pub const HUE: Bounds = Bounds::new(50.0, 0.0, 100.0);
    /// Lum Mix: 100 by default (p. 3064).
    pub const LUM_MIX: Bounds = Bounds::new(100.0, 0.0, 100.0);
    /// Temp: 0 changes nothing, from -4000 to 4000 (p. 3063).
    pub const TEMPERATURE: Bounds = Bounds::new(0.0, -4000.0, 4000.0);
    /// Tint: 0 changes nothing, from -100 to 100 (p. 3063).
    pub const TINT: Bounds = Bounds::new(0.0, -100.0, 100.0);
    /// Color Boost: 0 changes nothing, from -100 to 100 (p. 3063).
    pub const COLOR_BOOST: Bounds = Bounds::new(0.0, -100.0, 100.0);
    /// Midtone Detail: 0 changes nothing, from -100 to 100 (p. 3063).
    pub const MIDTONE_DETAIL: Bounds = Bounds::new(0.0, -100.0, 100.0);
    /// Shadows: 0 changes nothing, from -100 to 100 (p. 3063).
    pub const SHADOWS: Bounds = Bounds::new(0.0, -100.0, 100.0);
    /// Highlights: 0 changes nothing, from -100 to 100 (p. 3064).
    pub const HIGHLIGHTS: Bounds = Bounds::new(0.0, -100.0, 100.0);
    /// The Shadow, Midtone and Highlight wheels of the Log mode, R, G and B: 0 changes nothing.
    pub const LOG_WHEEL: Bounds = Bounds::new(0.0, -1.0, 1.0);
    /// Low Range, where the shadows meet the midtones in the Log mode (p. 3073). The default
    /// is the one that `DaVinci` shows, to be checked against it.
    pub const LOW_RANGE: Bounds = Bounds::new(0.333, 0.0, 1.0);
    /// High Range, where the midtones meet the highlights in the Log mode (p. 3073). The
    /// default is the one that `DaVinci` shows, to be checked against it.
    pub const HIGH_RANGE: Bounds = Bounds::new(0.55, 0.0, 1.0);
    /// The intensity of a custom curve: 100 gives all of it and 0 none (p. 3116).
    pub const CURVE_INTENSITY: Bounds = Bounds::new(100.0, 0.0, 100.0);
    /// The level below which Soft Clip clips, as a share of 100 of the 10-bit scale: 0, level
    /// 0, by default (p. 3120).
    pub const CLIP_LOW: Bounds = Bounds::new(0.0, 0.0, 100.0);
    /// The level above which Soft Clip clips, as a share of 100 of the 10-bit scale: 100,
    /// level 1023, by default (p. 3119).
    pub const CLIP_HIGH: Bounds = Bounds::new(100.0, 0.0, 100.0);
    /// How softly Soft Clip clips, low or high: 0 does not (pp. 3120–3121).
    pub const CLIP_SOFT: Bounds = Bounds::new(0.0, 0.0, 100.0);
    /// The gain of a key, coming in or going out: 1 by default, 0 takes the key away and 2 is
    /// the most (pp. 3362–3363).
    pub const KEY_GAIN: Bounds = Bounds::new(1.0, 0.0, 2.0);
    /// The offset of a key, which changes its contrast (pp. 3362–3363): 0 changes nothing.
    pub const KEY_OFFSET: Bounds = Bounds::new(0.0, -1.0, 1.0);
    /// The exponent of a channel of a shot match (see [`super::ShotMatch`]): 1 changes nothing.
    pub const SHOT_GAMMA: Bounds = Bounds::new(1.0, 0.1, 10.0);
    /// The gain of a channel of a shot match: 1 changes nothing.
    pub const SHOT_GAIN: Bounds = Bounds::new(1.0, 0.0, 16.0);
    /// What a shot match adds to a channel, white being 1: 0 changes nothing.
    pub const SHOT_OFFSET: Bounds = Bounds::new(0.0, -1.0, 1.0);
    /// The contrast of a shot match, the slope of its S-curve at the pivot: 1 changes nothing.
    pub const SHOT_CONTRAST: Bounds = Bounds::new(1.0, 0.0, 4.0);
    /// The pivot of the contrast of a shot match.
    pub const SHOT_PIVOT: Bounds = Bounds::new(0.435, 0.0, 1.0);
    /// The blur of the key that comes into a corrector (Key Input Blur Radius, p. 3362), and of
    /// the key of a qualifier (Matte Finesse Blur Radius, "0 to 2000, with 0 being the default",
    /// p. 3180): 0 blurs nothing.
    pub const KEY_BLUR: Bounds = Bounds::new(0.0, 0.0, 2000.0);
    /// How a blur of the key that comes in is shared between across and down (Blur H/V Ratio,
    /// p. 3362): 0.5, the same both ways, by default (the manual gives none), 1 across alone and
    /// 0 down alone.
    pub const KEY_BLUR_RATIO: Bounds = Bounds::new(0.5, 0.0, 1.0);
    /// The centre of the hues of an HSL qualifier (Hue Center, p. 3173), a turn of hue from 0 to
    /// 100, red at 0, green at 33.3 and blue at 66.7, going round.
    pub const HUE_CENTER: Bounds = Bounds::new(50.0, 0.0, 100.0);
    /// How wide the hues of an HSL qualifier are (Hue Width, p. 3173): 100, every hue, until a
    /// colour is picked.
    pub const HUE_WIDTH: Bounds = Bounds::new(100.0, 0.0, 100.0);
    /// How softly the hues of an HSL qualifier fall off on both sides (Hue Soft, p. 3173), in
    /// hundredths of a turn.
    pub const HUE_SOFT: Bounds = Bounds::new(0.0, 0.0, 100.0);
    /// How the softness of the hues is shared between their sides (Hue Symmetry, p. 3173): 50
    /// alike; lower, the right side steeper; higher, the left.
    pub const HUE_SYMMETRY: Bounds = Bounds::new(50.0, 0.0, 100.0);
    /// The lowest of a range of a qualifier (Low, pp. 3173–3175), 0 to 100 of black to white.
    pub const RANGE_LOW: Bounds = Bounds::new(0.0, 0.0, 100.0);
    /// The highest of a range of a qualifier (High).
    pub const RANGE_HIGH: Bounds = Bounds::new(100.0, 0.0, 100.0);
    /// How softly a range of a qualifier falls off below its lowest or above its highest (Low
    /// Soft, High Soft).
    pub const RANGE_SOFT: Bounds = Bounds::new(0.0, 0.0, 100.0);
    /// Matte Finesse's Pre-Filter, a blur of the picture before it is qualified (p. 3178).
    pub const PRE_FILTER: Bounds = Bounds::new(0.0, 0.0, 100.0);
    /// Clean Black and Clean White (pp. 3178–3179): 0 cleans nothing.
    pub const CLEAN: Bounds = Bounds::new(0.0, 0.0, 100.0);
    /// Black Clip: "The range is 0 to 100, with 0 being the default" (p. 3179).
    pub const BLACK_CLIP: Bounds = Bounds::new(0.0, 0.0, 100.0);
    /// White Clip: "The range is 0 to 100, with 100 being the default" (p. 3180).
    pub const WHITE_CLIP: Bounds = Bounds::new(100.0, 0.0, 100.0);
    /// In/Out Ratio (p. 3180): 0 blurs inside and outside the edge alike, below 0 inside alone
    /// and above 0 outside alone.
    pub const IN_OUT_RATIO: Bounds = Bounds::new(0.0, -100.0, 100.0);
    /// Morph Radius (p. 3181), in pixels of a frame 1080 high: 0 shrinks or grows nothing.
    pub const MORPH_RADIUS: Bounds = Bounds::new(0.0, 0.0, 100.0);
}

// ---------------------------------------------------------------------------------------------
// Nodes
// ---------------------------------------------------------------------------------------------

/// A node of a grade.
///
/// It is written `{"id", "kind", "label", "enabled", "locked", "pos", "params"}`, where `params`
/// is what a node of its kind is set by: a [`Corrector`], a [`ParallelMixer`], a [`LayerMixer`]
/// or a [`KeyMixer`]. `locked` and `pos` are always written, and may be left out where a node
/// is read: they are then `false` and `null`.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// What it is known by in its grade: `n` and a number where the engine made it. Never
    /// `source` or `output`.
    pub id: Id,
    /// The name a colourist gave it, or empty: at most [`LONGEST_LABEL`] characters. An editor
    /// shows 12 of them at the default size (p. 3313).
    pub label: String,
    /// Whether it does what it is set to. A node that is not passes its picture through.
    pub enabled: bool,
    /// Whether it is locked, as Lock Node locks it (pp. 3297, 3382): "prevent any changes from
    /// being made" to it, so that its settings are not changed by accident. Its `params` are not
    /// set while it is ([`crate::OpError::NodeLocked`]); its label, whether it is enabled, where it
    /// lies, its links, and whether it is in its grade at all are.
    pub locked: bool,
    /// Where it lies in the node editor, or none for where the editor lays it out (see
    /// [`Pos`]). `DaVinci` keeps the layout of a node graph with the grade, so that a colourist
    /// who comes back to it, or another, finds it as it was left (p. 3321).
    pub pos: Option<Pos>,
    /// What kind of node it is, and how it is set.
    pub kind: NodeKind,
}

/// Where a node lies in the node editor: the top left corner of its card, in the editor's
/// units, a pixel at a zoom of 1, `x` to the right and `y` down. Each is finite and within
/// [`FURTHEST_POS`] of 0.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pos {
    /// How far right.
    pub x: f64,
    /// How far down.
    pub y: f64,
}

/// The furthest from 0 that a node may lie, either way.
pub const FURTHEST_POS: f64 = 1_000_000.0;

/// How far apart the columns of nodes are across the node editor, as the editor of the studio
/// lays them out: a card and the room after it (`NODE_WIDTH + COLUMN_GAP` of
/// `desktop/src/studio/color/graph.ts`). Nodes that are put after a grade's own, as Append Node
/// Graph puts them (p. 3290), are moved right by columns of this.
pub const NODE_STEP: f64 = 148.0;

/// Where the editor of the studio puts the first column of nodes that it lays out
/// (`columnX(1)` of `desktop/src/studio/color/graph.ts`), and so where a node that lies nowhere
/// of its own may be as far left as.
pub const FIRST_COLUMN: f64 = 102.0;

impl Pos {
    /// Check it: each is finite and within [`FURTHEST_POS`] of 0.
    pub(crate) fn check(self) -> Result<(), OpError> {
        let fits = |v: f64| v.is_finite() && v.abs() <= FURTHEST_POS;
        if fits(self.x) && fits(self.y) {
            Ok(())
        } else {
            Err(OpError::OutOfRange { what: "pos" })
        }
    }
}

/// What kind of node a node is, and how it is set.
#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    /// A node that corrects the colour of its picture (p. 3311).
    Corrector(Box<Corrector>),
    /// A node that mixes its inputs equally (pp. 3334–3335).
    ParallelMixer(ParallelMixer),
    /// A node that lays its inputs over one another, the lowest input on top (pp. 3336–3338).
    LayerMixer(LayerMixer),
    /// A node that mixes the keys of its inputs into one (pp. 3356–3360).
    KeyMixer(KeyMixer),
}

/// The kinds of node, as they are named in `kind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// [`NodeKind::Corrector`].
    Corrector,
    /// [`NodeKind::ParallelMixer`].
    ParallelMixer,
    /// [`NodeKind::LayerMixer`].
    LayerMixer,
    /// [`NodeKind::KeyMixer`].
    KeyMixer,
}

impl Kind {
    /// Its name, as it is written.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Corrector => "corrector",
            Self::ParallelMixer => "parallel_mixer",
            Self::LayerMixer => "layer_mixer",
            Self::KeyMixer => "key_mixer",
        }
    }
}

/// A node as it is written, before its `params` are read for its kind.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawNode {
    id: Id,
    kind: Kind,
    label: String,
    enabled: bool,
    #[serde(default)]
    locked: bool,
    #[serde(default)]
    pos: Option<Pos>,
    params: serde_json::Value,
}

impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RawNode::deserialize(deserializer)?;
        let kind = NodeKind::from_params(raw.kind, raw.params).map_err(serde::de::Error::custom)?;
        Ok(Self {
            id: raw.id,
            label: raw.label,
            enabled: raw.enabled,
            locked: raw.locked,
            pos: raw.pos,
            kind,
        })
    }
}

impl Serialize for Node {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut node = serializer.serialize_struct("Node", 7)?;
        node.serialize_field("id", &self.id)?;
        node.serialize_field("kind", &self.kind.kind())?;
        node.serialize_field("label", &self.label)?;
        node.serialize_field("enabled", &self.enabled)?;
        node.serialize_field("locked", &self.locked)?;
        node.serialize_field("pos", &self.pos)?;
        match &self.kind {
            NodeKind::Corrector(params) => node.serialize_field("params", params)?,
            NodeKind::ParallelMixer(params) => node.serialize_field("params", params)?,
            NodeKind::LayerMixer(params) => node.serialize_field("params", params)?,
            NodeKind::KeyMixer(params) => node.serialize_field("params", params)?,
        }
        node.end()
    }
}

impl NodeKind {
    /// Its kind.
    #[must_use]
    pub fn kind(&self) -> Kind {
        match self {
            Self::Corrector(_) => Kind::Corrector,
            Self::ParallelMixer(_) => Kind::ParallelMixer,
            Self::LayerMixer(_) => Kind::LayerMixer,
            Self::KeyMixer(_) => Kind::KeyMixer,
        }
    }

    /// A node of `kind` as a new one is set: one that changes nothing, and a mixer with two
    /// inputs.
    #[must_use]
    pub fn new(kind: Kind) -> Self {
        match kind {
            Kind::Corrector => Self::Corrector(Box::default()),
            Kind::ParallelMixer => Self::ParallelMixer(ParallelMixer::default()),
            Kind::LayerMixer => Self::LayerMixer(LayerMixer::default()),
            Kind::KeyMixer => Self::KeyMixer(KeyMixer::default()),
        }
    }

    /// Read the settings of a node of `kind`.
    ///
    /// # Errors
    /// They are not the settings of a node of that kind.
    pub fn from_params(kind: Kind, params: serde_json::Value) -> Result<Self, serde_json::Error> {
        Ok(match kind {
            Kind::Corrector => Self::Corrector(serde_json::from_value(params)?),
            Kind::ParallelMixer => Self::ParallelMixer(serde_json::from_value(params)?),
            Kind::LayerMixer => Self::LayerMixer(serde_json::from_value(params)?),
            Kind::KeyMixer => Self::KeyMixer(serde_json::from_value(params)?),
        })
    }

    /// Its settings, as they are written in `params`.
    ///
    /// # Panics
    /// Never: the settings of a node are always written as JSON.
    #[must_use]
    pub fn params(&self) -> serde_json::Value {
        let value = match self {
            Self::Corrector(params) => serde_json::to_value(params),
            Self::ParallelMixer(params) => serde_json::to_value(params),
            Self::LayerMixer(params) => serde_json::to_value(params),
            Self::KeyMixer(params) => serde_json::to_value(params),
        };
        value.expect("the settings of a node are written as JSON")
    }

    /// How many inputs of `port` it has.
    #[must_use]
    pub fn inputs(&self, port: Port) -> u32 {
        match (self, port) {
            (Self::Corrector(_), _) => 1,
            (Self::ParallelMixer(mixer), Port::Rgb) => mixer.inputs,
            (Self::LayerMixer(mixer), Port::Rgb) => mixer.inputs,
            (Self::KeyMixer(mixer), Port::Key) => {
                u32::try_from(mixer.inputs.len()).unwrap_or(u32::MAX)
            }
            _ => 0,
        }
    }

    /// Whether it has an output of `port`.
    #[must_use]
    pub fn has_output(&self, port: Port) -> bool {
        match self {
            Self::Corrector(_) => true,
            Self::ParallelMixer(_) | Self::LayerMixer(_) => port == Port::Rgb,
            Self::KeyMixer(_) => port == Port::Key,
        }
    }

    /// Check its settings.
    pub(crate) fn check(&self) -> Result<(), OpError> {
        match self {
            Self::Corrector(corrector) => corrector.check(),
            Self::ParallelMixer(ParallelMixer { inputs })
            | Self::LayerMixer(LayerMixer { inputs, .. }) => {
                if (1..=MOST_INPUTS).contains(inputs) {
                    Ok(())
                } else {
                    Err(OpError::OutOfRange { what: "inputs" })
                }
            }
            Self::KeyMixer(mixer) => {
                let count = mixer.inputs.len();
                if count == 0 || count > MOST_INPUTS as usize {
                    return Err(OpError::OutOfRange { what: "inputs" });
                }
                for input in &mixer.inputs {
                    input.check()?;
                }
                mixer.output.check()
            }
        }
    }
}

impl Node {
    /// A node of `kind` as a new one is set, enabled, unlocked, with no label, where the editor
    /// lays it out.
    #[must_use]
    pub fn new(id: impl Into<Id>, kind: Kind) -> Self {
        Self {
            id: id.into(),
            label: String::new(),
            enabled: true,
            locked: false,
            pos: None,
            kind: NodeKind::new(kind),
        }
    }

    /// A corrector that changes nothing.
    #[must_use]
    pub fn corrector(id: impl Into<Id>) -> Self {
        Self::new(id, Kind::Corrector)
    }

    /// Its corrector, if it is one.
    #[must_use]
    pub fn as_corrector(&self) -> Option<&Corrector> {
        match &self.kind {
            NodeKind::Corrector(corrector) => Some(corrector),
            _ => None,
        }
    }
}

/// The settings of a corrector node, grouped as the palettes of the Color page group them.
///
/// Within the node they are applied in the order of p. 3331: Shadows and Highlights, Midtone
/// Detail, Color Boost, Temp and Tint, Lift, Gamma and Gain, Contrast and Pivot, the custom
/// curves, the Log wheels, Hue and Saturation, the LUT, the HSL curves and Soft Clip; and last
/// the adjustment of Shot Match, which no palette shows (p. 3048).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Corrector {
    /// The Primaries palette (pp. 3058–3076).
    pub primaries: Primaries,
    /// The Log mode of the Primaries palette (pp. 3068–3073).
    pub log: LogWheels,
    /// The custom curves (pp. 3112–3116).
    pub curves: Curves,
    /// The HSL curves (pp. 3121–3126).
    pub hsl: HslCurves,
    /// The LUT of the node, by its path or its name in the library of LUTs, or none.
    pub lut: Option<String>,
    /// Soft Clip (pp. 3117–3121).
    pub soft_clip: SoftClip,
    /// The Blur palette (pp. 3454–3456; see [`Blur`]), done after Soft Clip and the shot match,
    /// before the node's key. It is always written, and read as it is at first where it is left
    /// out.
    #[serde(default)]
    pub blur: Blur,
    /// The Key palette of a corrector (pp. 3361–3362).
    pub key: KeyParams,
    /// The adjustment that Shot Match put on the node, or none (see [`ShotMatch`]). It is
    /// always written, and read as none where it is left out.
    #[serde(default)]
    pub shot_match: Option<ShotMatch>,
    /// The Qualifier palette (pp. 3157–3182): the key that the node makes of its own picture,
    /// or none, as a node has none until a colour is picked (see [`Qualifier`]). It is always
    /// written, and read as none where it is left out.
    #[serde(default)]
    pub qualifier: Option<Qualifier>,
    /// The Power Window palette (pp. 3185–3199), in list order. Old projects without it read
    /// as an empty list. A preset copies this whole list into the project.
    #[serde(default)]
    pub windows: Vec<Window>,
}

/// The Qualifier palette of a corrector: which pixels of the picture that comes into the node it
/// works on, a key of 0 (not at all) to 1 (fully), made of the pixel's hue, saturation and luma
/// (HSL), its red, green and blue (RGB), or its luma alone (LUM) (pp. 3160–3175); then refined by
/// the Matte Finesse controls (pp. 3178–3181) and inverted if it is set to be (p. 3182).
///
/// Each range is set by its lowest and highest (from 0 to 100, black to white of the 0 to 1 of
/// the node's input) and how softly it falls off beyond them; hue, which goes round, by its centre
/// and width, how softly it falls off on both sides, and how that softness is shared between them.
/// A component that is off (the toggles left of each name, p. 3173) does not limit the key; in
/// LUM mode hue and saturation do not (p. 3175, "identical to the HSL qualifier with H and S
/// turned off"), and LUM shares the range of luma with HSL. As a qualifier is first, it keeps
/// every pixel: its key is 1 everywhere. To give a node one where it has none, `set_node` gives it
/// whole; one that the node has is patched as any other setting, and `{"qualifier": null}` takes
/// it away.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Qualifier {
    /// Which components of the pixel it keys by.
    pub mode: QualifierMode,
    /// Whether its key is inverted (the Qualifier palette's Invert, p. 3182), after Matte
    /// Finesse.
    pub invert: bool,
    /// Hue, of HSL.
    pub hue: HueRange,
    /// Saturation, of HSL.
    pub sat: Range,
    /// Luma, of HSL and LUM.
    pub lum: Range,
    /// Red, of RGB.
    pub red: Range,
    /// Green, of RGB.
    pub green: Range,
    /// Blue, of RGB.
    pub blue: Range,
    /// Matte Finesse, which refines the key (pp. 3178–3181).
    pub finesse: MatteFinesse,
}

impl Default for Qualifier {
    fn default() -> Self {
        Self {
            mode: QualifierMode::Hsl,
            invert: false,
            hue: HueRange::default(),
            sat: Range::default(),
            lum: Range::default(),
            red: Range::default(),
            green: Range::default(),
            blue: Range::default(),
            finesse: MatteFinesse::default(),
        }
    }
}

/// The modes of the Qualifier palette that are kept (pp. 3160–3175): HSL by default (p. 3167).
/// The 3D qualifier is not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualifierMode {
    /// Hue, saturation and luma.
    Hsl,
    /// Red, green and blue.
    Rgb,
    /// Luma alone.
    Lum,
}

/// The range of hue of an HSL qualifier (p. 3173): `center`, `width` (100 every hue), `soft` on
/// both sides and its `symmetry` (50 alike), each a share of 100 of a turn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HueRange {
    /// Whether hue limits the key.
    pub on: bool,
    /// Hue Center.
    pub center: Param,
    /// Hue Width.
    pub width: Param,
    /// Hue Soft.
    pub soft: Param,
    /// Hue Symmetry.
    pub symmetry: Param,
}

impl Default for HueRange {
    fn default() -> Self {
        use bounds::{HUE_CENTER, HUE_SOFT, HUE_SYMMETRY, HUE_WIDTH};
        Self {
            on: true,
            center: HUE_CENTER.param(),
            width: HUE_WIDTH.param(),
            soft: HUE_SOFT.param(),
            symmetry: HUE_SYMMETRY.param(),
        }
    }
}

/// A range of saturation, luma, red, green or blue of a qualifier (pp. 3173–3175): from `low` to
/// `high` (0 to 100), falling off softly by `low_soft` below and `high_soft` above.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Range {
    /// Whether it limits the key.
    pub on: bool,
    /// Low.
    pub low: Param,
    /// High.
    pub high: Param,
    /// Low Soft.
    pub low_soft: Param,
    /// High Soft.
    pub high_soft: Param,
}

impl Default for Range {
    fn default() -> Self {
        use bounds::{RANGE_HIGH, RANGE_LOW, RANGE_SOFT};
        Self {
            on: true,
            low: RANGE_LOW.param(),
            high: RANGE_HIGH.param(),
            low_soft: RANGE_SOFT.param(),
            high_soft: RANGE_SOFT.param(),
        }
    }
}

impl Range {
    fn check(&self, names: [&'static str; 4]) -> Result<(), OpError> {
        use bounds::{RANGE_HIGH, RANGE_LOW, RANGE_SOFT};
        RANGE_LOW.check(names[0], &self.low)?;
        RANGE_HIGH.check(names[1], &self.high)?;
        RANGE_SOFT.check(names[2], &self.low_soft)?;
        RANGE_SOFT.check(names[3], &self.high_soft)
    }
}

/// The Matte Finesse controls of a qualifier that are kept (pp. 3178–3181), in the order that
/// they refine its key: Pre-Filter blurs the picture before it is qualified; Clean Black and
/// Clean White take specks and holes out of the key; Black Clip and White Clip move its levels;
/// Blur Radius blurs it, inside or outside its edge as In/Out Ratio says; and Morph Operation
/// shrinks, grows, opens or closes it by Morph Radius. Each changes nothing at its default.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatteFinesse {
    /// Pre-Filter.
    pub pre_filter: Param,
    /// Clean Black.
    pub clean_black: Param,
    /// Black Clip.
    pub black_clip: Param,
    /// Clean White.
    pub clean_white: Param,
    /// White Clip.
    pub white_clip: Param,
    /// Blur Radius.
    pub blur_radius: Param,
    /// In/Out Ratio.
    pub in_out_ratio: Param,
    /// Morph Operation.
    pub morph: Morph,
    /// Morph Radius.
    pub morph_radius: Param,
}

impl Default for MatteFinesse {
    fn default() -> Self {
        use bounds::{
            BLACK_CLIP, CLEAN, IN_OUT_RATIO, KEY_BLUR, MORPH_RADIUS, PRE_FILTER, WHITE_CLIP,
        };
        Self {
            pre_filter: PRE_FILTER.param(),
            clean_black: CLEAN.param(),
            black_clip: BLACK_CLIP.param(),
            clean_white: CLEAN.param(),
            white_clip: WHITE_CLIP.param(),
            blur_radius: KEY_BLUR.param(),
            in_out_ratio: IN_OUT_RATIO.param(),
            morph: Morph::Shrink,
            morph_radius: MORPH_RADIUS.param(),
        }
    }
}

/// Matte Finesse's Morph Operation (p. 3181): shrink or grow the key by Morph Radius, or open it
/// (shrink, then grow: specks go and the rest is as it was) or close it (grow, then shrink:
/// holes are filled).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Morph {
    /// Shrink.
    Shrink,
    /// Grow.
    Grow,
    /// Opening.
    Opening,
    /// Closing.
    Closing,
}

impl Qualifier {
    fn check(&self) -> Result<(), OpError> {
        use bounds::{
            BLACK_CLIP, CLEAN, HUE_CENTER, HUE_SOFT, HUE_SYMMETRY, HUE_WIDTH, IN_OUT_RATIO,
            KEY_BLUR, MORPH_RADIUS, PRE_FILTER, WHITE_CLIP,
        };
        let hue = &self.hue;
        HUE_CENTER.check("qualifier.hue.center", &hue.center)?;
        HUE_WIDTH.check("qualifier.hue.width", &hue.width)?;
        HUE_SOFT.check("qualifier.hue.soft", &hue.soft)?;
        HUE_SYMMETRY.check("qualifier.hue.symmetry", &hue.symmetry)?;
        for (range, names) in [
            (
                &self.sat,
                [
                    "qualifier.sat.low",
                    "qualifier.sat.high",
                    "qualifier.sat.low_soft",
                    "qualifier.sat.high_soft",
                ],
            ),
            (
                &self.lum,
                [
                    "qualifier.lum.low",
                    "qualifier.lum.high",
                    "qualifier.lum.low_soft",
                    "qualifier.lum.high_soft",
                ],
            ),
            (
                &self.red,
                [
                    "qualifier.red.low",
                    "qualifier.red.high",
                    "qualifier.red.low_soft",
                    "qualifier.red.high_soft",
                ],
            ),
            (
                &self.green,
                [
                    "qualifier.green.low",
                    "qualifier.green.high",
                    "qualifier.green.low_soft",
                    "qualifier.green.high_soft",
                ],
            ),
            (
                &self.blue,
                [
                    "qualifier.blue.low",
                    "qualifier.blue.high",
                    "qualifier.blue.low_soft",
                    "qualifier.blue.high_soft",
                ],
            ),
        ] {
            range.check(names)?;
        }
        let finesse = &self.finesse;
        PRE_FILTER.check("qualifier.finesse.pre_filter", &finesse.pre_filter)?;
        CLEAN.check("qualifier.finesse.clean_black", &finesse.clean_black)?;
        BLACK_CLIP.check("qualifier.finesse.black_clip", &finesse.black_clip)?;
        CLEAN.check("qualifier.finesse.clean_white", &finesse.clean_white)?;
        WHITE_CLIP.check("qualifier.finesse.white_clip", &finesse.white_clip)?;
        KEY_BLUR.check("qualifier.finesse.blur_radius", &finesse.blur_radius)?;
        IN_OUT_RATIO.check("qualifier.finesse.in_out_ratio", &finesse.in_out_ratio)?;
        MORPH_RADIUS.check("qualifier.finesse.morph_radius", &finesse.morph_radius)
    }
}

/// The adjustment that Shot Match puts on the node that was chosen when it was used: "applied
/// invisibly, as the very last adjustment to the node … similar to a LUT" (p. 3048). No palette
/// shows it; it is undone, copied and reset with the rest of the node, and a node shows that it
/// has one.
///
/// `DaVinci` does not say what it is. Ours is a small adjustment of each channel that a solver
/// can fit to the statistics of two pictures (their levels at a few shares of their pixels, the
/// mean of their grey), on R'G'B' after Soft Clip, each channel `c` of `x` going to
///
/// `S(gain_c · x^gamma_c + offset_c)`
///
/// where the power keeps the sign of `x`, and `S` is the contrast `contrast` about `pivot` as an
/// S-curve: `p (v/p)^k` below the pivot `p` and `1 - (1 - p)((1 - v)/(1 - p))^k` above, values at
/// or beyond black and white kept. So `gamma`, `gain` and `offset` move the black, the middle and
/// the white of each channel apart, and `contrast` stretches all three about the pivot without
/// clipping, which the others cannot do. `S` is always the S-curve, whatever the project says
/// of the Contrast of the palette, so that what a solver fits does not change with the project.
///
/// Its settings as they are first are the identity, to the bit, as no shot match is. To set one
/// where a node has none, `set_node` gives it whole; one that the node has is patched as any
/// other setting.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShotMatch {
    /// The exponent of each channel: 1 changes nothing.
    pub gamma: Rgb,
    /// The gain of each channel, after the exponent: 1 changes nothing.
    pub gain: Rgb,
    /// What is added to each channel, after the gain, on the scale where 1 is white: 0 changes
    /// nothing.
    pub offset: Rgb,
    /// The contrast of the S-curve, the slope at its pivot: 1 changes nothing.
    pub contrast: Param,
    /// The pivot of the S-curve.
    pub pivot: Param,
}

impl Default for ShotMatch {
    fn default() -> Self {
        use bounds::{SHOT_CONTRAST, SHOT_GAIN, SHOT_GAMMA, SHOT_OFFSET, SHOT_PIVOT};
        Self {
            gamma: Rgb::of(SHOT_GAMMA),
            gain: Rgb::of(SHOT_GAIN),
            offset: Rgb::of(SHOT_OFFSET),
            contrast: SHOT_CONTRAST.param(),
            pivot: SHOT_PIVOT.param(),
        }
    }
}

impl ShotMatch {
    fn check(&self) -> Result<(), OpError> {
        use bounds::{SHOT_CONTRAST, SHOT_GAIN, SHOT_GAMMA, SHOT_OFFSET, SHOT_PIVOT};
        self.gamma.check(
            [
                "shot_match.gamma.r",
                "shot_match.gamma.g",
                "shot_match.gamma.b",
            ],
            SHOT_GAMMA,
        )?;
        self.gain.check(
            [
                "shot_match.gain.r",
                "shot_match.gain.g",
                "shot_match.gain.b",
            ],
            SHOT_GAIN,
        )?;
        self.offset.check(
            [
                "shot_match.offset.r",
                "shot_match.offset.g",
                "shot_match.offset.b",
            ],
            SHOT_OFFSET,
        )?;
        SHOT_CONTRAST.check("shot_match.contrast", &self.contrast)?;
        SHOT_PIVOT.check("shot_match.pivot", &self.pivot)
    }
}

/// The Primaries palette: the wheels, which the Bars mode shows as bars (p. 3065), and the
/// settings beside them (pp. 3062–3064).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Primaries {
    /// Lift: mostly the dark part of the picture.
    pub lift: Wheel,
    /// Gamma: mostly the middle.
    pub gamma: Wheel,
    /// Gain: mostly the light part.
    pub gain: Wheel,
    /// Offset: the whole range, each channel moved alike (p. 3067).
    pub offset: Rgb,
    /// Contrast.
    pub contrast: Param,
    /// Pivot.
    pub pivot: Param,
    /// Saturation.
    pub saturation: Param,
    /// Hue.
    pub hue: Param,
    /// Lum Mix.
    pub lum_mix: Param,
    /// Temp.
    pub temperature: Param,
    /// Tint.
    pub tint: Param,
    /// Color Boost.
    pub color_boost: Param,
    /// Midtone Detail.
    pub midtone_detail: Param,
    /// Shadows.
    pub shadows: Param,
    /// Highlights.
    pub highlights: Param,
}

impl Default for Primaries {
    fn default() -> Self {
        use bounds::{
            COLOR_BOOST, CONTRAST, GAIN, GAMMA, HIGHLIGHTS, HUE, LIFT, LUM_MIX, MIDTONE_DETAIL,
            OFFSET, PIVOT, SATURATION, SHADOWS, TEMPERATURE, TINT,
        };
        Self {
            lift: Wheel::of(LIFT),
            gamma: Wheel::of(GAMMA),
            gain: Wheel::of(GAIN),
            offset: Rgb::of(OFFSET),
            contrast: CONTRAST.param(),
            pivot: PIVOT.param(),
            saturation: SATURATION.param(),
            hue: HUE.param(),
            lum_mix: LUM_MIX.param(),
            temperature: TEMPERATURE.param(),
            tint: TINT.param(),
            color_boost: COLOR_BOOST.param(),
            midtone_detail: MIDTONE_DETAIL.param(),
            shadows: SHADOWS.param(),
            highlights: HIGHLIGHTS.param(),
        }
    }
}

/// A wheel of Lift, Gamma or Gain: its Y, R, G and B (p. 3062). The master ring moves all four
/// together, the ball the colours.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Wheel {
    /// Y: the lightness only.
    pub y: Param,
    /// Red.
    pub r: Param,
    /// Green.
    pub g: Param,
    /// Blue.
    pub b: Param,
}

impl Wheel {
    fn of(bounds: Bounds) -> Self {
        Self {
            y: bounds.param(),
            r: bounds.param(),
            g: bounds.param(),
            b: bounds.param(),
        }
    }

    fn check(&self, names: [&'static str; 4], bounds: Bounds) -> Result<(), OpError> {
        bounds.check(names[0], &self.y)?;
        bounds.check(names[1], &self.r)?;
        bounds.check(names[2], &self.g)?;
        bounds.check(names[3], &self.b)
    }
}

/// A setting of each colour channel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rgb {
    /// Red.
    pub r: Param,
    /// Green.
    pub g: Param,
    /// Blue.
    pub b: Param,
}

impl Rgb {
    fn of(bounds: Bounds) -> Self {
        Self {
            r: bounds.param(),
            g: bounds.param(),
            b: bounds.param(),
        }
    }

    fn check(&self, names: [&'static str; 3], bounds: Bounds) -> Result<(), OpError> {
        bounds.check(names[0], &self.r)?;
        bounds.check(names[1], &self.g)?;
        bounds.check(names[2], &self.b)
    }
}

/// The Log mode of the Primaries palette: three wheels over ranges of lightness that do not
/// overlap, with the borders between them (pp. 3071–3073).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogWheels {
    /// Shadow.
    pub shadow: Rgb,
    /// Midtone.
    pub midtone: Rgb,
    /// Highlight.
    pub highlight: Rgb,
    /// Low Range.
    pub low_range: Param,
    /// High Range.
    pub high_range: Param,
}

impl Default for LogWheels {
    fn default() -> Self {
        use bounds::{HIGH_RANGE, LOG_WHEEL, LOW_RANGE};
        Self {
            shadow: Rgb::of(LOG_WHEEL),
            midtone: Rgb::of(LOG_WHEEL),
            highlight: Rgb::of(LOG_WHEEL),
            low_range: LOW_RANGE.param(),
            high_range: HIGH_RANGE.param(),
        }
    }
}

/// The custom curves of Y, R, G and B (pp. 3112–3116). Ganged, as they are at first, the four
/// curves are edited as one, and the curve works on R, G and B; not ganged, the Y curve changes
/// the lightness only (p. 3116).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Curves {
    /// Whether the curves are ganged.
    pub ganged: bool,
    /// Y.
    pub y: Curve,
    /// Red.
    pub r: Curve,
    /// Green.
    pub g: Curve,
    /// Blue.
    pub b: Curve,
}

impl Default for Curves {
    fn default() -> Self {
        Self {
            ganged: true,
            y: Curve::default(),
            r: Curve::default(),
            g: Curve::default(),
            b: Curve::default(),
        }
    }
}

/// A custom curve: its points, from the black point to the white point, each `[in, out]` from 0
/// to 1, in the order of `in`, two at least; and its intensity. At first it is the diagonal
/// from `[0, 0]` to `[1, 1]` (p. 3112).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Curve {
    /// Its points.
    pub points: Vec<[f64; 2]>,
    /// Its intensity.
    pub intensity: Param,
}

impl Default for Curve {
    fn default() -> Self {
        Self {
            points: vec![[0.0, 0.0], [1.0, 1.0]],
            intensity: bounds::CURVE_INTENSITY.param(),
        }
    }
}

/// The six HSL curves (pp. 3121–3126). Each is a list of points `[in, out]`, in the order of
/// `in`, and is flat, changing nothing, where it has none.
///
/// `in` is from 0 to 1: a hue, from red through green and blue back to red, for the curves of
/// hue, which go round, so that their left end meets their right end (p. 3122); the
/// lightness or the saturation for the others. `out` is from -1 to 1, and 0 changes nothing: a
/// turn of the hue of half a circle each way for Hue vs Hue, the saturation multiplied by
/// `1 + out` for the curves of saturation, and the lightness (luma) multiplied by `1 + out` for
/// those of lightness, Hue vs Lum as much as the colour is saturated: lighter or darker in a
/// share, as film lets light through (p. 3127), so that black stays black.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HslCurves {
    /// Hue vs Hue.
    pub hue_vs_hue: Vec<[f64; 2]>,
    /// Hue vs Sat.
    pub hue_vs_sat: Vec<[f64; 2]>,
    /// Hue vs Lum.
    pub hue_vs_lum: Vec<[f64; 2]>,
    /// Lum vs Sat.
    pub lum_vs_sat: Vec<[f64; 2]>,
    /// Sat vs Sat.
    pub sat_vs_sat: Vec<[f64; 2]>,
    /// Sat vs Lum.
    pub sat_vs_lum: Vec<[f64; 2]>,
}

/// Soft Clip, each channel apart: where it clips low and high, and how softly (pp. 3117–3121).
/// Ganging the channels, as an editor does at first, sets the three alike.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoftClip {
    /// Low.
    pub low: Rgb,
    /// High.
    pub high: Rgb,
    /// Low Soft.
    pub low_soft: Rgb,
    /// High Soft.
    pub high_soft: Rgb,
}

impl Default for SoftClip {
    fn default() -> Self {
        use bounds::{CLIP_HIGH, CLIP_LOW, CLIP_SOFT};
        Self {
            low: Rgb::of(CLIP_LOW),
            high: Rgb::of(CLIP_HIGH),
            low_soft: Rgb::of(CLIP_SOFT),
            high_soft: Rgb::of(CLIP_SOFT),
        }
    }
}

/// The Key palette of a corrector (pp. 3361–3362): the key coming in at its key input, the key of
/// its qualifier, and the key going out.
///
/// A node's key is made in this order: the key of its qualifier (if it has one) through
/// `qualifier`; the key coming in (if a key is linked into its key input) blurred by `input_blur`
/// and through `input`, and met with the qualifier's in Matte mode or added to it in Mask mode
/// (`input.matte`); and the one key, or a full key where it has neither, through `output`. That
/// key is what the node's adjustment is mixed by and what goes out of its key output.
/// `qualifier` and `input_blur` are always written, and read as they are at first where they are
/// left out.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyParams {
    /// The key coming in. With `invert` on, the node works outside the key of the node before
    /// it, as an Outside node does (p. 3361).
    pub input: KeyInput,
    /// The key going out. Its gain is how strongly the node works (p. 3362).
    pub output: KeyOutput,
    /// The key of the qualifier, as the Key palette's Qualifier controls set it (p. 3362):
    /// inverted, its gain and offset; and whether it meets the key of the node's windows (Matte
    /// mode, at first) or is added to it (Mask mode), which tells only once a node has windows.
    #[serde(default = "KeyParams::qualifier_at_first")]
    pub qualifier: KeyInput,
    /// The blur of the key coming in (Blur Radius and Blur H/V Ratio, p. 3362).
    #[serde(default)]
    pub input_blur: KeyBlur,
}

impl Default for KeyParams {
    fn default() -> Self {
        Self {
            input: KeyInput {
                matte: true,
                ..KeyInput::default()
            },
            output: KeyOutput::default(),
            qualifier: Self::qualifier_at_first(),
            input_blur: KeyBlur::default(),
        }
    }
}

impl KeyParams {
    fn qualifier_at_first() -> KeyInput {
        KeyInput {
            matte: true,
            ..KeyInput::default()
        }
    }
}

/// A blur of a key (p. 3362): its radius (as Matte Finesse's Blur Radius) and how it is shared
/// between across and down.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyBlur {
    /// Blur Radius.
    pub radius: Param,
    /// Blur H/V Ratio.
    pub ratio: Param,
}

impl Default for KeyBlur {
    fn default() -> Self {
        Self {
            radius: bounds::KEY_BLUR.param(),
            ratio: bounds::KEY_BLUR_RATIO.param(),
        }
    }
}

/// A key coming into a node: inverted or not, in Matte mode (meeting the other keys) or Mask
/// mode (added to them), and its gain and offset (pp. 3362–3363). A corrector's is in Matte mode
/// at first, and an input of a key mixer in Mask mode.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyInput {
    /// Whether it is inverted.
    pub invert: bool,
    /// Whether it is in Matte mode.
    pub matte: bool,
    /// Its gain.
    pub gain: Param,
    /// Its offset.
    pub offset: Param,
}

impl Default for KeyInput {
    fn default() -> Self {
        Self {
            invert: false,
            matte: false,
            gain: bounds::KEY_GAIN.param(),
            offset: bounds::KEY_OFFSET.param(),
        }
    }
}

impl KeyInput {
    fn check(&self) -> Result<(), OpError> {
        self.check_as("key.input.gain", "key.input.offset")
    }

    fn check_as(&self, gain: &'static str, offset: &'static str) -> Result<(), OpError> {
        bounds::KEY_GAIN.check(gain, &self.gain)?;
        bounds::KEY_OFFSET.check(offset, &self.offset)
    }
}

/// A key going out of a node: inverted or not, and its gain and offset (pp. 3362–3363).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyOutput {
    /// Whether it is inverted.
    pub invert: bool,
    /// Its gain.
    pub gain: Param,
    /// Its offset.
    pub offset: Param,
}

impl Default for KeyOutput {
    fn default() -> Self {
        Self {
            invert: false,
            gain: bounds::KEY_GAIN.param(),
            offset: bounds::KEY_OFFSET.param(),
        }
    }
}

impl KeyOutput {
    fn check(&self) -> Result<(), OpError> {
        bounds::KEY_GAIN.check("key.output.gain", &self.gain)?;
        bounds::KEY_OFFSET.check("key.output.offset", &self.offset)
    }
}

impl Corrector {
    /// Check that each setting is within its bounds, the curves are in order and the path of
    /// the LUT is one.
    pub(crate) fn check(&self) -> Result<(), OpError> {
        self.check_primaries()?;
        self.check_rest()
    }

    fn check_primaries(&self) -> Result<(), OpError> {
        use bounds::{
            COLOR_BOOST, CONTRAST, GAIN, GAMMA, HIGHLIGHTS, HUE, LIFT, LUM_MIX, MIDTONE_DETAIL,
            OFFSET, PIVOT, SATURATION, SHADOWS, TEMPERATURE, TINT,
        };
        let p = &self.primaries;
        p.lift.check(
            [
                "primaries.lift.y",
                "primaries.lift.r",
                "primaries.lift.g",
                "primaries.lift.b",
            ],
            LIFT,
        )?;
        p.gamma.check(
            [
                "primaries.gamma.y",
                "primaries.gamma.r",
                "primaries.gamma.g",
                "primaries.gamma.b",
            ],
            GAMMA,
        )?;
        p.gain.check(
            [
                "primaries.gain.y",
                "primaries.gain.r",
                "primaries.gain.g",
                "primaries.gain.b",
            ],
            GAIN,
        )?;
        p.offset.check(
            [
                "primaries.offset.r",
                "primaries.offset.g",
                "primaries.offset.b",
            ],
            OFFSET,
        )?;
        CONTRAST.check("primaries.contrast", &p.contrast)?;
        PIVOT.check("primaries.pivot", &p.pivot)?;
        SATURATION.check("primaries.saturation", &p.saturation)?;
        HUE.check("primaries.hue", &p.hue)?;
        LUM_MIX.check("primaries.lum_mix", &p.lum_mix)?;
        TEMPERATURE.check("primaries.temperature", &p.temperature)?;
        TINT.check("primaries.tint", &p.tint)?;
        COLOR_BOOST.check("primaries.color_boost", &p.color_boost)?;
        MIDTONE_DETAIL.check("primaries.midtone_detail", &p.midtone_detail)?;
        SHADOWS.check("primaries.shadows", &p.shadows)?;
        HIGHLIGHTS.check("primaries.highlights", &p.highlights)
    }

    fn check_rest(&self) -> Result<(), OpError> {
        use bounds::{CURVE_INTENSITY, HIGH_RANGE, LOG_WHEEL, LOW_RANGE};
        let log = &self.log;
        log.shadow
            .check(["log.shadow.r", "log.shadow.g", "log.shadow.b"], LOG_WHEEL)?;
        log.midtone.check(
            ["log.midtone.r", "log.midtone.g", "log.midtone.b"],
            LOG_WHEEL,
        )?;
        log.highlight.check(
            ["log.highlight.r", "log.highlight.g", "log.highlight.b"],
            LOG_WHEEL,
        )?;
        LOW_RANGE.check("log.low_range", &log.low_range)?;
        HIGH_RANGE.check("log.high_range", &log.high_range)?;
        let curves = &self.curves;
        for (curve, what, intensity) in [
            (&curves.y, "curves.y.points", "curves.y.intensity"),
            (&curves.r, "curves.r.points", "curves.r.intensity"),
            (&curves.g, "curves.g.points", "curves.g.intensity"),
            (&curves.b, "curves.b.points", "curves.b.intensity"),
        ] {
            if curve.points.len() < 2 || !in_order(&curve.points, 0.0, 1.0) {
                return Err(OpError::OutOfRange { what });
            }
            CURVE_INTENSITY.check(intensity, &curve.intensity)?;
        }
        let hsl = &self.hsl;
        for (points, what) in [
            (&hsl.hue_vs_hue, "hsl.hue_vs_hue"),
            (&hsl.hue_vs_sat, "hsl.hue_vs_sat"),
            (&hsl.hue_vs_lum, "hsl.hue_vs_lum"),
            (&hsl.lum_vs_sat, "hsl.lum_vs_sat"),
            (&hsl.sat_vs_sat, "hsl.sat_vs_sat"),
            (&hsl.sat_vs_lum, "hsl.sat_vs_lum"),
        ] {
            if !in_order(points, -1.0, 1.0) {
                return Err(OpError::OutOfRange { what });
            }
        }
        if let Some(path) = &self.lut {
            if path.is_empty() || path.chars().count() > LONGEST_PATH || path.contains('\0') {
                return Err(OpError::OutOfRange { what: "lut" });
            }
        }
        let clip = &self.soft_clip;
        clip.low.check(
            ["soft_clip.low.r", "soft_clip.low.g", "soft_clip.low.b"],
            bounds::CLIP_LOW,
        )?;
        clip.high.check(
            ["soft_clip.high.r", "soft_clip.high.g", "soft_clip.high.b"],
            bounds::CLIP_HIGH,
        )?;
        clip.low_soft.check(
            [
                "soft_clip.low_soft.r",
                "soft_clip.low_soft.g",
                "soft_clip.low_soft.b",
            ],
            bounds::CLIP_SOFT,
        )?;
        clip.high_soft.check(
            [
                "soft_clip.high_soft.r",
                "soft_clip.high_soft.g",
                "soft_clip.high_soft.b",
            ],
            bounds::CLIP_SOFT,
        )?;
        self.key.input.check()?;
        self.key.output.check()?;
        self.key
            .qualifier
            .check_as("key.qualifier.gain", "key.qualifier.offset")?;
        bounds::KEY_BLUR.check("key.input_blur.radius", &self.key.input_blur.radius)?;
        bounds::KEY_BLUR_RATIO.check("key.input_blur.ratio", &self.key.input_blur.ratio)?;
        self.shot_match.as_ref().map_or(Ok(()), ShotMatch::check)?;
        self.blur.check()?;
        self.qualifier.as_ref().map_or(Ok(()), Qualifier::check)?;
        window::check_windows(&self.windows)
    }
}

/// Whether the points of a curve have their `in` from 0 to 1, rising, and their `out` from
/// `low` to `high`.
fn in_order(points: &[[f64; 2]], low: f64, high: f64) -> bool {
    points
        .iter()
        .all(|[x, y]| (0.0..=1.0).contains(x) && (low..=high).contains(y))
        && points.windows(2).all(|pair| pair[0][0] < pair[1][0])
}

/// The settings of a parallel mixer: how many inputs it has (pp. 3334–3335).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParallelMixer {
    /// How many RGB inputs it has: from 1 to [`MOST_INPUTS`], 2 at first.
    pub inputs: u32,
}

impl Default for ParallelMixer {
    fn default() -> Self {
        Self { inputs: 2 }
    }
}

/// The settings of a layer mixer: how many inputs it has, and how they are laid over one
/// another (pp. 3336–3338).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerMixer {
    /// How many RGB inputs it has: from 1 to [`MOST_INPUTS`], 2 at first.
    pub inputs: u32,
    /// How each input is laid over those above it.
    pub composite: Composite,
}

impl Default for LayerMixer {
    fn default() -> Self {
        Self {
            inputs: 2,
            composite: Composite::Normal,
        }
    }
}

/// The settings of a key mixer: how each input is mixed in, and the key that goes out
/// (pp. 3358–3363). It has two inputs at first (p. 3360).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyMixer {
    /// Its key inputs, one for each: from 1 to [`MOST_INPUTS`].
    pub inputs: Vec<KeyInput>,
    /// The key that goes out.
    pub output: KeyOutput,
}

impl Default for KeyMixer {
    fn default() -> Self {
        Self {
            inputs: vec![KeyInput::default(), KeyInput::default()],
            output: KeyOutput::default(),
        }
    }
}

/// How a layer mixer lays its inputs over one another: the composite modes of the timeline
/// (pp. 1055–1058), which the Layer Mixer uses too (p. 3337).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs, reason = "each is named as DaVinci names it")]
pub enum Composite {
    Normal,
    Add,
    Color,
    ColorBurn,
    ColorDodge,
    Darken,
    DarkerColor,
    Difference,
    Divide,
    Exclusion,
    HardMix,
    Hardlight,
    Hue,
    Lighten,
    LighterColor,
    LinearBurn,
    LinearDodge,
    LinearLight,
    Luminosity,
    Multiply,
    Overlay,
    PinLight,
    Saturation,
    Screen,
    Softlight,
    Subtract,
    VividLight,
}

// ---------------------------------------------------------------------------------------------
// Links and grades
// ---------------------------------------------------------------------------------------------

/// What a link carries: the picture, or a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Port {
    /// The picture: the green connections of an editor.
    Rgb,
    /// A key: the blue ones.
    Key,
}

impl Port {
    /// Its name, as it is written.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Rgb => "rgb",
            Self::Key => "key",
        }
    }
}

/// A link of a grade: from the output of `kind` of `from`, which is a node or [`SOURCE`], into
/// input `input` of `kind` of `to`, which is a node or [`OUTPUT`].
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Link {
    /// Where it comes from.
    pub from: Id,
    /// Where it goes.
    pub to: Id,
    /// What it carries.
    pub kind: Port,
    /// The input of `to` that it goes into, from 0.
    pub input: u32,
}

impl Link {
    /// A link of the picture from `from` into the first input of `to`.
    #[must_use]
    pub fn rgb(from: impl Into<Id>, to: impl Into<Id>) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            kind: Port::Rgb,
            input: 0,
        }
    }

    /// The input that it goes into, by which it is told from the other links of its grade.
    #[must_use]
    pub fn target(&self) -> (&str, Port, u32) {
        (&self.to, self.kind, self.input)
    }

    /// Whether it comes from or goes to `node`.
    #[must_use]
    pub fn touches(&self, node: &str) -> bool {
        self.from == node || self.to == node
    }
}

/// A grade: its nodes, in the order in which they were added, and its links, in the order of
/// the inputs that they go into (by `to`, `kind` and `input`).
///
/// At first it has no nodes, and the source is linked to the output: it changes nothing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grade {
    /// Its nodes.
    pub nodes: Vec<Node>,
    /// Its links.
    pub links: Vec<Link>,
}

impl Default for Grade {
    fn default() -> Self {
        Self {
            nodes: vec![],
            links: vec![Link::rgb(SOURCE, OUTPUT)],
        }
    }
}

impl Grade {
    /// Whether it changes nothing, as a new grade does: it has no node.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The node `id`, if it has one.
    #[must_use]
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Where the node `id` is in its list of nodes.
    #[must_use]
    pub fn position(&self, id: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.id == id)
    }

    /// The link into input `input` of `kind` of `to`, if there is one.
    #[must_use]
    pub fn link_into(&self, to: &str, kind: Port, input: u32) -> Option<&Link> {
        self.links.iter().find(|l| l.target() == (to, kind, input))
    }

    /// The links out of the output of `kind` of `from`.
    pub fn links_from<'a>(&'a self, from: &'a str, kind: Port) -> impl Iterator<Item = &'a Link> {
        self.links
            .iter()
            .filter(move |l| l.from == from && l.kind == kind)
    }

    /// The grade with each key of each setting of its nodes moved from frame `k` to `to(k)`,
    /// which is not to go back as `k` goes on, clamped within [`LARGEST`] of 0. Keys that come
    /// to one frame are one key there, the last of them: from that frame on the setting goes as
    /// it went after them all.
    ///
    /// # Panics
    /// Never: settings whose keys are at other frames are settings of the same kind.
    #[must_use]
    pub fn with_key_frames(&self, to: impl Fn(i64) -> i64) -> Grade {
        let mut grade = self.clone();
        for node in &mut grade.nodes {
            let mut params = node.kind.params();
            if move_keys(&mut params, &to) {
                node.kind = NodeKind::from_params(node.kind.kind(), params)
                    .expect("keys at other frames are settings of the same kind");
            }
        }
        grade
    }

    /// Whether a setting of one of its nodes has keys.
    #[must_use]
    pub fn has_keys(&self) -> bool {
        fn keyed(value: &serde_json::Value) -> bool {
            match value {
                serde_json::Value::Object(map) => keys_of(map).is_some() || map.values().any(keyed),
                serde_json::Value::Array(items) => items.iter().any(keyed),
                _ => false,
            }
        }
        self.nodes.iter().any(|node| keyed(&node.kind.params()))
    }

    /// Put its links in the order of the inputs that they go into.
    pub fn sort_links(&mut self) {
        self.links.sort_by(|a, b| a.target().cmp(&b.target()));
    }

    /// How many inputs of `kind` the node or end `id` has, if it is one of the grade.
    #[must_use]
    pub fn inputs_of(&self, id: &str, kind: Port) -> Option<u32> {
        if id == OUTPUT {
            return Some(u32::from(kind == Port::Rgb));
        }
        if id == SOURCE {
            return Some(0);
        }
        self.node(id).map(|n| n.kind.inputs(kind))
    }

    /// Whether the node or end `id` has an output of `kind`, if it is one of the grade.
    #[must_use]
    pub fn has_output(&self, id: &str, kind: Port) -> Option<bool> {
        if id == SOURCE {
            return Some(kind == Port::Rgb);
        }
        if id == OUTPUT {
            return Some(false);
        }
        self.node(id).map(|n| n.kind.has_output(kind))
    }

    /// Whether it colours the picture: the output takes a link, and every node takes a link
    /// into each of its inputs of the picture (each of its key inputs, for a key mixer) and
    /// gives its picture (its key, for a key mixer) to another. Otherwise `DaVinci` disables it
    /// (pp. 3312, 3319), and so does the engine: the picture passes through it unchanged.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        if self.link_into(OUTPUT, Port::Rgb, 0).is_none() {
            return false;
        }
        self.nodes.iter().all(|node| {
            let main = match node.kind {
                NodeKind::KeyMixer(_) => Port::Key,
                _ => Port::Rgb,
            };
            (0..node.kind.inputs(main)).all(|input| self.link_into(&node.id, main, input).is_some())
                && self.links_from(&node.id, main).next().is_some()
        })
    }

    /// Check the grade: the ids of its nodes, their settings and its links. The links are to be
    /// in order.
    pub(crate) fn check(&self) -> Result<(), OpError> {
        let mut ids = HashSet::new();
        for node in &self.nodes {
            check_node_id(&node.id)?;
            if !ids.insert(node.id.as_str()) {
                return Err(OpError::Taken {
                    what: "node",
                    id: node.id.clone(),
                });
            }
            check_node(node)?;
        }
        self.check_links()
    }

    /// Check its links: each goes from an output of its kind into an input of its kind that is
    /// there, no two go into one input, and they make no loop. The links are to be in order.
    pub(crate) fn check_links(&self) -> Result<(), OpError> {
        for link in &self.links {
            match self.has_output(&link.from, link.kind) {
                None => return Err(missing_node(&link.from)),
                Some(false) => {
                    return Err(OpError::NoPort {
                        node: link.from.clone(),
                        kind: link.kind,
                        input: None,
                    })
                }
                Some(true) => {}
            }
            match self.inputs_of(&link.to, link.kind) {
                None => return Err(missing_node(&link.to)),
                Some(count) if link.input >= count => {
                    return Err(OpError::NoPort {
                        node: link.to.clone(),
                        kind: link.kind,
                        input: Some(link.input),
                    })
                }
                Some(_) => {}
            }
        }
        if let Some(pair) = self
            .links
            .windows(2)
            .find(|p| p[0].target() == p[1].target())
        {
            return Err(OpError::InputTaken {
                node: pair[1].to.clone(),
                kind: pair[1].kind,
                input: pair[1].input,
            });
        }
        if let Some(node) = self.in_loop() {
            return Err(OpError::Loop { node });
        }
        Ok(())
    }

    /// A node that its links go round in a loop through, if there is one.
    fn in_loop(&self) -> Option<Id> {
        let mut after: HashMap<&str, Vec<&str>> = HashMap::new();
        for link in &self.links {
            after.entry(&link.from).or_default().push(&link.to);
        }
        // 0: not seen; 1: on the path; 2: done.
        let mut state: HashMap<&str, u8> = HashMap::new();
        for start in self.nodes.iter().map(|n| n.id.as_str()) {
            if state.contains_key(start) {
                continue;
            }
            let mut stack = vec![(start, 0_usize)];
            state.insert(start, 1);
            while let Some((node, next)) = stack.pop() {
                let outs = after.get(node).map_or(&[][..], Vec::as_slice);
                if let Some(&to) = outs.get(next) {
                    stack.push((node, next + 1));
                    match state.get(to) {
                        Some(1) => return Some(to.to_owned()),
                        Some(_) => {}
                        None => {
                            state.insert(to, 1);
                            stack.push((to, 0));
                        }
                    }
                } else {
                    state.insert(node, 2);
                }
            }
        }
        None
    }

    /// The links that take the place of node `id` when it is taken away, as `DaVinci` links
    /// the nodes on either side of a node that is deleted (p. 3318): for the picture and for the
    /// key, what comes into its first linked input of that kind goes into every input that it
    /// gave that kind to.
    #[must_use]
    pub fn bridge(&self, id: &str) -> Vec<Link> {
        let mut links = vec![];
        for kind in [Port::Rgb, Port::Key] {
            let count = self.inputs_of(id, kind).unwrap_or(0);
            let Some(before) = (0..count).find_map(|input| self.link_into(id, kind, input)) else {
                continue;
            };
            for after in self.links_from(id, kind) {
                links.push(Link {
                    from: before.from.clone(),
                    to: after.to.clone(),
                    kind,
                    input: after.input,
                });
            }
        }
        links
    }
}

/// Check the id of a node: it has one, it is neither of the ends, and it does not start with
/// `$`.
pub(crate) fn check_node_id(id: &str) -> Result<(), OpError> {
    if id.is_empty() {
        Err(OpError::NoId)
    } else if id == SOURCE || id == OUTPUT {
        Err(OpError::Taken {
            what: "node",
            id: id.into(),
        })
    } else if id.starts_with('$') {
        Err(OpError::Dollar { id: id.into() })
    } else {
        Ok(())
    }
}

/// Check a node but for its id: its label, where it lies and its settings.
pub(crate) fn check_node(node: &Node) -> Result<(), OpError> {
    check_label(&node.label)?;
    if let Some(pos) = node.pos {
        pos.check()?;
    }
    node.kind.check()
}

/// Check the label of a node or the name of a group.
pub(crate) fn check_label(label: &str) -> Result<(), OpError> {
    if label.chars().count() > LONGEST_LABEL {
        Err(OpError::OutOfRange { what: "label" })
    } else {
        Ok(())
    }
}

fn missing_node(id: &str) -> OpError {
    OpError::Missing {
        what: "node",
        id: id.into(),
    }
}

// ---------------------------------------------------------------------------------------------
// Where grades are
// ---------------------------------------------------------------------------------------------

/// One of the grades of a project, by its level (p. 3328).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "level", rename_all = "snake_case", deny_unknown_fields)]
pub enum Target {
    /// The grade of a clip.
    Clip {
        /// The timeline of the clip.
        timeline: Id,
        /// The clip.
        clip: Id,
    },
    /// The grade of a group that comes before the grades of its clips.
    PreClip {
        /// The group.
        group: Id,
    },
    /// The grade of a group that comes after the grades of its clips.
    PostClip {
        /// The group.
        group: Id,
    },
    /// The grade of a timeline, after its tracks are put together.
    Timeline {
        /// The timeline.
        timeline: Id,
    },
}

/// A group of clips that are graded together, as `DaVinci` groups them (pp. 3301–3306): its
/// pre-clip and post-clip grades work on every clip in it, before and after the clip's own. A
/// clip is in at most one group, which it names in its `group`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    /// What it is known by: `k` and a number where the engine made it.
    pub id: Id,
    /// The name that it is shown by: at most [`LONGEST_LABEL`] characters.
    pub name: String,
    /// Its pre-clip grade.
    pub pre: Grade,
    /// Its post-clip grade.
    pub post: Grade,
}

impl Group {
    /// A group named `name` whose grades change nothing.
    #[must_use]
    pub fn new(id: impl Into<Id>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            pre: Grade::default(),
            post: Grade::default(),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Changes to the settings of a node
// ---------------------------------------------------------------------------------------------

/// Put `patch` into `base`: where both are objects, each field of the patch into the field of
/// that name, and otherwise the patch in place of the base, whole. A `null` is a value like any
/// other.
pub(crate) fn merge(base: &mut serde_json::Value, patch: &serde_json::Value) {
    match (base, patch) {
        (serde_json::Value::Object(base), serde_json::Value::Object(patch)) => {
            for (name, value) in patch {
                match base.get_mut(name) {
                    Some(field) => merge(field, value),
                    None => {
                        base.insert(name.clone(), value.clone());
                    }
                }
            }
        }
        (base, patch) => *base = patch.clone(),
    }
}

/// What `base` has where `patch` would change it: a patch that, merged into what `patch` makes
/// of `base`, gives `base` back.
pub(crate) fn changed(base: &serde_json::Value, patch: &serde_json::Value) -> serde_json::Value {
    match (base, patch) {
        (serde_json::Value::Object(base), serde_json::Value::Object(patch)) => patch
            .iter()
            .map(|(name, value)| {
                let was = base
                    .get(name)
                    .map_or(serde_json::Value::Null, |field| changed(field, value));
                (name.clone(), was)
            })
            .collect::<serde_json::Map<_, _>>()
            .into(),
        (base, _) => base.clone(),
    }
}

/// The keys of a setting as it is written, `{"keys": [...]}`, if the object is one.
fn keys_of(map: &serde_json::Map<String, serde_json::Value>) -> Option<&Vec<serde_json::Value>> {
    match map.get("keys") {
        Some(serde_json::Value::Array(keys)) if map.len() == 1 => Some(keys),
        _ => None,
    }
}

/// Move the keys of every setting in `value`, settings as they are written, from frame `k` to
/// `to(k)` within [`LARGEST`] of 0, keeping the last of those that come to one frame (see
/// [`Grade::with_key_frames`]). Gives whether there were any.
fn move_keys(value: &mut serde_json::Value, to: &impl Fn(i64) -> i64) -> bool {
    match value {
        serde_json::Value::Object(map) => {
            if keys_of(map).is_some() {
                let keys: Vec<Key> = serde_json::from_value(map["keys"].clone())
                    .expect("the keys of a setting are written as keys");
                let mut moved: Vec<Key> = Vec::with_capacity(keys.len());
                for key in keys {
                    let frame = to(key.frame).clamp(-LARGEST, LARGEST);
                    let key = Key { frame, ..key };
                    match moved.last_mut() {
                        Some(last) if last.frame >= frame => *last = key,
                        _ => moved.push(key),
                    }
                }
                map.insert(
                    "keys".into(),
                    serde_json::to_value(moved).expect("keys are written as JSON"),
                );
                return true;
            }
            let mut any = false;
            for item in map.values_mut() {
                any |= move_keys(item, to);
            }
            any
        }
        serde_json::Value::Array(items) => {
            let mut any = false;
            for item in items {
                any |= move_keys(item, to);
            }
            any
        }
        _ => false,
    }
}

/// The value at a path of fields, written with dots (`primaries.lift.y`), if there is one.
#[must_use]
pub fn at_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    path.split('.')
        .try_fold(value, |value, name| value.get(name))
}

/// A patch that sets the value at a path of fields, written with dots, to `value`.
#[must_use]
pub fn patch_at(path: &str, value: serde_json::Value) -> serde_json::Value {
    path.rsplit('.').fold(value, |value, name| {
        let mut object = serde_json::Map::new();
        object.insert(name.into(), value);
        object.into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(frame: i64, value: f64, interp: Interp) -> Key {
        Key {
            frame,
            value,
            interp,
        }
    }

    #[test]
    fn a_fixed_value_is_the_same_on_every_frame() {
        let param = Param::Fixed(1.5);
        for frame in [-10.0, 0.0, 3.5, 1e9] {
            assert_eq!(param.value_at(frame), 1.5);
        }
        assert!(!param.is_animated());
    }

    #[test]
    fn a_linear_key_goes_on_a_straight_line_to_the_next() {
        let param = Param::Keys(vec![
            key(10, 0.0, Interp::Linear),
            key(20, 1.0, Interp::Linear),
        ]);
        assert_eq!(param.value_at(10.0), 0.0);
        assert_eq!(param.value_at(15.0), 0.5);
        assert_eq!(param.value_at(12.5), 0.25);
        assert_eq!(param.value_at(20.0), 1.0);
        assert!(param.is_animated());
    }

    #[test]
    fn before_the_first_key_and_after_the_last_the_value_is_theirs() {
        let param = Param::Keys(vec![
            key(10, 2.0, Interp::Linear),
            key(20, 4.0, Interp::Linear),
        ]);
        assert_eq!(param.value_at(-5.0), 2.0);
        assert_eq!(param.value_at(9.999), 2.0);
        assert_eq!(param.value_at(20.001), 4.0);
        assert_eq!(param.value_at(1e12), 4.0);
        let one = Param::Keys(vec![key(3, 7.0, Interp::Hold)]);
        for frame in [0.0, 3.0, 100.0] {
            assert_eq!(one.value_at(frame), 7.0);
        }
    }

    #[test]
    fn a_hold_key_keeps_its_value_until_the_next_key() {
        let param = Param::Keys(vec![
            key(0, 1.0, Interp::Hold),
            key(10, 5.0, Interp::Linear),
            key(20, 9.0, Interp::Linear),
        ]);
        assert_eq!(param.value_at(0.0), 1.0);
        assert_eq!(param.value_at(9.99), 1.0);
        assert_eq!(param.value_at(10.0), 5.0);
        assert_eq!(param.value_at(15.0), 7.0);
    }

    #[test]
    fn a_bezier_key_with_flat_handles_eases_out_and_in() {
        let ease = Interp::Bezier {
            out: [0.42, 0.0],
            into: [0.42, 0.0],
        };
        let param = Param::Keys(vec![key(0, 0.0, ease), key(100, 1.0, Interp::Linear)]);
        assert_eq!(param.value_at(0.0), 0.0);
        assert_eq!(param.value_at(100.0), 1.0);
        // Symmetric handles cross the middle at the middle.
        assert!((param.value_at(50.0) - 0.5).abs() < 1e-9);
        // Slow near the keys: below the straight line at first and above it at the end.
        assert!(param.value_at(10.0) < 0.1);
        assert!(param.value_at(90.0) > 0.9);
        // And it never goes back.
        let mut last = 0.0;
        for frame in 0..=100 {
            let value = param.value_at(f64::from(frame));
            assert!(value >= last - 1e-12, "{frame}: {value} < {last}");
            last = value;
        }
    }

    #[test]
    fn a_bezier_key_with_handles_on_the_line_goes_straight() {
        let straight = Interp::Bezier {
            out: [1.0 / 3.0, 1.0 / 3.0],
            into: [1.0 / 3.0, -1.0 / 3.0],
        };
        let param = Param::Keys(vec![key(0, 0.0, straight), key(30, 1.0, Interp::Hold)]);
        for frame in 0..=30 {
            let value = param.value_at(f64::from(frame));
            assert!(
                (value - f64::from(frame) / 30.0).abs() < 1e-9,
                "{frame}: {value}"
            );
        }
    }

    #[test]
    fn a_bezier_handle_may_overshoot_the_keys() {
        let overshoot = Interp::Bezier {
            out: [0.3, 0.0],
            into: [0.3, 0.5],
        };
        let param = Param::Keys(vec![key(0, 0.0, overshoot), key(10, 1.0, Interp::Linear)]);
        let highest = (0..=100)
            .map(|k| param.value_at(f64::from(k) / 10.0))
            .fold(f64::MIN, f64::max);
        assert!(highest > 1.0, "{highest}");
    }

    #[test]
    fn a_param_is_written_as_a_number_or_as_keys() {
        let fixed = Param::Fixed(0.5);
        assert_eq!(
            serde_json::to_value(&fixed).unwrap(),
            serde_json::json!(0.5)
        );
        assert_eq!(
            serde_json::from_value::<Param>(serde_json::json!(2)).unwrap(),
            Param::Fixed(2.0)
        );
        let keys = Param::Keys(vec![
            key(0, 1.0, Interp::Hold),
            key(
                5,
                2.0,
                Interp::Bezier {
                    out: [0.25, 0.0],
                    into: [0.5, 0.1],
                },
            ),
        ]);
        let written = serde_json::to_value(&keys).unwrap();
        assert_eq!(
            written,
            serde_json::json!({"keys": [
                {"frame": 0, "value": 1.0, "interp": "hold"},
                {"frame": 5, "value": 2.0, "interp": {"bezier": {"out": [0.25, 0.0], "in": [0.5, 0.1]}}},
            ]})
        );
        assert_eq!(serde_json::from_value::<Param>(written).unwrap(), keys);
        for wrong in [
            serde_json::json!("1"),
            serde_json::json!(null),
            serde_json::json!({"keys": [], "more": 1}),
            serde_json::json!({"keys": [{"frame": 0, "value": 1, "interp": "cubic"}]}),
        ] {
            assert!(
                serde_json::from_value::<Param>(wrong.clone()).is_err(),
                "{wrong}"
            );
        }
    }

    #[test]
    fn a_param_out_of_its_bounds_or_with_keys_out_of_order_is_refused() {
        let ok = |p: &Param| p.check("x", 0.0, 1.0).is_ok();
        assert!(ok(&Param::Fixed(0.0)));
        assert!(ok(&Param::Fixed(1.0)));
        assert!(!ok(&Param::Fixed(1.01)));
        assert!(!ok(&Param::Fixed(f64::NAN)));
        assert!(!ok(&Param::Keys(vec![])));
        let two = |a: i64, b: i64| {
            Param::Keys(vec![
                key(a, 0.0, Interp::Linear),
                key(b, 1.0, Interp::Linear),
            ])
        };
        assert!(ok(&two(0, 1)));
        assert!(!ok(&two(1, 1)));
        assert!(!ok(&two(2, 1)));
        assert!(!ok(&two(0, LARGEST + 1)));
        assert!(!ok(&Param::Keys(vec![key(0, 2.0, Interp::Hold)])));
        let handle = |out: [f64; 2]| {
            Param::Keys(vec![key(
                0,
                0.0,
                Interp::Bezier {
                    out,
                    into: [0.0, 0.0],
                },
            )])
        };
        assert!(ok(&handle([1.0, 5.0])));
        assert!(!ok(&handle([1.5, 0.0])));
        assert!(!ok(&handle([-0.1, 0.0])));
        assert!(!ok(&handle([0.5, f64::INFINITY])));
        let error = Param::Fixed(2.0)
            .check("primaries.saturation", 0.0, 1.0)
            .unwrap_err();
        assert_eq!(
            error,
            OpError::OutOfRange {
                what: "primaries.saturation"
            }
        );
    }

    #[test]
    fn a_new_corrector_is_set_as_davinci_sets_one() {
        let corrector = Corrector::default();
        let p = &corrector.primaries;
        assert_eq!(p.lift.y, Param::Fixed(0.0));
        assert_eq!(p.gain.r, Param::Fixed(1.0));
        assert_eq!(p.offset.g, Param::Fixed(25.0));
        assert_eq!(p.saturation, Param::Fixed(50.0));
        assert_eq!(p.hue, Param::Fixed(50.0));
        assert_eq!(p.lum_mix, Param::Fixed(100.0));
        assert_eq!(p.temperature, Param::Fixed(0.0));
        assert_eq!(corrector.curves.y.points, vec![[0.0, 0.0], [1.0, 1.0]]);
        assert_eq!(corrector.curves.b.intensity, Param::Fixed(100.0));
        assert!(corrector.curves.ganged);
        assert_eq!(corrector.soft_clip.high.r, Param::Fixed(100.0));
        assert!(corrector.key.input.matte);
        assert!(!KeyMixer::default().inputs[0].matte);
        assert_eq!(corrector.key.output.gain, Param::Fixed(1.0));
        assert_eq!(corrector.check(), Ok(()));
    }

    #[test]
    fn a_node_is_read_unlocked_and_laid_out_where_it_leaves_them_out() {
        // As the engine wrote nodes before Lock Node and the layout were kept.
        let old = serde_json::json!({
            "id": "n1", "kind": "parallel_mixer", "label": "", "enabled": true,
            "params": {"inputs": 2},
        });
        let node: Node = serde_json::from_value(old).unwrap();
        assert!(!node.locked);
        assert_eq!(node.pos, None);
        assert_eq!(node, Node::new("n1", Kind::ParallelMixer));
        let placed = serde_json::json!({
            "id": "n1", "kind": "parallel_mixer", "label": "", "enabled": true,
            "locked": true, "pos": {"x": -12.5, "y": 300},
            "params": {"inputs": 2},
        });
        let node: Node = serde_json::from_value(placed.clone()).unwrap();
        assert!(node.locked);
        assert_eq!(node.pos, Some(Pos { x: -12.5, y: 300.0 }));
        assert_eq!(check_node(&node), Ok(()));
        let mut written = serde_json::to_value(&node).unwrap();
        assert_eq!(written["pos"], serde_json::json!({"x": -12.5, "y": 300.0}));
        written["pos"]["z"] = 1.into();
        assert!(serde_json::from_value::<Node>(written).is_err());
        let mut half = placed;
        half["pos"] = serde_json::json!({"x": 1});
        assert!(serde_json::from_value::<Node>(half).is_err());
    }

    #[test]
    fn a_node_lies_within_a_million_of_0() {
        let mut node = Node::corrector("n1");
        for (x, y, fits) in [
            (FURTHEST_POS, -FURTHEST_POS, true),
            (0.0, 0.0, true),
            (FURTHEST_POS * 1.5, 0.0, false),
            (0.0, -FURTHEST_POS - 1.0, false),
            (f64::NAN, 0.0, false),
            (0.0, f64::INFINITY, false),
        ] {
            node.pos = Some(Pos { x, y });
            let checked = check_node(&node);
            if fits {
                assert_eq!(checked, Ok(()));
            } else {
                assert_eq!(checked, Err(OpError::OutOfRange { what: "pos" }));
            }
        }
    }

    #[test]
    fn a_node_is_written_with_the_params_of_its_kind() {
        let node = Node::new("n1", Kind::LayerMixer);
        let written = serde_json::to_value(&node).unwrap();
        assert_eq!(
            written,
            serde_json::json!({
                "id": "n1", "kind": "layer_mixer", "label": "", "enabled": true,
                "locked": false, "pos": null,
                "params": {"inputs": 2, "composite": "normal"},
            })
        );
        assert_eq!(serde_json::from_value::<Node>(written).unwrap(), node);
        let corrector = Node::corrector("n2");
        let written = serde_json::to_value(&corrector).unwrap();
        assert_eq!(
            written["params"]["primaries"]["gain"]["y"],
            serde_json::json!(1.0)
        );
        assert_eq!(written["params"]["lut"], serde_json::Value::Null);
        assert_eq!(serde_json::from_value::<Node>(written).unwrap(), corrector);
        let wrong = serde_json::json!({
            "id": "n1", "kind": "parallel_mixer", "label": "", "enabled": true,
            "params": {"inputs": 2, "composite": "normal"},
        });
        let error = serde_json::from_value::<Node>(wrong)
            .unwrap_err()
            .to_string();
        assert!(error.contains("composite"), "{error}");
    }

    #[test]
    fn a_new_grade_links_the_source_to_the_output() {
        let grade = Grade::default();
        assert!(grade.is_empty());
        assert!(grade.is_complete());
        assert_eq!(
            serde_json::to_value(&grade).unwrap(),
            serde_json::json!({"nodes": [], "links": [{"from": "source", "to": "output", "kind": "rgb", "input": 0}]})
        );
        assert_eq!(grade.check(), Ok(()));
    }

    fn serial() -> Grade {
        let mut grade = Grade {
            nodes: vec![Node::corrector("n1"), Node::corrector("n2")],
            links: vec![
                Link::rgb(SOURCE, "n1"),
                Link::rgb("n1", "n2"),
                Link::rgb("n2", OUTPUT),
            ],
        };
        grade.sort_links();
        grade
    }

    #[test]
    fn a_grade_with_a_node_left_out_is_kept_but_does_not_colour() {
        let mut grade = serial();
        assert!(grade.is_complete());
        grade.nodes.push(Node::corrector("n3"));
        assert_eq!(grade.check(), Ok(()));
        assert!(!grade.is_complete());
        let mut grade = serial();
        grade.links.retain(|l| l.to != OUTPUT);
        assert!(!grade.is_complete());
    }

    #[test]
    fn links_that_go_round_or_into_what_is_not_there_are_refused() {
        let mut grade = serial();
        grade.links.push(Link::rgb("n2", "n1"));
        grade.links.retain(|l| l.from != SOURCE);
        grade.sort_links();
        assert!(matches!(grade.check(), Err(OpError::Loop { .. })));

        let mut grade = serial();
        grade.links.push(Link {
            from: "n1".into(),
            to: "n2".into(),
            kind: Port::Rgb,
            input: 1,
        });
        grade.sort_links();
        assert_eq!(
            grade.check(),
            Err(OpError::NoPort {
                node: "n2".into(),
                kind: Port::Rgb,
                input: Some(1)
            })
        );

        let mut grade = serial();
        grade.links.push(Link::rgb(SOURCE, "n2"));
        grade.sort_links();
        assert_eq!(
            grade.check(),
            Err(OpError::InputTaken {
                node: "n2".into(),
                kind: Port::Rgb,
                input: 0
            })
        );

        let mut grade = serial();
        grade.links.push(Link {
            from: "n9".into(),
            to: "n1".into(),
            kind: Port::Key,
            input: 0,
        });
        grade.sort_links();
        assert!(matches!(
            grade.check(),
            Err(OpError::Missing { what: "node", .. })
        ));

        let mut grade = serial();
        grade.links.push(Link {
            from: SOURCE.into(),
            to: "n1".into(),
            kind: Port::Key,
            input: 0,
        });
        grade.sort_links();
        assert_eq!(
            grade.check(),
            Err(OpError::NoPort {
                node: SOURCE.into(),
                kind: Port::Key,
                input: None
            })
        );

        let mut grade = serial();
        grade.nodes[1].id = "output".into();
        assert!(matches!(grade.check(), Err(OpError::Taken { .. })));
    }

    #[test]
    fn a_node_taken_away_is_bridged_by_what_came_into_it() {
        let grade = serial();
        assert_eq!(grade.bridge("n1"), vec![Link::rgb(SOURCE, "n2")]);
        let mut fan = serial();
        fan.links.push(Link {
            from: "n1".into(),
            to: "n2".into(),
            kind: Port::Key,
            input: 0,
        });
        fan.nodes.push(Node::new("n3", Kind::ParallelMixer));
        fan.links.push(Link {
            from: "n1".into(),
            to: "n3".into(),
            kind: Port::Rgb,
            input: 1,
        });
        fan.sort_links();
        assert_eq!(
            fan.bridge("n1"),
            vec![
                Link::rgb(SOURCE, "n2"),
                Link {
                    from: SOURCE.into(),
                    to: "n3".into(),
                    kind: Port::Rgb,
                    input: 1
                },
            ]
        );
        // A node that nothing comes into is bridged by nothing.
        let mut lone = serial();
        lone.links.retain(|l| l.from != SOURCE);
        assert_eq!(lone.bridge("n1"), vec![]);
    }

    #[test]
    fn a_patch_merges_into_objects_and_replaces_the_rest() {
        let mut base =
            serde_json::json!({"a": {"b": 1, "c": [1, 2]}, "d": {"keys": []}, "e": null});
        let patch = serde_json::json!({"a": {"c": [3]}, "d": 5, "e": "x"});
        let before = base.clone();
        let back = changed(&base, &patch);
        merge(&mut base, &patch);
        assert_eq!(
            base,
            serde_json::json!({"a": {"b": 1, "c": [3]}, "d": 5, "e": "x"})
        );
        assert_eq!(
            back,
            serde_json::json!({"a": {"c": [1, 2]}, "d": {"keys": []}, "e": null})
        );
        merge(&mut base, &back);
        assert_eq!(base, before);
        assert_eq!(at_path(&before, "a.b"), Some(&serde_json::json!(1)));
        assert_eq!(at_path(&before, "a.x"), None);
        assert_eq!(
            patch_at("a.b", serde_json::json!(2)),
            serde_json::json!({"a": {"b": 2}})
        );
    }
}
