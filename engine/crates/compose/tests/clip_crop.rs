//! The crop of a clip and its composite mode, on the card. A crop cuts a layer in its own
//! picture, before its transform, and fades its cut edges inward; a layer without one, or with
//! one that cuts no side, is drawn as it was, to the bit. A composite mode blends a layer with
//! the canvas under it: held to the maths of the Layer Mixer of a grade on the processor
//! (`grade::maths::composite`), laid over what is under it as the W3C lays a blended colour over
//! a backdrop. Without an adapter the tests are skipped, unless `COLUMBINA_REQUIRE_GPU` is 1.

#![allow(clippy::too_many_lines, reason = "scenes, and what they are to show")]

use std::sync::Arc;

use columbina_compose::geometry::{fit, placement};
use columbina_compose::grade::{layout, maths, REC709_LUMA};
use columbina_compose::yuv::native_of;
use columbina_compose::{
    gpu_for_tests, ColorOp, Composite, Compositor, Crop, Gpu, LayerColor, Scene, SceneLayer,
    Source, Stage, Target, Text, Transform,
};
use columbina_media::{NativePicture, Picture};
use columbina_model::project::{Cropped, Rgba, TextAlign, TitleStyle};

/// The colour under the layers that are cut, and the colour of those layers.
const UNDER: [f32; 3] = [0.2, 0.5, 0.8];
const OVER: [f32; 3] = [0.9, 0.6, 0.1];

/// The most that a value of the card may be off `wanted`: 2 in 1000 of it, or of 1 where it is
/// less. The canvas is of half floats, which hold 11 bits, and a card may round them toward 0:
/// 1 in 1000 of a value from 1 to 2, and half that below 1. What is under a layer that is
/// blended is read back from the card as the canvas holds it, so that only the layer's own
/// arithmetic (in 32 bits, on both sides) and this one rounding are between the two.
fn most(wanted: f32) -> f32 {
    2e-3 * wanted.abs().max(1.0)
}

/// A frame read back from a preview: its half floats as they are, for what is to be the same
/// to the bit, and their values.
struct Frame {
    width: u32,
    height: u32,
    halves: Vec<u16>,
    values: Vec<f32>,
}

impl Frame {
    fn of(gpu: &Arc<Gpu>, compositor: &mut Compositor, scene: &Scene) -> Frame {
        let target = Target::preview(gpu, scene.width, scene.height).unwrap();
        compositor.render(scene, &target).unwrap();
        Frame {
            width: scene.width,
            height: scene.height,
            halves: target.read_halves().unwrap(),
            values: target.read_floats().unwrap(),
        }
    }

    /// The half floats of the pixel at (x, y): R, G, B and A.
    fn bits(&self, x: u32, y: u32) -> [u16; 4] {
        let at = (y * self.width + x) as usize * 4;
        self.halves[at..at + 4].try_into().unwrap()
    }

    /// The values of the pixel at (x, y).
    fn at(&self, x: u32, y: u32) -> [f32; 4] {
        let at = (y * self.width + x) as usize * 4;
        self.values[at..at + 4].try_into().unwrap()
    }
}

/// A layer of `source` as it is fitted, opaque, whole, laid as it is.
fn layer(source: Source) -> SceneLayer {
    SceneLayer {
        source,
        transform: Transform::NONE,
        opacity: 1.0,
        color: LayerColor::default(),
        crop: Crop::NONE,
        composite: Composite::Normal,
    }
}

/// A layer of an opaque colour over the whole canvas.
fn colour(rgb: [f32; 3]) -> SceneLayer {
    layer(Source::Color([rgb[0], rgb[1], rgb[2], 1.0]))
}

fn scene(size: (u32, u32), layers: Vec<SceneLayer>) -> Scene {
    Scene {
        width: size.0,
        height: size.1,
        layers,
        ..Scene::default()
    }
}

/// A stage that multiplies R, G and B by `gain`.
fn gain(gain: [f32; 3]) -> Stage {
    Stage {
        ops: vec![ColorOp {
            body: "rgb = rgb * param($P).rgb;".into(),
            params: vec![[gain[0], gain[1], gain[2], 0.0]],
            ..ColorOp::default()
        }],
    }
}

/// The codes of BT.709 of limited range, in 8 bits, of R'G'B' from 0 to 1.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "codes of 8 bits"
)]
fn codes(rgb: [f64; 3]) -> [u8; 3] {
    let (kr, kb) = (0.2126, 0.0722);
    let [r, g, b] = rgb;
    let y = kr * r + (1.0 - kr - kb) * g + kb * b;
    let cb = (b - y) / (2.0 * (1.0 - kb));
    let cr = (r - y) / (2.0 * (1.0 - kr));
    [16.0 + 219.0 * y, 128.0 + 224.0 * cb, 128.0 + 224.0 * cr].map(|code: f64| code.round() as u8)
}

/// A picture of 8 bits, 4:2:0, whose pixel at (x, y) has the luma that `luma` gives, and the
/// chroma of `rgb` everywhere.
fn picture(
    width: u32,
    height: u32,
    rgb: [f64; 3],
    luma: impl Fn(u32, u32, u8) -> u8,
) -> NativePicture {
    let [grey, cb, cr] = codes(rgb);
    let stride = (width as usize).next_multiple_of(64);
    let rows = height as usize;
    let mut plane = vec![0; stride * rows];
    for y in 0..height {
        for x in 0..width {
            plane[y as usize * stride + x as usize] = luma(x, y, grey);
        }
    }
    let chroma = (0..stride * rows.div_ceil(2))
        .map(|at| if at % 2 == 0 { cb } else { cr })
        .collect();
    native_of(Picture {
        frame: 0,
        width,
        height,
        bits: 8,
        luma: plane,
        chroma,
        luma_stride: stride,
        chroma_stride: stride,
    })
}

