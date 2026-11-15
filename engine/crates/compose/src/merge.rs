//! The mathematics of the Merge node of the Fusion page (Reference Manual of Resolve 20,
//! pp. 2162–2170): what a pixel of a foreground laid over a pixel of a background is made into,
//! alone. It is written on the processor as a shader computes it, in `f32` and a pixel at a
//! time, and is the reference that the shader of the card is to be held to.
//!
//! # Pictures
//!
//! A pixel is R, G, B and A of linear light, premultiplied by its alpha, as the compositor lays
//! its layers ([`crate::keyer`], [`crate::mosaic`]). A colour may go beyond white and below
//! black, and nothing here holds it within them: what is above 1 comes out above 1. The two
//! pictures are of the same size, that of the background, which "determines the resolution of
//! the output" (p. 2163).
//!
//! # The controls
//!
//! Those of the daily tier, in the order of the Merge tab:
//!
//! - **Apply Mode** (pp. 2164–2166, [`Settings::apply`]): "the math used when blending or
//!   combining the foreground and background pixels". Normal is "the default" (p. 2164). The
//!   other modes are the composite modes of the timeline that the Layer Mixer has
//!   ([`Composite`], [`crate::grade::maths::composite`]), which are not written again here.
//! - **Operator** (pp. 2166–2167, [`Operator`]): "the formula used to combine pixels in the
//!   merge is always `(fg * x) + (bg * y)`", and the operator says what `x` and `y` are. Over
//!   alone is done: `x = 1, y = 1 − a`, `a` the alpha of the foreground.
//! - **Subtractive/Additive** (p. 2167 and pp. 1653–1654, [`Settings::subtractive_additive`]):
//!   Additive, the slider all the way to the right, is for a foreground that is premultiplied:
//!   it "obscures the background (by multiplying with the inverse of the foreground Alpha), and
//!   then simply adds the pixels from the foreground". Subtractive, all the way to the left, is
//!   for one that is not: "the foreground image is first multiplied by its own Alpha". Between
//!   them the slider blends the two merges.
//! - **Alpha Gain** (p. 2168, [`Settings::alpha_gain`]): "linearly scales the values of the
//!   foreground's Alpha channel".
//! - **Burn In** (p. 2168, [`Settings::burn_in`]): "the amount of Alpha used to darken the
//!   background, without affecting the amount of foreground added in".
//! - **Blend** (p. 2168, [`Settings::blend`]): "it will blend the background with the merged
//!   result".
//!
//! # The formulas
//!
//! The manual gives those of the operators and describes the rest in words. With `F` and `a`
//! the colour and the alpha of the foreground as they are given, `B` and `b` those of the
//! background, `s` Subtractive/Additive (1 is Additive), `g` Alpha Gain, `n` Burn In and `t`
//! Blend, a pixel is worked out in this order, which is the one that the words need and not
//! that of the tab:
//!
//! 1. **Alpha Gain** ([`gained`]): `a' = a · g`. It is this alpha that every later step reads.
//! 2. **Apply Mode** ([`applied`]): Normal leaves `F` as it is. Another mode gives
//!    `F · (1 − b) + a · b · M(B / b, F / a)`, `M` the mode's formula of the two colours not
//!    premultiplied (see the decisions).
//! 3. **Subtractive/Additive** ([`added`]): the colour that is added in is
//!    `C = F · (s + (1 − s) · a')`: `F` itself for Additive, to the bit, and `F · a'` for
//!    Subtractive.
//! 4. **Burn In** ([`held_out`]): the alpha that holds the background out is
//!    `h = a' · (1 − n)`.
//! 5. **Operator** ([`laid`]), Over: the colour is `C · x + B · y` and the alpha
//!    `h · x + b · y`, with `x = 1` and `y = 1 − h` ([`Operator::factors`]).
//! 6. **Blend** ([`blended`]): `B + (merged − B) · t` of each of R, G, B and A; the background
//!    to the bit at 0 and the merged result to the bit at 1.
//!
//! So Additive with an Alpha Gain of 1, no Burn In and a Blend of 1, which is how a Merge is at
//! first, is the over of a premultiplied foreground, `F + B · (1 − a)` with an alpha of
//! `a + b · (1 − a)`, to the bit; Subtractive is the over of a foreground that is not
//! premultiplied, `F · a + B · (1 − a)`; an Alpha Gain of 0 in an Additive merge is `F + B`,
//! "the foreground pixels are simply added to the background" (p. 2168); and a Blend of 0 is
//! the background untouched.
//!
//! # What the manual did not settle
//!
//! Each of these is ours, and is to be set against `DaVinci` itself:
//!
//! - **The blend of Subtractive and Additive** is done on the colour of the foreground. The two
//!   merges hold the background out alike and are linear in that colour, so that it is the same
//!   as blending their results.
//! - **Alpha Gain comes before the multiplication of a Subtractive merge**, which is so by the
//!   gained alpha. It is what makes the manual's two sentences true at once: "in Subtractive
//!   merges, this controls the density of the composite, similarly to Blend" (with an Alpha
//!   Gain of `g` the result is exactly that of a Blend of `g`, the alpha with it), and in
//!   Additive merges it only "reduces the amount that the background is obscured". An Alpha
//!   Gain of 0 is so a pure add of the foreground in an Additive merge, and the background as
//!   it was in a Subtractive one.
//! - **The alpha of the result is that of the over of the alpha that holds the background
//!   out**: `h + b · (1 − h)`. Alpha Gain so scales the alpha of the foreground wherever it is
//!   read, and "for Additive merges, increasing the Burn In gives an identical result to
//!   decreasing Alpha Gain" (p. 2168) is true of the alpha as of the colour: a Burn In of `n`
//!   is an Alpha Gain of `1 − n`. With a Burn In of 1 the alpha of the result is that of the
//!   background. (The other readings: the alpha of the foreground added as it is,
//!   `a' + b · (1 − h)`, which goes above 1; or left out of Burn In, `a' + b · (1 − a')`.)
//! - **Burn In leaves the colour that is added as it is**, the multiplication of a Subtractive
//!   merge too: at 1 "the foreground is effectively added onto the background (after Alpha
//!   multiplication if in Subtractive mode)".
//! - **The other Apply Modes** are laid as the Layer Mixer lays its layers, by the formulas of
//!   the W3C's Compositing and Blending: the share `b` of the foreground that lies over the
//!   background is made what the mode makes of the two colours, and the rest of it is left.
//!   Over an opaque background it is the mix of the Layer Mixer, `B + (M − B) · a`; over none
//!   it is the foreground as it was. Normal the same way would be `F` again, and is not
//!   computed. The colours are divided by the alphas that they were given with, not by the
//!   gained one, and a pixel with no alpha has no colour to give the mode. The manual has it
//!   that Screen ignores the alpha (p. 2164); that is not followed.
//! - **Subtractive/Additive is read as Additive in every mode but Normal** ([`Settings::held`]):
//!   the slider "disappears when you choose any other Apply Mode option besides Normal, because
//!   the math would be invalid" (p. 1654).
//! - **The ranges**: the manual gives none. Each slider is from 0 to 1 here. Of what a Merge is
//!   at first the manual gives Normal (p. 2164) and Additive (p. 2167); Over, an Alpha Gain of
//!   1, no Burn In and a Blend of 1 are ours, the values that leave a plain over.
//!
//! # What is not a number
//!
//! Nothing is read but the two pixels, so that a channel that is no number stays in its pixel.
//! It is not made a number either, as a shader does not: in the foreground it is in the same
//! channel of the result, and in an alpha it is in all four. One in the background shows
//! through an opaque foreground, 0 times it being no number. A Blend of 0 gives the background
//! whatever the foreground is.
//!
//! # What is not here
//!
//! - The Center, Size and Angle of the foreground (p. 2164), its Filter Method (p. 2168), Edges,
//!   Invert Transform and Flatten Transform (p. 2169): the foreground is given as it lies over
//!   the background, pixel for pixel, already moved, and transparent where it has no picture.
//! - The effect mask (p. 2162), which shows the merge "where the mask is white" and the
//!   background alone where it is black: it is to be the Blend of each pixel, times the slider.
//! - The operators of the advanced tier ([`Operator`]), the Apply Modes that the timeline has
//!   not (Dissolve, p. 2164; Hypotenuse and Geometric, p. 2166), and the Z-depth merge of the
//!   Channels tab (p. 2170).
//!
//! # On the card
//!
//! One fragment for each pixel, with `textureLoad` of the two pictures and no sampler. The
//! shader is given the settings as [`Settings::held`] makes them: the number of the Apply Mode
//! ([`crate::grade::layout::composite`]) and the four sliders. The other modes are
//! `grade_composite` of `grade/corrector.wgsl`, which reads the weights of luma of the working
//! space as this reads them from [`Constants`]. The ends of Blend are to be done with `select`,
//! so that the background and the merged result are kept to the bit, and the division of a
//! colour by its alpha likewise, with nothing where the alpha is not above 0.

