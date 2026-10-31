//! The maths of the nodes on the processor, a control at a time; grades of every shape; and the
//! WGSL held to the processor on the card (those tests are skipped without an adapter, unless
//! `COLUMBINA_REQUIRE_GPU` is 1).

#![allow(
    clippy::float_cmp,
    clippy::too_many_lines,
    reason = "colours are compared to the bit where they are to be the same"
)]

use std::collections::HashMap;
use std::sync::Arc;

use columbina_model::grade::{
    Composite, Corrector, Grade, Interp, Key, KeyInput, KeyMixer, LayerMixer, Link, Node, NodeKind,
    ParallelMixer, Param, Port, ShotMatch, OUTPUT, SOURCE,
};

use super::maths::{self, Constants};
use super::*;
use crate::color::{program, FIRST_TEXTURE_BINDING};
use crate::{gpu_for_tests, Gpu, OpTexture, TextureKind};

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

/// A small generator of numbers, the same on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    #[allow(clippy::cast_precision_loss, reason = "53 bits of a number")]
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1_u64 << 53) as f64
    }

    fn range(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.unit()
    }

    #[allow(clippy::cast_possible_truncation, reason = "a small count")]
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }

    #[allow(clippy::cast_possible_truncation, reason = "a colour of the card")]
    fn colour(&mut self, low: f64, high: f64) -> [f32; 3] {
        [0; 3].map(|_| self.range(low, high) as f32)
    }
}

fn fixed(value: f64) -> Param {
    Param::Fixed(value)
}

fn corrector(id: &str, set: impl FnOnce(&mut Corrector)) -> Node {
    let mut node = Node::corrector(id);
    if let NodeKind::Corrector(corrector) = &mut node.kind {
        set(corrector);
    }
    node
}

fn link(from: &str, to: &str, kind: Port, input: u32) -> Link {
    Link {
        from: from.into(),
        to: to.into(),
        kind,
        input,
    }
}

fn graph(nodes: Vec<Node>, links: Vec<Link>) -> Grade {
    let mut grade = Grade { nodes, links };
    grade.sort_links();
    grade
}

/// The nodes one after another, from the source to the output.
fn serial(nodes: Vec<Node>) -> Grade {
    let mut ids: Vec<String> = vec![SOURCE.into()];
    ids.extend(nodes.iter().map(|n| n.id.clone()));
    ids.push(OUTPUT.into());
    let links = ids
        .windows(2)
        .map(|pair| link(&pair[0], &pair[1], Port::Rgb, 0))
        .collect();
    graph(nodes, links)
}

/// The nodes side by side, each on the source, into a mixer `m`.
fn mixed(nodes: Vec<Node>, mixer: NodeKind) -> Grade {
    let mut links = vec![link("m", OUTPUT, Port::Rgb, 0)];
    for (i, node) in nodes.iter().enumerate() {
        links.push(link(SOURCE, &node.id, Port::Rgb, 0));
        links.push(link(&node.id, "m", Port::Rgb, u32::try_from(i).unwrap()));
    }
    let mut all = nodes;
    all.push(Node {
        id: "m".into(),
        label: String::new(),
        enabled: true,
        locked: false,
        pos: None,
        kind: mixer,
    });
    graph(all, links)
}

fn options() -> GradeOptions {
    GradeOptions::default()
}

fn constants() -> Constants {
    options().constants()
}

fn eval(grade: &Grade, rgb: [f32; 3]) -> [f32; 3] {
    evaluate(grade, 0.0, &options(), rgb)
}

fn one(set: impl FnOnce(&mut Corrector)) -> Grade {
    serial(vec![corrector("n1", set)])
}

#[track_caller]
fn near(a: [f32; 3], b: [f32; 3], most: f32) {
    assert!(
        a.iter().zip(b).all(|(a, b)| (a - b).abs() <= most),
        "{a:?} is not {b:?}"
    );
}

fn luma(x: [f32; 3]) -> f32 {
    maths::luma(&constants(), x)
}

/// Colours of every kind: black, white, greys, primaries, beyond white and below black.
fn colours() -> Vec<[f32; 3]> {
    let mut rng = Rng(7);
    let mut all = vec![
        [0.0, 0.0, 0.0],
        [1.0, 1.0, 1.0],
        [0.5, 0.5, 0.5],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.7, 0.2, -0.1],
        [-0.05, -0.02, -0.3],
        [0.18, 0.18, 0.18],
        [3.5, 2.25, 1.0],
    ];
    for _ in 0..500 {
        all.push(rng.colour(-0.2, 1.5));
    }
    all
}

// ---------------------------------------------------------------------------------------------
// Neutral settings are the identity, to the bit
// ---------------------------------------------------------------------------------------------