/// A picture of one colour.
fn flat(width: u32, height: u32, rgb: [f64; 3]) -> Arc<NativePicture> {
    Arc::new(picture(width, height, rgb, |_, _, grey| grey))
}

fn assert_no_errors(gpu: &Gpu) {
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

// ---------------------------------------------------------------------------------------------
// The crop
// ---------------------------------------------------------------------------------------------

/// How much of the point `at` of a canvas of `canvas` pixels the crop of a layer keeps, the
/// layer fitted into a frame of `frame` pixels and put on the canvas by `transform`: the point
/// is taken back into the frame, where the crop cuts its shares of the width and of the
/// height; from how far inside the nearest cut edge it is, across and down, it is kept from 0
/// to 1 over a pixel of the canvas, or over the softness (a share of the height of the frame),
/// whichever is more, beginning half a pixel of the canvas outside the cut; the two are
/// multiplied. A side that is not cut is no edge.
fn kept_at(
    canvas: (f64, f64),
    frame: (f64, f64),
    transform: &Transform,
    crop: &Crop,
    at: (f64, f64),
) -> f64 {
    let back = placement(canvas, frame, transform)
        .inverse()
        .expect("a layer of some size");
    let (across, down) = back.apply(at);
    // A pixel of the canvas, in pixels of the frame.
    let pixel = 1.0 / transform.scale;
    let width = (crop.softness * frame.1).max(pixel);
    let within = |place: f64, size: f64, low: f64, high: f64| {
        let mut nearest = f64::INFINITY;
        if low > 0.0 {
            nearest = nearest.min(place - low * size);
        }
        if high > 0.0 {
            nearest = nearest.min(size - high * size - place);
        }
        nearest
    };
    let ramp = |depth: f64| ((depth + 0.5 * pixel) / width).clamp(0.0, 1.0);
    ramp(within(across, frame.0, crop.left, crop.right))
        * ramp(within(down, frame.1, crop.top, crop.bottom))
}

/// The frames of a layer that is cut: the layer with its crop over [`UNDER`]; that colour
/// alone; and the layer without its crop over it.
struct Cut {
    got: Frame,
    under: Frame,
    whole: Frame,
}

/// What a check of a cut found: how many pixels are wholly cut away, wholly kept, and partly
/// kept.
#[derive(Debug)]
struct Found {
    cut: usize,
    kept: usize,
    partly: usize,
}

impl Cut {
    fn of(gpu: &Arc<Gpu>, compositor: &mut Compositor, size: (u32, u32), top: &SceneLayer) -> Cut {
        let whole = SceneLayer {
            crop: Crop::NONE,
            ..top.clone()
        };
        Cut {
            got: Frame::of(
                gpu,
                compositor,
                &scene(size, vec![colour(UNDER), top.clone()]),
            ),
            under: Frame::of(gpu, compositor, &scene(size, vec![colour(UNDER)])),
            whole: Frame::of(gpu, compositor, &scene(size, vec![colour(UNDER), whole])),
        }
    }

    /// Hold every pixel to `kept`, how much of a point of the canvas the crop keeps
    /// ([`kept_at`]): a pixel that it does not keep at all is what is under the layer, to the
    /// bit; one that it keeps whole is the layer without a crop, to the bit; and one between
    /// is that much of the one over the other. A pixel is taken as wholly cut or kept where the
    /// points a quarter of a pixel about its centre are too, so that what the card is off in
    /// where a pixel lies does not count.
    #[track_caller]
    #[allow(
        clippy::cast_possible_truncation,
        reason = "a share from 0 to 1, as the card has it"
    )]
    fn check(&self, kept: impl Fn((f64, f64)) -> f64, what: &str) -> Found {
        let mut found = Found {
            cut: 0,
            kept: 0,
            partly: 0,
        };
        for y in 0..self.got.height {
            for x in 0..self.got.width {
                let centre = (f64::from(x) + 0.5, f64::from(y) + 0.5);
                let share = kept(centre);
                let about = [(-0.25, -0.25), (0.25, -0.25), (-0.25, 0.25), (0.25, 0.25)]
                    .map(|(right, down)| kept((centre.0 + right, centre.1 + down)));
                if share <= 0.0 && about.iter().all(|each| *each <= 0.0) {
                    assert_eq!(
                        self.got.bits(x, y),
                        self.under.bits(x, y),
                        "{what}: ({x}, {y}) is cut away"
                    );
                    found.cut += 1;
                } else if share >= 1.0 && about.iter().all(|each| *each >= 1.0) {
                    assert_eq!(
                        self.got.bits(x, y),
                        self.whole.bits(x, y),
                        "{what}: ({x}, {y}) is kept"
                    );
                    found.kept += 1;
                } else {
                    let (got, under, whole) =
                        (self.got.at(x, y), self.under.at(x, y), self.whole.at(x, y));
                    for channel in 0..3 {
                        let wanted =
                            under[channel] + (whole[channel] - under[channel]) * share as f32;
                        assert!(
                            (got[channel] - wanted).abs() <= most(wanted),
                            "{what}: ({x}, {y}) is kept {share}: {got:?}, not {wanted} of \
                             {under:?} and {whole:?}"
                        );
                    }
                    found.partly += usize::from(share > 0.0 && share < 1.0);
                }
                assert_eq!(
                    self.got.bits(x, y)[3],
                    0x3C00,
                    "{what}: ({x}, {y}) is opaque"
                );
            }
        }
        found
    }
}