use columbina_model::grade::Composite;

use crate::grade::layout;
use crate::grade::maths::{self, Constants, Rgb};

/// How the foreground and the background are combined (Operator, pp. 2166–2167): "the formula
/// used to combine pixels in the merge is always `(fg * x) + (bg * y)`", and each operator is an
/// `x` and a `y`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Operator {
    /// The foreground over the background: `x = 1, y = 1 − a`, `a` the alpha of the foreground.
    #[default]
    Over,
    // In, Held Out, Atop, XOr, Conjoint, Disjoint, Mask, Stencil and Under are of the advanced
    // tier and are not done. Each is another `x` and `y` (pp. 2166–2167), most of which read the
    // alpha of the background too, which `factors` is then to be given; Conjoint and Disjoint
    // choose theirs by comparing the two alphas.
}

impl Operator {
    /// The `x` and the `y` of `(fg * x) + (bg * y)` for a foreground whose alpha is `alpha`:
    /// the alpha that holds the background out ([`held_out`]).
    #[must_use]
    pub fn factors(self, alpha: f32) -> [f32; 2] {
        match self {
            Operator::Over => [1.0, 1.0 - alpha],
        }
    }
}

/// A setting that [`Settings::check`] refuses: no number, or one outside its range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the setting {what} is not a number from 0 to 1")]
pub struct OutOfRange {
    /// The setting, as a path: `merge.subtractive_additive`, `merge.alpha_gain`, `merge.burn_in`
    /// or `merge.blend`.
    pub what: &'static str,
}

/// The settings of a Merge (see the module).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Apply Mode (pp. 2164–2166): Normal, or a composite mode of the timeline.
    pub apply: Composite,
    /// Operator (pp. 2166–2167).
    pub operator: Operator,
    /// Subtractive/Additive (p. 2167), from 0 (Subtractive: the foreground is not premultiplied,
    /// and is multiplied by its alpha) to 1 (Additive: it is premultiplied, and is added as it
    /// is). It is read in the Normal mode alone.
    pub subtractive_additive: f32,
    /// Alpha Gain (p. 2168), from 0 to 1: what the alpha of the foreground is multiplied by.
    pub alpha_gain: f32,
    /// Burn In (p. 2168), from 0 (the background is held out by the whole alpha) to 1 (by none
    /// of it: the foreground is added onto the background).
    pub burn_in: f32,
    /// Blend (p. 2168), from 0 (the background as it is) to 1 (the merged result).
    pub blend: f32,
}

