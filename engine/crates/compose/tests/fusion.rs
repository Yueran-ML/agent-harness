//! The composition of a clip, drawn on the card: what comes out of its nodes takes the place of
//! the source of the clip, before the grade of the clip, its crop, its transform, its composite
//! mode and its opacity. The composition that a clip has at first draws the clip as it is, to
//! the bit; the shader of a Merge is held to the mathematics of the processor (`merge.rs`), and
//! that of the masks to theirs (`mask.rs`); an effect mask mixes what a node draws with the
//! picture of its first input; and a layer without a composition is drawn as it always was.
//! Without an adapter the tests are skipped, unless `COLUMBINA_REQUIRE_GPU` is 1.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use columbina_compose::grade::maths::Constants;
use columbina_compose::grade::REC709_LUMA;
use columbina_compose::mask::{self, Common, Shape};
use columbina_compose::merge::{merge_pixel, Settings};
use columbina_compose::{
    card_for_tests, gpu_for_tests, ColorOp, ColorRange, Composite, Compositor, Crop, Fusion, Gpu,
    LayerColor, Scene, SceneLayer, Source, Stage, Target, Text, Transform, YuvCoding, YuvMatrix,
};
use columbina_media::{Alpha, NativePicture, PictureFormat};
use columbina_model::fusion::{
    EllipseShape, FillMethod, Fused, FusedKind, FusedNode, Masked, Merged, Operator, PaintMode,
    PolygonShape, RectangleShape, Transformed, MOST_POINTS,
};
use columbina_model::project::{Rgba, Styled, TextAlign};

const WIDTH: u32 = 16;
const HEIGHT: u32 = 8;

/// A pixel of a preview over which nothing is drawn, in half floats: black, and opaque.
const NOTHING: [u16; 4] = [0, 0, 0, 0x3C00];

/// The words of R', G' and B' of the pixel at (x, y) of the picture of the tests: no two of its
/// pixels are of one colour.
fn words(x: u32, y: u32) -> [u16; 3] {
    [
        4096 + 3000 * x,
        8192 + 6000 * y,
        20000 + 1000 * ((x + y) % 5),
    ]
    .map(|word| u16::try_from(word).unwrap())
}

/// A picture of R'G'B'A of [`WIDTH`] × [`HEIGHT`], opaque, every pixel of which is another
/// colour ([`words`]).
fn pattern() -> Arc<NativePicture> {
    let format = PictureFormat::Rgba64;
    let mut luma = Vec::new();
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            for word in words(x, y).into_iter().chain([u16::MAX]) {
                luma.extend(word.to_le_bytes());
            }
        }
    }
    Arc::new(NativePicture {
        frame: 0,
        width: WIDTH,
        height: HEIGHT,
        format,
        bits: 16,
        matrix: YuvMatrix::Bt709,
        range: ColorRange::Full,
        chroma_location: 0,
        alpha: Alpha::Opaque,
        luma,
        chroma: Vec::new(),
        luma_stride: WIDTH as usize * format.pixel_bytes(),
        chroma_stride: 0,
        ..NativePicture::default()
    })
}

/// A picture of 4:2:0 in 8 bits of [`WIDTH`] × [`HEIGHT`], every sample of which has the codes
/// `pixel` of Y', Cb and Cr, of BT.709 in limited range.
fn flat(pixel: [u8; 3]) -> Arc<NativePicture> {
    let pixels = (WIDTH * HEIGHT) as usize;
    Arc::new(NativePicture {
        frame: 0,
        width: WIDTH,
        height: HEIGHT,
        format: PictureFormat::Nv12,
        bits: 8,
        matrix: YuvMatrix::Bt709,
        range: ColorRange::Limited,
        chroma_location: 1,
        alpha: Alpha::Opaque,
        luma: vec![pixel[0]; pixels],
        chroma: [pixel[1], pixel[2]].repeat(pixels / 4),
        luma_stride: WIDTH as usize,
        chroma_stride: WIDTH as usize,
        ..NativePicture::default()
    })
}

/// A layer of `source` as it is fitted, opaque, whole, laid as it is, its codes of BT.709 in
/// `range`.
fn layer(source: Source, range: ColorRange) -> SceneLayer {
    SceneLayer {
        source,
        transform: Transform::NONE,
        opacity: 1.0,
        color: LayerColor {
            coding: YuvCoding {
                matrix: YuvMatrix::Bt709,
                range,
            },
            ..LayerColor::default()
        },
        crop: Crop::NONE,
        composite: Composite::Normal,
    }
}

/// A layer of a colour, R', G', B' and how opaque it is.
fn colour(rgba: [f32; 4]) -> SceneLayer {
    layer(Source::Color(rgba), ColorRange::Full)
}

/// A scene of `width` × `height`, of `layers`.
fn scene_of(width: u32, height: u32, layers: Vec<SceneLayer>) -> Scene {
    Scene {
        width,
        height,
        layers,
        ..Scene::default()
    }
}

/// A scene of the size of the pictures of the tests, of `layers`.
fn scene(layers: Vec<SceneLayer>) -> Scene {
    scene_of(WIDTH, HEIGHT, layers)
}

/// The half floats of a preview of `scene`, as they are.
fn halves(gpu: &Arc<Gpu>, compositor: &mut Compositor, scene: &Scene) -> Vec<u16> {
    let target = Target::preview(gpu, scene.width, scene.height).unwrap();
    compositor.render(scene, &target).unwrap();
    target.read_halves().unwrap()
}

/// The values of a preview of `scene`: R, G, B and A of each pixel.
fn floats(gpu: &Arc<Gpu>, compositor: &mut Compositor, scene: &Scene) -> Vec<f32> {
    let target = Target::preview(gpu, scene.width, scene.height).unwrap();
    compositor.render(scene, &target).unwrap();
    target.read_floats().unwrap()
}

