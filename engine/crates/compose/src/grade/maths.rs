//! The maths of the nodes of a grade on the processor: the reference that `corrector.wgsl` is
//! held to, line for line, in `f32` as the card computes. The formulas, and the pages of the
//! manual whose behaviour each matches, are in the documentation of [`crate::grade`].
//!
//! A corrector reads its settings from the parameters of its grade, packed as
//! [`super::layout`] lays them out, so that the processor and the card read the same numbers.

#![allow(
    clippy::float_cmp,
    clippy::many_single_char_names,
    clippy::manual_midpoint,
    reason = "the maths of the shader, line for line: exact comparisons with neutral settings,               and the short names of the formulas"
)]

use std::f32::consts::{PI, TAU};

use super::layout as at;

/// A colour: R, G and B.
pub type Rgb = [f32; 3];

/// Half the width of the blend between two zones of the Log wheels.
pub const LOG_OVERLAP: f32 = 0.05;

const SQRT3_HALF: f32 = 0.866_025_4;
const INV_SQRT3: f32 = 0.577_350_26;

/// The parameters of an operation, as the card reads them (`param(i)`).
#[derive(Clone, Copy)]
pub(crate) struct Params<'a>(pub &'a [[f32; 4]]);

impl Params<'_> {
    fn at(&self, index: u32) -> [f32; 4] {
        // As the card reads a storage buffer beyond its end: something, and no fault.
        self.0.get(index as usize).copied().unwrap_or([0.0; 4])
    }

    fn rgb(&self, index: u32) -> Rgb {
        let [r, g, b, _] = self.at(index);
        [r, g, b]
    }
}

/// The constants of the options of a grade that the maths reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Constants {
    /// The weights of R, G and B in luma.
    pub luma: Rgb,
    /// Whether Contrast is an S-curve.
    pub s_curve: bool,
}

