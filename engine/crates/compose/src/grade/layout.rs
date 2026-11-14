//! How the settings of a node are laid out in the parameters of its grade (`vec4`s), in the
//! units that the maths reads: the settings of the Color page (as the model keeps them) are
//! turned into those units here, once a frame, so that the card does no more than the maths.
//!
//! The block of a corrector is [`CORRECTOR`] `vec4`s at a place fixed by the shape of the graph;
//! the points of its curves are after the blocks of every node, and the block says where (as
//! offsets from itself) and how many. So the shape of the graph alone says where everything is
//! read from, and the number of points of a curve is a parameter like any other.

#![allow(
    clippy::float_cmp,
    clippy::similar_names,
    reason = "settings are compared exactly with their neutral values"
)]

use std::f64::consts::PI;

use columbina_model::grade::{
    points_at, Composite, Corrector, Curve, CurvePoint, KeyInput, KeyOutput, Morph, Param,
    Qualifier, QualifierMode, Range, ShotMatch, Wheel,
};

use crate::color::Filter;

/// Lift of R, G, B and Y.
pub const LIFT: u32 = 0;
/// The exponents of Gamma of R, G, B and Y.
pub const GAMMA: u32 = 1;
/// Gain of R, G, B and Y.
pub const GAIN: u32 = 2;
/// Lum Mix, in `w` (over 100); `x`, `y` and `z` are not used.
pub const LUM_MIX: u32 = 3;
/// Offset of R, G and B.
pub const OFFSET: u32 = 4;
/// Contrast and Pivot.
pub const CONTRAST: u32 = 5;
/// Saturation (a factor), the turn of Hue (its cosine less 1 and its sine), and Color Boost.
pub const COLOUR: u32 = 6;
/// The gains of R, G and B of Temp and Tint.
pub const BALANCE: u32 = 7;
/// Shadows, Highlights, and the Low and High Range of the Log wheels.
pub const ZONES: u32 = 8;
/// The Shadow wheel of the Log mode.
pub const LOG_SHADOW: u32 = 9;
/// The Midtone wheel.
pub const LOG_MIDTONE: u32 = 10;
/// The Highlight wheel.
pub const LOG_HIGHLIGHT: u32 = 11;
/// The intensity of the custom curves of Y, R, G and B.
pub const CURVE_INTENSITY: u32 = 12;
/// Where the points of the custom curves of Y, R, G and B are, from the block.
pub const CURVE_OFFSETS: u32 = 13;
/// How many points they have.
pub const CURVE_COUNTS: u32 = 14;
/// Whether the custom curves are ganged (1) or not (0).
pub const CURVE_GANGED: u32 = 15;
/// Where the points of Hue vs Hue, Hue vs Sat, Hue vs Lum and Lum vs Sat are.
pub const HSL_OFFSETS: u32 = 16;
/// Where the points of Sat vs Sat and Sat vs Lum are.
pub const HSL_OFFSETS_2: u32 = 17;
/// How many points Hue vs Hue, Hue vs Sat, Hue vs Lum and Lum vs Sat have.
pub const HSL_COUNTS: u32 = 18;
/// How many points Sat vs Sat and Sat vs Lum have.
pub const HSL_COUNTS_2: u32 = 19;
/// Soft Clip: Low of R, G and B.
pub const CLIP_LOW: u32 = 20;
/// High.
pub const CLIP_HIGH: u32 = 21;
/// The width of the knee of Low Soft.
pub const CLIP_LOW_SOFT: u32 = 22;
/// The width of the knee of High Soft.
pub const CLIP_HIGH_SOFT: u32 = 23;
/// The key input: invert, gain, offset, matte.
pub const KEY_INPUT: u32 = 24;
/// The key output: invert, gain, offset.
pub const KEY_OUTPUT: u32 = 25;
/// Which steps of the corrector are not at their neutral settings: a sum of the `…_ON` below,
/// in `x`. The card leaves the others out (each is the identity at its neutral settings, so
/// leaving it out changes nothing).
pub const FEATURES: u32 = 26;
/// The shot match: the exponents of R, G and B.
pub const SHOT_GAMMA: u32 = 27;
/// The gains of R, G and B, and the contrast.
pub const SHOT_GAIN: u32 = 28;
/// What is added to R, G and B, and the pivot.
pub const SHOT_OFFSET: u32 = 29;
/// Midtone Detail: its amount (the setting over 100), [`MIDTONE_GAIN`] and [`MIDTONE_EDGE`].
pub const DETAIL: u32 = 30;
/// The qualifier: its mode (0 HSL, 1 RGB, 2 LUM), and whether hue, saturation and luma limit
/// its key (1) or not (0).
pub const QUALIFIER: u32 = 31;
/// Whether red, green and blue limit the key of an RGB qualifier, and whether the qualifier's
/// key is inverted (the Qualifier palette's Invert).
pub const QUALIFIER_RGB: u32 = 32;
/// The range of hue of a qualifier, in turns: its centre, half its width, and how softly it
/// falls off on its left (lower hues) and on its right.
pub const QUALIFIER_HUE: u32 = 33;
/// The range of saturation of a qualifier, from 0 to 1: low, high, low soft and high soft.
pub const QUALIFIER_SAT: u32 = 34;
/// The range of luma.
pub const QUALIFIER_LUM: u32 = 35;
/// The range of red.
pub const QUALIFIER_RED: u32 = 36;
/// The range of green.
pub const QUALIFIER_GREEN: u32 = 37;
/// The range of blue.
pub const QUALIFIER_BLUE: u32 = 38;
/// Matte Finesse, from 0 to 1: Clean Black, Clean White, Black Clip and White Clip.
pub const FINESSE: u32 = 39;
/// Matte Finesse: In/Out Ratio (from -1 to 1); whether the first and the second pass of Morph
/// Operation grow (1) or shrink (0); and [`MORPH_EDGE`].
pub const FINESSE_MORE: u32 = 40;
/// The Key palette's Qualifier controls: invert, gain, offset, matte.
pub const KEY_QUALIFIER: u32 = 41;
/// The size of the block of a corrector.
pub const CORRECTOR: u32 = 42;

