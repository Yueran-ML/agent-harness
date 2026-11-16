//! Compositions: the graphs of nodes that make the picture of a clip, as the Fusion page of
//! `DaVinci Resolve` builds them (Reference Manual 20, pp. 1211–1274; the Merge node,
//! pp. 2162–2170).
//!
//! A clip of a track of picture may have a composition ([`crate::project::Clip::fusion`]): of
//! media, a title, a colour or a still. A clip without one shows its source as it always did,
//! and is written as it always was. A clip of sound has none.
//!
//! # Where it is in the picture
//!
//! The composition makes the picture of its clip: what comes out of its `MediaOut` is taken
//! for the source of the clip, before the grade of the clip and before the transform, the crop
//! and the composite mode of the Edit page, which are done to it as they are to a source
//! ("`MediaOut`'s alpha returns to the Edit page", p. 1266). Its pictures are of the size of
//! the source of the clip (p. 1265): of media, its picture as it is shown upright; of a title
//! or a colour, the picture of the timeline ([`size_of`]).
//!
//! # Nodes and links
//!
//! A composition is a set of nodes and the links between them. A node has one output, but for
//! `MediaOut`, which has none; and the inputs of its kind, each known by its name
//! ([`Kind::inputs`]). A link goes from the output of a node into an input of another; an
//! output may feed any number of inputs, and an input takes at most one link. The links never
//! make a loop. The nodes are kept in the order in which they were added, and the links in the
//! order of the inputs that they go into: by the id of the node, and then by the name of the
//! input.
//!
//! The kinds of node ([`Kind`]):
//!
//! - `media_in`, no inputs: the picture of the clip's own source at the frame, as the engine
//!   reads it for a clip without a composition (along its speed points, and mixed of two
//!   pictures where its Retime Process says so). A composition may have several: they are the
//!   same picture.
//! - `media_out`, one input, `input`: the picture that goes back to the timeline. A
//!   composition has exactly one. Where its input is not linked the clip shows nothing: it is
//!   transparent.
//! - `merge`, the inputs `background` and `foreground`: the foreground laid over the
//!   background ([`Merge`]).
//! - `transform`, one input, `input`: its picture moved, made larger or smaller, turned and
//!   flipped ([`Transform`]).
//! - `background`, no inputs: one colour over the whole picture ([`Background`]).
//!
//! An input that is not linked gives no picture: a transparent one of the size of the
//! composition. A node that is passed through (Fusion's Pass Through, Ctrl-P) hands on the
//! picture of its first input as it is, and a node that has no input then gives none.
//!
//! # Coordinates
//!
//! The settings are kept in Fusion's own units, so that a number is what `DaVinci` shows for
//! it and what a tracker gives ([`Center`]): a place is a share of the width and of the height
//! of the picture, from its lower left corner, `x` to the right and `y` up, so that the middle
//! is 0.5 and 0.5; a size is a factor, 1 as it is; an angle is in degrees, counter-clockwise.
//! (The transform of the Edit page counts from the middle, `y` down and clockwise: the two are
//! not the same.)
//!
//! # Keys
//!
//! Each setting that is a number is a [`Param`], a number or keys. The keys count the frames
//! that the other keys of the clip count ([`crate::keyframes::KeyClock`]): those of its media,
//! or its own from its first for a title, a colour and a still. A setting is named by the id
//! of its node, a dot, and its path in the settings of the node: `n4.center.x`. So the id of a
//! node has no dot in it.

use std::collections::{HashMap, HashSet};

use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::grade::{check_label, Bounds, Composite, Param, Pos};
use crate::ops::OpError;
use crate::project::{Clip, Id, Project, Source, Timeline};

/// The most nodes of a composition. The manual gives none, and a grade has none: this is
/// ours, as many as a mixer of a grade has inputs at the most.
pub const MOST_NODES: usize = 256;

/// The id of the `MediaIn` of the composition that a clip has at first, as `DaVinci` names it.
pub const MEDIA_IN: &str = "MediaIn1";

/// The id of the `MediaOut` of the composition that a clip has at first, as `DaVinci` names it.
pub const MEDIA_OUT: &str = "MediaOut1";

/// The bounds of the settings of the nodes of a composition, in Fusion's units (see the
/// [module](self)). The manual gives no range of any of them: these are ours.
pub mod bounds {
    use crate::grade::Bounds;

    /// Subtractive/Additive of a Merge (p. 2167): 1, Additive, at first.
    pub const SUBTRACTIVE_ADDITIVE: Bounds = Bounds::new(1.0, 0.0, 1.0);
    /// Alpha Gain of a Merge (p. 2168): 1 changes nothing.
    pub const ALPHA_GAIN: Bounds = Bounds::new(1.0, 0.0, 1.0);
    /// Burn In of a Merge (p. 2168): 0 changes nothing.
    pub const BURN_IN: Bounds = Bounds::new(0.0, 0.0, 1.0);
    /// Blend of a Merge (p. 2168): 1, the merged result, at first.
    pub const BLEND: Bounds = Bounds::new(1.0, 0.0, 1.0);
    /// Center X and Y (p. 2164): 0.5, the middle, at first; as far as 16 pictures either way.
    pub const CENTER: Bounds = Bounds::new(0.5, -15.5, 16.5);
    /// Size (p. 2164): 1 changes nothing, and 0 leaves nothing of the picture.
    pub const SIZE: Bounds = Bounds::new(1.0, 0.0, 16.0);
    /// Angle (p. 2164), in degrees counter-clockwise: 0 changes nothing; ten turns either way.
    pub const ANGLE: Bounds = Bounds::new(0.0, -3600.0, 3600.0);
    /// The red, the green and the blue of a Background: black at first.
    pub const COLOR: Bounds = Bounds::new(0.0, 0.0, 1.0);
    /// The alpha of a Background: opaque at first.
    pub const ALPHA: Bounds = Bounds::new(1.0, 0.0, 1.0);
}

// ---------------------------------------------------------------------------------------------
// Kinds, inputs and links
// ---------------------------------------------------------------------------------------------

/// The kinds of node, as they are named in `kind`. Others are to come: masks, Blur, Mosaic,
/// the Delta Keyer, the Tracker and Text+.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Kind {
    /// The picture of the clip's own source.
    MediaIn,
    /// The picture that goes back to the timeline.
    MediaOut,
    /// A foreground laid over a background.
    Merge,
    /// A picture moved, made larger or smaller, turned and flipped.
    Transform,
    /// A colour over the whole picture.
    Background,
}

impl Kind {
    /// Its name, as it is written.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::MediaIn => "media_in",
            Self::MediaOut => "media_out",
            Self::Merge => "merge",
            Self::Transform => "transform",
            Self::Background => "background",
        }
    }

    /// The inputs that a node of the kind has. The first is the one whose picture the node
    /// hands on where it is passed through.
    #[must_use]
    pub fn inputs(self) -> &'static [Input] {
        match self {
            Self::MediaIn | Self::Background => &[],
            Self::MediaOut | Self::Transform => &[Input::Input],
            Self::Merge => &[Input::Background, Input::Foreground],
        }
    }

    /// Whether a node of the kind has an output: every kind but `media_out`.
    #[must_use]
    pub fn has_output(self) -> bool {
        self != Self::MediaOut
    }
}

/// An input of a node, by its name. The effect mask of a node is to come with the masks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Input {
    /// The one input of a node that has one: of `media_out` and of `transform`.
    Input,
    /// The background of a Merge (the orange input of an editor).
    Background,
    /// The foreground of a Merge (the green one).
    Foreground,
}

impl Input {
    /// Its name, as it is written.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Background => "background",
            Self::Foreground => "foreground",
        }
    }
}

/// A link of a composition: from the output of the node `from` into the input `input` of the
/// node `to`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Link {
    /// The node that it comes from.
    pub from: Id,
    /// The node that it goes into.
    pub to: Id,
    /// The input of `to` that it goes into.
    pub input: Input,
}