fn add(a: Rgb, b: Rgb) -> Rgb {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: Rgb, b: Rgb) -> Rgb {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: Rgb, k: f32) -> Rgb {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn splat(v: f32) -> Rgb {
    [v; 3]
}

/// Luma: the weighted sum of R, G and B.
#[must_use]
pub fn luma(c: &Constants, x: Rgb) -> f32 {
    c.luma[0] * x[0] + c.luma[1] * x[1] + c.luma[2] * x[2]
}

/// `x` to the power `e`, of the sign of `x`; `x` itself where `e` is 1.
#[must_use]
pub fn spow(x: f32, e: f32) -> f32 {
    if e == 1.0 || x == 0.0 {
        return x;
    }
    x.signum() * x.abs().powf(e)
}

/// The smooth step of WGSL.
#[must_use]
pub fn smoothstep(low: f32, high: f32, x: f32) -> f32 {
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The saturation of a colour, from 0 (grey) to 1: `(max - min) / max`.
#[must_use]
pub fn sat(x: Rgb) -> f32 {
    let most = x[0].max(x[1].max(x[2]));
    let least = x[0].min(x[1].min(x[2]));
    if most <= 0.0 {
        return 0.0;
    }
    ((most - least) / most).clamp(0.0, 1.0)
}

/// The hue of a colour, from 0 to 1: red 0, green 1/3, blue 2/3; 0 for a grey.
#[must_use]
pub fn hue(x: Rgb) -> f32 {
    let a = x[0] - 0.5 * (x[1] + x[2]);
    let b = SQRT3_HALF * (x[1] - x[2]);
    if a == 0.0 && b == 0.0 {
        return 0.0;
    }
    let h = b.atan2(a) / TAU;
    if h < 0.0 {
        h + 1.0
    } else {
        h
    }
}

/// A turn of the colour about the grey axis by an angle whose cosine less 1 is `cm1` and whose
/// sine is `s` (Rodrigues' formula about `(1, 1, 1) / √3`).
#[must_use]
pub fn rotate(x: Rgb, cm1: f32, s: f32) -> Rgb {
    let mean = (x[0] + x[1] + x[2]) / 3.0;
    let across = scale([x[2] - x[1], x[0] - x[2], x[1] - x[0]], INV_SQRT3);
    add(add(x, scale(sub(x, splat(mean)), cm1)), scale(across, s))
}

/// The colour with its difference from its luma multiplied by `k`.
#[must_use]
pub fn saturate(c: &Constants, x: Rgb, k: f32) -> Rgb {
    add(x, scale(sub(x, splat(luma(c, x))), k - 1.0))
}

/// Lift, Gamma (as an exponent) and Gain of one value.
#[must_use]
pub fn lgg(x: f32, lift: f32, e: f32, gain: f32) -> f32 {
    gain * spow(x + lift * (1.0 - x), e)
}

/// Contrast `k` about `pivot`: linear, or an S-curve that keeps black and white.
#[must_use]
pub fn contrast(c: &Constants, x: f32, k: f32, pivot: f32) -> f32 {
    if !c.s_curve {
        return x + (x - pivot) * (k - 1.0);
    }
    s_contrast(x, k, pivot)
}

/// Contrast `k` about `pivot` as an S-curve that keeps black and white, and the values beyond.
#[must_use]
pub fn s_contrast(x: f32, k: f32, pivot: f32) -> f32 {
    if k == 1.0 || x <= 0.0 || x >= 1.0 {
        return x;
    }
    if x < pivot {
        return pivot * (x / pivot).powf(k);
    }
    1.0 - (1.0 - pivot) * ((1.0 - x) / (1.0 - pivot)).powf(k)
}

/// A channel through a shot match: its exponent `e`, its gain and its offset, then the S-curve
/// of contrast `k` about `pivot` (`columbina_model::grade::ShotMatch`).
#[must_use]
pub fn shot_match(x: f32, e: f32, gain: f32, offset: f32, k: f32, pivot: f32) -> f32 {
    s_contrast(gain * spow(x, e) + offset, k, pivot)
}

/// The value at `x` of the curve of `count` points from `from` (each its x, its y and its
/// slope), held flat beyond its first and last points where `flat`, or carried on along their
/// slopes. Between two points it is the cubic of Hermite through them with their slopes; none
/// is 0.
#[must_use]
pub(crate) fn curve(p: Params, from: u32, count: u32, x: f32, flat: bool) -> f32 {
    if count == 0 {
        return 0.0;
    }
    let first = p.at(from);
    if x <= first[0] || count == 1 {
        return if flat {
            first[1]
        } else {
            first[1] + first[2] * (x - first[0])
        };
    }
    let last = p.at(from + count - 1);
    if x >= last[0] {
        return if flat {
            last[1]
        } else {
            last[1] + last[2] * (x - last[0])
        };
    }
    let mut i = 0;
    let mut j = 1;
    while j + 1 < count {
        if p.at(from + j)[0] <= x {
            i = j;
        } else {
            break;
        }
        j += 1;
    }
    let p0 = p.at(from + i);
    let p1 = p.at(from + i + 1);
    let dx = p1[0] - p0[0];
    let dy = p1[1] - p0[1];
    let t = (x - p0[0]) / dx;
    let t2 = t * t;
    let t3 = t2 * t;
    let h10 = t3 - 2.0 * t2 + t;
    let h11 = t3 - t2;
    p0[1] + dy * t + h10 * (dx * p0[2] - dy) + h11 * (dx * p1[2] - dy)
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "offsets and counts are small whole numbers, written as floats for the card"
)]
fn whole(value: f32) -> u32 {
    value as u32
}

/// What a custom curve changes `x` by: its intensity times its value less `x`.
fn custom(p: Params, node: u32, offset: f32, count: f32, intensity: f32, x: f32) -> f32 {
    intensity * (curve(p, node + whole(offset), whole(count), x, false) - x)
}

/// The custom curves (pp. 3111–3116), with Lum Mix (p. 3063) where they are not ganged.
pub(crate) fn curves(c: &Constants, p: Params, node: u32, x: Rgb) -> Rgb {
    let intensity = p.at(node + at::CURVE_INTENSITY);
    let offsets = p.at(node + at::CURVE_OFFSETS);
    let counts = p.at(node + at::CURVE_COUNTS);
    if p.at(node + at::CURVE_GANGED)[0] != 0.0 {
        return add(
            x,
            [
                custom(p, node, offsets[0], counts[0], intensity[1], x[0]),
                custom(p, node, offsets[0], counts[0], intensity[2], x[1]),
                custom(p, node, offsets[0], counts[0], intensity[3], x[2]),
            ],
        );
    }
    let mix = p.at(node + at::LGG_MEAN)[3];
    let by = |channel: usize| -> Rgb {
        x.map(|v| {
            custom(
                p,
                node,
                offsets[channel],
                counts[channel],
                intensity[channel],
                v,
            )
        })
    };
    let (dr, dg, db) = (by(1), by(2), by(3));
    let mut o = add(x, [dr[0], dg[1], db[2]]);
    let n = add(
        add(add(x, scale(dr, c.luma[0])), scale(dg, c.luma[1])),
        scale(db, c.luma[2]),
    );
    o = add(o, splat(mix * (luma(c, n) - luma(c, o))));
    let y = luma(c, o);
    add(
        o,
        splat(mix * custom(p, node, offsets[0], counts[0], intensity[0], y)),
    )
}

/// Offset, which is done as a part of the Log controls (p. 3073), and the Log wheels
/// (pp. 3068–3073): an offset of each zone of lightness.
pub(crate) fn log(c: &Constants, p: Params, node: u32, x0: Rgb) -> Rgb {
    let x = add(x0, p.rgb(node + at::OFFSET));
    let ranges = p.at(node + at::ZONES);
    let y = luma(c, x);
    let shadow = 1.0 - smoothstep(ranges[2] - LOG_OVERLAP, ranges[2] + LOG_OVERLAP, y);
    let highlight = smoothstep(ranges[3] - LOG_OVERLAP, ranges[3] + LOG_OVERLAP, y);
    let midtone = (1.0 - shadow - highlight).max(0.0);
    add(
        add(
            add(x, scale(p.rgb(node + at::LOG_SHADOW), shadow)),
            scale(p.rgb(node + at::LOG_MIDTONE), midtone),
        ),
        scale(p.rgb(node + at::LOG_HIGHLIGHT), highlight),
    )
}

/// A corrector up to its LUT (p. 3331): Shadows and Highlights, Color Boost, Temp and Tint,
/// Lift, Gamma and Gain with Lum Mix, Contrast, the custom curves, Offset and the Log wheels,
/// Hue and Saturation.
pub(crate) fn primary(c: &Constants, p: Params, node: u32, x0: Rgb) -> Rgb {
    let mut x = x0;
    // The steps that are at their neutral settings (each the identity there) are left out.
    let features = whole(p.at(node + at::FEATURES)[0]);
    let on = |feature: u32| features & feature != 0;
    let colour = p.at(node + at::COLOUR);
    if on(at::ZONES_ON) {
        let zones = p.at(node + at::ZONES);
        let y = luma(c, x);
        x = add(
            x,
            splat(zones[0] * (1.0 - smoothstep(0.0, 0.5, y)) + zones[1] * smoothstep(0.5, 1.0, y)),
        );
    }
    if on(at::BOOST_ON) {
        x = add(
            x,
            scale(sub(x, splat(luma(c, x))), colour[3] * (1.0 - sat(x))),
        );
    }
    if on(at::BALANCE_ON) {
        let balance = p.rgb(node + at::BALANCE);
        x = [x[0] * balance[0], x[1] * balance[1], x[2] * balance[2]];
    }
    if on(at::LGG_ON) {
        let lift = p.at(node + at::LIFT);
        let e = p.at(node + at::GAMMA);
        let gain = p.at(node + at::GAIN);
        let mean = p.at(node + at::LGG_MEAN);
        let mut o = [0, 1, 2].map(|i| lgg(x[i], lift[i], e[i], gain[i]));
        let n = x.map(|v| lgg(v, mean[0], mean[1], mean[2]));
        o = add(o, splat(mean[3] * (luma(c, n) - luma(c, o))));
        let y0 = luma(c, o);
        x = add(o, splat(mean[3] * (lgg(y0, lift[3], e[3], gain[3]) - y0)));
    }
    if on(at::CONTRAST_ON) {
        let k = p.at(node + at::CONTRAST);
        x = x.map(|v| contrast(c, v, k[0], k[1]));
    }
    if on(at::CURVES_ON) {
        x = curves(c, p, node, x);
    }
    if on(at::LOG_ON) {
        x = log(c, p, node, x);
    }
    if on(at::HUE_ON) {
        x = rotate(x, colour[1], colour[2]);
    }
    if on(at::SATURATION_ON) {
        x = saturate(c, x, colour[0]);
    }
    x
}

/// The low end of Soft Clip (p. 3120).
#[must_use]
pub fn clip_low(x: f32, low: f32, width: f32) -> f32 {
    if low <= 0.0 && width <= 0.0 {
        return x;
    }
    if width <= 0.0 {
        return x.max(low);
    }
    let knee = low + width;
    if x >= knee {
        return x;
    }
    knee - width * (1.0 - ((x - knee) / width).exp())
}

/// The high end of Soft Clip (pp. 3118–3119).
#[must_use]
pub fn clip_high(x: f32, high: f32, width: f32) -> f32 {
    if high >= 1.0 && width <= 0.0 {
        return x;
    }
    if width <= 0.0 {
        return x.min(high);
    }
    let knee = high - width;
    if x <= knee {
        return x;
    }
    knee + width * (1.0 - ((knee - x) / width).exp())
}

/// A corrector after its LUT: the HSL curves, which read the hue, saturation and luma of the
/// picture that came into the node (p. 3122), Soft Clip, and last the shot match (p. 3048).
pub(crate) fn secondary(c: &Constants, p: Params, node: u32, x0: Rgb, input: Rgb) -> Rgb {
    let mut x = x0;
    let features = whole(p.at(node + at::FEATURES)[0]);
    if features & at::HSL_ON != 0 {
        let offsets = p.at(node + at::HSL_OFFSETS);
        let offsets2 = p.at(node + at::HSL_OFFSETS_2);
        let counts = p.at(node + at::HSL_COUNTS);
        let counts2 = p.at(node + at::HSL_COUNTS_2);
        let h = hue(input);
        let s = sat(input);
        let l = luma(c, input).clamp(0.0, 1.0);
        let of =
            |offset: f32, count: f32, v: f32| curve(p, node + whole(offset), whole(count), v, true);
        if counts[0] > 0.0 {
            let turn = of(offsets[0], counts[0], h) * PI;
            x = rotate(x, turn.cos() - 1.0, turn.sin());
        }
        let k = (1.0 + of(offsets[1], counts[1], h))
            * (1.0 + of(offsets[3], counts[3], l))
            * (1.0 + of(offsets2[0], counts2[0], s));
        x = saturate(c, x, k);
        let light = of(offsets[2], counts[2], h) * s + of(offsets2[1], counts2[1], s);
        x = add(x, splat(luma(c, x) * light));
    }
    if features & at::CLIP_ON != 0 {
        let low = p.at(node + at::CLIP_LOW);
        let high = p.at(node + at::CLIP_HIGH);
        let low_width = p.at(node + at::CLIP_LOW_SOFT);
        let high_width = p.at(node + at::CLIP_HIGH_SOFT);
        x = [0, 1, 2]
            .map(|i| clip_high(clip_low(x[i], low[i], low_width[i]), high[i], high_width[i]));
    }
    if features & at::SHOT_ON != 0 {
        let e = p.at(node + at::SHOT_GAMMA);
        let gain = p.at(node + at::SHOT_GAIN);
        let offset = p.at(node + at::SHOT_OFFSET);
        x = [0, 1, 2].map(|i| shot_match(x[i], e[i], gain[i], offset[i], gain[3], offset[3]));
    }
    x
}

/// A key through the settings `p` of a key input or output: inverted where `p[0]` is not 0,
/// its contrast moved by the offset `p[2]` (white held), and multiplied by the gain `p[1]`.
#[must_use]
pub fn key(k: f32, p: [f32; 4]) -> f32 {
    let a = if p[0] == 0.0 { k } else { 1.0 - k };
    ((a + p[2] * (1.0 - a)) * p[1]).clamp(0.0, 1.0)
}

/// The key of a key mixer so far and the next input's (adjusted), in Matte mode (`matte` not 0:
/// where both are) or Mask mode (either).
#[must_use]
pub fn key_combine(sofar: f32, k: f32, matte: f32) -> f32 {
    if matte == 0.0 {
        (sofar + k).min(1.0)
    } else {
        sofar * k
    }
}

// ---------------------------------------------------------------------------------------------
// Composite modes
// ---------------------------------------------------------------------------------------------

fn clip_colour(c: &Constants, x: Rgb) -> Rgb {
    let l = luma(c, x);
    let least = x[0].min(x[1].min(x[2]));
    let most = x[0].max(x[1].max(x[2]));
    let mut o = x;
    if least < 0.0 && l - least > 0.0 {
        o = add(splat(l), scale(sub(o, splat(l)), l / (l - least)));
    }
    if most > 1.0 && most - l > 0.0 {
        o = add(splat(l), scale(sub(o, splat(l)), (1.0 - l) / (most - l)));
    }
    o
}

fn set_lum(c: &Constants, x: Rgb, l: f32) -> Rgb {
    clip_colour(c, add(x, splat(l - luma(c, x))))
}

fn chroma(x: Rgb) -> f32 {
    x[0].max(x[1].max(x[2])) - x[0].min(x[1].min(x[2]))
}

fn set_sat(x: Rgb, s: f32) -> Rgb {
    let range = chroma(x);
    if range <= 0.0 {
        return [0.0; 3];
    }
    scale(sub(x, splat(x[0].min(x[1].min(x[2])))), s / range)
}

fn dodge(b: f32, f: f32) -> f32 {
    if b <= 0.0 {
        return 0.0;
    }
    if f >= 1.0 {
        return 1.0;
    }
    (b / (1.0 - f)).min(1.0)
}

fn burn(b: f32, f: f32) -> f32 {
    if b >= 1.0 {
        return 1.0;
    }
    if f <= 0.0 {
        return 0.0;
    }
    1.0 - ((1.0 - b) / f).min(1.0)
}

fn soft_light(b: f32, f: f32) -> f32 {
    if f <= 0.5 {
        return b - (1.0 - 2.0 * f) * b * (1.0 - b);
    }
    let d = if b <= 0.25 {
        ((16.0 * b - 12.0) * b + 4.0) * b
    } else {
        b.max(0.0).sqrt()
    };
    b + (2.0 * f - 1.0) * (d - b)
}

fn hard_light(b: f32, f: f32) -> f32 {
    if f <= 0.5 {
        return 2.0 * b * f;
    }
    1.0 - 2.0 * (1.0 - b) * (1.0 - f)
}

fn vivid_light(b: f32, f: f32) -> f32 {
    if f <= 0.5 {
        return burn(b, 2.0 * f);
    }
    dodge(b, 2.0 * f - 1.0)
}

fn pin_light(b: f32, f: f32) -> f32 {
    if f <= 0.5 {
        return b.min(2.0 * f);
    }
    b.max(2.0 * f - 1.0)
}

fn divide(b: f32, f: f32) -> f32 {
    if f > 0.0 {
        return b / f;
    }
    if b > 0.0 {
        1.0
    } else {
        0.0
    }
}

/// A channel of `f` laid over `b` in the separable mode `mode`.
fn blend(mode: u32, b: f32, f: f32) -> f32 {
    match mode {
        1 | 16 => b + f,
        3 => burn(b, f),
        4 => dodge(b, f),
        5 => b.min(f),
        7 => (b - f).abs(),
        8 => divide(b, f),
        9 => b + f - 2.0 * b * f,
        10 => {
            if b + f >= 1.0 {
                1.0
            } else {
                0.0
            }
        }
        11 => hard_light(b, f),
        13 => b.max(f),
        15 => b + f - 1.0,
        17 => b + 2.0 * f - 1.0,
        19 => b * f,
        20 => hard_light(f, b),
        21 => pin_light(b, f),
        23 => b + f - b * f,
        24 => soft_light(b, f),
        25 => b - f,
        26 => vivid_light(b, f),
        _ => f,
    }
}

/// `f` laid over `b` in the composite mode numbered `mode` (see [`super::layout::composite`]).
#[must_use]
pub fn composite(c: &Constants, mode: u32, b: Rgb, f: Rgb) -> Rgb {
    match mode {
        2 => set_lum(c, f, luma(c, b)),
        6 => {
            if luma(c, f) < luma(c, b) {
                f
            } else {
                b
            }
        }
        12 => set_lum(c, set_sat(f, chroma(b)), luma(c, b)),
        14 => {
            if luma(c, f) > luma(c, b) {
                f
            } else {
                b
            }
        }
        18 => set_lum(c, b, luma(c, f)),
        22 => set_lum(c, set_sat(b, chroma(f)), luma(c, b)),
        _ => [0, 1, 2].map(|i| blend(mode, b[i], f[i])),
    }
}
