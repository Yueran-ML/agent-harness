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
    let mix = p.at(node + at::LUM_MIX)[3];
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

/// Shadows and Highlights (pp. 3063–3064), the first step of a corrector.
pub(crate) fn shadows(c: &Constants, p: Params, node: u32, x0: Rgb) -> Rgb {
    let features = whole(p.at(node + at::FEATURES)[0]);
    if features & at::ZONES_ON == 0 {
        return x0;
    }
    let zones = p.at(node + at::ZONES);
    let y = luma(c, x0);
    add(
        x0,
        splat(zones[0] * (1.0 - smoothstep(0.0, 0.5, y)) + zones[1] * smoothstep(0.5, 1.0, y)),
    )
}

/// The colour that the neighbourhood of Midtone Detail filters, of the input `x0` of its node:
/// its luma after Shadows and Highlights less a half, and the square of that, in R and G. Their
/// means about a pixel are the mean of luma there, less a half, and its spread
/// ([`midtone_detail`]); the half is taken away for the square to keep its precision in a
/// texture of half floats.
pub(crate) fn detail_source(c: &Constants, p: Params, node: u32, x0: Rgb) -> Rgb {
    let y = luma(c, shadows(c, p, node, x0)) - 0.5;
    [y, y * y, 0.0]
}

/// Midtone Detail (p. 3063) of `x`, `around` the mean about it of what [`detail_source`] gives
/// (see the documentation of [`crate::grade`]).
pub(crate) fn midtone_detail(c: &Constants, p: Params, node: u32, x: Rgb, around: Rgb) -> Rgb {
    let features = whole(p.at(node + at::FEATURES)[0]);
    if features & at::DETAIL_ON == 0 {
        return x;
    }
    let detail = p.at(node + at::DETAIL);
    let y = luma(c, x);
    let mean = around[0] + 0.5;
    let spread = (around[1] - around[0] * around[0]).max(0.0).sqrt();
    let t = y.clamp(0.0, 1.0);
    let midtones = 4.0 * t * (1.0 - t);
    let edges = smoothstep(0.0, detail[2], spread);
    let change = if detail[0] > 0.0 {
        detail[0] * detail[1] * edges
    } else {
        detail[0] * (1.0 - edges)
    };
    add(x, splat(change * midtones * (y - mean)))
}

