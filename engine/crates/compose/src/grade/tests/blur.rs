//! The Blur palette (pp. 3454–3456) on the processor: neutral in every mode to the bit, what each
//! mode does to pictures, channel by channel, and with keys and windows; and the neighbourhoods
//! and code that it makes.

use columbina_model::grade::{blur as model_blur, BlurMode, Qualifier, Window, WindowShape};

use super::*;
use crate::grade::blur::{self, BLUR_SIGMA, MIST_SIGMA, SHARPEN_GAIN};

/// A corrector set by `set` with its Blur palette in `mode`.
fn blurred(mode: BlurMode, set: impl FnOnce(&mut columbina_model::grade::Blur)) -> Grade {
    one(|c| {
        c.blur.mode = mode;
        set(&mut c.blur);
    })
}

fn radius(r: f64) -> impl FnOnce(&mut columbina_model::grade::Blur) {
    move |b| {
        b.radius.r = fixed(r);
        b.radius.g = fixed(r);
        b.radius.b = fixed(r);
    }
}

fn neighbourhoods(grade: &Grade) -> Vec<crate::Neighbourhood> {
    grade_ops(grade, 0.0, &options(), &NoLuts)
        .into_iter()
        .flat_map(|op| op.neighbourhoods)
        .collect()
}

fn code(grade: &Grade) -> String {
    grade_ops(grade, 0.0, &options(), &NoLuts)
        .last()
        .unwrap()
        .body
        .to_string()
}

fn picture_of(
    grade: &Grade,
    frame: f64,
    (width, height): (u32, u32),
    pixels: &[[f32; 3]],
) -> Vec<[f32; 3]> {
    evaluate_picture(
        grade,
        frame,
        &options(),
        &NoLuts,
        &|_, rgb| rgb,
        (width, height),
        pixels,
        1.0,
    )
}

/// A dark picture of 81 × 41 with a line of light 1 pixel wide down its middle column and
/// another along its middle row.
#[allow(clippy::cast_precision_loss, reason = "places of pixels")]
fn cross() -> Vec<[f32; 3]> {
    (0..41_u32)
        .flat_map(|y| (0..81_u32).map(move |x| (x, y)))
        .map(|(x, y)| {
            if x == 40 || y == 20 {
                [1.0, 0.5, 0.25]
            } else {
                [0.0; 3]
            }
        })
        .collect()
}

/// A dark picture of 81 × 41 with a line of light 1 pixel wide down its middle column.
fn line() -> Vec<[f32; 3]> {
    (0..41_u32)
        .flat_map(|y| (0..81_u32).map(move |x| (x, y)))
        .map(|(x, _)| if x == 40 { [1.0, 0.5, 0.25] } else { [0.0; 3] })
        .collect()
}

/// A picture of 80 × 40, dark on its left half and light on its right, with a faint checker of
/// noise on both.
fn edge() -> Vec<[f32; 3]> {
    (0..40_u32)
        .flat_map(|y| (0..80_u32).map(move |x| (x, y)))
        .map(|(x, y)| {
            let base = if x < 40 { 0.25 } else { 0.75 };
            let noise = if (x + y) % 2 == 0 {
                0.007_812_5
            } else {
                -0.007_812_5
            };
            [base + noise; 3]
        })
        .collect()
}

#[test]
fn a_blur_palette_that_changes_nothing_is_the_node_as_it_was_to_the_bit() {
    let plain = one(|_| {});
    for mode in [BlurMode::Blur, BlurMode::Sharpen, BlurMode::Mist] {
        // The mode, the gangs, and what does nothing in it make no neighbourhood and no other code.
        let quiet = blurred(mode, |b| {
            b.linked.radius = false;
            b.scaling.r = fixed(1.0);
            b.ratio.g = fixed(0.9);
            b.level = fixed(0.4);
            b.coring_softness = fixed(0.2);
            if mode != BlurMode::Mist {
                b.mix = fixed(30.0);
            }
        });
        assert!(neighbourhoods(&quiet).is_empty(), "{mode:?}");
        assert_eq!(code(&quiet), code(&plain), "{mode:?}");
        let ops = grade_ops(&quiet, 0.0, &options(), &NoLuts);
        let before = grade_ops(&plain, 0.0, &options(), &NoLuts);
        assert_eq!(ops.len(), before.len());
        assert_eq!(ops.last().unwrap().params, before.last().unwrap().params);
        for rgb in colours() {
            assert_eq!(eval(&quiet, rgb), eval(&plain, rgb), "{mode:?} {rgb:?}");
        }
    }
}