fn assert_no_errors(gpu: &Gpu) {
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

/// Check that each half float of `shown` is within `steps` of that of `wanted`: steps of a half
/// float, the values being of one sign.
#[track_caller]
fn assert_within(shown: &[u16], wanted: &[u16], steps: u16, what: &str) {
    assert_eq!(shown.len(), wanted.len(), "{what}");
    for (at, (shown, wanted)) in shown.iter().zip(wanted).enumerate() {
        assert!(
            shown.abs_diff(*wanted) <= steps,
            "{what}: value {at} is {shown:#06x}, not {wanted:#06x}"
        );
    }
}

/// What comes out of a composition of `nodes`, the `MediaOut` reading the one at `output`, of
/// a clip whose source is `source`: its pictures of `size` pixels.
fn fused_of(
    source: Source,
    nodes: Vec<FusedKind>,
    output: Option<usize>,
    size: (u32, u32),
) -> Source {
    let nodes = nodes
        .into_iter()
        .enumerate()
        .map(|(at, kind)| FusedNode {
            id: format!("n{at}"),
            kind,
        })
        .collect();
    Source::Fused(Box::new(Fusion {
        source,
        composition: Fused { nodes, output },
        size,
    }))
}

/// As [`fused_of`], its pictures of the size of the pictures of the tests.
fn fused(source: Source, nodes: Vec<FusedKind>, output: Option<usize>) -> Source {
    fused_of(source, nodes, output, (WIDTH, HEIGHT))
}

/// The composition that a clip has at first: a `MediaIn` that the `MediaOut` shows.
fn first(source: Source, size: (u32, u32)) -> Source {
    fused_of(source, vec![FusedKind::MediaIn], Some(0), size)
}

/// A Merge as it is at first.
fn merge() -> Merged {
    Merged {
        apply: Composite::Normal,
        operator: Operator::Over,
        subtractive_additive: 1.0,
        alpha_gain: 1.0,
        burn_in: 0.0,
        blend: 1.0,
        center: [0.5, 0.5],
        size: 1.0,
        angle: 0.0,
    }
}

/// A Transform as it is at first.
fn transform() -> Transformed {
    Transformed {
        center: [0.5, 0.5],
        size: 1.0,
        angle: 0.0,
        flip_horizontal: false,
        flip_vertical: false,
    }
}

/// A Transform of `input` that flips it left for right.
fn flip(input: usize) -> FusedKind {
    FusedKind::Transform {
        input: Some(input),
        mask: None,
        settings: Transformed {
            flip_horizontal: true,
            ..transform()
        },
    }
}

/// A `MediaIn` flipped left for right and flipped back: two passes that leave its picture as
/// it was, drawn through the textures of the composition.
fn there_and_back() -> (Vec<FusedKind>, Option<usize>) {
    (vec![FusedKind::MediaIn, flip(0), flip(1)], Some(2))
}

fn style(size: u32) -> Styled {
    Styled {
        font: String::new(),
        size: f64::from(size),
        color: Rgba([255, 200, 100, 255]),
        x: 0.5,
        y: 0.5,
        align: TextAlign::Center,
    }
}

fn title(size: u32) -> Source {
    Source::Title(Text {
        text: "Ab".into(),
        style: style(size),
    })
}

/// A source of a clip: what it is, the range of its codes, and the size of the scene that it
/// is drawn in.
type Kind = (&'static str, Source, ColorRange, (u32, u32));

/// The pixel of the picture that the pixel (x, y) of a frame reads, the picture being so many
/// pixels wide and high; none where it reads nothing.
type Reads = fn(i64, i64, i64, i64) -> Option<(i64, i64)>;

/// The sources of the kinds that a clip has: a picture, two pictures mixed, a title and a
/// colour.
fn sources() -> Vec<Kind> {
    let own = (WIDTH, HEIGHT);
    let mixed = Source::Between {
        first: flat([180, 100, 140]),
        second: flat([60, 150, 110]),
        weight: 0.25,
        rotation: 0,
    };
    vec![
        (
            "a picture",
            Source::Picture(pattern(), 0),
            ColorRange::Full,
            own,
        ),
        ("two pictures mixed", mixed, ColorRange::Limited, own),
        ("a title", title(24), ColorRange::Full, (96, 48)),
        (
            "a colour",
            Source::Color([0.8, 0.4, 0.2, 0.75]),
            ColorRange::Full,
            own,
        ),
    ]
}

#[test]
fn the_composition_that_a_clip_has_at_first_draws_the_clip_as_it_is_to_the_bit() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    for (what, source, range, size) in sources() {
        // As it is fitted; and moved, made smaller, turned, cut and half seen, over a colour.
        let placed = |source: Source| SceneLayer {
            transform: Transform {
                x: 0.125,
                y: -0.0625,
                scale: 0.75,
                rotation: 10.0,
            },
            opacity: 0.5,
            crop: Crop {
                left: 0.25,
                ..Crop::NONE
            },
            composite: Composite::Screen,
            ..layer(source, range)
        };
        let under = colour([0.2, 0.5, 0.3, 1.0]);
        for moved in [false, true] {
            let drawn = |source: Source| {
                let layers = if moved {
                    vec![under.clone(), placed(source)]
                } else {
                    vec![layer(source, range)]
                };
                scene_of(size.0, size.1, layers)
            };
            let wanted = halves(&gpu, &mut compositor, &drawn(source.clone()));
            let shown = halves(&gpu, &mut compositor, &drawn(first(source.clone(), size)));
            assert!(shown == wanted, "{what}, moved: {moved}");
            // The layer is drawn: the frame is not the black of one with nothing in it.
            assert!(
                wanted
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|pixel| *pixel != NOTHING),
                "{what}, moved: {moved}"
            );
        }
    }
    // Nothing is drawn into a texture for it.
    assert_eq!(compositor.fusion_passes(), 0);
    assert_no_errors(&gpu);
}

#[test]
fn a_source_drawn_through_the_textures_of_a_composition_is_the_source_to_a_step() {
    // The source is drawn alone into a texture, and that through two passes that leave it as
    // it was, and then laid on the canvas as the picture of a title is: each value goes
    // through one more half float than that of a layer drawn at once, and is within a step of
    // it.
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    for (what, source, range, size) in sources() {
        let drawn = |source: Source| scene_of(size.0, size.1, vec![layer(source, range)]);
        let wanted = halves(&gpu, &mut compositor, &drawn(source.clone()));
        assert!(
            wanted
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| *pixel != NOTHING),
            "{what}"
        );
        let passes = compositor.fusion_passes();
        let (nodes, output) = there_and_back();
        let through = fused_of(source.clone(), nodes, output, size);
        let shown = halves(&gpu, &mut compositor, &drawn(through));
        assert_within(&shown, &wanted, 1, what);
        // The source alone, and the two flips.
        assert_eq!(compositor.fusion_passes(), passes + 3, "{what}");
    }
    // A composition larger than the frame is drawn as large as it is seen: a picture in a
    // composition of twice its size, in a frame of its size, is drawn pixel for pixel.
    let picture = Source::Picture(pattern(), 0);
    let wanted = halves(
        &gpu,
        &mut compositor,
        &scene(vec![layer(picture.clone(), ColorRange::Full)]),
    );
    let (nodes, output) = there_and_back();
    let twice = fused_of(picture, nodes, output, (WIDTH * 2, HEIGHT * 2));
    let shown = halves(
        &gpu,
        &mut compositor,
        &scene(vec![layer(twice, ColorRange::Full)]),
    );
    assert_within(&shown, &wanted, 1, "a composition of twice the size");
    assert_no_errors(&gpu);
}

/// Pixels of every kind, premultiplied: nothing, an opaque one, ones seen through in part, one
/// beyond white and one with a channel below black. Their colours, not premultiplied, are not
/// at the steps of the composite modes (0.5 of a channel, a sum of 1 of two of them), where
/// the least difference of rounding would be another result.
const PIXELS: [[f32; 4]; 7] = [
    [0.0, 0.0, 0.0, 0.0],
    [0.45, 0.25, 0.125, 1.0],
    [0.3, 0.2, 0.1, 0.5],
    [0.06, 0.6, 0.15, 0.75],
    [0.09, 0.03, 0.06, 0.3],
    [4.0, 2.5, 8.0, 1.0],
    [-0.05, 0.02, 1.5, 0.25],
];

/// Every composite mode, which a Merge has as its Apply Modes.
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

/// The settings that the shader is held to the processor at: in the Normal mode, each of the
/// four sliders at its ends and half way, with each of the others; and every Apply Mode, as a
/// Merge is at first and with an Alpha Gain, a Burn In and a Blend between their ends.
fn grid() -> Vec<Settings> {
    let levels = [0.0, 0.5, 1.0];
    let mut grid = Vec::new();
    for subtractive_additive in levels {
        for alpha_gain in levels {
            for burn_in in levels {
                for blend in levels {
                    grid.push(Settings {
                        subtractive_additive,
                        alpha_gain,
                        burn_in,
                        blend,
                        ..Settings::default()
                    });
                }
            }
        }
    }
    for apply in MODES {
        for (alpha_gain, burn_in, blend) in [(1.0, 0.0, 1.0), (0.5, 0.25, 0.5)] {
            grid.push(Settings {
                apply,
                alpha_gain,
                burn_in,
                blend,
                ..Settings::default()
            });
        }
    }
    grid
}

#[test]
fn the_shader_of_a_merge_is_held_to_the_mathematics_of_the_processor() {
    // Every pixel of the kinds over every one, given to the card as floats of 32 bits: what
    // the card makes of each pair, which it keeps in a half float, is within a thousandth of
    // what the processor makes of it, or of its size where that is above 1: more than the
    // half a step of a half float that rounding it takes, and less than any error of a
    // formula.
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let side = PIXELS.len();
    let background: Vec<[f32; 4]> = (0..side * side).map(|at| PIXELS[at / side]).collect();
    let foreground: Vec<[f32; 4]> = (0..side * side).map(|at| PIXELS[at % side]).collect();
    let across = u32::try_from(side).unwrap();
    let target = Target::preview(&gpu, across, across).unwrap();
    let weights = Constants {
        luma: REC709_LUMA,
        s_curve: false,
    };
    for settings in grid() {
        compositor
            .merge_for_tests(&background, &foreground, &settings, REC709_LUMA, &target)
            .unwrap();
        let shown = target.read_floats().unwrap();
        assert_eq!(shown.len(), side * side * 4);
        for (at, shown) in shown.as_chunks::<4>().0.iter().enumerate() {
            let wanted = merge_pixel(&weights, background[at], foreground[at], &settings);
            for channel in 0..4 {
                let most = 1e-3 * wanted[channel].abs().max(1.0);
                assert!(
                    (shown[channel] - wanted[channel]).abs() <= most,
                    "{settings:?}: {:?} over {:?} is {shown:?}, not {wanted:?}",
                    foreground[at],
                    background[at]
                );
            }
        }
    }
    // The pictures are to be of the size of the target.
    let short = &background[1..];
    assert!(compositor
        .merge_for_tests(
            short,
            &foreground,
            &Settings::default(),
            REC709_LUMA,
            &target
        )
        .is_err());
    assert_no_errors(&gpu);
}

