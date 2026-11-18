//! The Text+ of a composition (Reference Manual 20, Chapter 104, pp. 2311–2331), its everyday
//! part: lines of text in a font, laid out about a place and turned there, its characters
//! written on, each unit of it moved about its own middle, each character followed one after
//! another by Fusion's Follower, and what is drawn shaded by a fill, an outline and a border
//! ([`TextPlus`]). What it is in a composition is told with the composition (Text+, in the
//! module above); how it is drawn, with the compositor.
//!
//! # Units
//!
//! Its place is a [`Center`], as those of the other nodes are. The size of its letters, the
//! offsets of its units and of its follower and how far its border reaches are shares of the
//! height of the picture, so that a Text+ is the same share of a picture of any width, `y` up as
//! a place counts it; the thickness of its outline is a share of the em of its letters, and its
//! tracking a number of ems. An angle is in degrees counter-clockwise, as that of a Transform.
//!
//! # Characters
//!
//! The characters of a Text+ are the Unicode scalars of its text but its line breaks (`\n`),
//! counted from 0 in the order of the text. Write On counts them so ([`written`]), and so does
//! the follower ([`follower_rank`], [`follower_delays`]).

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};

use super::{bounds, Center, Color};
use crate::grade::{merge, Bounds, Param};
use crate::ops::OpError;

/// The most characters of the text of a [`TextPlus`], its line breaks with them. Ours: the
/// manual gives none.
pub const MOST_CHARACTERS: usize = 4096;

/// The most bytes of the name of the font of a [`TextPlus`]. Ours.
pub const LONGEST_FONT: usize = 256;

/// How each line of a [`TextPlus`] lies across about its centre (Horizontal Justification): its
/// left end, its middle or its right end there. It is written `"left"`, `"center"` or
/// `"right"`, and takes no keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextJustify {
    /// Its left end at the centre.
    Left,
    /// Its middle at the centre. Every Text+ has it at first.
    #[default]
    Center,
    /// Its right end at the centre.
    Right,
}

/// How the block of the lines of a [`TextPlus`] lies up and down about its centre (Vertical
/// Justification): its top, its middle or its bottom there. It is written `"top"`, `"center"` or
/// `"bottom"`, and takes no keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextVertical {
    /// Its top at the centre.
    Top,
    /// Its middle at the centre. Every Text+ has it at first.
    #[default]
    Center,
    /// Its bottom at the centre.
    Bottom,
}

/// What a unit of the transform of a [`TextPlus`] is (Select Transform): each character, each
/// word (the characters between whitespace) or each line. It is written `"characters"`,
/// `"words"` or `"lines"`, and takes no keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextSelect {
    /// Each character. Every Text+ has it at first.
    #[default]
    Characters,
    /// Each word.
    Words,
    /// Each line.
    Lines,
}

/// In what order the follower of a [`TextPlus`] takes its characters (Order): the rank of each
/// ([`follower_rank`]). It is written `"left_to_right"`, `"right_to_left"`, `"inside_out"` or
/// `"outside_in"`, and takes no keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FollowerOrder {
    /// The first character first. Every follower has it at first.
    #[default]
    LeftToRight,
    /// The last character first.
    RightToLeft,
    /// The middle first, and the two ends last.
    InsideOut,
    /// The two ends first, and the middle last.
    OutsideIn,
}

/// How the delay of the follower of a [`TextPlus`] is counted (Delay Type): the delay for each
/// rank, or the delay shared out from the first rank to the last. It is written `"each"` or
/// `"first_to_last"`, and takes no keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FollowerDelay {
    /// Each character is the delay after the one before it in the order. Every follower has it
    /// at first.
    #[default]
    Each,
    /// The last character in the order is the delay after the first, and those between are
    /// spread evenly.
    FirstToLast,
}

/// Write On of a [`TextPlus`]: the shares of its characters from which and up to which they are
/// drawn ([`written`]). Each is a number or keys; either may be left out, and is then as it is
/// at first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WriteOn {
    /// Where the characters drawn start: 0 at first.
    pub start: Param,
    /// Where they end: 1 at first, all of them drawn.
    pub end: Param,
}

impl Default for WriteOn {
    fn default() -> Self {
        Self {
            start: bounds::TEXT_WRITE_START.param(),
            end: bounds::TEXT_WRITE_END.param(),
        }
    }
}

/// A shift of a unit or of a character of a [`TextPlus`]: `x` to the right and `y` up, each in
/// heights of the picture, a number or keys. Either may be left out, and is then none.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TextShift {
    /// How far right.
    pub x: Param,
    /// How far up.
    pub y: Param,
}

impl Default for TextShift {
    fn default() -> Self {
        Self {
            x: bounds::TEXT_OFFSET.param(),
            y: bounds::TEXT_OFFSET.param(),
        }
    }
}

/// The transform of the units of a [`TextPlus`] (its Transform tab): each unit, as `select`
/// says, made `scale` times as large and turned `angle` degrees counter-clockwise about its own
/// middle, the middle of the box of its glyphs as they are laid out, and moved by `offset`. Any
/// of them may be left out, and is then as it is at first, which changes nothing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UnitTransform {
    /// What a unit is.
    pub select: TextSelect,
    /// How far each unit is moved.
    pub offset: TextShift,
    /// How far each unit is turned about its middle, in degrees counter-clockwise.
    pub angle: Param,
    /// How many times as large each unit is made about its middle.
    pub scale: Param,
}

impl Default for UnitTransform {
    fn default() -> Self {
        Self {
            select: TextSelect::Characters,
            offset: TextShift::default(),
            angle: bounds::ANGLE.param(),
            scale: bounds::TEXT_SCALE.param(),
        }
    }
}