#[test]
fn a_picture_of_one_colour_and_a_pixel_alone_are_left_as_they_are() {
    // Values a half float keeps, as the neighbourhoods are: within, beyond white, below black.
    let grades = [
        blurred(BlurMode::Blur, radius(0.9)),
        blurred(BlurMode::Blur, radius(0.1)),
        blurred(BlurMode::Sharpen, |b| {
            radius(0.0)(b);
            b.scaling.g = fixed(1.0);
            b.level = fixed(0.3);
        }),
        blurred(BlurMode::Mist, |b| {
            radius(0.2)(b);
            b.mix = fixed(20.0);
        }),
    ];
    for grade in &grades {
        for colour in [[0.25, 0.5, 0.75], [4.0, 2.5, 1.5], [-0.25, -0.125, 0.0625]] {
            for pixel in picture_of(grade, 0.0, (9, 7), &[colour; 63]) {
                near(pixel, colour, 1e-6);
            }
            near(eval(grade, colour), colour, 0.0);
        }
    }
}

#[allow(clippy::cast_precision_loss, reason = "places of pixels")]
#[test]
fn a_radius_above_half_blurs_by_a_gaussian_shared_by_the_ratio() {
    // A Radius of 0.6: a Gaussian of a standard deviation of BLUR_SIGMA / 25, here 1 pixel.
    let sigma = BLUR_SIGMA * 0.2 * 0.2;
    assert!((sigma - 1.0).abs() < 1e-9);
    let at = |pixels: &[[f32; 3]], x: usize, y: usize| pixels[y * 81 + x];
    let both = picture_of(
        &blurred(BlurMode::Blur, radius(0.6)),
        0.0,
        (81, 41),
        &cross(),
    );
    // Far from the crossing, the line down the middle is spread across, as a Gaussian of 1.
    let peak = at(&both, 40, 5)[0];
    assert!((at(&both, 41, 5)[0] / peak - (-0.5_f32).exp()).abs() < 0.01);
    assert!((at(&both, 42, 5)[0] / peak - (-2.0_f32).exp()).abs() < 0.01);
    // Each channel of it, its own colour.
    assert!((at(&both, 41, 5)[1] / at(&both, 41, 5)[0] - 0.5).abs() < 0.01);
    // H/V Ratio 1: across alone; the line along the middle row is not spread down.
    let across = picture_of(
        &blurred(BlurMode::Blur, |b| {
            radius(0.6)(b);
            b.ratio.r = fixed(1.0);
            b.ratio.g = fixed(1.0);
            b.ratio.b = fixed(1.0);
        }),
        0.0,
        (81, 41),
        &cross(),
    );
    assert!(at(&across, 41, 5)[0] > 0.1);
    near(at(&across, 10, 21), [0.0; 3], 1e-6);
    // 0: down alone.
    let down = picture_of(
        &blurred(BlurMode::Blur, |b| {
            radius(0.6)(b);
            b.ratio.r = fixed(0.0);
            b.ratio.g = fixed(0.0);
            b.ratio.b = fixed(0.0);
        }),
        0.0,
        (81, 41),
        &cross(),
    );
    near(at(&down, 41, 5), [0.0; 3], 1e-6);
    assert!(at(&down, 10, 21)[0] > 0.1);
    // Higher, wider.
    let wider = picture_of(
        &blurred(BlurMode::Blur, radius(0.8)),
        0.0,
        (81, 41),
        &cross(),
    );
    assert!(at(&wider, 45, 5)[0] > 0.01 && at(&both, 45, 5)[0] < 1e-3);
}

/// The most that a picture overshoots its two levels near the edge of [`edge`], along row 20.
fn overshoot(pixels: &[[f32; 3]]) -> f32 {
    let row = &pixels[20 * 80..21 * 80];
    let high = row[40..44].iter().map(|p| p[0]).fold(f32::MIN, f32::max);
    let low = row[36..40].iter().map(|p| p[0]).fold(f32::MAX, f32::min);
    (high - 0.76).max(0.24 - low)
}