/// The pixel at (x, y) of half floats of a frame of the size of the pictures of the tests.
fn pixel(halves: &[u16], x: i64, y: i64) -> [u16; 4] {
    let at = usize::try_from((y * i64::from(WIDTH) + x) * 4).unwrap();
    [halves[at], halves[at + 1], halves[at + 2], halves[at + 3]]
}

/// A frame each pixel (x, y) of which is the pixel of `plain` that `reads` gives for it, or
/// `beyond` where it gives none or one that is not in the frame.
fn rearranged(
    plain: &[u16],
    beyond: [u16; 4],
    reads: impl Fn(i64, i64) -> Option<(i64, i64)>,
) -> Vec<u16> {
    let (width, height) = (i64::from(WIDTH), i64::from(HEIGHT));
    let mut frame = Vec::with_capacity(plain.len());
    for y in 0..height {
        for x in 0..width {
            let read =
                reads(x, y).filter(|(x, y)| (0..width).contains(x) && (0..height).contains(y));
            frame.extend(read.map_or(beyond, |(x, y)| pixel(plain, x, y)));
        }
    }
    frame
}

#[test]
fn a_background_covers_the_picture_with_its_colour_premultiplied() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    // The source of the clip is not read: the composition has no `MediaIn`.
    let background = |rgba| {
        fused(
            Source::Color([1.0, 0.0, 1.0, 1.0]),
            vec![FusedKind::Background { rgba, mask: None }],
            Some(0),
        )
    };
    let half = layer(background([0.2, 0.4, 0.6, 0.5]), ColorRange::Full);
    // Over the black of the frame: its colour times its alpha.
    let shown = floats(&gpu, &mut compositor, &scene(vec![half.clone()]));
    for (at, pixel) in shown.as_chunks::<4>().0.iter().enumerate() {
        for (channel, wanted) in [0.1, 0.2, 0.3, 1.0].into_iter().enumerate() {
            assert!(
                (pixel[channel] - wanted).abs() <= 1e-3,
                "pixel {at}: {pixel:?}"
            );
        }
    }
    // Over white, the white shows through by what its alpha leaves.
    let white = colour([1.0; 4]);
    let shown = floats(&gpu, &mut compositor, &scene(vec![white, half]));
    for (at, pixel) in shown.as_chunks::<4>().0.iter().enumerate() {
        for (channel, wanted) in [0.6, 0.7, 0.8, 1.0].into_iter().enumerate() {
            assert!(
                (pixel[channel] - wanted).abs() <= 1e-3,
                "pixel {at}: {pixel:?}"
            );
        }
    }
    // One pass, the clear of a texture, for each frame.
    assert_eq!(compositor.fusion_passes(), 2);
    // A background of which nothing is seen is no picture, and the clip shows nothing.
    let clear = layer(background([1.0, 1.0, 1.0, 0.0]), ColorRange::Full);
    let shown = halves(&gpu, &mut compositor, &scene(vec![clear]));
    assert_eq!(shown, NOTHING.repeat((WIDTH * HEIGHT) as usize));
    assert_eq!(compositor.fusion_passes(), 2);
    assert_no_errors(&gpu);
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "each way of moving a picture, one after another"
)]
fn a_transform_moves_flips_and_turns_its_picture_pixel_for_pixel() {
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let picture = Source::Picture(pattern(), 0);
    let plain = halves(
        &gpu,
        &mut compositor,
        &scene(vec![layer(picture.clone(), ColorRange::Full)]),
    );
    let (width, height) = (i64::from(WIDTH), i64::from(HEIGHT));
    // How the picture is put, and the pixel of it that each pixel (x, y) of the frame reads.
    let cases: [(&str, Transformed, Reads); 8] = [
        (
            "two pixels to the right and one up",
            Transformed {
                center: [0.5 + 2.0 / 16.0, 0.5 + 1.0 / 8.0],
                ..transform()
            },
            |x, y, _, _| Some((x - 2, y + 1)),
        ),
        (
            "flipped left for right",
            Transformed {
                flip_horizontal: true,
                ..transform()
            },
            |x, y, width, _| Some((width - 1 - x, y)),
        ),
        (
            "flipped top for bottom",
            Transformed {
                flip_vertical: true,
                ..transform()
            },
            |x, y, _, height| Some((x, height - 1 - y)),
        ),
        (
            "half a turn",
            Transformed {
                angle: 180.0,
                ..transform()
            },
            |x, y, width, height| Some((width - 1 - x, height - 1 - y)),
        ),
        // A quarter turn counter-clockwise about the middle, (8, 4): what was at the right
        // comes to the top, and what the picture does not cover is transparent.
        (
            "a quarter turn counter-clockwise",
            Transformed {
                angle: 90.0,
                ..transform()
            },
            |x, y, _, _| Some((11 - y, x - 4)),
        ),
        (
            "a quarter turn clockwise",
            Transformed {
                angle: -90.0,
                ..transform()
            },
            |x, y, _, _| Some((y + 4, 11 - x)),
        ),
        // A whole turn changes nothing, and a size of 0 leaves nothing.
        (
            "a whole turn",
            Transformed {
                angle: 360.0,
                ..transform()
            },
            |x, y, _, _| Some((x, y)),
        ),
        (
            "a size of 0",
            Transformed {
                size: 0.0,
                ..transform()
            },
            |_, _, _, _| None,
        ),
    ];
    for (what, settings, reads) in cases {
        let nodes = vec![
            FusedKind::MediaIn,
            FusedKind::Transform {
                input: Some(0),
                mask: None,
                settings,
            },
        ];
        let moved = layer(fused(picture.clone(), nodes, Some(1)), ColorRange::Full);
        let shown = halves(&gpu, &mut compositor, &scene(vec![moved]));
        let wanted = rearranged(&plain, NOTHING, |x, y| reads(x, y, width, height));
        assert_within(&shown, &wanted, 1, what);
    }
    assert_no_errors(&gpu);
}

#[test]
fn a_merge_lays_its_foreground_where_its_centre_puts_it_on_its_background() {
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let picture = Source::Picture(pattern(), 0);
    let plain = halves(
        &gpu,
        &mut compositor,
        &scene(vec![layer(picture.clone(), ColorRange::Full)]),
    );
    let rgba = [0.1, 0.2, 0.3, 1.0];
    let behind = fused(
        picture.clone(),
        vec![FusedKind::Background { rgba, mask: None }],
        Some(0),
    );
    let backdrop = halves(
        &gpu,
        &mut compositor,
        &scene(vec![layer(behind, ColorRange::Full)]),
    );
    let beyond = pixel(&backdrop, 0, 0);
    let over = |settings: Merged| {
        let nodes = vec![
            FusedKind::MediaIn,
            FusedKind::Background { rgba, mask: None },
            FusedKind::Merge {
                background: Some(1),
                foreground: Some(0),
                mask: None,
                settings,
            },
        ];
        layer(fused(picture.clone(), nodes, Some(2)), ColorRange::Full)
    };
    // A quarter of the width to the right: the background shows where the foreground left.
    let right = over(Merged {
        center: [0.75, 0.5],
        ..merge()
    });
    let shown = halves(&gpu, &mut compositor, &scene(vec![right]));
    let wanted = rearranged(&plain, beyond, |x, y| Some((x - 4, y)));
    assert_within(&shown, &wanted, 1, "a quarter to the right");
    // Half as large about the middle: the background all about it, and the foreground, each
    // pixel of it made of four of the picture, in the middle.
    let small = over(Merged {
        size: 0.5,
        ..merge()
    });
    let shown = halves(&gpu, &mut compositor, &scene(vec![small]));
    for (x, y) in [(0, 0), (15, 0), (0, 7), (15, 7), (2, 3), (13, 4), (8, 0)] {
        assert_eq!(pixel(&shown, x, y), beyond, "({x}, {y})");
    }
    for (x, y) in [(5, 3), (8, 4), (10, 4)] {
        assert_ne!(pixel(&shown, x, y), beyond, "({x}, {y})");
    }
    // With no foreground, and with a Blend of 0, it is its background.
    let nodes = vec![
        FusedKind::Background { rgba, mask: None },
        FusedKind::Merge {
            background: Some(0),
            foreground: None,
            mask: None,
            settings: merge(),
        },
    ];
    let alone = layer(fused(picture.clone(), nodes, Some(1)), ColorRange::Full);
    assert_eq!(halves(&gpu, &mut compositor, &scene(vec![alone])), backdrop);
    let unseen = over(Merged {
        blend: 0.0,
        ..merge()
    });
    assert_eq!(
        halves(&gpu, &mut compositor, &scene(vec![unseen])),
        backdrop
    );
    assert_no_errors(&gpu);
}

