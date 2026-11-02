//! The Blur palette of a corrector (Reference Manual 20, pp. 3454–3456): Blur, Sharpen and Mist,
//! on the processor and, in `blur.wgsl`, on the card, line for line; the filters of the
//! neighbourhoods that it reads; and where its settings go in the parameters of its grade.
//!
//! # Where it is in a node
//!
//! After the rest of the node (its HSL curves, Soft Clip and shot match, `maths::secondary`) and
//! before its key: what the node made is blurred or sharpened whole, and then mixed with its
//! input by the key of its qualifier, its windows and the key coming in, as every other setting
//! of the node is (p. 3453: "limited as a secondary operation using HSL Qualifiers, Windows, or
//! Imported mattes"). So what a neighbourhood filters is the picture the node made, not that
//! picture already keyed: a window does not draw the black outside it into a blur at its edge.
//! (`DaVinci` puts its Defocus after Soft Clip, p. 3331; the shot match, which no palette shows, is
//! before it here, as it is already the last of the other steps.)
//!
//! # What each mode does
//!
//! Each channel `x` of the picture that the node made, with `n` that channel filtered about the
//! pixel (the near neighbourhood of the channel) and Radius `r`:
//!
//! - `r` = 0.5: `x`, to the bit ("no effect", p. 3454).
//! - `r` > 0.5: `n`, a Gaussian of [`BLUR_SIGMA`] `· t^`[`BLUR_POWER`] pixels of a frame 1080
//!   high, `t = (r − 0.5) / 0.5`: "raising the radius slider increases blur".
//! - `r` < 0.5: an unsharp mask, `x + a · w · D(x, n)`, `n` a Gaussian of [`SHARPEN_SIGMA`]
//!   pixels and `D` the detail ([`detail`]: `x − n` of both in half floats, as the textures of
//!   neighbourhoods keep them, less three steps of half floats, so that a flat field or a slow
//!   gradient has none at all, rather than what half floats round off, multiplied by up to 12),
//!   `a = `[`SHARPEN_GAIN`]` · u · s / 0.25`, `u = (0.5 − r) / 0.5`, so that 0 is the "maximum
//!   sharpness". `s` is Scaling, which "multiplies the amount of scaling being applied by
//!   the Radius control for sharpening operations" (p. 3455), 0.25 as it is at first; the Blur
//!   mode, whose Scaling is grey (p. 3454), takes it as 0.25.
//! - H/V Ratio `h` shares the filter between across and down, `2h` and `2(1 − h)` of it, as the
//!   Blur H/V Ratio of a key does (p. 3362): 0.5 alike, 1 across alone ("increasingly
//!   directional along the horizontal axis").
//!
//! Sharpen weighs the sharpening by `w`, which keeps out the pixels of little detail (p. 3455):
//! with `d` the most of `|D(x, n)|` of the channels sharpened, `w` is 1 from Level `L` on and 0 up
//! to `L − C` (0 at least), smoothly between, `C` Coring Softness, both scaled by
//! [`CORING_REACH`]: the softness blends the border below the threshold, so that Level 0 sharpens
//! the entire image ("The default setting of 0 sets the threshold low enough to sharpen the entire
//! image", p. 3455) whatever the softness. Elsewhere `w` is 1.
//!
//! Mist then mixes each channel with a wider blur of the picture, `m` the channel filtered by a
//! Gaussian of [`MIST_SIGMA`] pixels (the wide neighbourhood): `m + (y − m) · Mix / 100`, `y`
//! what the radius made. Mix 100 adds none; lowered, it "adds a superimposed blur that mixes with
//! the high-detail areas of the picture" (p. 3456), which the sharpening of a lowered Radius keeps
//! ("you first need to lower Radius to sharpen the image", p. 3456).
//!
//! `DaVinci` publishes none of these numbers; the shapes are the manual's, and the constants
//! below are guesses, kept here in one place to be set against `DaVinci` itself (T29).
//!
//! # Neighbourhoods
//!
//! A node reads a near neighbourhood for each different pair of Radius and H/V Ratio of its
//! channels whose Radius is ever other than 0.5 (one where they are ganged, as they are at
//! first), and a wide one for each different H/V Ratio where Mist's Mix is ever below 100 in the
//! Mist mode; each filters the whole colour, and a channel takes its own of it. A palette that
//! never changes the picture reads none and has no block of parameters, so that its grade is the
//! grade of before, to the bit; the mode, the gangs, and Scaling, Coring Softness, Level and Mix
//! where they do nothing change no shader.

