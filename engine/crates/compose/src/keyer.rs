//! The mathematics of the keyer: what a pixel of a picture shot against a green or a blue screen
//! is made into, alone: a matte, and a picture ready to be laid over another. It is the one core
//! of the Delta Keyer of the Fusion page (Reference Manual of Resolve 20, pp. 2440–2448, and its
//! use pp. 1664–1668) and of the keyer of the Edit page (the 3D Keyer, pp. 3497–3502), which
//! differ in the controls that they show and not in what is done with them. It is written on the
//! processor as a shader computes it, in `f32` and a pixel at a time, and is the reference that
//! the shader of the card is to be held to.
//!
//! # Colours and mattes
//!
//! A colour is R, G and B of linear light. It may go beyond white and below black, and is not
//! held within them. A matte is 1 where the picture is kept (the subject) and 0 where it is the
//! screen: white is opaque and black transparent, as the manual shows a matte (p. 1666). It is
//! the alpha of the layer, and the way round of a key of the Color page, which is 1 where a node
//! works ([`crate::grade::maths::key`]). What is given is premultiplied by its alpha, as the
//! compositor lays its layers.
//!
//! # The controls
//!
//! - **Background Color** (p. 2443, [`Settings::screen`]): the colour of the screen, picked from
//!   the picture. Any colour that has one channel above its other two will do: the manual has
//!   it that the keyer is not for green and blue alone (p. 1664).
//! - **Gain** (p. 2443): more of it gives the screen colour more say, so that what is of that
//!   colour is more transparent.
//! - **Balance** (p. 2443): the keyer compares the dominant channel of the screen colour with
//!   the other two, and Balance says in what shares those two are taken: the lesser of them at
//!   0, the greater at 1, and half of each at 0.5.
//! - **Threshold** (p. 2445, [`Settings::low`] and [`Settings::high`]): a matte below the lower
//!   threshold is made transparent and one above the upper solid; the levels between them keep
//!   their places among themselves.
//! - **Spill Method** and **Spill Suppression** (p. 2446): None, Rare, Medium, Well Done and
//!   Burnt, each stronger than the one before, and how much of the method is done, none at 0.
//!   The Despill of the keyer of the Edit page (p. 3498) is Spill Suppression.
//! - **Replace Mode** and **Replace Color** (p. 2445): what the adjustments of the matte give
//!   back to the picture. Source is the one that the manual's own example chooses, to leave the
//!   spill to a later correction (p. 1668).
//! - **View Mode** (p. 2442, [`View`]): Pre Matte, Matte, Status and Final Result.
//!
//! # The formulas
//!
//! `DaVinci` does not publish its own. These are those of a colour difference keyer, which the
//! manual says the Delta Keyer is (p. 2440), made to behave as its pages describe. With `S` the
//! screen colour, `d` its dominant channel (its greatest: green before blue before red where two
//! are alike), and `least` and `most` the lesser and the greater of the other two channels of a
//! colour `x`:
//!
//! 1. **The colour difference** ([`difference`]): `D(x) = x[d] − (least + b (most − least))`,
//!    `b` the Balance. It is 0 for a grey, above 0 where the colour is towards the screen's, and
//!    below where it is away from it.
//! 2. **The pre-matte** ([`pre_matte`]), the matte of the Key tab: `1 − g D(x) / D(S)` held
//!    within 0 and 1, `g` the Gain. A pure green or blue screen has a `D(S)` of 1, and the
//!    formula is then the common one of such a screen; dividing by `D(S)` is what makes it serve
//!    a picked colour, which is keyed out whole, to the bit, whatever the Balance. A grey is kept
//!    whole. A screen in shadow is keyed in part, and more Gain keys it whole.
//! 3. **The matte** ([`thresholds`], [`matte`]): the pre-matte with its levels from the lower
//!    threshold to the upper made those from 0 to 1, as Black Clip and White Clip of Matte
//!    Finesse do ([`crate::grade::maths`]), line for line.
//! 4. **Spill** ([`spill`], [`despilled`]): the dominant channel is limited to
//!    `least + w (most − least)`, `w` 1 for Rare (the greater of the other two), 0.5 for Medium
//!    (their mean), 0.25 for Well Done and 0 for Burnt (the lesser). What is above the limit is
//!    the spill, and Spill Suppression times it is taken away. The other two channels are left.
//! 5. **The result** ([`result_with`]), of a pre-matte `p` and a matte `m`. The key takes the
//!    screen away: `y = x − (1 − p) S`, of each channel no more than is there (none of one below
//!    0). A pixel that is `a` of a grey over `1 − a` of the screen is so made `a` of the grey,
//!    which is the grey premultiplied. Then the spill of `y` is suppressed, which gives `kept`,
//!    and `removed = x − kept`. Where the matte was made more opaque than the key made it, the
//!    share `u = (m − p) / (1 − p)` of what was removed is filled as Replace Mode says: with
//!    nothing (None); with `u · removed`, the colour of the picture itself, so that a pixel made
//!    solid is the picture as it was, its spill with it (Source, p. 1668); with `(m − p) R`, `R`
//!    the Replace Color (Hard Color); or with `u w R`, `w` the removed share of the screen's
//!    dominant channel, `removed[d] / S[d]`, at most 1 (Soft Color, which the manual has weigh
//!    its colour by how much of the screen's was taken away). Where the matte was made less
//!    opaque, the colour is multiplied by `m / p`. The alpha is `m`.
//!
//! # What is not a colour
//!
//! A channel that is not a number is read as 0, and one that is infinite, or beyond the largest
//! value that a half float holds, as that value with its sign ([`sound`]). So every result is a
//! finite number, and a filter that reads a matte or a result reads nothing that spreads. A
//! pre-matte or a matte given that is not a number is read as 0.
//!
//! # What reads the pixels about a pixel
//!
//! The Delta Keyer's Pre-Blur, Erode/Dilate, Blur, Clean Foreground and Clean Background
//! (pp. 2443, 2445) are what Matte Finesse does to the key of a qualifier with its Pre-Filter,
//! Morph Operation, Blur Radius, Clean White and Clean Black (pp. 3178–3181; `clean`, `spread`
//! and `morph` of [`crate::grade::maths`], over the filters of [`crate::neighbourhood`]), and
//! are done with those, which are not written a second time ([`Finesse`], [`mattes`]):
//!
//! 1. **Pre-Blur**: [`pre_matte`] is given the picture through a Gaussian of that standard
//!    deviation, premultiplied as it is filtered and then not. The colours of [`result_with`]
//!    are those of the picture as it is.
//! 2. The pre-matte through the **thresholds** is the matte.
//! 3. **Erode/Dilate** ([`eroded`]): a pass of Morph Operation over a box of that radius, which
//!    shrinks the matte below 0 and grows it above.
//! 4. **Matte Blur** ([`softened`]): the matte through a Gaussian of that standard deviation,
//!    as Blur Radius with no In/Out Ratio.
//! 5. **Clean Foreground** and **Clean Background** ([`cleaned`]): Clean White and Clean Black,
//!    which fill the holes of what is solid and take away the specks of what is transparent,
//!    over the Gaussian that they have there.
//!
//! A length is in pixels of a picture 1080 high, as the reach of a filter is. Each of them at
//! 0 does nothing; and a keyer that keys nothing (a Gain of 0) reads no pixel about another,
//! its matte being one level all over, which they leave as it is.
//!
//! # The node of a composition
//!
//! The Delta Keyer node of the composition of a clip ([`columbina_model::fusion`]) keeps these
//! settings, by these names, in these units, within these ranges and with these values at
//! first ([`of_node`] gives those of a node at one moment). What it makes of a pixel
//! ([`node_pixel`]), which the shader of the compositor is held to:
//!
//! - **The picture** that comes in is premultiplied. Its colour is that not premultiplied, and
//!   black where its alpha is not above 0 ([`unpremultiplied`]); the final result is made of
//!   that colour and then multiplied, all four channels, by the alpha that came in, so that a
//!   picture that is seen through in part is keyed as what is seen of it, and its alpha is the
//!   matte times the alpha that it had. An opaque picture is keyed as the formulas above have
//!   it, to the bit. The values are keyed **as they are**, in the working space of the
//!   timeline: where the project is not colour managed they are coded with their gamma and are
//!   not of linear light, and nothing here makes them so.
//! - **The solid and garbage mattes** (p. 2440, [`met`]) change the matte after all of the
//!   above: first where the solid matte is 1 the matte is made 1, `m + (1 − m) s`; then where
//!   the garbage matte is 1 the matte is made 0, `m (1 − g)`. So the garbage matte wins where
//!   both are 1 (ours). Each is the alpha of a picture, within 0 and 1; the garbage matte may
//!   be read the other way round, `1 − g`; one that is not there changes nothing.
//! - **The view**: Pre Matte shows the pre-matte, Matte the matte before the solid and garbage
//!   mattes (p. 2442), Status and Final Result are of the matte after them. The mattes and
//!   the status are opaque pictures, whatever the alpha that came in.
//! - **The effect mask** mixes what the view shows with the pixel as it came in:
//!   `first + (own − first) · mask`, the pixel as it came in where the mask is 0 and what the
//!   view shows where it is 1, both to the bit.
//!
//! The tabs of the advanced tier are not kept: Pre Matte (a keyer of ranges of colour, with a
//! clean plate), Tuning, Restore Fringe, the gamma, size, shape and colour of the fringe, and
//! the colour balance reference.

#![allow(
    clippy::float_cmp,
    reason = "the maths of a shader: exact comparisons with neutral settings and ends of a matte"
)]

use columbina_model::fusion::{self as model, DeltaKeyed, FusedKind};

use crate::color::Filter;
use crate::grade::layout as at;
use crate::grade::maths::{self, Params, Rgb};
use crate::neighbourhood::{self, Image};

/// A colour and its alpha: R, G and B, premultiplied, and A.
pub type Rgba = [f32; 4];

/// The most Pre-Blur and the most Matte Blur, in pixels of a picture 1080 high. Ours: the
/// manual gives no range.
pub const MOST_BLUR: f32 = 100.0;

/// The most Erode/Dilate, either way, in pixels of a picture 1080 high. Ours.
pub const MOST_ERODE_DILATE: f32 = 100.0;

/// How much of the box of Erode/Dilate about a pixel is to be in the matte for the pixel to be
/// grown into it, or out of it for the pixel to be shrunk out of it: that of the Morph
/// Operation of a qualifier.
#[allow(
    clippy::cast_possible_truncation,
    reason = "a share of a box, which a float of 32 bits holds as the card reads it"
)]
pub const MORPH_EDGE: f32 = at::MORPH_EDGE as f32;

/// The largest value of a channel: the largest that a half float holds, which is what the canvas
/// is made of. What is beyond is read as this ([`sound`]).
pub const LARGEST: f32 = 65504.0;

/// The least by which the dominant channel of a screen colour is above its other two. Ours.
pub const LEAST_DOMINANCE: f32 = 1e-4;

/// The most Gain. Ours: the manual gives no range.
pub const MOST_GAIN: f32 = 10.0;

/// How far a matte is from its pre-matte before the Status view shows it as changed: more than
/// the textures that a matte goes through on its way round off. Ours.
pub const STATUS_TOLERANCE: f32 = 1.0 / 256.0;

/// The least that a matte is divided by.
const LEAST: f32 = 1e-6;