#[test]
fn a_graph_of_several_nodes_is_drawn_each_from_those_that_it_reads() {
    // The picture of the clip flipped and moved a quarter of the width to the right, over a
    // background: MediaIn into a Transform, that and a Background into a Merge.
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let picture = Source::Picture(pattern(), 0);
    let plain = halves(
        &gpu,
        &mut compositor,
        &scene(vec![layer(picture.clone(), ColorRange::Full)]),
    );
    let rgba = [0.3, 0.1, 0.2, 1.0];
    let behind = fused(
        picture.clone(),
        vec![FusedKind::Background { rgba, mask: None }],
        Some(0),
    );
    let backdrop = halves(
        &gpu,
        &mut compositor,
        &scene(vec![layer(behind, ColorRange::Full)]),
    );
    let nodes = vec![
        FusedKind::MediaIn,
        FusedKind::Transform {
            input: Some(0),
            mask: None,
            settings: Transformed {
                center: [0.75, 0.5],
                flip_horizontal: true,
                ..transform()
            },
        },
        FusedKind::Background { rgba, mask: None },
        FusedKind::Merge {
            background: Some(2),
            foreground: Some(1),
            mask: None,
            settings: merge(),
        },
    ];
    let graph = layer(fused(picture, nodes, Some(3)), ColorRange::Full);
    let passes = compositor.fusion_passes();
    let shown = halves(&gpu, &mut compositor, &scene(vec![graph]));
    // Flipped in its own frame, and then moved: the pixel at x reads the one at 19 - x.
    let wanted = rearranged(&plain, pixel(&backdrop, 0, 0), |x, y| Some((19 - x, y)));
    assert_within(&shown, &wanted, 1, "flipped and moved over a background");
    // The source alone, the flip, the background and the merge.
    assert_eq!(compositor.fusion_passes(), passes + 4);
    assert_no_errors(&gpu);
}

#[test]
fn an_input_that_reads_nothing_is_transparent_and_so_is_a_clip_whose_output_reads_nothing() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let picture = Source::Picture(pattern(), 0);
    let under = colour([0.2, 0.5, 0.3, 1.0]);
    let alone = halves(&gpu, &mut compositor, &scene(vec![under.clone()]));
    let nothing: [(&str, Vec<FusedKind>, Option<usize>); 4] = [
        (
            "an output that reads nothing",
            vec![FusedKind::MediaIn],
            None,
        ),
        ("no nodes", vec![], None),
        (
            "a transform of nothing",
            vec![FusedKind::Transform {
                input: None,
                mask: None,
                settings: Transformed {
                    angle: 30.0,
                    ..transform()
                },
            }],
            Some(0),
        ),
        (
            "a merge of nothing over nothing",
            vec![FusedKind::Merge {
                background: None,
                foreground: None,
                mask: None,
                settings: merge(),
            }],
            Some(0),
        ),
    ];
    for (what, nodes, output) in nothing {
        let clip = layer(fused(picture.clone(), nodes, output), ColorRange::Full);
        // The layer under it is seen whole, and alone the frame is black.
        let shown = halves(
            &gpu,
            &mut compositor,
            &scene(vec![under.clone(), clip.clone()]),
        );
        assert_eq!(shown, alone, "{what}");
        let shown = halves(&gpu, &mut compositor, &scene(vec![clip]));
        assert_eq!(shown, NOTHING.repeat((WIDTH * HEIGHT) as usize), "{what}");
    }
    assert_eq!(compositor.fusion_passes(), 0);
    // A merge with no background lays its foreground over nothing: with a Blend of a half,
    // half of the picture, and the layer under it shows through by the other half.
    let plain = floats(
        &gpu,
        &mut compositor,
        &scene(vec![layer(picture.clone(), ColorRange::Full)]),
    );
    let nodes = vec![
        FusedKind::MediaIn,
        FusedKind::Merge {
            background: None,
            foreground: Some(0),
            mask: None,
            settings: Merged {
                blend: 0.5,
                ..merge()
            },
        },
    ];
    let half = layer(fused(picture, nodes, Some(1)), ColorRange::Full);
    let shown = floats(&gpu, &mut compositor, &scene(vec![under, half]));
    for (at, (shown, plain)) in shown
        .as_chunks::<4>()
        .0
        .iter()
        .zip(plain.as_chunks::<4>().0)
        .enumerate()
    {
        for (channel, below) in [0.2, 0.5, 0.3].into_iter().enumerate() {
            let wanted = 0.5 * plain[channel] + 0.5 * below;
            assert!(
                (shown[channel] - wanted).abs() <= 2e-3,
                "pixel {at}: {shown:?}, not {wanted} of {plain:?}"
            );
        }
    }
    assert_no_errors(&gpu);
}

/// A stage that multiplies R, G and B by `by`.
fn gains(by: [f32; 3]) -> Stage {
    Stage {
        ops: vec![ColorOp {
            body: "rgb = rgb * param($P).rgb;".into(),
            params: vec![[by[0], by[1], by[2], 0.0]],
            ..ColorOp::default()
        }],
    }
}

#[test]
fn the_grade_the_crop_the_transform_and_the_composite_mode_are_done_after_the_composition() {
    // A composition that makes an opaque colour half transparent (a merge over nothing with a
    // Blend of a half) is, to all that is done to the clip after it, a colour that is half
    // transparent: graded, cut, moved, made smaller, half seen and blended alike, to two steps
    // of a half float (the texture of the composition keeps its picture in half floats, and
    // the layer divides it by its alpha to grade it).
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let under = colour([0.2, 0.5, 0.3, 1.0]);
    let halved = fused(
        Source::Color([0.8, 0.4, 0.2, 1.0]),
        vec![
            FusedKind::MediaIn,
            FusedKind::Merge {
                background: None,
                foreground: Some(0),
                mask: None,
                settings: Merged {
                    blend: 0.5,
                    ..merge()
                },
            },
        ],
        Some(1),
    );
    let by_hand = Source::Color([0.8, 0.4, 0.2, 0.5]);
    for composite in [Composite::Normal, Composite::Multiply, Composite::Screen] {
        let over = |source: Source| SceneLayer {
            transform: Transform {
                x: 0.1,
                y: -0.05,
                scale: 0.8,
                rotation: 0.0,
            },
            opacity: 0.5,
            color: LayerColor {
                grade: gains([0.5, 1.0, 0.75]),
                ..LayerColor::default()
            },
            crop: Crop {
                left: 0.25,
                ..Crop::NONE
            },
            composite,
            ..layer(source, ColorRange::Full)
        };
        let wanted = halves(
            &gpu,
            &mut compositor,
            &scene(vec![under.clone(), over(by_hand.clone())]),
        );
        let shown = halves(
            &gpu,
            &mut compositor,
            &scene(vec![under.clone(), over(halved.clone())]),
        );
        // The layer is seen: the frame is not the colour under it alone.
        let alone = halves(&gpu, &mut compositor, &scene(vec![under.clone()]));
        assert_ne!(wanted, alone, "{composite:?}");
        assert_within(&shown, &wanted, 2, &format!("{composite:?}"));
    }
    assert_no_errors(&gpu);
}

#[test]
fn a_layer_without_a_composition_draws_and_keeps_what_it_did() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let picture = Source::Picture(pattern(), 0);
    let under = colour([0.2, 0.5, 0.3, 1.0]);
    let plain = scene(vec![
        under.clone(),
        layer(picture.clone(), ColorRange::Full),
    ]);
    let wanted = halves(&gpu, &mut compositor, &plain);
    let pipelines = compositor.pipelines_made();
    // The composition that a clip has at first: no texture, no pass and no pipeline more.
    let as_first = scene(vec![
        under.clone(),
        layer(first(picture.clone(), (WIDTH, HEIGHT)), ColorRange::Full),
    ]);
    assert_eq!(halves(&gpu, &mut compositor, &as_first), wanted);
    assert_eq!(compositor.pipelines_made(), pipelines);
    assert_eq!(compositor.fusion_passes(), 0);
    assert_eq!(compositor.neighbourhood_bytes(), 0);
    // A composition that flips the picture: the source alone and the flip, into two textures
    // of half floats, which are kept and drawn into again at each frame; and one pipeline
    // more, that of the passes (the source is drawn alone by the pipeline of its layer, and
    // the picture laid by that of the colour).
    let nodes = vec![FusedKind::MediaIn, flip(0)];
    let flipped = scene(vec![
        under,
        layer(fused(picture, nodes, Some(1)), ColorRange::Full),
    ]);
    let once = halves(&gpu, &mut compositor, &flipped);
    assert_ne!(once, wanted);
    let kept = compositor.neighbourhood_bytes();
    assert_eq!(kept, 2 * u64::from(WIDTH * HEIGHT) * 8);
    assert_eq!(compositor.pipelines_made(), pipelines + 1);
    for _ in 0..3 {
        assert_eq!(halves(&gpu, &mut compositor, &flipped), once);
    }
    assert_eq!(compositor.fusion_passes(), 8);
    assert_eq!(compositor.neighbourhood_bytes(), kept);
    assert_eq!(compositor.pipelines_made(), pipelines + 1);
    // And a layer without one after it is drawn as it was, with no pass.
    assert_eq!(halves(&gpu, &mut compositor, &plain), wanted);
    assert_eq!(compositor.fusion_passes(), 8);
    assert_no_errors(&gpu);
}