impl Link {
    /// A link from `from` into `input` of `to`.
    #[must_use]
    pub fn new(from: impl Into<Id>, to: impl Into<Id>, input: Input) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            input,
        }
    }

    /// The input that it goes into, by which it is told from the other links of its
    /// composition.
    #[must_use]
    pub fn target(&self) -> (&str, Input) {
        (&self.to, self.input)
    }

    /// Whether it comes from or goes to `node`.
    #[must_use]
    pub fn touches(&self, node: &str) -> bool {
        self.from == node || self.to == node
    }

    /// What the links of a composition are put in the order of: the node that it goes into,
    /// and the name of the input.
    fn place(&self) -> (&str, &'static str) {
        (&self.to, self.input.name())
    }
}

// ---------------------------------------------------------------------------------------------
// The settings of the kinds
// ---------------------------------------------------------------------------------------------

/// A place in a picture, as Fusion counts it: `x` the share of its width from its left edge,
/// and `y` the share of its height from its lower edge, so that the middle is 0.5 and 0.5.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Center {
    /// How far right, as a share of the width.
    pub x: Param,
    /// How far up, as a share of the height.
    pub y: Param,
}

impl Center {
    /// The middle of the picture.
    fn middle() -> Self {
        Self {
            x: bounds::CENTER.param(),
            y: bounds::CENTER.param(),
        }
    }

    fn at(&self, frame: f64) -> [f64; 2] {
        [self.x.value_at(frame), self.y.value_at(frame)]
    }

    fn check(&self, names: [&'static str; 2]) -> Result<(), OpError> {
        bounds::CENTER.check(names[0], &self.x)?;
        bounds::CENTER.check(names[1], &self.y)
    }
}

/// How the foreground and the background of a Merge are combined (Operator, pp. 2166–2167).
/// Over alone is done, as the mathematics of the Merge has it; the others are of the advanced
/// tier. It is written `"over"`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operator {
    /// The foreground over the background.
    #[default]
    Over,
}

/// The settings of a Merge (pp. 2162–2170): those that the mathematics of the Merge takes, by
/// the same names, and where the foreground lies over the background.
///
/// The Apply Modes are the composite modes of the timeline ([`Composite`]), which a Layer
/// Mixer has too: `normal` at first. The four sliders are each from 0 to 1.
/// Subtractive/Additive is read in the Normal mode alone. The foreground is put with its
/// middle at `center` of the background, made `size` times as large about its middle and
/// turned by `angle` there (p. 2164); the picture that comes out is of the size of the
/// background.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Merge {
    /// Apply Mode (pp. 2164–2166).
    pub apply: Composite,
    /// Operator (pp. 2166–2167).
    pub operator: Operator,
    /// Subtractive/Additive (p. 2167): from 0, Subtractive, to 1, Additive.
    pub subtractive_additive: Param,
    /// Alpha Gain (p. 2168): what the alpha of the foreground is multiplied by.
    pub alpha_gain: Param,
    /// Burn In (p. 2168): how much of the alpha is kept from darkening the background.
    pub burn_in: Param,
    /// Blend (p. 2168): from 0, the background as it is, to 1, the merged result.
    pub blend: Param,
    /// Center X and Y of the foreground (p. 2164), in the background.
    pub center: Center,
    /// Size of the foreground (p. 2164).
    pub size: Param,
    /// Angle of the foreground (p. 2164), in degrees counter-clockwise.
    pub angle: Param,
}

impl Default for Merge {
    /// The over of a premultiplied foreground, laid where it is.
    fn default() -> Self {
        Self {
            apply: Composite::Normal,
            operator: Operator::Over,
            subtractive_additive: bounds::SUBTRACTIVE_ADDITIVE.param(),
            alpha_gain: bounds::ALPHA_GAIN.param(),
            burn_in: bounds::BURN_IN.param(),
            blend: bounds::BLEND.param(),
            center: Center::middle(),
            size: bounds::SIZE.param(),
            angle: bounds::ANGLE.param(),
        }
    }
}