/// A scene of layers of every kind, moved, turned and partly opaque: a picture whose luma
/// differs from pixel to pixel, a colour that is half clear, and a title.
fn mixed(crop: Crop) -> Scene {
    let ramp = Arc::new(picture(48, 32, [0.5, 0.4, 0.6], |x, y, _| {
        u8::try_from(16 + (x * 5 + y * 3) % 200).unwrap()
    }));
    let layers = vec![
        colour(UNDER),
        SceneLayer {
            transform: Transform {
                x: 0.1,
                y: -0.05,
                scale: 0.8,
                rotation: 15.0,
            },
            opacity: 0.7,
            crop,
            ..layer(Source::Picture(ramp, 0))
        },
        SceneLayer {
            transform: Transform {
                x: -0.2,
                y: 0.1,
                scale: 0.5,
                rotation: -20.0,
            },
            opacity: 0.6,
            crop,
            ..layer(Source::Color([0.1, 0.3, 0.9, 0.5]))
        },
        SceneLayer {
            crop,
            ..title("Hh", 0.5)
        },
    ];
    scene((96, 48), layers)
}

/// A title of letters 32 pixels high, white, its middle at `x` of the width of the canvas and
/// half way down.
fn title(text: &str, x: f64) -> SceneLayer {
    layer(Source::Title(Text {
        text: text.into(),
        style: TitleStyle {
            font: String::new(),
            size: 32,
            color: Rgba([255, 255, 255, 255]),
            x,
            y: 0.5,
            align: TextAlign::Center,
        },
    }))
}

#[test]
fn a_crop_that_cuts_no_side_draws_the_frame_as_one_without_a_crop_to_the_bit() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let plain = Frame::of(&gpu, &mut compositor, &mixed(Crop::NONE));
    let made = compositor.pipelines_made();
    // The scene is of something: not one colour everywhere.
    assert!(plain
        .halves
        .chunks(4)
        .any(|pixel| pixel != &plain.halves[..4]));
    let crops = [
        // As the model has a clip that is not cropped.
        ("the model's", Crop::from(Cropped::NONE)),
        // A softness fades the edges that are cut, and none is.
        (
            "softness alone",
            Crop {
                softness: 1.0,
                ..Crop::NONE
            },
        ),
        // Shares of less than nothing, and that are not numbers, cut nothing.
        (
            "less than nothing",
            Crop {
                left: -0.5,
                right: f64::NAN,
                top: -0.0,
                bottom: f64::NEG_INFINITY,
                softness: 0.5,
            },
        ),
    ];
    for (what, crop) in crops {
        let frame = Frame::of(&gpu, &mut compositor, &mixed(crop));
        assert!(frame.halves == plain.halves, "{what}");
    }
    // And by the pipelines that drew the frame without a crop: none more is made.
    assert_eq!(compositor.pipelines_made(), made);
    assert_no_errors(&gpu);
}

#[test]
fn a_crop_of_each_side_cuts_those_pixels_and_leaves_the_rest() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let size = (64, 32);
    let canvas = (64.0, 32.0);
    // The crop, a pixel that it cuts away, and one that it keeps.
    let sides = [
        (
            "left",
            Crop {
                left: 0.25,
                ..Crop::NONE
            },
            (8, 16),
            (40, 16),
        ),
        (
            "right",
            Crop {
                right: 0.25,
                ..Crop::NONE
            },
            (56, 16),
            (24, 16),
        ),
        (
            "top",
            Crop {
                top: 0.25,
                ..Crop::NONE
            },
            (32, 3),
            (32, 20),
        ),
        (
            "bottom",
            Crop {
                bottom: 0.25,
                ..Crop::NONE
            },
            (32, 28),
            (32, 12),
        ),
        // Kept from 16 to 56 across and from 16 to 24 down.
        (
            "every side",
            Crop {
                left: 0.25,
                right: 0.125,
                top: 0.5,
                bottom: 0.25,
                softness: 0.0,
            },
            (10, 20),
            (30, 20),
        ),
    ];
    for (what, crop, away, within) in sides {
        let top = SceneLayer {
            crop,
            ..colour(OVER)
        };
        let frames = Cut::of(&gpu, &mut compositor, size, &top);
        let kept = |at| kept_at(canvas, canvas, &Transform::NONE, &crop, at);
        let found = frames.check(kept, what);
        // Cut on whole pixels: none is partly kept.
        assert!(
            found.cut > 0 && found.kept > 0 && found.partly == 0,
            "{what}: {found:?}"
        );
        assert_eq!(
            frames.got.bits(away.0, away.1),
            frames.under.bits(away.0, away.1),
            "{what}"
        );
        assert_eq!(
            frames.got.bits(within.0, within.1),
            frames.whole.bits(within.0, within.1),
            "{what}"
        );
        // The layer was there, and is the colour of the layer where it is kept.
        assert_ne!(
            frames.whole.bits(away.0, away.1),
            frames.under.bits(away.0, away.1),
            "{what}"
        );
    }
    // Cut half way through a column of pixels, 16.5 pixels from the left: that column is half
    // of each colour, and those beside it are of one.
    let between = Crop {
        left: 16.5 / 64.0,
        ..Crop::NONE
    };
    let top = SceneLayer {
        crop: between,
        ..colour(OVER)
    };
    let frames = Cut::of(&gpu, &mut compositor, size, &top);
    let kept = |at| kept_at(canvas, canvas, &Transform::NONE, &between, at);
    let found = frames.check(kept, "between pixels");
    assert_eq!(found.partly, 32, "{found:?}");
    let half = frames.got.at(16, 10);
    for channel in 0..3 {
        let wanted = f32::midpoint(UNDER[channel], OVER[channel]);
        assert!((half[channel] - wanted).abs() <= most(wanted), "{half:?}");
    }
    assert_eq!(frames.got.bits(14, 10), frames.under.bits(14, 10));
    assert_eq!(frames.got.bits(18, 10), frames.whole.bits(18, 10));
    // A crop that cuts all of the layer draws none of it.
    for crop in [
        Crop {
            left: 0.5,
            right: 0.5,
            ..Crop::NONE
        },
        Crop {
            bottom: 1.0,
            softness: 0.5,
            ..Crop::NONE
        },
    ] {
        let top = SceneLayer {
            crop,
            ..colour(OVER)
        };
        let frames = Cut::of(&gpu, &mut compositor, size, &top);
        assert!(frames.got.halves == frames.under.halves, "{crop:?}");
    }
    assert_no_errors(&gpu);
}