#[test]
fn a_composition_is_shown_as_its_source_until_the_pipeline_of_its_passes_is_made() {
    let Some(gpu) = gpu_for_tests() else { return };
    let picture = Source::Picture(pattern(), 0);
    let plain = scene(vec![layer(picture.clone(), ColorRange::Full)]);
    let nodes = vec![FusedKind::MediaIn, flip(0)];
    let flipped = scene(vec![layer(
        fused(picture, nodes, Some(1)),
        ColorRange::Full,
    )]);
    // A compositor that waits for its pipelines draws each frame as its scene says.
    let mut waiting = Compositor::new(Arc::clone(&gpu));
    let source = halves(&gpu, &mut waiting, &plain);
    let wanted = halves(&gpu, &mut waiting, &flipped);
    assert_ne!(source, wanted);
    assert!(!waiting.is_provisional());
    // One that does not shows the clip without its composition at first, and says so.
    let woken = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&woken);
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    compositor.compile_in_background(move || {
        count.fetch_add(1, Ordering::SeqCst);
    });
    assert_eq!(halves(&gpu, &mut compositor, &flipped), source);
    assert!(compositor.is_provisional());
    assert_eq!(compositor.fusion_passes(), 0);
    // Once the pipeline is made, it is woken (by the thread that made it, a moment after),
    // and draws the frame as it is.
    compositor.finish_compiling();
    let until = Instant::now() + Duration::from_secs(5);
    while woken.load(Ordering::SeqCst) == 0 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(woken.load(Ordering::SeqCst) >= 1);
    assert_eq!(halves(&gpu, &mut compositor, &flipped), wanted);
    assert!(!compositor.is_provisional());
    assert_eq!(compositor.fusion_passes(), 2);
    // A layer without a composition is not provisional for it.
    assert_eq!(halves(&gpu, &mut compositor, &plain), source);
    assert!(!compositor.is_provisional());
    assert_no_errors(&gpu);
}

#[test]
fn the_nodes_that_are_not_drawn_are_told_of() {
    // Every kind that the model has is drawn, the masks among them: none is told of.
    let (mut nodes, output) = there_and_back();
    nodes.extend([
        FusedKind::Rectangle {
            mask: None,
            common: masked(),
            shape: rectangle_shape(),
        },
        FusedKind::Ellipse {
            mask: Some(3),
            common: masked(),
            shape: ellipse_shape(),
        },
        FusedKind::Polygon {
            mask: Some(4),
            common: masked(),
            shape: star(FillMethod::Alternate),
        },
        FusedKind::Background {
            rgba: [0.0, 0.0, 0.0, 1.0],
            mask: Some(5),
        },
    ]);
    let Source::Fused(fusion) = fused(Source::Color([1.0; 4]), nodes, output) else {
        panic!("a composition");
    };
    assert_eq!(fusion.unknown(), [] as [&str; 0]);
    // What a `MediaIn` shows is the source of the clip, whatever is made of it.
    let source = Source::Fused(fusion);
    assert!(matches!(source.own(), Source::Color(_)));
    assert!(matches!(Source::Color([0.0; 4]).own(), Source::Color(_)));
}

// ---------------------------------------------------------------------------------------------
// The masks
// ---------------------------------------------------------------------------------------------

/// The size of the frames that the masks are drawn in: not square, so that a share of the
/// width and a share of the height are not the same pixels.
const MASK: (u32, u32) = (48, 32);

/// How far what the card makes of a mask may be from what the processor makes of it
/// (`mask.rs`): a thousandth, as of a Merge. A value goes through two half floats, that of the
/// picture of the composition and that of the frame, each within a quarter of a thousandth of
/// it; and the distances that the two work out differ by the rounding of a few floats, which
/// an edge as soft as these makes far less than that.
const MASK_TOLERANCE: f32 = 1e-3;

/// How near the crisp edge of a mask, in pixels, the middle of a pixel is not held to the
/// processor: there the mask steps from 0 to 1, and the least difference of rounding is the
/// other value.
const ON_THE_EDGE: f32 = 1e-2;

/// How far a picture limited by an effect mask may be from the formula worked out of the
/// frames of its parts, each of which went through its own half floats.
const LIMITED_TOLERANCE: f32 = 3e-3;

/// The controls of a mask as they are at first.
fn masked() -> Masked {
    Masked {
        level: 1.0,
        soft_edge: 0.0,
        border_width: 0.0,
        invert: false,
        solid: true,
        paint_mode: PaintMode::Merge,
    }
}

/// The shape of a Rectangle as it is at first.
fn rectangle_shape() -> RectangleShape {
    RectangleShape {
        center: [0.5, 0.5],
        width: 0.5,
        height: 0.5,
        corner_radius: 0.0,
        angle: 0.0,
    }
}

/// The shape of an Ellipse as it is at first.
fn ellipse_shape() -> EllipseShape {
    EllipseShape {
        center: [0.5, 0.5],
        width: 0.5,
        height: 0.5,
        angle: 0.0,
    }
}

/// A polygon of `points` about the middle of the picture, as they are drawn.
fn polygon_shape(points: Vec<[f64; 2]>, fill: FillMethod) -> PolygonShape {
    PolygonShape {
        center: [0.5, 0.5],
        size: 1.0,
        angle: 0.0,
        points,
        fill,
    }
}

/// A star of five points, each joined to the next but one, so that it lies over itself in
/// its middle.
fn star(fill: FillMethod) -> PolygonShape {
    let points = (0..5)
        .map(|point| {
            let angle = (90.0 + 144.0 * f64::from(point)).to_radians();
            [0.25 * angle.cos(), 0.25 * angle.sin()]
        })
        .collect();
    polygon_shape(points, fill)
}

/// A Rectangle mask into which no mask comes.
fn rectangle(common: Masked, shape: RectangleShape) -> FusedKind {
    FusedKind::Rectangle {
        mask: None,
        common,
        shape,
    }
}

/// An Ellipse mask into which no mask comes.
fn ellipse(common: Masked, shape: EllipseShape) -> FusedKind {
    FusedKind::Ellipse {
        mask: None,
        common,
        shape,
    }
}

/// A Polygon mask into which no mask comes.
fn polygon(common: Masked, shape: PolygonShape) -> FusedKind {
    FusedKind::Polygon {
        mask: None,
        common,
        shape,
    }
}

/// `kind` with the node at `mask` linked into its `mask` input; none: with nothing there.
fn with_mask(kind: &FusedKind, mask: Option<usize>) -> FusedKind {
    let mut kind = kind.clone();
    if let FusedKind::Background { mask: own, .. }
    | FusedKind::Merge { mask: own, .. }
    | FusedKind::Transform { mask: own, .. }
    | FusedKind::Rectangle { mask: own, .. }
    | FusedKind::Ellipse { mask: own, .. }
    | FusedKind::Polygon { mask: own, .. } = &mut kind
    {
        *own = mask;
    }
    kind
}

/// A clip whose composition is of `nodes`, the `MediaOut` reading the one at `output`, its
/// pictures of `size` pixels: for the compositions that do not read the source of the clip.
fn clip_of(nodes: Vec<FusedKind>, output: usize, size: (u32, u32)) -> Source {
    fused_of(
        Source::Color([1.0, 0.0, 1.0, 1.0]),
        nodes,
        Some(output),
        size,
    )
}

/// The values of a preview of a frame of `size` pixels that shows `clip` as it is fitted, over
/// a layer of the colour `under` where there is one, and else over the black of the frame: R,
/// G, B and A of each pixel.
fn frame_of(
    gpu: &Arc<Gpu>,
    compositor: &mut Compositor,
    under: Option<[f32; 4]>,
    clip: Source,
    size: (u32, u32),
) -> Vec<f32> {
    let layers = under
        .map(colour)
        .into_iter()
        .chain([layer(clip, ColorRange::Full)])
        .collect();
    floats(gpu, compositor, &scene_of(size.0, size.1, layers))
}