#![allow(
    clippy::float_cmp,
    reason = "the maths of the shader, line for line: exact comparisons with neutral settings"
)]

use columbina_model::grade::{blur, Blur, BlurMode, Param};

use super::maths::{smoothstep, Params, Rgb};
use crate::color::Filter;
use crate::neighbourhood::half;

/// The maths of the palette on the card.
pub(crate) const LIBRARY: &str = include_str!("blur.wgsl");

/// The standard deviation of the blur of a Radius of 1.00, in pixels of a frame 1080 high
/// (p. 3454: "If you raise the Radius slider all the way to 1.00 and the image isn't blurred
/// enough, add another node").
pub const BLUR_SIGMA: f64 = 25.0;
/// How the blur grows from a Radius of 0.5 to 1: the power of `(r − 0.5) / 0.5`, so that the
/// blur starts from nothing and is fine near 0.5.
pub const BLUR_POWER: f64 = 2.0;
/// The standard deviation of the Gaussian that sharpening takes the detail against, in pixels of
/// a frame 1080 high: the finest detail of the picture.
pub const SHARPEN_SIGMA: f64 = 1.0;
/// How many times its detail a Radius of 0 adds, at a Scaling of 0.25.
pub const SHARPEN_GAIN: f64 = 3.0;
/// The detail `|x − n|` that a Level of 1 omits, and the softness of a Coring Softness of 1.
pub const CORING_REACH: f64 = 0.2;
/// The standard deviation of the superimposed blur of Mist, in pixels of a frame 1080 high.
pub const MIST_SIGMA: f64 = 12.0;

/// The size of the block of the palette, in `vec4`s: what each channel does (0 nothing, 1 blur,
/// 2 sharpen) and whether Sharpen's coring is on; how much each channel is sharpened; and Level,
/// Coring Softness, Mix and whether Mist is on.
pub const BLOCK: u32 = 3;

/// What a channel does at a frame.
const NOTHING: f32 = 0.0;
const BLURS: f32 = 1.0;
const SHARPENS: f32 = 2.0;

#[allow(
    clippy::cast_possible_truncation,
    reason = "the card computes in f32; the settings are within its range"
)]
fn f(value: f64) -> f32 {
    value as f32
}

fn channels(rgb: &columbina_model::grade::Rgb) -> [&Param; 3] {
    [&rgb.r, &rgb.g, &rgb.b]
}

/// The block of the palette at `frame`.
#[must_use]
#[allow(
    clippy::float_cmp,
    reason = "a Radius of 0.5 and a Mix of 100 change nothing, exactly"
)]
pub fn block(palette: &Blur, frame: f64) -> [[f32; 4]; 3] {
    let radius = channels(&palette.radius).map(|p| p.value_at(frame));
    let scaling = channels(&palette.scaling).map(|p| p.value_at(frame));
    let mut out = [[0.0_f32; 4]; 3];
    for i in 0..3 {
        let r = radius[i];
        let neutral = blur::RADIUS.default;
        out[0][i] = if r == neutral {
            NOTHING
        } else if r > neutral {
            BLURS
        } else {
            SHARPENS
        };
        if r < neutral {
            let scale = match palette.mode {
                BlurMode::Blur => 1.0,
                BlurMode::Sharpen | BlurMode::Mist => scaling[i] / blur::SCALING.default,
            };
            out[1][i] = f(SHARPEN_GAIN * (neutral - r) / neutral * scale);
        }
    }
    let level = palette.level.value_at(frame).max(0.0);
    let coring = palette.coring_softness.value_at(frame).max(0.0);
    let mix = palette.mix.value_at(frame);
    // Coring Softness without a Level keeps nothing out.
    let cored = palette.mode == BlurMode::Sharpen && level > 0.0;
    let mist = palette.mode == BlurMode::Mist && mix != blur::MIX.default;
    out[0][3] = if cored { 1.0 } else { 0.0 };
    out[2] = [
        f(level * CORING_REACH),
        f(coring * CORING_REACH),
        f((mix / 100.0).clamp(0.0, 1.0)),
        if mist { 1.0 } else { 0.0 },
    ];
    out
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "radii of filters, far within f32"
)]
fn reach(value: f64) -> f32 {
    if value.is_finite() {
        value.max(0.0) as f32
    } else {
        0.0
    }
}