impl Default for Settings {
    /// Normal, which the manual has as "the default" (p. 2164), and Additive, which the slider
    /// "defaults to" (p. 2167). The manual does not give the others: Over (the first of its
    /// operators), an Alpha Gain of 1, a Burn In of 0 and a Blend of 1 are ours, to be checked
    /// against `DaVinci`. Together they are the over of a premultiplied foreground.
    fn default() -> Self {
        Self {
            apply: Composite::Normal,
            operator: Operator::Over,
            subtractive_additive: 1.0,
            alpha_gain: 1.0,
            burn_in: 0.0,
            blend: 1.0,
        }
    }
}

impl Settings {
    /// Check that each slider is a number from 0 to 1.
    ///
    /// # Errors
    ///
    /// The first of Subtractive/Additive, Alpha Gain, Burn In and Blend that is not.
    pub fn check(&self) -> Result<(), OutOfRange> {
        unit("merge.subtractive_additive", self.subtractive_additive)?;
        unit("merge.alpha_gain", self.alpha_gain)?;
        unit("merge.burn_in", self.burn_in)?;
        unit("merge.blend", self.blend)
    }

    /// The settings as the maths takes them, and as a shader is to be given them. Those that
    /// [`Settings::check`] passes are as they are, but for Subtractive/Additive, which is
    /// Additive wherever the Apply Mode is not Normal (the slider is shown in Normal alone,
    /// p. 1654). Of others, a slider out of its range is brought to its nearest end, and one
    /// that is not a number is read as 0.
    #[must_use]
    pub fn held(&self) -> Settings {
        Settings {
            apply: self.apply,
            operator: self.operator,
            subtractive_additive: if self.apply == Composite::Normal {
                level(self.subtractive_additive)
            } else {
                1.0
            },
            alpha_gain: level(self.alpha_gain),
            burn_in: level(self.burn_in),
            blend: level(self.blend),
        }
    }
}

/// Check that `value` is a number from 0 to 1.
fn unit(what: &'static str, value: f32) -> Result<(), OutOfRange> {
    if (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(OutOfRange { what })
    }
}

/// A slider as the maths takes it: within 0 and 1, and 0 where it is no number.
fn level(value: f32) -> f32 {
    if value > 0.0 {
        value.min(1.0)
    } else {
        0.0
    }
}

/// The colour of a pixel: its R, G and B.
fn rgb(pixel: [f32; 4]) -> Rgb {
    [pixel[0], pixel[1], pixel[2]]
}

/// A premultiplied colour divided by its alpha; nothing where the alpha is not above 0.
fn straight(colour: Rgb, alpha: f32) -> Rgb {
    if alpha > 0.0 {
        colour.map(|value| value / alpha)
    } else {
        [0.0; 3]
    }
}

/// The alpha of the foreground through Alpha Gain (p. 2168), which "linearly scales the values
/// of the foreground's Alpha channel": `alpha · alpha_gain`. It is the alpha that the rest of
/// the merge reads: what a Subtractive merge multiplies the colour by ([`added`]), and what
/// holds the background out ([`held_out`]).
#[must_use]
pub fn gained(alpha: f32, alpha_gain: f32) -> f32 {
    alpha * alpha_gain
}

/// The colour of the foreground as the Apply Mode makes it against the background (pp.
/// 2164–2166), premultiplied as the foreground is; `c` has the weights of luma that the modes
/// of hue, saturation, colour and luminosity read. Normal gives the colour of the foreground as
/// it is. Another mode gives `F · (1 − b) + a · b · M(B / b, F / a)`: the share `b`, the alpha
/// of the background, of the foreground is made what the mode makes of the two colours, each
/// divided by its own alpha, and the rest is left
/// ([`maths::composite`], which the Layer Mixer lays its layers with). A pixel with no alpha
/// has no colour to give the mode.
#[must_use]
pub fn applied(c: &Constants, mode: Composite, background: [f32; 4], foreground: [f32; 4]) -> Rgb {
    let front = rgb(foreground);
    if mode == Composite::Normal {
        return front;
    }
    let behind = background[3];
    let alpha = foreground[3];
    let mixed = maths::composite(
        c,
        layout::composite(mode),
        straight(rgb(background), behind),
        straight(front, alpha),
    );
    [0, 1, 2].map(|i| front[i] * (1.0 - behind) + alpha * behind * mixed[i])
}

/// The colour of the foreground that is added in (Subtractive/Additive, p. 2167):
/// `colour · (s + (1 − s) · alpha)`, `s` the slider and `alpha` the alpha of the foreground
/// through Alpha Gain ([`gained`]). Additive, 1, is `colour` as it is, to the bit: the
/// foreground is taken to be premultiplied. Subtractive, 0, is `colour · alpha`: it is taken
/// not to be, and "is first multiplied by its own Alpha". Between them it is the blend of the
/// two.
#[must_use]
pub fn added(colour: Rgb, alpha: f32, subtractive_additive: f32) -> Rgb {
    let share = subtractive_additive + (1.0 - subtractive_additive) * alpha;
    colour.map(|value| value * share)
}

/// The alpha that holds the background out (Burn In, p. 2168), "the amount of Alpha used to
/// darken the background": `alpha · (1 − burn_in)`, `alpha` the alpha of the foreground through
/// Alpha Gain ([`gained`]). At 0 it is that alpha, "a straight Alpha blend"; at 1 it is nothing,
/// and "the foreground is effectively added onto the background".
#[must_use]
pub fn held_out(alpha: f32, burn_in: f32) -> f32 {
    alpha * (1.0 - burn_in)
}