#[test]
fn a_crop_is_of_the_layer_before_its_transform_and_goes_where_the_layer_goes() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let size = (64, 32);
    let canvas = (64.0, 32.0);
    // Half as large, a quarter of the width to the right: over 32 to 64 across and 8 to 24
    // down. Its left half and its top quarter cut away: kept from 48 across and from 12 down.
    let moved = Transform {
        x: 0.25,
        y: 0.0,
        scale: 0.5,
        rotation: 0.0,
    };
    // Half as large, turned a quarter clockwise: over 24 to 40 across, all the way down. Its
    // left is now its top, and its top its right: kept from 16 down, and as far as 36 across.
    let turned = Transform {
        x: 0.0,
        y: 0.0,
        scale: 0.5,
        rotation: 90.0,
    };
    let sharp = Crop {
        left: 0.5,
        top: 0.25,
        ..Crop::NONE
    };
    // Each cut edge fading over a quarter of the height of the layer: 8 of its pixels, 4 of
    // the canvas's.
    let soft = Crop {
        softness: 0.25,
        ..sharp
    };
    // The transform, a pixel of the layer that is cut away, and one that is kept.
    let cases = [
        ("moved", moved, sharp, (40, 16), (56, 18)),
        ("turned", turned, sharp, (30, 8), (30, 24)),
        ("moved, soft", moved, soft, (40, 16), (60, 21)),
        ("turned, soft", turned, soft, (30, 8), (27, 28)),
    ];
    for (what, transform, crop, away, within) in cases {
        let top = SceneLayer {
            transform,
            crop,
            ..colour(OVER)
        };
        let frames = Cut::of(&gpu, &mut compositor, size, &top);
        let kept = |at| kept_at(canvas, canvas, &transform, &crop, at);
        let found = frames.check(kept, what);
        assert!(found.cut > 0 && found.kept > 0, "{what}: {found:?}");
        assert_eq!(
            frames.got.bits(away.0, away.1),
            frames.under.bits(away.0, away.1),
            "{what}"
        );
        assert_ne!(
            frames.whole.bits(away.0, away.1),
            frames.under.bits(away.0, away.1),
            "{what}: the layer was there"
        );
        assert_eq!(
            frames.got.bits(within.0, within.1),
            frames.whole.bits(within.0, within.1),
            "{what}"
        );
        assert_ne!(
            frames.got.bits(within.0, within.1),
            frames.under.bits(within.0, within.1),
            "{what}: the layer is there"
        );
    }
    assert_no_errors(&gpu);
}

#[test]
fn softness_fades_each_cut_edge_inward_over_its_share_of_the_height() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let size = (64, 32);
    let canvas = (64.0, 32.0);
    // Cut 16 pixels from the left, fading over a quarter of the height: 8 pixels.
    let crop = Crop {
        left: 0.25,
        softness: 0.25,
        ..Crop::NONE
    };
    let top = SceneLayer {
        crop,
        ..colour(OVER)
    };
    let frames = Cut::of(&gpu, &mut compositor, size, &top);
    let kept = |at| kept_at(canvas, canvas, &Transform::NONE, &crop, at);
    let found = frames.check(kept, "soft");
    // The columns from 16 to 22 are partly kept: an eighth more each, from an eighth.
    assert_eq!(found.partly, 7 * 32, "{found:?}");
    // Along a row: nothing of the layer at the cut, more of it at each pixel, and all of it
    // 8 pixels in. Red is less under the layer than in it.
    let red = |x: u32| frames.got.at(x, 16)[0];
    assert_eq!(frames.got.bits(14, 16), frames.under.bits(14, 16));
    for x in 15..23 {
        assert!(
            red(x) < red(x + 1),
            "{} at {x}, {} after",
            red(x),
            red(x + 1)
        );
    }
    for x in 24..64 {
        assert_eq!(frames.got.bits(x, 16), frames.whole.bits(x, 16), "{x}");
    }
    // Half way in, half of each.
    let wanted = f32::midpoint(UNDER[0], OVER[0]);
    assert!((red(19) - wanted).abs() <= most(wanted), "{}", red(19));
    // An edge that is not cut does not fade: the top, the bottom and the right of the layer
    // are as they are without a crop.
    for (x, y) in [(40, 0), (40, 31), (63, 16), (63, 0), (63, 31)] {
        assert_eq!(frames.got.bits(x, y), frames.whole.bits(x, y), "({x}, {y})");
    }
    assert_no_errors(&gpu);
}