/// Shadows or Highlights.
pub const ZONES_ON: u32 = 1;
/// Color Boost.
pub const BOOST_ON: u32 = 2;
/// Temp or Tint.
pub const BALANCE_ON: u32 = 4;
/// Lift, Gamma or Gain.
pub const LGG_ON: u32 = 8;
/// Contrast.
pub const CONTRAST_ON: u32 = 16;
/// The custom curves.
pub const CURVES_ON: u32 = 32;
/// Offset or the Log wheels.
pub const LOG_ON: u32 = 64;
/// Hue.
pub const HUE_ON: u32 = 128;
/// Saturation.
pub const SATURATION_ON: u32 = 256;
/// The HSL curves.
pub const HSL_ON: u32 = 512;
/// Soft Clip.
pub const CLIP_ON: u32 = 1024;
/// The shot match.
pub const SHOT_ON: u32 = 2048;
/// Midtone Detail.
pub const DETAIL_ON: u32 = 4096;
/// Clean Black or Clean White of a qualifier.
pub const CLEAN_ON: u32 = 8192;
/// Blur Radius or In/Out Ratio of a qualifier.
pub const SPREAD_ON: u32 = 16384;
/// Morph Radius of a qualifier.
pub const MORPH_ON: u32 = 32768;
/// Pre-Filter of a qualifier.
pub const PRE_ON: u32 = 65536;
/// The blur of the key coming in.
pub const KEY_BLUR_ON: u32 = 131_072;

/// How far Gamma bends: the exponent of a Gamma of `g` is `2^(-GAMMA_POWER · g)`.
pub const GAMMA_POWER: f64 = 2.0;
/// The change of a Log wheel at its edge (1), and of Shadows or Highlights at 100.
pub const ZONE_REACH: f64 = 0.25;
/// The stops of gain of R, G and B for Temp at 4000.
pub const TEMPERATURE_STOPS: [f64; 3] = [0.5, 0.0, -0.5];
/// The stops of gain of R, G and B for Tint at 100.
pub const TINT_STOPS: [f64; 3] = [0.25, -0.5, 0.25];
/// The width of the knee of Soft Clip at a softness of 100.
pub const SOFT_REACH: f64 = 0.5;

