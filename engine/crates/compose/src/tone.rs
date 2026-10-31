//! The tone mappings of colour management, its DRTs (display rendering transforms), and the
//! matrices, gains and limits of its gamuts, as operations of a stage ([`ColorOp`]), each with
//! its reference on the processor.
//!
//! # Where they work
//!
//! On linear light, where 1.0 is 100 nits: the scale of OCIO's configs, whose curves of PQ put 1.0
//! at 100 nits and whose encodings of SDR put white at 1.0 (`columbina_ocio`). Colour
//! management converts a picture into such a space (linear Rec.2020), does these, and converts
//! it on; a DRT is told the luma coefficients of that space.
//!
//! # The curve (ours: Resolve publishes none of its DRTs)
//!
//! RM20 gives one rule for its conversions between SDR and HDR (pp. 232–233): from HDR to SDR,
//! "0-50 nits … mapped to 0-50 nits (no change)", "51 to 100 nits … remapped from 51-90 nits",
//! "101 to 1000 nits … remapped from 91-100 nits"; from SDR to HDR the reverse. Every DRT here
//! maps by that rule made a curve and made general: a picture whose most is `S` nits mapped to a
//! most of `T` (`S > T`) goes through
//!
//! ```text
//! f(x) = x                                   x ≤ T/2       (no change below the knee)
//!        Hermite (T/2, T/2, 1) → (T, 0.9 T, d_T)           T/2 ≤ x ≤ T
//!        Hermite (T, 0.9 T, d_T) → (S, T, d_S)             T ≤ x ≤ S
//!        T + d_S (x − S)                     x ≥ S         (beyond the most, on as straight)
//! ```
//!
//! cubic segments of Hermite between the knots of the rule (for 1000 to 100: 50 → 50, 100 → 90,
//! 1000 → 100, exactly), whose slopes are 1 at the knee (so that the curve leaves the line of no
//! change without a corner), at `T` the harmonic mean of the slopes of the chords on either side
//! (as Fritsch and Carlson's monotone interpolation takes it), and at `S` that of the chord before
//! it, `d_S = 0.1 T / (S − T)`. With those slopes each segment is monotone (Fritsch–Carlson:
//! `α² + β² ≤ 9`, here at most 5.6 and 5), so the curve is monotone, and smooth: its slope is
//! continuous everywhere (C¹). Mapping the other way (`S < T`: SDR to HDR, with Use Inverse DRT,
//! p. 242) is the inverse of the curve that maps `T` to `S`, solved by halving on the processor
//! and the card alike; so 50 → 50, 90 → 100 and 100 → 1000 for 100 to 1000, as the rule says.
//!
//! So nothing changes below half the most of where it goes: below 100 nits for any target of
//! 200 nits or more (an HDR timeline or output), below 50 nits for one of 100. The tests hold the
//! curve to that, to its knots, to being monotone and to the continuity of its slope.
//!
//! # The DRTs (pp. 239–242)
//!
//! - **Simple** ("a good mapping for color transforms from HDR to SDR", p. 239): the curve on
//!   each of R, G and B, from about 5500 nits to 100 (p. 2089: "maps between approximately 5500
//!   nits and 100 nits"), whatever the most of the source; so what is above 5500 nits is still
//!   clipped by the output. In general from 55 times the target, at most 10000. Each channel
//!   mapped on its own, colours bright enough lose saturation and shift in hue, as a curve of
//!   RGB does.
//! - **Luminance Mapping** ("Same as `DaVinci`, but more accurate when … a single
//!   standards-based color space"): the curve on luminance (the luma of the linear space), each
//!   channel scaled by as much, so that hue and saturation keep their ratios; no desaturation.
//! - **`DaVinci`**: Luminance Mapping, and "controlled desaturation of image values in the very
//!   brightest and darkest parts": towards grey, from none at the target's most to all at the
//!   source's (`1 − smoothstep(T, S, Y)`), and in the shadows a quarter at black, to none at 1%
//!   of the target (`0.75 + 0.25 smoothstep(0, 0.01 T, Y)`). Mapping up, the highlights keep
//!   their colour.
//! - **Saturation Preserving**: Luminance Mapping, without desaturation but that of its Sat.
//!   Rolloff (p. 240): none below Start, all at Limit, smoothly between (`1 − smoothstep(Start,
//!   Limit, Y)`), of the luminance of the source in nits (of the result, mapping up).
//!
//! All four keep values below black as they are (a luminance of 0 or less is not mapped).

use crate::color::ColorOp;

/// The nits of 1.0 of the linear light that the operations here work on.
pub const NITS_PER_UNIT: f32 = 100.0;