/// The strength of spill suppression (Spill Method, p. 2446), from the weakest: what the
/// dominant channel is limited to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SpillMethod {
    /// No spill suppression.
    None,
    /// The lightest, which takes little: the greater of the other two channels.
    Rare,
    /// The one that the manual gives for green screens: the mean of the other two channels.
    #[default]
    Medium,
    /// The one that it gives for blue screens: a quarter of the way from the lesser of the
    /// other two channels to the greater.
    WellDone,
    /// For the shots that give the most trouble: the lesser of the other two channels.
    Burnt,
}

impl SpillMethod {
    /// Every method, from the weakest.
    pub const ALL: [SpillMethod; 5] = [
        SpillMethod::None,
        SpillMethod::Rare,
        SpillMethod::Medium,
        SpillMethod::WellDone,
        SpillMethod::Burnt,
    ];

    /// The share of the greater of the other two channels in what the dominant channel is
    /// limited to, the rest being of the lesser. None has no limit, and is given that of Rare.
    #[must_use]
    pub fn weight(self) -> f32 {
        match self {
            SpillMethod::None | SpillMethod::Rare => 1.0,
            SpillMethod::Medium => 0.5,
            SpillMethod::WellDone => 0.25,
            SpillMethod::Burnt => 0.0,
        }
    }
}

/// What fills the part of a pixel that the key took away and the matte gave back (Replace Mode,
/// p. 2445).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Replace {
    /// Nothing: the adjustments of the matte leave the colour as it is.
    None,
    /// The colour of the picture itself: what was taken away.
    Source,
    /// Hard Color: the replace colour.
    HardColour,
    /// Soft Color: the replace colour, by how much of the screen's colour was taken away.
    #[default]
    SoftColour,
}

impl Replace {
    /// The number of the mode, as a shader is given it: 0 None, 1 Source, 2 Hard Color and 3
    /// Soft Color.
    #[must_use]
    pub fn code(self) -> u8 {
        match self {
            Replace::None => 0,
            Replace::Source => 1,
            Replace::HardColour => 2,
            Replace::SoftColour => 3,
        }
    }
}

/// What the viewer shows of a keyer (View Mode, p. 2442). Tuning Ranges and Intermediate Result
/// are not kept.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum View {
    /// The matte of the Key tab, before the Matte tab: white where the picture is kept. The
    /// manual's Pre Matte is the key of its Pre Matte tab, which is not kept; this is the stage
    /// of the key that is in its place here.
    PreMatte,
    /// The matte: white where the picture is kept.
    Matte,
    /// What is solid, what is transparent, what is in between, and what the adjustments of the
    /// matte changed ([`status`]).
    Status,
    /// The keyed picture with its spill suppressed, ready to be laid over a scene. It is what
    /// the menu shows at first, as the manual has it.
    #[default]
    FinalResult,
}

impl View {
    /// Every view, in the order of the menu.
    pub const ALL: [View; 4] = [View::PreMatte, View::Matte, View::Status, View::FinalResult];
}

/// The controls of a keyer (see the module).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Background Color: the colour of the screen, each channel from 0, one of them above the
    /// other two by [`LEAST_DOMINANCE`] or more.
    pub screen: Rgb,
    /// Gain, from 0 to [`MOST_GAIN`]: 0 keys nothing.
    pub gain: f32,
    /// Balance, from 0 (the lesser of the other two channels) to 1 (the greater).
    pub balance: f32,
    /// The lower threshold, from 0 to the upper: a matte at or below it is transparent.
    pub low: f32,
    /// The upper threshold, up to 1: a matte at or above it is solid (above it, where the two
    /// thresholds are alike).
    pub high: f32,
    /// Spill Method.
    pub spill_method: SpillMethod,
    /// Spill Suppression, from 0 (none) to 1 (all that the method takes).
    pub spill_suppression: f32,
    /// Replace Mode.
    pub replace: Replace,
    /// Replace Color, each channel from 0.
    pub replace_colour: Rgb,
}

impl Default for Settings {
    /// A pure green screen, a Gain of 1, a Balance of 0.5, thresholds of 0 and 1 (which leave
    /// the matte as the key made it), Medium (the method that the manual gives for green
    /// screens) done whole, and Soft Color with a mid grey. The manual gives the default of none
    /// of them: these are ours, to be checked against `DaVinci` itself.
    fn default() -> Self {
        Self {
            screen: [0.0, 1.0, 0.0],
            gain: 1.0,
            balance: 0.5,
            low: 0.0,
            high: 1.0,
            spill_method: SpillMethod::Medium,
            spill_suppression: 1.0,
            replace: Replace::SoftColour,
            replace_colour: [0.5; 3],
        }
    }
}

impl Settings {
    /// Check that each value is in its range, the thresholds are in order, and the screen colour
    /// has a channel above its other two.
    ///
    /// # Errors
    /// A value is not a number, or out of its range: what, said for a person.
    pub fn check(&self) -> Result<(), String> {
        let within = |name: &str, value: f32, low: f32, high: f32| {
            if value.is_finite() && (low..=high).contains(&value) {
                Ok(())
            } else {
                Err(format!("{name} is from {low} to {high}, not {value}"))
            }
        };
        for value in self.screen {
            within("a channel of screen", value, 0.0, LARGEST)?;
        }
        within("gain", self.gain, 0.0, MOST_GAIN)?;
        within("balance", self.balance, 0.0, 1.0)?;
        within("low", self.low, 0.0, 1.0)?;
        within("high", self.high, 0.0, 1.0)?;
        within("spill_suppression", self.spill_suppression, 0.0, 1.0)?;
        for value in self.replace_colour {
            within("a channel of replace_colour", value, 0.0, LARGEST)?;
        }
        if self.low > self.high {
            return Err(format!(
                "low is at most high ({}), not {}",
                self.high, self.low
            ));
        }
        if dominance(self.screen) < LEAST_DOMINANCE {
            return Err(format!(
                "screen has no channel above its other two by {LEAST_DOMINANCE}: {:?}",
                self.screen
            ));
        }
        Ok(())
    }
}

/// A keyer worked out of its settings: the numbers that the maths of a pixel reads, and that a
/// shader is given ([`Keyer::uniform`]). Made by [`Keyer::of`], which keeps each within what the
/// maths can take.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Keyer {
    /// The screen colour.
    pub screen: Rgb,
    /// The dominant channel of the screen colour: 0 red, 1 green, 2 blue.
    pub dominant: u8,
    /// Gain.
    pub gain: f32,
    /// The colour difference of the screen colour itself, [`LEAST_DOMINANCE`] or more.
    pub screen_difference: f32,
    /// Balance.
    pub balance: f32,
    /// The lower threshold.
    pub low: f32,
    /// The upper threshold.
    pub high: f32,
    /// The share of the greater of the other two channels in the limit of spill
    /// ([`SpillMethod::weight`]).
    pub spill_weight: f32,
    /// How much of the spill is taken away: Spill Suppression, or 0 where the method is None.
    pub spill: f32,
    /// Replace Mode.
    pub replace: Replace,
    /// Replace Color.
    pub replace_colour: Rgb,
}

impl Keyer {
    /// The keyer of `settings`. Settings that [`Settings::check`] passes are taken as they are.
    /// Of others, a number out of its range is brought to its nearest end, and what is not a
    /// number is read as 0; a screen colour with no channel far enough above its other two keys
    /// nothing.
    #[must_use]
    pub fn of(settings: &Settings) -> Keyer {
        let screen = settings.screen.map(|value| sound(value).max(0.0));
        let dominant = dominant_of(screen);
        let balance = held(settings.balance, 0.0, 1.0);
        let screen_difference = above(dominant, balance, screen);
        let keys = screen_difference >= LEAST_DOMINANCE;
        Keyer {
            screen,
            dominant,
            gain: if keys {
                held(settings.gain, 0.0, MOST_GAIN)
            } else {
                0.0
            },
            screen_difference: if keys { screen_difference } else { 1.0 },
            balance,
            low: held(settings.low, 0.0, 1.0),
            high: held(settings.high, 0.0, 1.0),
            spill_weight: settings.spill_method.weight(),
            spill: if settings.spill_method == SpillMethod::None {
                0.0
            } else {
                held(settings.spill_suppression, 0.0, 1.0)
            },
            replace: settings.replace,
            replace_colour: settings.replace_colour.map(|value| sound(value).max(0.0)),
        }
    }

    /// The numbers of the keyer as a shader is given them, four rows of four: the screen colour
    /// and its dominant channel; Gain, the colour difference of the screen colour, Balance and
    /// nothing; the lower and the upper threshold, the share of the greater channel in the limit
    /// of spill, and how much of the spill is taken away; the replace colour and the number of
    /// the replace mode ([`Replace::code`]).
    #[must_use]
    pub fn uniform(&self) -> [f32; 16] {
        [
            self.screen[0],
            self.screen[1],
            self.screen[2],
            f32::from(self.dominant),
            self.gain,
            self.screen_difference,
            self.balance,
            0.0,
            self.low,
            self.high,
            self.spill_weight,
            self.spill,
            self.replace_colour[0],
            self.replace_colour[1],
            self.replace_colour[2],
            f32::from(self.replace.code()),
        ]
    }
}

/// `value` as the maths reads a channel: 0 where it is not a number, and no further from 0 than
/// [`LARGEST`].
#[must_use]
pub fn sound(value: f32) -> f32 {
    if value.is_nan() {
        return 0.0;
    }
    value.clamp(-LARGEST, LARGEST)
}

/// `value` as the maths reads a matte: 0 where it is not a number, and within 0 and 1.
fn unit(value: f32) -> f32 {
    if value.is_nan() {
        return 0.0;
    }
    value.clamp(0.0, 1.0)
}

/// `value` of a setting, within `low` and `high`: 0 where it is not a number, before that.
fn held(value: f32, low: f32, high: f32) -> f32 {
    sound(value).clamp(low, high)
}

/// The dominant channel of a screen colour: its greatest, green before blue before red where two
/// are alike.
fn dominant_of(screen: Rgb) -> u8 {
    if screen[1] >= screen[0] && screen[1] >= screen[2] {
        1
    } else if screen[2] >= screen[0] {
        2
    } else {
        0
    }
}

/// The channel of `x` that is the dominant one of the screen, and the lesser and the greater of
/// its other two.
fn split(dominant: u8, x: Rgb) -> (f32, f32, f32) {
    let (main, first, second) = match dominant {
        0 => (x[0], x[1], x[2]),
        1 => (x[1], x[0], x[2]),
        _ => (x[2], x[0], x[1]),
    };
    (main, first.min(second), first.max(second))
}

/// `x` with `value` for the channel that is the dominant one of the screen.
fn put(dominant: u8, x: Rgb, value: f32) -> Rgb {
    match dominant {
        0 => [value, x[1], x[2]],
        1 => [x[0], value, x[2]],
        _ => [x[0], x[1], value],
    }
}

/// How far the dominant channel of `x` is above its other two, mixed: `weight` of the greater
/// and the rest of the lesser.
fn above(dominant: u8, weight: f32, x: Rgb) -> f32 {
    let (main, least, most) = split(dominant, x);
    main - (least + weight * (most - least))
}

/// How far the greatest channel of a screen colour is above the greater of its other two: the
/// least that its colour difference is, whatever the Balance. A screen colour is one where this
/// is [`LEAST_DOMINANCE`] or more.
#[must_use]
pub fn dominance(screen: Rgb) -> f32 {
    let screen = screen.map(sound);
    above(dominant_of(screen), 1.0, screen)
}

/// The colour difference of `x`: how far its channel that is the dominant one of the screen is
/// above its other two, mixed as Balance says (p. 2443). It is 0 for a grey.
#[must_use]
pub fn difference(k: &Keyer, x: Rgb) -> f32 {
    above(k.dominant, k.balance, x.map(sound))
}