/// The shape and the controls of the mask node `kind`, as the mathematics takes them.
fn maths_of(kind: &FusedKind) -> (Shape, Common) {
    mask::of_node(kind).expect("a mask node")
}

/// The middle of the pixel (x, y) of a frame, in pixels from its top left corner.
fn middle(x: u32, y: u32) -> [f32; 2] {
    [x, y].map(|at| f32::from(u16::try_from(at).unwrap()) + 0.5)
}

/// Whether the middle of each pixel of a frame of `size` pixels, row after row, is on the
/// crisp edge of the mask of `shape` and `common` ([`ON_THE_EDGE`]): never, of a soft edge.
fn on_the_edge(size: (u32, u32), shape: &Shape, common: &Common) -> Vec<bool> {
    let placed = shape.placed(size.0, size.1);
    let edge = common.edge(size.0);
    let mut near = Vec::new();
    for y in 0..size.1 {
        for x in 0..size.0 {
            let away = mask::drawn(placed.distance(middle(x, y)), &edge);
            near.push(edge.soft <= 0.0 && away.abs() < ON_THE_EDGE);
        }
    }
    near
}

/// Check that R, G and B of each pixel of the frame `shown`, which is over black, are the
/// value of the mask `wanted` there, within [`MASK_TOLERANCE`]: but for the pixels of
/// `unsure`.
#[track_caller]
fn assert_mask(shown: &[f32], wanted: &[f32], unsure: &[bool], what: &str) {
    assert_eq!(shown.len(), wanted.len() * 4, "{what}");
    assert_eq!(unsure.len(), wanted.len(), "{what}");
    for (at, (pixel, wanted)) in shown.as_chunks::<4>().0.iter().zip(wanted).enumerate() {
        if unsure[at] {
            continue;
        }
        for value in &pixel[..3] {
            assert!(
                (value - wanted).abs() <= MASK_TOLERANCE,
                "{what}: pixel {at} is {pixel:?}, not {wanted}"
            );
        }
    }
}

/// The masks that the shader is held to the processor at: each shape with a crisp edge and
/// with soft ones, with a border outward and inward, as an outline, inverted and at a level,
/// turned, with rounded corners; and polygons of both Fill Methods.
#[allow(clippy::too_many_lines, reason = "each mask, one after another")]
fn shapes() -> Vec<(&'static str, FusedKind)> {
    let turned = RectangleShape {
        center: [0.4, 0.6],
        width: 0.6,
        height: 0.5,
        corner_radius: 0.5,
        angle: 30.0,
    };
    let long = EllipseShape {
        center: [0.55, 0.45],
        width: 0.7,
        height: 0.3,
        angle: -40.0,
    };
    let thin = EllipseShape {
        width: 0.9,
        height: 0.08,
        angle: 15.0,
        ..ellipse_shape()
    };
    let triangle = vec![[-0.3, -0.2], [0.35, -0.1], [-0.05, 0.25]];
    let moved = PolygonShape {
        center: [0.45, 0.55],
        size: 1.2,
        angle: 25.0,
        points: triangle.clone(),
        fill: FillMethod::Alternate,
    };
    // As many points as a Polygon may have, on a circle.
    let most = u32::try_from(MOST_POINTS).unwrap();
    let round = (0..most)
        .map(|point| {
            let angle = std::f64::consts::TAU * f64::from(point) / f64::from(most);
            [0.3 * angle.cos(), 0.3 * angle.sin()]
        })
        .collect();
    let soft = |soft_edge: f64| Masked {
        soft_edge,
        ..masked()
    };
    vec![
        (
            "a rectangle as it is at first",
            rectangle(masked(), rectangle_shape()),
        ),
        (
            "a rectangle with a soft edge",
            rectangle(soft(0.125), rectangle_shape()),
        ),
        (
            "a rectangle turned, its corners rounded",
            rectangle(masked(), turned),
        ),
        (
            "a rectangle turned, its corners rounded, with a soft edge",
            rectangle(soft(0.05), turned),
        ),
        (
            "a rectangle with a border outward",
            rectangle(
                Masked {
                    border_width: 0.1,
                    ..masked()
                },
                turned,
            ),
        ),
        (
            "a rectangle with a border inward and a soft edge",
            rectangle(
                Masked {
                    border_width: -0.1,
                    ..soft(0.04)
                },
                rectangle_shape(),
            ),
        ),
        (
            "the outline of a rectangle",
            rectangle(
                Masked {
                    solid: false,
                    border_width: 0.08,
                    ..soft(0.03)
                },
                turned,
            ),
        ),
        (
            "the crisp outline of a rectangle, its border below 0",
            rectangle(
                Masked {
                    solid: false,
                    border_width: -0.09,
                    ..masked()
                },
                rectangle_shape(),
            ),
        ),
        (
            "a rectangle inverted, at a level",
            rectangle(
                Masked {
                    invert: true,
                    level: 0.4,
                    ..soft(0.06)
                },
                turned,
            ),
        ),
        (
            "an ellipse as it is at first, a circle",
            ellipse(masked(), ellipse_shape()),
        ),
        ("an ellipse turned", ellipse(masked(), long)),
        (
            "an ellipse turned, with a soft edge",
            ellipse(soft(0.1), long),
        ),
        ("a long thin ellipse", ellipse(soft(0.03), thin)),
        (
            "the outline of an ellipse",
            ellipse(
                Masked {
                    solid: false,
                    border_width: 0.06,
                    ..soft(0.03)
                },
                long,
            ),
        ),
        (
            "an ellipse inverted, at a level, its border inward",
            ellipse(
                Masked {
                    invert: true,
                    level: 0.75,
                    border_width: -0.05,
                    ..soft(0.05)
                },
                long,
            ),
        ),
        (
            "a triangle",
            polygon(
                masked(),
                polygon_shape(triangle, FillMethod::NonZeroWinding),
            ),
        ),
        (
            "a triangle moved, made larger and turned, inverted",
            polygon(
                Masked {
                    invert: true,
                    level: 0.9,
                    ..soft(0.05)
                },
                moved,
            ),
        ),
        (
            "a star that lies over itself, by the alternate fill",
            polygon(masked(), star(FillMethod::Alternate)),
        ),
        (
            "a star that lies over itself, by the alternate fill, with a soft edge",
            polygon(soft(0.03), star(FillMethod::Alternate)),
        ),
        (
            "a star that lies over itself, by the non zero winding fill",
            polygon(masked(), star(FillMethod::NonZeroWinding)),
        ),
        (
            "the outline of a star, by the non zero winding fill",
            polygon(
                Masked {
                    solid: false,
                    border_width: 0.04,
                    ..soft(0.02)
                },
                star(FillMethod::NonZeroWinding),
            ),
        ),
        (
            "a polygon of as many points as it may have",
            polygon(soft(0.02), polygon_shape(round, FillMethod::NonZeroWinding)),
        ),
    ]
}

#[test]
fn the_shader_of_the_masks_is_held_to_the_mathematics_of_the_processor() {
    // The picture of each mask, of a frame that is not square: white premultiplied by what
    // the processor makes of the mask at each pixel, to a thousandth. The middle of a pixel
    // that is on a crisp edge is not held to it.
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let pixels = (MASK.0 * MASK.1) as usize;
    let mut drawn = 0;
    for (what, kind) in shapes() {
        let (shape, common) = maths_of(&kind);
        let wanted = mask::mask(MASK.0, MASK.1, &shape, &common, None);
        let unsure = on_the_edge(MASK, &shape, &common);
        // Nearly every pixel is held to the processor, and the mask is not one value all over.
        let skipped = unsure.iter().filter(|unsure| **unsure).count();
        assert!(
            skipped * 20 <= pixels,
            "{what}: {skipped} pixels on the edge"
        );
        let least = wanted.iter().copied().fold(f32::INFINITY, f32::min);
        let most = wanted.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        assert!(most - least > 0.1, "{what}: from {least} to {most}");
        let clip = clip_of(vec![kind], 0, MASK);
        let shown = frame_of(&gpu, &mut compositor, None, clip, MASK);
        assert_mask(&shown, &wanted, &unsure, what);
        drawn += 1;
    }
    // A pass each, and no source drawn alone.
    assert_eq!(compositor.fusion_passes(), drawn);
    assert_no_errors(&gpu);
}