impl Merge {
    /// The four sliders that the mathematics of the Merge takes, each as a refusal names it
    /// and with its bounds, in the order of the Merge tab. The compositor is held to them.
    pub const SLIDERS: [(&'static str, Bounds); 4] = [
        ("merge.subtractive_additive", bounds::SUBTRACTIVE_ADDITIVE),
        ("merge.alpha_gain", bounds::ALPHA_GAIN),
        ("merge.burn_in", bounds::BURN_IN),
        ("merge.blend", bounds::BLEND),
    ];

    /// Its settings at a frame of its keys.
    #[must_use]
    pub fn at(&self, frame: f64) -> Merged {
        Merged {
            apply: self.apply,
            operator: self.operator,
            subtractive_additive: self.subtractive_additive.value_at(frame),
            alpha_gain: self.alpha_gain.value_at(frame),
            burn_in: self.burn_in.value_at(frame),
            blend: self.blend.value_at(frame),
            center: self.center.at(frame),
            size: self.size.value_at(frame),
            angle: self.angle.value_at(frame),
        }
    }

    fn sliders(&self) -> [&Param; 4] {
        [
            &self.subtractive_additive,
            &self.alpha_gain,
            &self.burn_in,
            &self.blend,
        ]
    }

    fn check(&self) -> Result<(), OpError> {
        for ((what, bounds), slider) in Self::SLIDERS.into_iter().zip(self.sliders()) {
            bounds.check(what, slider)?;
        }
        self.center.check(["merge.center.x", "merge.center.y"])?;
        bounds::SIZE.check("merge.size", &self.size)?;
        bounds::ANGLE.check("merge.angle", &self.angle)
    }

    fn settings(&self) -> [(&'static str, &Param); 8] {
        [
            ("subtractive_additive", &self.subtractive_additive),
            ("alpha_gain", &self.alpha_gain),
            ("burn_in", &self.burn_in),
            ("blend", &self.blend),
            ("center.x", &self.center.x),
            ("center.y", &self.center.y),
            ("size", &self.size),
            ("angle", &self.angle),
        ]
    }

    fn settings_mut(&mut self) -> [(&'static str, &mut Param); 8] {
        [
            ("subtractive_additive", &mut self.subtractive_additive),
            ("alpha_gain", &mut self.alpha_gain),
            ("burn_in", &mut self.burn_in),
            ("blend", &mut self.blend),
            ("center.x", &mut self.center.x),
            ("center.y", &mut self.center.y),
            ("size", &mut self.size),
            ("angle", &mut self.angle),
        ]
    }
}

/// The settings of a Transform node, those of the daily tier: its picture is put with its
/// middle at `center`, made `size` times as large about its middle, turned by `angle` there,
/// and flipped; the picture that comes out is of the size that came in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transform {
    /// Center X and Y.
    pub center: Center,
    /// Size.
    pub size: Param,
    /// Angle, in degrees counter-clockwise.
    pub angle: Param,
    /// Flip Horizontally: left for right.
    pub flip_horizontal: bool,
    /// Flip Vertically: top for bottom.
    pub flip_vertical: bool,
}

impl Default for Transform {
    /// The picture as it is.
    fn default() -> Self {
        Self {
            center: Center::middle(),
            size: bounds::SIZE.param(),
            angle: bounds::ANGLE.param(),
            flip_horizontal: false,
            flip_vertical: false,
        }
    }
}

impl Transform {
    /// Its settings at a frame of its keys.
    #[must_use]
    pub fn at(&self, frame: f64) -> Transformed {
        Transformed {
            center: self.center.at(frame),
            size: self.size.value_at(frame),
            angle: self.angle.value_at(frame),
            flip_horizontal: self.flip_horizontal,
            flip_vertical: self.flip_vertical,
        }
    }

    fn check(&self) -> Result<(), OpError> {
        self.center
            .check(["transform.center.x", "transform.center.y"])?;
        bounds::SIZE.check("transform.size", &self.size)?;
        bounds::ANGLE.check("transform.angle", &self.angle)
    }

    fn settings(&self) -> [(&'static str, &Param); 4] {
        [
            ("center.x", &self.center.x),
            ("center.y", &self.center.y),
            ("size", &self.size),
            ("angle", &self.angle),
        ]
    }

    fn settings_mut(&mut self) -> [(&'static str, &mut Param); 4] {
        [
            ("center.x", &mut self.center.x),
            ("center.y", &mut self.center.y),
            ("size", &mut self.size),
            ("angle", &mut self.angle),
        ]
    }
}

/// The colour of a Background, as Fusion keeps one: its red, its green, its blue and its
/// alpha, each from 0 to 1 and each a number or keys. The colour is not premultiplied by the
/// alpha: the compositor does that.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Color {
    /// Red.
    pub r: Param,
    /// Green.
    pub g: Param,
    /// Blue.
    pub b: Param,
    /// Alpha: from 0, not seen, to 1.
    pub a: Param,
}

/// The settings of a Background node: one colour over the whole picture (its Solid type; the
/// gradients are not kept). It is opaque black at first, as `DaVinci` makes one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Background {
    /// The colour.
    pub color: Color,
}

impl Default for Background {
    fn default() -> Self {
        Self {
            color: Color {
                r: bounds::COLOR.param(),
                g: bounds::COLOR.param(),
                b: bounds::COLOR.param(),
                a: bounds::ALPHA.param(),
            },
        }
    }
}

impl Background {
    /// Its colour at a frame of its keys: red, green, blue and alpha, not premultiplied.
    #[must_use]
    pub fn at(&self, frame: f64) -> [f64; 4] {
        let Color { r, g, b, a } = &self.color;
        [r, g, b, a].map(|channel| channel.value_at(frame))
    }

    fn check(&self) -> Result<(), OpError> {
        let Color { r, g, b, a } = &self.color;
        bounds::COLOR.check("background.color.r", r)?;
        bounds::COLOR.check("background.color.g", g)?;
        bounds::COLOR.check("background.color.b", b)?;
        bounds::ALPHA.check("background.color.a", a)
    }

    fn settings(&self) -> [(&'static str, &Param); 4] {
        let Color { r, g, b, a } = &self.color;
        [
            ("color.r", r),
            ("color.g", g),
            ("color.b", b),
            ("color.a", a),
        ]
    }

    fn settings_mut(&mut self) -> [(&'static str, &mut Param); 4] {
        let Color { r, g, b, a } = &mut self.color;
        [
            ("color.r", r),
            ("color.g", g),
            ("color.b", b),
            ("color.a", a),
        ]
    }
}

// ---------------------------------------------------------------------------------------------
// Nodes
// ---------------------------------------------------------------------------------------------

/// What kind of node a node is, and how it is set.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum NodeKind {
    /// The picture of the clip's own source: it has no settings.
    MediaIn,
    /// The picture that goes back to the timeline: it has no settings.
    MediaOut,
    /// A foreground laid over a background.
    Merge(Merge),
    /// A picture moved, made larger or smaller, turned and flipped.
    Transform(Transform),
    /// A colour over the whole picture.
    Background(Background),
}

/// The settings of a kind that has none, as they are written: `{}`.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoSettings {}

impl NodeKind {
    /// Its kind.
    #[must_use]
    pub fn kind(&self) -> Kind {
        match self {
            Self::MediaIn => Kind::MediaIn,
            Self::MediaOut => Kind::MediaOut,
            Self::Merge(_) => Kind::Merge,
            Self::Transform(_) => Kind::Transform,
            Self::Background(_) => Kind::Background,
        }
    }

    /// A node of `kind` as a new one is set.
    #[must_use]
    pub fn new(kind: Kind) -> Self {
        match kind {
            Kind::MediaIn => Self::MediaIn,
            Kind::MediaOut => Self::MediaOut,
            Kind::Merge => Self::Merge(Merge::default()),
            Kind::Transform => Self::Transform(Transform::default()),
            Kind::Background => Self::Background(Background::default()),
        }
    }

    /// Read the settings of a node of `kind`.
    ///
    /// # Errors
    /// They are not the settings of a node of that kind.
    pub fn from_params(kind: Kind, params: serde_json::Value) -> Result<Self, serde_json::Error> {
        Ok(match kind {
            Kind::MediaIn => {
                serde_json::from_value::<NoSettings>(params)?;
                Self::MediaIn
            }
            Kind::MediaOut => {
                serde_json::from_value::<NoSettings>(params)?;
                Self::MediaOut
            }
            Kind::Merge => Self::Merge(serde_json::from_value(params)?),
            Kind::Transform => Self::Transform(serde_json::from_value(params)?),
            Kind::Background => Self::Background(serde_json::from_value(params)?),
        })
    }

    /// Its settings, as they are written in `params`: an object with no fields for a kind
    /// that has none.
    ///
    /// # Panics
    /// Never: the settings of a node are always written as JSON.
    #[must_use]
    pub fn params(&self) -> serde_json::Value {
        let value = match self {
            Self::MediaIn | Self::MediaOut => serde_json::to_value(NoSettings {}),
            Self::Merge(params) => serde_json::to_value(params),
            Self::Transform(params) => serde_json::to_value(params),
            Self::Background(params) => serde_json::to_value(params),
        };
        value.expect("the settings of a node are written as JSON")
    }

    /// Check its settings: each within its bounds, as a number or at each key.
    pub(crate) fn check(&self) -> Result<(), OpError> {
        match self {
            Self::MediaIn | Self::MediaOut => Ok(()),
            Self::Merge(merge) => merge.check(),
            Self::Transform(transform) => transform.check(),
            Self::Background(background) => background.check(),
        }
    }

    /// Its settings that take keys, each with its path in `params`.
    fn keyed(&self) -> Vec<(&'static str, &Param)> {
        match self {
            Self::MediaIn | Self::MediaOut => vec![],
            Self::Merge(merge) => merge.settings().into(),
            Self::Transform(transform) => transform.settings().into(),
            Self::Background(background) => background.settings().into(),
        }
    }

    /// Its settings that take keys, each with its path in `params`, to be changed.
    fn settings_mut(&mut self) -> Vec<(&'static str, &mut Param)> {
        match self {
            Self::MediaIn | Self::MediaOut => vec![],
            Self::Merge(merge) => merge.settings_mut().into(),
            Self::Transform(transform) => transform.settings_mut().into(),
            Self::Background(background) => background.settings_mut().into(),
        }
    }

    /// The setting at `path` of its `params`, if it is one that takes keys: `center.x`.
    pub(crate) fn setting_mut(&mut self, path: &str) -> Option<&mut Param> {
        self.settings_mut()
            .into_iter()
            .find_map(|(name, setting)| (name == path).then_some(setting))
    }

    /// The paths of its settings that take keys, in the order of its `params`.
    #[must_use]
    pub fn settings(&self) -> Vec<&'static str> {
        self.keyed().into_iter().map(|(name, _)| name).collect()
    }

    /// Whether any of its settings has keys.
    #[must_use]
    pub fn is_animated(&self) -> bool {
        self.keyed()
            .into_iter()
            .any(|(_, setting)| setting.is_animated())
    }

    /// It with each key of its settings at the frame `to` of its frame
    /// ([`Param::with_key_frames`]).
    #[must_use]
    pub fn with_key_frames(&self, to: &impl Fn(i64) -> i64) -> NodeKind {
        let mut moved = self.clone();
        for (_, setting) in moved.settings_mut() {
            *setting = setting.with_key_frames(to);
        }
        moved
    }
}

/// A node of a composition.
///
/// It is written `{"id", "kind", "label", "pass_through", "pos", "params"}`, where `params` is
/// what a node of its kind is set by: a [`Merge`], a [`Transform`] or a [`Background`], and an
/// object with no fields for `media_in` and `media_out`. `pass_through` and `pos` are always
/// written, and may be left out where a node is read: they are then `false` and `null`.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// What it is known by in its composition: `n` and a number where the engine made it. It
    /// has no dot in it, since a dot ends it in the path of a setting.
    pub id: Id,
    /// The name that it was given, or empty: at most
    /// [`LONGEST_LABEL`](crate::grade::LONGEST_LABEL) characters.
    pub label: String,
    /// Whether it is passed through (Pass Through, Ctrl-P): it then hands on the picture of
    /// its first input as it is, and a node with no input gives none.
    pub pass_through: bool,
    /// Where it lies in the node editor, or none for where the editor lays it out, as a node
    /// of a grade has it ([`Pos`]).
    pub pos: Option<Pos>,
    /// What kind of node it is, and how it is set.
    pub kind: NodeKind,
}

/// A node as it is written, before its `params` are read for its kind.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawNode {
    id: Id,
    kind: Kind,
    label: String,
    #[serde(default)]
    pass_through: bool,
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
            pass_through: raw.pass_through,
            pos: raw.pos,
            kind,
        })
    }
}