/// The pre-matte of `x`, the matte of the Key tab (p. 2443): 1 less Gain times the colour
/// difference of `x` over that of the screen colour, within 0 and 1. With a Pre-Blur, `x` is the
/// picture blurred.
#[must_use]
pub fn pre_matte(k: &Keyer, x: Rgb) -> f32 {
    (1.0 - k.gain * (difference(k, x) / k.screen_difference)).clamp(0.0, 1.0)
}

/// A matte through the thresholds (p. 2445): its levels from the lower to the upper made those
/// from 0 to 1. Where they are alike, what is above them is solid and the rest transparent.
#[must_use]
pub fn thresholds(k: &Keyer, matte: f32) -> f32 {
    let matte = unit(matte);
    if k.low == 0.0 && k.high == 1.0 {
        return matte;
    }
    if k.high <= k.low {
        return if matte > k.low { 1.0 } else { 0.0 };
    }
    ((matte - k.low) / (k.high - k.low)).clamp(0.0, 1.0)
}

/// The matte of `x` alone: its pre-matte through the thresholds.
#[must_use]
pub fn matte(k: &Keyer, x: Rgb) -> f32 {
    thresholds(k, pre_matte(k, x))
}

/// `x` with its spill suppressed (see [`spill`]); `x` is sound.
fn suppress(k: &Keyer, x: Rgb) -> Rgb {
    let (main, _, _) = split(k.dominant, x);
    let over = above(k.dominant, k.spill_weight, x).max(0.0);
    put(k.dominant, x, main - k.spill * over)
}

/// The spill that is taken from `x` (p. 2446): how far its channel that is the dominant one of
/// the screen is above what the method limits it to, times Spill Suppression.
#[must_use]
pub fn spill(k: &Keyer, x: Rgb) -> f32 {
    k.spill * above(k.dominant, k.spill_weight, x.map(sound)).max(0.0)
}

/// `x` with its spill suppressed: its channel that is the dominant one of the screen less its
/// [`spill`], and its other two as they are.
#[must_use]
pub fn despilled(k: &Keyer, x: Rgb) -> Rgb {
    suppress(k, x.map(sound))
}

/// The final result of `x` (p. 2442), premultiplied: of the pre-matte `pre` that the key made of
/// it ([`pre_matte`]) and the matte `matte` that the adjustments of the matte made of that
/// ([`thresholds`], and whatever else was done to it). The screen is taken away by the pre-matte
/// and the spill suppressed; what the matte gave back is filled as Replace Mode says, and what
/// it took away is taken from the colour too. The alpha is the matte. See the module.
#[must_use]
pub fn result_with(k: &Keyer, x: Rgb, pre: f32, matte: f32) -> Rgba {
    let x = x.map(sound);
    let pre = unit(pre);
    let matte = unit(matte);
    // The screen that the key takes away: of each channel no more than is there.
    let gone = 1.0 - pre;
    let lifted = [0, 1, 2].map(|i| x[i] - (gone * k.screen[i]).clamp(0.0, x[i].max(0.0)));
    let kept = suppress(k, lifted);
    let removed = [0, 1, 2].map(|i| x[i] - kept[i]);
    // The share of what the key made transparent that the matte gave back, and the share of
    // what the key kept that the matte left.
    let back = ((matte - pre) / gone.max(LEAST)).clamp(0.0, 1.0);
    let left = (matte / pre.max(LEAST)).min(1.0);
    let fill = match k.replace {
        Replace::None => [0.0; 3],
        Replace::Source => removed,
        Replace::HardColour => k.replace_colour.map(|value| value * gone),
        Replace::SoftColour => {
            let (taken, _, _) = split(k.dominant, removed);
            let (whole, _, _) = split(k.dominant, k.screen);
            let share = (taken / whole.max(LEAST)).min(1.0);
            k.replace_colour.map(|value| value * share)
        }
    };
    [
        (kept[0] + back * fill[0]) * left,
        (kept[1] + back * fill[1]) * left,
        (kept[2] + back * fill[2]) * left,
        matte,
    ]
}

/// The final result of `x` alone: with its own pre-matte, and that through the thresholds.
#[must_use]
pub fn result(k: &Keyer, x: Rgb) -> Rgba {
    let pre = pre_matte(k, x);
    result_with(k, x, pre, thresholds(k, pre))
}

/// What the Status view shows of a pixel, of its pre-matte and its matte (p. 2442): whether it
/// is solid, transparent or in between, and whether the adjustments of the matte changed it.
/// Its level is black where the matte is transparent, white where it is solid, and a mid grey
/// in between. A pixel that the adjustments of the matte left as the key made it (within
/// [`STATUS_TOLERANCE`]) is a grey of its level; one that they made more opaque is green, of its
/// level; one that they made less opaque is red, of 1 less its level. The colours are ours.
#[must_use]
pub fn status(pre: f32, matte: f32) -> Rgb {
    let pre = unit(pre);
    let matte = unit(matte);
    let level = if matte <= 0.0 {
        0.0
    } else if matte >= 1.0 {
        1.0
    } else {
        0.5
    };
    if matte > pre + STATUS_TOLERANCE {
        return [0.0, level, 0.0];
    }
    if matte < pre - STATUS_TOLERANCE {
        return [1.0 - level, 0.0, 0.0];
    }
    [level; 3]
}

/// A grey of `level`, opaque.
fn grey(level: f32) -> Rgba {
    [level, level, level, 1.0]
}

/// What `view` shows of `x`, of the pre-matte and the matte of [`result_with`]. The mattes and
/// the status are shown opaque, as pictures of grey and of colour; the final result is given as
/// it is, premultiplied. For the Matte view, `matte` is the matte before the solid and garbage
/// mattes are met with it (p. 2442).
#[must_use]
pub fn shown_with(k: &Keyer, view: View, x: Rgb, pre: f32, matte: f32) -> Rgba {
    match view {
        View::PreMatte => grey(unit(pre)),
        View::Matte => grey(unit(matte)),
        View::Status => {
            let colour = status(pre, matte);
            [colour[0], colour[1], colour[2], 1.0]
        }
        View::FinalResult => result_with(k, x, pre, matte),
    }
}

/// What `view` shows of `x` alone: with its own pre-matte, and that through the thresholds.
#[must_use]
pub fn shown(k: &Keyer, view: View, x: Rgb) -> Rgba {
    let pre = pre_matte(k, x);
    shown_with(k, view, x, pre, thresholds(k, pre))
}

// ---------------------------------------------------------------------------------------------
// What reads the pixels about a pixel
// ---------------------------------------------------------------------------------------------

/// The controls of a keyer that read the pixels about each one (see the module): none of them
/// does anything at 0.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Finesse {
    /// Pre-Blur, from 0 to [`MOST_BLUR`]: the standard deviation of the Gaussian that the
    /// picture is keyed through, in pixels of a picture 1080 high.
    pub pre_blur: f32,
    /// Erode/Dilate, from less [`MOST_ERODE_DILATE`] to it: the radius of the box that the
    /// matte is eroded over, below 0, or dilated over, above it, in pixels of a picture 1080
    /// high.
    pub erode_dilate: f32,
    /// Matte Blur, from 0 to [`MOST_BLUR`]: the standard deviation of the Gaussian that the
    /// matte is blurred by, in pixels of a picture 1080 high.
    pub matte_blur: f32,
    /// Clean Foreground, from 0 to 1.
    pub clean_foreground: f32,
    /// Clean Background, from 0 to 1.
    pub clean_background: f32,
}

impl Finesse {
    /// Check that each value is in its range.
    ///
    /// # Errors
    /// A value is not a number, or out of its range: what, said for a person.
    pub fn check(&self) -> Result<(), String> {
        let within = |name: &str, value: f32, low: f32, high: f32| {
            if value.is_finite() && (low..=high).contains(&value) {
                Ok(())
            } else {
                Err(format!("{name} is from {low} to {high}, not {value}"))
            }
        };
        within("pre_blur", self.pre_blur, 0.0, MOST_BLUR)?;
        let most = MOST_ERODE_DILATE;
        within("erode_dilate", self.erode_dilate, -most, most)?;
        within("matte_blur", self.matte_blur, 0.0, MOST_BLUR)?;
        within("clean_foreground", self.clean_foreground, 0.0, 1.0)?;
        within("clean_background", self.clean_background, 0.0, 1.0)
    }

    /// The controls as the maths takes them: those that [`Finesse::check`] passes as they
    /// are, a number out of its range brought to its nearest end, and what is not a number
    /// read as 0.
    #[must_use]
    pub fn held(&self) -> Finesse {
        Finesse {
            pre_blur: held(self.pre_blur, 0.0, MOST_BLUR),
            erode_dilate: held(self.erode_dilate, -MOST_ERODE_DILATE, MOST_ERODE_DILATE),
            matte_blur: held(self.matte_blur, 0.0, MOST_BLUR),
            clean_foreground: held(self.clean_foreground, 0.0, 1.0),
            clean_background: held(self.clean_background, 0.0, 1.0),
        }
    }
}

/// The parameters of a corrector of a grade that has nothing but Matte Finesse: what `clean`,
/// `spread` and `morph` of the maths of a grade read of it, so that a keyer does to its matte
/// what a qualifier does to its key, by the same lines.
#[allow(
    clippy::cast_precision_loss,
    reason = "a sum of a few flags, far below what a float counts"
)]
fn finesse_block(
    features: u32,
    finesse: [f32; 4],
    more: [f32; 4],
) -> [[f32; 4]; at::CORRECTOR as usize] {
    let mut block = [[0.0; 4]; at::CORRECTOR as usize];
    block[at::FEATURES as usize] = [features as f32, 0.0, 0.0, 0.0];
    block[at::FINESSE as usize] = finesse;
    block[at::FINESSE_MORE as usize] = more;
    block
}

/// A matte through Erode/Dilate (p. 2445), `around` the mean of the matte in the box of that
/// radius about the pixel: a pass of Morph Operation, which grows the matte where
/// `erode_dilate` is above 0 (a pixel is as much in the matte as any of the box is, to
/// [`MORPH_EDGE`]) and shrinks it where it is below. At 0 the matte is as it is.
#[must_use]
pub fn eroded(matte: f32, around: f32, erode_dilate: f32) -> f32 {
    let matte = unit(matte);
    if erode_dilate.is_nan() || erode_dilate == 0.0 {
        return matte;
    }
    let grow = if erode_dilate > 0.0 { 1.0 } else { 0.0 };
    let block = finesse_block(at::MORPH_ON, [0.0; 4], [0.0, grow, 0.0, MORPH_EDGE]);
    maths::morph(Params(&block), 0, matte, around, false)
}

/// A matte through the Matte Blur (p. 2445), `around` it through its Gaussian: Blur Radius
/// with no In/Out Ratio, the blurred matte within 0 and 1.
#[must_use]
pub fn softened(around: f32) -> f32 {
    let block = finesse_block(at::SPREAD_ON, [0.0; 4], [0.0; 4]);
    maths::spread(Params(&block), 0, 0.0, around)
}

/// A matte through Clean Foreground and Clean Background (p. 2445), `around` the mean of the
/// matte about the pixel: Clean Black by Clean Background, which takes a pixel in a region
/// that is less in the matte than it towards 0, as a speck is; and Clean White by Clean
/// Foreground, which takes one in a region that is more in the matte than 1 less it towards
/// 1, as a hole is.
#[must_use]
pub fn cleaned(matte: f32, around: f32, clean_foreground: f32, clean_background: f32) -> f32 {
    let finesse = [clean_background, clean_foreground, 0.0, 1.0];
    let block = finesse_block(at::CLEAN_ON, finesse, [0.0; 4]);
    maths::clean(Params(&block), 0, unit(matte), around)
}