#[test]
fn the_crop_of_a_picture_is_in_shares_of_the_picture_not_of_the_canvas() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let size = (64, 32);
    let canvas = (64.0, 32.0);
    // A square picture on a canvas twice as wide: fitted from 16 to 48 across.
    let square = flat(32, 32, [0.9, 0.6, 0.1]);
    let frame = fit(canvas, (32.0, 32.0));
    assert_eq!(frame, (32.0, 32.0));
    // A quarter of its width is 8 pixels: cut from 16 to 24. A quarter of the width of the
    // canvas would be 16, and cut as far as 32.
    let crop = Crop {
        left: 0.25,
        top: 0.5,
        ..Crop::NONE
    };
    let top = SceneLayer {
        crop,
        ..layer(Source::Picture(square, 0))
    };
    let frames = Cut::of(&gpu, &mut compositor, size, &top);
    let kept = |at| kept_at(canvas, frame, &Transform::NONE, &crop, at);
    let found = frames.check(kept, "a picture");
    assert!(found.cut > 0 && found.kept > 0, "{found:?}");
    // Within the picture, cut away; and kept, between 24 and 32 across.
    assert_eq!(frames.got.bits(20, 24), frames.under.bits(20, 24));
    assert_ne!(frames.whole.bits(20, 24), frames.under.bits(20, 24));
    assert_eq!(frames.got.bits(28, 24), frames.whole.bits(28, 24));
    assert_ne!(frames.got.bits(28, 24), frames.under.bits(28, 24));
    // Its top half is cut away, where it is kept across.
    assert_eq!(frames.got.bits(40, 8), frames.under.bits(40, 8));
    assert_ne!(frames.whole.bits(40, 8), frames.under.bits(40, 8));
    assert_no_errors(&gpu);
}

/// How many pixels of the columns from `from` up to `to` of `frame` are not those of `other`.
fn differing(frame: &Frame, other: &Frame, from: u32, to: u32) -> usize {
    let mut count = 0;
    for y in 0..frame.height {
        for x in from..to {
            count += usize::from(frame.bits(x, y) != other.bits(x, y));
        }
    }
    count
}

#[test]
fn the_crop_of_a_title_is_of_the_canvas_not_of_the_block_of_its_text() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let size = (256, 64);
    let canvas = (256.0, 64.0);
    // Two letters in the right half of the canvas.
    let letters = title("HH", 0.75);
    // The left half of the canvas cut away: all of the text is in the half that is kept, and
    // is drawn as it is without a crop. Were the crop of its block, half of it would be gone.
    let half = SceneLayer {
        crop: Crop {
            left: 0.5,
            ..Crop::NONE
        },
        ..letters.clone()
    };
    let frames = Cut::of(&gpu, &mut compositor, size, &half);
    let inked: Vec<u32> = (0..256)
        .filter(|x| differing(&frames.whole, &frames.under, *x, *x + 1) > 0)
        .collect();
    assert!(!inked.is_empty(), "no text is drawn");
    let (first, last) = (inked[0], inked[inked.len() - 1]);
    assert!(
        first > 130 && last < 254 && last - first > 16,
        "the text is drawn from {first} to {last}"
    );
    assert_eq!(frames.got.halves, frames.whole.halves);
    // Cut through the middle of the text, on a whole pixel of the canvas: what is left of
    // there is gone, and what is right of it is as it was.
    let middle = (first + last).div_ceil(2);
    let crop = Crop {
        left: f64::from(middle) / 256.0,
        ..Crop::NONE
    };
    let through = SceneLayer {
        crop,
        ..letters.clone()
    };
    let frames = Cut::of(&gpu, &mut compositor, size, &through);
    let kept = |at| kept_at(canvas, canvas, &Transform::NONE, &crop, at);
    let found = frames.check(kept, "a title");
    assert!(found.cut > 0 && found.kept > 0, "{found:?}");
    assert!(differing(&frames.whole, &frames.under, 0, middle - 1) > 16);
    assert!(differing(&frames.whole, &frames.under, middle + 1, 256) > 16);
    // But for the columns at the cut, which the card may take as a hair kept or cut.
    assert_eq!(differing(&frames.got, &frames.under, 0, middle - 1), 0);
    assert_eq!(differing(&frames.got, &frames.whole, middle + 1, 256), 0);
    assert_no_errors(&gpu);
}

// ---------------------------------------------------------------------------------------------
// The composite mode
// ---------------------------------------------------------------------------------------------

/// Every composite mode, in the order of their numbers.
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

/// A colour under and a colour over it: none of their channels add up to 1, where Hard Mix
/// steps, and their lumas are well apart, where Darker Color and Lighter Color choose; one is
/// under a grey, which has no hue and no saturation to keep.
const PAIRS: [([f32; 3], [f32; 3]); 4] = [
    ([0.75, 0.25, 0.5], [0.2, 0.6, 0.9]),
    ([0.1, 0.8, 0.3], [0.7, 0.35, 0.05]),
    ([0.9, 0.9, 0.2], [0.45, 0.3, 0.6]),
    ([0.5, 0.5, 0.5], [0.25, 0.25, 0.75]),
];

/// What the canvas holds once a layer of the colour `over` (not premultiplied), `alpha`
/// opaque, is laid in `mode` over `under`, what the canvas held (R'G'B' premultiplied, and its
/// alpha): as the W3C lays a blended colour over a backdrop (Compositing and Blending Level 1).
/// With `Cb` the colour under, not premultiplied, `Ab` its alpha, and `B` the mode
/// (`grade::maths::composite`, by the weights `luma` of luma), the colour of the layer is
/// `(1 - Ab) Cs + Ab B(Cb, Cs)`, and is laid over by its alpha. Over nothing, and in the Normal
/// mode, it is the colour of the layer. R'G'B', premultiplied.
fn laid(under: [f32; 4], over: [f32; 3], alpha: f32, mode: Composite, luma: [f32; 3]) -> [f32; 3] {
    let below = under[3];
    let colour = if mode == Composite::Normal || below <= 0.0 {
        over
    } else {
        let constants = maths::Constants {
            luma,
            s_curve: true,
        };
        let backdrop = [under[0] / below, under[1] / below, under[2] / below];
        let mixed = maths::composite(&constants, layout::composite(mode), backdrop, over);
        [0, 1, 2].map(|channel| (1.0 - below) * over[channel] + below * mixed[channel])
    };
    [0, 1, 2].map(|channel| colour[channel] * alpha + (1.0 - alpha) * under[channel])
}