impl Serialize for Node {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut node = serializer.serialize_struct("Node", 6)?;
        node.serialize_field("id", &self.id)?;
        node.serialize_field("kind", &self.kind.kind())?;
        node.serialize_field("label", &self.label)?;
        node.serialize_field("pass_through", &self.pass_through)?;
        node.serialize_field("pos", &self.pos)?;
        match &self.kind {
            NodeKind::MediaIn | NodeKind::MediaOut => {
                node.serialize_field("params", &NoSettings {})?;
            }
            NodeKind::Merge(params) => node.serialize_field("params", params)?,
            NodeKind::Transform(params) => node.serialize_field("params", params)?,
            NodeKind::Background(params) => node.serialize_field("params", params)?,
        }
        node.end()
    }
}

impl Node {
    /// A node of `kind` as a new one is set, not passed through, with no label, where the
    /// editor lays it out.
    #[must_use]
    pub fn new(id: impl Into<Id>, kind: Kind) -> Self {
        Self {
            id: id.into(),
            label: String::new(),
            pass_through: false,
            pos: None,
            kind: NodeKind::new(kind),
        }
    }
}

/// Check the id of a node: it has one, it does not start with `$`, and it has no dot in it.
pub(crate) fn check_node_id(id: &str) -> Result<(), OpError> {
    if id.is_empty() {
        Err(OpError::NoId)
    } else if id.starts_with('$') {
        Err(OpError::Dollar { id: id.into() })
    } else if id.contains('.') {
        Err(OpError::OutOfRange { what: "fusion_id" })
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

fn missing_node(id: &str) -> OpError {
    OpError::Missing {
        what: "node",
        id: id.into(),
    }
}

// ---------------------------------------------------------------------------------------------
// Compositions
// ---------------------------------------------------------------------------------------------

/// A composition: its nodes, in the order in which they were added, and its links, in the
/// order of the inputs that they go into (by `to`, then by the name of `input`).
///
/// At first it is the one that `DaVinci` gives a clip (p. 1265): a `MediaIn` linked to a
/// `MediaOut`, which shows the clip as it is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Composition {
    /// Its nodes.
    pub nodes: Vec<Node>,
    /// Its links.
    pub links: Vec<Link>,
}

impl Default for Composition {
    /// The composition that a clip has when the Fusion page is first opened on it: a `MediaIn`
    /// ([`MEDIA_IN`]) linked to a `MediaOut` ([`MEDIA_OUT`]), each where the editor lays it out.
    fn default() -> Self {
        Self {
            nodes: vec![
                Node::new(MEDIA_IN, Kind::MediaIn),
                Node::new(MEDIA_OUT, Kind::MediaOut),
            ],
            links: vec![Link::new(MEDIA_IN, MEDIA_OUT, Input::Input)],
        }
    }
}

impl Composition {
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

    /// The link into `input` of `to`, if there is one.
    #[must_use]
    pub fn link_into(&self, to: &str, input: Input) -> Option<&Link> {
        self.links.iter().find(|l| l.target() == (to, input))
    }

    /// Its `MediaOut`: the first, where a composition that was not checked has several.
    #[must_use]
    pub fn media_out(&self) -> Option<&Node> {
        self.nodes
            .iter()
            .find(|node| node.kind.kind() == Kind::MediaOut)
    }

    /// Put its links in the order of the inputs that they go into.
    pub fn sort_links(&mut self) {
        self.links.sort_by(|a, b| a.place().cmp(&b.place()));
    }

    /// Whether a setting of one of its nodes has keys.
    #[must_use]
    pub fn is_animated(&self) -> bool {
        self.nodes.iter().any(|node| node.kind.is_animated())
    }

    /// The composition with each key of each setting of its nodes moved from frame `k` to
    /// `to(k)`, as [`Param::with_key_frames`] moves the keys of a setting.
    #[must_use]
    pub fn with_key_frames(&self, to: &impl Fn(i64) -> i64) -> Composition {
        let mut moved = self.clone();
        for node in &mut moved.nodes {
            if node.kind.is_animated() {
                node.kind = node.kind.with_key_frames(to);
            }
        }
        moved
    }

    /// Check the composition: how many nodes it has, their ids and their settings, that it has
    /// one `MediaOut`, and its links. The links are to be in order.
    pub(crate) fn check(&self) -> Result<(), OpError> {
        if self.nodes.len() > MOST_NODES {
            return Err(OpError::OutOfRange {
                what: "fusion_nodes",
            });
        }
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
        let outs = self
            .nodes
            .iter()
            .filter(|node| node.kind.kind() == Kind::MediaOut)
            .count();
        if outs != 1 {
            return Err(OpError::OneMediaOut { count: outs });
        }
        self.check_links()
    }

    /// Check its links: each comes from a node that has an output and goes into an input that
    /// its node has, no two go into one input, and they make no loop. The links are to be in
    /// order.
    fn check_links(&self) -> Result<(), OpError> {
        for link in &self.links {
            let from = self
                .node(&link.from)
                .ok_or_else(|| missing_node(&link.from))?;
            if !from.kind.kind().has_output() {
                return Err(OpError::NoOutput {
                    node: link.from.clone(),
                });
            }
            let to = self.node(&link.to).ok_or_else(|| missing_node(&link.to))?;
            if !to.kind.kind().inputs().contains(&link.input) {
                return Err(OpError::NoInput {
                    node: link.to.clone(),
                    input: link.input,
                });
            }
        }
        if let Some(pair) = self
            .links
            .windows(2)
            .find(|pair| pair[0].target() == pair[1].target())
        {
            return Err(OpError::InputLinked {
                node: pair[1].to.clone(),
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
        // Not in it: not seen; false: on the path; true: done.
        let mut done: HashMap<&str, bool> = HashMap::new();
        for start in self.nodes.iter().map(|n| n.id.as_str()) {
            if done.contains_key(start) {
                continue;
            }
            let mut stack = vec![(start, 0_usize)];
            done.insert(start, false);
            while let Some((node, next)) = stack.pop() {
                let outs = after.get(node).map_or(&[][..], Vec::as_slice);
                if let Some(&to) = outs.get(next) {
                    stack.push((node, next + 1));
                    match done.get(to) {
                        Some(false) => return Some(to.to_owned()),
                        Some(true) => {}
                        None => {
                            done.insert(to, false);
                            stack.push((to, 0));
                        }
                    }
                } else {
                    done.insert(node, true);
                }
            }
        }
        None
    }

    /// The node whose picture comes into `input` of the node `to`: the one that the link
    /// there comes from, or, where that one is passed through, the one whose picture it hands
    /// on, and so on. None where no picture comes in: the input, or the first input of a node
    /// that is passed through on the way, is not linked, or such a node has no input.
    fn feeding(&self, to: &str, input: Input) -> Option<&Node> {
        let (mut to, mut input) = (to, input);
        // A composition that is checked has no loop: one that is not is not followed for ever.
        for _ in 0..=self.nodes.len() {
            let node = self.node(&self.link_into(to, input)?.from)?;
            if !node.pass_through {
                return Some(node);
            }
            (to, input) = (node.id.as_str(), *node.kind.kind().inputs().first()?);
        }
        None
    }

    /// The composition at a frame of its keys: the nodes that the picture of its `MediaOut`
    /// is made of, each with its settings as the numbers that they are there (see [`Fused`]).
    #[must_use]
    pub fn at(&self, frame: f64) -> Fused {
        let mut nodes = Vec::new();
        let mut placed = HashMap::new();
        let output = self
            .media_out()
            .and_then(|out| self.feeding(&out.id, Input::Input))
            .and_then(|node| self.place(node, frame, &mut nodes, &mut placed));
        Fused { nodes, output }
    }

    /// Put `node` among `nodes`, after the nodes whose pictures it reads, and give its place
    /// there: the one that it has already where it was put before, and none for a node that
    /// has no picture to give.
    fn place<'a>(
        &'a self,
        node: &'a Node,
        frame: f64,
        nodes: &mut Vec<FusedNode>,
        placed: &mut HashMap<&'a str, Option<usize>>,
    ) -> Option<usize> {
        if let Some(at) = placed.get(node.id.as_str()) {
            return *at;
        }
        // A node on a loop, which a composition that is checked has none of, reads nothing of
        // itself.
        placed.insert(&node.id, None);
        let mut read = |input: Input| {
            self.feeding(&node.id, input)
                .and_then(|from| self.place(from, frame, nodes, placed))
        };
        let kind = match &node.kind {
            NodeKind::MediaIn => FusedKind::MediaIn,
            NodeKind::MediaOut => return None,
            NodeKind::Background(background) => FusedKind::Background {
                rgba: background.at(frame),
            },
            NodeKind::Merge(merge) => FusedKind::Merge {
                background: read(Input::Background),
                foreground: read(Input::Foreground),
                settings: merge.at(frame),
            },
            NodeKind::Transform(transform) => FusedKind::Transform {
                input: read(Input::Input),
                settings: transform.at(frame),
            },
        };
        let at = nodes.len();
        nodes.push(FusedNode {
            id: node.id.clone(),
            kind,
        });
        placed.insert(&node.id, Some(at));
        Some(at)
    }
}

/// The size of the pictures of the composition of a clip of a timeline of a project, in
/// pixels: that of the source of the clip (p. 1265). Of media with a picture, its picture as
/// it is shown upright, so that one that is turned a quarter has its width for its height; of
/// a title, of a colour, and of media that the project has not, the picture of the timeline.
#[must_use]
pub fn size_of(clip: &Clip, project: &Project, timeline: &Timeline) -> (u32, u32) {
    let picture = match &clip.source {
        Source::Media { media, .. } => project
            .media
            .iter()
            .find(|m| m.id == *media)
            .and_then(|media| media.facts.video.as_ref()),
        Source::Title { .. } | Source::Color { .. } => None,
    };
    match picture {
        Some(video) if video.rotation.rem_euclid(180) == 90 => (video.height, video.width),
        Some(video) => (video.width, video.height),
        None => (timeline.width, timeline.height),
    }
}

// ---------------------------------------------------------------------------------------------
// A composition at one moment
// ---------------------------------------------------------------------------------------------

/// A composition at one moment, as the compositor draws it: the nodes that the picture of its
/// `MediaOut` is made of, each with its settings as the numbers that they are at the frame.
///
/// The nodes are in an order in which each comes after those whose pictures it reads, and a
/// node names the one that an input reads by its place in the list. Only the nodes that the
/// `MediaOut` depends on are there, each once: a node whose picture is not read is left out,
/// and so is one that is passed through, whose place is taken by the node that it hands on.
/// The `MediaOut` itself is not in the list: `output` is what it shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fused {
    /// The nodes, each after those that it reads.
    pub nodes: Vec<FusedNode>,
    /// The place in `nodes` of the node whose picture goes back to the timeline; none where
    /// no picture comes into the `MediaOut`, and the clip is transparent.
    pub output: Option<usize>,
}

/// A node of a composition at one moment.
#[derive(Clone, Debug, PartialEq)]
pub struct FusedNode {
    /// The node of the composition that it is.
    pub id: Id,
    /// What it draws.
    pub kind: FusedKind,
}

/// What a node of a composition draws at one moment. A picture that an input reads is named
/// by its place in [`Fused::nodes`], which is before that of the node; none is no picture, a
/// transparent one of the size of the composition.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum FusedKind {
    /// The picture of the clip's own source at the frame.
    MediaIn,
    /// A colour over the whole picture.
    Background {
        /// Its red, green, blue and alpha, each from 0 to 1, not premultiplied.
        rgba: [f64; 4],
    },
    /// A foreground laid over a background.
    Merge {
        /// The picture of the background.
        background: Option<usize>,
        /// The picture of the foreground.
        foreground: Option<usize>,
        /// How it is laid.
        settings: Merged,
    },
    /// A picture moved, made larger or smaller, turned and flipped.
    Transform {
        /// The picture.
        input: Option<usize>,
        /// How it is put.
        settings: Transformed,
    },
}

impl FusedKind {
    /// The picture that the first input of a node of the kind reads, by its place in
    /// [`Fused::nodes`]: of a Merge its background, of a Transform its one input. None of a
    /// kind that has no input, and where the input reads no picture. It is the picture that
    /// what cannot draw a kind hands on in its place, as a node that is passed through hands
    /// on that of its first input.
    #[must_use]
    pub fn first_input(&self) -> Option<usize> {
        match self {
            Self::MediaIn | Self::Background { .. } => None,
            Self::Merge { background, .. } => *background,
            Self::Transform { input, .. } => *input,
        }
    }
}

/// The settings of a [`Merge`] at one moment, each that takes keys as the number that it is
/// there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Merged {
    /// Apply Mode.
    pub apply: Composite,
    /// Operator.
    pub operator: Operator,
    /// Subtractive/Additive: from 0, Subtractive, to 1, Additive.
    pub subtractive_additive: f64,
    /// Alpha Gain.
    pub alpha_gain: f64,
    /// Burn In.
    pub burn_in: f64,
    /// Blend.
    pub blend: f64,
    /// Where the middle of the foreground is in the background: `x` and `y`, as [`Center`]
    /// counts them.
    pub center: [f64; 2],
    /// How many times as large the foreground is made.
    pub size: f64,
    /// How far the foreground is turned, in degrees counter-clockwise.
    pub angle: f64,
}