/// The colour of a premultiplied pixel and its alpha, as a keyer reads them: the colour not
/// premultiplied, and the alpha no less than 0 and no more than [`LARGEST`]. Where the alpha
/// is not above 0, or is no number, nothing is seen: the colour is black and the alpha 0.
#[must_use]
pub fn unpremultiplied(pixel: Rgba) -> (Rgb, f32) {
    let alpha = sound(pixel[3]);
    if alpha > 0.0 {
        (
            [pixel[0] / alpha, pixel[1] / alpha, pixel[2] / alpha],
            alpha,
        )
    } else {
        ([0.0; 3], 0.0)
    }
}

/// The pre-matte and the matte of each pixel of `image`, a premultiplied picture, row after
/// row: what the key and the controls of the matte make of it, before the solid and garbage
/// mattes (see the module). `units` of the pixels of the picture make a pixel of a picture
/// [`crate::color::REFERENCE_HEIGHT`] high, which the lengths of `finesse` are counted in.
///
/// The picture and the matte are filtered as the card filters them
/// ([`neighbourhood::filtered`]), and what a filter made is read between its texels at the
/// middle of each pixel ([`neighbourhood::sample`]). A matte that goes through a control that
/// reads the pixels about each one is kept in half floats between two of them, as the card
/// keeps it, and is one when it comes out; a matte that goes through none is as the formulas
/// make it.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    reason = "places of pixels, which a float holds"
)]
pub fn mattes(k: &Keyer, finesse: &Finesse, image: &Image, units: f32) -> Vec<[f32; 2]> {
    // A keyer that keys nothing has a matte of one level, which nothing here changes.
    let finesse = if k.gain > 0.0 {
        finesse.held()
    } else {
        Finesse::default()
    };
    let width = image.width as usize;
    let size = [image.width as f32, image.height as f32];
    let middle = |pixel: usize| {
        let (x, y) = (pixel % width.max(1), pixel / width.max(1));
        [(x as f32 + 0.5) / size[0], (y as f32 + 0.5) / size[1]]
    };
    let soft = (finesse.pre_blur > 0.0).then(|| {
        let filter = Filter::Gaussian {
            sigma: [finesse.pre_blur; 2],
        };
        neighbourhood::filtered(image, filter, units)
    });
    let pres: Vec<f32> = image
        .pixels
        .iter()
        .enumerate()
        .map(|(pixel, own)| {
            let read = match &soft {
                Some(soft) => neighbourhood::sample(soft, middle(pixel)),
                None => *own,
            };
            pre_matte(k, unpremultiplied(read).0)
        })
        .collect();
    let mut matte: Vec<f32> = pres.iter().map(|pre| thresholds(k, *pre)).collect();
    // The mean of the matte about each pixel, through `filter`: of the matte as the card
    // keeps it, white premultiplied by it.
    let around = |matte: &[f32], filter: Filter| -> Vec<f32> {
        let picture = Image {
            width: image.width,
            height: image.height,
            pixels: matte.iter().map(|level| [*level; 4]).collect(),
        };
        let filtered = neighbourhood::filtered(&picture, filter, units);
        (0..matte.len())
            .map(|pixel| neighbourhood::sample(&filtered, middle(pixel))[3])
            .collect()
    };
    let kept =
        |matte: Vec<f32>| -> Vec<f32> { matte.into_iter().map(neighbourhood::half).collect() };
    if finesse.erode_dilate != 0.0 {
        matte = kept(matte);
        let radius = [finesse.erode_dilate.abs(); 2];
        let means = around(&matte, Filter::Box { radius });
        let grown = matte
            .iter()
            .zip(&means)
            .map(|(level, mean)| eroded(*level, *mean, finesse.erode_dilate));
        matte = kept(grown.collect());
    }
    if finesse.matte_blur > 0.0 {
        matte = kept(matte);
        let filter = Filter::Gaussian {
            sigma: [finesse.matte_blur; 2],
        };
        let means = around(&matte, filter);
        matte = kept(means.into_iter().map(softened).collect());
    }
    if finesse.clean_foreground > 0.0 || finesse.clean_background > 0.0 {
        matte = kept(matte);
        let means = around(&matte, at::clean_filter());
        let clean = matte.iter().zip(&means).map(|(level, mean)| {
            let (foreground, background) = (finesse.clean_foreground, finesse.clean_background);
            cleaned(*level, *mean, foreground, background)
        });
        matte = kept(clean.collect());
    }
    pres.into_iter()
        .zip(matte)
        .map(|(pre, matte)| [pre, matte])
        .collect()
}

// ---------------------------------------------------------------------------------------------
// The node of a composition
// ---------------------------------------------------------------------------------------------

/// The Delta Keyer node of a composition at one moment, as the maths takes it (see the
/// module).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Node {
    /// The controls of the key, of the thresholds, of the spill and of the replace mode.
    pub settings: Settings,
    /// The controls that read the pixels about each one.
    pub finesse: Finesse,
    /// What the node shows.
    pub view: View,
    /// Whether the garbage matte is read the other way round.
    pub garbage_invert: bool,
}

impl Default for Node {
    /// The settings as they are at first, with none of the controls that read the pixels
    /// about each one, the final result shown, and the garbage matte read as it is.
    fn default() -> Self {
        Self {
            settings: Settings::default(),
            finesse: Finesse::default(),
            view: View::FinalResult,
            garbage_invert: false,
        }
    }
}

impl Node {
    /// The keyer of its settings ([`Keyer::of`]).
    #[must_use]
    pub fn keyer(&self) -> Keyer {
        Keyer::of(&self.settings)
    }
}

// The settings of the Delta Keyer node of the model (`columbina_model::fusion`) are these, by
// the same names and within the same ranges: a node at one moment is these settings as they
// are.

impl From<&DeltaKeyed> for Node {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "settings of a node, as the card takes them"
    )]
    fn from(settings: &DeltaKeyed) -> Self {
        Self {
            settings: Settings {
                screen: settings.screen.map(|channel| channel as f32),
                gain: settings.gain as f32,
                balance: settings.balance as f32,
                low: settings.low as f32,
                high: settings.high as f32,
                spill_method: match settings.spill_method {
                    model::SpillMethod::None => SpillMethod::None,
                    model::SpillMethod::Rare => SpillMethod::Rare,
                    model::SpillMethod::Medium => SpillMethod::Medium,
                    model::SpillMethod::WellDone => SpillMethod::WellDone,
                    model::SpillMethod::Burnt => SpillMethod::Burnt,
                },
                spill_suppression: settings.spill_suppression as f32,
                replace: match settings.replace {
                    model::ReplaceMode::None => Replace::None,
                    model::ReplaceMode::Source => Replace::Source,
                    model::ReplaceMode::HardColor => Replace::HardColour,
                    model::ReplaceMode::SoftColor => Replace::SoftColour,
                },
                replace_colour: settings.replace_color.map(|channel| channel as f32),
            },
            finesse: Finesse {
                pre_blur: settings.pre_blur as f32,
                erode_dilate: settings.erode_dilate as f32,
                matte_blur: settings.matte_blur as f32,
                clean_foreground: settings.clean_foreground as f32,
                clean_background: settings.clean_background as f32,
            },
            view: match settings.view {
                model::KeyerView::PreMatte => View::PreMatte,
                model::KeyerView::Matte => View::Matte,
                model::KeyerView::Status => View::Status,
                model::KeyerView::FinalResult => View::FinalResult,
            },
            garbage_invert: settings.garbage_invert,
        }
    }
}

/// The settings of a Delta Keyer node of a composition at one moment, as the maths takes
/// them. None of a node that is no Delta Keyer. The mattes and the mask that come into the
/// node are not among them: they are the [`Incoming`] of [`node_pixel`].
#[must_use]
pub fn of_node(kind: &FusedKind) -> Option<Node> {
    match kind {
        FusedKind::DeltaKeyer { settings, .. } => Some(settings.into()),
        _ => None,
    }
}

/// What comes into a keyer from other nodes at a pixel: the alpha of the picture that is
/// linked to each of its `garbage_matte`, `solid_matte` and `mask` inputs, and none of one
/// that is not linked. Each is read within 0 and 1, and as 0 where it is no number.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Incoming {
    /// The garbage matte, as it is linked: where it is 1 the matte is made transparent, or
    /// where it is 0 with the invert of the node.
    pub garbage: Option<f32>,
    /// The solid matte: where it is 1 the matte is made solid.
    pub solid: Option<f32>,
    /// The effect mask of the node.
    pub mask: Option<f32>,
}

/// A matte met with the solid and the garbage mattes that come in (p. 2440): first made 1
/// where the solid matte is 1, `m + (1 − m) s`, and then 0 where the garbage matte is 1,
/// `m (1 − g)`, so that the garbage matte wins where both are 1 (ours). With `garbage_invert`
/// the garbage matte is read as 1 less what it is. A matte that is not there changes nothing,
/// and neither does a solid matte of 0 or a garbage matte of 0, to the bit.
#[must_use]
pub fn met(matte: f32, incoming: &Incoming, garbage_invert: bool) -> f32 {
    let mut matte = unit(matte);
    if let Some(solid) = incoming.solid {
        let solid = unit(solid);
        if solid >= 1.0 {
            matte = 1.0;
        } else if solid > 0.0 {
            matte += (1.0 - matte) * solid;
        }
    }
    if let Some(garbage) = incoming.garbage {
        let level = unit(garbage);
        let garbage = if garbage_invert { 1.0 - level } else { level };
        if garbage >= 1.0 {
            matte = 0.0;
        } else if garbage > 0.0 {
            matte *= 1.0 - garbage;
        }
    }
    matte
}

/// What a keyer node makes of the pixel `pixel`, premultiplied, of the pre-matte and the
/// matte that the key and the controls of the matte made of it ([`mattes`]) and of what comes
/// into the node there (see the module): what its view shows, mixed with the pixel as it came
/// in by the effect mask.
#[must_use]
pub fn node_pixel(node: &Node, pixel: Rgba, pre: f32, matte: f32, incoming: &Incoming) -> Rgba {
    let k = node.keyer();
    let (colour, alpha) = unpremultiplied(pixel);
    let after = met(matte, incoming, node.garbage_invert);
    let own = match node.view {
        View::Matte => shown_with(&k, View::Matte, colour, pre, matte),
        View::PreMatte | View::Status => shown_with(&k, node.view, colour, pre, after),
        View::FinalResult => result_with(&k, colour, pre, after).map(|value| value * alpha),
    };
    let Some(mask) = incoming.mask else {
        return own;
    };
    let mask = unit(mask);
    if mask <= 0.0 {
        return pixel;
    }
    if mask >= 1.0 {
        return own;
    }
    [0, 1, 2, 3].map(|channel| pixel[channel] + (own[channel] - pixel[channel]) * mask)
}