/// Midtone Detail (p. 3063): the standard deviation of the Gaussian that the mean and the
/// spread of luma about a pixel are taken by, in pixels of a frame 1080 high
/// ([`crate::color::REFERENCE_HEIGHT`]): about the scale of the "edge detail" of the manual,
/// textures and the edges of things, which the setting makes stand out or softens, and wider
/// than noise.
pub const MIDTONE_RADIUS: f64 = 10.0;
/// How much more the detail of the midtones is at a Midtone Detail of +100: its difference from
/// the mean about it is made `1 + MIDTONE_GAIN` times as large.
pub const MIDTONE_GAIN: f64 = 1.0;
/// The spread of luma about a pixel (its standard deviation) from which a region counts as one
/// of "high edge detail": a raised Midtone Detail works on regions as they reach it, a lowered
/// one on regions as they fall from it to none ("regions of the image with low amounts of
/// detail are softened while areas of high-detail are left alone").
pub const MIDTONE_EDGE: f64 = 0.04;

/// Pre-Filter (p. 3178): the standard deviation of the Gaussian that the picture is blurred by
/// before it is qualified, in pixels of a frame 1080 high, for each unit of the setting: 5 pixels
/// at 100, enough to take the blocks of MPEG out of what is keyed.
pub const PRE_FILTER_SCALE: f64 = 0.05;
/// Clean Black and Clean White (pp. 3178–3179): the standard deviation of the Gaussian, in pixels
/// of a frame 1080 high, that a pixel's key is weighed against the key about it by: the size of
/// the specks and holes that they take out.
pub const CLEAN_RADIUS: f64 = 3.0;
/// The Blur Radius of a key (pp. 3180, 3362), "0 to 2000": the standard deviation of its
/// Gaussian, in pixels of a frame 1080 high, for each unit of the setting (200 pixels at 2000).
pub const KEY_BLUR_SCALE: f64 = 0.1;
/// In/Out Ratio "even in situations where no Blur Radius is applied at all" fills small holes or
/// takes small specks away (p. 3181): it works over a Gaussian of at least this standard
/// deviation, in pixels of a frame 1080 high.
pub const IN_OUT_RADIUS: f64 = 2.0;
/// Morph Operation (p. 3181): how much of the box of Morph Radius about a pixel is to be keyed
/// for the pixel to be grown into the key, or not for it to be shrunk out of it; and so how soft
/// the edge that it moves is.
pub const MORPH_EDGE: f64 = 0.05;
/// The flat grey of the Highlight mode of the viewer (p. 3170), "the unselected portion of the
/// image with a flat gray".
pub const HIGHLIGHT_GREY: f64 = 0.5;

/// The number of a composite mode, as the maths numbers them: the place of each in
/// [`Composite`], from `Normal` at 0 to `VividLight` at 26.
#[must_use]
pub fn composite(mode: Composite) -> u32 {
    match mode {
        Composite::Normal => 0,
        Composite::Add => 1,
        Composite::Color => 2,
        Composite::ColorBurn => 3,
        Composite::ColorDodge => 4,
        Composite::Darken => 5,
        Composite::DarkerColor => 6,
        Composite::Difference => 7,
        Composite::Divide => 8,
        Composite::Exclusion => 9,
        Composite::HardMix => 10,
        Composite::Hardlight => 11,
        Composite::Hue => 12,
        Composite::Lighten => 13,
        Composite::LighterColor => 14,
        Composite::LinearBurn => 15,
        Composite::LinearDodge => 16,
        Composite::LinearLight => 17,
        Composite::Luminosity => 18,
        Composite::Multiply => 19,
        Composite::Overlay => 20,
        Composite::PinLight => 21,
        Composite::Saturation => 22,
        Composite::Screen => 23,
        Composite::Softlight => 24,
        Composite::Subtract => 25,
        Composite::VividLight => 26,
    }
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "the card computes in f32; the settings are within its range"
)]
fn f(value: f64) -> f32 {
    value as f32
}

#[allow(
    clippy::cast_precision_loss,
    reason = "places and counts of parameters are far below 2^24"
)]
fn place(value: usize) -> f32 {
    value as f32
}

fn flag(on: bool) -> f32 {
    if on {
        1.0
    } else {
        0.0
    }
}

/// What a parameter is at `frame`.
fn at(param: &Param, frame: f64) -> f64 {
    param.value_at(frame)
}

fn wheel(wheel: &Wheel, frame: f64) -> [f64; 4] {
    [
        at(&wheel.r, frame),
        at(&wheel.g, frame),
        at(&wheel.b, frame),
        at(&wheel.y, frame),
    ]
}