/// A colour of a [`TextPlus`] whose red, green and blue are `rgb` and whose alpha is `alpha` at
/// first.
fn colour(rgb: Bounds, alpha: Bounds) -> Color {
    Color {
        r: rgb.param(),
        g: rgb.param(),
        b: rgb.param(),
        a: alpha.param(),
    }
}

/// Read what is given as a patch of `first`, as `set_fusion_node` patches the settings of a
/// node: an object into an object, field by field, and anything else in the place of what is
/// there. So a colour or a centre of a Text+ may be given in part, the rest as it is at first.
fn over<'de, D, T>(deserializer: D, first: &T) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Serialize + DeserializeOwned,
{
    let given = serde_json::Value::deserialize(deserializer)?;
    let mut value = serde_json::to_value(first).map_err(<D::Error as serde::de::Error>::custom)?;
    merge(&mut value, &given);
    serde_json::from_value(value).map_err(<D::Error as serde::de::Error>::custom)
}

/// The centre of a [`TextPlus`] as it is read: the middle of the picture where a part of it is
/// left out.
fn middle<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Center, D::Error> {
    over(deserializer, &Center::middle())
}

/// The colour of a fill as it is read: white and opaque where a part of it is left out.
fn white<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Color, D::Error> {
    over(deserializer, &colour(bounds::TEXT_FILL, bounds::ALPHA))
}

/// The colour of an outline as it is read: black and opaque where a part of it is left out.
fn black<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Color, D::Error> {
    over(deserializer, &colour(bounds::COLOR, bounds::ALPHA))
}

/// The colour of a border as it is read: black, at an alpha of 0.6, where a part of it is left
/// out.
fn shaded<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Color, D::Error> {
    over(
        deserializer,
        &colour(bounds::COLOR, bounds::TEXT_BORDER_ALPHA),
    )
}

/// The fill of a [`TextPlus`], its first element of shading: the colour that its glyphs are
/// filled with, not premultiplied, white and opaque at first. A colour that is left out, or a
/// channel of it, is so.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TextFill {
    /// The colour.
    #[serde(deserialize_with = "white")]
    pub color: Color,
}

impl Default for TextFill {
    fn default() -> Self {
        Self {
            color: colour(bounds::TEXT_FILL, bounds::ALPHA),
        }
    }
}

/// The outline of a [`TextPlus`], its second element of shading: a line about the outside of its
/// glyphs, `thickness` ems of its letters wide, under the fill. A setting that is left out is as
/// it is at first: black and opaque, a fiftieth of an em.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TextOutline {
    /// The colour, not premultiplied.
    #[serde(deserialize_with = "black")]
    pub color: Color,
    /// How far beyond the edges of the glyphs it reaches, in ems.
    pub thickness: Param,
}

impl Default for TextOutline {
    fn default() -> Self {
        Self {
            color: colour(bounds::COLOR, bounds::ALPHA),
            thickness: bounds::TEXT_OUTLINE.param(),
        }
    }
}

/// The border of a [`TextPlus`], its third element of shading: a box behind the whole block of
/// its text, the bounds of its glyphs as they are laid out (before the transform of the units
/// and the follower), grown by `extend` on every side, its corners rounded by `round`, and
/// turned with the text about its centre. A setting that is left out is as it is at first:
/// black at an alpha of 0.6, a fiftieth of the height of the picture beyond the text, square.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TextBorder {
    /// The colour, not premultiplied.
    #[serde(deserialize_with = "shaded")]
    pub color: Color,
    /// How far beyond the bounds of the glyphs it reaches on every side, in heights of the
    /// picture.
    pub extend: Param,
    /// The radius of its corners, a share of half its shorter side.
    pub round: Param,
}

impl Default for TextBorder {
    fn default() -> Self {
        Self {
            color: colour(bounds::COLOR, bounds::TEXT_BORDER_ALPHA),
            extend: bounds::TEXT_BORDER_EXTEND.param(),
            round: bounds::TEXT_BORDER_ROUND.param(),
        }
    }
}

/// The follower of a [`TextPlus`]: Fusion's Follower, which reads its numbers for each
/// character at the frame less the delay of that character ([`TextFollower::at`]), so that the
/// characters do in turn what the keys of its numbers do. Its numbers are added to those of the
/// transform of the units (`offset`, `angle`) or multiply them (`scale`), and `opacity`
/// multiplies the alpha of the character in every element of shading, its outline too. A
/// setting that is left out is as it is at first, which changes nothing but for `delay`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TextFollower {
    /// The order in which it takes the characters.
    pub order: FollowerOrder,
    /// The delay, in frames of the keys of the clip, from 0 to 10000: 2 at first. It takes no
    /// keys.
    pub delay: f64,
    /// How the delay is counted.
    pub delay_type: FollowerDelay,
    /// What the alpha of each character is multiplied by, from 0 to 1.
    pub opacity: Param,
    /// How far each character is moved more.
    pub offset: TextShift,
    /// How far each character is turned more, in degrees counter-clockwise.
    pub angle: Param,
    /// What the size of each character is multiplied by.
    pub scale: Param,
}

impl Default for TextFollower {
    fn default() -> Self {
        Self {
            order: FollowerOrder::LeftToRight,
            delay: bounds::FOLLOWER_DELAY.default,
            delay_type: FollowerDelay::Each,
            opacity: bounds::FOLLOWER_OPACITY.param(),
            offset: TextShift::default(),
            angle: bounds::ANGLE.param(),
            scale: bounds::TEXT_SCALE.param(),
        }
    }
}