#[test]
fn nodes_as_they_are_made_change_nothing_to_the_bit() {
    let mut key_mixed = serial(vec![Node::corrector("n1"), Node::corrector("n2")]);
    key_mixed
        .nodes
        .push(Node::new("km", columbina_model::grade::Kind::KeyMixer));
    key_mixed.links.extend([
        link("n1", "km", Port::Key, 0),
        link("n1", "km", Port::Key, 1),
        link("km", "n2", Port::Key, 0),
    ]);
    key_mixed.sort_links();
    let grades = [
        one(|_| {}),
        // A shot match at its neutral settings, as no shot match.
        one(|c| c.shot_match = Some(ShotMatch::default())),
        serial(vec![
            Node::corrector("n1"),
            Node::corrector("n2"),
            Node::corrector("n3"),
        ]),
        mixed(
            vec![
                Node::corrector("a"),
                Node::corrector("b"),
                Node::corrector("c"),
            ],
            NodeKind::ParallelMixer(ParallelMixer { inputs: 3 }),
        ),
        mixed(
            vec![Node::corrector("a"), Node::corrector("b")],
            NodeKind::LayerMixer(LayerMixer::default()),
        ),
        key_mixed,
    ];
    for s_curve_contrast in [true, false] {
        let options = GradeOptions {
            s_curve_contrast,
            ..GradeOptions::default()
        };
        for grade in &grades {
            assert!(grade.is_complete());
            for rgb in colours() {
                let out = evaluate(grade, 0.0, &options, rgb);
                assert_eq!(out.map(f32::to_bits), rgb.map(f32::to_bits), "{rgb:?}");
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Each control
// ---------------------------------------------------------------------------------------------

#[test]
fn gain_scales_from_black() {
    let grade = one(|c| {
        // The master ring: Y, R, G and B together (p. 3061).
        let g = &mut c.primaries.gain;
        (g.y, g.r, g.g, g.b) = (fixed(2.0), fixed(2.0), fixed(2.0), fixed(2.0));
    });
    for rgb in colours() {
        near(
            eval(&grade, rgb),
            rgb.map(|v| v * 2.0),
            2e-6 * (1.0 + rgb[0].abs()) * 4.0,
        );
    }
}

#[test]
fn lift_raises_black_and_keeps_white() {
    let grade = one(|c| {
        let l = &mut c.primaries.lift;
        (l.y, l.r, l.g, l.b) = (fixed(0.2), fixed(0.2), fixed(0.2), fixed(0.2));
    });
    near(eval(&grade, [0.0; 3]), [0.2; 3], 1e-6);
    near(eval(&grade, [1.0; 3]), [1.0; 3], 1e-6);
    near(eval(&grade, [0.5; 3]), [0.6; 3], 1e-6);
    // Lowered, black goes below 0 and white stays (a channel of its own, at Lum Mix 0).
    let grade = one(|c| {
        c.primaries.lift.g = fixed(-0.1);
        c.primaries.lum_mix = fixed(0.0);
    });
    let out = eval(&grade, [0.0, 0.0, 0.0]);
    assert!(out[1] < -0.05);
}

#[test]
fn gamma_bends_the_midtones_and_keeps_black_and_white() {
    let grade = one(|c| {
        let g = &mut c.primaries.gamma;
        (g.y, g.r, g.g, g.b) = (fixed(0.25), fixed(0.25), fixed(0.25), fixed(0.25));
    });
    near(eval(&grade, [0.0; 3]), [0.0; 3], 1e-6);
    near(eval(&grade, [1.0; 3]), [1.0; 3], 1e-6);
    let mid = eval(&grade, [0.5; 3]);
    near(mid, [0.5_f32.powf(0.5_f32.sqrt()); 3], 1e-5);
    assert!(mid[0] > 0.6);
    let darker = one(|c| c.primaries.gamma.b = fixed(-0.25));
    assert!(eval(&darker, [0.5; 3])[2] < 0.45);
}

#[test]
fn offset_adds_to_the_whole_range_outside_lum_mix() {
    let grade = one(|c| {
        c.primaries.offset.r = fixed(35.0);
        c.primaries.offset.b = fixed(15.0);
    });
    for rgb in [[0.0; 3], [0.5, 0.2, 0.9], [1.2, 1.0, 0.0]] {
        near(
            eval(&grade, rgb),
            [rgb[0] + 0.1, rgb[1], rgb[2] - 0.1],
            1e-6,
        );
    }
}

#[test]
fn contrast_turns_about_its_pivot() {
    let grade = |k: f64| {
        one(|c| {
            c.primaries.contrast = fixed(k);
            c.primaries.pivot = fixed(0.4);
        })
    };
    let more = grade(1.5);
    // The pivot, black and white stay; the slope at the pivot is the contrast.
    for v in [0.0, 0.4, 1.0] {
        near(eval(&more, [v; 3]), [v; 3], 1e-6);
    }
    let slope = (eval(&more, [0.401; 3])[0] - eval(&more, [0.399; 3])[0]) / 0.002;
    assert!((slope - 1.5).abs() < 0.01, "{slope}");
    assert!(eval(&more, [0.2; 3])[0] < 0.2 && eval(&more, [0.7; 3])[0] > 0.7);
    // An S-curve does not clip, and keeps what is beyond white.
    assert!(eval(&more, [0.99; 3])[0] < 1.0);
    near(eval(&more, [1.4; 3]), [1.4; 3], 0.0);
    // Linear, it clips nothing either, but goes beyond.
    let linear = GradeOptions {
        s_curve_contrast: false,
        ..GradeOptions::default()
    };
    let out = evaluate(&more, 0.0, &linear, [0.9; 3]);
    near(out, [0.4 + 0.5 * 1.5; 3], 1e-6);
    let less = grade(0.5);
    assert!(eval(&less, [0.1; 3])[0] > 0.1 && eval(&less, [0.9; 3])[0] < 0.9);
}

#[test]
fn saturation_and_hue_turn_and_scale_the_colour_about_its_luma() {
    let grey = one(|c| c.primaries.saturation = fixed(0.0));
    let red = [0.8, 0.2, 0.1];
    let y = luma(red);
    near(eval(&grey, red), [y; 3], 1e-6);
    let double = one(|c| c.primaries.saturation = fixed(100.0));
    near(eval(&double, red), red.map(|v| y + 2.0 * (v - y)), 1e-6);
    // Half a turn: red to cyan, and a grey stays.
    let turned = one(|c| c.primaries.hue = fixed(100.0));
    let out = eval(&turned, [1.0, 0.0, 0.0]);
    assert!(out[0] < out[1] && (out[1] - out[2]).abs() < 1e-6, "{out:?}");
    near(eval(&turned, [0.3; 3]), [0.3; 3], 1e-6);
    // A little more hue: red towards yellow.
    let less = one(|c| c.primaries.hue = fixed(55.0));
    let out = eval(&less, [1.0, 0.0, 0.0]);
    assert!(out[1] > out[2]);
}

#[test]
fn lum_mix_keeps_luma_against_the_channels_and_lets_y_change_it() {
    // A channel apart, at Lum Mix 0, changes only itself.
    let red = |mix: f64| {
        one(|c| {
            c.primaries.gain.r = fixed(1.5);
            c.primaries.gain.g = fixed(0.8);
            c.primaries.lum_mix = fixed(mix);
        })
    };
    let x = [0.5, 0.4, 0.3];
    let apart = eval(&red(0.0), x);
    near(apart, [0.75, 0.32, 0.3], 1e-6);
    // At 100, the other channels keep the luma as it was (p. 3064).
    let mixed = eval(&red(100.0), x);
    assert!((luma(mixed) - luma(x)).abs() < 1e-6);
    assert!((luma(apart) - luma(x)).abs() > 1e-3);
    // The differences of the channels are those of the gains, all moved alike.
    near(
        [mixed[0] - mixed[1], mixed[1] - mixed[2], 0.0],
        [apart[0] - apart[1], apart[1] - apart[2], 0.0],
        1e-6,
    );
    // Y alone changes luma at 100 and nothing at 0.
    let y_gain = |mix: f64| {
        one(|c| {
            c.primaries.gain.y = fixed(1.2);
            c.primaries.lum_mix = fixed(mix);
        })
    };
    near(eval(&y_gain(0.0), x), x, 0.0);
    let out = eval(&y_gain(100.0), x);
    assert!((luma(out) - 1.2 * luma(x)).abs() < 1e-6);
    // Added to each channel, so saturation falls as luma rises.
    assert!((out[0] - out[2] - (x[0] - x[2])).abs() < 1e-6);
}

#[test]
fn a_master_ring_moves_y_r_g_and_b_together_once_whatever_lum_mix_is() {
    // The master ring writes Y, R, G and B alike (p. 3061): a Gain of 1.2 takes mid grey to
    // 0.6 at any Lum Mix, and a colour as the channels alone take it.
    let ring = |mix: f64, lift: f64, gamma: f64, gain: f64| {
        one(|c| {
            for (wheel, value) in [
                (&mut c.primaries.lift, lift),
                (&mut c.primaries.gamma, gamma),
                (&mut c.primaries.gain, gain),
            ] {
                wheel.y = fixed(value);
                wheel.r = fixed(value);
                wheel.g = fixed(value);
                wheel.b = fixed(value);
            }
            c.primaries.lum_mix = fixed(mix);
        })
    };
    for mix in [0.0, 37.0, 100.0] {
        near(eval(&ring(mix, 0.0, 0.0, 1.2), [0.5; 3]), [0.6; 3], 1e-6);
        let colour = [0.6, 0.35, 0.2];
        near(
            eval(&ring(mix, 0.0, 0.0, 1.2), colour),
            colour.map(|v| v * 1.2),
            1e-6,
        );
        // Lift is as linear: once too.
        near(
            eval(&ring(mix, 0.1, 0.0, 1.0), colour),
            colour.map(|v| v + 0.1 * (1.0 - v)),
            1e-6,
        );
        // Gamma on a grey: once.
        #[allow(clippy::cast_possible_truncation, reason = "an exponent near 1")]
        let e = layout::gamma_exponent(0.2) as f32;
        near(
            eval(&ring(mix, 0.0, 0.2, 1.0), [0.4; 3]),
            [0.4_f32.powf(e); 3],
            1e-6,
        );
    }
}

#[test]
fn temperature_and_tint_balance_the_gain_and_keep_the_luma_of_grey() {
    let warm = one(|c| c.primaries.temperature = fixed(2000.0));
    let grey = [0.5; 3];
    let out = eval(&warm, grey);
    assert!(out[0] > out[1] && out[1] > out[2], "{out:?}");
    assert!((luma(out) - 0.5).abs() < 1e-6);
    let cool = one(|c| c.primaries.temperature = fixed(-2000.0));
    let out = eval(&cool, grey);
    assert!(out[2] > out[0]);
    let magenta = one(|c| c.primaries.tint = fixed(50.0));
    let out = eval(&magenta, grey);
    assert!(out[1] < out[0] && out[1] < out[2]);
    assert!((luma(out) - 0.5).abs() < 1e-6);
}

#[test]
fn color_boost_raises_low_saturation_more() {
    let boost = one(|c| c.primaries.color_boost = fixed(100.0));
    let sat = |x: [f32; 3]| maths::sat(x);
    let dull = [0.5, 0.45, 0.4];
    let vivid = [0.9, 0.1, 0.05];
    let gain = |x: [f32; 3]| sat(eval(&boost, x)) / sat(x);
    assert!(gain(dull) > gain(vivid) && gain(vivid) >= 1.0);
    let cut = one(|c| c.primaries.color_boost = fixed(-100.0));
    assert!(sat(eval(&cut, dull)) < sat(dull));
    near(eval(&boost, [0.3; 3]), [0.3; 3], 1e-6);
}

#[test]
fn shadows_and_highlights_leave_the_midtones() {
    let shadows = one(|c| c.primaries.shadows = fixed(100.0));
    assert!(eval(&shadows, [0.05; 3])[0] > 0.2);
    near(eval(&shadows, [0.5; 3]), [0.5; 3], 1e-6);
    near(eval(&shadows, [0.9; 3]), [0.9; 3], 1e-6);
    let highlights = one(|c| c.primaries.highlights = fixed(-100.0));
    assert!(eval(&highlights, [1.1; 3])[0] < 0.9);
    near(eval(&highlights, [0.3; 3]), [0.3; 3], 1e-6);
}

fn detailed(amount: f64) -> Grade {
    one(|c| c.primaries.midtone_detail = fixed(amount))
}

/// A grey picture of 320 × 240 in four parts of 160 × 120: at the top left, stripes two pixels
/// wide of 0.35 and 0.65; at the top right, a flat 0.5; at the bottom left, stripes of 0.01 and
/// 0.09, near black; at the bottom right, a checker of 0.495 and 0.505, a faint noise.
fn quarters() -> Vec<[f32; 3]> {
    (0..240_u32)
        .flat_map(|y| (0..320_u32).map(move |x| (x, y)))
        .map(|(x, y)| {
            let odd = (x / 2) % 2 == 1;
            let v = match (x < 160, y < 120) {
                (true, true) => {
                    if odd {
                        0.65
                    } else {
                        0.35
                    }
                }
                (false, true) => 0.5,
                (true, false) => {
                    if odd {
                        0.09
                    } else {
                        0.01
                    }
                }
                (false, false) => {
                    if (x + y) % 2 == 1 {
                        0.505
                    } else {
                        0.495
                    }
                }
            };
            [v; 3]
        })
        .collect()
}

/// What `grade` makes of [`quarters`], a pixel of it to a pixel of the reference frame.
fn graded_quarters(grade: &Grade) -> Vec<[f32; 3]> {
    evaluate_picture(
        grade,
        0.0,
        &options(),
        &NoLuts,
        &|_, rgb| rgb,
        (320, 240),
        &quarters(),
        1.0,
    )
}

/// The least and the most luma within the middle of a quarter (`left`, `top`), 40 pixels (four
/// standard deviations of the filter) from its edges.
fn spread_of(pixels: &[[f32; 3]], left: bool, top: bool) -> (f32, f32) {
    let (x0, y0) = (if left { 0 } else { 160 }, if top { 0 } else { 120 });
    let mut least = f32::MAX;
    let mut most = f32::MIN;
    for y in y0 + 40..y0 + 80 {
        for x in x0 + 40..x0 + 120 {
            let v = luma(pixels[y * 320 + x]);
            least = least.min(v);
            most = most.max(v);
        }
    }
    (least, most)
}

#[test]
fn midtone_detail_raises_the_contrast_of_the_detail_of_the_midtones() {
    let before = quarters();
    let after = graded_quarters(&detailed(100.0));
    let range = |pixels: &[[f32; 3]], left, top| {
        let (least, most) = spread_of(pixels, left, top);
        most - least
    };
    // The stripes of the midtones nearly twice as far apart (by 1 + 0.91 there).
    let raised = range(&after, true, true) / range(&before, true, true);
    assert!((1.8..2.0).contains(&raised), "{raised}");
    // About their mean: the stripes still about 0.5.
    let (least, most) = spread_of(&after, true, true);
    assert!((f32::midpoint(least, most) - 0.5).abs() < 0.01);
    // Flat: as it was.
    let (least, most) = spread_of(&after, false, true);
    assert!((least - 0.5).abs() < 1e-3 && (most - 0.5).abs() < 1e-3);
    // The stripes near black are left nearly as they are: no midtones there.
    let dark = range(&after, true, false) / range(&before, true, false);
    assert!((1.0..1.3).contains(&dark), "{dark}");
    // A noise too faint to be detail is not raised much.
    let faint = range(&after, false, false) / range(&before, false, false);
    assert!((1.0..1.1).contains(&faint), "{faint}");
    // Half as much, about half the change.
    let half = graded_quarters(&detailed(50.0));
    let halved = range(&half, true, true) / range(&before, true, true);
    assert!(
        (halved - 1.0 - (raised - 1.0) / 2.0).abs() < 0.02,
        "{halved}"
    );
}

#[test]
fn midtone_detail_lowered_softens_what_has_little_detail_and_leaves_the_rest() {
    let before = quarters();
    let after = graded_quarters(&detailed(-100.0));
    let range = |pixels: &[[f32; 3]], left, top| {
        let (least, most) = spread_of(pixels, left, top);
        most - least
    };
    // The faint noise is taken nearly all the way to its mean.
    let softened = range(&after, false, false) / range(&before, false, false);
    assert!(softened < 0.1, "{softened}");
    // The stripes, which are detail, stay.
    let kept = range(&after, true, true) / range(&before, true, true);
    assert!((0.98..=1.0).contains(&kept), "{kept}");
}

#[test]
fn midtone_detail_leaves_a_picture_of_one_colour_and_a_pixel_alone() {
    for amount in [100.0, -100.0, 37.0] {
        let grade = detailed(amount);
        // A pixel alone has nothing about it.
        for rgb in colours() {
            assert_eq!(eval(&grade, rgb), rgb);
        }
        // A picture of one colour: as it was, but for the half floats of its mean.
        let flat = vec![[0.3, 0.45, 0.6]; 40 * 30];
        let out = evaluate_picture(
            &grade,
            0.0,
            &options(),
            &NoLuts,
            &|_, rgb| rgb,
            (40, 30),
            &flat,
            1.0,
        );
        for pixel in out {
            near(pixel, [0.3, 0.45, 0.6], 5e-4);
        }
    }
}

#[test]
fn midtone_detail_reads_a_neighbourhood_unless_it_is_always_0() {
    let neighbourhoods = |grade: &Grade| -> Vec<crate::Neighbourhood> {
        grade_ops(grade, 0.0, &options(), &NoLuts)
            .into_iter()
            .flat_map(|op| op.neighbourhoods)
            .collect()
    };
    let code = |grade: &Grade| {
        let ops = grade_ops(grade, 0.0, &options(), &NoLuts);
        ops.last().unwrap().body.to_string()
    };
    // At 0, the grade of a pixel alone: the code of a node as it is made.
    assert!(neighbourhoods(&detailed(0.0)).is_empty());
    assert_eq!(code(&detailed(0.0)), code(&one(|_| {})));
    // Otherwise one, a Gaussian of its radius; keyframed through 0 as well, with the same code.
    let fixed_on = neighbourhoods(&detailed(30.0));
    assert_eq!(fixed_on.len(), 1);
    #[allow(
        clippy::cast_possible_truncation,
        reason = "a constant of a few pixels"
    )]
    let radius = layout::MIDTONE_RADIUS as f32;
    assert_eq!(
        fixed_on[0].filter,
        crate::Filter::Gaussian { sigma: [radius; 2] }
    );
    let keyed = one(|c| {
        c.primaries.midtone_detail = Param::Keys(vec![
            Key {
                frame: 0,
                value: 0.0,
                interp: Interp::Linear,
            },
            Key {
                frame: 10,
                value: 50.0,
                interp: Interp::Linear,
            },
        ]);
    });
    assert_eq!(neighbourhoods(&keyed).len(), 1);
    assert_eq!(code(&keyed), code(&detailed(30.0)));
    assert!(code(&keyed).contains("$N0"));
    // Two, the second after the first: what the second filters reads the first.
    let two = serial(vec![
        corrector("a", |c| c.primaries.midtone_detail = fixed(20.0)),
        gain_node("b", [1.2, 1.0, 1.0]),
        corrector("c", |c| c.primaries.midtone_detail = fixed(-20.0)),
    ]);
    let both = neighbourhoods(&two);
    assert_eq!(both.len(), 2);
    assert!(!both[0].body.contains("$N"));
    assert!(both[1].body.contains("$N0") && !both[1].body.contains("$N1"));
    assert!(code(&two).contains("$N0") && code(&two).contains("$N1"));
    // In a branch: what it filters comes of what its input comes of, and of nothing else.
    let branch = mixed(
        vec![
            gain_node("a", [1.5, 1.0, 1.0]),
            corrector("b", |c| c.primaries.midtone_detail = fixed(40.0)),
        ],
        NodeKind::ParallelMixer(ParallelMixer { inputs: 2 }),
    );
    let only = neighbourhoods(&branch);
    assert_eq!(only.len(), 1);
    assert!(!only[0].body.contains("o0"), "{}", only[0].body);
    assert!(only[0].body.contains("grade_detail_source(grade_source"));
}

#[test]
fn midtone_detail_is_in_its_node_after_shadows_and_highlights() {
    // Shadows raise what is filtered as well as the pixel: a node that raises the shadows of
    // stripes near black raises their mean as much, and they are not taken for detail more.
    let before = graded_quarters(&one(|c| c.primaries.shadows = fixed(60.0)));
    let after = graded_quarters(&one(|c| {
        c.primaries.shadows = fixed(60.0);
        c.primaries.midtone_detail = fixed(100.0);
    }));
    let (least, most) = spread_of(&before, true, false);
    let (least_after, most_after) = spread_of(&after, true, false);
    assert!(((least + most) - (least_after + most_after)).abs() < 0.02);
    assert!(most_after - least_after > most - least);
}

#[test]
fn log_wheels_move_their_own_zones() {
    let shadow = one(|c| c.log.shadow.b = fixed(0.4));
    assert!((eval(&shadow, [0.05; 3])[2] - 0.15).abs() < 1e-6);
    near(eval(&shadow, [0.8; 3]), [0.8; 3], 1e-6);
    let highlight = one(|c| c.log.highlight.r = fixed(-0.4));
    assert!((eval(&highlight, [0.9; 3])[0] - 0.8).abs() < 1e-6);
    near(eval(&highlight, [0.1; 3]), [0.1; 3], 1e-6);
    let midtone = one(|c| c.log.midtone.g = fixed(0.4));
    assert!((eval(&midtone, [0.44; 3])[1] - 0.54).abs() < 1e-6);
    near(eval(&midtone, [0.05; 3]), [0.05; 3], 1e-6);
    // The ranges move the borders.
    let wide = one(|c| {
        c.log.shadow.b = fixed(0.4);
        c.log.low_range = fixed(0.6);
        c.log.high_range = fixed(0.8);
    });
    assert!((eval(&wide, [0.45; 3])[2] - 0.55).abs() < 1e-6);
}

#[test]
fn custom_curves_pass_through_their_points_and_mix_by_intensity() {
    let lifted = |intensity: f64| {
        one(|c| {
            c.curves.y.points = vec![[0.0, 0.0], [0.5, 0.7], [1.0, 1.0]];
            c.curves.y.intensity = fixed(100.0);
            c.curves.r.intensity = fixed(intensity);
            c.curves.g.intensity = fixed(intensity);
            c.curves.b.intensity = fixed(intensity);
        })
    };
    near(eval(&lifted(100.0), [0.5; 3]), [0.7; 3], 1e-6);
    near(eval(&lifted(100.0), [0.0; 3]), [0.0; 3], 1e-6);
    near(eval(&lifted(100.0), [1.0; 3]), [1.0; 3], 1e-6);
    near(eval(&lifted(50.0), [0.5; 3]), [0.6; 3], 1e-6);
    near(eval(&lifted(0.0), [0.5; 3]), [0.5; 3], 0.0);
    // Smooth between, and carried on beyond white along the last slope.
    let a = eval(&lifted(100.0), [0.25; 3])[0];
    assert!(a > 0.25 && a < 0.7);
    let beyond = eval(&lifted(100.0), [1.2; 3])[0];
    assert!((beyond - (1.0 + 0.6 * 0.2)).abs() < 1e-5, "{beyond}");
    // Not ganged: the red curve on red alone at Lum Mix 0, and the Y curve on luma.
    let apart = one(|c| {
        c.curves.ganged = false;
        c.curves.r.points = vec![[0.0, 0.0], [0.5, 0.7], [1.0, 1.0]];
        c.primaries.lum_mix = fixed(0.0);
    });
    near(eval(&apart, [0.5, 0.5, 0.5]), [0.7, 0.5, 0.5], 1e-6);
    let y = one(|c| {
        c.curves.ganged = false;
        c.curves.y.points = vec![[0.0, 0.0], [0.5, 0.7], [1.0, 1.0]];
    });
    let out = eval(&y, [0.6, 0.5, 0.3]);
    assert!((out[0] - out[2] - 0.3).abs() < 1e-6);
    assert!(luma(out) > luma([0.6, 0.5, 0.3]));
}

#[test]
fn a_curve_may_overshoot_its_points_as_the_curves_of_davinci_do() {
    let steep = one(|c| {
        c.curves.y.points = vec![[0.0, 0.0], [0.4, 0.1], [0.5, 0.9], [1.0, 1.0]];
    });
    let mut highest: f32 = 0.0;
    for i in 0..=100 {
        #[allow(clippy::cast_precision_loss, reason = "a small count")]
        let v = i as f32 / 100.0;
        highest = highest.max(eval(&steep, [v; 3])[0]);
    }
    assert!(highest > 1.0, "{highest}");
}

#[test]
fn hsl_curves_work_on_the_hue_saturation_and_luma_of_the_input() {
    let red = [0.8, 0.2, 0.2];
    let blue = [0.2, 0.2, 0.8];
    // Hue vs Sat: reds less saturated, blues alone.
    let cut = one(|c| c.hsl.hue_vs_sat = vec![[0.0, -0.5]]);
    let s = |x: [f32; 3]| maths::sat(x);
    assert!(s(eval(&cut, red)) < s(red) - 0.1);
    let out = eval(&cut, red);
    assert!((luma(out) - luma(red)).abs() < 1e-6);
    let around = one(|c| {
        c.hsl.hue_vs_sat = vec![[0.0, -0.5], [0.3, 0.0], [0.5, 0.0], [0.8, 0.0], [0.9, 0.0]];
    });
    near(eval(&around, blue), blue, 1e-6);
    // The curves of hue go round: a point near the end works near red from the other side.
    let wrapped = one(|c| c.hsl.hue_vs_sat = vec![[0.3, 0.0], [0.6, 0.0], [0.98, -0.5]]);
    let magenta_red = [0.8, 0.2, 0.25];
    assert!(s(eval(&wrapped, magenta_red)) < s(magenta_red) - 0.1);
    // Hue vs Hue: half a turn at red.
    let turn = one(|c| c.hsl.hue_vs_hue = vec![[0.0, 1.0]]);
    let out = eval(&turn, red);
    assert!(out[0] < out[1], "{out:?}");
    // Hue vs Lum darkens the reds, and leaves a grey (it has no hue).
    let dark = one(|c| c.hsl.hue_vs_lum = vec![[0.0, -0.5]]);
    assert!(luma(eval(&dark, red)) < luma(red) - 0.05);
    near(eval(&dark, [0.4; 3]), [0.4; 3], 1e-6);
    // Lum vs Sat and Sat vs Sat scale saturation; Sat vs Lum brightens by saturation.
    let lum_sat = one(|c| c.hsl.lum_vs_sat = vec![[0.0, 0.0], [1.0, -1.0]]);
    assert!(s(eval(&lum_sat, [1.0, 0.9, 0.9])) < 0.01);
    let sat_sat = one(|c| c.hsl.sat_vs_sat = vec![[0.0, 0.0], [1.0, 1.0]]);
    assert!(s(eval(&sat_sat, red)) > s(red));
    let sat_lum = one(|c| c.hsl.sat_vs_lum = vec![[0.0, 0.5]]);
    near(eval(&sat_lum, [0.4; 3]), [0.6; 3], 1e-6);
    near(eval(&sat_lum, [0.0; 3]), [0.0; 3], 0.0);
}

#[test]
fn hsl_curves_of_lightness_multiply_luma_by_one_and_their_output() {
    // Flat curves, each of one point: Sat vs Lum lowers every colour's luma by 40 %, and Hue vs
    // Lum raises it by half, times its saturation; the differences of R, G and B from luma
    // stay, black stays black.
    let sat_lum = one(|c| c.hsl.sat_vs_lum = vec![[0.5, -0.4]]);
    let hue_lum = one(|c| c.hsl.hue_vs_lum = vec![[0.5, 0.5]]);
    for rgb in [
        [0.8, 0.2, 0.2],
        [0.3, 0.6, 0.9],
        [0.5, 0.5, 0.5],
        [0.05, 0.02, 0.0],
        [1.2, 0.9, 0.3],
    ] {
        let chroma = |x: [f32; 3]| x.map(|v| v - luma(x));
        let out = eval(&sat_lum, rgb);
        assert!((luma(out) - luma(rgb) * 0.6).abs() < 1e-6, "{rgb:?}");
        near(chroma(out), chroma(rgb), 1e-6);
        let out = eval(&hue_lum, rgb);
        let by = 1.0 + 0.5 * maths::sat(rgb);
        assert!((luma(out) - luma(rgb) * by).abs() < 1e-6, "{rgb:?}");
        near(chroma(out), chroma(rgb), 1e-6);
    }
    assert_eq!(eval(&sat_lum, [0.0; 3]), [0.0; 3]);
    assert_eq!(eval(&hue_lum, [0.0; 3]), [0.0; 3]);
}

#[test]
fn hsl_curves_read_the_picture_that_came_into_the_node() {
    // The node turns red to cyan with Hue; Hue vs Sat at red still takes it, as its input was
    // red (p. 3122).
    let grade = one(|c| {
        c.primaries.hue = fixed(100.0);
        c.hsl.hue_vs_sat = vec![[0.0, -1.0], [0.2, 0.0], [0.8, 0.0]];
    });
    let out = eval(&grade, [0.8, 0.2, 0.2]);
    assert!(maths::sat(out) < 1e-6, "{out:?}");
}

#[test]
fn soft_clip_keeps_everything_until_it_is_set() {
    let high = one(|c| {
        let h = &mut c.soft_clip.high;
        (h.r, h.g, h.b) = (fixed(90.0), fixed(90.0), fixed(90.0));
    });
    near(eval(&high, [1.5, 0.95, 0.5]), [0.9, 0.9, 0.5], 0.0);
    let soft = one(|c| {
        let s = &mut c.soft_clip.high_soft;
        (s.r, s.g, s.b) = (fixed(40.0), fixed(40.0), fixed(40.0));
    });
    // The knee at 0.8: below it nothing, above it towards 1 and never over.
    near(eval(&soft, [0.7; 3]), [0.7; 3], 0.0);
    let out = eval(&soft, [3.0, 1.0, 0.85]);
    assert!(out[0] < 1.0 && out[0] > out[1] && out[1] > out[2] && out[2] < 0.85);
    let low = one(|c| {
        c.soft_clip.low.g = fixed(10.0);
        c.soft_clip.low_soft.b = fixed(20.0);
    });
    let out = eval(&low, [-0.2, -0.2, -0.2]);
    // Low Soft lifts what is below its knee (0.1) towards Low (0), and not under it.
    assert!(
        out[0] == -0.2 && out[1] == 0.1 && out[2] > 0.0 && out[2] < 0.01,
        "{out:?}"
    );
}

/// A shot match of `gamma`, `gain` and `offset` on each channel, and `contrast` about `pivot`.
fn shot(gamma: [f64; 3], gain: [f64; 3], offset: [f64; 3], contrast: f64, pivot: f64) -> ShotMatch {
    let mut m = ShotMatch::default();
    (m.gamma.r, m.gamma.g, m.gamma.b) = (fixed(gamma[0]), fixed(gamma[1]), fixed(gamma[2]));
    (m.gain.r, m.gain.g, m.gain.b) = (fixed(gain[0]), fixed(gain[1]), fixed(gain[2]));
    (m.offset.r, m.offset.g, m.offset.b) = (fixed(offset[0]), fixed(offset[1]), fixed(offset[2]));
    (m.contrast, m.pivot) = (fixed(contrast), fixed(pivot));
    m
}

#[test]
fn a_shot_match_moves_each_channel_by_its_power_gain_and_offset() {
    let matched = one(|c| {
        c.shot_match = Some(shot(
            [0.5, 1.0, 2.0],
            [1.0, 1.2, 1.0],
            [0.0, 0.0, -0.1],
            1.0,
            0.4,
        ));
    });
    let x = [0.25, 0.5, 0.6];
    near(eval(&matched, x), [0.5, 0.6, 0.26], 1e-6);
    // The power keeps the sign; black stays black where nothing is added.
    near(eval(&matched, [-0.25, 0.0, 0.0]), [-0.5, 0.0, -0.1], 1e-6);
    // A channel at its neutral settings is kept to the bit, the others moved.
    let red = one(|c| c.shot_match = Some(shot([1.0; 3], [2.0, 1.0, 1.0], [0.0; 3], 1.0, 0.4)));
    for rgb in colours() {
        let out = eval(&red, rgb);
        assert_eq!(out[1].to_bits(), rgb[1].to_bits(), "{rgb:?}");
        assert_eq!(out[0], rgb[0] * 2.0, "{rgb:?}");
    }
}

#[test]
fn the_contrast_of_a_shot_match_is_an_s_curve_whatever_the_project_says() {
    let matched = one(|c| c.shot_match = Some(shot([1.0; 3], [1.0; 3], [0.0; 3], 1.5, 0.4)));
    for s_curve_contrast in [true, false] {
        let options = GradeOptions {
            s_curve_contrast,
            ..GradeOptions::default()
        };
        let out = evaluate(&matched, 0.0, &options, [0.2, 0.4, 0.8]);
        let s = |x: f32| maths::s_contrast(x, 1.5, 0.4);
        near(out, [s(0.2), 0.4, s(0.8)], 1e-6);
        assert!(out[0] < 0.2 && out[2] > 0.8 && out[2] < 1.0, "{out:?}");
        // Black, white and beyond are kept.
        near(
            evaluate(&matched, 0.0, &options, [0.0, 1.0, 1.4]),
            [0.0, 1.0, 1.4],
            0.0,
        );
    }
}

#[test]
fn a_shot_match_is_the_last_step_of_its_node_and_mixed_by_its_key() {
    // Soft Clip at 50 before it, a gain of 2 after: white comes back out of the clip.
    let after_clip = one(|c| {
        let h = &mut c.soft_clip.high;
        (h.r, h.g, h.b) = (fixed(50.0), fixed(50.0), fixed(50.0));
        c.shot_match = Some(shot([1.0; 3], [2.0; 3], [0.0; 3], 1.0, 0.4));
    });
    near(eval(&after_clip, [0.8, 0.3, 0.1]), [1.0, 0.6, 0.2], 1e-6);
    // Within the node, so its Key Output Gain weakens it with the rest.
    let half = one(|c| {
        c.key.output.gain = fixed(0.5);
        c.shot_match = Some(shot([1.0; 3], [2.0; 3], [0.0; 3], 1.0, 0.4));
    });
    near(eval(&half, [0.4; 3]), [0.6; 3], 1e-6);
}

#[test]
fn key_output_gain_is_the_strength_of_the_node() {
    let node = |gain: f64| {
        one(|c| {
            c.primaries.gain.r = fixed(2.0);
            c.primaries.lum_mix = fixed(0.0);
            c.key.output.gain = fixed(gain);
        })
    };
    near(eval(&node(0.0), [0.4; 3]), [0.4; 3], 0.0);
    near(eval(&node(0.5), [0.4; 3]), [0.6, 0.4, 0.4], 1e-6);
    near(eval(&node(2.0), [0.4; 3]), [0.8, 0.4, 0.4], 1e-6);
    // Inverted, a full key is none.
    let inverted = one(|c| {
        c.primaries.gain.r = fixed(2.0);
        c.key.output.invert = true;
    });
    near(eval(&inverted, [0.4; 3]), [0.4; 3], 0.0);
}

#[test]
fn keys_go_through_their_settings() {
    let k = |k: f32, invert: bool, gain: f32, offset: f32| {
        maths::key(k, [if invert { 1.0 } else { 0.0 }, gain, offset, 0.0])
    };
    assert_eq!(k(0.3, false, 1.0, 0.0), 0.3);
    assert!((k(0.3, true, 1.0, 0.0) - 0.7).abs() < 1e-6);
    assert!((k(0.3, false, 2.0, 0.0) - 0.6).abs() < 1e-6);
    assert_eq!(k(0.8, false, 2.0, 0.0), 1.0);
    // The offset moves the contrast and keeps white.
    assert_eq!(k(1.0, false, 1.0, -0.7), 1.0);
    assert!(k(0.5, false, 1.0, -0.5) < 0.5 && k(0.5, false, 1.0, 0.5) > 0.5);
    assert_eq!(maths::key_combine(0.5, 0.4, 1.0), 0.2);
    assert!((maths::key_combine(0.5, 0.4, 0.0) - 0.9).abs() < 1e-6);
    assert_eq!(maths::key_combine(0.8, 0.4, 0.0), 1.0);
}

#[test]
fn composite_modes_lay_the_front_over_the_back() {
    let c = constants();
    let b = [0.6, 0.3, 0.2];
    let f = [0.2, 0.5, 0.9];
    let mode = |m: Composite| maths::composite(&c, layout::composite(m), b, f);
    near(mode(Composite::Normal), f, 0.0);
    near(mode(Composite::Add), [0.8, 0.8, 1.1], 1e-6);
    near(mode(Composite::Subtract), [0.4, -0.2, -0.7], 1e-6);
    near(mode(Composite::Multiply), [0.12, 0.15, 0.18], 1e-6);
    near(mode(Composite::Screen), [0.68, 0.65, 0.92], 1e-6);
    near(mode(Composite::Darken), [0.2, 0.3, 0.2], 0.0);
    near(mode(Composite::Lighten), [0.6, 0.5, 0.9], 0.0);
    near(mode(Composite::Difference), [0.4, 0.2, 0.7], 1e-6);
    near(mode(Composite::LinearBurn), [-0.2, -0.2, 0.1], 1e-6);
    near(mode(Composite::Overlay), [0.36, 0.3, 0.36], 1e-6);
    near(mode(Composite::Hardlight), [0.24, 0.3, 0.84], 1e-6);
    near(mode(Composite::HardMix), [0.0, 0.0, 1.0], 0.0);
    // The modes of colour keep the luma they say.
    assert!((luma(mode(Composite::Luminosity)) - luma(f)).abs() < 1e-6);
    assert!((luma(mode(Composite::Color)) - luma(b)).abs() < 1e-6);
    assert!((luma(mode(Composite::Hue)) - luma(b)).abs() < 1e-6);
    assert!((luma(mode(Composite::Saturation)) - luma(b)).abs() < 1e-6);
    near(mode(Composite::DarkerColor), b, 0.0);
    near(mode(Composite::LighterColor), f, 0.0);
    // Every mode laid with itself over a grey of 0 or 1 is defined.
    for m in 0..27 {
        for (b, f) in [
            ([0.0; 3], [0.0; 3]),
            ([1.0; 3], [1.0; 3]),
            ([0.5; 3], [0.0; 3]),
        ] {
            assert!(
                maths::composite(&c, m, b, f).iter().all(|v| v.is_finite()),
                "{m}"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Graphs
// ---------------------------------------------------------------------------------------------

fn gain_node(id: &str, gain: [f64; 3]) -> Node {
    corrector(id, |c| {
        c.primaries.gain.r = fixed(gain[0]);
        c.primaries.gain.g = fixed(gain[1]);
        c.primaries.gain.b = fixed(gain[2]);
        c.primaries.lum_mix = fixed(0.0);
    })
}

#[test]
fn serial_nodes_are_done_one_after_another() {
    let a = gain_node("a", [2.0, 1.0, 1.0]);
    let b = corrector("b", |c| c.primaries.offset.r = fixed(35.0));
    let both = serial(vec![a.clone(), b.clone()]);
    let x = [0.3, 0.3, 0.3];
    near(eval(&both, x), [0.7, 0.3, 0.3], 1e-6);
    // The order of the links, not of the nodes, is the order of the grade.
    let mut reversed = serial(vec![b, a]);
    reversed.nodes.reverse();
    near(eval(&reversed, x), [0.8, 0.3, 0.3], 1e-6);
}

#[test]
fn a_parallel_mixer_means_its_inputs() {
    let grade = mixed(
        vec![
            gain_node("a", [2.0, 1.0, 1.0]),
            gain_node("b", [1.0, 1.0, 0.5]),
        ],
        NodeKind::ParallelMixer(ParallelMixer::default()),
    );
    near(eval(&grade, [0.4; 3]), [0.6, 0.4, 0.3], 1e-6);
    // A node's Key Output Gain gives less of its change to the mean.
    let mut weaker = grade.clone();
    if let NodeKind::Corrector(c) = &mut weaker.nodes[0].kind {
        c.key.output.gain = fixed(0.5);
    }
    near(eval(&weaker, [0.4; 3]), [0.5, 0.4, 0.3], 1e-6);
}

#[test]
fn a_layer_mixer_puts_its_lowest_input_on_top() {
    let layer = |composite| {
        mixed(
            vec![
                gain_node("a", [2.0, 1.0, 1.0]),
                gain_node("b", [1.0, 1.0, 0.5]),
            ],
            NodeKind::LayerMixer(LayerMixer {
                inputs: 2,
                composite,
            }),
        )
    };
    near(
        eval(&layer(Composite::Normal), [0.4; 3]),
        [0.4, 0.4, 0.2],
        1e-6,
    );
    near(
        eval(&layer(Composite::Add), [0.4; 3]),
        [1.2, 0.8, 0.6],
        1e-6,
    );
    // The lower layer at half its key: half over the upper.
    let mut half = layer(Composite::Normal);
    if let NodeKind::Corrector(c) = &mut half.nodes[1].kind {
        c.key.output.gain = fixed(0.5);
    }
    near(eval(&half, [0.4; 3]), [0.6, 0.4, 0.3], 1e-6);
    // The upper one's gain changes what is under the lower one only.
    let mut upper = layer(Composite::Normal);
    if let NodeKind::Corrector(c) = &mut upper.nodes[0].kind {
        c.key.output.gain = fixed(0.0);
    }
    near(eval(&upper, [0.4; 3]), [0.4, 0.4, 0.2], 1e-6);
}

#[test]
fn an_outside_node_works_where_the_key_before_it_is_not() {
    // n1 has a key of 0.25 (its output gain); n2 takes it inverted, so works at 0.75.
    let mut grade = serial(vec![
        corrector("n1", |c| c.key.output.gain = fixed(0.25)),
        corrector("n2", |c| {
            c.primaries.offset.r = fixed(45.0);
            c.key.input.invert = true;
        }),
    ]);
    grade.links.push(link("n1", "n2", Port::Key, 0));
    grade.sort_links();
    near(eval(&grade, [0.4; 3]), [0.55, 0.4, 0.4], 1e-6);
}

#[test]
fn a_key_mixer_meets_or_adds_its_keys() {
    // Two correctors give keys of 0.5 and 0.4 into a key mixer; the key goes into n3.
    let build = |matte: bool, output_gain: f64| {
        let mut grade = serial(vec![
            corrector("n1", |c| c.key.output.gain = fixed(0.5)),
            corrector("n2", |c| c.key.output.gain = fixed(0.4)),
            corrector("n3", |c| c.primaries.offset.b = fixed(125.0)),
        ]);
        let mut mixer = KeyMixer::default();
        mixer.inputs[1] = KeyInput {
            matte,
            ..KeyInput::default()
        };
        mixer.output.gain = fixed(output_gain);
        grade.nodes.push(Node {
            id: "km".into(),
            label: String::new(),
            enabled: true,
            locked: false,
            pos: None,
            kind: NodeKind::KeyMixer(mixer),
        });
        grade.links.extend([
            link("n1", "km", Port::Key, 0),
            link("n2", "km", Port::Key, 1),
            link("km", "n3", Port::Key, 0),
        ]);
        grade.sort_links();
        grade
    };
    // What n1 and n2 do to blue is nothing; n3 adds 1 by the mixed key.
    let blue = |grade: &Grade| eval(grade, [0.0; 3])[2];
    assert!((blue(&build(false, 1.0)) - 0.9).abs() < 1e-6);
    assert!((blue(&build(true, 1.0)) - 0.2).abs() < 1e-6);
    assert!((blue(&build(false, 0.5)) - 0.45).abs() < 1e-6);
    // Disabled, the mixer gives the key of its first input.
    let mut off = build(true, 1.0);
    off.nodes[3].enabled = false;
    assert!((blue(&off) - 0.5).abs() < 1e-6);
}

#[test]
fn a_disabled_node_passes_its_input() {
    let mut grade = serial(vec![
        gain_node("a", [2.0, 2.0, 2.0]),
        gain_node("b", [1.0, 1.0, 3.0]),
    ]);
    grade.nodes[0].enabled = false;
    near(eval(&grade, [0.2; 3]), [0.2, 0.2, 0.6], 1e-6);
    let mut mixers = mixed(
        vec![
            gain_node("a", [2.0, 1.0, 1.0]),
            gain_node("b", [1.0, 1.0, 0.5]),
        ],
        NodeKind::LayerMixer(LayerMixer::default()),
    );
    mixers.nodes[2].enabled = false;
    near(eval(&mixers, [0.4; 3]), [0.8, 0.4, 0.4], 1e-6);
}

#[test]
fn a_grade_that_is_not_complete_changes_nothing() {
    let mut grade = serial(vec![gain_node("a", [2.0, 2.0, 2.0])]);
    // A node left over, not linked to the output.
    grade.nodes.push(gain_node("b", [3.0, 3.0, 3.0]));
    grade.links.push(link(SOURCE, "b", Port::Rgb, 0));
    grade.sort_links();
    assert!(!grade.is_complete());
    assert!(stage_of(&grade, 0.0, &options(), &NoLuts).is_identity());
    near(eval(&grade, [0.2; 3]), [0.2; 3], 0.0);
    // Nor does one without nodes.
    assert!(stage_of(&Grade::default(), 0.0, &options(), &NoLuts).is_identity());
}

#[test]
fn keyframes_are_taken_at_the_frame() {
    let grade = one(|c| {
        c.primaries.gain.r = Param::Keys(vec![
            Key {
                frame: 0,
                value: 1.0,
                interp: Interp::Linear,
            },
            Key {
                frame: 10,
                value: 2.0,
                interp: Interp::Hold,
            },
            Key {
                frame: 20,
                value: 0.5,
                interp: Interp::Linear,
            },
        ]);
        c.primaries.lum_mix = fixed(0.0);
    });
    let at = |frame: f64| evaluate(&grade, frame, &options(), [0.4; 3])[0];
    assert!((at(-5.0) - 0.4).abs() < 1e-6);
    assert!((at(5.0) - 0.6).abs() < 1e-6);
    assert!((at(15.0) - 0.8).abs() < 1e-6);
    assert!((at(25.0) - 0.2).abs() < 1e-6);
    // The same shader at every frame, with other parameters.
    let ops = |frame| grade_ops(&grade, frame, &options(), &NoLuts);
    let (a, b) = (ops(0.0), ops(15.0));
    assert_eq!((&a[0].body, &a[0].library), (&b[0].body, &b[0].library));
    assert_ne!(a[0].params, b[0].params);
}

#[test]
fn only_the_shape_of_a_graph_makes_its_code() {
    let code = |grade: &Grade| {
        let ops = grade_ops(grade, 0.0, &options(), &NoLuts);
        let (program, _) = program(&[&Stage { ops }]).unwrap();
        program.code
    };
    let base = serial(vec![gain_node("a", [2.0, 1.0, 1.0]), Node::corrector("b")]);
    // Every setting, the points of the curves too.
    let mut settings = base.clone();
    if let NodeKind::Corrector(c) = &mut settings.nodes[1].kind {
        c.primaries.saturation = fixed(80.0);
        c.curves.y.points = vec![[0.0, 0.1], [0.3, 0.5], [0.7, 0.6], [1.0, 0.9]];
        c.hsl.hue_vs_hue = vec![[0.2, 0.1], [0.5, -0.3]];
        c.key.output.gain = fixed(0.4);
        c.key.input.invert = true;
        c.curves.ganged = false;
    }
    assert_eq!(code(&base), code(&settings));
    // Another shape: a node more, a node disabled, another mode of a mixer.
    let more = serial(vec![
        gain_node("a", [2.0, 1.0, 1.0]),
        Node::corrector("b"),
        Node::corrector("c"),
    ]);
    assert_ne!(code(&base), code(&more));
    let mut disabled = base.clone();
    disabled.nodes[0].enabled = false;
    assert_ne!(code(&base), code(&disabled));
    let layer = |composite| {
        mixed(
            vec![Node::corrector("a"), Node::corrector("b")],
            NodeKind::LayerMixer(LayerMixer {
                inputs: 2,
                composite,
            }),
        )
    };
    assert_ne!(
        code(&layer(Composite::Normal)),
        code(&layer(Composite::Screen))
    );
}

// ---------------------------------------------------------------------------------------------
// The card, held to the processor
// ---------------------------------------------------------------------------------------------

/// Runs stages on the card as the compositor's shaders do (`colour()` of their program), on
/// colours read from and written to buffers, keeping a pipeline for each program.
struct Card {
    gpu: Arc<Gpu>,
    pipelines: HashMap<String, (wgpu::ComputePipeline, wgpu::BindGroupLayout)>,
    io_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

const COMPUTE: &str = "
@group(0) @binding(0) var<storage, read> grade_test_in: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> grade_test_out: array<vec4<f32>>;
@compute @workgroup_size(64)
fn grade_test_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= arrayLength(&grade_test_in)) {
        return;
    }
    grade_test_out[id.x] = vec4<f32>(colour(grade_test_in[id.x].rgb), 1.0);
}
";

fn storage(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

impl Card {
    fn new() -> Option<Card> {
        let gpu = gpu_for_tests()?;
        let device = gpu.device();
        let io_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("grade test io"),
            entries: &[storage(0, true), storage(1, false)],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("grade test"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..wgpu::SamplerDescriptor::default()
        });
        Some(Card {
            gpu,
            pipelines: HashMap::new(),
            io_layout,
            sampler,
        })
    }

    fn buffer(&self, bytes: &[u8], usage: wgpu::BufferUsages) -> wgpu::Buffer {
        let buffer = self.gpu.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("grade test"),
            size: bytes.len() as u64,
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.gpu.queue().write_buffer(&buffer, 0, bytes);
        buffer
    }

    /// What the stage makes of each colour on the card.
    fn run(&mut self, stage: &Stage, colours: &[[f32; 3]]) -> Vec<[f32; 3]> {
        let (program, inputs) = program(&[stage]).expect("a stage that does something");
        let device = self.gpu.device().clone();
        if !self.pipelines.contains_key(&program.code) {
            let mut entries = vec![
                storage(0, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ];
            for (binding, kind) in (FIRST_TEXTURE_BINDING..).zip(&program.textures) {
                entries.push(wgpu::BindGroupLayoutEntry {
                    binding,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: kind.dimension(),
                        multisampled: false,
                    },
                    count: None,
                });
            }
            let colour_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("grade test colour"),
                entries: &entries,
            });
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("grade test"),
                source: wgpu::ShaderSource::Wgsl(format!("{}\n{COMPUTE}", program.code).into()),
            });
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("grade test"),
                bind_group_layouts: &[Some(&self.io_layout), Some(&colour_layout)],
                immediate_size: 0,
            });
            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("grade test"),
                layout: Some(&layout),
                module: &module,
                entry_point: Some("grade_test_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            });
            self.pipelines
                .insert(program.code.clone(), (pipeline, colour_layout));
        }
        let (pipeline, colour_layout) = &self.pipelines[&program.code];
        let floats = |values: &mut dyn Iterator<Item = f32>| -> Vec<u8> {
            values.flat_map(f32::to_le_bytes).collect()
        };
        let input_bytes = floats(&mut colours.iter().flat_map(|c| [c[0], c[1], c[2], 1.0]));
        let input = self.buffer(&input_bytes, wgpu::BufferUsages::STORAGE);
        let output = self.buffer(
            &vec![0; input_bytes.len()],
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grade test read"),
            size: input_bytes.len() as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let params = self.buffer(
            &floats(&mut inputs.params.iter().flatten().copied()),
            wgpu::BufferUsages::STORAGE,
        );
        let io = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("grade test io"),
            layout: &self.io_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: input.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: output.as_entire_binding(),
                },
            ],
        });
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: params.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&self.sampler),
            },
        ];
        for (binding, view) in (FIRST_TEXTURE_BINDING..).zip(&inputs.textures) {
            entries.push(wgpu::BindGroupEntry {
                binding,
                resource: wgpu::BindingResource::TextureView(view),
            });
        }
        let colour = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("grade test colour"),
            layout: colour_layout,
            entries: &entries,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &io, &[]);
            pass.set_bind_group(1, &colour, &[]);
            pass.dispatch_workgroups(u32::try_from(colours.len().div_ceil(64)).unwrap(), 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &read, 0, input_bytes.len() as u64);
        self.gpu.queue().submit([encoder.finish()]);
        read.slice(..)
            .map_async(wgpu::MapMode::Read, |result| result.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let bytes = read.slice(..).get_mapped_range().unwrap().to_vec();
        assert_eq!(self.gpu.uncaught_errors(), (0, None));
        bytes
            .as_chunks::<16>()
            .0
            .iter()
            .map(|texel| {
                let at = |i: usize| f32::from_le_bytes(texel[i * 4..i * 4 + 4].try_into().unwrap());
                [at(0), at(1), at(2)]
            })
            .collect()
    }
}