/// The colour of a frame of 8 × 8 of `scene`, whose layers each cover it whole: that of its
/// pixels within a pixel of its edges, which are to be the same to the bit.
#[track_caller]
fn even(gpu: &Arc<Gpu>, compositor: &mut Compositor, scene: &Scene) -> [f32; 4] {
    let frame = Frame::of(gpu, compositor, scene);
    for y in 1..scene.height - 1 {
        for x in 1..scene.width - 1 {
            assert_eq!(frame.bits(x, y), frame.bits(4, 4), "({x}, {y})");
        }
    }
    frame.at(4, 4)
}

/// Check that a pixel is `wanted`, within [`most`], and opaque.
#[track_caller]
fn assert_near(got: [f32; 4], wanted: [f32; 3], what: &str) {
    assert_within(got, wanted, 1.0, what);
}

/// Check that a pixel is `wanted`, within `times` [`most`], and opaque: twice it for a colour
/// that went through half floats twice on its way (read back and then blended, or blended and
/// then graded on the canvas).
#[track_caller]
fn assert_within(got: [f32; 4], wanted: [f32; 3], times: f32, what: &str) {
    for channel in 0..3 {
        assert!(
            (got[channel] - wanted[channel]).abs() <= times * most(wanted[channel]),
            "{what}: {got:?}, not {wanted:?}"
        );
    }
    assert_eq!(got[3].to_bits(), 1.0_f32.to_bits(), "{what}: {got:?}");
}

/// Whether two colours are further apart than the card may be off, in some channel.
fn apart(a: [f32; 3], b: [f32; 3]) -> bool {
    (0..3).any(|channel| (a[channel] - b[channel]).abs() > 4.0 * most(a[channel]))
}

#[test]
fn the_modes_are_numbered_as_the_maths_of_a_grade_numbers_them() {
    for (number, mode) in MODES.into_iter().enumerate() {
        assert_eq!(layout::composite(mode) as usize, number, "{mode:?}");
    }
}

#[test]
fn each_mode_over_an_opaque_and_over_a_half_clear_canvas_is_that_of_the_processor() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let size = (8, 8);
    let mut changed = 0;
    for (under, over) in PAIRS {
        for below in [1.0_f32, 0.5] {
            let bottom = SceneLayer {
                opacity: below,
                ..colour(under)
            };
            // What the canvas holds under the layer, as the card has it: over nothing, so
            // premultiplied by `below`, which is its alpha there.
            let back = even(&gpu, &mut compositor, &scene(size, vec![bottom.clone()]));
            let held = [back[0], back[1], back[2], below];
            let normal = laid(held, over, 1.0, Composite::Normal, REC709_LUMA);
            for mode in MODES {
                let top = SceneLayer {
                    composite: mode,
                    ..colour(over)
                };
                let layers = vec![bottom.clone(), top];
                let got = even(&gpu, &mut compositor, &scene(size, layers));
                let wanted = laid(held, over, 1.0, mode, REC709_LUMA);
                let what = format!("{mode:?} of {over:?} over {under:?}, {below} opaque");
                assert_near(got, wanted, &what);
                changed += usize::from(apart(wanted, normal));
            }
        }
    }
    // The modes do something: nearly every one, over every colour, is not the Normal one.
    assert!(changed > 26 * 8 * 3 / 4, "{changed} of {}", 26 * 8);
    assert_no_errors(&gpu);
}

#[test]
fn opacity_and_the_alpha_of_a_layer_scale_what_its_mode_does() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let size = (8, 8);
    let (under, over) = PAIRS[0];
    let modes = [
        Composite::Multiply,
        Composite::Screen,
        Composite::Overlay,
        Composite::Difference,
        Composite::Hue,
        Composite::Subtract,
    ];
    let (mut cases, mut scaled) = (0_usize, 0_usize);
    for below in [1.0_f32, 0.5] {
        let bottom = SceneLayer {
            opacity: below,
            ..colour(under)
        };
        let back = even(&gpu, &mut compositor, &scene(size, vec![bottom.clone()]));
        let held = [back[0], back[1], back[2], below];
        for mode in modes {
            // Its opacity alone; and its opacity times the alpha of its colour.
            for (opacity, alpha) in [(0.75_f32, 1.0_f32), (0.25, 1.0), (0.5, 0.5)] {
                let top = SceneLayer {
                    opacity,
                    composite: mode,
                    ..layer(Source::Color([over[0], over[1], over[2], alpha]))
                };
                let layers = vec![bottom.clone(), top];
                let got = even(&gpu, &mut compositor, &scene(size, layers));
                let wanted = laid(held, over, opacity * alpha, mode, REC709_LUMA);
                let what = format!("{mode:?} at {opacity} × {alpha} over {below}");
                // Over a canvas that is not opaque, the frame is over black.
                assert_near(got, wanted, &what);
                // Not what it is when opaque, nor what is under it: but for a mode that gives
                // what is under it for these colours, which is the same at every opacity.
                let full = laid(held, over, 1.0, mode, REC709_LUMA);
                cases += 1;
                scaled +=
                    usize::from(apart(wanted, full) && apart(wanted, [back[0], back[1], back[2]]));
            }
        }
    }
    assert!(scaled > cases * 3 / 4, "{scaled} of {cases}");
    assert_no_errors(&gpu);
}