/// The settings of a Text+ (Reference Manual 20, Chapter 104, pp. 2311–2331), its everyday
/// part: see the section Text+ of [`crate::fusion`].
///
/// Every field may be left out where it is read, and is then as it is at first; one that it has
/// not is refused. `outline`, `border` and `follower` are none at first, and are written only
/// where there is one: a Text+ without one has no such element, and its settings are no
/// settings that take keys.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TextPlus {
    /// What it says: at most [`MOST_CHARACTERS`] characters, its lines broken by `\n`. Empty at
    /// first. It takes no keys.
    pub text: String,
    /// The family of its font, at most [`LONGEST_FONT`] bytes; empty for the engine's own, as
    /// a title has it. It takes no keys.
    pub font: String,
    /// The height of the em of its letters, in heights of the picture.
    pub size: Param,
    /// Where it is laid out about. It may follow a Tracker, as the centre of a Merge may.
    #[serde(deserialize_with = "middle")]
    pub center: Center,
    /// How each line lies across about the centre.
    pub justify: TextJustify,
    /// How the block of the lines lies up and down about the centre.
    pub vertical: TextVertical,
    /// How far apart its lines are, in lines of its font.
    pub line_spacing: Param,
    /// The advance added between its characters, in ems.
    pub tracking: Param,
    /// How far all of it is turned about the centre, in degrees counter-clockwise.
    pub angle: Param,
    /// Which of its characters are drawn.
    pub write_on: WriteOn,
    /// How each unit of it is moved.
    pub transform: UnitTransform,
    /// The fill of its glyphs.
    pub fill: TextFill,
    /// The outline of its glyphs, or none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline: Option<TextOutline>,
    /// The box behind it, or none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border: Option<TextBorder>,
    /// The follower of its characters, or none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub follower: Option<TextFollower>,
}

impl Default for TextPlus {
    /// No text, in the engine's own font, its em 0.08 of the height of the picture, in the
    /// middle of the picture, centred across and down; white and opaque, with no outline, no
    /// border and no follower; nothing moved.
    fn default() -> Self {
        Self {
            text: String::new(),
            font: String::new(),
            size: bounds::TEXT_SIZE.param(),
            center: Center::middle(),
            justify: TextJustify::Center,
            vertical: TextVertical::Center,
            line_spacing: bounds::TEXT_LINE_SPACING.param(),
            tracking: bounds::TEXT_TRACKING.param(),
            angle: bounds::ANGLE.param(),
            write_on: WriteOn::default(),
            transform: UnitTransform::default(),
            fill: TextFill::default(),
            outline: None,
            border: None,
            follower: None,
        }
    }
}

/// How many characters `text` has: its Unicode scalars but its line breaks.
#[must_use]
pub fn characters(text: &str) -> usize {
    text.chars().filter(|character| *character != '\n').count()
}

/// Which characters of `count` Write On draws from `start` to `end`, shares of them: the first
/// is `floor(start · count + 0.5)` and the last `floor(end · count + 0.5)`, each held within 0
/// and `count`, and character `i` is drawn where `first ≤ i < last`. Where `start` is not below
/// `end` none is.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a count of some thousands of characters, and a place among them"
)]
pub fn written(start: f64, end: f64, count: usize) -> [usize; 2] {
    let whole = count as f64;
    let place = |share: f64| {
        let at = (share * whole + 0.5).floor();
        if at >= whole {
            count
        } else if at > 0.0 {
            at as usize
        } else {
            0
        }
    };
    [place(start), place(end)]
}

/// The rank of character `index` of `count` in the order of a follower: `index` from the left;
/// `count − 1 − index` from the right; inside out, `floor(|index − (count − 1) / 2|)`, its
/// distance from the middle; and outside in, the greatest of those less its own.
#[must_use]
pub fn follower_rank(order: FollowerOrder, index: usize, count: usize) -> usize {
    let last = count.saturating_sub(1);
    let from_middle = index.saturating_mul(2).abs_diff(last) / 2;
    match order {
        FollowerOrder::LeftToRight => index,
        FollowerOrder::RightToLeft => last.saturating_sub(index),
        FollowerOrder::InsideOut => from_middle,
        FollowerOrder::OutsideIn => (last / 2).saturating_sub(from_middle),
    }
}

/// The greatest rank of `count` characters in an order.
fn greatest_rank(order: FollowerOrder, count: usize) -> usize {
    let last = count.saturating_sub(1);
    match order {
        FollowerOrder::LeftToRight | FollowerOrder::RightToLeft => last,
        FollowerOrder::InsideOut | FollowerOrder::OutsideIn => last / 2,
    }
}

/// The delay of each of `count` characters, in frames, by the order, the delay and the delay
/// type of a follower: of each, its rank times `delay`; from the first to the last, its rank
/// times `delay` over the greatest rank, and 0 where that is 0.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    reason = "ranks of some thousands of characters"
)]
pub fn follower_delays(
    order: FollowerOrder,
    delay: f64,
    delay_type: FollowerDelay,
    count: usize,
) -> Vec<f64> {
    let most = greatest_rank(order, count);
    (0..count)
        .map(|index| {
            let rank = follower_rank(order, index, count) as f64;
            match delay_type {
                FollowerDelay::Each => rank * delay,
                FollowerDelay::FirstToLast if most == 0 => 0.0,
                FollowerDelay::FirstToLast => rank * delay / most as f64,
            }
        })
        .collect()
}