/// A Gaussian of `sigma` shared between across and down by an H/V Ratio of `ratio`.
fn shared(sigma: f64, ratio: f64) -> Filter {
    let ratio = ratio.clamp(0.0, 1.0);
    Filter::Gaussian {
        sigma: [
            reach(sigma * 2.0 * ratio),
            reach(sigma * 2.0 * (1.0 - ratio)),
        ],
    }
}

/// The filter of the near neighbourhood of `channel` (0 red, 1 green, 2 blue) at `frame`: the
/// blur of its Radius above 0.5, the detail that sharpening takes below, and nothing at 0.5.
#[must_use]
pub fn near_filter(palette: &Blur, channel: usize, frame: f64) -> Filter {
    let r = channels(&palette.radius)[channel].value_at(frame);
    let h = channels(&palette.ratio)[channel].value_at(frame);
    let neutral = blur::RADIUS.default;
    let sigma = if r > neutral {
        BLUR_SIGMA * ((r - neutral) / (1.0 - neutral)).powf(BLUR_POWER)
    } else if r < neutral {
        SHARPEN_SIGMA
    } else {
        0.0
    };
    shared(sigma, h)
}

/// The filter of the wide neighbourhood of `channel` at `frame`: Mist's superimposed blur.
#[must_use]
pub fn wide_filter(palette: &Blur, channel: usize, frame: f64) -> Filter {
    let h = channels(&palette.ratio)[channel].value_at(frame);
    shared(MIST_SIGMA, h)
}

/// Neighbourhoods of a palette, each the first channel of it, whose settings give its filter,
/// and the channels that read it.
pub type Groups = Vec<(usize, [bool; 3])>;

/// `channel` put into the group of `groups` whose first channel is `like` it, or into a new one.
fn put(groups: &mut Groups, channel: usize, like: impl Fn(usize) -> bool) {
    if let Some((_, reads)) = groups.iter_mut().find(|(first, _)| like(*first)) {
        reads[channel] = true;
    } else {
        let mut reads = [false; 3];
        reads[channel] = true;
        groups.push((channel, reads));
    }
}

/// Which near and wide neighbourhoods the channels of `palette` read. Channels whose Radius never
/// changes read no near one, and none reads a wide one but in the Mist mode with a Mix ever below
/// 100.
#[must_use]
pub fn groups(palette: &Blur) -> (Groups, Groups) {
    let radius = channels(&palette.radius);
    let ratio = channels(&palette.ratio);
    let mut near = Groups::new();
    for channel in (0..3).filter(|&c| palette.radius_ever(c)) {
        put(&mut near, channel, |first| {
            radius[first] == radius[channel] && ratio[first] == ratio[channel]
        });
    }
    let mut wide = Groups::new();
    if palette.mist_ever() {
        for channel in 0..3 {
            put(&mut wide, channel, |first| ratio[first] == ratio[channel]);
        }
    }
    (near, wide)
}