#[test]
fn a_polygon_that_lies_over_itself_has_a_hole_by_the_alternate_fill_alone() {
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let pixels = (MASK.0 * MASK.1) as usize;
    let sure = vec![false; pixels];
    // The pixel in the middle of the frame is in the middle of the star, where its sides go
    // twice around.
    let centre = ((MASK.1 / 2) * MASK.0 + MASK.0 / 2) as usize * 4;
    for (fill, wanted) in [
        (FillMethod::Alternate, 0.0),
        (FillMethod::NonZeroWinding, 1.0),
    ] {
        let clip = clip_of(vec![polygon(masked(), star(fill))], 0, MASK);
        let shown = frame_of(&gpu, &mut compositor, None, clip, MASK);
        assert!(
            (shown[centre] - wanted).abs() <= MASK_TOLERANCE,
            "{fill:?}: {}",
            shown[centre]
        );
    }
    // Fewer than three points are no outline: a mask of nothing, and inverted the whole
    // picture at its level.
    for count in 0..3 {
        let points: Vec<[f64; 2]> = star(FillMethod::Alternate)
            .points
            .into_iter()
            .take(count)
            .collect();
        for (invert, level) in [(false, 0.0_f32), (true, 0.5)] {
            let common = Masked {
                invert,
                level: 0.5,
                soft_edge: 0.1,
                ..masked()
            };
            let kind = polygon(
                common,
                polygon_shape(points.clone(), FillMethod::NonZeroWinding),
            );
            let (shape, common) = maths_of(&kind);
            let wanted = mask::mask(MASK.0, MASK.1, &shape, &common, None);
            assert!(
                wanted.iter().all(|value| (value - level).abs() < 1e-6),
                "{count} points, inverted: {invert}"
            );
            let clip = clip_of(vec![kind], 0, MASK);
            let shown = frame_of(&gpu, &mut compositor, None, clip, MASK);
            let what = format!("{count} points, inverted: {invert}");
            assert_mask(&shown, &wanted, &sure, &what);
        }
    }
    assert_no_errors(&gpu);
}

#[test]
fn the_paint_mode_combines_a_mask_with_the_one_that_comes_into_it() {
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let pixels = (MASK.0 * MASK.1) as usize;
    let sure = vec![false; pixels];
    // A Rectangle to the left, and an Ellipse to the right that lies over a part of it, both
    // with soft edges and below their full level, so that no sum is held at 0 or at 1 all
    // over.
    let under = rectangle(
        Masked {
            soft_edge: 0.1,
            level: 0.8,
            ..masked()
        },
        RectangleShape {
            center: [0.4, 0.5],
            height: 0.6,
            ..rectangle_shape()
        },
    );
    let (shape, common) = maths_of(&under);
    let incoming = mask::mask(MASK.0, MASK.1, &shape, &common, None);
    let over = |paint_mode: PaintMode| {
        ellipse(
            Masked {
                soft_edge: 0.1,
                level: 0.7,
                paint_mode,
                ..masked()
            },
            EllipseShape {
                center: [0.6, 0.5],
                height: 0.4,
                ..ellipse_shape()
            },
        )
    };
    for mode in [PaintMode::Merge, PaintMode::Add, PaintMode::Subtract] {
        let (shape, common) = maths_of(&over(mode));
        let own = mask::mask(MASK.0, MASK.1, &shape, &common, None);
        let wanted: Vec<f32> = own
            .iter()
            .zip(&incoming)
            .map(|(own, incoming)| mask::painted(common.paint_mode, *own, Some(*incoming)))
            .collect();
        // Which is what the processor makes of the whole mask.
        let whole = mask::mask(MASK.0, MASK.1, &shape, &common, Some(&incoming));
        assert!(wanted == whole, "{mode:?}");
        let nodes = vec![under.clone(), with_mask(&over(mode), Some(0))];
        let shown = frame_of(&gpu, &mut compositor, None, clip_of(nodes, 1, MASK), MASK);
        assert_mask(&shown, &wanted, &sure, &format!("{mode:?}"));
        // With nothing coming in there is no Paint Mode: the mask is its own.
        let alone = clip_of(vec![over(mode)], 0, MASK);
        let shown = frame_of(&gpu, &mut compositor, None, alone, MASK);
        assert_mask(&shown, &own, &sure, &format!("{mode:?} of nothing"));
    }
    // The mask that comes in is the alpha of a picture, whatever made it: a Background that
    // is half seen, which the Ellipse is added to.
    let half = FusedKind::Background {
        rgba: [1.0, 0.0, 0.0, 0.5],
        mask: None,
    };
    let (shape, common) = maths_of(&over(PaintMode::Add));
    let alpha = vec![0.5; pixels];
    let wanted = mask::mask(MASK.0, MASK.1, &shape, &common, Some(&alpha));
    let nodes = vec![half, with_mask(&over(PaintMode::Add), Some(0))];
    let shown = frame_of(&gpu, &mut compositor, None, clip_of(nodes, 1, MASK), MASK);
    assert_mask(&shown, &wanted, &sure, "added to the alpha of a background");
    assert_no_errors(&gpu);
}

#[test]
fn a_mask_is_the_same_share_of_the_picture_at_whatever_size_it_is_drawn() {
    // The settings of a mask are shares of the picture, its Soft Edge and its Border Width
    // too: drawn at another size, the mask is what the processor makes of it for a frame of
    // that size.
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let kind = ellipse(
        Masked {
            solid: false,
            border_width: 0.1,
            soft_edge: 0.08,
            ..masked()
        },
        EllipseShape {
            center: [0.45, 0.55],
            width: 0.7,
            height: 0.35,
            angle: 25.0,
        },
    );
    let (shape, common) = maths_of(&kind);
    // The size of the composition, and that of the frame that shows it: the pictures are
    // drawn as large as they are seen, and never larger than the composition.
    let sizes: [((u32, u32), (u32, u32)); 4] = [
        ((48, 32), (48, 32)),
        ((96, 64), (48, 32)),
        ((48, 32), (24, 16)),
        ((96, 64), (96, 64)),
    ];
    for (nominal, seen) in sizes {
        let wanted = mask::mask(seen.0, seen.1, &shape, &common, None);
        let sure = vec![false; wanted.len()];
        let clip = clip_of(vec![kind.clone()], 0, nominal);
        let shown = frame_of(&gpu, &mut compositor, None, clip, seen);
        let what = format!("a composition of {nominal:?} in a frame of {seen:?}");
        assert_mask(&shown, &wanted, &sure, &what);
    }
    assert_no_errors(&gpu);
}

#[test]
fn the_picture_of_a_mask_is_white_premultiplied_by_it() {
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let soft = ellipse(
        Masked {
            soft_edge: 0.1,
            ..masked()
        },
        EllipseShape {
            center: [0.55, 0.45],
            width: 0.7,
            height: 0.3,
            angle: -40.0,
        },
    );
    let (shape, common) = maths_of(&soft);
    let wanted = mask::mask(MASK.0, MASK.1, &shape, &common, None);
    let sure = vec![false; wanted.len()];
    // Over black its R, G and B are the mask; and over white the frame is white all over,
    // as its alpha is the mask too.
    let clip = clip_of(vec![soft.clone()], 0, MASK);
    let shown = frame_of(&gpu, &mut compositor, None, clip.clone(), MASK);
    assert_mask(&shown, &wanted, &sure, "over black");
    let shown = frame_of(&gpu, &mut compositor, Some([1.0; 4]), clip, MASK);
    assert_mask(&shown, &vec![1.0; wanted.len()], &sure, "over white");
    // As the foreground of a Merge it is laid as any picture is: white, by the mask, over
    // the background.
    let rgba = [0.2, 0.4, 0.6, 1.0];
    let nodes = vec![
        FusedKind::Background { rgba, mask: None },
        soft.clone(),
        FusedKind::Merge {
            background: Some(0),
            foreground: Some(1),
            mask: None,
            settings: merge(),
        },
    ];
    let shown = frame_of(&gpu, &mut compositor, None, clip_of(nodes, 2, MASK), MASK);
    for (at, (pixel, mask)) in shown.as_chunks::<4>().0.iter().zip(&wanted).enumerate() {
        for (channel, behind) in [0.2_f32, 0.4, 0.6].into_iter().enumerate() {
            let laid = mask + behind * (1.0 - mask);
            assert!(
                (pixel[channel] - laid).abs() <= LIMITED_TOLERANCE,
                "pixel {at}: {pixel:?}, not {laid}"
            );
        }
    }
    // And over no background such a Merge is the picture of the mask itself.
    let nodes = vec![
        soft,
        FusedKind::Merge {
            background: None,
            foreground: Some(0),
            mask: None,
            settings: merge(),
        },
    ];
    let shown = frame_of(&gpu, &mut compositor, None, clip_of(nodes, 1, MASK), MASK);
    assert_mask(&shown, &wanted, &sure, "merged over nothing");
    assert_no_errors(&gpu);
}