/// The most of a source that Simple maps from, to a target of `T` nits: 55 times it (5500 to
/// 100, p. 2089), at most the 10000 of PQ.
const SIMPLE_REACH: f32 = 55.0;

/// The most nits of PQ.
const MOST_NITS: f32 = 10_000.0;

/// The part of the target's most that the shadows of `DaVinci` lose their saturation below,
/// and how much of it they keep at black.
const SHADOW_REACH: f32 = 0.01;
const SHADOW_KEPT: f32 = 0.75;

/// Which DRT.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Simple,
    Luminance,
    Davinci,
    /// Saturation Preserving, with its Sat. Rolloff Start and Limit in nits.
    SaturationPreserving,
}

impl Kind {
    fn code(self) -> f32 {
        match self {
            Kind::Simple => 0.0,
            Kind::Luminance => 1.0,
            Kind::Davinci => 2.0,
            Kind::SaturationPreserving => 3.0,
        }
    }
}

/// The curve that maps a most of `peak` nits down to `top` (`peak > top`): its knots and slopes
/// (see the module's documentation).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Curve {
    pub knee: f32,
    pub top: f32,
    pub peak: f32,
    /// The slope at `top` and at `peak`.
    pub slope_top: f32,
    pub slope_peak: f32,
}

impl Curve {
    /// The curve that maps `peak` nits to `top`; none where `peak` is not above it.
    #[must_use]
    pub fn new(peak: f32, top: f32) -> Option<Curve> {
        if !(peak > top && top > 0.0 && peak.is_finite()) {
            return None;
        }
        let knee = 0.5 * top;
        let before = (0.9 * top - knee) / (top - knee);
        let after = (0.1 * top) / (peak - top);
        Some(Curve {
            knee,
            top,
            peak,
            slope_top: 2.0 / (1.0 / before + 1.0 / after),
            slope_peak: after,
        })
    }

    /// `x` nits mapped down.
    #[must_use]
    pub fn compress(&self, x: f32) -> f32 {
        let at_top = 0.9 * self.top;
        if x <= self.knee {
            x
        } else if x <= self.top {
            hermite(
                x,
                [self.knee, self.top],
                [self.knee, at_top],
                [1.0, self.slope_top],
            )
        } else if x <= self.peak {
            hermite(
                x,
                [self.top, self.peak],
                [at_top, self.top],
                [self.slope_top, self.slope_peak],
            )
        } else {
            self.top + self.slope_peak * (x - self.peak)
        }
    }

    /// `x` nits mapped up: what [`Curve::compress`] maps to `x`.
    #[must_use]
    pub fn expand(&self, x: f32) -> f32 {
        if x <= self.knee {
            return x;
        }
        if x >= self.top {
            return self.peak + (x - self.top) / self.slope_peak;
        }
        let (mut low, mut high) = (self.knee, self.peak);
        for _ in 0..HALVINGS {
            let middle = f32::midpoint(low, high);
            if self.compress(middle) < x {
                low = middle;
            } else {
                high = middle;
            }
        }
        f32::midpoint(low, high)
    }

    /// The four numbers that the shader reads it by: knee, top, peak and the slope at top.
    fn params(&self) -> [f32; 4] {
        [self.knee, self.top, self.peak, self.slope_top]
    }
}

/// How many times the inverse of a curve is halved: from 10000 nits to well within a step of a
/// float.
const HALVINGS: u32 = 32;

/// A cubic of Hermite between `ends[0]` and `ends[1]`, from `values[0]` to `values[1]` with
/// slopes `slopes`.
fn hermite(at: f32, ends: [f32; 2], values: [f32; 2], slopes: [f32; 2]) -> f32 {
    let width = ends[1] - ends[0];
    let along = (at - ends[0]) / width;
    let square = along * along;
    let cube = square * along;
    (2.0 * cube - 3.0 * square + 1.0) * values[0]
        + (cube - 2.0 * square + along) * width * slopes[0]
        + (-2.0 * cube + 3.0 * square) * values[1]
        + (cube - square) * width * slopes[1]
}

fn smoothstep(low: f32, high: f32, x: f32) -> f32 {
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A DRT: a picture whose most is `from` nits mapped to a most of `to`, on linear light of a
/// space whose luma coefficients are `luma`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drt {
    pub kind: Kind,
    pub from: f32,
    pub to: f32,
    pub luma: [f32; 3],
    /// Sat. Rolloff Start and Limit, in nits, of Saturation Preserving.
    pub rolloff: [f32; 2],
}