#[test]
fn a_mode_on_a_layer_with_nothing_under_it_is_the_normal_mode_to_the_bit() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let size = (64, 32);
    let blended = |mode: Composite, rgb: [f32; 3]| SceneLayer {
        composite: mode,
        ..colour(rgb)
    };
    // The lowest layer: drawn by the pipeline of a layer that is not blended, and no other.
    let normal = Frame::of(&gpu, &mut compositor, &scene(size, vec![colour(OVER)]));
    let made = compositor.pipelines_made();
    for mode in [Composite::Multiply, Composite::Screen, Composite::Hue] {
        let layers = vec![blended(mode, OVER)];
        let frame = Frame::of(&gpu, &mut compositor, &scene(size, layers));
        assert!(frame.halves == normal.halves, "{mode:?}");
    }
    assert_eq!(compositor.pipelines_made(), made);
    // The lowest layer that is drawn: one of no opacity under it is not.
    let unseen = SceneLayer {
        opacity: 0.0,
        ..colour(UNDER)
    };
    let layers = vec![unseen, blended(Composite::Multiply, OVER)];
    let frame = Frame::of(&gpu, &mut compositor, &scene(size, layers));
    assert_eq!(frame.halves, normal.halves);
    assert_eq!(compositor.pipelines_made(), made);

    // Over a layer that covers a part of the canvas: half as large, from 16 to 48 across and
    // 8 to 24 down. Beyond it nothing is under the layer, which is there as in the Normal mode,
    // to the bit; over it, the layer is blended.
    let small = SceneLayer {
        transform: Transform {
            scale: 0.5,
            ..Transform::NONE
        },
        ..colour(UNDER)
    };
    let back = Frame::of(&gpu, &mut compositor, &scene(size, vec![small.clone()]));
    for mode in [Composite::Multiply, Composite::Difference] {
        let layers = vec![small.clone(), blended(mode, OVER)];
        let frame = Frame::of(&gpu, &mut compositor, &scene(size, layers));
        for (x, y) in [(2, 2), (60, 28), (8, 16), (56, 16), (32, 3), (32, 28)] {
            assert_eq!(
                frame.bits(x, y),
                normal.bits(x, y),
                "{mode:?} at ({x}, {y})"
            );
        }
        for (x, y) in [(32, 16), (20, 10), (44, 20)] {
            let under = back.at(x, y);
            let held = [under[0], under[1], under[2], 1.0];
            let wanted = laid(held, OVER, 1.0, mode, REC709_LUMA);
            assert_near(frame.at(x, y), wanted, &format!("{mode:?} at ({x}, {y})"));
            assert!(apart(wanted, OVER), "{mode:?}");
        }
        // The frame is opaque everywhere, as one without such a layer is.
        assert!(frame.halves.chunks(4).all(|pixel| pixel[3] == 0x3C00));
    }
    // That made the pipeline of a layer of a colour that is blended, and no other.
    assert_eq!(compositor.pipelines_made(), made + 1);
    assert_no_errors(&gpu);
}

#[test]
fn a_frame_with_a_layer_that_is_blended_keeps_its_other_pixels_to_the_bit() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    // Over the scene of every kind of layer, a small layer in its middle: a quarter as
    // large, from 36 to 60 across and 18 to 30 down.
    let small = |mode: Composite| SceneLayer {
        transform: Transform {
            scale: 0.25,
            ..Transform::NONE
        },
        composite: mode,
        ..colour(OVER)
    };
    let with = |mode: Composite| {
        let mut whole = mixed(Crop::NONE);
        whole.layers.push(small(mode));
        whole
    };
    let normal = Frame::of(&gpu, &mut compositor, &with(Composite::Normal));
    let blended = Frame::of(&gpu, &mut compositor, &with(Composite::Screen));
    let mut changed = 0;
    for y in 0..48 {
        for x in 0..96 {
            // Two pixels clear of the small layer: what is there is drawn as it was, though
            // the canvas is cleared to nothing, copied, and made opaque for the layer.
            let near = (34..62).contains(&x) && (16..32).contains(&y);
            if !near {
                assert_eq!(blended.bits(x, y), normal.bits(x, y), "({x}, {y})");
            }
            changed += usize::from(blended.bits(x, y) != normal.bits(x, y));
            assert_eq!(blended.bits(x, y)[3], 0x3C00, "({x}, {y}) is opaque");
        }
    }
    // And the layer is blended where it is.
    assert!(changed > 24 * 12 / 2, "{changed} pixels");
    assert_no_errors(&gpu);
}

#[test]
fn a_layer_is_blended_with_all_the_layers_under_it() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let size = (8, 8);
    let (bottom, middle, top) = ([0.75, 0.25, 0.5], [0.1, 0.8, 0.3], [0.45, 0.3, 0.6]);
    let opaque = |pixel: [f32; 4]| [pixel[0], pixel[1], pixel[2], 1.0];
    // The modes of the middle layer, which is half opaque, and of the top one.
    let cases = [
        (Composite::Normal, Composite::Screen),
        (Composite::Multiply, Composite::Difference),
        (Composite::Overlay, Composite::Normal),
        (Composite::Luminosity, Composite::Softlight),
    ];
    for (second, third) in cases {
        let what = format!("{third:?} over {second:?}");
        let first = colour(bottom);
        let second_layer = SceneLayer {
            opacity: 0.5,
            composite: second,
            ..colour(middle)
        };
        let third_layer = SceneLayer {
            composite: third,
            ..colour(top)
        };
        // The middle one over the bottom one, as the card has that.
        let one = even(&gpu, &mut compositor, &scene(size, vec![first.clone()]));
        let layers = vec![first.clone(), second_layer.clone()];
        let two = even(&gpu, &mut compositor, &scene(size, layers));
        let wanted = laid(opaque(one), middle, 0.5, second, REC709_LUMA);
        assert_near(two, wanted, &what);
        // The top one over the two, as the card has them: not over the bottom one alone, nor
        // over the middle one alone.
        let layers = vec![first, second_layer, third_layer];
        let three = even(&gpu, &mut compositor, &scene(size, layers));
        let wanted = laid(opaque(two), top, 1.0, third, REC709_LUMA);
        assert_near(three, wanted, &what);
        if third != Composite::Normal {
            let over_bottom = laid(opaque(one), top, 1.0, third, REC709_LUMA);
            let over_middle = laid(
                [middle[0], middle[1], middle[2], 1.0],
                top,
                1.0,
                third,
                REC709_LUMA,
            );
            assert!(
                apart(wanted, over_bottom) && apart(wanted, over_middle),
                "{what}"
            );
        }
    }
    assert_no_errors(&gpu);
}