/// The foreground laid on the background by an operator (pp. 2166–2167): `(fg * x) + (bg * y)`
/// of R, G and B, `fg` being `colour`, the colour that is added in ([`added`]), and of the
/// alpha, `fg` being `alpha`, the alpha that holds the background out ([`held_out`]), which is
/// the alpha that `x` and `y` are of too ([`Operator::factors`]). For Over it is
/// `colour + background · (1 − alpha)`, and an alpha of `alpha + b · (1 − alpha)`.
#[must_use]
pub fn laid(operator: Operator, background: [f32; 4], colour: Rgb, alpha: f32) -> [f32; 4] {
    let [x, y] = operator.factors(alpha);
    [
        colour[0] * x + background[0] * y,
        colour[1] * x + background[1] * y,
        colour[2] * x + background[2] * y,
        alpha * x + background[3] * y,
    ]
}

/// The background blended with the merged result (Blend, p. 2168): `background` where `blend`
/// is 0 (or below it, or no number) and `merged` where it is 1 (or above), both to the bit, and
/// `background + (merged − background) · blend` of each of R, G, B and A between.
#[must_use]
pub fn blended(background: [f32; 4], merged: [f32; 4], blend: f32) -> [f32; 4] {
    if blend >= 1.0 {
        merged
    } else if blend > 0.0 {
        std::array::from_fn(|i| background[i] + (merged[i] - background[i]) * blend)
    } else {
        background
    }
}

/// A pixel of a merge whose settings are as [`Settings::held`] makes them: the stages of the
/// module, in their order.
fn merged(c: &Constants, background: [f32; 4], foreground: [f32; 4], s: &Settings) -> [f32; 4] {
    let alpha = gained(foreground[3], s.alpha_gain);
    let colour = added(
        applied(c, s.apply, background, foreground),
        alpha,
        s.subtractive_additive,
    );
    let over = laid(s.operator, background, colour, held_out(alpha, s.burn_in));
    blended(background, over, s.blend)
}

/// A pixel of the foreground merged with a pixel of the background, both premultiplied, and so
/// the result: Alpha Gain ([`gained`]), the Apply Mode ([`applied`]), Subtractive/Additive
/// ([`added`]), Burn In ([`held_out`]), the operator ([`laid`]) and Blend ([`blended`]). See the
/// module for each, and for why they are in this order. `c` has the weights of luma of the
/// working space, which only the Apply Modes of hue, saturation, colour and luminosity read.
/// The settings are taken as [`Settings::held`] makes them.
#[must_use]
pub fn merge_pixel(
    c: &Constants,
    background: [f32; 4],
    foreground: [f32; 4],
    settings: &Settings,
) -> [f32; 4] {
    merged(c, background, foreground, &settings.held())
}