#[test]
fn a_radius_below_half_sharpens_more_as_it_is_lowered_and_scaling_multiplies_it() {
    let at = |grade: &Grade| overshoot(&picture_of(grade, 0.0, (80, 40), &edge()));
    assert!(at(&blurred(BlurMode::Blur, |_| {})) < 0.0);
    let some = at(&blurred(BlurMode::Blur, radius(0.4)));
    let more = at(&blurred(BlurMode::Blur, radius(0.1)));
    assert!(some > 0.01 && more > 2.0 * some, "{some} {more}");
    // In the Sharpen mode, Scaling multiplies it; in the Blur mode its Scaling is grey.
    let scaled = |mode: BlurMode| {
        at(&blurred(mode, |b| {
            radius(0.4)(b);
            b.scaling.r = fixed(1.0);
            b.scaling.g = fixed(1.0);
            b.scaling.b = fixed(1.0);
        }))
    };
    let sharpen = at(&blurred(BlurMode::Sharpen, radius(0.4)));
    assert!((sharpen - some).abs() < 1e-6);
    assert!(scaled(BlurMode::Sharpen) > 3.0 * sharpen);
    assert!((scaled(BlurMode::Blur) - some).abs() < 1e-6);
    // Scaling does nothing to a blur.
    let blur_at = |scaling: f64| {
        picture_of(
            &blurred(BlurMode::Sharpen, |b| {
                radius(0.7)(b);
                b.scaling.r = fixed(scaling);
            }),
            0.0,
            (80, 40),
            &edge(),
        )
    };
    assert_eq!(blur_at(0.25), blur_at(1.0));
    // The amount: Radius 0, Scaling 0.25.
    let block = blur::block(
        &{
            let mut b = model_blur::Blur::default();
            b.radius.r = fixed(0.0);
            b
        },
        0.0,
    );
    #[allow(clippy::cast_possible_truncation, reason = "a constant")]
    let gain = SHARPEN_GAIN as f32;
    assert_eq!(block[1][0], gain);
    assert_eq!(block[0][..3], [2.0, 0.0, 0.0]);
}

#[test]
fn level_omits_the_faint_detail_and_coring_softness_blends_its_border() {
    let pixels = edge();
    let graded = |level: f64, soft: f64| {
        picture_of(
            &blurred(BlurMode::Sharpen, |b| {
                radius(0.0)(b);
                b.level = fixed(level);
                b.coring_softness = fixed(soft);
            }),
            0.0,
            (80, 40),
            &pixels,
        )
    };
    // The noise, far from the edge, and the edge itself.
    let noise = |out: &[[f32; 3]]| (out[20 * 80 + 10][0] - pixels[20 * 80 + 10][0]).abs();
    let everything = graded(0.0, 0.0);
    assert!(noise(&everything) > 0.01);
    let outlines = graded(0.25, 0.0);
    assert!(noise(&outlines) < 1e-6, "{}", noise(&outlines));
    assert!(overshoot(&outlines) > 0.05);
    // Softness between: some of the noise, less than all of it.
    let soft = graded(0.0, 0.25);
    assert!(noise(&soft) > 1e-4 && noise(&soft) < noise(&everything));
    // Level and Coring Softness are grey in the other modes.
    for mode in [BlurMode::Blur, BlurMode::Mist] {
        let other = picture_of(
            &blurred(mode, |b| {
                radius(0.0)(b);
                b.level = fixed(0.25);
            }),
            0.0,
            (80, 40),
            &pixels,
        );
        assert_eq!(other, everything, "{mode:?}");
    }
}

#[allow(clippy::cast_precision_loss, reason = "places of pixels")]
#[test]
fn mist_mixes_a_wider_superimposed_blur_by_mix_with_what_the_radius_made() {
    let sharpened = blurred(BlurMode::Mist, radius(0.2));
    let misted = |mix: f64, r: f64| {
        blurred(BlurMode::Mist, |b| {
            radius(r)(b);
            b.mix = fixed(mix);
        })
    };
    let at = |pixels: &[[f32; 3]], x: usize, y: usize| pixels[y * 81 + x][0];
    let sharp = picture_of(&sharpened, 0.0, (81, 41), &line());
    // Mix 100 adds nothing: the sharpening alone, and nothing far from the line.
    assert_eq!(
        picture_of(&misted(100.0, 0.2), 0.0, (81, 41), &line()),
        sharp
    );
    assert!(at(&sharp, 50, 5).abs() < 1e-6);
    // Lowered, a glow wider than the detail: light ten pixels from the line, the line dimmer.
    let half = picture_of(&misted(50.0, 0.2), 0.0, (81, 41), &line());
    assert!(at(&half, 50, 5) > 0.005, "{}", at(&half, 50, 5));
    assert!(at(&half, 40, 5) < at(&sharp, 40, 5));
    // Mist with the detail kept: the sharpened line, less the glow, is more than the line
    // mixed alone with the glow.
    let unsharpened = picture_of(&misted(50.0, 0.5), 0.0, (81, 41), &line());
    assert!(at(&half, 40, 5) > at(&unsharpened, 40, 5));
    // Mix 0: the superimposed blur alone, a Gaussian of MIST_SIGMA.
    let all = picture_of(&misted(0.0, 0.5), 0.0, (81, 41), &line());
    let ratio = at(&all, 52, 5) / at(&all, 40, 5);
    #[allow(clippy::cast_possible_truncation, reason = "a constant")]
    let sigma = MIST_SIGMA as f32;
    let wanted = (-0.5 * 144.0 / (sigma * sigma)).exp();
    assert!((ratio - wanted).abs() < 0.05, "{ratio} {wanted}");
}