impl TextFollower {
    /// Its numbers for each of `count` characters at `frame` of the keys of the clip: each read
    /// at the frame less the delay of the character ([`follower_delays`]).
    #[must_use]
    pub fn at(&self, frame: f64, count: usize) -> Vec<Followed> {
        follower_delays(self.order, self.delay, self.delay_type, count)
            .into_iter()
            .map(|delay| {
                let at = frame - delay;
                Followed {
                    opacity: self.opacity.value_at(at),
                    offset: [self.offset.x.value_at(at), self.offset.y.value_at(at)],
                    angle: self.angle.value_at(at),
                    scale: self.scale.value_at(at),
                }
            })
            .collect()
    }
}

/// A colour of a [`TextPlus`] at a frame of its keys: red, green, blue and alpha, not
/// premultiplied.
fn colour_at(color: &Color, frame: f64) -> [f64; 4] {
    [&color.r, &color.g, &color.b, &color.a].map(|channel| channel.value_at(frame))
}

fn refused(what: &'static str) -> OpError {
    OpError::OutOfRange { what }
}

impl TextPlus {
    /// Each of its settings that may take keys: its path in its `params`, the name that it is
    /// refused by, and its bounds, in the order of its `params`. Those of `outline`, `border`
    /// and `follower` are its settings only where it has them.
    pub const SETTINGS: [(&'static str, &'static str, Bounds); 32] = [
        ("size", "text_plus.size", bounds::TEXT_SIZE),
        ("center.x", "text_plus.center.x", bounds::CENTER),
        ("center.y", "text_plus.center.y", bounds::CENTER),
        (
            "line_spacing",
            "text_plus.line_spacing",
            bounds::TEXT_LINE_SPACING,
        ),
        ("tracking", "text_plus.tracking", bounds::TEXT_TRACKING),
        ("angle", "text_plus.angle", bounds::ANGLE),
        (
            "write_on.start",
            "text_plus.write_on.start",
            bounds::TEXT_WRITE_START,
        ),
        (
            "write_on.end",
            "text_plus.write_on.end",
            bounds::TEXT_WRITE_END,
        ),
        (
            "transform.offset.x",
            "text_plus.transform.offset.x",
            bounds::TEXT_OFFSET,
        ),
        (
            "transform.offset.y",
            "text_plus.transform.offset.y",
            bounds::TEXT_OFFSET,
        ),
        (
            "transform.angle",
            "text_plus.transform.angle",
            bounds::ANGLE,
        ),
        (
            "transform.scale",
            "text_plus.transform.scale",
            bounds::TEXT_SCALE,
        ),
        ("fill.color.r", "text_plus.fill.color.r", bounds::TEXT_FILL),
        ("fill.color.g", "text_plus.fill.color.g", bounds::TEXT_FILL),
        ("fill.color.b", "text_plus.fill.color.b", bounds::TEXT_FILL),
        ("fill.color.a", "text_plus.fill.color.a", bounds::ALPHA),
        (
            "outline.color.r",
            "text_plus.outline.color.r",
            bounds::COLOR,
        ),
        (
            "outline.color.g",
            "text_plus.outline.color.g",
            bounds::COLOR,
        ),
        (
            "outline.color.b",
            "text_plus.outline.color.b",
            bounds::COLOR,
        ),
        (
            "outline.color.a",
            "text_plus.outline.color.a",
            bounds::ALPHA,
        ),
        (
            "outline.thickness",
            "text_plus.outline.thickness",
            bounds::TEXT_OUTLINE,
        ),
        ("border.color.r", "text_plus.border.color.r", bounds::COLOR),
        ("border.color.g", "text_plus.border.color.g", bounds::COLOR),
        ("border.color.b", "text_plus.border.color.b", bounds::COLOR),
        (
            "border.color.a",
            "text_plus.border.color.a",
            bounds::TEXT_BORDER_ALPHA,
        ),
        (
            "border.extend",
            "text_plus.border.extend",
            bounds::TEXT_BORDER_EXTEND,
        ),
        (
            "border.round",
            "text_plus.border.round",
            bounds::TEXT_BORDER_ROUND,
        ),
        (
            "follower.opacity",
            "text_plus.follower.opacity",
            bounds::FOLLOWER_OPACITY,
        ),
        (
            "follower.offset.x",
            "text_plus.follower.offset.x",
            bounds::TEXT_OFFSET,
        ),
        (
            "follower.offset.y",
            "text_plus.follower.offset.y",
            bounds::TEXT_OFFSET,
        ),
        ("follower.angle", "text_plus.follower.angle", bounds::ANGLE),
        (
            "follower.scale",
            "text_plus.follower.scale",
            bounds::TEXT_SCALE,
        ),
    ];

    /// Its settings at a frame of its keys: its follower's for each of its characters, each at
    /// the frame less its delay.
    #[must_use]
    pub fn at(&self, frame: f64) -> TextPlused {
        let count = characters(&self.text);
        let transform = &self.transform;
        TextPlused {
            text: self.text.clone(),
            font: self.font.clone(),
            size: self.size.value_at(frame),
            center: self.center.at(frame),
            justify: self.justify,
            vertical: self.vertical,
            line_spacing: self.line_spacing.value_at(frame),
            tracking: self.tracking.value_at(frame),
            angle: self.angle.value_at(frame),
            written: written(
                self.write_on.start.value_at(frame),
                self.write_on.end.value_at(frame),
                count,
            ),
            transform: UnitTransformed {
                select: transform.select,
                offset: [
                    transform.offset.x.value_at(frame),
                    transform.offset.y.value_at(frame),
                ],
                angle: transform.angle.value_at(frame),
                scale: transform.scale.value_at(frame),
            },
            fill: colour_at(&self.fill.color, frame),
            outline: self.outline.as_ref().map(|outline| Outlined {
                color: colour_at(&outline.color, frame),
                thickness: outline.thickness.value_at(frame),
            }),
            border: self.border.as_ref().map(|border| Bordered {
                color: colour_at(&border.color, frame),
                extend: border.extend.value_at(frame),
                round: border.round.value_at(frame),
            }),
            followed: self
                .follower
                .as_ref()
                .map_or_else(Vec::new, |follower| follower.at(frame, count)),
        }
    }