/// The palette at the block `at` on the processor, as `grade_blur` does it on the card: `x` what
/// the node made, `near` and `wide` each channel of it filtered by its near and wide
/// neighbourhoods (`x` itself where a channel reads none).
pub(crate) fn apply(p: Params, at: u32, x: Rgb, near: Rgb, wide: Rgb) -> Rgb {
    let kind = p.at(at);
    let amount = p.at(at + 1);
    let more = p.at(at + 2);
    let detail = [0, 1, 2].map(|i| {
        if kind[i] == SHARPENS {
            detail(x[i], near[i])
        } else {
            0.0
        }
    });
    let mut w = 1.0;
    if kind[3] != 0.0 {
        let d = detail[0].abs().max(detail[1].abs().max(detail[2].abs()));
        w = core(d, more[0], more[1]);
    }
    let one = |i: usize| -> f32 {
        if kind[i] == BLURS {
            near[i]
        } else if kind[i] == SHARPENS {
            x[i] + amount[i] * w * detail[i]
        } else {
            x[i]
        }
    };
    let made = [one(0), one(1), one(2)];
    if more[3] == 0.0 {
        return made;
    }
    [0, 1, 2].map(|i| wide[i] + (made[i] - wide[i]) * more[2])
}

/// The most a half float keeps: the detail is taken of values within it, as on the card.
const HALF_MOST: f32 = 65504.0;

/// How many steps of half floats the detail leaves out either way. A card that rounds what it
/// writes towards 0 (the RTX 4080 under Direct3D 12 does) keeps a flat field in the texture of a
/// neighbourhood a step below `x` rounded to the nearest, and each of the two passes of its
/// filter may take another step off: three in all, measured so.
const DETAIL_STEPS: f32 = 3.0;

/// The detail of `x`, a channel sharpened, about `near`, its neighbourhood: `x` in half floats
/// less `near` in half floats, as the textures of neighbourhoods keep them, less three steps of
/// half floats either way ([`DETAIL_STEPS`]). Where the picture is flat, or a gradient is too slow
/// for half floats to tell, it is none, rather than what half floats round off (up to 12 times
/// that, sharpened, a banding), however the card rounds what it writes into those textures.
#[must_use]
pub fn detail(x: f32, near: f32) -> f32 {
    let kept = half(x.clamp(-HALF_MOST, HALF_MOST));
    let held = half(near.clamp(-HALF_MOST, HALF_MOST));
    let d = kept - held;
    let size = kept.abs().max(held.abs());
    let step =
        (f32::from_bits(size.to_bits() & 0x7F80_0000) * 2.0_f32.powi(-10)).max(2.0_f32.powi(-24));
    d - d.clamp(-DETAIL_STEPS * step, DETAIL_STEPS * step)
}