#[test]
fn a_layer_that_is_graded_is_blended_as_its_grade_leaves_it_and_under_the_timeline_grade() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let size = (8, 8);
    let (under, over) = ([0.75, 0.25, 0.5], [0.8, 0.8, 0.4]);
    let factors = [0.5, 0.25, 2.0];
    let graded = [0, 1, 2].map(|channel| over[channel] * factors[channel]);
    let back = even(&gpu, &mut compositor, &scene(size, vec![colour(under)]));
    let held = [back[0], back[1], back[2], 1.0];
    for mode in [Composite::Screen, Composite::Overlay, Composite::Color] {
        let top = SceneLayer {
            color: LayerColor {
                grade: gain(factors),
                ..LayerColor::default()
            },
            composite: mode,
            ..colour(over)
        };
        let layers = vec![colour(under), top];
        let got = even(&gpu, &mut compositor, &scene(size, layers.clone()));
        // Graded, then blended: not blended and then graded.
        let wanted = laid(held, graded, 1.0, mode, REC709_LUMA);
        assert_near(got, wanted, &format!("{mode:?}, graded"));
        let ungraded = laid(held, over, 1.0, mode, REC709_LUMA);
        let after = [0, 1, 2].map(|channel| ungraded[channel] * factors[channel]);
        assert!(apart(wanted, after), "{mode:?}");

        // The timeline grade is done over the canvas once the layers are blended on it.
        let timeline = [0.75, 0.5, 0.25];
        let whole = Scene {
            grade: gain(timeline),
            ..scene(size, layers)
        };
        let got = even(&gpu, &mut compositor, &whole);
        let wanted = [0, 1, 2].map(|channel| wanted[channel] * timeline[channel]);
        let what = format!("{mode:?}, under the timeline grade");
        assert_within(got, wanted, 2.0, &what);
    }
    assert_no_errors(&gpu);
}

#[test]
fn a_picture_is_blended_as_a_colour_is_and_a_crop_cuts_what_is_blended() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let size = (64, 32);
    let square = flat(32, 32, [0.6, 0.3, 0.2]);
    let top = |mode: Composite, crop: Crop| SceneLayer {
        composite: mode,
        crop,
        ..layer(Source::Picture(Arc::clone(&square), 0))
    };
    // The picture alone, as the card decodes it; and what is under it.
    let alone = Frame::of(
        &gpu,
        &mut compositor,
        &scene(size, vec![top(Composite::Normal, Crop::NONE)]),
    );
    let back = Frame::of(&gpu, &mut compositor, &scene(size, vec![colour(UNDER)]));
    // Its left half cut away: of the picture, which is fitted from 16 to 48 across, as far as
    // 32.
    let crop = Crop {
        left: 0.5,
        ..Crop::NONE
    };
    for mode in [Composite::Multiply, Composite::Screen] {
        let layers = vec![colour(UNDER), top(mode, crop)];
        let frame = Frame::of(&gpu, &mut compositor, &scene(size, layers));
        // Where the picture is kept, it is blended with the colour under it.
        for (x, y) in [(40, 16), (36, 4), (45, 28)] {
            let (under, over) = (back.at(x, y), alone.at(x, y));
            let held = [under[0], under[1], under[2], 1.0];
            let wanted = laid(held, [over[0], over[1], over[2]], 1.0, mode, REC709_LUMA);
            // The colour of the picture is read back too: through half floats twice.
            let what = format!("{mode:?} at ({x}, {y})");
            assert_within(frame.at(x, y), wanted, 2.0, &what);
            assert!(apart(wanted, [over[0], over[1], over[2]]), "{mode:?}");
        }
        // Where it is cut away, and beside it, the colour under it is as it was, to the bit.
        for (x, y) in [(20, 16), (28, 8), (4, 16), (60, 16)] {
            assert_eq!(frame.bits(x, y), back.bits(x, y), "{mode:?} at ({x}, {y})");
        }
        assert!(frame.halves.chunks(4).all(|pixel| pixel[3] == 0x3C00));
    }
    assert_no_errors(&gpu);
}

#[test]
fn the_modes_that_go_by_luma_take_the_luma_of_the_scene() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let size = (8, 8);
    // Weights of another working space than Rec.709.
    let luma = [0.3, 0.5, 0.2];
    let mut changed = 0;
    for (under, over) in PAIRS {
        let back = even(&gpu, &mut compositor, &scene(size, vec![colour(under)]));
        let held = [back[0], back[1], back[2], 1.0];
        for mode in [
            Composite::Hue,
            Composite::Saturation,
            Composite::Color,
            Composite::Luminosity,
        ] {
            let top = SceneLayer {
                composite: mode,
                ..colour(over)
            };
            let other = Scene {
                luma,
                ..scene(size, vec![colour(under), top])
            };
            let got = even(&gpu, &mut compositor, &other);
            let wanted = laid(held, over, 1.0, mode, luma);
            assert_near(got, wanted, &format!("{mode:?} of {over:?} over {under:?}"));
            changed += usize::from(apart(wanted, laid(held, over, 1.0, mode, REC709_LUMA)));
        }
    }
    // By the weights of Rec.709 they would be other colours.
    assert!(changed >= 8, "{changed} of 16");
    assert_no_errors(&gpu);
}