    /// Check it: its text of at most [`MOST_CHARACTERS`] characters, the name of its font of at
    /// most [`LONGEST_FONT`] bytes, each setting within its bounds, as a number or at each key,
    /// what its centre follows, and the delay of its follower within its bounds.
    pub(crate) fn check(&self) -> Result<(), OpError> {
        if self.text.chars().count() > MOST_CHARACTERS {
            return Err(refused("text_plus.text"));
        }
        if self.font.len() > LONGEST_FONT {
            return Err(refused("text_plus.font"));
        }
        for (path, setting) in self.settings() {
            let listed = Self::SETTINGS.iter().find(|(name, ..)| *name == path);
            if let Some(&(_, what, range)) = listed {
                range.check(what, setting)?;
            }
        }
        let center = ["text_plus.center.x", "text_plus.center.y"];
        self.center.check_within(bounds::CENTER, center)?;
        if let Some(follower) = &self.follower {
            let range = bounds::FOLLOWER_DELAY;
            let delay = follower.delay;
            if !(delay.is_finite() && (range.low..=range.high).contains(&delay)) {
                return Err(refused("text_plus.follower.delay"));
            }
        }
        Ok(())
    }

    /// Its settings that take keys, each with its path in its `params`, in their order: those of
    /// its outline, its border and its follower where it has them.
    pub(super) fn settings(&self) -> Vec<(&'static str, &Param)> {
        let fill = &self.fill.color;
        let mut all = vec![
            ("size", &self.size),
            ("center.x", &self.center.x),
            ("center.y", &self.center.y),
            ("line_spacing", &self.line_spacing),
            ("tracking", &self.tracking),
            ("angle", &self.angle),
            ("write_on.start", &self.write_on.start),
            ("write_on.end", &self.write_on.end),
            ("transform.offset.x", &self.transform.offset.x),
            ("transform.offset.y", &self.transform.offset.y),
            ("transform.angle", &self.transform.angle),
            ("transform.scale", &self.transform.scale),
            ("fill.color.r", &fill.r),
            ("fill.color.g", &fill.g),
            ("fill.color.b", &fill.b),
            ("fill.color.a", &fill.a),
        ];
        if let Some(outline) = &self.outline {
            all.extend([
                ("outline.color.r", &outline.color.r),
                ("outline.color.g", &outline.color.g),
                ("outline.color.b", &outline.color.b),
                ("outline.color.a", &outline.color.a),
                ("outline.thickness", &outline.thickness),
            ]);
        }
        if let Some(border) = &self.border {
            all.extend([
                ("border.color.r", &border.color.r),
                ("border.color.g", &border.color.g),
                ("border.color.b", &border.color.b),
                ("border.color.a", &border.color.a),
                ("border.extend", &border.extend),
                ("border.round", &border.round),
            ]);
        }
        if let Some(follower) = &self.follower {
            all.extend([
                ("follower.opacity", &follower.opacity),
                ("follower.offset.x", &follower.offset.x),
                ("follower.offset.y", &follower.offset.y),
                ("follower.angle", &follower.angle),
                ("follower.scale", &follower.scale),
            ]);
        }
        all
    }

    /// Its settings that take keys, each with its path in its `params`, to be changed.
    pub(super) fn settings_mut(&mut self) -> Vec<(&'static str, &mut Param)> {
        let fill = &mut self.fill.color;
        let mut all = vec![
            ("size", &mut self.size),
            ("center.x", &mut self.center.x),
            ("center.y", &mut self.center.y),
            ("line_spacing", &mut self.line_spacing),
            ("tracking", &mut self.tracking),
            ("angle", &mut self.angle),
            ("write_on.start", &mut self.write_on.start),
            ("write_on.end", &mut self.write_on.end),
            ("transform.offset.x", &mut self.transform.offset.x),
            ("transform.offset.y", &mut self.transform.offset.y),
            ("transform.angle", &mut self.transform.angle),
            ("transform.scale", &mut self.transform.scale),
            ("fill.color.r", &mut fill.r),
            ("fill.color.g", &mut fill.g),
            ("fill.color.b", &mut fill.b),
            ("fill.color.a", &mut fill.a),
        ];
        if let Some(outline) = &mut self.outline {
            all.extend([
                ("outline.color.r", &mut outline.color.r),
                ("outline.color.g", &mut outline.color.g),
                ("outline.color.b", &mut outline.color.b),
                ("outline.color.a", &mut outline.color.a),
                ("outline.thickness", &mut outline.thickness),
            ]);
        }
        if let Some(border) = &mut self.border {
            all.extend([
                ("border.color.r", &mut border.color.r),
                ("border.color.g", &mut border.color.g),
                ("border.color.b", &mut border.color.b),
                ("border.color.a", &mut border.color.a),
                ("border.extend", &mut border.extend),
                ("border.round", &mut border.round),
            ]);
        }
        if let Some(follower) = &mut self.follower {
            all.extend([
                ("follower.opacity", &mut follower.opacity),
                ("follower.offset.x", &mut follower.offset.x),
                ("follower.offset.y", &mut follower.offset.y),
                ("follower.angle", &mut follower.angle),
                ("follower.scale", &mut follower.scale),
            ]);
        }
        all
    }
}