/// How much a pixel whose detail is `d` is sharpened, with a Level of `level` and a Coring
/// Softness of `soft` (both scaled): all of it from the Level on, none below `level − soft` (0 at
/// least), smoothly between; all of it everywhere where the Level is 0.
#[must_use]
pub fn core(d: f32, level: f32, soft: f32) -> f32 {
    if level <= 0.0 {
        return 1.0;
    }
    let low = (level - soft).max(0.0);
    if low < level {
        return smoothstep(low, level, d);
    }
    if d >= level {
        1.0
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_filters_grow_from_nothing_at_half_and_share_by_the_ratio() {
        let mut palette = Blur::default();
        assert_eq!(
            near_filter(&palette, 0, 0.0),
            Filter::Gaussian { sigma: [0.0; 2] }
        );
        palette.radius.r = Param::Fixed(1.0);
        #[allow(clippy::cast_possible_truncation, reason = "a constant")]
        let most = BLUR_SIGMA as f32;
        assert_eq!(
            near_filter(&palette, 0, 0.0),
            Filter::Gaussian { sigma: [most; 2] }
        );
        palette.radius.r = Param::Fixed(0.75);
        let Filter::Gaussian { sigma } = near_filter(&palette, 0, 0.0) else {
            panic!("a Gaussian")
        };
        assert!((sigma[0] - most / 4.0).abs() < 1e-4, "{sigma:?}");
        palette.radius.r = Param::Fixed(0.2);
        palette.ratio.r = Param::Fixed(1.0);
        #[allow(clippy::cast_possible_truncation, reason = "a constant")]
        let fine = SHARPEN_SIGMA as f32;
        assert_eq!(
            near_filter(&palette, 0, 0.0),
            Filter::Gaussian {
                sigma: [2.0 * fine, 0.0]
            }
        );
        palette.ratio.g = Param::Fixed(0.0);
        #[allow(clippy::cast_possible_truncation, reason = "a constant")]
        let wide = MIST_SIGMA as f32;
        assert_eq!(
            wide_filter(&palette, 1, 0.0),
            Filter::Gaussian {
                sigma: [0.0, 2.0 * wide]
            }
        );
    }

    #[test]
    fn coring_keeps_out_the_detail_below_its_level_softly() {
        // Level 0 sharpens the whole image (p. 3455), whatever the softness.
        for soft in [0.0, 0.1, 1.0] {
            assert_eq!(core(0.0, 0.0, soft), 1.0);
            assert_eq!(core(0.3, 0.0, soft), 1.0);
        }
        assert_eq!(core(0.05, 0.1, 0.0), 0.0);
        assert_eq!(core(0.15, 0.1, 0.0), 1.0);
        // The softness is a band below the Level, from L − C up to L.
        assert_eq!(core(0.0, 0.1, 0.05), 0.0);
        assert_eq!(core(0.05, 0.1, 0.05), 0.0);
        assert_eq!(core(0.1, 0.1, 0.05), 1.0);
        assert!((core(0.075, 0.1, 0.05) - 0.5).abs() < 1e-6);
        assert_eq!(core(0.12, 0.1, 0.05), 1.0);
        // Wider than the Level, the band stops at 0.
        assert_eq!(core(0.0, 0.1, 0.3), 0.0);
        assert!((core(0.05, 0.1, 0.3) - 0.5).abs() < 1e-6);
        assert_eq!(core(0.1, 0.1, 0.3), 1.0);
    }

    #[test]
    fn a_level_of_one_omits_detail_up_to_the_reach_of_coring() {
        let mut palette = Blur {
            mode: BlurMode::Sharpen,
            ..Blur::default()
        };
        palette.radius.r = Param::Fixed(0.0);
        // Level 0.5 omits detail below 0.1.
        palette.level = Param::Fixed(0.5);
        let block = block(&palette, 0.0);
        let sharpened = |near: f32| {
            let x = [0.5, 0.0, 0.0];
            apply(Params(&block), 0, x, [near, 0.0, 0.0], x)[0]
        };
        assert_eq!(sharpened(0.41), 0.5, "a detail of 0.09 is kept out");
        assert!(sharpened(0.38) > 0.6, "a detail of 0.12 is sharpened");
    }

    #[test]
    fn mix_superimposes_nothing_but_in_the_mist_mode() {
        for mode in [BlurMode::Blur, BlurMode::Sharpen] {
            let mut palette = Blur {
                mode,
                ..Blur::default()
            };
            palette.mix = Param::Fixed(50.0);
            assert_eq!(block(&palette, 0.0)[2][3], 0.0, "{mode:?}");
        }
        let mut palette = Blur {
            mode: BlurMode::Mist,
            ..Blur::default()
        };
        assert_eq!(block(&palette, 0.0)[2][3], 0.0, "a Mix of 100");
        palette.mix = Param::Fixed(50.0);
        assert_eq!(block(&palette, 0.0)[2][3], 1.0);
    }

    #[test]
    fn coring_softness_alone_turns_no_coring_on() {
        let mut palette = Blur {
            mode: BlurMode::Sharpen,
            ..Blur::default()
        };
        palette.radius.r = Param::Fixed(0.2);
        palette.coring_softness = Param::Fixed(0.5);
        assert_eq!(block(&palette, 0.0)[0][3], 0.0);
        palette.level = Param::Fixed(0.1);
        assert_eq!(block(&palette, 0.0)[0][3], 1.0);
    }
}