#[test]
fn each_channel_has_its_own_radius_ratio_and_scaling_once_unganged() {
    let only_red = blurred(BlurMode::Blur, |b| {
        b.linked.radius = false;
        b.radius.r = fixed(0.8);
    });
    let pixels = cross();
    let out = picture_of(&only_red, 0.0, (81, 41), &pixels);
    for (made, was) in out.iter().zip(&pixels) {
        assert_eq!(made[1], was[1]);
        assert_eq!(made[2], was[2]);
    }
    assert!(out[5 * 81 + 43][0] > 0.01);
    // Red blurred, green sharpened, blue blurred another way: one neighbourhood each.
    let three = blurred(BlurMode::Blur, |b| {
        b.radius.r = fixed(0.8);
        b.radius.g = fixed(0.2);
        b.radius.b = fixed(0.8);
        b.ratio.b = fixed(1.0);
    });
    assert_eq!(neighbourhoods(&three).len(), 3);
    let out = picture_of(&three, 0.0, (81, 41), &pixels);
    let each = |grade: Grade, channel: usize| {
        picture_of(&grade, 0.0, (81, 41), &pixels)
            .iter()
            .map(|p| p[channel])
            .collect::<Vec<_>>()
    };
    let channel = |c: usize| out.iter().map(|p| p[c]).collect::<Vec<_>>();
    assert_eq!(channel(0), each(blurred(BlurMode::Blur, radius(0.8)), 0));
    assert_eq!(channel(1), each(blurred(BlurMode::Blur, radius(0.2)), 1));
    let across = blurred(BlurMode::Blur, |b| {
        radius(0.8)(b);
        b.ratio.b = fixed(1.0);
    });
    assert_eq!(channel(2), each(across, 2));
    // Ganged alike: one; two alike and one other: two.
    assert_eq!(
        neighbourhoods(&blurred(BlurMode::Blur, radius(0.8))).len(),
        1
    );
    let two = blurred(BlurMode::Blur, |b| {
        radius(0.8)(b);
        b.radius.g = fixed(0.3);
    });
    assert_eq!(neighbourhoods(&two).len(), 2);
    // Mist: a wide one more for each ratio.
    let mist = blurred(BlurMode::Mist, |b| {
        radius(0.3)(b);
        b.mix = fixed(60.0);
    });
    assert_eq!(neighbourhoods(&mist).len(), 2);
}

#[test]
fn a_radius_keyframed_across_half_makes_one_code_and_changes_only_its_filters() {
    let keyed = blurred(BlurMode::Blur, |b| {
        b.radius.r = Param::Keys(vec![
            Key {
                frame: 0,
                value: 0.2,
                interp: Interp::Linear,
            },
            Key {
                frame: 10,
                value: 0.5,
                interp: Interp::Linear,
            },
            Key {
                frame: 20,
                value: 0.9,
                interp: Interp::Linear,
            },
        ]);
        b.radius.g = b.radius.r.clone();
        b.radius.b = b.radius.r.clone();
    });
    let prepared = Prepared::new(&keyed, &options(), &NoLuts);
    let at = |frame: f64| prepared.ops_at(frame).last().unwrap();
    let (sharp, none, blurry) = (at(0.0), at(10.0), at(20.0));
    assert_eq!(sharp.body, blurry.body);
    assert_eq!(sharp.neighbourhoods.len(), 1);
    assert_eq!(none.neighbourhoods.len(), 1);
    assert_ne!(
        sharp.neighbourhoods[0].filter,
        blurry.neighbourhoods[0].filter
    );
    // At 0.5 the picture is the node's, to the bit, though the neighbourhood is read.
    let pixels = edge();
    let plain = picture_of(&one(|_| {}), 0.0, (80, 40), &pixels);
    assert_eq!(picture_of(&keyed, 10.0, (80, 40), &pixels), plain);
    assert_ne!(picture_of(&keyed, 5.0, (80, 40), &pixels), plain);
    // Mix keyframed: one wide neighbourhood throughout, and none at 100.
    let mixed_keys = blurred(BlurMode::Mist, |b| {
        b.mix = Param::Keys(vec![
            Key {
                frame: 0,
                value: 100.0,
                interp: Interp::Linear,
            },
            Key {
                frame: 8,
                value: 40.0,
                interp: Interp::Linear,
            },
        ]);
    });
    assert_eq!(neighbourhoods(&mixed_keys).len(), 1);
    assert_eq!(picture_of(&mixed_keys, 0.0, (80, 40), &pixels), plain);
    assert_ne!(picture_of(&mixed_keys, 8.0, (80, 40), &pixels), plain);
}