/// The numbers of the follower of a [`TextPlus`] for one character at one moment: each read at
/// the frame less the delay of the character.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Followed {
    /// What the alpha of the character is multiplied by.
    pub opacity: f64,
    /// How far it is moved more: across and up, in heights of the picture.
    pub offset: [f64; 2],
    /// How far it is turned more, in degrees counter-clockwise.
    pub angle: f64,
    /// What its size is multiplied by.
    pub scale: f64,
}

impl Followed {
    /// The numbers of a character that is not followed: they change nothing.
    pub const NONE: Followed = Followed {
        opacity: 1.0,
        offset: [0.0, 0.0],
        angle: 0.0,
        scale: 1.0,
    };
}

/// The transform of the units of a [`TextPlus`] at one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnitTransformed {
    /// What a unit is.
    pub select: TextSelect,
    /// How far each unit is moved: across and up, in heights of the picture.
    pub offset: [f64; 2],
    /// How far each unit is turned about its middle, in degrees counter-clockwise.
    pub angle: f64,
    /// How many times as large each unit is made about its middle.
    pub scale: f64,
}

/// The outline of a [`TextPlus`] at one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Outlined {
    /// Its colour: red, green, blue and alpha, not premultiplied.
    pub color: [f64; 4],
    /// How far beyond the edges of the glyphs it reaches, in ems.
    pub thickness: f64,
}

/// The border of a [`TextPlus`] at one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bordered {
    /// Its colour: red, green, blue and alpha, not premultiplied.
    pub color: [f64; 4],
    /// How far beyond the bounds of the glyphs it reaches, in heights of the picture.
    pub extend: f64,
    /// The radius of its corners, a share of half its shorter side.
    pub round: f64,
}

/// The settings of a [`TextPlus`] at one moment, each that takes keys as the number that it is
/// there: all that decides its picture.
#[derive(Clone, Debug, PartialEq)]
pub struct TextPlused {
    /// What it says.
    pub text: String,
    /// The family of its font; empty for the engine's own.
    pub font: String,
    /// The height of the em of its letters, in heights of the picture.
    pub size: f64,
    /// Where it is laid out about: `x` and `y`, as [`Center`] counts them.
    pub center: [f64; 2],
    /// How each line lies across about the centre.
    pub justify: TextJustify,
    /// How the block of the lines lies up and down about the centre.
    pub vertical: TextVertical,
    /// How far apart its lines are, in lines of its font.
    pub line_spacing: f64,
    /// The advance added between its characters, in ems.
    pub tracking: f64,
    /// How far all of it is turned about the centre, in degrees counter-clockwise.
    pub angle: f64,
    /// The characters that are drawn, by Write On ([`written`]): those from the first up to the
    /// second, none where the first is not below it.
    pub written: [usize; 2],
    /// How each unit of it is moved.
    pub transform: UnitTransformed,
    /// The colour of its fill: red, green, blue and alpha, not premultiplied.
    pub fill: [f64; 4],
    /// Its outline, or none.
    pub outline: Option<Outlined>,
    /// Its border, or none.
    pub border: Option<Bordered>,
    /// The numbers of its follower for each of its characters, in their order; none where it
    /// has no follower.
    pub followed: Vec<Followed>,
}

impl TextPlused {
    /// The numbers of the follower for character `index`: those that change nothing where it
    /// has no follower.
    #[must_use]
    pub fn followed_at(&self, index: usize) -> Followed {
        self.followed.get(index).copied().unwrap_or(Followed::NONE)
    }