/// Whether the card's colour is the processor's, within 1e-3 (of the size of the value, where
/// it is more than 1), and within 1e-2 beyond 16.
fn close(card: [f32; 3], processor: [f32; 3]) -> bool {
    // Beyond 16 a value comes only of a division by almost nothing (Color Dodge, Divide), whose
    // error on the card grows with it: FXC there differs from the processor by some 2e-3.
    card.iter().zip(processor).all(|(a, b)| {
        let share = if b.abs() > 16.0 { 1e-2 } else { 1e-3 };
        (a - b).abs() <= share * b.abs().max(1.0)
    })
}

fn points(rng: &mut Rng, least: usize, round: bool) -> Vec<[f64; 2]> {
    let count = least + rng.below(5 - least.min(4));
    let mut xs: Vec<f64> = (0..count).map(|_| rng.unit()).collect();
    xs.sort_by(f64::total_cmp);
    xs.dedup_by(|a, b| (*a - *b).abs() < 0.02);
    if !round && least >= 2 {
        if let Some(first) = xs.first_mut() {
            *first = 0.0;
        }
        if xs.len() < 2 {
            xs.push(1.0);
        } else if let Some(last) = xs.last_mut() {
            *last = 1.0;
        }
    }
    xs.into_iter()
        .map(|x| {
            if least >= 2 {
                [x, (x + rng.range(-0.15, 0.15)).clamp(0.0, 1.0)]
            } else {
                [x, rng.range(-0.6, 0.6)]
            }
        })
        .collect()
}