/// A corrector from Color Boost up to its LUT (p. 3331): Color Boost, Temp and Tint, Lift,
/// Gamma and Gain with Lum Mix, Contrast, the custom curves, Offset and the Log wheels, Hue and
/// Saturation.
pub(crate) fn after_detail(c: &Constants, p: Params, node: u32, x0: Rgb) -> Rgb {
    let mut x = x0;
    // The steps that are at their neutral settings (each the identity there) are left out.
    let features = whole(p.at(node + at::FEATURES)[0]);
    let on = |feature: u32| features & feature != 0;
    let colour = p.at(node + at::COLOUR);
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
        let mix = p.at(node + at::LUM_MIX)[3];
        let o = [0, 1, 2].map(|i| lgg(x[i], lift[i], e[i], gain[i]));
        let y = lgg(luma(c, x), lift[3], e[3], gain[3]);
        x = add(o, splat(mix * (y - luma(c, o))));
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

/// A corrector up to its LUT (p. 3331), without its Midtone Detail, which needs the pixels about
/// each: Shadows and Highlights, and then [`after_detail`].
pub(crate) fn primary(c: &Constants, p: Params, node: u32, x0: Rgb) -> Rgb {
    after_detail(c, p, node, shadows(c, p, node, x0))
}

/// A corrector up to its LUT with its Midtone Detail, `around` the mean about the pixel of what
/// [`detail_source`] gives.
pub(crate) fn primary_detailed(c: &Constants, p: Params, node: u32, x0: Rgb, around: Rgb) -> Rgb {
    let x = midtone_detail(c, p, node, shadows(c, p, node, x0), around);
    after_detail(c, p, node, x)
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
// Qualifiers
// ---------------------------------------------------------------------------------------------

/// How much `v` is within the range `r` of a qualifier (low, high, low soft and high soft): 1
/// from low to high, falling off smoothly to 0 over the softness beyond each, at once where it
/// is 0.
#[must_use]
pub fn range(v: f32, r: [f32; 4]) -> f32 {
    let mut k = 1.0;
    if v < r[0] {
        k = if r[2] > 0.0 {
            smoothstep(r[0] - r[2], r[0], v)
        } else {
            0.0
        };
    }
    if v > r[1] {
        k *= if r[3] > 0.0 {
            1.0 - smoothstep(r[1], r[1] + r[3], v)
        } else {
            0.0
        };
    }
    k
}

/// How much the hue `h` (in turns) is within the range of hue `r` (its centre, half its width,
/// and its softness on the left and on the right, in turns), going round.
#[must_use]
pub fn hue_range(h: f32, r: [f32; 4]) -> f32 {
    if r[1] >= 0.5 {
        return 1.0;
    }
    let mut d = h - r[0];
    d -= (d + 0.5).floor();
    let beyond = d.abs() - r[1];
    if beyond <= 0.0 {
        return 1.0;
    }
    let soft = if d > 0.0 { r[3] } else { r[2] };
    if soft <= 0.0 {
        return 0.0;
    }
    1.0 - smoothstep(0.0, soft, beyond)
}

/// The key that the qualifier of a corrector makes of the colour `x` that it is given, before
/// Matte Finesse: the product of how much each component that limits it is within its range.
pub(crate) fn qualify(c: &Constants, p: Params, node: u32, x: Rgb) -> f32 {
    let q = p.at(node + at::QUALIFIER);
    let mut k = 1.0;
    if q[0] == 1.0 {
        let on = p.at(node + at::QUALIFIER_RGB);
        if on[0] != 0.0 {
            k *= range(x[0].clamp(0.0, 1.0), p.at(node + at::QUALIFIER_RED));
        }
        if on[1] != 0.0 {
            k *= range(x[1].clamp(0.0, 1.0), p.at(node + at::QUALIFIER_GREEN));
        }
        if on[2] != 0.0 {
            k *= range(x[2].clamp(0.0, 1.0), p.at(node + at::QUALIFIER_BLUE));
        }
        return k;
    }
    if q[0] == 0.0 {
        if q[1] != 0.0 {
            k *= hue_range(hue(x), p.at(node + at::QUALIFIER_HUE));
        }
        if q[2] != 0.0 {
            k *= range(sat(x), p.at(node + at::QUALIFIER_SAT));
        }
    }
    if q[3] != 0.0 {
        k *= range(luma(c, x).clamp(0.0, 1.0), p.at(node + at::QUALIFIER_LUM));
    }
    k
}

fn features(p: Params, node: u32) -> u32 {
    whole(p.at(node + at::FEATURES)[0])
}

/// The colour that a qualifier keys: its corrector's input, or that input blurred, `around`,
/// where its Pre-Filter is on at the frame.
pub(crate) fn picked(p: Params, node: u32, input: Rgb, around: Rgb) -> Rgb {
    if features(p, node) & at::PRE_ON == 0 {
        return input;
    }
    around
}

/// Clean Black and Clean White of the key `q`, `m` the mean of the key about the pixel: a pixel
/// in a region keyed less than Clean Black is taken towards 0, as a speck is, and one in a region
/// keyed more than 1 less Clean White towards 1, as a hole is.
pub(crate) fn clean(p: Params, node: u32, q: f32, m: f32) -> f32 {
    if features(p, node) & at::CLEAN_ON == 0 {
        return q;
    }
    let f = p.at(node + at::FINESSE);
    let mut k = q;
    if f[0] > 0.0 {
        k *= smoothstep(0.0, f[0], m);
    }
    if f[1] > 0.0 {
        k = 1.0 - (1.0 - k) * smoothstep(0.0, f[1], 1.0 - m);
    }
    k
}

/// Black Clip and White Clip of the key `q`: its levels from Black Clip to White Clip made
/// those from 0 to 1.
pub(crate) fn key_clip(p: Params, node: u32, q: f32) -> f32 {
    let f = p.at(node + at::FINESSE);
    if f[2] == 0.0 && f[3] == 1.0 {
        return q;
    }
    if f[3] <= f[2] {
        return if q > f[2] { 1.0 } else { 0.0 };
    }
    ((q - f[2]) / (f[3] - f[2])).clamp(0.0, 1.0)
}

/// The key `q` blurred, `b` it through its Gaussian, with In/Out Ratio `t`: the blur kept on
/// both sides of the edge at 0, and at 1 (or -1) all of it outside (or inside) the edge, as
/// the blurred key from 0 to 1/2 (or 1/2 to 1) is stretched to 0 to 1.
pub(crate) fn spread(p: Params, node: u32, q: f32, b: f32) -> f32 {
    if features(p, node) & at::SPREAD_ON == 0 {
        return q;
    }
    let t = p.at(node + at::FINESSE_MORE)[0];
    let low = (-t).max(0.0) * 0.5;
    let high = 1.0 - t.max(0.0) * 0.5;
    ((b - low) / (high - low)).clamp(0.0, 1.0)
}

/// A pass of Morph Operation (`second` the second, of Opening and Closing) of the key `q`, `a`
/// the mean of the key in the box of Morph Radius about the pixel: grown, a pixel is keyed as
/// much as any of that box is, to [`super::layout::MORPH_EDGE`]; shrunk, as little.
pub(crate) fn morph(p: Params, node: u32, q: f32, a: f32, second: bool) -> f32 {
    if features(p, node) & at::MORPH_ON == 0 {
        return q;
    }
    let f = p.at(node + at::FINESSE_MORE);
    let grow = if second { f[2] } else { f[1] };
    if grow == 0.0 {
        q.min(1.0 - ((1.0 - a) / f[3]).clamp(0.0, 1.0))
    } else {
        q.max((a / f[3]).clamp(0.0, 1.0))
    }
}

/// The key of a qualifier refined: inverted where its palette says, and through the Key
/// palette's Qualifier controls.
pub(crate) fn qualified(p: Params, node: u32, q: f32) -> f32 {
    let k = if p.at(node + at::QUALIFIER_RGB)[3] == 0.0 {
        q
    } else {
        1.0 - q
    };
    key(k, p.at(node + at::KEY_QUALIFIER))
}

/// The key coming into a corrector, `k`, or it blurred, `around`, where its blur is on at the
/// frame.
pub(crate) fn key_blurred(p: Params, node: u32, k: f32, around: f32) -> f32 {
    if features(p, node) & at::KEY_BLUR_ON == 0 {
        return k;
    }
    around
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