fn rgb(r: &Param, g: &Param, b: &Param, frame: f64, map: impl Fn(f64) -> f64) -> [f32; 4] {
    [
        f(map(at(r, frame))),
        f(map(at(g, frame))),
        f(map(at(b, frame))),
        0.0,
    ]
}

/// The exponent of a Gamma of `gamma`.
#[must_use]
pub fn gamma_exponent(gamma: f64) -> f64 {
    (-GAMMA_POWER * gamma).exp2()
}

/// The gains of R, G and B of a Temp of `temperature` and a Tint of `tint`, divided by what
/// they make of the luma of a grey so as to keep it.
#[must_use]
pub fn balance(temperature: f64, tint: f64, luma: [f64; 3]) -> [f64; 3] {
    let u = temperature / 4000.0;
    let v = tint / 100.0;
    let gains: [f64; 3] =
        std::array::from_fn(|i| (TEMPERATURE_STOPS[i] * u + TINT_STOPS[i] * v).exp2());
    let grey = 1.0 + (0..3).map(|i| luma[i] * (gains[i] - 1.0)).sum::<f64>();
    gains.map(|gain| gain / grey)
}

/// The slopes of a curve through `points` (in the order of x, not two at one x): the slope of
/// the line through the two points about each, and at each end that of its segment.
fn slopes(points: &[[f64; 2]]) -> Vec<f64> {
    let n = points.len();
    if n < 2 {
        return vec![0.0; n];
    }
    let slope = |a: [f64; 2], b: [f64; 2]| {
        if b[0] > a[0] {
            (b[1] - a[1]) / (b[0] - a[0])
        } else {
            0.0
        }
    };
    (0..n)
        .map(|i| match i {
            0 => slope(points[0], points[1]),
            _ if i == n - 1 => slope(points[n - 2], points[n - 1]),
            _ => slope(points[i - 1], points[i + 1]),
        })
        .collect()
}

/// The points of a curve as the card reads them, `(x, y, slope, 0)`. A curve of hue goes round:
/// its last point is put again before its first, a turn less, and its first after its last, a
/// turn more, so that the curve is smooth where its ends meet.
fn points(points: &[[f64; 2]], round: bool) -> Vec<[f32; 4]> {
    let mut points = points.to_vec();
    if round {
        if let (Some(&[x0, y0]), Some(&[xn, yn])) = (points.first(), points.last()) {
            points.insert(0, [xn - 1.0, yn]);
            points.push([x0 + 1.0, y0]);
        }
    }
    let slopes = slopes(&points);
    points
        .iter()
        .zip(slopes)
        .map(|([x, y], m)| [f(*x), f(*y), f(m), 0.0])
        .collect()
}