/// A corrector with every setting at random, within the ranges a colourist uses.
#[allow(clippy::too_many_lines, reason = "every setting")]
fn random_corrector(rng: &mut Rng, id: &str) -> Node {
    let mut r = |low: f64, high: f64| fixed(rng.range(low, high));
    let mut node = Node::corrector(id);
    let NodeKind::Corrector(c) = &mut node.kind else {
        unreachable!()
    };
    let p = &mut c.primaries;
    for wheel in [&mut p.lift, &mut p.gamma] {
        (wheel.y, wheel.r, wheel.g, wheel.b) =
            (r(-0.3, 0.3), r(-0.3, 0.3), r(-0.3, 0.3), r(-0.3, 0.3));
    }
    (p.gain.y, p.gain.r, p.gain.g, p.gain.b) = (r(0.6, 1.6), r(0.6, 1.6), r(0.6, 1.6), r(0.6, 1.6));
    (p.offset.r, p.offset.g, p.offset.b) = (r(15.0, 35.0), r(15.0, 35.0), r(15.0, 35.0));
    p.contrast = r(0.5, 1.6);
    p.pivot = r(0.3, 0.6);
    p.saturation = r(0.0, 100.0);
    p.hue = r(0.0, 100.0);
    p.lum_mix = r(0.0, 100.0);
    p.temperature = r(-4000.0, 4000.0);
    p.tint = r(-100.0, 100.0);
    p.color_boost = r(-100.0, 100.0);
    p.shadows = r(-100.0, 100.0);
    p.highlights = r(-100.0, 100.0);
    let log = &mut c.log;
    for wheel in [&mut log.shadow, &mut log.midtone, &mut log.highlight] {
        (wheel.r, wheel.g, wheel.b) = (r(-0.4, 0.4), r(-0.4, 0.4), r(-0.4, 0.4));
    }
    log.low_range = r(0.2, 0.4);
    log.high_range = r(0.5, 0.7);
    let clip = &mut c.soft_clip;
    (clip.low.r, clip.low.g, clip.low.b) = (r(0.0, 10.0), r(0.0, 10.0), r(0.0, 10.0));
    (clip.high.r, clip.high.g, clip.high.b) = (r(85.0, 100.0), r(85.0, 100.0), r(85.0, 100.0));
    (clip.low_soft.r, clip.low_soft.g, clip.low_soft.b) =
        (r(0.0, 30.0), r(0.0, 30.0), r(0.0, 30.0));
    (clip.high_soft.r, clip.high_soft.g, clip.high_soft.b) =
        (r(0.0, 30.0), r(0.0, 30.0), r(0.0, 30.0));
    c.key.output.gain = r(0.0, 2.0);
    c.key.output.offset = r(-0.5, 0.5);
    c.key.input.gain = r(0.0, 2.0);
    let curves = &mut c.curves;
    for curve in [&mut curves.y, &mut curves.r, &mut curves.g, &mut curves.b] {
        curve.intensity = r(0.0, 100.0);
    }
    curves.ganged = rng.chance(0.5);
    curves.y.points = points(rng, 2, false);
    curves.r.points = points(rng, 2, false);
    curves.g.points = points(rng, 2, false);
    curves.b.points = points(rng, 2, false);
    let hsl = &mut c.hsl;
    hsl.hue_vs_hue = points(rng, 0, true);
    hsl.hue_vs_sat = points(rng, 0, true);
    hsl.hue_vs_lum = points(rng, 0, true);
    hsl.lum_vs_sat = points(rng, 0, false);
    hsl.sat_vs_sat = points(rng, 0, false);
    hsl.sat_vs_lum = points(rng, 0, false);
    // Some settings at their neutral values, as most are in a grade.
    if rng.chance(0.3) {
        c.hsl = columbina_model::grade::HslCurves::default();
    }
    if rng.chance(0.3) {
        c.soft_clip = columbina_model::grade::SoftClip::default();
    }
    if rng.chance(0.2) {
        c.primaries.contrast = fixed(1.0);
    }
    node
}