#[allow(clippy::cast_precision_loss, reason = "places of pixels")]
#[test]
fn keys_and_windows_limit_it_and_a_window_brings_in_no_black_at_its_edge() {
    // A flat grey blurred is the same grey: inside a window, at its edge, and outside it, with
    // a softness, there is no dark ring of what is outside drawn in.
    let windowed = one(|c| {
        radius(1.0)(&mut c.blur);
        let mut window = Window::new("w", WindowShape::circular());
        window.transform.size = fixed(30.0);
        c.windows.push(window);
    });
    let grey = vec![[0.5_f32, 0.25, 0.125]; 64 * 48];
    for pixel in picture_of(&windowed, 0.0, (64, 48), &grey) {
        near(pixel, [0.5, 0.25, 0.125], 1e-6);
    }
    // A picture of stripes: blurred inside the window, as it was far outside it.
    let stripes: Vec<[f32; 3]> = (0..64 * 48)
        .map(|at| {
            let v = if (at % 64 / 2) % 2 == 0 { 0.25 } else { 0.75 };
            [v; 3]
        })
        .collect();
    let out = picture_of(&windowed, 0.0, (64, 48), &stripes);
    let centre = 24 * 64 + 32;
    assert!((out[centre][0] - 0.5).abs() < 0.05, "{:?}", out[centre]);
    assert_eq!(out[0], stripes[0]);
    // A qualifier keys it: only what is keyed is blurred.
    let qualified = one(|c| {
        radius(1.0)(&mut c.blur);
        let mut q = Qualifier::default();
        q.lum.low = fixed(60.0);
        c.qualifier = Some(q);
    });
    let out = picture_of(&qualified, 0.0, (64, 48), &stripes);
    for (made, was) in out.iter().zip(&stripes) {
        if was[0] < 0.5 {
            assert_eq!(made, was);
        } else {
            assert!(made[0] < 0.7, "{made:?}");
        }
    }
    // Key Output Gain 0: none of it.
    let gained = one(|c| {
        radius(1.0)(&mut c.blur);
        c.key.output.gain = fixed(0.0);
    });
    assert_eq!(picture_of(&gained, 0.0, (64, 48), &stripes), stripes);
}

#[test]
fn what_a_neighbourhood_of_the_palette_filters_is_what_its_node_made_before_its_key() {
    let grade = serial(vec![
        gain_node("a", [1.5, 1.0, 1.0]),
        corrector("b", |c| {
            c.primaries.midtone_detail = fixed(30.0);
            c.primaries.saturation = fixed(70.0);
            radius(0.7)(&mut c.blur);
            c.key.output.gain = fixed(0.5);
        }),
    ]);
    let hoods = neighbourhoods(&grade);
    assert_eq!(hoods.len(), 2);
    let body = &hoods[1].body;
    assert!(body.contains("o0"), "{body}");
    assert!(
        body.contains("grade_primary_detailed") && body.contains("$N0"),
        "{body}"
    );
    assert!(
        body.contains("grade_secondary") && !body.contains("k1"),
        "{body}"
    );
    let code = code(&grade);
    assert!(code.contains("grade_blur(x, vec3<f32>("), "{code}");
    // Its library is put once, before the grade's.
    let ops = grade_ops(&grade, 0.0, &options(), &NoLuts);
    assert_eq!(ops.len(), 2);
    assert!(ops[0].library.contains("fn grade_blur("));
}