impl Drt {
    /// The curve that it maps by, and whether it maps up (by the curve's inverse); none where
    /// it maps nothing (from as many nits as it maps to).
    #[must_use]
    pub fn curve(&self) -> Option<(Curve, bool)> {
        if self.kind == Kind::Simple && self.from > self.to {
            let reach = (SIMPLE_REACH * self.to).min(MOST_NITS).max(self.to * 1.01);
            return Curve::new(reach, self.to).map(|curve| (curve, false));
        }
        if self.from > self.to {
            Curve::new(self.from, self.to).map(|curve| (curve, false))
        } else {
            Curve::new(self.to, self.from).map(|curve| (curve, true))
        }
    }

    /// Whether it changes anything.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.curve().is_none()
    }

    /// The colour of linear light `rgb` mapped: the reference that the card is held to.
    #[must_use]
    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let Some((curve, up)) = self.curve() else {
            return rgb;
        };
        let map = |nits: f32| {
            if up {
                curve.expand(nits)
            } else {
                curve.compress(nits)
            }
        };
        if self.kind == Kind::Simple {
            return rgb.map(|value| map(value * NITS_PER_UNIT) / NITS_PER_UNIT);
        }
        let luminance =
            (rgb[0] * self.luma[0] + rgb[1] * self.luma[1] + rgb[2] * self.luma[2]) * NITS_PER_UNIT;
        if luminance <= 0.0 {
            return rgb;
        }
        let out = map(luminance);
        let scale = out / luminance;
        let saturation = match self.kind {
            Kind::Davinci => {
                let highlights = if up {
                    1.0
                } else {
                    1.0 - smoothstep(curve.top, curve.peak, luminance)
                };
                let reach = SHADOW_REACH * if up { curve.peak } else { curve.top };
                highlights * (SHADOW_KEPT + (1.0 - SHADOW_KEPT) * smoothstep(0.0, reach, luminance))
            }
            Kind::SaturationPreserving => {
                let measured = if up { out } else { luminance };
                1.0 - smoothstep(self.rolloff[0], self.rolloff[1], measured)
            }
            Kind::Simple | Kind::Luminance => 1.0,
        };
        let grey = out / NITS_PER_UNIT;
        rgb.map(|value| grey + (value * scale - grey) * saturation)
    }

    /// The operation of the card that does what [`Drt::apply`] does; none where it maps nothing.
    #[must_use]
    pub fn op(&self) -> Option<ColorOp> {
        let (curve, up) = self.curve()?;
        let reach = SHADOW_REACH * if up { curve.peak } else { curve.top };
        Some(ColorOp {
            body: "rgb = tone_drt(rgb, param($P), param($P + 1u), param($P + 2u), param($P + 3u));"
                .into(),
            library: WGSL.into(),
            params: vec![
                curve.params(),
                [
                    curve.slope_peak,
                    self.kind.code(),
                    if up { 1.0 } else { 0.0 },
                    NITS_PER_UNIT,
                ],
                [self.luma[0], self.luma[1], self.luma[2], 0.0],
                [self.rolloff[0], self.rolloff[1], reach, 0.0],
            ],
            ..ColorOp::default()
        })
    }
}

/// The WGSL of the DRTs, as [`Drt::apply`] does them.
pub const WGSL: &str = include_str!("tone.wgsl");

/// A matrix of 3 × 3, by its rows.
pub type Matrix = [[f64; 3]; 3];

/// An operation that multiplies `rgb` by `matrix`.
#[must_use]
#[allow(clippy::cast_possible_truncation, reason = "the card takes floats")]
pub fn matrix_op(matrix: &Matrix) -> ColorOp {
    let row = |r: [f64; 3]| [r[0] as f32, r[1] as f32, r[2] as f32, 0.0];
    ColorOp {
        body: "rgb = vec3<f32>(dot(param($P).xyz, rgb), dot(param($P + 1u).xyz, rgb), \
               dot(param($P + 2u).xyz, rgb));"
            .into(),
        params: vec![row(matrix[0]), row(matrix[1]), row(matrix[2])],
        ..ColorOp::default()
    }
}

/// An operation that holds each channel within `low` to `high`.
#[must_use]
pub fn clamp_op(low: f32, high: f32) -> ColorOp {
    ColorOp {
        body: "rgb = clamp(rgb, param($P).xyz, param($P + 1u).xyz);".into(),
        params: vec![[low, low, low, 0.0], [high, high, high, 0.0]],
        ..ColorOp::default()
    }
}