/// `node`, a corrector, with a random shot match half the time.
fn with_random_shot_match(rng: &mut Rng, mut node: Node) -> Node {
    let NodeKind::Corrector(c) = &mut node.kind else {
        unreachable!()
    };
    if rng.chance(0.5) {
        let mut r = |low: f64, high: f64| rng.range(low, high);
        c.shot_match = Some(shot(
            [r(0.5, 2.0), r(0.5, 2.0), r(0.5, 2.0)],
            [r(0.7, 1.4), r(0.7, 1.4), r(0.7, 1.4)],
            [r(-0.1, 0.1), r(-0.1, 0.1), r(-0.1, 0.1)],
            if rng.chance(0.3) {
                1.0
            } else {
                rng.range(0.6, 1.8)
            },
            rng.range(0.3, 0.6),
        ));
    }
    node
}

#[track_caller]
fn assert_card_is_processor(card: &mut Card, grade: &Grade, options: &GradeOptions, rng: &mut Rng) {
    let stage = stage_of(grade, 0.0, options, &NoLuts);
    let colours: Vec<[f32; 3]> = (0..256).map(|_| rng.colour(-0.1, 1.3)).collect();
    let card_colours = card.run(&stage, &colours);
    for (x, on_card) in colours.iter().zip(card_colours) {
        let on_processor = evaluate(grade, 0.0, options, *x);
        assert!(
            close(on_card, on_processor),
            "{x:?}: the card made {on_card:?}, the processor {on_processor:?}"
        );
    }
}