/// The nodes that the compositions of the tests of effect masks begin with: a soft Ellipse
/// in the middle of the picture, which is the mask; the `MediaIn`; and an opaque Background.
fn limiting() -> Vec<FusedKind> {
    vec![
        ellipse(
            Masked {
                soft_edge: 0.125,
                ..masked()
            },
            EllipseShape {
                height: 0.25,
                ..ellipse_shape()
            },
        ),
        FusedKind::MediaIn,
        FusedKind::Background {
            rgba: [0.1, 0.2, 0.3, 1.0],
            mask: None,
        },
    ]
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "each kind of node that has an effect mask, one after another"
)]
fn an_effect_mask_mixes_what_a_node_draws_with_the_picture_of_its_first_input() {
    // Each node is drawn without its mask (`own`), with it, and so is the picture of its
    // first input alone (`first`; nothing, of a Background and of a Merge over no
    // background), over black and over white: the node with its mask is
    // `first + (own - first) * mask` of each of R, G and B of the frames, which over white
    // holds the alpha to the formula too, a frame being the picture laid over what is
    // under it.
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let picture = Source::Picture(pattern(), 0);
    let size = (WIDTH, HEIGHT);
    // The node, without its mask, and the place among the nodes of the picture of its first
    // input.
    let cases: [(&str, FusedKind, Option<usize>); 5] = [
        (
            "a merge",
            FusedKind::Merge {
                background: Some(2),
                foreground: Some(1),
                mask: None,
                settings: Merged {
                    center: [0.75, 0.5],
                    ..merge()
                },
            },
            Some(2),
        ),
        (
            "a merge as it is at first over no background",
            FusedKind::Merge {
                background: None,
                foreground: Some(1),
                mask: None,
                settings: merge(),
            },
            None,
        ),
        (
            "a transform",
            FusedKind::Transform {
                input: Some(1),
                mask: None,
                settings: Transformed {
                    center: [0.75, 0.5],
                    ..transform()
                },
            },
            Some(1),
        ),
        (
            "a transform that leaves nothing of its picture",
            FusedKind::Transform {
                input: Some(1),
                mask: None,
                settings: Transformed {
                    size: 0.0,
                    ..transform()
                },
            },
            Some(1),
        ),
        (
            "a background",
            FusedKind::Background {
                rgba: [0.2, 0.4, 0.6, 0.5],
                mask: None,
            },
            None,
        ),
    ];
    // The mask: the red of its picture over black. It is 1 in its middle, 0 at the corners
    // of the frame, and between them about its edge.
    let alone = fused(picture.clone(), limiting(), Some(0));
    let mask: Vec<f32> = frame_of(&gpu, &mut compositor, None, alone, size)
        .as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| pixel[0])
        .collect();
    assert!(mask.iter().any(|value| *value >= 0.999));
    assert!(mask.iter().any(|value| *value <= 0.001));
    assert!(mask.iter().any(|value| (0.2..0.8).contains(value)));
    for under in [None, Some([1.0; 4])] {
        for (what, kind, first) in &cases {
            let mut frame = |mask: Option<Option<usize>>, output: Option<usize>| {
                let mut nodes = limiting();
                if let Some(mask) = mask {
                    nodes.push(with_mask(kind, mask));
                }
                let clip = fused(picture.clone(), nodes, output);
                frame_of(&gpu, &mut compositor, under, clip, size)
            };
            let own = frame(Some(None), Some(3));
            let limited = frame(Some(Some(0)), Some(3));
            let first = frame(None, *first);
            let pixels = limited
                .as_chunks::<4>()
                .0
                .iter()
                .zip(own.as_chunks::<4>().0)
                .zip(first.as_chunks::<4>().0)
                .zip(&mask);
            for (at, (((limited, own), first), mask)) in pixels.enumerate() {
                for channel in 0..3 {
                    let wanted = first[channel] + (own[channel] - first[channel]) * mask;
                    assert!(
                        (limited[channel] - wanted).abs() <= LIMITED_TOLERANCE,
                        "{what}, over {under:?}: pixel {at} is {limited:?}, not {wanted} of \
                         {own:?} and {first:?} by {mask}"
                    );
                }
            }
            // The mask does something: the node with it is not the node without it.
            assert!(
                limited
                    .iter()
                    .zip(&own)
                    .any(|(limited, own)| (limited - own).abs() > 0.05),
                "{what}, over {under:?}"
            );
        }
    }
    assert_no_errors(&gpu);
}

#[test]
fn a_mask_of_1_leaves_what_a_node_draws_to_the_bit_and_a_node_without_one_is_as_it_was() {
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let picture = Source::Picture(pattern(), 0);
    let drawn = |compositor: &mut Compositor, nodes: Vec<FusedKind>, output: usize| {
        let clip = fused(picture.clone(), nodes, Some(output));
        halves(
            &gpu,
            compositor,
            &scene(vec![layer(clip, ColorRange::Full)]),
        )
    };
    // A Rectangle as large as the picture: a mask of 1 all over, and inverted one of 0.
    let whole = |invert: bool| {
        rectangle(
            Masked { invert, ..masked() },
            RectangleShape {
                width: 1.0,
                height: 1.0,
                ..rectangle_shape()
            },
        )
    };
    let source = drawn(&mut compositor, vec![FusedKind::MediaIn], 0);
    assert_eq!(compositor.fusion_passes(), 0);
    // A flip without a mask: the source alone and the flip, as it always was.
    let plain = drawn(&mut compositor, vec![FusedKind::MediaIn, flip(0)], 1);
    assert_ne!(plain, source);
    assert_eq!(compositor.fusion_passes(), 2);
    // A mask node that nothing reads is not drawn, and changes nothing.
    let idle = drawn(
        &mut compositor,
        vec![whole(false), FusedKind::MediaIn, flip(1)],
        2,
    );
    assert_eq!(idle, plain);
    assert_eq!(compositor.fusion_passes(), 4);
    // Limited by a mask of 1: one pass more, that of the mask, and the same picture to the
    // bit.
    let limited = |invert: bool| {
        vec![
            whole(invert),
            FusedKind::MediaIn,
            with_mask(&flip(1), Some(0)),
        ]
    };
    let all = drawn(&mut compositor, limited(false), 2);
    assert_eq!(all, plain);
    assert_eq!(compositor.fusion_passes(), 7);
    // Limited by a mask of 0: the picture as it came in, which went through a texture of
    // the composition, and is within a step of the source.
    let none = drawn(&mut compositor, limited(true), 2);
    assert_within(&none, &source, 1, "a mask of 0");
    assert_eq!(compositor.fusion_passes(), 10);
    assert_no_errors(&gpu);
}

#[test]
fn a_composition_of_masks_is_shown_as_its_source_until_its_pipelines_are_made() {
    let Some(gpu) = gpu_for_tests() else { return };
    let picture = Source::Picture(pattern(), 0);
    let under = colour([0.2, 0.5, 0.3, 1.0]);
    let plain = scene(vec![
        under.clone(),
        layer(picture.clone(), ColorRange::Full),
    ]);
    // The picture of the clip flipped, limited by an Ellipse: a pass that lays and a pass
    // that draws a mask, each by a pipeline of its own.
    let mut nodes = limiting();
    nodes.push(with_mask(&flip(1), Some(0)));
    let limited = scene(vec![
        under,
        layer(fused(picture, nodes, Some(3)), ColorRange::Full),
    ]);
    // A compositor that waits for its pipelines draws each frame as its scene says.
    let mut waiting = Compositor::new(Arc::clone(&gpu));
    let source = halves(&gpu, &mut waiting, &plain);
    let pipelines = waiting.pipelines_made();
    let wanted = halves(&gpu, &mut waiting, &limited);
    assert_ne!(source, wanted);
    assert!(!waiting.is_provisional());
    assert_eq!(waiting.pipelines_made(), pipelines + 2);
    // The source alone, the mask and the flip.
    assert_eq!(waiting.fusion_passes(), 3);
    // One that does not shows the clip without its composition at first, and says so.
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    compositor.compile_in_background(|| {});
    assert_eq!(halves(&gpu, &mut compositor, &limited), source);
    assert!(compositor.is_provisional());
    assert_eq!(compositor.fusion_passes(), 0);
    // Once both pipelines are made, it draws the frame as it is.
    compositor.finish_compiling();
    assert_eq!(halves(&gpu, &mut compositor, &limited), wanted);
    assert!(!compositor.is_provisional());
    assert_eq!(compositor.fusion_passes(), 3);
    assert_no_errors(&gpu);
}