/// The block of a corrector at `frame`, at place `block` of the parameters, its points put at
/// the end of `tail`, which begins at place `tail_at`.
#[allow(
    clippy::too_many_lines,
    reason = "the settings, one block after another"
)]
#[must_use]
pub fn corrector(
    corrector: &Corrector,
    frame: f64,
    luma: [f64; 3],
    block: usize,
    tail_at: usize,
    tail: &mut Vec<[f32; 4]>,
) -> Vec<[f32; 4]> {
    let p = &corrector.primaries;
    let mut out = vec![[0.0_f32; 4]; CORRECTOR as usize];
    let lift = wheel(&p.lift, frame);
    let gamma = wheel(&p.gamma, frame);
    let gain = wheel(&p.gain, frame);
    out[LIFT as usize] = lift.map(f);
    out[GAMMA as usize] = gamma.map(|g| f(gamma_exponent(g)));
    out[GAIN as usize] = gain.map(f);
    out[LUM_MIX as usize] = [0.0, 0.0, 0.0, f(at(&p.lum_mix, frame) / 100.0)];
    out[OFFSET as usize] = rgb(&p.offset.r, &p.offset.g, &p.offset.b, frame, |o| {
        (o - 25.0) / 100.0
    });
    out[CONTRAST as usize] = [f(at(&p.contrast, frame)), f(at(&p.pivot, frame)), 0.0, 0.0];
    let turn = (at(&p.hue, frame) - 50.0) / 50.0 * PI;
    let half = (turn / 2.0).sin();
    out[COLOUR as usize] = [
        f(at(&p.saturation, frame) / 50.0),
        f(-2.0 * half * half),
        f(turn.sin()),
        f(at(&p.color_boost, frame) / 100.0),
    ];
    let gains = balance(at(&p.temperature, frame), at(&p.tint, frame), luma);
    out[BALANCE as usize] = [f(gains[0]), f(gains[1]), f(gains[2]), 0.0];
    let log = &corrector.log;
    out[ZONES as usize] = [
        f(at(&p.shadows, frame) / 100.0 * ZONE_REACH),
        f(at(&p.highlights, frame) / 100.0 * ZONE_REACH),
        f(at(&log.low_range, frame)),
        f(at(&log.high_range, frame)),
    ];
    let reach = |v: f64| v * ZONE_REACH;
    out[LOG_SHADOW as usize] = rgb(&log.shadow.r, &log.shadow.g, &log.shadow.b, frame, reach);
    out[LOG_MIDTONE as usize] = rgb(&log.midtone.r, &log.midtone.g, &log.midtone.b, frame, reach);
    out[LOG_HIGHLIGHT as usize] = rgb(
        &log.highlight.r,
        &log.highlight.g,
        &log.highlight.b,
        frame,
        reach,
    );
    // The points: where each list goes, from the block, and how many it has.
    let mut put = |list: &[[f64; 2]], round: bool| -> (f32, f32) {
        let made = points(list, round);
        let offset = tail_at + tail.len() - block;
        let count = made.len();
        tail.extend(made);
        (place(offset), place(count))
    };
    let curves = &corrector.curves;
    let custom: [&Curve; 4] = [&curves.y, &curves.r, &curves.g, &curves.b];
    let mut offsets = [0.0; 4];
    let mut counts = [0.0; 4];
    for (i, curve) in custom.iter().enumerate() {
        // Where the points of the curve are at this frame.
        (offsets[i], counts[i]) = put(&curve.at(frame), false);
    }
    out[CURVE_INTENSITY as usize] = custom.map(|curve| f(at(&curve.intensity, frame) / 100.0));
    out[CURVE_OFFSETS as usize] = offsets;
    out[CURVE_COUNTS as usize] = counts;
    out[CURVE_GANGED as usize] = [flag(curves.ganged), 0.0, 0.0, 0.0];
    let hsl = &corrector.hsl;
    let lists: [(&[CurvePoint], bool); 6] = [
        (&hsl.hue_vs_hue, true),
        (&hsl.hue_vs_sat, true),
        (&hsl.hue_vs_lum, true),
        (&hsl.lum_vs_sat, false),
        (&hsl.sat_vs_sat, false),
        (&hsl.sat_vs_lum, false),
    ];
    let mut offsets = [0.0; 8];
    let mut counts = [0.0; 8];
    for (i, (list, round)) in lists.iter().enumerate() {
        (offsets[i], counts[i]) = put(&points_at(list, frame), *round);
    }
    out[HSL_OFFSETS as usize] = [offsets[0], offsets[1], offsets[2], offsets[3]];
    out[HSL_OFFSETS_2 as usize] = [offsets[4], offsets[5], 0.0, 0.0];
    out[HSL_COUNTS as usize] = [counts[0], counts[1], counts[2], counts[3]];
    out[HSL_COUNTS_2 as usize] = [counts[4], counts[5], 0.0, 0.0];
    let clip = &corrector.soft_clip;
    let share = |v: f64| v / 100.0;
    let soft = |v: f64| v / 100.0 * SOFT_REACH;
    out[CLIP_LOW as usize] = rgb(&clip.low.r, &clip.low.g, &clip.low.b, frame, share);
    out[CLIP_HIGH as usize] = rgb(&clip.high.r, &clip.high.g, &clip.high.b, frame, share);
    out[CLIP_LOW_SOFT as usize] = rgb(
        &clip.low_soft.r,
        &clip.low_soft.g,
        &clip.low_soft.b,
        frame,
        soft,
    );
    out[CLIP_HIGH_SOFT as usize] = rgb(
        &clip.high_soft.r,
        &clip.high_soft.g,
        &clip.high_soft.b,
        frame,
        soft,
    );
    out[KEY_INPUT as usize] = key_input(&corrector.key.input, frame);
    out[KEY_OUTPUT as usize] = key_output(&corrector.key.output, frame);
    // No shot match is the neutral one.
    let neutral = ShotMatch::default();
    let shot = corrector.shot_match.as_ref().unwrap_or(&neutral);
    let same = |v: f64| v;
    out[SHOT_GAMMA as usize] = rgb(&shot.gamma.r, &shot.gamma.g, &shot.gamma.b, frame, same);
    out[SHOT_GAIN as usize] = rgb(&shot.gain.r, &shot.gain.g, &shot.gain.b, frame, same);
    out[SHOT_GAIN as usize][3] = f(at(&shot.contrast, frame));
    out[SHOT_OFFSET as usize] = rgb(&shot.offset.r, &shot.offset.g, &shot.offset.b, frame, same);
    out[SHOT_OFFSET as usize][3] = f(at(&shot.pivot, frame));
    out[DETAIL as usize] = [
        f(at(&p.midtone_detail, frame) / 100.0),
        f(MIDTONE_GAIN),
        f(MIDTONE_EDGE),
        0.0,
    ];
    if let Some(qualifier) = &corrector.qualifier {
        qualifier_block(qualifier, frame, &mut out);
    }
    out[KEY_QUALIFIER as usize] = key_input(&corrector.key.qualifier, frame);
    let diagonal = |curve: &Curve| curve.is_diagonal();
    let curved = if curves.ganged {
        !diagonal(&curves.y)
    } else {
        !custom.iter().all(|curve| diagonal(curve))
    };
    let mut on = features(&out, curved);
    if let Some(qualifier) = &corrector.qualifier {
        let finesse = &qualifier.finesse;
        let spread =
            at(&finesse.blur_radius, frame) > 0.0 || at(&finesse.in_out_ratio, frame) != 0.0;
        for (feature, set) in [
            (PRE_ON, at(&finesse.pre_filter, frame) > 0.0),
            (
                CLEAN_ON,
                out[FINESSE as usize][0] > 0.0 || out[FINESSE as usize][1] > 0.0,
            ),
            (SPREAD_ON, spread),
            (MORPH_ON, at(&finesse.morph_radius, frame) > 0.0),
        ] {
            if set {
                on |= feature;
            }
        }
    }
    if at(&corrector.key.input_blur.radius, frame) > 0.0 {
        on |= KEY_BLUR_ON;
    }
    out[FEATURES as usize] = [place(on as usize), 0.0, 0.0, 0.0];
    out
}