#[test]
fn the_card_does_each_corrector_as_the_processor_does() {
    let Some(mut card) = Card::new() else { return };
    let mut rng = Rng(0x5eed_1234_abcd_0001);
    for round in 0..300 {
        let node = random_corrector(&mut rng, "n1");
        let grade = serial(vec![with_random_shot_match(&mut rng, node)]);
        let options = GradeOptions {
            s_curve_contrast: round % 3 != 0,
            ..GradeOptions::default()
        };
        assert_card_is_processor(&mut card, &grade, &options, &mut rng);
    }
    // Two shapes only (the S-curve on and off): the parameters changed, the pipelines did not.
    assert_eq!(card.pipelines.len(), 2);
}

/// The composite modes, in the order of their numbers.
const MODES: [Composite; 27] = [
    Composite::Normal,
    Composite::Add,
    Composite::Color,
    Composite::ColorBurn,
    Composite::ColorDodge,
    Composite::Darken,
    Composite::DarkerColor,
    Composite::Difference,
    Composite::Divide,
    Composite::Exclusion,
    Composite::HardMix,
    Composite::Hardlight,
    Composite::Hue,
    Composite::Lighten,
    Composite::LighterColor,
    Composite::LinearBurn,
    Composite::LinearDodge,
    Composite::LinearLight,
    Composite::Luminosity,
    Composite::Multiply,
    Composite::Overlay,
    Composite::PinLight,
    Composite::Saturation,
    Composite::Screen,
    Composite::Softlight,
    Composite::Subtract,
    Composite::VividLight,
];