    /// Whether Write On draws any of its characters, and its letters have a size: else its
    /// picture is transparent.
    #[must_use]
    pub fn writes_any(&self) -> bool {
        self.written[0] < self.written[1] && self.size > 0.0
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::float_cmp,
        reason = "delays and shares that floats hold exactly"
    )]

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

    #[test]
    fn a_text_plus_is_written_with_every_setting_but_the_elements_it_has_not() {
        let first = TextPlus::default();
        let written = serde_json::to_value(&first).unwrap();
        assert_eq!(
            written,
            json!({
                "text": "", "font": "", "size": 0.08, "center": {"x": 0.5, "y": 0.5},
                "justify": "center", "vertical": "center", "line_spacing": 1.0,
                "tracking": 0.0, "angle": 0.0, "write_on": {"start": 0.0, "end": 1.0},
                "transform": {"select": "characters", "offset": {"x": 0.0, "y": 0.0},
                              "angle": 0.0, "scale": 1.0},
                "fill": {"color": {"r": 1.0, "g": 1.0, "b": 1.0, "a": 1.0}},
            })
        );
        assert_eq!(first.check(), Ok(()));
        // Every field may be left out, and is then as it is at first.
        assert_eq!(
            serde_json::from_value::<TextPlus>(json!({})).unwrap(),
            first
        );
        let some: TextPlus = serde_json::from_value(json!({
            "text": "Hi", "write_on": {"end": 0.5}, "outline": {}, "follower": {"delay": 4},
            "border": {"round": 0.5},
        }))
        .unwrap();
        assert_eq!(some.text, "Hi");
        assert_eq!(some.write_on.start, Param::Fixed(0.0));
        assert_eq!(some.write_on.end, Param::Fixed(0.5));
        assert_eq!(some.outline, Some(TextOutline::default()));
        assert_eq!(some.follower.as_ref().map(|f| f.delay), Some(4.0));
        let border = some.border.clone().unwrap();
        assert_eq!(border.color.a, Param::Fixed(0.6));
        assert_eq!(border.round, Param::Fixed(0.5));
        // Written whole, the elements that it has with every setting of each, and read back.
        let mut whole = some.clone();
        whole.follower = Some(TextFollower {
            order: FollowerOrder::OutsideIn,
            delay: 4.0,
            delay_type: FollowerDelay::FirstToLast,
            opacity: keys(&[(0, 0.0), (10, 1.0)]),
            ..TextFollower::default()
        });
        let written = serde_json::to_value(&whole).unwrap();
        assert_eq!(
            written["outline"],
            json!({"color": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0}, "thickness": 0.02})
        );
        assert_eq!(written["follower"]["order"], "outside_in");
        assert_eq!(written["follower"]["delay_type"], "first_to_last");
        assert_eq!(written["follower"]["delay"], 4.0);
        assert_eq!(serde_json::from_value::<TextPlus>(written).unwrap(), whole);
        // A colour and a centre may be given in part, the rest as it is at first.
        let parts: TextPlus = serde_json::from_value(json!({
            "center": {"x": 0.25}, "fill": {"color": {"a": 0.5}},
            "border": {"color": {"r": 1}}, "outline": {"color": {"b": {"keys": [
                {"frame": 0, "value": 0, "interp": "linear"},
            ]}}},
        }))
        .unwrap();
        assert_eq!(
            (parts.center.x.clone(), parts.center.y.clone()),
            (Param::Fixed(0.25), Param::Fixed(0.5))
        );
        assert_eq!(parts.center.follow, None);
        assert_eq!(parts.fill.color.r, Param::Fixed(1.0));
        assert_eq!(parts.fill.color.a, Param::Fixed(0.5));
        let border = parts.border.clone().unwrap().color;
        assert_eq!([border.r, border.g], [Param::Fixed(1.0), Param::Fixed(0.0)]);
        assert_eq!(border.a, Param::Fixed(0.6));
        let outline = parts.outline.clone().unwrap().color;
        assert!(outline.b.is_animated());
        assert_eq!(outline.a, Param::Fixed(1.0));
        // A setting that it has not, a way that a menu has not, and a colour or a centre with a
        // part that they have not cannot be read; nor can an element given as a number.
        for wrong in [
            json!({"glyphs": 1}),
            json!({"justify": "full"}),
            json!({"vertical": "middle"}),
            json!({"transform": {"select": "paragraphs"}}),
            json!({"follower": {"order": "random"}}),
            json!({"follower": {"delay_type": "last"}}),
            json!({"follower": {"delay": {"keys": []}}}),
            json!({"fill": {"color": {"w": 1}}}),
            json!({"fill": {"color": 1}}),
            json!({"center": {"z": 0.5}}),
            json!({"center": null}),
            json!({"outline": 0.1}),
            json!({"text": 7}),
        ] {
            assert!(
                serde_json::from_value::<TextPlus>(wrong.clone()).is_err(),
                "{wrong}"
            );
        }
        // An element given null is none.
        let none: TextPlus = serde_json::from_value(json!({"outline": null})).unwrap();
        assert_eq!(none.outline, None);
    }

    #[test]
    fn write_on_draws_the_characters_from_its_start_up_to_its_end() {
        // floor(share · count + 0.5), held within 0 and the count.
        assert_eq!(written(0.0, 1.0, 10), [0, 10]);
        assert_eq!(written(0.0, 0.5, 10), [0, 5]);
        assert_eq!(written(0.25, 0.75, 10), [3, 8]);
        assert_eq!(written(0.04, 0.96, 10), [0, 10]);
        assert_eq!(written(0.05, 0.95, 10), [1, 10]);
        assert_eq!(written(0.5, 0.5, 10), [5, 5]);
        // A start beyond the end draws nothing, and is no refusal.
        assert_eq!(written(0.8, 0.2, 10), [8, 2]);
        assert_eq!(written(0.0, 1.0, 0), [0, 0]);
        assert_eq!(written(f64::NAN, 2.0, 3), [0, 3]);
        assert_eq!(written(-1.0, 1.0, 3), [0, 3]);
        // The line breaks are not characters.
        assert_eq!(characters("ab\ncd\n"), 4);
        assert_eq!(characters("标题 ab"), 5);
    }

    #[test]
    fn the_follower_ranks_the_characters_by_its_order() {
        let ranks = |order: FollowerOrder, count: usize| {
            (0..count)
                .map(|index| follower_rank(order, index, count))
                .collect::<Vec<_>>()
        };
        assert_eq!(ranks(FollowerOrder::LeftToRight, 5), [0, 1, 2, 3, 4]);
        assert_eq!(ranks(FollowerOrder::RightToLeft, 5), [4, 3, 2, 1, 0]);
        assert_eq!(ranks(FollowerOrder::InsideOut, 5), [2, 1, 0, 1, 2]);
        assert_eq!(ranks(FollowerOrder::OutsideIn, 5), [0, 1, 2, 1, 0]);
        // Of an even count, the two in the middle are first inside out.
        assert_eq!(ranks(FollowerOrder::InsideOut, 6), [2, 1, 0, 0, 1, 2]);
        assert_eq!(ranks(FollowerOrder::OutsideIn, 6), [0, 1, 2, 2, 1, 0]);
        assert_eq!(ranks(FollowerOrder::InsideOut, 1), [0]);
        assert_eq!(ranks(FollowerOrder::OutsideIn, 1), [0]);
        assert_eq!(ranks(FollowerOrder::RightToLeft, 0), [] as [usize; 0]);
    }

    #[test]
    fn the_follower_delays_each_rank_or_spreads_its_delay_from_first_to_last() {
        let each = FollowerDelay::Each;
        let spread = FollowerDelay::FirstToLast;
        assert_eq!(
            follower_delays(FollowerOrder::LeftToRight, 2.0, each, 4),
            [0.0, 2.0, 4.0, 6.0]
        );
        assert_eq!(
            follower_delays(FollowerOrder::RightToLeft, 2.0, each, 4),
            [6.0, 4.0, 2.0, 0.0]
        );
        assert_eq!(
            follower_delays(FollowerOrder::InsideOut, 3.0, each, 5),
            [6.0, 3.0, 0.0, 3.0, 6.0]
        );
        assert_eq!(
            follower_delays(FollowerOrder::OutsideIn, 3.0, each, 5),
            [0.0, 3.0, 6.0, 3.0, 0.0]
        );
        // From the first to the last: the greatest rank is the delay.
        assert_eq!(
            follower_delays(FollowerOrder::LeftToRight, 6.0, spread, 4),
            [0.0, 2.0, 4.0, 6.0]
        );
        assert_eq!(
            follower_delays(FollowerOrder::RightToLeft, 6.0, spread, 4),
            [6.0, 4.0, 2.0, 0.0]
        );
        assert_eq!(
            follower_delays(FollowerOrder::InsideOut, 8.0, spread, 5),
            [8.0, 4.0, 0.0, 4.0, 8.0]
        );
        assert_eq!(
            follower_delays(FollowerOrder::OutsideIn, 8.0, spread, 5),
            [0.0, 4.0, 8.0, 4.0, 0.0]
        );
        // One character, whose greatest rank is 0, is not delayed.
        assert_eq!(
            follower_delays(FollowerOrder::LeftToRight, 6.0, spread, 1),
            [0.0]
        );
        assert_eq!(
            follower_delays(FollowerOrder::InsideOut, 6.0, spread, 2),
            [0.0, 0.0]
        );
        assert_eq!(
            follower_delays(FollowerOrder::LeftToRight, 6.0, each, 0),
            [] as [f64; 0]
        );
    }

    #[test]
    fn each_character_reads_the_follower_at_the_frame_less_its_delay() {
        let follower = TextFollower {
            delay: 5.0,
            opacity: keys(&[(0, 0.0), (10, 1.0)]),
            scale: keys(&[(0, 2.0), (10, 1.0)]),
            ..TextFollower::default()
        };
        let text = TextPlus {
            text: "ab\nc".into(),
            follower: Some(follower),
            ..TextPlus::default()
        };
        let at = text.at(10.0);
        let opacities: Vec<f64> = at.followed.iter().map(|f| f.opacity).collect();
        assert_eq!(opacities, [1.0, 0.5, 0.0]);
        let scales: Vec<f64> = at.followed.iter().map(|f| f.scale).collect();
        assert_eq!(scales, [1.0, 1.5, 2.0]);
        assert_eq!(at.followed_at(1).offset, [0.0, 0.0]);
        // Beyond the characters, and without a follower, nothing is changed.
        assert_eq!(at.followed_at(3), Followed::NONE);
        let plain = TextPlus {
            text: "ab".into(),
            ..TextPlus::default()
        };
        assert_eq!(plain.at(3.0).followed, [] as [Followed; 0]);
        assert_eq!(plain.at(3.0).followed_at(0), Followed::NONE);
        assert!(plain.at(3.0).writes_any());
        assert!(!TextPlus::default().at(0.0).writes_any());
    }

    #[test]
    fn the_settings_that_take_keys_are_those_of_the_elements_that_it_has() {
        let paths = |text: &TextPlus| -> Vec<&'static str> {
            text.settings().into_iter().map(|(path, _)| path).collect()
        };
        let plain = TextPlus::default();
        let listed: Vec<&str> = TextPlus::SETTINGS.iter().map(|(path, ..)| *path).collect();
        assert_eq!(paths(&plain), listed[..16]);
        let mut whole = TextPlus {
            outline: Some(TextOutline::default()),
            border: Some(TextBorder::default()),
            follower: Some(TextFollower::default()),
            ..TextPlus::default()
        };
        assert_eq!(paths(&whole), listed);
        let mutable: Vec<&str> = whole.settings_mut().into_iter().map(|(p, _)| p).collect();
        assert_eq!(mutable, listed);
        // Each is refused out of its range by its name, and taken at its ends.
        for (path, what, range) in TextPlus::SETTINGS {
            for (value, taken) in [
                (range.low, true),
                (range.high, true),
                (range.low - 1e-6, false),
                (range.high + 1e-6, false),
                (f64::NAN, false),
            ] {
                let mut changed = whole.clone();
                for (name, setting) in changed.settings_mut() {
                    if name == path {
                        *setting = Param::Fixed(value);
                    }
                }
                let wanted = if taken {
                    Ok(())
                } else {
                    Err(OpError::OutOfRange { what })
                };
                assert_eq!(changed.check(), wanted, "{path} at {value}");
            }
        }
        // The text, the font and the delay of the follower, which take no keys.
        whole.text = "x".repeat(MOST_CHARACTERS);
        whole.font = "f".repeat(LONGEST_FONT);
        assert_eq!(whole.check(), Ok(()));
        whole.text.push('\n');
        assert_eq!(whole.check(), Err(refused("text_plus.text")));
        whole.text = "标".repeat(MOST_CHARACTERS);
        assert_eq!(whole.check(), Ok(()));
        whole.font.push('g');
        assert_eq!(whole.check(), Err(refused("text_plus.font")));
        whole.font = String::new();
        for (delay, taken) in [
            (0.0, true),
            (10_000.0, true),
            (-0.5, false),
            (10_000.5, false),
        ] {
            if let Some(follower) = &mut whole.follower {
                follower.delay = delay;
            }
            let wanted = if taken {
                Ok(())
            } else {
                Err(refused("text_plus.follower.delay"))
            };
            assert_eq!(whole.check(), wanted, "{delay}");
        }
    }
}