/// What a keyer node makes of each pixel of `image`, a premultiplied picture, row after row
/// ([`mattes`] and [`node_pixel`]): `units` of its pixels make a pixel of a picture
/// [`crate::color::REFERENCE_HEIGHT`] high, and `incoming` gives what comes into the node at
/// the pixel of each number.
#[must_use]
pub fn node_picture(
    node: &Node,
    image: &Image,
    units: f32,
    incoming: &dyn Fn(usize) -> Incoming,
) -> Vec<Rgba> {
    let made = mattes(&node.keyer(), &node.finesse, image, units);
    image
        .pixels
        .iter()
        .zip(made)
        .enumerate()
        .map(|(at, (pixel, [pre, matte]))| node_pixel(node, *pixel, pre, matte, &incoming(at)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GREEN: Rgb = [0.0, 1.0, 0.0];
    const BLUE: Rgb = [0.0, 0.0, 1.0];
    /// A green screen as a camera sees one.
    const SCREEN: Rgb = [0.08, 0.45, 0.12];
    const GREY: Rgb = [0.18, 0.18, 0.18];
    const SKIN: Rgb = [0.45, 0.25, 0.18];

    /// Colours of every kind: screens lit and in shadow, greys, a skin tone, a subject with
    /// spill on it, the primaries, and colours beyond white and below black.
    const COLOURS: [Rgb; 13] = [
        GREEN,
        BLUE,
        SCREEN,
        [0.04, 0.2, 0.06],
        GREY,
        SKIN,
        [0.3, 0.5, 0.2],
        [0.2, 0.6, 0.4],
        [1.0, 0.0, 0.0],
        [0.9, 0.9, 0.1],
        [0.0, 0.0, 0.0],
        [2.5, 4.0, 1.5],
        [-0.05, 0.02, -0.01],
    ];

    /// The keyer of the settings as they are first, set by `set`, which are checked.
    fn keyer(set: impl FnOnce(&mut Settings)) -> Keyer {
        let mut settings = Settings::default();
        set(&mut settings);
        assert_eq!(settings.check(), Ok(()));
        Keyer::of(&settings)
    }

    #[track_caller]
    fn about(got: f32, wanted: f32) {
        assert!((got - wanted).abs() <= 1e-6, "{got} is not {wanted}");
    }

    #[track_caller]
    fn all_about(got: Rgba, wanted: Rgba) {
        for (got, wanted) in got.into_iter().zip(wanted) {
            about(got, wanted);
        }
    }

    /// A colour with its green and its blue changed over.
    fn changed_over(x: Rgb) -> Rgb {
        [x[0], x[2], x[1]]
    }

    #[test]
    fn the_screen_colour_is_keyed_out_whole() {
        for screen in [GREEN, BLUE, SCREEN, [0.5, 0.8, 0.1], [0.7, 0.1, 0.15]] {
            for balance in [0.0, 0.3, 0.5, 1.0] {
                let k = keyer(|s| {
                    s.screen = screen;
                    s.balance = balance;
                });
                assert_eq!(pre_matte(&k, screen), 0.0, "{screen:?}");
                assert_eq!(matte(&k, screen), 0.0, "{screen:?}");
                assert_eq!(result(&k, screen), [0.0; 4], "{screen:?}");
                // And so is the screen lit twice as brightly.
                assert_eq!(result(&k, screen.map(|v| 2.0 * v)), [0.0; 4]);
            }
        }
        // A red screen has red for its dominant channel; of two channels alike, green is before
        // blue and blue before red.
        assert_eq!(keyer(|s| s.screen = [0.7, 0.1, 0.15]).dominant, 0);
        assert_eq!(dominant_of([0.0, 0.6, 0.6]), 1);
        assert_eq!(dominant_of([0.6, 0.0, 0.6]), 2);
        assert_eq!(dominant_of([0.6, 0.6, 0.0]), 1);
    }

    #[test]
    fn a_grey_and_a_skin_tone_are_kept_whole_at_first() {
        for screen in [GREEN, BLUE, SCREEN, [0.1, 0.15, 0.5]] {
            let k = keyer(|s| s.screen = screen);
            for colour in [GREY, SKIN, [0.9, 0.9, 0.9], [0.0, 0.0, 0.0]] {
                assert_eq!(pre_matte(&k, colour), 1.0, "{colour:?} on {screen:?}");
                assert_eq!(matte(&k, colour), 1.0, "{colour:?} on {screen:?}");
                // The picture is as it was, to the bit, and opaque.
                assert_eq!(
                    result(&k, colour),
                    [colour[0], colour[1], colour[2], 1.0],
                    "{colour:?} on {screen:?}"
                );
            }
        }
    }

    #[test]
    fn a_grey_over_its_screen_comes_back_premultiplied() {
        let k = keyer(|s| s.screen = SCREEN);
        for share in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let seen = [0, 1, 2].map(|i| share * GREY[i] + (1.0 - share) * SCREEN[i]);
            all_about(
                result(&k, seen),
                [share * GREY[0], share * GREY[1], share * GREY[2], share],
            );
        }
    }

    #[test]
    fn a_matte_never_falls_as_it_rises_through_the_thresholds() {
        let levels: Vec<f32> = (0..=64_u8).map(|step| f32::from(step) / 64.0).collect();
        let with = |low: f32, high: f32| {
            keyer(|s| {
                s.low = low;
                s.high = high;
            })
        };
        for (low, high) in [
            (0.0, 1.0),
            (0.25, 0.75),
            (0.5, 0.5),
            (0.0, 0.25),
            (0.75, 1.0),
        ] {
            let k = with(low, high);
            let mut last = 0.0;
            for &level in &levels {
                let through = thresholds(&k, level);
                assert!(through >= last, "{level} between {low} and {high}");
                assert!((0.0..=1.0).contains(&through));
                if level < low {
                    assert_eq!(through, 0.0, "{level} below {low}");
                }
                if level > high {
                    assert_eq!(through, 1.0, "{level} above {high}");
                }
                last = through;
            }
        }
        // Thresholds of 0 and 1 leave a matte as it is, and others keep the levels between them
        // as far apart as they were, each from the next.
        for &level in &levels {
            assert_eq!(thresholds(&with(0.0, 1.0), level), level);
        }
        about(thresholds(&with(0.25, 0.75), 0.5), 0.5);
        about(thresholds(&with(0.25, 0.75), 0.375), 0.25);
        // A higher lower threshold makes no pixel more opaque, and a lower upper one none less.
        for &level in &levels {
            assert!(thresholds(&with(0.5, 1.0), level) <= thresholds(&with(0.25, 1.0), level));
            assert!(thresholds(&with(0.0, 0.5), level) >= thresholds(&with(0.0, 0.75), level));
        }
    }

    #[test]
    fn gain_makes_more_of_the_picture_transparent() {
        let with = |gain: f32| {
            keyer(|s| {
                s.screen = SCREEN;
                s.gain = gain;
            })
        };
        let gains = [0.0, 0.5, 1.0, 2.0, 4.0, 10.0];
        for colour in COLOURS {
            for pair in gains.windows(2) {
                assert!(
                    pre_matte(&with(pair[1]), colour) <= pre_matte(&with(pair[0]), colour),
                    "{colour:?} at {pair:?}"
                );
            }
            // No gain keys nothing.
            assert_eq!(pre_matte(&with(0.0), colour), 1.0, "{colour:?}");
        }
        // A screen in shadow, half as bright, is half keyed, and keyed whole at a Gain of 2.
        let shadow = SCREEN.map(|v| 0.5 * v);
        about(pre_matte(&with(1.0), shadow), 0.5);
        assert_eq!(pre_matte(&with(2.0), shadow), 0.0);
        assert_eq!(result(&with(2.0), shadow), [0.0; 4]);
    }

    #[test]
    fn balance_goes_from_the_lesser_of_the_other_two_channels_to_the_greater() {
        let with = |balance: f32| keyer(|s| s.balance = balance);
        // Green is above red by 0.4 and above blue by 0.2.
        let colour = [0.2, 0.6, 0.4];
        about(difference(&with(0.0), colour), 0.4);
        about(difference(&with(1.0), colour), 0.2);
        about(difference(&with(0.5), colour), 0.3);
        about(pre_matte(&with(0.0), colour), 0.6);
        about(pre_matte(&with(1.0), colour), 0.8);
        about(pre_matte(&with(0.5), colour), 0.7);
        // It is the same with red and blue changed over: the lesser and the greater, not the
        // first and the second.
        about(difference(&with(0.0), [0.4, 0.6, 0.2]), 0.4);
        // More Balance keys no colour more, on a pure screen.
        let balances = [0.0, 0.25, 0.5, 0.75, 1.0];
        for colour in COLOURS {
            for pair in balances.windows(2) {
                assert!(
                    pre_matte(&with(pair[1]), colour) >= pre_matte(&with(pair[0]), colour),
                    "{colour:?} at {pair:?}"
                );
            }
        }
        // A grey has no colour difference, whatever the Balance.
        for balance in balances {
            assert_eq!(difference(&with(balance), GREY), 0.0);
        }
    }

    #[test]
    fn each_spill_method_takes_at_least_as_much_as_the_one_before() {
        for screen in [GREEN, BLUE, SCREEN, [0.7, 0.1, 0.15]] {
            for colour in COLOURS {
                let mut last = 0.0;
                for method in SpillMethod::ALL {
                    let k = keyer(|s| {
                        s.screen = screen;
                        s.spill_method = method;
                    });
                    let taken = spill(&k, colour);
                    assert!(taken >= last, "{method:?} of {colour:?} on {screen:?}");
                    last = taken;
                    // It is taken from the channel of the screen, and the others are left.
                    let mut wanted = colour;
                    wanted[usize::from(k.dominant)] -= taken;
                    assert_eq!(despilled(&k, colour), wanted, "{method:?} of {colour:?}");
                }
            }
        }
        // A subject with green on it: red 0.3 and blue 0.2 limit its green.
        let green_of = |method: SpillMethod, suppression: f32| {
            let k = keyer(|s| {
                s.spill_method = method;
                s.spill_suppression = suppression;
            });
            despilled(&k, [0.3, 0.5, 0.2])[1]
        };
        assert_eq!(green_of(SpillMethod::None, 1.0), 0.5);
        about(green_of(SpillMethod::Rare, 1.0), 0.3);
        about(green_of(SpillMethod::Medium, 1.0), 0.25);
        about(green_of(SpillMethod::WellDone, 1.0), 0.225);
        about(green_of(SpillMethod::Burnt, 1.0), 0.2);
        // Spill Suppression at 0 takes nothing, and at a half, half.
        assert_eq!(green_of(SpillMethod::Burnt, 0.0), 0.5);
        about(green_of(SpillMethod::Burnt, 0.5), 0.35);
    }

    #[test]
    fn a_blue_screen_is_keyed_as_a_green_one_with_green_and_blue_changed_over() {
        let replaces = [
            Replace::None,
            Replace::Source,
            Replace::HardColour,
            Replace::SoftColour,
        ];
        for replace in replaces {
            for method in SpillMethod::ALL {
                let with = |screen: Rgb, replace_colour: Rgb| {
                    keyer(|s| {
                        *s = Settings {
                            screen,
                            gain: 1.5,
                            balance: 0.3,
                            low: 0.1,
                            high: 0.8,
                            spill_method: method,
                            spill_suppression: 1.0,
                            replace,
                            replace_colour,
                        };
                    })
                };
                let green = with(SCREEN, [0.4, 0.3, 0.2]);
                let blue = with(changed_over(SCREEN), changed_over([0.4, 0.3, 0.2]));
                assert_eq!((green.dominant, blue.dominant), (1, 2));
                for colour in COLOURS {
                    let over = changed_over(colour);
                    assert_eq!(pre_matte(&blue, over), pre_matte(&green, colour));
                    assert_eq!(
                        despilled(&blue, over),
                        changed_over(despilled(&green, colour))
                    );
                    for view in View::ALL {
                        let made = shown(&green, view, colour);
                        // The colours of the status are of the matte, not of the picture.
                        let wanted = if view == View::Status {
                            made
                        } else {
                            [made[0], made[2], made[1], made[3]]
                        };
                        assert_eq!(
                            shown(&blue, view, over),
                            wanted,
                            "{view:?} of {colour:?} with {replace:?} and {method:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn what_the_matte_gives_back_is_filled_as_the_replace_mode_says() {
        let with = |replace: Replace, method: SpillMethod| {
            keyer(|s| {
                s.spill_method = method;
                s.replace = replace;
                s.replace_colour = [0.5, 0.4, 0.3];
            })
        };
        // Four fifths of a grey of 0.5 over a fifth of a pure green screen: the key makes it
        // four fifths opaque, and takes the fifth of green away.
        let seen = [0.4, 0.6, 0.4];
        let of = |replace: Replace, matte: f32| {
            result_with(&with(replace, SpillMethod::None), seen, 0.8, matte)
        };
        about(
            pre_matte(&with(Replace::None, SpillMethod::None), seen),
            0.8,
        );
        for replace in [
            Replace::None,
            Replace::Source,
            Replace::HardColour,
            Replace::SoftColour,
        ] {
            // A matte left as the key made it gives nothing back.
            all_about(of(replace, 0.8), [0.4, 0.4, 0.4, 0.8]);
            // A matte made less opaque takes as much of the colour, and all of it at 0.
            all_about(of(replace, 0.4), [0.2, 0.2, 0.2, 0.4]);
            assert_eq!(of(replace, 0.0), [0.0; 4]);
        }
        // A matte made solid gives the fifth back.
        all_about(of(Replace::None, 1.0), [0.4, 0.4, 0.4, 1.0]);
        all_about(of(Replace::Source, 1.0), [0.4, 0.6, 0.4, 1.0]);
        all_about(of(Replace::HardColour, 1.0), [0.5, 0.48, 0.46, 1.0]);
        all_about(of(Replace::SoftColour, 1.0), [0.5, 0.48, 0.46, 1.0]);
        // And half of it where it is made half way to solid.
        all_about(of(Replace::Source, 0.9), [0.4, 0.5, 0.4, 0.9]);
        all_about(of(Replace::HardColour, 0.9), [0.45, 0.44, 0.43, 0.9]);
        // Soft Color counts the spill that was taken with the screen, and Hard Color does not:
        // of this green 0.2 goes with the screen and 0.1 as spill.
        let spilt = [0.5, 0.6, 0.3];
        let burnt =
            |replace: Replace| result_with(&with(replace, SpillMethod::Burnt), spilt, 0.8, 1.0);
        all_about(burnt(Replace::None), [0.5, 0.3, 0.3, 1.0]);
        all_about(burnt(Replace::HardColour), [0.6, 0.38, 0.36, 1.0]);
        all_about(burnt(Replace::SoftColour), [0.65, 0.42, 0.39, 1.0]);
        // Source gives the picture back as it was, its spill with it.
        all_about(burnt(Replace::Source), [0.5, 0.6, 0.3, 1.0]);
    }

    #[test]
    fn each_view_shows_its_stage_of_the_key() {
        let k = keyer(|s| {
            s.screen = SCREEN;
            s.low = 0.2;
            s.high = 0.6;
        });
        // A pixel of the edge: keyed in part, and more opaque through the thresholds.
        let colour = [0.1, 0.3, 0.12];
        let pre = pre_matte(&k, colour);
        let through = matte(&k, colour);
        assert!((pre - 0.457).abs() < 1e-3 && (through - 0.643).abs() < 1e-3);
        assert_eq!(shown(&k, View::PreMatte, colour), [pre, pre, pre, 1.0]);
        assert_eq!(
            shown(&k, View::Matte, colour),
            [through, through, through, 1.0]
        );
        assert_eq!(shown(&k, View::Status, colour), [0.0, 0.5, 0.0, 1.0]);
        assert_eq!(shown(&k, View::FinalResult, colour), result(&k, colour));
        assert_eq!(result(&k, colour)[3], through);
        // The status: transparent, solid and in between, as the key made them.
        assert_eq!(status(0.0, 0.0), [0.0; 3]);
        assert_eq!(status(1.0, 1.0), [1.0; 3]);
        assert_eq!(status(0.4, 0.4), [0.5; 3]);
        assert_eq!(status(0.5, 0.502), [0.5; 3]);
        // Made more opaque by the matte's adjustments: solid, and in between.
        assert_eq!(status(0.8, 1.0), [0.0, 1.0, 0.0]);
        assert_eq!(status(0.3, 0.6), [0.0, 0.5, 0.0]);
        // Made less opaque: transparent, and in between.
        assert_eq!(status(0.2, 0.0), [1.0, 0.0, 0.0]);
        assert_eq!(status(0.9, 0.5), [0.5, 0.0, 0.0]);
        // The views are of the mattes that they are given.
        assert_eq!(
            shown_with(&k, View::Status, colour, 0.8, 1.0),
            [0.0, 1.0, 0.0, 1.0]
        );
        assert_eq!(
            shown_with(&k, View::PreMatte, colour, 0.8, 1.0),
            [0.8, 0.8, 0.8, 1.0]
        );
        assert_eq!(shown_with(&k, View::Matte, colour, 0.8, 1.0), [1.0; 4]);
        assert_eq!(View::default(), View::FinalResult);
    }

    #[test]
    fn what_is_not_a_number_is_read_as_nothing_and_what_is_endless_as_the_largest() {
        assert_eq!(sound(f32::NAN), 0.0);
        assert_eq!(sound(f32::INFINITY), LARGEST);
        assert_eq!(sound(f32::NEG_INFINITY), -LARGEST);
        assert_eq!(sound(f32::MAX), LARGEST);
        assert_eq!(sound(-0.25), -0.25);
        let k = keyer(|s| {
            s.screen = SCREEN;
            s.low = 0.1;
            s.high = 0.9;
        });
        // A pixel with such a channel is keyed as the pixel with the channel read so.
        assert_eq!(
            result(&k, [f32::NAN, 0.5, 0.2]),
            result(&k, [0.0, 0.5, 0.2])
        );
        assert_eq!(
            result(&k, [0.3, f32::INFINITY, 0.2]),
            result(&k, [0.3, LARGEST, 0.2])
        );
        assert_eq!(
            despilled(&k, [0.3, 0.5, f32::NEG_INFINITY]),
            despilled(&k, [0.3, 0.5, -LARGEST])
        );
        // Whatever the channels and the mattes given, what is shown is a number.
        let bad = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.5];
        let mattes = [
            (f32::NAN, 0.5),
            (0.5, f32::NAN),
            (f32::INFINITY, f32::NEG_INFINITY),
            (0.0, 1.0),
        ];
        for red in bad {
            for green in bad {
                for blue in bad {
                    let colour = [red, green, blue];
                    for view in View::ALL {
                        let seen = shown(&k, view, colour);
                        assert!(
                            seen.iter().all(|value| value.is_finite()),
                            "{view:?} of {colour:?}: {seen:?}"
                        );
                        for (pre, matte) in mattes {
                            let seen = shown_with(&k, view, colour, pre, matte);
                            assert!(
                                seen.iter().all(|value| value.is_finite()),
                                "{view:?} of {colour:?} with {pre} and {matte}: {seen:?}"
                            );
                        }
                    }
                }
            }
        }
        // A matte that is not a number is read as transparent.
        assert_eq!(result_with(&k, GREY, f32::NAN, f32::NAN), [0.0; 4]);
        assert_eq!(thresholds(&k, f32::NAN), 0.0);
        assert_eq!(status(f32::NAN, f32::NAN), [0.0; 3]);
    }

    #[test]
    fn the_settings_at_first_are_in_their_ranges_and_others_are_refused() {
        let first = Settings::default();
        assert_eq!(first.check(), Ok(()));
        assert_eq!(
            first,
            Settings {
                screen: GREEN,
                gain: 1.0,
                balance: 0.5,
                low: 0.0,
                high: 1.0,
                spill_method: SpillMethod::Medium,
                spill_suppression: 1.0,
                replace: Replace::SoftColour,
                replace_colour: [0.5; 3],
            }
        );
        let refused = |set: fn(&mut Settings)| {
            let mut settings = Settings::default();
            set(&mut settings);
            settings.check().is_err()
        };
        // The ends of each range are within it.
        assert!(!refused(|s| s.gain = 0.0));
        assert!(!refused(|s| s.gain = MOST_GAIN));
        assert!(!refused(|s| s.balance = 0.0));
        assert!(!refused(|s| s.balance = 1.0));
        assert!(!refused(|s| s.spill_suppression = 0.0));
        assert!(!refused(|s| {
            s.low = 0.5;
            s.high = 0.5;
        }));
        assert!(!refused(|s| s.screen = [4.0, 0.5, 0.25]));
        // What is beyond them is not, nor what is not a number.
        assert!(refused(|s| s.gain = -0.1));
        assert!(refused(|s| s.gain = 10.5));
        assert!(refused(|s| s.gain = f32::NAN));
        assert!(refused(|s| s.balance = -0.1));
        assert!(refused(|s| s.balance = 1.1));
        assert!(refused(|s| s.low = -0.1));
        assert!(refused(|s| s.high = 1.1));
        assert!(refused(|s| s.high = f32::NAN));
        assert!(refused(|s| s.spill_suppression = 1.5));
        assert!(refused(|s| s.spill_suppression = f32::NAN));
        assert!(refused(|s| s.replace_colour = [0.5, -0.1, 0.5]));
        assert!(refused(|s| s.replace_colour = [0.5, f32::NAN, 0.5]));
        // Thresholds out of order.
        assert!(refused(|s| {
            s.low = 0.6;
            s.high = 0.4;
        }));
        // A screen colour is of no channel below 0, and has one above its other two.
        assert!(refused(|s| s.screen = [-0.1, 1.0, 0.0]));
        assert!(refused(|s| s.screen = [0.0, f32::INFINITY, 0.0]));
        assert!(refused(|s| s.screen = [0.5, 0.5, 0.5]));
        assert!(refused(|s| s.screen = [0.0, 0.0, 0.0]));
        assert!(refused(|s| s.screen = [0.2, 0.6, 0.6]));
        about(dominance([0.2, 0.6, 0.5]), 0.1);
        assert!(!refused(|s| s.screen = [0.2, 0.6, 0.5]));
    }

    #[test]
    fn settings_that_are_refused_make_a_keyer_that_gives_numbers() {
        // A screen colour with no channel above its other two keys nothing.
        let grey = Keyer::of(&Settings {
            screen: [0.5; 3],
            ..Settings::default()
        });
        for colour in COLOURS {
            assert_eq!(pre_matte(&grey, colour), 1.0, "{colour:?}");
        }
        // Numbers out of their ranges are brought to the nearest end, and what is not a number
        // is read as 0.
        let wild = Keyer::of(&Settings {
            screen: [f32::NAN, f32::INFINITY, -1.0],
            gain: f32::INFINITY,
            balance: f32::NAN,
            low: -3.0,
            high: 7.0,
            spill_method: SpillMethod::Burnt,
            spill_suppression: f32::NAN,
            replace: Replace::SoftColour,
            replace_colour: [f32::NAN, f32::INFINITY, -1.0],
        });
        assert_eq!(wild.screen, [0.0, LARGEST, 0.0]);
        assert_eq!((wild.gain, wild.balance), (MOST_GAIN, 0.0));
        assert_eq!((wild.low, wild.high, wild.spill), (0.0, 1.0, 0.0));
        assert_eq!(wild.replace_colour, [0.0, LARGEST, 0.0]);
        for colour in COLOURS {
            for view in View::ALL {
                let seen = shown(&wild, view, colour);
                assert!(seen.iter().all(|value| value.is_finite()), "{seen:?}");
            }
        }
    }

    #[test]
    fn a_shader_is_given_the_numbers_of_the_keyer() {
        let k = keyer(|s| {
            s.screen = changed_over(SCREEN);
            s.gain = 2.0;
            s.balance = 0.25;
            s.low = 0.125;
            s.high = 0.75;
            s.spill_method = SpillMethod::WellDone;
            s.spill_suppression = 0.5;
            s.replace = Replace::HardColour;
            s.replace_colour = [0.1, 0.2, 0.3];
        });
        assert_eq!(
            k.uniform(),
            [
                0.08,
                0.12,
                0.45,
                2.0,
                2.0,
                k.screen_difference,
                0.25,
                0.0,
                0.125,
                0.75,
                0.25,
                0.5,
                0.1,
                0.2,
                0.3,
                2.0,
            ]
        );
        about(k.screen_difference, 0.45 - (0.08 + 0.25 * 0.04));
        // No method takes no spill, whatever Spill Suppression is.
        assert_eq!(
            keyer(|s| s.spill_method = SpillMethod::None).uniform()[11],
            0.0
        );
    }

    /// A node whose settings are those at first, set by `set`.
    fn node(set: impl FnOnce(&mut Node)) -> Node {
        let mut node = Node::default();
        set(&mut node);
        node
    }

    /// A picture 24 pixels wide and 16 high, opaque: `colour` gives that of the pixel (x, y).
    fn picture(colour: impl Fn(u32, u32) -> Rgb) -> Image {
        let (width, height) = (24, 16);
        let pixels = (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .map(|(x, y)| {
                let rgb = colour(x, y);
                [rgb[0], rgb[1], rgb[2], 1.0]
            })
            .collect();
        Image {
            width,
            height,
            pixels,
        }
    }

    /// A pure green screen on the left half of [`picture`], and a grey on the right.
    fn halved() -> Image {
        picture(|x, _| if x < 12 { GREEN } else { [0.5; 3] })
    }

    /// The matte of the row 8 of a picture of [`picture`] through `finesse`, with the
    /// settings as they are first: the lengths are pixels of the picture.
    fn row(image: &Image, finesse: Finesse) -> Vec<f32> {
        let made = mattes(&Keyer::of(&Settings::default()), &finesse, image, 1.0);
        made[8 * 24..9 * 24].iter().map(|both| both[1]).collect()
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "each setting of the node of the model against its own here"
    )]
    fn the_delta_keyer_of_the_model_has_these_settings_by_their_ranges_and_first_values() {
        use columbina_model::fusion::bounds;
        use columbina_model::grade::{Interp, Key, Param};
        // What a Delta Keyer node of the model is at first, at any frame, is what these are
        // at first.
        let first = model::DeltaKeyer::default();
        assert_eq!(Node::from(&first.at(12.0)), Node::default());
        assert_eq!(Node::default().settings, Settings::default());
        assert_eq!(Node::default().finesse, Finesse::default());
        assert_eq!(Node::default().view, View::default());
        // Each range of the model is the range of that setting here, with the same value at
        // first: what the model takes these take, and what it refuses these refuse.
        let settings = Settings::default();
        for (name, theirs, ours) in [
            (
                "screen.r",
                bounds::KEYER_SCREEN,
                [settings.screen[0], 0.0, LARGEST],
            ),
            (
                "screen.g",
                bounds::KEYER_SCREEN_GREEN,
                [settings.screen[1], 0.0, LARGEST],
            ),
            (
                "screen.b",
                bounds::KEYER_SCREEN,
                [settings.screen[2], 0.0, LARGEST],
            ),
            ("gain", bounds::KEYER_GAIN, [settings.gain, 0.0, MOST_GAIN]),
            (
                "balance",
                bounds::KEYER_BALANCE,
                [settings.balance, 0.0, 1.0],
            ),
            ("blur", bounds::KEYER_BLUR, [0.0, 0.0, MOST_BLUR]),
            ("low", bounds::KEYER_LOW, [settings.low, 0.0, 1.0]),
            ("high", bounds::KEYER_HIGH, [settings.high, 0.0, 1.0]),
            (
                "erode_dilate",
                bounds::KEYER_ERODE_DILATE,
                [0.0, -MOST_ERODE_DILATE, MOST_ERODE_DILATE],
            ),
            ("clean", bounds::KEYER_CLEAN, [0.0, 0.0, 1.0]),
            (
                "replace_color",
                bounds::KEYER_REPLACE_COLOR,
                [settings.replace_colour[0], 0.0, LARGEST],
            ),
            (
                "spill_suppression",
                bounds::KEYER_SPILL_SUPPRESSION,
                [settings.spill_suppression, 0.0, 1.0],
            ),
        ] {
            let ours = ours.map(f64::from);
            assert_eq!([theirs.default, theirs.low, theirs.high], ours, "{name}");
        }
        assert_eq!(model::LEAST_DOMINANCE, LEAST_DOMINANCE);
        // A node of the model as it is at first is refused by none of the checks here; and a
        // node of another kind has no settings of a keyer.
        let mut composition = model::Composition::default();
        composition
            .nodes
            .push(model::Node::new("k", model::Kind::DeltaKeyer));
        composition.links = vec![
            model::Link::new(model::MEDIA_IN, "k", model::Input::Input),
            model::Link::new("k", model::MEDIA_OUT, model::Input::Input),
        ];
        composition.sort_links();
        let fused = composition.at(0.0);
        assert_eq!(fused.nodes.len(), 2);
        let made = of_node(&fused.nodes[1].kind).expect("a delta keyer");
        assert_eq!(made.settings.check(), Ok(()));
        assert_eq!(made.finesse.check(), Ok(()));
        assert_eq!(made, Node::default());
        assert_eq!(of_node(&fused.nodes[0].kind), None);
        // A node at a frame of its keys is its numbers there, by the same names.
        let rising = Param::Keys(vec![
            Key {
                frame: 0,
                value: 1.0,
                interp: Interp::Linear,
            },
            Key {
                frame: 10,
                value: 3.0,
                interp: Interp::Linear,
            },
        ]);
        let moving = model::DeltaKeyer {
            screen: model::KeyColor::new(0.125, 0.25, 0.75),
            gain: rising,
            balance: Param::Fixed(0.25),
            pre_blur: Param::Fixed(4.0),
            low: Param::Fixed(0.125),
            high: Param::Fixed(0.875),
            erode_dilate: Param::Fixed(-2.5),
            matte_blur: Param::Fixed(3.0),
            clean_foreground: Param::Fixed(0.5),
            clean_background: Param::Fixed(0.25),
            replace: model::ReplaceMode::HardColor,
            replace_color: model::KeyColor::new(0.5, 0.25, 2.0),
            garbage_invert: true,
            spill_method: model::SpillMethod::WellDone,
            spill_suppression: Param::Fixed(0.75),
            view: model::KeyerView::Status,
        };
        let kind = FusedKind::DeltaKeyer {
            input: None,
            garbage_matte: None,
            solid_matte: None,
            mask: None,
            settings: moving.at(5.0),
        };
        let wanted = Node {
            settings: Settings {
                screen: [0.125, 0.25, 0.75],
                gain: 2.0,
                balance: 0.25,
                low: 0.125,
                high: 0.875,
                spill_method: SpillMethod::WellDone,
                spill_suppression: 0.75,
                replace: Replace::HardColour,
                replace_colour: [0.5, 0.25, 2.0],
            },
            finesse: Finesse {
                pre_blur: 4.0,
                erode_dilate: -2.5,
                matte_blur: 3.0,
                clean_foreground: 0.5,
                clean_background: 0.25,
            },
            view: View::Status,
            garbage_invert: true,
        };
        assert_eq!(of_node(&kind), Some(wanted));
        assert_eq!(wanted.settings.check(), Ok(()));
        assert_eq!(wanted.finesse.check(), Ok(()));
        assert_eq!(wanted.keyer().dominant, 2);
        // Every spelling of the model is one of the enums here.
        let spelled = |change: &dyn Fn(&mut model::DeltaKeyer)| {
            let mut keyer = model::DeltaKeyer::default();
            change(&mut keyer);
            Node::from(&keyer.at(0.0))
        };
        for (theirs, ours) in [
            (model::SpillMethod::None, SpillMethod::None),
            (model::SpillMethod::Rare, SpillMethod::Rare),
            (model::SpillMethod::Medium, SpillMethod::Medium),
            (model::SpillMethod::WellDone, SpillMethod::WellDone),
            (model::SpillMethod::Burnt, SpillMethod::Burnt),
        ] {
            let made = spelled(&|keyer| keyer.spill_method = theirs);
            assert_eq!(made.settings.spill_method, ours);
        }
        for (theirs, ours) in [
            (model::ReplaceMode::None, Replace::None),
            (model::ReplaceMode::Source, Replace::Source),
            (model::ReplaceMode::HardColor, Replace::HardColour),
            (model::ReplaceMode::SoftColor, Replace::SoftColour),
        ] {
            assert_eq!(
                spelled(&|keyer| keyer.replace = theirs).settings.replace,
                ours
            );
        }
        for (theirs, ours) in [
            (model::KeyerView::PreMatte, View::PreMatte),
            (model::KeyerView::Matte, View::Matte),
            (model::KeyerView::Status, View::Status),
            (model::KeyerView::FinalResult, View::FinalResult),
        ] {
            assert_eq!(spelled(&|keyer| keyer.view = theirs).view, ours);
        }
    }

    #[test]
    fn the_controls_about_a_pixel_are_in_their_ranges_and_held_within_them() {
        let refused = |set: fn(&mut Finesse)| {
            let mut finesse = Finesse::default();
            set(&mut finesse);
            finesse.check().is_err()
        };
        assert_eq!(Finesse::default().check(), Ok(()));
        assert!(!refused(|f| f.pre_blur = MOST_BLUR));
        assert!(!refused(|f| f.erode_dilate = -MOST_ERODE_DILATE));
        assert!(!refused(|f| f.erode_dilate = MOST_ERODE_DILATE));
        assert!(!refused(|f| f.clean_foreground = 1.0));
        assert!(refused(|f| f.pre_blur = -0.5));
        assert!(refused(|f| f.pre_blur = 100.5));
        assert!(refused(|f| f.erode_dilate = 100.5));
        assert!(refused(|f| f.erode_dilate = f32::NAN));
        assert!(refused(|f| f.matte_blur = -1.0));
        assert!(refused(|f| f.matte_blur = f32::INFINITY));
        assert!(refused(|f| f.clean_foreground = 1.5));
        assert!(refused(|f| f.clean_background = -0.5));
        let wild = Finesse {
            pre_blur: f32::INFINITY,
            erode_dilate: -1e9,
            matte_blur: f32::NAN,
            clean_foreground: 7.0,
            clean_background: -3.0,
        };
        assert_eq!(
            wild.held(),
            Finesse {
                pre_blur: MOST_BLUR,
                erode_dilate: -MOST_ERODE_DILATE,
                matte_blur: 0.0,
                clean_foreground: 1.0,
                clean_background: 0.0,
            }
        );
        assert_eq!(wild.held().check(), Ok(()));
    }

    #[test]
    fn the_solid_matte_is_met_first_and_the_garbage_matte_wins() {
        let with = |garbage: Option<f32>, solid: Option<f32>| Incoming {
            garbage,
            solid,
            mask: None,
        };
        for level in [0.0, 0.25, 0.8, 1.0] {
            // A matte that is not there, and one of 0, change nothing, to the bit.
            assert_eq!(met(level, &with(None, None), false), level);
            assert_eq!(met(level, &with(Some(0.0), Some(0.0)), false), level);
            // A solid matte of 1 makes the matte solid, and a garbage matte of 1 transparent.
            assert_eq!(met(level, &with(None, Some(1.0)), false), 1.0);
            assert_eq!(met(level, &with(Some(1.0), None), false), 0.0);
            // Where both are 1 the garbage matte wins.
            assert_eq!(met(level, &with(Some(1.0), Some(1.0)), false), 0.0);
            // Read the other way round, the garbage matte takes where it is 0, and a garbage
            // matte that is not there is not there either way.
            assert_eq!(met(level, &with(Some(0.0), None), true), 0.0);
            assert_eq!(met(level, &with(Some(1.0), None), true), level);
            assert_eq!(met(level, &with(None, None), true), level);
            // What is beyond 0 and 1, or no number, is read within them.
            assert_eq!(met(level, &with(Some(7.0), None), false), 0.0);
            assert_eq!(met(level, &with(Some(-2.0), Some(f32::NAN)), false), level);
        }
        // Between their ends: the solid matte fills its share of what is not solid, and the
        // garbage matte takes its share of what is left.
        about(met(0.5, &with(None, Some(0.5)), false), 0.75);
        about(met(0.5, &with(Some(0.5), None), false), 0.25);
        about(met(0.5, &with(Some(0.5), Some(0.5)), false), 0.375);
        about(met(0.5, &with(Some(0.25), None), true), 0.125);
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "each view of a pixel, with the mattes and the mask that come in"
    )]
    fn a_node_shows_its_view_of_a_pixel_with_what_comes_into_it() {
        let nothing = Incoming::default();
        let edge = [0.4, 0.6, 0.4, 1.0];
        let rgb = [edge[0], edge[1], edge[2]];
        // An opaque pixel alone is what the formulas make of its colour, to the bit.
        for view in View::ALL {
            let shows = node(|n| n.view = view);
            let k = shows.keyer();
            let pre = pre_matte(&k, rgb);
            assert_eq!(
                node_pixel(&shows, edge, pre, pre, &nothing),
                shown(&k, view, rgb),
                "{view:?}"
            );
        }
        // A picture seen through in part is keyed as what is seen of it: its colour not
        // premultiplied, and the result times the alpha that came in.
        let plain = Node::default();
        let k = plain.keyer();
        let pre = pre_matte(&k, rgb);
        let whole = result(&k, rgb);
        let half = edge.map(|value| 0.5 * value);
        all_about(
            node_pixel(&plain, half, pre, pre, &nothing),
            whole.map(|value| 0.5 * value),
        );
        assert_eq!(unpremultiplied(half), (rgb, 0.5));
        // What is not seen at all is nothing, whatever its channels hold; but its mattes
        // are shown opaque, those of black, which is kept.
        for unseen in [
            [0.3, 0.9, 0.2, 0.0],
            [0.3, 0.9, 0.2, -1.0],
            [0.3, 0.9, 0.2, f32::NAN],
        ] {
            assert_eq!(unpremultiplied(unseen), ([0.0; 3], 0.0));
            assert_eq!(node_pixel(&plain, unseen, 1.0, 1.0, &nothing), [0.0; 4]);
            let matte = node(|n| n.view = View::Matte);
            assert_eq!(node_pixel(&matte, unseen, 1.0, 1.0, &nothing), [1.0; 4]);
        }
        // The Matte view is of the matte before the solid and garbage mattes; the status and
        // the final result are of the matte after them.
        let garbage = Incoming {
            garbage: Some(1.0),
            ..nothing
        };
        let solid = Incoming {
            solid: Some(1.0),
            ..nothing
        };
        let of = |view: View, incoming: &Incoming| {
            node_pixel(&node(|n| n.view = view), edge, pre, pre, incoming)
        };
        assert_eq!(of(View::Matte, &garbage), [pre, pre, pre, 1.0]);
        assert_eq!(of(View::PreMatte, &garbage), [pre, pre, pre, 1.0]);
        assert_eq!(of(View::Status, &garbage), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(of(View::Status, &solid), [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(of(View::FinalResult, &garbage), [0.0; 4]);
        assert_eq!(
            of(View::FinalResult, &solid),
            result_with(&k, rgb, pre, 1.0)
        );
        // Read the other way round, a garbage matte of 1 takes nothing.
        let inverted = node(|n| n.garbage_invert = true);
        assert_eq!(
            node_pixel(&inverted, edge, pre, pre, &garbage),
            result(&k, rgb)
        );
        // The effect mask mixes what the view shows with the pixel as it came in, and keeps
        // both of its ends to the bit.
        let masked = |mask: f32| Incoming {
            mask: Some(mask),
            ..nothing
        };
        assert_eq!(node_pixel(&plain, edge, pre, pre, &masked(0.0)), edge);
        assert_eq!(node_pixel(&plain, edge, pre, pre, &masked(-3.0)), edge);
        assert_eq!(node_pixel(&plain, edge, pre, pre, &masked(1.0)), whole);
        assert_eq!(node_pixel(&plain, edge, pre, pre, &masked(2.0)), whole);
        let mixed = node_pixel(&plain, edge, pre, pre, &masked(0.25));
        all_about(
            mixed,
            [0, 1, 2, 3].map(|i| edge[i] + (whole[i] - edge[i]) * 0.25),
        );
        // A screen colour with no channel above its other two keys nothing, and thresholds
        // that have crossed are a step at the lower.
        let grey = node(|n| n.settings.screen = [0.0; 3]);
        assert_eq!(pre_matte(&grey.keyer(), GREEN), 1.0);
        let crossed = node(|n| {
            n.settings.low = 0.75;
            n.settings.high = 0.25;
        });
        assert_eq!(thresholds(&crossed.keyer(), 0.75), 0.0);
        assert_eq!(thresholds(&crossed.keyer(), 0.76), 1.0);
    }

    #[test]
    fn erode_dilate_moves_the_edge_of_a_matte_and_the_matte_blur_softens_it() {
        let image = halved();
        // As the key made it: transparent on the screen, solid on the grey.
        let plain = row(&image, Finesse::default());
        assert!(plain[..12].iter().all(|level| *level == 0.0));
        assert!(plain[12..].iter().all(|level| *level == 1.0));
        // Eroded by two pixels, the two columns of the grey next to the screen go; dilated,
        // the two of the screen next to the grey are taken in.
        let eroded = row(
            &image,
            Finesse {
                erode_dilate: -2.0,
                ..Finesse::default()
            },
        );
        assert!(eroded[..14].iter().all(|level| *level == 0.0), "{eroded:?}");
        assert!(eroded[14..].iter().all(|level| *level == 1.0), "{eroded:?}");
        let dilated = row(
            &image,
            Finesse {
                erode_dilate: 2.0,
                ..Finesse::default()
            },
        );
        assert!(
            dilated[..10].iter().all(|level| *level == 0.0),
            "{dilated:?}"
        );
        assert!(
            dilated[10..].iter().all(|level| *level == 1.0),
            "{dilated:?}"
        );
        // Blurred, the edge is a ramp about where it was, alike on its two sides, and what
        // is far from it is as it was.
        let soft = row(
            &image,
            Finesse {
                matte_blur: 2.0,
                ..Finesse::default()
            },
        );
        assert!(soft[11] > 0.1 && soft[11] < 0.5, "{soft:?}");
        assert!(soft[12] > 0.5 && soft[12] < 0.9, "{soft:?}");
        assert!((soft[11] + soft[12] - 1.0).abs() < 2e-3, "{soft:?}");
        assert!(soft.windows(2).all(|pair| pair[0] <= pair[1]), "{soft:?}");
        assert_eq!((soft[0], soft[23]), (0.0, 1.0));
        // The one control alone, by its formula.
        assert_eq!(super::eroded(0.5, 0.2, 0.0), 0.5);
        assert_eq!(super::eroded(0.5, 0.2, 3.0), 1.0);
        about(super::eroded(0.5, 0.03, 3.0), 0.6);
        assert_eq!(super::eroded(0.5, 0.8, -3.0), 0.0);
        about(super::eroded(0.5, 0.98, -3.0), 0.5);
        about(super::eroded(1.0, 0.98, -3.0), 0.6);
        assert_eq!(softened(0.375), 0.375);
        assert_eq!(softened(1.5), 1.0);
    }

    #[test]
    fn the_cleans_take_specks_out_and_fill_holes_and_the_pre_blur_softens_the_key() {
        // A speck of grey on the screen, and a hole of the screen in the grey.
        let speck = picture(|x, y| if (x, y) == (12, 8) { [0.5; 3] } else { GREEN });
        let gap = picture(|x, y| if (x, y) == (12, 8) { GREEN } else { [0.5; 3] });
        let cleans = |foreground: f32, background: f32| Finesse {
            clean_foreground: foreground,
            clean_background: background,
            ..Finesse::default()
        };
        assert_eq!(row(&speck, Finesse::default())[12], 1.0);
        assert_eq!(row(&gap, Finesse::default())[12], 0.0);
        // Clean Background takes the speck, and leaves the hole; Clean Foreground fills the
        // hole, and leaves the speck.
        assert!(row(&speck, cleans(0.0, 1.0))[12] < 0.01);
        assert_eq!(row(&gap, cleans(0.0, 1.0))[12], 0.0);
        assert!(row(&gap, cleans(1.0, 0.0))[12] > 0.99);
        assert_eq!(row(&speck, cleans(1.0, 0.0))[12], 1.0);
        // What is far from either is as it was.
        assert_eq!(row(&speck, cleans(1.0, 1.0))[0], 0.0);
        assert_eq!(row(&gap, cleans(1.0, 1.0))[0], 1.0);
        // The one control alone, by its formula: at 0 it does nothing.
        assert_eq!(cleaned(0.75, 0.3, 0.0, 0.0), 0.75);
        about(cleaned(1.0, 0.25, 0.0, 0.5), 0.5);
        about(cleaned(0.0, 0.75, 0.5, 0.0), 0.5);
        // The Pre-Blur blurs what is keyed: the pre-matte of the edge is a ramp, the share of
        // the grey in the blurred picture, and the matte is that.
        let image = halved();
        let blurred = Finesse {
            pre_blur: 2.0,
            ..Finesse::default()
        };
        let k = Keyer::of(&Settings::default());
        let made = mattes(&k, &blurred, &image, 1.0);
        let at = |x: usize| made[8 * 24 + x];
        assert!(at(11)[0] > 0.1 && at(11)[0] < 0.5, "{:?}", at(11));
        assert!(at(12)[0] > 0.5 && at(12)[0] < 0.9, "{:?}", at(12));
        assert_eq!(at(11)[0], at(11)[1]);
        assert_eq!((at(0), at(23)), ([0.0, 0.0], [1.0, 1.0]));
        // A keyer that keys nothing has a matte of 1 all over, whatever these are.
        let idle = Keyer::of(&Settings {
            gain: 0.0,
            ..Settings::default()
        });
        let every = Finesse {
            pre_blur: 3.0,
            erode_dilate: -4.0,
            matte_blur: 3.0,
            clean_foreground: 1.0,
            clean_background: 1.0,
        };
        assert!(mattes(&idle, &every, &image, 1.0)
            .iter()
            .all(|both| *both == [1.0, 1.0]));
        // And a picture is made of its mattes pixel by pixel.
        let shows = node(|n| n.finesse = blurred);
        let whole = node_picture(&shows, &image, 1.0, &|_| Incoming::default());
        assert_eq!(whole.len(), image.pixels.len());
        let pixel = 8 * 24 + 12;
        let [pre, matte] = made[pixel];
        assert_eq!(
            whole[pixel],
            node_pixel(
                &shows,
                image.pixels[pixel],
                pre,
                matte,
                &Incoming::default()
            )
        );
    }
}