#[test]
fn the_card_lays_every_composite_mode_as_the_processor_does() {
    let Some(mut card) = Card::new() else { return };
    let mut rng = Rng(0x0dd_ba11);
    for (mode, composite) in MODES.into_iter().enumerate() {
        assert_eq!(layout::composite(composite) as usize, mode);
        for _ in 0..4 {
            let grade = mixed(
                vec![
                    random_corrector(&mut rng, "a"),
                    random_corrector(&mut rng, "b"),
                    random_corrector(&mut rng, "c"),
                ],
                NodeKind::LayerMixer(LayerMixer {
                    inputs: 3,
                    composite,
                }),
            );
            assert_card_is_processor(&mut card, &grade, &options(), &mut rng);
        }
    }
}

#[test]
fn the_card_does_graphs_of_every_kind_as_the_processor_does() {
    let Some(mut card) = Card::new() else { return };
    let mut rng = Rng(0xfeed_beef);
    for _ in 0..20 {
        let r = &mut rng;
        // Serial, then a parallel of two, then a layer of two whose lower one takes the key of
        // the first node inverted, then a node keyed by a key mixer of two keys.
        let mut grade = graph(
            vec![
                random_corrector(r, "s1"),
                random_corrector(r, "p1"),
                random_corrector(r, "p2"),
                Node {
                    id: "pm".into(),
                    label: String::new(),
                    enabled: true,
                    locked: false,
                    pos: None,
                    kind: NodeKind::ParallelMixer(ParallelMixer::default()),
                },
                random_corrector(r, "l1"),
                random_corrector(r, "l2"),
                Node {
                    id: "lm".into(),
                    label: String::new(),
                    enabled: true,
                    locked: false,
                    pos: None,
                    kind: NodeKind::LayerMixer(LayerMixer {
                        inputs: 2,
                        composite: Composite::Screen,
                    }),
                },
                random_corrector(r, "k1"),
                Node {
                    id: "km".into(),
                    label: String::new(),
                    enabled: true,
                    locked: false,
                    pos: None,
                    kind: NodeKind::KeyMixer(KeyMixer::default()),
                },
            ],
            vec![
                link(SOURCE, "s1", Port::Rgb, 0),
                link("s1", "p1", Port::Rgb, 0),
                link("s1", "p2", Port::Rgb, 0),
                link("p1", "pm", Port::Rgb, 0),
                link("p2", "pm", Port::Rgb, 1),
                link("pm", "l1", Port::Rgb, 0),
                link("pm", "l2", Port::Rgb, 0),
                link("l1", "lm", Port::Rgb, 0),
                link("l2", "lm", Port::Rgb, 1),
                link("s1", "l2", Port::Key, 0),
                link("lm", "k1", Port::Rgb, 0),
                link("p1", "km", Port::Key, 0),
                link("l1", "km", Port::Key, 1),
                link("km", "k1", Port::Key, 0),
                link("k1", OUTPUT, Port::Rgb, 0),
            ],
        );
        if let NodeKind::Corrector(c) = &mut grade.nodes[5].kind {
            c.key.input.invert = true;
        }
        if let NodeKind::KeyMixer(m) = &mut grade.nodes[8].kind {
            m.inputs[1].matte = r.chance(0.5);
            m.inputs[0].gain = fixed(r.range(0.0, 2.0));
            m.output.offset = fixed(r.range(-0.5, 0.5));
        }
        // Now and then a node disabled.
        if r.chance(0.3) {
            let i = r.below(9);
            grade.nodes[i].enabled = false;
        }
        assert!(grade.is_complete());
        assert_card_is_processor(&mut card, &grade, &options(), &mut rng);
    }
}

#[test]
fn the_card_leaves_a_colour_as_it_is_through_nodes_as_they_are_made() {
    let Some(mut card) = Card::new() else { return };
    let grade = mixed(
        vec![Node::corrector("a"), Node::corrector("b")],
        NodeKind::ParallelMixer(ParallelMixer::default()),
    );
    let colours = colours();
    let out = card.run(&stage_of(&grade, 0.0, &options(), &NoLuts), &colours);
    for (x, out) in colours.iter().zip(out) {
        assert_eq!(out, *x);
    }
}

/// LUTs that invert, by a cube of 2 × 2 × 2 on the card.
struct Inverting(OpTexture);

impl Luts for Inverting {
    fn lut(&self, name: &str) -> Option<ColorOp> {
        (name == "invert.cube").then(|| ColorOp {
            body: "let at = (clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * (param($P).x - 1.0) \
                   + 0.5) / param($P).x;\n\
                   rgb = textureSampleLevel($T0, $S, at, 0.0).rgb;"
                .into(),
            library: "fn lut_test_unused() {}".into(),
            params: vec![[2.0, 0.0, 0.0, 0.0]],
            textures: vec![self.0.clone()],
            ..ColorOp::default()
        })
    }
}