/// An operation that hard clips what is outside a smaller gamut (Limit Output Gamut To, p. 239):
/// `into` takes linear light into the smaller gamut, where what is below 0 is outside it and made
/// 0, and `back` takes it back.
#[must_use]
#[allow(clippy::cast_possible_truncation, reason = "the card takes floats")]
pub fn limit_op(into: &Matrix, back: &Matrix) -> ColorOp {
    let row = |r: [f64; 3]| [r[0] as f32, r[1] as f32, r[2] as f32, 0.0];
    let mut params: Vec<[f32; 4]> = into.iter().map(|r| row(*r)).collect();
    params.extend(back.iter().map(|r| row(*r)));
    ColorOp {
        body: "let limited = max(vec3<f32>(dot(param($P).xyz, rgb), dot(param($P + 1u).xyz, \
               rgb), dot(param($P + 2u).xyz, rgb)), vec3<f32>(0.0));\n        \
               rgb = vec3<f32>(dot(param($P + 3u).xyz, limited), dot(param($P + 4u).xyz, \
               limited), dot(param($P + 5u).xyz, limited));"
            .into(),
        params,
        ..ColorOp::default()
    }
}

/// `matrix` applied to `rgb`, on the processor.
#[must_use]
pub fn apply_matrix(matrix: &Matrix, rgb: [f64; 3]) -> [f64; 3] {
    matrix.map(|row| row[0] * rgb[0] + row[1] * rgb[1] + row[2] * rgb[2])
}