/// The settings of a qualifier at `frame`, put into the block `out` of its corrector.
fn qualifier_block(qualifier: &Qualifier, frame: f64, out: &mut [[f32; 4]]) {
    let mode = match qualifier.mode {
        QualifierMode::Hsl => 0.0,
        QualifierMode::Rgb => 1.0,
        QualifierMode::Lum => 2.0,
    };
    out[QUALIFIER as usize] = [
        mode,
        flag(qualifier.hue.on),
        flag(qualifier.sat.on),
        flag(qualifier.lum.on),
    ];
    out[QUALIFIER_RGB as usize] = [
        flag(qualifier.red.on),
        flag(qualifier.green.on),
        flag(qualifier.blue.on),
        flag(qualifier.invert),
    ];
    let hue = &qualifier.hue;
    let soft = at(&hue.soft, frame) / 100.0;
    let symmetry = at(&hue.symmetry, frame) / 50.0;
    out[QUALIFIER_HUE as usize] = [
        f(at(&hue.center, frame) / 100.0),
        f(at(&hue.width, frame) / 200.0),
        f(soft * (2.0 - symmetry)),
        f(soft * symmetry),
    ];
    let range = |range: &Range| {
        [
            f(at(&range.low, frame) / 100.0),
            f(at(&range.high, frame) / 100.0),
            f(at(&range.low_soft, frame) / 100.0),
            f(at(&range.high_soft, frame) / 100.0),
        ]
    };
    out[QUALIFIER_SAT as usize] = range(&qualifier.sat);
    out[QUALIFIER_LUM as usize] = range(&qualifier.lum);
    out[QUALIFIER_RED as usize] = range(&qualifier.red);
    out[QUALIFIER_GREEN as usize] = range(&qualifier.green);
    out[QUALIFIER_BLUE as usize] = range(&qualifier.blue);
    let finesse = &qualifier.finesse;
    out[FINESSE as usize] = [
        f(at(&finesse.clean_black, frame) / 100.0),
        f(at(&finesse.clean_white, frame) / 100.0),
        f(at(&finesse.black_clip, frame) / 100.0),
        f(at(&finesse.white_clip, frame) / 100.0),
    ];
    // Opening shrinks and then grows; closing grows and then shrinks (p. 3181).
    let (first, second) = match finesse.morph {
        Morph::Shrink | Morph::Opening => (false, matches!(finesse.morph, Morph::Opening)),
        Morph::Grow | Morph::Closing => (true, false),
    };
    out[FINESSE_MORE as usize] = [
        f(at(&finesse.in_out_ratio, frame) / 100.0),
        flag(first),
        flag(second),
        f(MORPH_EDGE),
    ];
}