#[test]
fn a_lut_is_put_in_its_place_in_the_node() {
    let Some(mut card) = Card::new() else { return };
    let device = card.gpu.device();
    let cube = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("inverting cube"),
        size: wgpu::Extent3d {
            width: 2,
            height: 2,
            depth_or_array_layers: 2,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut texels = Vec::new();
    for b in 0..2_u8 {
        for g in 0..2_u8 {
            for r in 0..2_u8 {
                texels.extend([255 * (1 - r), 255 * (1 - g), 255 * (1 - b), 255]);
            }
        }
    }
    card.gpu.queue().write_texture(
        cube.as_image_copy(),
        &texels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(8),
            rows_per_image: Some(2),
        },
        cube.size(),
    );
    let luts = Inverting(OpTexture {
        kind: TextureKind::D3,
        view: cube.create_view(&wgpu::TextureViewDescriptor::default()),
    });
    // The LUT after the saturation and before Soft Clip; two nodes name it, one names another.
    let node = |id: &str, lut: &str| {
        corrector(id, |c| {
            c.primaries.saturation = fixed(70.0);
            c.lut = Some(lut.into());
            let h = &mut c.soft_clip.high;
            (h.r, h.g, h.b) = (fixed(80.0), fixed(80.0), fixed(80.0));
        })
    };
    let grade = serial(vec![
        node("n1", "invert.cube"),
        node("n2", "missing.cube"),
        node("n3", "invert.cube"),
    ]);
    let invert = |_: &str, x: [f32; 3]| x.map(|v| 1.0 - v.clamp(0.0, 1.0));
    let stage = stage_of(&grade, 0.0, &options(), &luts);
    // The library of the LUT once, in an operation of its own; the textures of both nodes.
    assert_eq!(stage.ops.len(), 2);
    assert_eq!(stage.ops[1].textures.len(), 2);
    let colours: Vec<[f32; 3]> = (0..64)
        .map(|i| {
            #[allow(clippy::cast_precision_loss, reason = "a small count")]
            let v = i as f32 / 64.0;
            [v, 1.0 - v, 0.5]
        })
        .collect();
    let out = card.run(&stage, &colours);
    for (x, out) in colours.iter().zip(out) {
        // The card filters a texture with weights of 8 bits (1/256), so the LUT is near.
        let wanted = evaluate_with(&grade, 0.0, &options(), &luts, &invert, *x);
        assert!(
            out.iter().zip(wanted).all(|(a, b)| (a - b).abs() < 0.01),
            "{x:?}: {out:?}, not {wanted:?}"
        );
    }
    // Without the LUTs, the nodes have none.
    let without = evaluate(&grade, 0.0, &options(), [0.2, 0.4, 0.6]);
    let with = evaluate_with(&grade, 0.0, &options(), &luts, &invert, [0.2, 0.4, 0.6]);
    assert_ne!(without, with);
}

#[test]
fn a_change_of_settings_makes_no_pipeline_and_a_change_of_shape_does() {
    // The compositor keeps a pipeline for each program of colour, by its code, as this does.
    let Some(mut card) = Card::new() else { return };
    let x = [[0.4, 0.3, 0.2]];
    let mut grade = serial(vec![gain_node("a", [2.0, 1.0, 1.0])]);
    let first = card.run(&stage_of(&grade, 0.0, &options(), &NoLuts), &x);
    if let NodeKind::Corrector(c) = &mut grade.nodes[0].kind {
        c.primaries.gain.r = fixed(0.5);
        c.curves.y.points = vec![[0.0, 0.0], [0.5, 0.6], [1.0, 1.0]];
    }
    let second = card.run(&stage_of(&grade, 0.0, &options(), &NoLuts), &x);
    assert_ne!(first, second);
    assert_eq!(card.pipelines.len(), 1);
    grade.nodes[0].enabled = false;
    let _ = card.run(&stage_of(&grade, 0.0, &options(), &NoLuts), &x);
    assert_eq!(card.pipelines.len(), 2);
    let more = serial(vec![gain_node("a", [2.0, 1.0, 1.0]), Node::corrector("b")]);
    let _ = card.run(&stage_of(&more, 0.0, &options(), &NoLuts), &x);
    assert_eq!(card.pipelines.len(), 3);
}

/// A folder of its own for the files of a test.
fn folder(name: &str) -> std::path::PathBuf {
    let folder =
        std::env::temp_dir().join(format!("columbina-grade-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

/// A `.cube` of `size`³ of `f`, with a shaper of `shaper` points that squares, if any.
fn cube_file(size: usize, shaper: usize, f: impl Fn([f64; 3]) -> [f64; 3]) -> String {
    use std::fmt::Write as _;
    let mut text = String::from("TITLE \"test\"\n");
    if shaper > 0 {
        let _ = writeln!(text, "LUT_1D_SIZE {shaper}\nLUT_1D_INPUT_RANGE 0.0 1.0");
    }
    let _ = writeln!(text, "LUT_3D_SIZE {size}\nLUT_3D_INPUT_RANGE 0.0 1.0");
    #[allow(clippy::cast_precision_loss, reason = "small counts")]
    for i in 0..shaper {
        let v = (i as f64 / (shaper - 1) as f64).powi(2);
        let _ = writeln!(text, "{v} {v} {v}");
    }
    #[allow(clippy::cast_precision_loss, reason = "small counts")]
    let at = |i: usize| i as f64 / (size - 1) as f64;
    for b in 0..size {
        for g in 0..size {
            for r in 0..size {
                let [x, y, z] = f([at(r), at(g), at(b)]);
                let _ = writeln!(text, "{x} {y} {z}");
            }
        }
    }
    text
}

#[test]
fn luts_named_by_nodes_are_read_and_put_on_the_card() {
    let Some(mut card) = Card::new() else { return };
    let dir = folder("luts");
    let mixing = |[r, g, b]: [f64; 3]| [r * r, f64::midpoint(g, b), b.sqrt() * 0.9 + 0.05];
    let other = |[r, g, b]: [f64; 3]| [1.0 - r, g * g, 0.3 * r + 0.7 * b];
    std::fs::write(dir.join("look.cube"), cube_file(9, 0, mixing)).unwrap();
    std::fs::write(dir.join("other.cube"), cube_file(9, 0, other)).unwrap();
    std::fs::write(dir.join("shaped.cube"), cube_file(5, 64, other)).unwrap();
    std::fs::write(dir.join("broken.cube"), "LUT_3D_SIZE 2\n0 0 0\n").unwrap();
    let luts = CardLuts::new(
        Arc::clone(&card.gpu),
        Arc::new(columbina_lut::LutCache::default()),
        columbina_lut::Roots {
            project: Some(dir.clone()),
            library: Vec::new(),
        },
    );
    let graded = |first: &str, second: &str| {
        serial(vec![
            corrector("n1", |c| {
                c.primaries.saturation = fixed(80.0);
                c.lut = Some(first.into());
                c.hsl.hue_vs_sat = vec![[0.0, -0.3]];
            }),
            corrector("n2", |c| {
                c.primaries.gain.g = fixed(1.2);
                c.lut = Some(second.into());
            }),
        ])
    };
    let mut rng = Rng(99);
    let colours: Vec<[f32; 3]> = (0..256).map(|_| rng.colour(0.0, 1.0)).collect();
    let mut codes = Vec::new();
    for (first, second) in [
        ("look.cube", "other.cube"),
        ("other.cube", "look.cube"),
        ("shaped.cube", "look.cube"),
    ] {
        let grade = graded(first, second);
        luts.prepare(&grade);
        let stage = stage_of(&grade, 0.0, &options(), &luts);
        codes.push(program(&[&stage]).unwrap().0.code);
        let table = |name: &str, x: [f32; 3]| {
            luts.table(name)
                .unwrap()
                .eval(x, columbina_lut::Interpolation::Trilinear)
        };
        let out = card.run(&stage, &colours);
        for (x, out) in colours.iter().zip(out) {
            // The tables are of half floats on the card.
            let wanted = evaluate_with(&grade, 0.0, &options(), &luts, &table, *x);
            assert!(
                out.iter().zip(wanted).all(|(a, b)| (a - b).abs() < 2e-3),
                "{first}, {second}, {x:?}: {out:?}, not {wanted:?}"
            );
        }
    }
    // Two LUTs of a shape swapped: the same shader. A shaper more: another.
    assert_eq!(codes[0], codes[1]);
    assert_ne!(codes[0], codes[2]);
    // A LUT that cannot be read leaves its node without one, and says why.
    let grade = graded("broken.cube", "nowhere.cube");
    luts.prepare(&grade);
    assert!(luts.error("broken.cube").is_some());
    assert!(luts.error("nowhere.cube").unwrap().contains("nowhere.cube"));
    assert!(luts.error("look.cube").is_none());
    let stage = stage_of(&grade, 0.0, &options(), &luts);
    assert!(stage.ops.iter().all(|op| op.textures.is_empty()));
    let x = [0.3, 0.5, 0.7];
    assert!(close(
        card.run(&stage, &[x])[0],
        evaluate(&grade, 0.0, &options(), x)
    ));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn textures_of_luts_used_longest_ago_go_while_they_take_more_than_the_budget() {
    type Table = fn([f64; 3]) -> [f64; 3];
    let Some(mut card) = Card::new() else { return };
    let dir = folder("budget");
    let tables: [(&str, Table); 3] = [
        ("a.cube", |[r, g, b]| [1.0 - r, g, b]),
        ("b.cube", |[r, g, b]| [r, 1.0 - g, b]),
        ("c.cube", |[r, g, b]| [r, g, 1.0 - b]),
    ];
    for (name, f) in tables {
        std::fs::write(dir.join(name), cube_file(9, 0, f)).unwrap();
    }
    let mut luts = CardLuts::new(
        Arc::clone(&card.gpu),
        Arc::new(columbina_lut::LutCache::default()),
        columbina_lut::Roots {
            project: Some(dir.clone()),
            library: Vec::new(),
        },
    );
    assert!(luts.lut("a.cube").is_some());
    // A cube of 9³ texels of four half floats.
    let one = 9 * 9 * 9 * 8;
    assert_eq!(luts.texture_bytes(), (one, 1));
    luts.set_budget(2 * one);
    assert!(luts.lut("b.cube").is_some());
    assert_eq!(luts.texture_bytes(), (2 * one, 2));
    // a used again; then c: b, used longest ago, goes.
    let node = |name: &str| serial(vec![corrector("n", |c| c.lut = Some(name.into()))]);
    let colours = [[0.2, 0.3, 0.8], [0.9, 0.1, 0.4]];
    let before = card.run(&stage_of(&node("b.cube"), 0.0, &options(), &luts), &colours);
    assert!(luts.lut("a.cube").is_some());
    assert!(luts.lut("c.cube").is_some());
    assert_eq!(luts.texture_bytes(), (2 * one, 2));
    // b is made again as it is asked for, the same LUT; a goes, used longest ago now.
    let again = card.run(&stage_of(&node("b.cube"), 0.0, &options(), &luts), &colours);
    assert_eq!(before, again);
    near(again[0], [0.2, 0.7, 0.8], 2e-3);
    assert_eq!(luts.texture_bytes(), (2 * one, 2));
    // A budget of nothing keeps none; what a grade made ready holds is still on the card.
    let held = stage_of(&node("c.cube"), 0.0, &options(), &luts);
    luts.set_budget(0);
    assert_eq!(luts.texture_bytes(), (0, 0));
    let out = card.run(&held, &colours);
    near(out[1], [0.9, 0.1, 0.6], 2e-3);
    let _ = std::fs::remove_dir_all(dir);
}

mod keys;