/// Hard clipping to a smaller gamut on the processor, as [`limit_op`] does it.
#[must_use]
pub fn apply_limit(into: &Matrix, back: &Matrix, rgb: [f64; 3]) -> [f64; 3] {
    apply_matrix(back, apply_matrix(into, rgb).map(|value| value.max(0.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const REC2020: [f32; 3] = [0.2627, 0.6780, 0.0593];

    fn drt(kind: Kind, from: f32, to: f32) -> Drt {
        Drt {
            kind,
            from,
            to,
            luma: REC2020,
            rolloff: [100.0, 1000.0],
        }
    }

    /// The pairs of peaks that the tests go through: down from HDR to SDR, between levels of
    /// HDR, and up.
    const PEAKS: [(f32, f32); 8] = [
        (1000.0, 100.0),
        (4000.0, 100.0),
        (10000.0, 100.0),
        (4000.0, 1000.0),
        (10000.0, 1000.0),
        (2000.0, 1000.0),
        (100.0, 1000.0),
        (100.0, 203.0),
    ];

    #[test]
    fn the_curve_from_1000_nits_to_100_is_the_rule_of_the_manual_at_its_knots() {
        let curve = Curve::new(1000.0, 100.0).unwrap();
        for (x, y) in [
            (0.0, 0.0),
            (25.0, 25.0),
            (50.0, 50.0),
            (100.0, 90.0),
            (1000.0, 100.0),
        ] {
            assert!(
                (curve.compress(x) - y).abs() < 1e-4,
                "{x} → {}",
                curve.compress(x)
            );
        }
        // And the other way, SDR to HDR (pp. 232–233): 0–50 kept, 90 → 100, 100 → 1000.
        for (x, y) in [(30.0, 30.0), (50.0, 50.0), (90.0, 100.0), (100.0, 1000.0)] {
            assert!(
                (curve.expand(x) - y).abs() < 1e-3 * y.max(1.0),
                "{x} → {}",
                curve.expand(x)
            );
        }
    }

    #[test]
    fn every_curve_is_monotone_smooth_and_changes_nothing_below_half_its_target() {
        for (from, to) in PEAKS {
            let (peak, top) = if from > to { (from, to) } else { (to, from) };
            let curve = Curve::new(peak, top).unwrap();
            let most = peak * 1.5;
            let steps = 20_000;
            let mut last = f32::MIN;
            #[allow(clippy::cast_precision_loss)]
            for i in 0..=steps {
                let x = most * i as f32 / steps as f32;
                let y = curve.compress(x);
                assert!(y >= last, "{from}→{to}: not monotone at {x}");
                last = y;
                if x <= 0.5 * top {
                    assert!((y - x).abs() <= 0.0, "{from}→{to}: {x} changed");
                }
            }
            // No corner at a knot: its slope from the left is its slope from the right, but for
            // what the curvature makes of a step of a thousandth.
            for knot in [curve.knee, curve.top, curve.peak] {
                let h = 1e-3 * knot;
                let left = (curve.compress(knot) - curve.compress(knot - h)) / h;
                let right = (curve.compress(knot + h) - curve.compress(knot)) / h;
                assert!(
                    (left - right).abs() <= 0.01 + 0.05 * left.abs(),
                    "{from}→{to}: a corner at {knot}: {left} then {right}"
                );
            }
            // The most maps to the top, and the inverse undoes it.
            assert!((curve.compress(peak) - top).abs() < 1e-3 * top);
            #[allow(clippy::cast_precision_loss)]
            for i in 0..=200 {
                let x = peak * 1.2 * i as f32 / 200.0;
                let back = curve.expand(curve.compress(x));
                assert!(
                    (back - x).abs() <= 2e-4 * x.max(1.0),
                    "{from}→{to}: {x} → {back}"
                );
            }
        }
    }

    #[test]
    fn below_half_the_lower_peak_nothing_changes() {
        // So below 100 nits wherever both sides are HDR, and below 50 between SDR and HDR.
        for (from, to) in PEAKS {
            let scale = 0.5 * from.min(to) / NITS_PER_UNIT;
            for kind in [
                Kind::Luminance,
                Kind::Davinci,
                Kind::SaturationPreserving,
                Kind::Simple,
            ] {
                let mut drt = drt(kind, from, to);
                // Saturation Preserving desaturates from its Start, which is the colourist's.
                drt.rolloff = [0.5 * from.min(to), MOST_NITS];
                // Colours whose every channel is below the knee, and whose luminance is above
                // the shadows that DaVinci desaturates.
                for rgb in [
                    [0.5, 0.5, 0.5],
                    [0.9, 0.2, 0.1],
                    [0.05, 0.6, 0.3],
                    [0.99, 0.99, 0.99],
                ] {
                    let rgb = rgb.map(|value: f32| value * scale);
                    let mapped = drt.apply(rgb);
                    for c in 0..3 {
                        assert!(
                            (mapped[c] - rgb[c]).abs() <= 1e-5 * rgb[c].max(1.0),
                            "{kind:?} {from}→{to}: {rgb:?} → {mapped:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn luminance_mapping_keeps_the_ratios_of_a_colour_and_davinci_greys_the_brightest() {
        let luminance = drt(Kind::Luminance, 10000.0, 100.0);
        let rgb = [40.0, 10.0, 5.0];
        let mapped = luminance.apply(rgb);
        assert!((mapped[0] / mapped[1] - 4.0).abs() < 1e-4);
        assert!((mapped[1] / mapped[2] - 2.0).abs() < 1e-4);
        let y = |c: [f32; 3]| c[0] * REC2020[0] + c[1] * REC2020[1] + c[2] * REC2020[2];
        let davinci = drt(Kind::Davinci, 10000.0, 100.0).apply(rgb);
        // The same luminance, less colourful.
        assert!((y(davinci) - y(mapped)).abs() < 1e-4);
        assert!(davinci[0] - davinci[2] < mapped[0] - mapped[2]);
        // At the most of the source: grey.
        let at_peak = drt(Kind::Davinci, 1000.0, 100.0).apply([20.0, 8.0, 5.0].map(|v| v * 1.0));
        let spread = at_peak
            .iter()
            .fold(0.0f32, |s, v| s.max((v - y(at_peak)).abs()));
        assert!(spread < 0.05, "{at_peak:?}");
        // Saturation Preserving keeps its colour up to its Start, and none past its Limit.
        let preserving = drt(Kind::SaturationPreserving, 10000.0, 100.0);
        let dim = [0.6, 0.3, 0.2];
        let kept = preserving.apply(dim);
        assert!((kept[0] / kept[1] - 2.0).abs() < 1e-4);
        let bright = preserving.apply([30.0, 15.0, 10.0]);
        assert!(
            bright.iter().all(|v| (v - y(bright)).abs() < 1e-4),
            "{bright:?}"
        );
        // Simple maps each channel: about 5500 nits to 100, whatever the source.
        let simple = drt(Kind::Simple, 1000.0, 100.0);
        assert!((simple.apply([55.0, 0.0, 0.0])[0] - 1.0).abs() < 1e-3);
        assert!(simple.apply([10.0, 10.0, 10.0])[0] < 1.0);
    }

    #[test]
    fn a_drt_between_the_same_nits_does_nothing() {
        assert!(drt(Kind::Davinci, 100.0, 100.0).is_identity());
        assert!(drt(Kind::Davinci, 100.0, 100.0).op().is_none());
        assert_eq!(
            drt(Kind::Simple, 100.0, 100.0).apply([3.0, 0.5, -0.2]),
            [3.0, 0.5, -0.2]
        );
    }

    #[test]
    fn values_below_black_are_kept() {
        for kind in [Kind::Luminance, Kind::Davinci, Kind::SaturationPreserving] {
            let rgb = [-0.1, -0.05, -0.2];
            assert_eq!(drt(kind, 10000.0, 100.0).apply(rgb), rgb);
        }
    }
}