/// A frame through a merge: `background` and `foreground` two pictures of the same size of
/// premultiplied R, G, B and A, row after row, and the result one more, each pixel
/// [`merge_pixel`] of the two at its place. The foreground is given as it lies over the
/// background: its Center, Size and Angle are not done here (see the module).
///
/// # Panics
///
/// Where the two pictures are not of the same size.
#[must_use]
pub fn merge(
    c: &Constants,
    background: &[[f32; 4]],
    foreground: &[[f32; 4]],
    settings: &Settings,
) -> Vec<[f32; 4]> {
    assert_eq!(
        background.len(),
        foreground.len(),
        "a foreground of the size of the background"
    );
    let settings = settings.held();
    background
        .iter()
        .zip(foreground)
        .map(|(back, front)| merged(c, *back, *front, &settings))
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::float_cmp,
        reason = "pixels kept to the bit, and sums and products that floats keep exactly"
    )]

    use super::*;
    use crate::grade::REC709_LUMA;

    /// The weights of luma that the tests merge with.
    const WEIGHTS: Constants = Constants {
        luma: REC709_LUMA,
        s_curve: false,
    };

    /// Pixels of every kind: nothing, opaque ones, ones seen through in part, and, the last
    /// two, one beyond white and one with a channel below black.
    const PIXELS: [[f32; 4]; 7] = [
        [0.0, 0.0, 0.0, 0.0],
        [0.5, 0.25, 0.125, 1.0],
        [0.3, 0.2, 0.1, 0.5],
        [0.05, 0.6, 0.15, 0.75],
        [0.1, 0.02, 0.07, 0.3],
        [4.0, 2.5, 8.0, 1.0],
        [-0.05, 0.02, 1.5, 0.25],
    ];

    /// The settings as they are first, set by `set`, which are checked.
    fn with(set: impl FnOnce(&mut Settings)) -> Settings {
        let mut settings = Settings::default();
        set(&mut settings);
        assert_eq!(settings.check(), Ok(()));
        settings
    }

    /// `foreground` merged with `background`.
    fn pixel(background: [f32; 4], foreground: [f32; 4], settings: &Settings) -> [f32; 4] {
        merge_pixel(&WEIGHTS, background, foreground, settings)
    }

    /// `foreground` over `background`, as a merge is at first.
    fn over(background: [f32; 4], foreground: [f32; 4]) -> [f32; 4] {
        pixel(background, foreground, &Settings::default())
    }

    #[track_caller]
    fn about(got: f32, wanted: f32) {
        assert!((got - wanted).abs() <= 1e-6, "{got} is not {wanted}");
    }

    #[track_caller]
    fn all_about(got: [f32; 4], wanted: [f32; 4]) {
        for (got, wanted) in got.into_iter().zip(wanted) {
            about(got, wanted);
        }
    }

    #[test]
    fn a_merge_is_at_first_the_over_of_a_premultiplied_foreground() {
        let first = Settings::default();
        assert_eq!(first.check(), Ok(()));
        assert_eq!(
            first,
            Settings {
                apply: Composite::Normal,
                operator: Operator::Over,
                subtractive_additive: 1.0,
                alpha_gain: 1.0,
                burn_in: 0.0,
                blend: 1.0,
            }
        );
        assert_eq!(first.held(), first);
        // The foreground, and the background where the foreground is not: to the bit.
        for back in PIXELS {
            for front in PIXELS {
                let rest = 1.0 - front[3];
                let wanted = [
                    front[0] + back[0] * rest,
                    front[1] + back[1] * rest,
                    front[2] + back[2] * rest,
                    front[3] + back[3] * rest,
                ];
                assert_eq!(over(back, front), wanted, "{front:?} over {back:?}");
            }
        }
    }

    #[test]
    fn an_opaque_foreground_covers_the_background_and_a_transparent_one_leaves_it() {
        for back in PIXELS {
            for front in [PIXELS[1], PIXELS[5]] {
                assert_eq!(front[3], 1.0);
                assert_eq!(over(back, front), front, "{front:?} over {back:?}");
            }
            assert_eq!(over(back, [0.0; 4]), back, "nothing over {back:?}");
        }
    }

    #[test]
    fn the_alpha_of_an_over_is_the_two_alphas_less_their_product() {
        for back in PIXELS {
            for front in PIXELS {
                let alpha = over(back, front)[3];
                about(alpha, front[3] + back[3] - front[3] * back[3]);
                assert!((0.0..=1.0).contains(&alpha), "{front:?} over {back:?}");
            }
        }
        assert_eq!(over([0.0, 0.0, 0.0, 0.5], [0.0, 0.0, 0.0, 0.5])[3], 0.75);
    }

    #[test]
    fn over_is_associative() {
        // The pixels within white; of the others an `f32` keeps less than a millionth.
        let soft = &PIXELS[..5];
        for &lowest in soft {
            for &middle in soft {
                for &highest in soft {
                    all_about(
                        over(over(lowest, middle), highest),
                        over(lowest, over(middle, highest)),
                    );
                }
            }
        }
    }

    #[test]
    fn subtractive_is_the_over_of_a_foreground_that_is_not_premultiplied() {
        let subtractive = with(|s| s.subtractive_additive = 0.0);
        for back in PIXELS {
            for front in PIXELS {
                let alpha = front[3];
                let rest = 1.0 - alpha;
                let wanted = [
                    front[0] * alpha + back[0] * rest,
                    front[1] * alpha + back[1] * rest,
                    front[2] * alpha + back[2] * rest,
                    alpha + back[3] * rest,
                ];
                assert_eq!(pixel(back, front, &subtractive), wanted);
                // Which is the Additive merge of the foreground premultiplied.
                let premultiplied = [front[0] * alpha, front[1] * alpha, front[2] * alpha, alpha];
                assert_eq!(pixel(back, front, &subtractive), over(back, premultiplied));
            }
        }
    }

    #[test]
    fn subtractive_darkens_a_premultiplied_foreground_by_its_alpha_once_more() {
        let back = [0.25, 0.5, 0.75, 1.0];
        // A half of [1.0, 0.5, 0.25], premultiplied.
        let front = [0.5, 0.25, 0.125, 0.5];
        let additive = over(back, front);
        let subtractive = pixel(back, front, &with(|s| s.subtractive_additive = 0.0));
        assert_eq!(additive, [0.625, 0.5, 0.5, 1.0]);
        assert_eq!(subtractive, [0.375, 0.375, 0.4375, 1.0]);
        // What is less is the colour of the foreground times what its alpha is short of 1: the
        // "darker edges" of p. 2167. The alpha is the same.
        for i in 0..3 {
            about(additive[i] - subtractive[i], front[i] * (1.0 - front[3]));
        }
        // Between the two, the slider blends them.
        let half = pixel(back, front, &with(|s| s.subtractive_additive = 0.5));
        assert_eq!(half, [0.5, 0.4375, 0.46875, 1.0]);
        // An opaque foreground is the same either way.
        for slider in [0.0, 0.25, 0.5, 1.0] {
            let settings = with(|s| s.subtractive_additive = slider);
            assert_eq!(pixel(back, PIXELS[5], &settings), PIXELS[5]);
        }
    }

    #[test]
    fn an_alpha_gain_of_0_adds_the_foreground_to_the_background() {
        let added_on = with(|s| s.alpha_gain = 0.0);
        for back in PIXELS {
            for front in PIXELS {
                let wanted = [
                    front[0] + back[0],
                    front[1] + back[1],
                    front[2] + back[2],
                    back[3],
                ];
                assert_eq!(pixel(back, front, &added_on), wanted);
            }
        }
        // Less Alpha Gain holds less of the background out, and leaves the foreground as it is.
        let half = pixel(PIXELS[1], PIXELS[2], &with(|s| s.alpha_gain = 0.5));
        all_about(
            half,
            [0.3 + 0.5 * 0.75, 0.2 + 0.25 * 0.75, 0.1 + 0.125 * 0.75, 1.0],
        );
    }

    #[test]
    fn alpha_gain_in_a_subtractive_merge_is_the_density_that_blend_is() {
        for gain in [0.0, 0.25, 0.6, 1.0] {
            let by_gain = with(|s| {
                s.subtractive_additive = 0.0;
                s.alpha_gain = gain;
            });
            let by_blend = with(|s| {
                s.subtractive_additive = 0.0;
                s.blend = gain;
            });
            // The pixels within white; of the others an `f32` keeps less than a millionth.
            for &back in &PIXELS[..5] {
                for &front in &PIXELS[..5] {
                    all_about(pixel(back, front, &by_gain), pixel(back, front, &by_blend));
                }
            }
        }
        // None of it is the background as it was.
        let none = with(|s| {
            s.subtractive_additive = 0.0;
            s.alpha_gain = 0.0;
        });
        assert_eq!(pixel(PIXELS[3], PIXELS[2], &none), PIXELS[3]);
    }

    #[test]
    fn burn_in_holds_less_of_the_background_out_and_adds_the_same_foreground() {
        // In an Additive merge, more Burn In is less Alpha Gain.
        for burn in [0.0, 0.25, 0.5, 1.0] {
            let burnt = with(|s| s.burn_in = burn);
            let gained_less = with(|s| s.alpha_gain = 1.0 - burn);
            for back in PIXELS {
                for front in PIXELS {
                    all_about(pixel(back, front, &burnt), pixel(back, front, &gained_less));
                }
            }
        }
        // All of it adds the foreground onto the background, whose alpha is the result's.
        let all = with(|s| s.burn_in = 1.0);
        let all_subtractive = with(|s| {
            s.burn_in = 1.0;
            s.subtractive_additive = 0.0;
        });
        for back in PIXELS {
            for front in PIXELS {
                let alpha = front[3];
                assert_eq!(
                    pixel(back, front, &all),
                    [
                        front[0] + back[0],
                        front[1] + back[1],
                        front[2] + back[2],
                        back[3]
                    ]
                );
                // After the multiplication by its alpha, in a Subtractive merge.
                assert_eq!(
                    pixel(back, front, &all_subtractive),
                    [
                        front[0] * alpha + back[0],
                        front[1] * alpha + back[1],
                        front[2] * alpha + back[2],
                        back[3]
                    ]
                );
            }
        }
        // Half of it: the foreground as it was, over the background held out by half the alpha.
        let half = pixel(PIXELS[1], PIXELS[2], &with(|s| s.burn_in = 0.5));
        all_about(
            half,
            [0.3 + 0.5 * 0.75, 0.2 + 0.25 * 0.75, 0.1 + 0.125 * 0.75, 1.0],
        );
    }

    #[test]
    fn blend_goes_from_the_background_untouched_to_the_merged_result() {
        let none = with(|s| s.blend = 0.0);
        let unlike = with(|s| {
            s.blend = 0.0;
            s.subtractive_additive = 0.25;
            s.alpha_gain = 0.5;
            s.burn_in = 0.75;
            s.apply = Composite::Screen;
        });
        for back in PIXELS {
            for front in PIXELS {
                assert_eq!(pixel(back, front, &none), back);
                assert_eq!(pixel(back, front, &unlike), back);
                // Between, each channel is as far from the background to the merged result.
                let whole = over(back, front);
                let half = pixel(back, front, &with(|s| s.blend = 0.5));
                let wanted: [f32; 4] =
                    std::array::from_fn(|i| back[i] + (whole[i] - back[i]) * 0.5);
                assert_eq!(half, wanted);
            }
        }
        assert_eq!(
            pixel(
                [0.0, 0.0, 0.0, 0.0],
                [0.5, 0.25, 0.125, 1.0],
                &with(|s| s.blend = 0.5)
            ),
            [0.25, 0.125, 0.0625, 0.5]
        );
        // The ends are to the bit, and a Blend beyond them or of no number is the nearer end
        // or the background.
        let back = PIXELS[3];
        let whole = PIXELS[4];
        assert_eq!(blended(back, whole, 0.0), back);
        assert_eq!(blended(back, whole, 1.0), whole);
        assert_eq!(blended(back, whole, -2.0), back);
        assert_eq!(blended(back, whole, 3.0), whole);
        assert_eq!(blended(back, whole, f32::NAN), back);
    }

    #[test]
    fn colours_beyond_white_and_below_black_are_not_held_within_them() {
        let back = [3.0, 6.0, 0.5, 1.0];
        let front = [2.0, 1.0, 4.0, 0.5];
        assert_eq!(over(back, front), [3.5, 4.0, 4.25, 1.0]);
        assert_eq!(
            pixel(back, front, &with(|s| s.blend = 0.5)),
            [3.25, 5.0, 2.375, 1.0]
        );
        assert_eq!(
            pixel(back, front, &with(|s| s.alpha_gain = 0.0)),
            [5.0, 7.0, 4.5, 1.0]
        );
        assert_eq!(
            pixel(back, front, &with(|s| s.subtractive_additive = 0.0)),
            [2.5, 3.5, 2.25, 1.0]
        );
        // Below black, in either picture.
        assert_eq!(
            over([-0.5, 0.0, -2.0, 1.0], [0.0; 4]),
            [-0.5, 0.0, -2.0, 1.0]
        );
        assert_eq!(
            over([0.0, 0.0, 0.0, 1.0], [-0.25, 0.5, -1.0, 0.5]),
            [-0.25, 0.5, -1.0, 1.0]
        );
    }

    #[test]
    fn a_channel_that_is_no_number_stays_in_its_pixel() {
        let back = [PIXELS[1], PIXELS[2], PIXELS[3], PIXELS[6]];
        let front = [PIXELS[4], PIXELS[3], PIXELS[2], PIXELS[1]];
        let settings = Settings::default();
        let clean = merge(&WEIGHTS, &back, &front, &settings);
        // A colour of the foreground: that channel of that pixel, and nothing else.
        let mut spoilt = front;
        spoilt[2][1] = f32::NAN;
        let out = merge(&WEIGHTS, &back, &spoilt, &settings);
        for (at, (got, wanted)) in out.iter().zip(&clean).enumerate() {
            if at == 2 {
                assert!(got[1].is_nan());
                assert_eq!(got[0], wanted[0]);
                assert_eq!(got[2..], wanted[2..]);
            } else {
                assert_eq!(got, wanted, "{at}");
            }
        }
        // An alpha of the foreground: the four channels of that pixel.
        let mut spoilt = front;
        spoilt[1][3] = f32::NAN;
        let out = merge(&WEIGHTS, &back, &spoilt, &settings);
        for (at, (got, wanted)) in out.iter().zip(&clean).enumerate() {
            if at == 1 {
                assert!(got.iter().all(|value| value.is_nan()), "{got:?}");
            } else {
                assert_eq!(got, wanted, "{at}");
            }
        }
        // A colour of the background: that channel of that pixel, an opaque foreground or not.
        let mut spoilt = back;
        spoilt[0][2] = f32::NAN;
        spoilt[3][0] = f32::NAN;
        let out = merge(&WEIGHTS, &spoilt, &front, &settings);
        for (at, (got, wanted)) in out.iter().zip(&clean).enumerate() {
            match at {
                0 => {
                    assert!(got[2].is_nan());
                    assert_eq!(got[..2], wanted[..2]);
                    assert_eq!(got[3], wanted[3]);
                }
                3 => {
                    assert_eq!(front[3][3], 1.0);
                    assert!(got[0].is_nan());
                    assert_eq!(got[1..], wanted[1..]);
                }
                _ => assert_eq!(got, wanted, "{at}"),
            }
        }
        // Under a Blend of 0 the foreground is not read into the result at all.
        let mut spoilt = front;
        spoilt[0] = [f32::NAN; 4];
        spoilt[1][3] = f32::INFINITY;
        let none = with(|s| s.blend = 0.0);
        assert_eq!(merge(&WEIGHTS, &back, &spoilt, &none), back);
    }

    #[test]
    fn a_frame_is_merged_pixel_by_pixel() {
        let settings = with(|s| {
            s.subtractive_additive = 0.5;
            s.alpha_gain = 0.75;
            s.burn_in = 0.25;
            s.blend = 0.5;
        });
        let back: Vec<[f32; 4]> = PIXELS.to_vec();
        let front: Vec<[f32; 4]> = PIXELS.iter().rev().copied().collect();
        let out = merge(&WEIGHTS, &back, &front, &settings);
        assert_eq!(out.len(), back.len());
        for (at, got) in out.iter().enumerate() {
            assert_eq!(*got, pixel(back[at], front[at], &settings), "{at}");
        }
        // Two pictures of no pixel make one of none.
        assert_eq!(merge(&WEIGHTS, &[], &[], &settings), Vec::<[f32; 4]>::new());
    }

    #[test]
    #[should_panic(expected = "a foreground of the size of the background")]
    fn a_foreground_of_another_size_is_not_merged() {
        let _ = merge(&WEIGHTS, &PIXELS[..3], &PIXELS[..2], &Settings::default());
    }

    #[test]
    fn each_stage_is_what_the_module_says() {
        assert_eq!(gained(0.5, 0.5), 0.25);
        assert_eq!(gained(0.75, 1.0), 0.75);
        assert_eq!(gained(0.75, 0.0), 0.0);
        let colour = [0.75, 0.5, 2.0];
        assert_eq!(added(colour, 0.5, 1.0), colour);
        assert_eq!(added(colour, 0.5, 0.0), [0.375, 0.25, 1.0]);
        assert_eq!(added(colour, 0.5, 0.5), [0.5625, 0.375, 1.5]);
        assert_eq!(held_out(0.5, 0.0), 0.5);
        assert_eq!(held_out(0.5, 0.5), 0.25);
        assert_eq!(held_out(0.5, 1.0), 0.0);
        assert_eq!(Operator::default(), Operator::Over);
        assert_eq!(Operator::Over.factors(0.25), [1.0, 0.75]);
        assert_eq!(
            laid(Operator::Over, [0.5, 1.0, 2.0, 1.0], colour, 0.25),
            [1.125, 1.25, 3.5, 1.0]
        );
        assert_eq!(
            laid(Operator::Over, [0.5, 1.0, 2.0, 0.5], colour, 0.0),
            [1.25, 1.5, 4.0, 0.5]
        );
        // The Normal mode leaves the colour of the foreground as it is.
        assert_eq!(
            applied(&WEIGHTS, Composite::Normal, PIXELS[3], PIXELS[2]),
            [0.3, 0.2, 0.1]
        );
    }

    #[test]
    fn another_apply_mode_mixes_the_two_colours_where_the_background_is() {
        // Both opaque: what the mode makes of the two colours.
        let multiply = with(|s| s.apply = Composite::Multiply);
        assert_eq!(
            pixel([0.5, 0.25, 1.0, 1.0], [0.5, 0.5, 0.25, 1.0], &multiply),
            [0.25, 0.125, 0.25, 1.0]
        );
        let back = [0.5, 0.25, 1.0, 1.0];
        // A half of [0.4, 0.2, 0.6], premultiplied.
        let front = [0.2, 0.1, 0.3, 0.5];
        for mode in [
            Composite::Multiply,
            Composite::Screen,
            Composite::Overlay,
            Composite::Difference,
            Composite::Add,
            Composite::Luminosity,
        ] {
            let settings = with(|s| s.apply = mode);
            // Over an opaque background, as the Layer Mixer lays a layer by its key.
            let mixed = maths::composite(
                &WEIGHTS,
                layout::composite(mode),
                [0.5, 0.25, 1.0],
                [0.4, 0.2, 0.6],
            );
            let wanted = [0, 1, 2].map(|i| back[i] + (mixed[i] - back[i]) * 0.5);
            all_about(
                pixel(back, front, &settings),
                [wanted[0], wanted[1], wanted[2], 1.0],
            );
            // Over no background, the foreground as it was.
            assert_eq!(pixel([0.0; 4], front, &settings), front, "{mode:?}");
            // And no foreground leaves the background.
            assert_eq!(pixel(back, [0.0; 4], &settings), back, "{mode:?}");
        }
        // Subtractive/Additive is of the Normal mode alone.
        let screen = |slider: f32| {
            with(|s| {
                s.apply = Composite::Screen;
                s.subtractive_additive = slider;
            })
        };
        assert_eq!(
            pixel(back, front, &screen(0.0)),
            pixel(back, front, &screen(1.0))
        );
        assert_eq!(screen(0.0).held().subtractive_additive, 1.0);
        assert_eq!(
            with(|s| s.subtractive_additive = 0.25)
                .held()
                .subtractive_additive,
            0.25
        );
    }

    #[test]
    fn check_refuses_each_slider_that_is_out_of_its_range() {
        let refused = |settings: Settings, what: &'static str| {
            assert_eq!(settings.check(), Err(OutOfRange { what }), "{settings:?}");
        };
        for value in [-0.1, 1.1, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let slider = Settings {
                subtractive_additive: value,
                ..Settings::default()
            };
            refused(slider, "merge.subtractive_additive");
            let gain = Settings {
                alpha_gain: value,
                ..Settings::default()
            };
            refused(gain, "merge.alpha_gain");
            let burn = Settings {
                burn_in: value,
                ..Settings::default()
            };
            refused(burn, "merge.burn_in");
            let blend = Settings {
                blend: value,
                ..Settings::default()
            };
            refused(blend, "merge.blend");
        }
        // The first that is out of its range is the one named.
        let several = Settings {
            alpha_gain: 2.0,
            burn_in: f32::NAN,
            blend: -1.0,
            ..Settings::default()
        };
        refused(several, "merge.alpha_gain");
        // The ends of each range are within it, whatever the Apply Mode.
        let least = Settings {
            apply: Composite::Overlay,
            operator: Operator::Over,
            subtractive_additive: 0.0,
            alpha_gain: 0.0,
            burn_in: 0.0,
            blend: 0.0,
        };
        assert_eq!(least.check(), Ok(()));
        let most = Settings {
            subtractive_additive: 1.0,
            alpha_gain: 1.0,
            burn_in: 1.0,
            blend: 1.0,
            ..least
        };
        assert_eq!(most.check(), Ok(()));
        let error = OutOfRange {
            what: "merge.burn_in",
        };
        assert_eq!(
            error.to_string(),
            "the setting merge.burn_in is not a number from 0 to 1"
        );
    }

    /// A setting of the model, which keeps doubles, as these settings take it.
    #[allow(clippy::cast_possible_truncation, reason = "a slider from 0 to 1")]
    fn single(value: f64) -> f32 {
        value as f32
    }

    /// The slider at a place in the order of the Merge tab: Subtractive/Additive, Alpha
    /// Gain, Burn In and Blend.
    fn slider(settings: &mut Settings, at: usize) -> &mut f32 {
        match at {
            0 => &mut settings.subtractive_additive,
            1 => &mut settings.alpha_gain,
            2 => &mut settings.burn_in,
            _ => &mut settings.blend,
        }
    }

    #[test]
    fn the_merge_of_the_model_has_these_settings_by_their_names_ranges_and_first_values() {
        use columbina_model::fusion;
        // What a Merge node of the model is at first, at any frame, is what these are at first.
        let merged = fusion::Merge::default().at(12.0);
        let first = Settings {
            apply: merged.apply,
            operator: match merged.operator {
                fusion::Operator::Over => Operator::Over,
            },
            subtractive_additive: single(merged.subtractive_additive),
            alpha_gain: single(merged.alpha_gain),
            burn_in: single(merged.burn_in),
            blend: single(merged.blend),
        };
        assert_eq!(first, Settings::default());
        // Its foreground lies where it is: in the middle, as large as it is and not turned.
        assert_eq!(
            (merged.center, merged.size, merged.angle),
            ([0.5, 0.5], 1.0, 0.0)
        );
        // Each slider of the model is the slider of that name here: as the model refuses it,
        // so do these, within the same range and with the same value at first.
        for (at, (what, bounds)) in fusion::Merge::SLIDERS.into_iter().enumerate() {
            let set = |value: f64| {
                let mut settings = Settings::default();
                *slider(&mut settings, at) = single(value);
                settings
            };
            assert_eq!(
                *slider(&mut Settings::default(), at),
                single(bounds.default),
                "{what}"
            );
            assert_eq!(set(bounds.low).check(), Ok(()), "{what}");
            assert_eq!(set(bounds.high).check(), Ok(()), "{what}");
            for beyond in [bounds.low - 0.5, bounds.high + 0.5] {
                assert_eq!(set(beyond).check(), Err(OutOfRange { what }), "{what}");
            }
        }
        // And every Apply Mode of the model is one of these: both are the composite modes of
        // the timeline, of one type.
        let screen = fusion::Merge {
            apply: Composite::Screen,
            ..fusion::Merge::default()
        };
        assert_eq!(screen.at(0.0).apply, Composite::Screen);
    }

    #[test]
    fn settings_that_are_refused_are_taken_as_the_nearest_within_their_ranges() {
        let wild = Settings {
            apply: Composite::Normal,
            operator: Operator::Over,
            subtractive_additive: f32::NAN,
            alpha_gain: 7.0,
            burn_in: -3.0,
            blend: f32::INFINITY,
        };
        assert!(wild.check().is_err());
        let held = Settings {
            subtractive_additive: 0.0,
            alpha_gain: 1.0,
            burn_in: 0.0,
            blend: 1.0,
            ..wild
        };
        assert_eq!(wild.held(), held);
        assert_eq!(held.check(), Ok(()));
        for back in PIXELS {
            for front in PIXELS {
                assert_eq!(pixel(back, front, &wild), pixel(back, front, &held));
            }
        }
        // A Blend that is no number is none: the background.
        let unblended = Settings {
            blend: f32::NAN,
            ..Settings::default()
        };
        assert_eq!(pixel(PIXELS[3], PIXELS[2], &unblended), PIXELS[3]);
    }
}