/// Whether a setting is ever other than `neutral`: a fixed value of it is a part of the grade
/// that does nothing at any frame.
fn ever(param: &Param, neutral: f64) -> bool {
    !matches!(param, Param::Fixed(value) if *value == neutral)
}

/// Which neighbourhoods the qualifier of a corrector reads (none where it has none): of its
/// Pre-Filter, its Clean Black and Clean White, its Blur Radius and In/Out Ratio, and the first
/// and the second pass of its Morph Operation. Each is read where its settings are ever other
/// than those that do nothing, whatever they are at a frame.
#[must_use]
pub fn qualifier_reads(corrector: &Corrector) -> [bool; 5] {
    let Some(qualifier) = &corrector.qualifier else {
        return [false; 5];
    };
    let finesse = &qualifier.finesse;
    let morph = ever(&finesse.morph_radius, 0.0);
    [
        ever(&finesse.pre_filter, 0.0),
        ever(&finesse.clean_black, 0.0) || ever(&finesse.clean_white, 0.0),
        ever(&finesse.blur_radius, 0.0) || ever(&finesse.in_out_ratio, 0.0),
        morph,
        morph && matches!(finesse.morph, Morph::Opening | Morph::Closing),
    ]
}

/// Whether the key coming into a corrector is blurred at some frame: then it reads a
/// neighbourhood of that key.
#[must_use]
pub fn reads_key_blur(corrector: &Corrector) -> bool {
    ever(&corrector.key.input_blur.radius, 0.0)
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "radii of filters, far within f32"
)]
fn radius(value: f64) -> f32 {
    value.max(0.0) as f32
}

/// The filter of the Pre-Filter of `qualifier` at `frame`.
#[must_use]
pub fn pre_filter(qualifier: &Qualifier, frame: f64) -> Filter {
    let sigma = radius(at(&qualifier.finesse.pre_filter, frame) * PRE_FILTER_SCALE);
    Filter::Gaussian { sigma: [sigma; 2] }
}

/// The filter of Clean Black and Clean White.
#[must_use]
pub fn clean_filter() -> Filter {
    let sigma = radius(CLEAN_RADIUS);
    Filter::Gaussian { sigma: [sigma; 2] }
}

/// The filter of the Blur Radius and the In/Out Ratio of `qualifier` at `frame`.
#[must_use]
pub fn spread_filter(qualifier: &Qualifier, frame: f64) -> Filter {
    let finesse = &qualifier.finesse;
    let mut sigma = at(&finesse.blur_radius, frame) * KEY_BLUR_SCALE;
    if at(&finesse.in_out_ratio, frame) != 0.0 {
        sigma = sigma.max(IN_OUT_RADIUS);
    }
    let sigma = radius(sigma);
    Filter::Gaussian { sigma: [sigma; 2] }
}

/// The filter of the Morph Operation of `qualifier` at `frame`, each pass.
#[must_use]
pub fn morph_filter(qualifier: &Qualifier, frame: f64) -> Filter {
    let reach = radius(at(&qualifier.finesse.morph_radius, frame));
    Filter::Box { radius: [reach; 2] }
}

/// The filter of the blur of the key coming into `corrector` at `frame`: its Blur Radius,
/// shared between across and down by its Blur H/V Ratio (0.5 alike, 1 across alone).
#[must_use]
pub fn key_blur_filter(corrector: &Corrector, frame: f64) -> Filter {
    let blur = &corrector.key.input_blur;
    let sigma = at(&blur.radius, frame) * KEY_BLUR_SCALE;
    let ratio = at(&blur.ratio, frame).clamp(0.0, 1.0);
    Filter::Gaussian {
        sigma: [
            radius(sigma * 2.0 * ratio),
            radius(sigma * 2.0 * (1.0 - ratio)),
        ],
    }
}