/// The settings of a [`Transform`] at one moment, each that takes keys as the number that it
/// is there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transformed {
    /// Where the middle of the picture is put: `x` and `y`, as [`Center`] counts them.
    pub center: [f64; 2],
    /// How many times as large it is made.
    pub size: f64,
    /// How far it is turned, in degrees counter-clockwise.
    pub angle: f64,
    /// Whether it is flipped left for right.
    pub flip_horizontal: bool,
    /// Whether it is flipped top for bottom.
    pub flip_vertical: bool,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::grade::{Interp, Key};

    fn keys(frames: &[(i64, f64)]) -> Param {
        Param::Keys(
            frames
                .iter()
                .map(|&(frame, value)| Key {
                    frame,
                    value,
                    interp: Interp::Linear,
                })
                .collect(),
        )
    }

    /// A background under the picture of the clip, moved by a Transform: `bg` and `in` into
    /// `over`, whose picture `move` moves into `out`.
    fn merged() -> Composition {
        let mut composition = Composition {
            nodes: vec![
                Node::new("in", Kind::MediaIn),
                Node::new("bg", Kind::Background),
                Node::new("over", Kind::Merge),
                Node::new("move", Kind::Transform),
                Node::new("out", Kind::MediaOut),
            ],
            links: vec![
                Link::new("bg", "over", Input::Background),
                Link::new("in", "over", Input::Foreground),
                Link::new("over", "move", Input::Input),
                Link::new("move", "out", Input::Input),
            ],
        };
        composition.sort_links();
        composition
    }

    #[test]
    fn a_composition_is_at_first_a_media_in_linked_to_a_media_out() {
        let first = Composition::default();
        assert_eq!(first.check(), Ok(()));
        assert_eq!(
            serde_json::to_value(&first).unwrap(),
            json!({
                "nodes": [
                    {"id": "MediaIn1", "kind": "media_in", "label": "", "pass_through": false,
                     "pos": null, "params": {}},
                    {"id": "MediaOut1", "kind": "media_out", "label": "", "pass_through": false,
                     "pos": null, "params": {}},
                ],
                "links": [{"from": "MediaIn1", "to": "MediaOut1", "input": "input"}],
            })
        );
        assert!(!first.is_animated());
        // It shows the clip as it is: the picture of its source goes back to the timeline.
        assert_eq!(
            first.at(0.0),
            Fused {
                nodes: vec![FusedNode {
                    id: "MediaIn1".into(),
                    kind: FusedKind::MediaIn,
                }],
                output: Some(0),
            }
        );
    }

    #[test]
    fn a_node_is_written_with_the_params_of_its_kind_and_read_back() {
        let written = serde_json::to_value(merged()).unwrap();
        assert_eq!(
            written["nodes"][2],
            json!({
                "id": "over", "kind": "merge", "label": "", "pass_through": false, "pos": null,
                "params": {
                    "apply": "normal", "operator": "over", "subtractive_additive": 1.0,
                    "alpha_gain": 1.0, "burn_in": 0.0, "blend": 1.0,
                    "center": {"x": 0.5, "y": 0.5}, "size": 1.0, "angle": 0.0,
                },
            })
        );
        assert_eq!(
            written["nodes"][3]["params"],
            json!({
                "center": {"x": 0.5, "y": 0.5}, "size": 1.0, "angle": 0.0,
                "flip_horizontal": false, "flip_vertical": false,
            })
        );
        assert_eq!(
            written["nodes"][1]["params"],
            json!({"color": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0}})
        );
        assert_eq!(
            written["links"],
            json!([
                {"from": "over", "to": "move", "input": "input"},
                {"from": "move", "to": "out", "input": "input"},
                {"from": "bg", "to": "over", "input": "background"},
                {"from": "in", "to": "over", "input": "foreground"},
            ])
        );
        let read: Composition = serde_json::from_value(written).unwrap();
        assert_eq!(read, merged());
        // Whether it is passed through and where it lies may be left out.
        let short: Node = serde_json::from_value(
            json!({"id": "a", "kind": "media_in", "label": "", "params": {}}),
        )
        .unwrap();
        assert_eq!(short, Node::new("a", Kind::MediaIn));
        // The settings of another kind, a setting that is not known, and settings of a kind
        // that has none are refused.
        for wrong in [
            json!({"id": "a", "kind": "media_in", "label": "", "params": {"size": 1.0}}),
            json!({"id": "a", "kind": "transform", "label": "", "params": {}}),
            json!({"id": "a", "kind": "background", "label": "",
                   "params": {"color": {"r": 0, "g": 0, "b": 0, "a": 1}, "type": "solid"}}),
            json!({"id": "a", "kind": "blur", "label": "", "params": {}}),
            json!({"id": "a", "kind": "media_in", "label": "", "params": {}, "enabled": true}),
        ] {
            assert!(
                serde_json::from_value::<Node>(wrong.clone()).is_err(),
                "{wrong}"
            );
        }
        let link = json!({"from": "a", "to": "b", "input": "effect_mask"});
        assert!(serde_json::from_value::<Link>(link).is_err());
    }

    #[test]
    fn each_kind_has_its_inputs_and_every_kind_but_media_out_an_output() {
        assert_eq!(Kind::MediaIn.inputs(), &[] as &[Input]);
        assert_eq!(Kind::Background.inputs(), &[] as &[Input]);
        assert_eq!(Kind::MediaOut.inputs(), &[Input::Input]);
        assert_eq!(Kind::Transform.inputs(), &[Input::Input]);
        assert_eq!(
            Kind::Merge.inputs(),
            &[Input::Background, Input::Foreground]
        );
        for kind in [
            Kind::MediaIn,
            Kind::MediaOut,
            Kind::Merge,
            Kind::Transform,
            Kind::Background,
        ] {
            assert_eq!(kind.has_output(), kind != Kind::MediaOut);
            assert_eq!(NodeKind::new(kind).kind(), kind);
            assert_eq!(serde_json::to_value(kind).unwrap(), json!(kind.name()));
            let params = NodeKind::new(kind).params();
            assert_eq!(
                NodeKind::from_params(kind, params).unwrap(),
                NodeKind::new(kind)
            );
            assert_eq!(NodeKind::new(kind).check(), Ok(()));
        }
        for input in [Input::Input, Input::Background, Input::Foreground] {
            assert_eq!(serde_json::to_value(input).unwrap(), json!(input.name()));
        }
    }

    #[test]
    fn links_are_kept_in_the_order_of_the_inputs_that_they_go_into() {
        let mut composition = merged();
        composition.links.reverse();
        composition.sort_links();
        let order: Vec<(&str, &str)> = composition
            .links
            .iter()
            .map(|link| (link.to.as_str(), link.input.name()))
            .collect();
        assert_eq!(
            order,
            [
                ("move", "input"),
                ("out", "input"),
                ("over", "background"),
                ("over", "foreground"),
            ]
        );
        assert_eq!(composition, merged());
    }

    #[test]
    fn a_composition_has_one_media_out_and_no_more_nodes_than_the_most() {
        let mut none = merged();
        none.nodes.pop();
        none.links.retain(|link| link.to != "out");
        assert_eq!(none.check(), Err(OpError::OneMediaOut { count: 0 }));
        let mut two = merged();
        two.nodes.push(Node::new("out2", Kind::MediaOut));
        assert_eq!(two.check(), Err(OpError::OneMediaOut { count: 2 }));
        // A MediaOut whose input is not linked is kept: the clip is then transparent.
        let mut unlinked = merged();
        unlinked.links.retain(|link| link.to != "out");
        assert_eq!(unlinked.check(), Ok(()));
        assert_eq!(unlinked.at(0.0), Fused::default());
        // Nor need it have a MediaIn.
        let mut made = merged();
        made.nodes.remove(0);
        made.links.retain(|link| link.from != "in");
        assert_eq!(made.check(), Ok(()));
        let mut many = Composition::default();
        for n in 0..MOST_NODES - 2 {
            many.nodes
                .push(Node::new(format!("b{n}"), Kind::Background));
        }
        assert_eq!(many.check(), Ok(()));
        many.nodes.push(Node::new("one-more", Kind::Background));
        assert_eq!(
            many.check(),
            Err(OpError::OutOfRange {
                what: "fusion_nodes"
            })
        );
    }

    #[test]
    fn the_id_of_a_node_is_its_own_and_has_no_dot_in_it() {
        let with = |id: &str| {
            let mut composition = merged();
            composition.nodes[1].id = id.into();
            composition.links.retain(|link| link.from != "bg");
            composition.check()
        };
        assert_eq!(with("bg2"), Ok(()));
        assert_eq!(with(""), Err(OpError::NoId));
        assert_eq!(with("$bg"), Err(OpError::Dollar { id: "$bg".into() }));
        assert_eq!(with("bg.1"), Err(OpError::OutOfRange { what: "fusion_id" }));
        assert_eq!(
            with("in"),
            Err(OpError::Taken {
                what: "node",
                id: "in".into()
            })
        );
        let mut labelled = merged();
        labelled.nodes[0].label = "x".repeat(crate::grade::LONGEST_LABEL + 1);
        assert_eq!(labelled.check(), Err(OpError::OutOfRange { what: "label" }));
        let mut far = merged();
        far.nodes[0].pos = Some(Pos {
            x: f64::INFINITY,
            y: 0.0,
        });
        assert_eq!(far.check(), Err(OpError::OutOfRange { what: "pos" }));
    }

    #[test]
    fn links_that_go_round_or_into_what_is_not_there_are_refused() {
        let with = |link: Link| {
            let mut composition = merged();
            composition.links.push(link);
            composition.sort_links();
            composition.check()
        };
        let missing = |id: &str| {
            Err(OpError::Missing {
                what: "node",
                id: id.into(),
            })
        };
        assert_eq!(
            with(Link::new("nowhere", "move", Input::Input)),
            missing("nowhere")
        );
        assert_eq!(
            with(Link::new("in", "nowhere", Input::Input)),
            missing("nowhere")
        );
        // A MediaOut has no output, and a kind has the inputs of its kind only.
        assert_eq!(
            with(Link::new("out", "over", Input::Foreground)),
            Err(OpError::NoOutput { node: "out".into() })
        );
        assert_eq!(
            with(Link::new("in", "move", Input::Foreground)),
            Err(OpError::NoInput {
                node: "move".into(),
                input: Input::Foreground
            })
        );
        assert_eq!(
            with(Link::new("in", "bg", Input::Input)),
            Err(OpError::NoInput {
                node: "bg".into(),
                input: Input::Input
            })
        );
        assert_eq!(
            with(Link::new("in", "over", Input::Input)),
            Err(OpError::NoInput {
                node: "over".into(),
                input: Input::Input
            })
        );
        // An input takes one link.
        assert_eq!(
            with(Link::new("in", "over", Input::Background)),
            Err(OpError::InputLinked {
                node: "over".into(),
                input: Input::Background
            })
        );
        // And the links make no loop: not through one node, and not through several.
        let mut round = merged();
        round.links.retain(|link| link.from != "bg");
        round
            .links
            .push(Link::new("move", "over", Input::Background));
        round.sort_links();
        assert!(matches!(round.check(), Err(OpError::Loop { .. })));
        let mut own = merged();
        own.links.retain(|link| link.from != "over");
        own.links.push(Link::new("move", "move", Input::Input));
        own.sort_links();
        assert_eq!(
            own.check(),
            Err(OpError::Loop {
                node: "move".into()
            })
        );
        // One output may feed several inputs.
        let mut twice = merged();
        twice.links.retain(|link| link.from != "bg");
        twice.links.push(Link::new("in", "over", Input::Background));
        twice.sort_links();
        assert_eq!(twice.check(), Ok(()));
    }

    #[test]
    #[allow(clippy::too_many_lines, reason = "a refusal for each setting")]
    fn a_setting_is_within_its_bounds_as_a_number_and_at_each_key() {
        let refused = |change: &dyn Fn(&mut Composition), what: &'static str| {
            let mut composition = merged();
            change(&mut composition);
            assert_eq!(
                composition.check(),
                Err(OpError::OutOfRange { what }),
                "{what}"
            );
        };
        // Each slider of the Merge, by the name that the compositor refuses it by.
        for (at, (what, bounds)) in Merge::SLIDERS.into_iter().enumerate() {
            assert_eq!((bounds.low, bounds.high), (0.0, 1.0), "{what}");
            for value in [-0.001, 1.001, f64::NAN, f64::INFINITY] {
                let set = move |composition: &mut Composition| {
                    let NodeKind::Merge(merge) = &mut composition.nodes[2].kind else {
                        unreachable!("the third node is the merge")
                    };
                    let slider = match at {
                        0 => &mut merge.subtractive_additive,
                        1 => &mut merge.alpha_gain,
                        2 => &mut merge.burn_in,
                        _ => &mut merge.blend,
                    };
                    *slider = Param::Fixed(value);
                };
                refused(&set, what);
            }
        }
        refused(
            &|composition| {
                if let NodeKind::Merge(merge) = &mut composition.nodes[2].kind {
                    merge.center.x = Param::Fixed(17.0);
                }
            },
            "merge.center.x",
        );
        refused(
            &|composition| {
                if let NodeKind::Merge(merge) = &mut composition.nodes[2].kind {
                    merge.center.y = keys(&[(0, 0.5), (10, -16.0)]);
                }
            },
            "merge.center.y",
        );
        refused(
            &|composition| {
                if let NodeKind::Merge(merge) = &mut composition.nodes[2].kind {
                    merge.size = Param::Fixed(-0.5);
                }
            },
            "merge.size",
        );
        refused(
            &|composition| {
                if let NodeKind::Merge(merge) = &mut composition.nodes[2].kind {
                    merge.angle = Param::Fixed(3600.5);
                }
            },
            "merge.angle",
        );
        refused(
            &|composition| {
                if let NodeKind::Transform(transform) = &mut composition.nodes[3].kind {
                    transform.center.x = Param::Fixed(f64::NAN);
                }
            },
            "transform.center.x",
        );
        refused(
            &|composition| {
                if let NodeKind::Transform(transform) = &mut composition.nodes[3].kind {
                    transform.center.y = Param::Fixed(-15.6);
                }
            },
            "transform.center.y",
        );
        refused(
            &|composition| {
                if let NodeKind::Transform(transform) = &mut composition.nodes[3].kind {
                    transform.size = Param::Fixed(16.5);
                }
            },
            "transform.size",
        );
        refused(
            &|composition| {
                if let NodeKind::Transform(transform) = &mut composition.nodes[3].kind {
                    // Keys that are not in the order of their frames.
                    transform.angle = keys(&[(10, 0.0), (10, 90.0)]);
                }
            },
            "transform.angle",
        );
        for (channel, what) in [
            "background.color.r",
            "background.color.g",
            "background.color.b",
            "background.color.a",
        ]
        .into_iter()
        .enumerate()
        {
            refused(
                &move |composition| {
                    if let NodeKind::Background(background) = &mut composition.nodes[1].kind {
                        let (_, setting) = background
                            .settings_mut()
                            .into_iter()
                            .nth(channel)
                            .expect("a colour has four channels");
                        *setting = Param::Fixed(1.5);
                    }
                },
                what,
            );
        }
        // The ends of each range are within it.
        let mut ends = merged();
        if let NodeKind::Merge(merge) = &mut ends.nodes[2].kind {
            merge.blend = Param::Fixed(0.0);
            merge.center.x = Param::Fixed(-15.5);
            merge.center.y = Param::Fixed(16.5);
            merge.size = Param::Fixed(0.0);
            merge.angle = keys(&[(0, -3600.0), (50, 3600.0)]);
        }
        assert_eq!(ends.check(), Ok(()));
    }

    #[test]
    fn the_settings_that_take_keys_are_named_by_their_paths() {
        let paths = |kind: Kind| NodeKind::new(kind).settings();
        assert_eq!(paths(Kind::MediaIn), [] as [&str; 0]);
        assert_eq!(paths(Kind::MediaOut), [] as [&str; 0]);
        assert_eq!(
            paths(Kind::Merge),
            [
                "subtractive_additive",
                "alpha_gain",
                "burn_in",
                "blend",
                "center.x",
                "center.y",
                "size",
                "angle"
            ]
        );
        assert_eq!(
            paths(Kind::Transform),
            ["center.x", "center.y", "size", "angle"]
        );
        assert_eq!(
            paths(Kind::Background),
            ["color.r", "color.g", "color.b", "color.a"]
        );
        let mut merge = NodeKind::new(Kind::Merge);
        *merge.setting_mut("center.y").unwrap() = keys(&[(0, 0.25), (10, 0.75)]);
        assert!(merge.is_animated());
        assert_eq!(
            merge.params()["center"]["y"]["keys"][1],
            json!({"frame": 10, "value": 0.75, "interp": "linear"})
        );
        // What is no number takes no keys, and a kind has its own settings only.
        assert!(merge.setting_mut("apply").is_none());
        assert!(merge.setting_mut("center").is_none());
        assert!(merge.setting_mut("color.r").is_none());
        assert!(NodeKind::new(Kind::Transform)
            .setting_mut("flip_horizontal")
            .is_none());
        assert!(NodeKind::new(Kind::MediaIn).setting_mut("size").is_none());
    }

    #[test]
    fn keys_are_moved_with_the_frames_that_they_count() {
        let mut composition = merged();
        if let NodeKind::Transform(transform) = &mut composition.nodes[3].kind {
            transform.size = keys(&[(10, 1.0), (30, 2.0)]);
        }
        if let NodeKind::Background(background) = &mut composition.nodes[1].kind {
            background.color.a = keys(&[(0, 0.0), (20, 1.0)]);
        }
        assert!(composition.is_animated());
        let later = composition.with_key_frames(&|frame| frame + 5);
        let frames = |param: &Param| match param {
            Param::Keys(keys) => keys.iter().map(|k| k.frame).collect::<Vec<_>>(),
            Param::Fixed(_) => vec![],
        };
        let NodeKind::Transform(transform) = &later.nodes[3].kind else {
            panic!("the fourth node is the transform")
        };
        assert_eq!(frames(&transform.size), [15, 35]);
        assert_eq!(transform.angle, Param::Fixed(0.0));
        let NodeKind::Background(background) = &later.nodes[1].kind else {
            panic!("the second node is the background")
        };
        assert_eq!(frames(&background.color.a), [5, 25]);
        // Nothing else of it is changed, and it shows later what it showed.
        assert_eq!(later.links, composition.links);
        assert_eq!(later.at(25.0), composition.at(20.0));
        assert_eq!(later.check(), Ok(()));
    }

    #[test]
    fn a_composition_at_a_frame_lists_what_its_media_out_reads_each_after_its_inputs() {
        let mut composition = merged();
        // A node that nothing reads, and one that only it reads.
        composition.nodes.push(Node::new("spare", Kind::Transform));
        composition
            .nodes
            .push(Node::new("unseen", Kind::Background));
        composition
            .links
            .push(Link::new("unseen", "spare", Input::Input));
        composition.sort_links();
        if let NodeKind::Merge(merge) = &mut composition.nodes[2].kind {
            merge.blend = keys(&[(0, 0.0), (10, 1.0)]);
            merge.apply = Composite::Screen;
        }
        if let NodeKind::Transform(transform) = &mut composition.nodes[3].kind {
            transform.center.x = keys(&[(0, 0.5), (10, 1.5)]);
            transform.flip_vertical = true;
        }
        assert_eq!(composition.check(), Ok(()));
        let fused = composition.at(5.0);
        let ids: Vec<&str> = fused.nodes.iter().map(|node| node.id.as_str()).collect();
        assert_eq!(ids, ["bg", "in", "over", "move"]);
        assert_eq!(fused.output, Some(3));
        assert_eq!(
            fused.nodes[0].kind,
            FusedKind::Background {
                rgba: [0.0, 0.0, 0.0, 1.0]
            }
        );
        assert_eq!(fused.nodes[1].kind, FusedKind::MediaIn);
        assert_eq!(
            fused.nodes[2].kind,
            FusedKind::Merge {
                background: Some(0),
                foreground: Some(1),
                settings: Merged {
                    apply: Composite::Screen,
                    operator: Operator::Over,
                    subtractive_additive: 1.0,
                    alpha_gain: 1.0,
                    burn_in: 0.0,
                    blend: 0.5,
                    center: [0.5, 0.5],
                    size: 1.0,
                    angle: 0.0,
                },
            }
        );
        assert_eq!(
            fused.nodes[3].kind,
            FusedKind::Transform {
                input: Some(2),
                settings: Transformed {
                    center: [1.0, 0.5],
                    size: 1.0,
                    angle: 0.0,
                    flip_horizontal: false,
                    flip_vertical: true,
                },
            }
        );
        // A node that two inputs read is there once.
        let mut twice = merged();
        twice.links.retain(|link| link.from != "bg");
        twice.links.push(Link::new("in", "over", Input::Background));
        twice.sort_links();
        let fused = twice.at(0.0);
        let ids: Vec<&str> = fused.nodes.iter().map(|node| node.id.as_str()).collect();
        assert_eq!(ids, ["in", "over", "move"]);
        assert!(matches!(
            fused.nodes[1].kind,
            FusedKind::Merge {
                background: Some(0),
                foreground: Some(0),
                ..
            }
        ));
        // An input that is not linked reads no picture.
        let mut bare = merged();
        bare.links.retain(|link| link.from != "in");
        let fused = bare.at(0.0);
        assert!(matches!(
            fused.nodes[1].kind,
            FusedKind::Merge {
                background: Some(0),
                foreground: None,
                ..
            }
        ));
    }

    #[test]
    fn a_node_that_is_passed_through_hands_on_its_first_input() {
        // The merge passed through: its background, and nothing of its foreground.
        let mut composition = merged();
        composition.nodes[2].pass_through = true;
        let fused = composition.at(0.0);
        let ids: Vec<&str> = fused.nodes.iter().map(|node| node.id.as_str()).collect();
        assert_eq!(ids, ["bg", "move"]);
        assert!(matches!(
            fused.nodes[1].kind,
            FusedKind::Transform { input: Some(0), .. }
        ));
        // The transform too: the MediaOut shows the background as it is.
        composition.nodes[3].pass_through = true;
        let fused = composition.at(0.0);
        assert_eq!(fused.nodes.len(), 1);
        assert_eq!((fused.nodes[0].id.as_str(), fused.output), ("bg", Some(0)));
        // A node with no input that is passed through gives no picture.
        composition.nodes[1].pass_through = true;
        assert_eq!(composition.at(0.0), Fused::default());
        // A MediaOut that is passed through shows what it showed.
        let mut out = merged();
        out.nodes[4].pass_through = true;
        assert_eq!(out.at(0.0), merged().at(0.0));
        // And a node passed through whose first input is not linked gives none.
        let mut bare = merged();
        bare.nodes[2].pass_through = true;
        bare.links.retain(|link| link.from != "bg");
        let fused = bare.at(0.0);
        assert_eq!(fused.nodes.len(), 1);
        assert!(matches!(
            fused.nodes[0].kind,
            FusedKind::Transform { input: None, .. }
        ));
    }

    #[test]
    fn a_composition_that_was_not_checked_is_not_followed_round_a_loop() {
        let mut round = merged();
        round.links.retain(|link| link.from != "bg");
        round
            .links
            .push(Link::new("move", "over", Input::Background));
        round.sort_links();
        let fused = round.at(0.0);
        assert_eq!(fused.output, Some(fused.nodes.len() - 1));
        // Passed through all the way round, it gives nothing.
        for node in &mut round.nodes {
            node.pass_through = true;
        }
        assert_eq!(round.at(0.0), Fused::default());
    }

    #[test]
    fn the_pictures_of_a_composition_are_of_the_size_of_the_source_of_its_clip() {
        let mut project = Project::sample();
        let timeline = project.timelines[0].clone();
        let clip = timeline.tracks[0].clips[0].clone();
        project.media[0].facts.video.as_mut().unwrap().width = 1280;
        project.media[0].facts.video.as_mut().unwrap().height = 720;
        assert_eq!(size_of(&clip, &project, &timeline), (1280, 720));
        // Media that is turned a quarter is as high as it is stored wide.
        for (rotation, size) in [(90, (720, 1280)), (180, (1280, 720)), (270, (720, 1280))] {
            project.media[0].facts.video.as_mut().unwrap().rotation = rotation;
            assert_eq!(size_of(&clip, &project, &timeline), size, "{rotation}");
        }
        // A title and a colour are of the size of the timeline.
        let colour = Clip::new(
            "k1",
            Source::Color {
                color: crate::project::Rgba([0, 0, 0, 255]),
            },
            0,
            10,
        );
        assert_eq!(size_of(&colour, &project, &timeline), (1920, 1080));
    }
}