/// Which steps of a block are not at their neutral settings (see [`FEATURES`]).
fn features(block: &[[f32; 4]], curved: bool) -> u32 {
    let at = |index: u32| block[index as usize];
    let all = |index: u32, count: usize, value: f32| at(index)[..count].iter().all(|v| *v == value);
    let steps = [
        (ZONES_ON, at(ZONES)[0] != 0.0 || at(ZONES)[1] != 0.0),
        (BOOST_ON, at(COLOUR)[3] != 0.0),
        (BALANCE_ON, !all(BALANCE, 3, 1.0)),
        (
            LGG_ON,
            !all(LIFT, 4, 0.0) || !all(GAMMA, 4, 1.0) || !all(GAIN, 4, 1.0),
        ),
        (CONTRAST_ON, at(CONTRAST)[0] != 1.0),
        (CURVES_ON, curved),
        (
            LOG_ON,
            !all(OFFSET, 3, 0.0)
                || !all(LOG_SHADOW, 3, 0.0)
                || !all(LOG_MIDTONE, 3, 0.0)
                || !all(LOG_HIGHLIGHT, 3, 0.0),
        ),
        (HUE_ON, at(COLOUR)[1] != 0.0 || at(COLOUR)[2] != 0.0),
        (SATURATION_ON, at(COLOUR)[0] != 1.0),
        (
            HSL_ON,
            at(HSL_COUNTS).iter().any(|n| *n > 0.0) || at(HSL_COUNTS_2).iter().any(|n| *n > 0.0),
        ),
        (
            CLIP_ON,
            (0..3).any(|i| {
                at(CLIP_LOW)[i] > 0.0
                    || at(CLIP_LOW_SOFT)[i] > 0.0
                    || at(CLIP_HIGH)[i] < 1.0
                    || at(CLIP_HIGH_SOFT)[i] > 0.0
            }),
        ),
        (
            SHOT_ON,
            !all(SHOT_GAMMA, 3, 1.0)
                || !all(SHOT_GAIN, 3, 1.0)
                || !all(SHOT_OFFSET, 3, 0.0)
                || at(SHOT_GAIN)[3] != 1.0,
        ),
        (DETAIL_ON, at(DETAIL)[0] != 0.0),
    ];
    steps
        .iter()
        .filter(|(_, on)| *on)
        .map(|(feature, _)| feature)
        .sum()
}

/// Whether the Midtone Detail of `corrector` is ever other than 0: then its grade reads the
/// pixels about each one (a neighbourhood), whatever it is at a frame; a setting of 0 that never
/// changes is the grade of a pixel alone.
#[must_use]
pub fn reads_detail(corrector: &Corrector) -> bool {
    !matches!(corrector.primaries.midtone_detail, Param::Fixed(value) if value == 0.0)
}

/// A key input: invert, gain, offset, matte.
#[must_use]
pub fn key_input(input: &KeyInput, frame: f64) -> [f32; 4] {
    [
        flag(input.invert),
        f(at(&input.gain, frame)),
        f(at(&input.offset, frame)),
        flag(input.matte),
    ]
}

/// A key output: invert, gain, offset.
#[must_use]
pub fn key_output(output: &KeyOutput, frame: f64) -> [f32; 4] {
    [
        flag(output.invert),
        f(at(&output.gain, frame)),
        f(at(&output.offset, frame)),
        0.0,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_filters_of_the_key_palette_and_pre_filter_are_scaled_and_shared() {
        let mut corrector = Corrector::default();
        corrector.key.input_blur.radius = Param::Fixed(40.0);
        let sigma = 40.0 * KEY_BLUR_SCALE;
        #[allow(clippy::cast_possible_truncation, reason = "small radii")]
        let at = |share: f64| (sigma * 2.0 * share) as f32;
        for (ratio, across, down) in [(0.5, 0.5, 0.5), (1.0, 1.0, 0.0), (0.25, 0.25, 0.75)] {
            corrector.key.input_blur.ratio = Param::Fixed(ratio);
            assert_eq!(
                key_blur_filter(&corrector, 0.0),
                Filter::Gaussian {
                    sigma: [at(across), at(down)]
                },
                "{ratio}"
            );
        }
        let mut qualifier = Qualifier::default();
        qualifier.finesse.pre_filter = Param::Fixed(30.0);
        #[allow(clippy::cast_possible_truncation, reason = "small radii")]
        let pre = (30.0 * PRE_FILTER_SCALE) as f32;
        assert_eq!(
            pre_filter(&qualifier, 0.0),
            Filter::Gaussian { sigma: [pre; 2] }
        );
    }
}
