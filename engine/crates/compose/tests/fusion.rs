//! The composition of a clip, drawn on the card: what comes out of its nodes takes the place of
//! the source of the clip, before the grade of the clip, its crop, its transform, its composite
//! mode and its opacity. The composition that a clip has at first draws the clip as it is, to
//! the bit; the shader of a Merge is held to the mathematics of the processor (`merge.rs`), and
//! that of the masks to theirs (`mask.rs`); an effect mask mixes what a node draws with the
//! picture of its first input; a Blur is held to the filters of the neighbourhoods on the
//! processor (`neighbourhood.rs`) and a Mosaic to its mathematics (`mosaic.rs`); and a layer
//! without a composition is drawn as it always was. Without an adapter the tests are skipped,
//! unless `COLUMBINA_REQUIRE_GPU` is 1.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use columbina_compose::grade::maths::Constants;
use columbina_compose::grade::REC709_LUMA;
use columbina_compose::keyer::{self, Incoming};
use columbina_compose::mask::{self, Common, Shape};
use columbina_compose::merge::{merge_pixel, Settings};
use columbina_compose::mosaic;
use columbina_compose::neighbourhood::{self, Image};
use columbina_compose::{
    card_for_tests, gpu_for_tests, ColorOp, ColorRange, Composite, Compositor, Crop, Filter,
    Fusion, Gpu, LayerColor, Scene, SceneLayer, Source, Stage, Target, Text, Transform, YuvCoding,
    YuvMatrix,
};
use columbina_media::{Alpha, NativePicture, PictureFormat};
use columbina_model::fusion::{
    BlurFilter, Blurred, DeltaKeyed, DeltaKeyer, EllipseShape, FillMethod, Fused, FusedKind,
    FusedNode, KeyerView, Masked, Merged, Mosaicked, Operator, PaintMode, Pattern, PolygonShape,
    RectangleShape, ReplaceMode, SpillMethod, Tracked, Tracker, TrackerMerge, TrackerOperation,
    Transformed, MOST_POINTS,
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
        stroke: None,
        shadow: None,
        background: None,
        spans: Vec::new(),
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

/// A case of the controls of a keyer: what it is, what it sets of a keyer as it is at first,
/// and how far from the processor the card may be.
type KeyerCase = (&'static str, fn(&mut DeltaKeyed), f32);

/// A case of the fitting of the picture of a composition: what it is, the size of the
/// composition, the size of the frame, and where the picture is to lie in the frame (x, y,
/// width and height).
type FittedCase = (&'static str, (u32, u32), (u32, u32), [u32; 4]);

/// The sources of the kinds that a clip has: a picture, two pictures mixed, a title and a
/// colour.
fn sources() -> Vec<Kind> {
    let own = (WIDTH, HEIGHT);
    let mixed = Source::Between {
        first: flat([180, 100, 140]),
        second: flat([60, 150, 110]),
        weight: 0.25,
        rotation: 0,
        flow: None,
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
        FusedKind::Blur {
            input: Some(6),
            mask: Some(5),
            settings: blur_of(BlurFilter::Gaussian, [5.0, 5.0], 1.0),
        },
        FusedKind::Mosaic {
            input: Some(7),
            mask: None,
            settings: tiles(32.0),
        },
        FusedKind::DeltaKeyer {
            input: Some(8),
            garbage_matte: Some(3),
            solid_matte: Some(4),
            mask: Some(5),
            settings: keyed(|_| {}),
        },
        FusedKind::Tracker {
            input: Some(9),
            foreground: Some(6),
            mask: Some(5),
            settings: put_by(TrackerMerge::FgOverBg, [0.625, 0.375]),
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
    | FusedKind::Polygon { mask: own, .. }
    | FusedKind::Blur { mask: own, .. }
    | FusedKind::Mosaic { mask: own, .. }
    | FusedKind::DeltaKeyer { mask: own, .. }
    | FusedKind::Tracker { mask: own, .. } = &mut kind
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

// ---------------------------------------------------------------------------------------------
// Blur and Mosaic
// ---------------------------------------------------------------------------------------------

/// How far what the card makes of a Blur may be from what the processor makes of it
/// (`neighbourhood.rs`): what the neighbourhoods of a grade are held to (`tests/blur.rs`).
/// The picture goes through the textures of half floats of the two passes of the filter, which
/// the card may round towards 0 and the processor rounds to the nearest, and then through
/// that of the composition and that of the frame.
const BLUR_TOLERANCE: f32 = 4e-3;

/// The same for a Blur that is limited by an effect mask: the mask that the processor is given
/// is read from a frame, and may be a step of a half float off the one that the card read.
const BLUR_LIMITED_TOLERANCE: f32 = 5e-3;

/// How far the colour of a cell of a Mosaic on the card may be from the mean that the
/// processor makes of it (`mosaic.rs`), of a picture from 0 to 1: the card sums in floats of 32
/// bits, and its two passes write half floats, each to the nearest or towards 0, a thousandth
/// of the value at the most each; and the picture that the processor is given is read from a
/// frame, and may be a step of a half float off the one that the card read.
const MOSAIC_TOLERANCE: f32 = 3e-3;

/// The same for a Mosaic that is keyed: the key that the processor is given is read from a
/// frame too, and the pixel that is mixed is written into one half float more.
const MOSAIC_KEYED_TOLERANCE: f32 = 5e-3;

/// The size of the frames that the mosaics are drawn in: neither side is a whole number of
/// most of the cells of the tests.
const MOSAIC: (u32, u32) = (40, 24);

/// The settings of a Blur of `filter`, of the Blur Size `size` across and down, blended in by
/// `blend`.
fn blur_of(filter: BlurFilter, size: [f64; 2], blend: f64) -> Blurred {
    Blurred {
        filter,
        size,
        blend,
    }
}

/// A Blur of the picture at `input`, with no effect mask.
fn blur(input: usize, settings: Blurred) -> FusedKind {
    FusedKind::Blur {
        input: Some(input),
        mask: None,
        settings,
    }
}

/// The settings of a Mosaic as it is at first, but of `frequency` cells across.
fn tiles(frequency: f64) -> Mosaicked {
    Mosaicked {
        frequency,
        aspect: 1.0,
        offset: [0.0, 0.0],
        whole_cells_only: false,
    }
}

/// A Mosaic of the picture at `input`, with no effect mask.
fn tiled(input: usize, settings: Mosaicked) -> FusedKind {
    FusedKind::Mosaic {
        input: Some(input),
        mask: None,
        settings,
    }
}

/// A number of pixels as a float.
fn float(pixels: u32) -> f32 {
    f32::from(u16::try_from(pixels).unwrap())
}

/// A picture of R'G'B'A of `width` × `height`, opaque, no two neighbours of which are of one
/// colour.
fn busy(width: u32, height: u32) -> Arc<NativePicture> {
    let format = PictureFormat::Rgba64;
    let mut luma = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let words = [
                6000 + (x * 7919 + y * 104_729) % 40_000,
                9000 + (x * 3571 + y * 1543) % 30_000,
                4000 + 9000 * ((x + 2 * y) % 5),
                u32::from(u16::MAX),
            ];
            for word in words {
                luma.extend(u16::try_from(word).unwrap().to_le_bytes());
            }
        }
    }
    Arc::new(NativePicture {
        frame: 0,
        width,
        height,
        format,
        bits: 16,
        matrix: YuvMatrix::Bt709,
        range: ColorRange::Full,
        chroma_location: 0,
        alpha: Alpha::Opaque,
        luma,
        chroma: Vec::new(),
        luma_stride: width as usize * format.pixel_bytes(),
        chroma_stride: 0,
        ..NativePicture::default()
    })
}

/// The pixels of a frame: R, G, B and A of each, row after row.
fn pixels_of(frame: &[f32]) -> Vec<[f32; 4]> {
    frame.as_chunks::<4>().0.to_vec()
}

/// R, G and B of each of `pixels`.
fn rgb_of(pixels: &[[f32; 4]]) -> Vec<[f32; 3]> {
    pixels
        .iter()
        .map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .collect()
}

/// The red of each pixel of a frame: of the frame of a mask over black, the mask.
fn reds(frame: &[f32]) -> Vec<f32> {
    frame
        .as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| pixel[0])
        .collect()
}

/// Whether two frames differ somewhere by more than `by`.
fn differ(one: &[f32], other: &[f32], by: f32) -> bool {
    one.iter()
        .zip(other)
        .any(|(one, other)| (one - other).abs() > by)
}

/// Check that R, G and B of each pixel of the frame `shown` are those of `wanted`, within
/// `tolerance`.
#[track_caller]
fn assert_colours(shown: &[f32], wanted: &[[f32; 3]], tolerance: f32, what: &str) {
    assert_eq!(shown.len(), wanted.len() * 4, "{what}");
    for (at, (pixel, wanted)) in shown.as_chunks::<4>().0.iter().zip(wanted).enumerate() {
        for channel in 0..3 {
            assert!(
                (pixel[channel] - wanted[channel]).abs() <= tolerance,
                "{what}: pixel {at} is {pixel:?}, not {wanted:?}"
            );
        }
    }
}

/// What a Blur of `settings` makes on the processor of the frame `input`, of `size` pixels,
/// each pixel mixed with that of the frame by `share` of it (its Blend, times its mask
/// there): R, G and B of each pixel.
///
/// The frame is filtered as the card filters the picture (`neighbourhood::filtered`), a Blur
/// Size being so many pixels of a picture 1080 high, and what that makes is read between its
/// texels at the middle of each pixel (`neighbourhood::sample`). A frame is the picture laid
/// over what is under it, and the filter is a mean whose weights are 1 in all: so the frame of
/// a blurred picture is the frame of the picture, blurred, whatever the alpha of the picture.
#[allow(
    clippy::cast_possible_truncation,
    reason = "a Blur Size, as the card takes it"
)]
fn blurred_of(
    input: &[f32],
    size: (u32, u32),
    settings: &Blurred,
    share: &dyn Fn(usize) -> f32,
) -> Vec<[f32; 3]> {
    let image = Image {
        width: size.0,
        height: size.1,
        pixels: pixels_of(input),
    };
    let reach = settings.size.map(|reach| reach as f32);
    let filter = match settings.filter {
        BlurFilter::Gaussian => Filter::Gaussian { sigma: reach },
        BlurFilter::Box => Filter::Box { radius: reach },
    };
    let height = f64::from(size.1);
    let units = neighbourhood::units(height, height, height);
    let soft = neighbourhood::filtered(&image, filter, units);
    let mut mixed = Vec::with_capacity(image.pixels.len());
    for y in 0..size.1 {
        for x in 0..size.0 {
            let at = (y * size.0 + x) as usize;
            let place = [
                (float(x) + 0.5) / float(size.0),
                (float(y) + 0.5) / float(size.1),
            ];
            let own = neighbourhood::sample(&soft, place);
            let first = image.pixels[at];
            let share = share(at).clamp(0.0, 1.0);
            let mix = |channel: usize| first[channel] + (own[channel] - first[channel]) * share;
            mixed.push([mix(0), mix(1), mix(2)]);
        }
    }
    mixed
}

/// A picture of colours that needs no source: a soft white Ellipse, turned, over a
/// Background. The picture is that of the node at 2; the Ellipse alone, whose alpha is not 1
/// all over, is at 1.
fn soft_over() -> Vec<FusedKind> {
    vec![
        FusedKind::Background {
            rgba: [0.2, 0.4, 0.6, 1.0],
            mask: None,
        },
        ellipse(
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
        ),
        FusedKind::Merge {
            background: Some(0),
            foreground: Some(1),
            mask: None,
            settings: merge(),
        },
    ]
}

#[test]
fn the_blur_of_a_composition_is_that_of_the_filters_of_the_processor() {
    // A Blur of the picture of the clip, in a frame that is not square: what the processor
    // makes of the frame of the clip by the filters of the neighbourhoods. A Blur Size is so
    // many pixels of a picture 1080 high: in one 8 pixels high, 270 is 2 pixels.
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let size = (WIDTH, HEIGHT);
    let picture = Source::Picture(pattern(), 0);
    let source = frame_of(&gpu, &mut compositor, None, picture.clone(), size);
    let cases: [(&str, Blurred, f32); 5] = [
        (
            "a gaussian",
            blur_of(BlurFilter::Gaussian, [200.0, 200.0], 1.0),
            1.0,
        ),
        ("a box", blur_of(BlurFilter::Box, [300.0, 300.0], 1.0), 1.0),
        (
            "a gaussian wider across than down",
            blur_of(BlurFilter::Gaussian, [300.0, 100.0], 1.0),
            1.0,
        ),
        (
            "a box across alone",
            blur_of(BlurFilter::Box, [400.0, 0.0], 1.0),
            1.0,
        ),
        (
            "a gaussian half blended in",
            blur_of(BlurFilter::Gaussian, [200.0, 200.0], 0.5),
            0.5,
        ),
    ];
    let mut drawn = 0;
    for (what, settings, share) in cases {
        let wanted = blurred_of(&source, size, &settings, &|_| share);
        let nodes = vec![FusedKind::MediaIn, blur(0, settings)];
        let clip = fused(picture.clone(), nodes, Some(1));
        let shown = frame_of(&gpu, &mut compositor, None, clip, size);
        assert_colours(&shown, &wanted, BLUR_TOLERANCE, what);
        // The blur does something.
        assert!(differ(&shown, &source, 0.01), "{what}");
        drawn += 4;
    }
    // The source alone, the two passes of the filter and the pass that mixes, each time.
    assert_eq!(compositor.fusion_passes(), drawn);
    assert_no_errors(&gpu);
}

#[test]
fn a_wide_blur_is_taken_at_fewer_points_and_blurs_the_alpha_with_the_colour() {
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    // In a picture 32 pixels high, 270 is 8 pixels and 540 is 16: wide enough for the filter
    // to be taken at a pixel of four, into a texture that is read between its texels.
    let cases = [
        (
            "a gaussian of 8 pixels",
            blur_of(BlurFilter::Gaussian, [270.0, 270.0], 1.0),
        ),
        (
            "a box of 16 pixels across and 4 down",
            blur_of(BlurFilter::Box, [540.0, 135.0], 1.0),
        ),
    ];
    for (what, settings) in cases {
        // A picture of colours, which is opaque; and an Ellipse alone, whose alpha is its
        // mask: over white its frame is white all over, and so is that of its blur, the alpha
        // being blurred as the colour is.
        for (picture, at) in [(soft_over(), 2), (soft_over()[..2].to_vec(), 1)] {
            for under in [None, Some([1.0; 4])] {
                let what = format!("{what}, of the node at {at}, over {under:?}");
                let clip = clip_of(picture.clone(), at, MASK);
                let input = frame_of(&gpu, &mut compositor, under, clip, MASK);
                let mut nodes = picture.clone();
                nodes.push(blur(at, settings));
                let clip = clip_of(nodes, at + 1, MASK);
                let shown = frame_of(&gpu, &mut compositor, under, clip, MASK);
                let wanted = blurred_of(&input, MASK, &settings, &|_| 1.0);
                assert_colours(&shown, &wanted, BLUR_TOLERANCE, &what);
                // Over black the blur does something.
                assert!(under.is_some() || differ(&shown, &input, 0.01), "{what}");
            }
        }
    }
    assert_no_errors(&gpu);
}

#[test]
fn a_blur_is_the_same_share_of_the_picture_at_whatever_size_it_is_drawn() {
    // A Blur Size is a share of the height of the picture: drawn at another size, the blur is
    // what the processor makes of the picture as it is drawn at that size. 270 is 8 pixels of
    // a picture 32 high, and 16 of one 64 high.
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let settings = blur_of(BlurFilter::Gaussian, [270.0, 135.0], 1.0);
    // The size of the composition, and that of the frame that shows it.
    let sizes: [((u32, u32), (u32, u32)); 3] = [
        ((48, 32), (48, 32)),
        ((96, 64), (48, 32)),
        ((96, 64), (96, 64)),
    ];
    for (nominal, seen) in sizes {
        let what = format!("a composition of {nominal:?} in a frame of {seen:?}");
        let clip = clip_of(soft_over(), 2, nominal);
        let input = frame_of(&gpu, &mut compositor, None, clip, seen);
        let mut nodes = soft_over();
        nodes.push(blur(2, settings));
        let clip = clip_of(nodes, 3, nominal);
        let shown = frame_of(&gpu, &mut compositor, None, clip, seen);
        let wanted = blurred_of(&input, seen, &settings, &|_| 1.0);
        assert_colours(&shown, &wanted, BLUR_TOLERANCE, &what);
        assert!(differ(&shown, &input, 0.01), "{what}");
    }
    assert_no_errors(&gpu);
}

#[test]
fn the_blend_and_the_effect_mask_of_a_blur_mix_it_with_its_picture_as_it_came_in() {
    // `input + (blurred - input) * blend * mask` of each of R, G and B: the mask is the red
    // of the frame of its picture over black.
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let size = (WIDTH, HEIGHT);
    let picture = Source::Picture(pattern(), 0);
    let source = frame_of(&gpu, &mut compositor, None, picture.clone(), size);
    let alone = fused(picture.clone(), limiting(), Some(0));
    let mask = reds(&frame_of(&gpu, &mut compositor, None, alone, size));
    assert!(mask.iter().any(|value| *value >= 0.999));
    assert!(mask.iter().any(|value| *value <= 0.001));
    assert!(mask.iter().any(|value| (0.2..0.8).contains(value)));
    let cases: [(&str, Blurred, f32); 2] = [
        (
            "a gaussian",
            blur_of(BlurFilter::Gaussian, [300.0, 200.0], 1.0),
            1.0,
        ),
        (
            "a box half blended in",
            blur_of(BlurFilter::Box, [300.0, 200.0], 0.5),
            0.5,
        ),
    ];
    for (what, settings, blend) in cases {
        let mut nodes = limiting();
        nodes.push(with_mask(&blur(1, settings), Some(0)));
        let clip = fused(picture.clone(), nodes, Some(3));
        let shown = frame_of(&gpu, &mut compositor, None, clip, size);
        let wanted = blurred_of(&source, size, &settings, &|at| blend * mask[at]);
        assert_colours(&shown, &wanted, BLUR_LIMITED_TOLERANCE, what);
        // The mask does something: the node with it is not the node without it.
        let nodes = vec![FusedKind::MediaIn, blur(0, settings)];
        let clip = fused(picture.clone(), nodes, Some(1));
        let whole = frame_of(&gpu, &mut compositor, None, clip, size);
        assert!(differ(&shown, &whole, 0.005), "{what}");
    }
    assert_no_errors(&gpu);
}

#[test]
fn a_blur_that_changes_nothing_is_no_pass_and_its_mask_keeps_its_ends_to_the_bit() {
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
    let source = drawn(&mut compositor, vec![FusedKind::MediaIn], 0);
    assert_eq!(compositor.fusion_passes(), 0);
    // A Blur Size of 0 and a Blend of 0, and a Mosaic whose cells are a pixel: the clip as it
    // is, to the bit, and no pass.
    for idle in [
        blur(0, blur_of(BlurFilter::Gaussian, [0.0, 0.0], 1.0)),
        blur(0, blur_of(BlurFilter::Box, [300.0, 300.0], 0.0)),
        tiled(0, tiles(f64::from(WIDTH))),
    ] {
        let shown = drawn(&mut compositor, vec![FusedKind::MediaIn, idle.clone()], 1);
        assert!(shown == source, "{idle:?}");
    }
    assert_eq!(compositor.fusion_passes(), 0);
    // A composition without a Blur or a Mosaic is drawn by the passes that it was drawn by,
    // the source alone and two flips; and with either after them, changing nothing, by the
    // same passes and to the bit.
    let (nodes, _) = there_and_back();
    let plain = drawn(&mut compositor, nodes.clone(), 2);
    assert_eq!(compositor.fusion_passes(), 3);
    let mut longer = nodes;
    longer.push(blur(2, blur_of(BlurFilter::Gaussian, [0.0, 0.0], 1.0)));
    longer.push(tiled(3, tiles(16_384.0)));
    assert_eq!(drawn(&mut compositor, longer, 4), plain);
    assert_eq!(compositor.fusion_passes(), 6);
    // A Blur: the source alone, the two passes of its filter and the pass that mixes.
    let settings = blur_of(BlurFilter::Gaussian, [300.0, 200.0], 1.0);
    let blurred = drawn(
        &mut compositor,
        vec![FusedKind::MediaIn, blur(0, settings)],
        1,
    );
    assert_ne!(blurred, source);
    assert_eq!(compositor.fusion_passes(), 10);
    // A Rectangle as large as the picture: a mask of 1 all over, and inverted one of 0.
    let limited = |invert: bool| {
        let whole = rectangle(
            Masked { invert, ..masked() },
            RectangleShape {
                width: 1.0,
                height: 1.0,
                ..rectangle_shape()
            },
        );
        vec![
            whole,
            FusedKind::MediaIn,
            with_mask(&blur(1, settings), Some(0)),
        ]
    };
    // Limited by a mask of 1: one pass more, that of the mask, and the same picture to the
    // bit.
    let all = drawn(&mut compositor, limited(false), 2);
    assert_eq!(all, blurred);
    assert_eq!(compositor.fusion_passes(), 15);
    // Limited by a mask of 0: the picture as it came in, which went through a texture of the
    // composition, and is within a step of the source.
    let none = drawn(&mut compositor, limited(true), 2);
    assert_within(&none, &source, 1, "a mask of 0");
    assert_eq!(compositor.fusion_passes(), 20);
    assert_no_errors(&gpu);
}

#[test]
fn the_shader_of_a_mosaic_is_held_to_the_mathematics_of_the_processor() {
    // The picture of the clip laid out in cells, in a frame whose sides are no whole number
    // of most of them: each pixel is the mean of its cell as the processor makes it of the
    // frame of the clip, and every pixel of a cell is of one colour, to the bit, so that the
    // cells of the card are those of the processor.
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let (width, height) = MOSAIC;
    let picture = Source::Picture(busy(width, height), 0);
    let source = frame_of(&gpu, &mut compositor, None, picture.clone(), MOSAIC);
    let src = pixels_of(&source);
    let cases = [
        ("an even number of cells across", tiles(6.0)),
        ("an odd number of cells across", tiles(5.0)),
        (
            "cells twice as wide as high",
            Mosaicked {
                aspect: 2.0,
                ..tiles(6.0)
            },
        ),
        (
            "cells half as wide as high, the grid moved",
            Mosaicked {
                aspect: 0.5,
                offset: [1.5, -2.25],
                ..tiles(7.0)
            },
        ),
        (
            "the grid moved by more than a cell",
            Mosaicked {
                offset: [-11.0, 30.5],
                ..tiles(4.0)
            },
        ),
        ("a cell as wide as the frame", tiles(1.0)),
    ];
    let mut drawn = 0;
    for (what, settings) in cases {
        let kind = tiled(0, settings);
        let held = mosaic::of_node(&kind).expect("a mosaic");
        let wanted = mosaic::mosaic(&src, width, height, &held, None);
        let nodes = vec![FusedKind::MediaIn, kind];
        let clip = fused_of(picture.clone(), nodes, Some(1), MOSAIC);
        let shown = frame_of(&gpu, &mut compositor, None, clip, MOSAIC);
        assert_colours(&shown, &rgb_of(&wanted), MOSAIC_TOLERANCE, what);
        assert!(differ(&shown, &source, 0.05), "{what}");
        let grid = mosaic::Grid::new(width, height, &held);
        let colours = pixels_of(&shown);
        let [across, down] = grid.cells();
        for j in 0..down {
            for i in 0..across {
                let rect = grid.rect([i, j]);
                assert!(!rect.is_empty(), "{what}: the cell {i}, {j}");
                let first = colours[(rect.y0 * width + rect.x0) as usize];
                for y in rect.y0..rect.y1 {
                    for x in rect.x0..rect.x1 {
                        // One colour, but for what the laying of the picture of the
                        // composition on the canvas rounds: two steps of a half float.
                        let pixel = colours[(y * width + x) as usize];
                        let apart = pixel
                            .iter()
                            .zip(&first)
                            .map(|(one, other)| (one - other).abs())
                            .fold(0.0, f32::max);
                        assert!(
                            apart <= 1.0 / 1024.0,
                            "{what}: the cell {i}, {j} at {x}, {y}: {pixel:?} and {first:?}"
                        );
                    }
                }
            }
        }
        drawn += 4;
    }
    // The source alone, the cells across and down, and the picture, each time.
    assert_eq!(compositor.fusion_passes(), drawn);
    assert_no_errors(&gpu);
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "a soft key, and a crisp one with and without Whole Cells Only"
)]
fn the_effect_mask_of_a_mosaic_is_its_key_and_whole_cells_only_shows_the_cells_it_covers() {
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let (width, height) = MOSAIC;
    let picture = Source::Picture(busy(width, height), 0);
    let source = frame_of(&gpu, &mut compositor, None, picture.clone(), MOSAIC);
    let src = pixels_of(&source);
    // What the card makes of the picture of the clip keyed by the mask `mask`; what the
    // processor makes of the frame of the clip by the key that the frame of the mask shows;
    // and that key.
    let mut keyed = |mask: &FusedKind, settings: Mosaicked| {
        let alone = fused_of(picture.clone(), vec![mask.clone()], Some(0), MOSAIC);
        let key = reds(&frame_of(&gpu, &mut compositor, None, alone, MOSAIC));
        let kind = with_mask(&tiled(1, settings), Some(0));
        let held = mosaic::of_node(&kind).expect("a mosaic");
        let wanted = mosaic::mosaic(&src, width, height, &held, Some(key.as_slice()));
        let nodes = vec![mask.clone(), FusedKind::MediaIn, kind];
        let clip = fused_of(picture.clone(), nodes, Some(2), MOSAIC);
        let shown = frame_of(&gpu, &mut compositor, None, clip, MOSAIC);
        (shown, wanted, key)
    };
    // A soft Ellipse: each pixel is as far from the picture to the colour of its cell as the
    // key is there.
    let soft = ellipse(
        Masked {
            soft_edge: 0.125,
            ..masked()
        },
        EllipseShape {
            height: 0.25,
            ..ellipse_shape()
        },
    );
    let (shown, wanted, key) = keyed(&soft, tiles(6.0));
    assert!(key.iter().any(|value| *value >= 0.999));
    assert!(key.iter().any(|value| *value <= 0.001));
    assert!(key.iter().any(|value| (0.2..0.8).contains(value)));
    assert_colours(
        &shown,
        &rgb_of(&wanted),
        MOSAIC_KEYED_TOLERANCE,
        "a soft key",
    );
    // The key does something: the picture is neither as it came in nor all in cells.
    let held = mosaic::of_node(&tiled(0, tiles(6.0))).expect("a mosaic");
    let whole = mosaic::mosaic(&src, width, height, &held, None);
    assert!(differ(&shown, &source, 0.05));
    assert!(differ(&shown, whole.as_flattened(), 0.05));
    // A crisp Rectangle whose sides lie between pixels, from 10 to 30 across and from 3 to
    // 21 down: a key of 0 and of 1 alone. Over cells of 8 pixels, whose sides are at 4, 12,
    // 20, 28 and 36 across and at 4, 12 and 20 down, it covers four cells whole.
    let crisp = rectangle(
        masked(),
        RectangleShape {
            height: 0.75,
            ..rectangle_shape()
        },
    );
    let only = Mosaicked {
        whole_cells_only: true,
        ..tiles(5.0)
    };
    let (covered, wanted, key) = keyed(&crisp, only);
    assert!(key.iter().all(|value| *value <= 0.0 || *value >= 1.0));
    let held = mosaic::of_node(&tiled(0, only)).expect("a mosaic");
    let grid = mosaic::Grid::new(width, height, &held);
    let leasts = mosaic::cell_leasts(&key, &grid);
    assert_eq!(leasts.iter().filter(|least| **least >= 1.0).count(), 4);
    assert_colours(
        &covered,
        &rgb_of(&wanted),
        MOSAIC_KEYED_TOLERANCE,
        "whole cells only",
    );
    assert!(differ(&covered, &source, 0.05));
    // Without Whole Cells Only the same key shows the cells that it crosses too, cut by its
    // edge: more of the picture is in cells.
    let (cut, wanted, _) = keyed(&crisp, tiles(5.0));
    assert_colours(
        &cut,
        &rgb_of(&wanted),
        MOSAIC_KEYED_TOLERANCE,
        "cells cut by the key",
    );
    assert!(differ(&cut, &covered, 0.05));
    // The mask alone, each time; and then the mask, the source alone, the cells across and
    // down, the picture, and with Whole Cells Only the least of the key across and down.
    assert_eq!(compositor.fusion_passes(), 6 + 8 + 6);
    assert_no_errors(&gpu);
}

#[test]
fn a_mosaic_drawn_smaller_has_the_same_cells_over_the_same_parts_of_the_picture() {
    // A composition of twice the size of the frame that shows it is drawn at the size of the
    // frame: as many cells lie across it, and its offsets, which are pixels of the
    // composition, are half as many pixels of the frame. At its own size they are as they
    // are.
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let (width, height) = MOSAIC;
    let picture = Source::Picture(busy(width, height), 0);
    let source = frame_of(&gpu, &mut compositor, None, picture.clone(), MOSAIC);
    let halved = Mosaicked {
        offset: [1.5, -2.25],
        ..tiles(6.0)
    };
    let held = mosaic::of_node(&tiled(0, halved)).expect("a mosaic");
    let wanted = rgb_of(&mosaic::mosaic(
        &pixels_of(&source),
        width,
        height,
        &held,
        None,
    ));
    let twice = (width * 2, height * 2);
    let doubled = Mosaicked {
        offset: [3.0, -4.5],
        ..tiles(6.0)
    };
    for (what, nominal, settings) in [
        ("at its own size", MOSAIC, halved),
        ("at half its size", twice, doubled),
    ] {
        let nodes = vec![FusedKind::MediaIn, tiled(0, settings)];
        let clip = fused_of(picture.clone(), nodes, Some(1), nominal);
        let shown = frame_of(&gpu, &mut compositor, None, clip, MOSAIC);
        assert_colours(&shown, &wanted, MOSAIC_TOLERANCE, what);
        assert!(differ(&shown, &source, 0.05), "{what}");
    }
    assert_no_errors(&gpu);
}

#[test]
fn a_composition_with_a_blur_and_a_mosaic_is_shown_as_its_source_until_its_pipelines_are_made() {
    let Some(gpu) = gpu_for_tests() else { return };
    let picture = Source::Picture(pattern(), 0);
    let under = colour([0.2, 0.5, 0.3, 1.0]);
    let plain = scene(vec![
        under.clone(),
        layer(picture.clone(), ColorRange::Full),
    ]);
    // The picture of the clip blurred, and that laid out in cells of four pixels: the passes
    // of a filter of the neighbourhoods, whose pipeline is made at once, and a pass that mixes
    // and the passes of a mosaic, each by a pipeline of its own.
    let nodes = vec![
        FusedKind::MediaIn,
        blur(0, blur_of(BlurFilter::Gaussian, [300.0, 200.0], 1.0)),
        tiled(1, tiles(4.0)),
    ];
    let filtered = scene(vec![
        under,
        layer(fused(picture, nodes, Some(2)), ColorRange::Full),
    ]);
    // A compositor that waits for its pipelines draws each frame as its scene says.
    let mut waiting = Compositor::new(Arc::clone(&gpu));
    let source = halves(&gpu, &mut waiting, &plain);
    let pipelines = waiting.pipelines_made();
    let wanted = halves(&gpu, &mut waiting, &filtered);
    assert_ne!(source, wanted);
    assert!(!waiting.is_provisional());
    assert_eq!(waiting.pipelines_made(), pipelines + 2);
    // The source alone; the two passes of the filter and the pass that mixes; and the cells
    // across and down, and the picture.
    assert_eq!(waiting.fusion_passes(), 7);
    // One that does not shows the clip without its composition at first, and says so.
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    compositor.compile_in_background(|| {});
    assert_eq!(halves(&gpu, &mut compositor, &filtered), source);
    assert!(compositor.is_provisional());
    assert_eq!(compositor.fusion_passes(), 0);
    // Once both pipelines are made, it draws the frame as it is.
    compositor.finish_compiling();
    assert_eq!(halves(&gpu, &mut compositor, &filtered), wanted);
    assert!(!compositor.is_provisional());
    assert_eq!(compositor.fusion_passes(), 7);
    assert_no_errors(&gpu);
}

// ---------------------------------------------------------------------------------------------
// The Delta Keyer
// ---------------------------------------------------------------------------------------------

/// How far what the card makes of a pixel by a keyer may be from what the processor makes of
/// it (`keyer.rs`), of pixels given to the card as floats of 32 bits: a thousandth, or of its
/// size where that is above 1, as of a Merge. It is more than the step of a half float that
/// keeping the result takes, and less than any error of a formula.
const KEYER_TOLERANCE: f32 = 1e-3;

/// The same for a matte that went through a Pre-Blur, a Matte Blur or a Clean: the picture or
/// the matte goes through the half floats of the two passes of a filter, which the card may
/// round towards 0 and the processor rounds to the nearest, and the key makes some three
/// times as much of a difference of the picture.
const KEYER_FILTERED_TOLERANCE: f32 = 1e-2;

/// The same for a matte that went through an Erode/Dilate: where the mean of the matte about
/// a pixel is within the edge of the control, a twentieth wide, a step of a half float of
/// that mean is twenty of the matte.
const KEYER_MORPHED_TOLERANCE: f32 = 4e-2;

/// How far a frame of a keyed picture may be from what the processor makes of the frames of
/// what the keyer read: each of those went through its own half floats, a step off what the
/// card read, of which the key makes some three; and what is keyed goes through two more.
const KEYED_TOLERANCE: f32 = 6e-3;

/// The colour of the screen of the pictures of these tests: a green one as a camera sees it.
const STAGE: [f32; 3] = [0.08, 0.45, 0.12];

/// [`STAGE`] as the Background Color of a keyer.
const STAGE_PICKED: [f64; 3] = [0.08, 0.45, 0.12];

/// Colours of every kind, not premultiplied: a pure green; the screen, in shadow and lit
/// twice as brightly; greys; a skin tone; subjects with spill on them; the edge of a grey
/// against the screen; the primaries and the colours of other screens; one beyond white and
/// one with channels below black.
const COLOURS: [[f32; 3]; 16] = [
    [0.0, 1.0, 0.0],
    STAGE,
    [0.04, 0.225, 0.06],
    [0.16, 0.9, 0.24],
    [0.18, 0.18, 0.18],
    [0.9, 0.9, 0.9],
    [0.45, 0.25, 0.18],
    [0.3, 0.5, 0.2],
    [0.2, 0.6, 0.4],
    [0.13, 0.315, 0.15],
    [1.0, 0.0, 0.0],
    [0.1, 0.15, 0.5],
    [0.7, 0.1, 0.15],
    [0.9, 0.9, 0.1],
    [2.5, 4.0, 1.5],
    [-0.05, 0.02, -0.01],
];

/// The settings of a Delta Keyer as they are at first, but for what `set` sets.
fn keyed(set: impl FnOnce(&mut DeltaKeyed)) -> DeltaKeyed {
    let mut settings = DeltaKeyer::default().at(0.0);
    set(&mut settings);
    settings
}

/// The settings of a Delta Keyer of the screen of these tests, set by `set`.
fn staged(set: impl FnOnce(&mut DeltaKeyed)) -> DeltaKeyed {
    keyed(|keyer| {
        keyer.screen = STAGE_PICKED;
        set(keyer);
    })
}

/// A Delta Keyer of the picture at `input`, with neither mattes nor a mask.
fn key(input: usize, settings: DeltaKeyed) -> FusedKind {
    FusedKind::DeltaKeyer {
        input: Some(input),
        garbage_matte: None,
        solid_matte: None,
        mask: None,
        settings,
    }
}

/// What a Delta Keyer of `settings` makes on the processor of `pixels`, a premultiplied
/// picture of `size` pixels, row after row (`keyer::node_picture`): `incoming` gives the
/// mattes and the mask that come into it at the pixel of each number. A length of the keyer
/// is so many pixels of a picture 1080 high.
fn keyed_of(
    settings: &DeltaKeyed,
    pixels: &[[f32; 4]],
    size: (u32, u32),
    incoming: &dyn Fn(usize) -> Incoming,
) -> Vec<[f32; 4]> {
    let image = Image {
        width: size.0,
        height: size.1,
        pixels: pixels.to_vec(),
    };
    let height = f64::from(size.1);
    let units = neighbourhood::units(height, height, height);
    keyer::node_picture(&keyer::Node::from(settings), &image, units, incoming)
}

/// Check that each of R, G, B and A of each pixel of `shown` is that of `wanted`, within
/// `tolerance`, or within that share of its size where it is above 1.
#[track_caller]
fn assert_keyed(shown: &[f32], wanted: &[[f32; 4]], tolerance: f32, what: &str) {
    assert_eq!(shown.len(), wanted.len() * 4, "{what}");
    for (at, (pixel, wanted)) in shown.as_chunks::<4>().0.iter().zip(wanted).enumerate() {
        for channel in 0..4 {
            let most = tolerance * wanted[channel].abs().max(1.0);
            assert!(
                (pixel[channel] - wanted[channel]).abs() <= most,
                "{what}: pixel {at} is {pixel:?}, not {wanted:?}"
            );
        }
    }
}

/// The pixels of the frames of a picture over black and over white, premultiplied: its
/// colour is that over black, and its alpha 1 less what the white under it adds.
fn pixels_over(black: &[f32], white: &[f32]) -> Vec<[f32; 4]> {
    black
        .as_chunks::<4>()
        .0
        .iter()
        .zip(white.as_chunks::<4>().0)
        .map(|(dark, light)| [dark[0], dark[1], dark[2], 1.0 - (light[0] - dark[0])])
        .collect()
}

/// Check that the frames `black` and `white` of a picture, over black and over white, are
/// those of the premultiplied pixels `wanted`, within `tolerance`: over black its colour, and
/// over white that and what its alpha leaves of the white.
#[track_caller]
fn assert_over(black: &[f32], white: &[f32], wanted: &[[f32; 4]], tolerance: f32, what: &str) {
    assert_eq!(black.len(), wanted.len() * 4, "{what}");
    assert_eq!(white.len(), wanted.len() * 4, "{what}");
    let frames = black
        .as_chunks::<4>()
        .0
        .iter()
        .zip(white.as_chunks::<4>().0);
    for (at, ((dark, light), wanted)) in frames.zip(wanted).enumerate() {
        for channel in 0..3 {
            let over = wanted[channel] + (1.0 - wanted[3]);
            assert!(
                (dark[channel] - wanted[channel]).abs() <= tolerance
                    && (light[channel] - over).abs() <= tolerance,
                "{what}: pixel {at} is {dark:?} over black and {light:?} over white, not \
                 {wanted:?}"
            );
        }
    }
}

/// The picture that the shader of a keyer is held to the processor at, 16 pixels by 3: each
/// of [`COLOURS`] opaque; each half seen through, premultiplied; and each with an alpha of 0
/// over channels that hold it still, of which nothing is seen.
fn swatches() -> Vec<[f32; 4]> {
    let mut pixels = Vec::new();
    for alpha in [1.0_f32, 0.5, 0.0] {
        let by = if alpha > 0.0 { alpha } else { 1.0 };
        for [r, g, b] in COLOURS {
            pixels.push([r * by, g * by, b * by, alpha]);
        }
    }
    pixels
}

/// The settings that the shader of a keyer is held to the processor at: the screen as it is
/// at first, picked, blue and red; Gain and Balance varied; thresholds, those that have
/// crossed among them; a screen colour that keys nothing; each spill method, whole and by
/// half; each replace mode, with a matte that gives back; and each view.
#[allow(clippy::too_many_lines, reason = "each setting, one after another")]
fn keyers() -> Vec<(String, DeltaKeyed)> {
    let mut all = vec![
        ("as it is at first".to_owned(), keyed(|_| {})),
        ("a picked screen".to_owned(), staged(|_| {})),
        (
            "a blue screen".to_owned(),
            keyed(|keyer| {
                keyer.screen = [0.1, 0.15, 0.5];
                keyer.spill_method = SpillMethod::WellDone;
            }),
        ),
        (
            "a red screen".to_owned(),
            keyed(|keyer| keyer.screen = [0.7, 0.1, 0.15]),
        ),
        (
            "thresholds that have crossed".to_owned(),
            keyed(|keyer| {
                keyer.low = 0.5;
                keyer.high = 0.2;
            }),
        ),
        (
            "a screen with no channel above its other two".to_owned(),
            keyed(|keyer| keyer.screen = [0.3, 0.3, 0.3]),
        ),
    ];
    for gain in [0.5, 2.0, 10.0] {
        let settings = staged(|keyer| keyer.gain = gain);
        all.push((format!("a gain of {gain}"), settings));
    }
    for balance in [0.0, 0.3, 1.0] {
        let settings = staged(|keyer| keyer.balance = balance);
        all.push((format!("a balance of {balance}"), settings));
    }
    // With the screen as it is at first, no pre-matte of the picture is on a threshold.
    for (low, high) in [(0.1, 0.9), (0.25, 0.75), (0.0, 0.5), (0.5, 1.0), (0.4, 0.4)] {
        let settings = keyed(|keyer| {
            keyer.low = low;
            keyer.high = high;
        });
        all.push((format!("thresholds of {low} and {high}"), settings));
    }
    for method in [
        SpillMethod::None,
        SpillMethod::Rare,
        SpillMethod::Medium,
        SpillMethod::WellDone,
        SpillMethod::Burnt,
    ] {
        for suppression in [1.0, 0.5] {
            let settings = staged(|keyer| {
                keyer.spill_method = method;
                keyer.spill_suppression = suppression;
            });
            all.push((format!("{method:?} by {suppression}"), settings));
        }
    }
    for replace in [
        ReplaceMode::None,
        ReplaceMode::Source,
        ReplaceMode::HardColor,
        ReplaceMode::SoftColor,
    ] {
        let settings = staged(|keyer| {
            keyer.high = 0.5;
            keyer.replace = replace;
            keyer.replace_color = [0.4, 0.3, 0.2];
        });
        all.push((format!("{replace:?}"), settings));
    }
    // With this Gain and these thresholds, no pixel of the picture is on a step of the
    // status.
    for view in VIEWS {
        all.push((format!("{view:?}"), viewed(view)));
    }
    all
}

/// Every view of a keyer.
const VIEWS: [KeyerView; 4] = [
    KeyerView::PreMatte,
    KeyerView::Matte,
    KeyerView::Status,
    KeyerView::FinalResult,
];

/// The settings of a keyer of the screen of these tests that shows `view`, with a Gain and
/// thresholds by which every pixel of [`COLOURS`] is far from the steps of the status: its
/// pre-matte is 0, a quarter, some 0.79 or 1, and its matte 0, some 0.19, some 0.86 or 1.
fn viewed(view: KeyerView) -> DeltaKeyed {
    staged(|keyer| {
        keyer.gain = 1.5;
        keyer.low = 0.1;
        keyer.high = 0.9;
        keyer.view = view;
    })
}

#[test]
fn the_shader_of_a_keyer_is_held_to_the_mathematics_of_the_processor() {
    // Colours of every kind, opaque, seen through in part and not seen, given to the card as
    // floats of 32 bits, keyed with each setting: what the card makes of each pixel, which
    // it keeps in a half float, is within a thousandth of what the processor makes of it, or
    // of its size where that is above 1.
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let size = (16, 3);
    let pixels = swatches();
    let target = Target::preview(&gpu, size.0, size.1).unwrap();
    let alone = [Some(pixels.as_slice()), None, None, None];
    let mut made = Vec::new();
    for (what, settings) in keyers() {
        compositor.key_for_tests(alone, &settings, &target).unwrap();
        let shown = target.read_floats().unwrap();
        let wanted = keyed_of(&settings, &pixels, size, &|_| Incoming::default());
        assert_keyed(&shown, &wanted, KEYER_TOLERANCE, &what);
        made.push(shown);
    }
    // The settings do something: no two of the first four screens key the picture alike.
    for pair in made[..4].windows(2) {
        assert!(differ(&pair[0], &pair[1], 0.05));
    }
    // A keyer that changes nothing has no pass to draw; and the pictures are to be of the
    // size of the target.
    let idle = keyed(|keyer| {
        keyer.gain = 0.0;
        keyer.spill_method = SpillMethod::None;
    });
    assert!(compositor.key_for_tests(alone, &idle, &target).is_err());
    let short = [Some(&pixels[1..]), None, None, None];
    assert!(compositor
        .key_for_tests(short, &keyed(|_| {}), &target)
        .is_err());
    let none = [None, Some(pixels.as_slice()), None, None];
    assert!(compositor
        .key_for_tests(none, &keyed(|_| {}), &target)
        .is_err());
    assert_no_errors(&gpu);
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "each of the mattes and the mask, alone and together, in the views that they change"
)]
fn the_mattes_and_the_mask_of_a_keyer_are_met_as_the_processor_meets_them() {
    // Each of the colours under each pair of a garbage matte and a solid matte, 16 pixels by
    // 16, given to the card as floats: the mattes and the mask are the alphas of pictures,
    // some of them above 1 and below 0, over channels that are not read.
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let size = (16, 16);
    let target = Target::preview(&gpu, size.0, size.1).unwrap();
    let places = || (0..16_usize).flat_map(|y| (0..16_usize).map(move |x| (x, y)));
    let pixels: Vec<[f32; 4]> = places()
        .map(|(x, _)| {
            let colour = COLOURS[x];
            [colour[0], colour[1], colour[2], 1.0]
        })
        .collect();
    let levels = |of: &dyn Fn(usize, usize) -> f32| -> Vec<[f32; 4]> {
        places().map(|(x, y)| [0.3, 0.2, 0.1, of(x, y)]).collect()
    };
    let garbage = levels(&|_, y| [0.0, 0.25, 1.0, 1.5][y % 4]);
    let solid = levels(&|_, y| [0.0, 0.5, 1.0, -1.0][y / 4]);
    let mask = levels(&|x, y| [0.0, 0.3, 1.0, 2.0][(x + y) % 4]);
    // Which of the garbage matte, the solid matte and the mask come in, whether the garbage
    // matte is read the other way round, and the view.
    let cases: [(&str, [bool; 3], bool, KeyerView); 9] = [
        (
            "a garbage matte",
            [true, false, false],
            false,
            KeyerView::FinalResult,
        ),
        (
            "a garbage matte the other way round",
            [true, false, false],
            true,
            KeyerView::FinalResult,
        ),
        (
            "a solid matte",
            [false, true, false],
            false,
            KeyerView::FinalResult,
        ),
        (
            "both mattes",
            [true, true, false],
            false,
            KeyerView::FinalResult,
        ),
        (
            "an effect mask",
            [false, false, true],
            false,
            KeyerView::FinalResult,
        ),
        (
            "both mattes and a mask",
            [true, true, true],
            true,
            KeyerView::FinalResult,
        ),
        (
            "the status of both mattes",
            [true, true, false],
            false,
            KeyerView::Status,
        ),
        (
            "the status of both mattes, the garbage matte the other way round, and a mask",
            [true, true, true],
            true,
            KeyerView::Status,
        ),
        (
            "the matte, which is before both mattes",
            [true, true, false],
            false,
            KeyerView::Matte,
        ),
    ];
    let plain = [Some(pixels.as_slice()), None, None, None];
    let mut frames = Vec::new();
    for (what, [junk, filled, limited], invert, view) in cases {
        let mut settings = viewed(view);
        settings.garbage_invert = invert;
        let given = [
            Some(pixels.as_slice()),
            junk.then_some(garbage.as_slice()),
            filled.then_some(solid.as_slice()),
            limited.then_some(mask.as_slice()),
        ];
        compositor.key_for_tests(given, &settings, &target).unwrap();
        let shown = target.read_floats().unwrap();
        let incoming = |at: usize| Incoming {
            garbage: junk.then(|| garbage[at][3]),
            solid: filled.then(|| solid[at][3]),
            mask: limited.then(|| mask[at][3]),
        };
        let wanted = keyed_of(&settings, &pixels, size, &incoming);
        assert_keyed(&shown, &wanted, KEYER_TOLERANCE, what);
        // Each does something, but to the view of the matte, which is before the mattes.
        compositor.key_for_tests(plain, &settings, &target).unwrap();
        let alone = target.read_floats().unwrap();
        if view == KeyerView::Matte {
            assert_eq!(shown, alone, "{what}");
        } else {
            assert!(differ(&shown, &alone, 0.05), "{what}");
        }
        frames.push(shown);
    }
    // The garbage matte read the other way round is another picture, and where both mattes
    // are 1 the garbage matte wins: the pixel is nothing. The rows 8 to 11 are under a solid
    // matte of 1, and of them the row 10 under a garbage matte of 1.
    assert!(differ(&frames[0], &frames[1], 0.05));
    let both = frames[3].as_chunks::<4>().0;
    for pixel in &both[10 * 16..11 * 16] {
        assert!(pixel.iter().all(|value| value.abs() < 1e-6), "{pixel:?}");
    }
    for pixel in &both[8 * 16..9 * 16] {
        assert!((pixel[3] - 1.0).abs() <= KEYER_TOLERANCE, "{pixel:?}");
    }
    assert_no_errors(&gpu);
}

/// The colour of the pixel (x, y) of a picture of `size` pixels shot against the screen of
/// these tests, which is lit whole over its left quarter and in shadow towards its right: a
/// subject in its middle with a soft edge, a grey above, a skin tone across its middle and a
/// colour near the screen's below; specks of the subject on the screen, and holes of the
/// screen in the subject.
fn shot(x: u32, y: u32, size: (u32, u32)) -> [f32; 3] {
    let across = (float(x) + 0.5) / float(size.0);
    let down = (float(y) + 0.5) / float(size.1);
    let lit = 1.0 - 0.5 * (across - 0.25).max(0.0);
    let subject = if down < 0.4 {
        [0.18, 0.18, 0.18]
    } else if down < 0.6 {
        [0.45, 0.25, 0.18]
    } else {
        [0.2, 0.42, 0.2]
    };
    let away = (((across - 0.5) / 0.3).powi(2) + ((down - 0.5) / 0.35).powi(2)).sqrt();
    let mut share = ((1.15 - away) / 0.3).clamp(0.0, 1.0);
    if x % 16 == 3 && y % 12 == 5 {
        share = 1.0;
    }
    if x % 16 == 11 && y % 12 == 9 {
        share = 0.0;
    }
    [0, 1, 2].map(|channel| share * subject[channel] + (1.0 - share) * STAGE[channel] * lit)
}

/// The greatest difference between a channel of a pixel of `shown` and that of `wanted`.
fn farthest(shown: &[f32], wanted: &[[f32; 4]]) -> f32 {
    assert_eq!(shown.len(), wanted.len() * 4);
    shown
        .iter()
        .zip(wanted.as_flattened())
        .map(|(shown, wanted)| (shown - wanted).abs())
        .fold(0.0, f32::max)
}

/// The greatest difference between a channel of a pixel of `shown` and that of `wanted`, but
/// for the farthest 1 in 100 of them.
fn nearly(shown: &[f32], wanted: &[[f32; 4]]) -> f32 {
    assert_eq!(shown.len(), wanted.len() * 4);
    let mut offs: Vec<f32> = shown
        .iter()
        .zip(wanted.as_flattened())
        .map(|(shown, wanted)| (shown - wanted).abs())
        .collect();
    offs.sort_by(f32::total_cmp);
    offs[offs.len() - 1 - offs.len() / 100]
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "each control of a keyer that reads the pixels about each one, and all of them"
)]
fn the_controls_of_a_keyer_about_a_pixel_are_those_of_the_processor() {
    // A picture shot against a green screen, 64 pixels by 136, given to the card as floats:
    // its matte through each control that reads the pixels about each one is what the
    // processor makes of it by the filters of the neighbourhoods and the maths of Matte
    // Finesse. In a picture 136 high, 16 is some 2 pixels and 80 some 10, wide enough for
    // the filter to be taken at fewer points, into a texture that is read between its
    // texels.
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let size = (64, 136);
    let target = Target::preview(&gpu, size.0, size.1).unwrap();
    let pixels: Vec<[f32; 4]> = (0..size.1)
        .flat_map(|y| (0..size.0).map(move |x| (x, y)))
        .map(|(x, y)| {
            let colour = shot(x, y, size);
            [colour[0], colour[1], colour[2], 1.0]
        })
        .collect();
    let alone = [Some(pixels.as_slice()), None, None, None];
    let mut drawn = |settings: &DeltaKeyed| {
        compositor.key_for_tests(alone, settings, &target).unwrap();
        target.read_floats().unwrap()
    };
    let matte = |set: fn(&mut DeltaKeyed)| {
        staged(|keyer| {
            keyer.view = KeyerView::Matte;
            set(keyer);
        })
    };
    // The matte of the key alone: within a thousandth, and of every level.
    let plain = drawn(&matte(|_| {}));
    let wanted = keyed_of(&matte(|_| {}), &pixels, size, &|_| Incoming::default());
    assert_keyed(&plain, &wanted, KEYER_TOLERANCE, "the matte of the key");
    let levels = reds(&plain);
    assert!(levels.iter().any(|level| *level <= 0.001));
    assert!(levels.iter().any(|level| *level >= 0.999));
    assert!(levels.iter().any(|level| (0.2..0.8).contains(level)));
    let cases: [KeyerCase; 10] = [
        (
            "a pre-blur",
            |keyer| keyer.pre_blur = 16.0,
            KEYER_FILTERED_TOLERANCE,
        ),
        (
            "a wide pre-blur",
            |keyer| keyer.pre_blur = 40.0,
            KEYER_FILTERED_TOLERANCE,
        ),
        (
            "an erode",
            |keyer| keyer.erode_dilate = -16.0,
            KEYER_MORPHED_TOLERANCE,
        ),
        (
            "a dilate",
            |keyer| keyer.erode_dilate = 16.0,
            KEYER_MORPHED_TOLERANCE,
        ),
        (
            "a wide dilate",
            |keyer| keyer.erode_dilate = 80.0,
            KEYER_MORPHED_TOLERANCE,
        ),
        (
            "a matte blur",
            |keyer| keyer.matte_blur = 16.0,
            KEYER_FILTERED_TOLERANCE,
        ),
        (
            "a wide matte blur",
            |keyer| keyer.matte_blur = 48.0,
            KEYER_FILTERED_TOLERANCE,
        ),
        (
            "a clean foreground",
            |keyer| keyer.clean_foreground = 0.5,
            KEYER_FILTERED_TOLERANCE,
        ),
        (
            "a clean background",
            |keyer| keyer.clean_background = 0.5,
            KEYER_FILTERED_TOLERANCE,
        ),
        (
            "both cleans, with thresholds",
            |keyer| {
                keyer.low = 0.1;
                keyer.high = 0.9;
                keyer.clean_foreground = 0.75;
                keyer.clean_background = 0.5;
            },
            KEYER_FILTERED_TOLERANCE,
        ),
    ];
    for (what, set, tolerance) in cases {
        let settings = matte(set);
        let shown = drawn(&settings);
        let wanted = keyed_of(&settings, &pixels, size, &|_| Incoming::default());
        let off = farthest(&shown, &wanted);
        assert!(off <= tolerance, "{what}: {off} off");
        // The control does something.
        assert!(differ(&shown, &plain, 0.05), "{what}");
    }
    // All of them, one after another, each on what the one before made, which was a step of
    // a half float or two off: the matte, but for the farthest 1 in 100 of its values.
    let settings = matte(|keyer| {
        keyer.pre_blur = 12.0;
        keyer.erode_dilate = -12.0;
        keyer.matte_blur = 16.0;
        keyer.clean_foreground = 0.5;
        keyer.clean_background = 0.5;
    });
    let shown = drawn(&settings);
    let wanted = keyed_of(&settings, &pixels, size, &|_| Incoming::default());
    let off = nearly(&shown, &wanted);
    assert!(off <= KEYER_MORPHED_TOLERANCE, "all of them: {off} off");
    assert!(differ(&shown, &plain, 0.05));
    // The final result of a keyer with a Pre-Blur and a Matte Blur: its colours are those of
    // the picture as it is, with the matte that the controls made, but for the few pixels
    // whose pre-matte is all but 1 and less than their matte, where what the matte gives
    // back is a share of next to nothing.
    let settings = staged(|keyer| {
        keyer.pre_blur = 16.0;
        keyer.matte_blur = 16.0;
    });
    let shown = drawn(&settings);
    let wanted = keyed_of(&settings, &pixels, size, &|_| Incoming::default());
    let off = nearly(&shown, &wanted);
    assert!(
        off <= KEYER_FILTERED_TOLERANCE,
        "the final result: {off} off"
    );
    assert_no_errors(&gpu);
}

/// A picture of R'G'B'A of `width` × `height`, opaque, shot against the screen of these
/// tests ([`shot`]).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "channels from 0 to 1, as words of 16 bits"
)]
fn shot_picture(width: u32, height: u32) -> Arc<NativePicture> {
    let format = PictureFormat::Rgba64;
    let mut luma = Vec::new();
    for y in 0..height {
        for x in 0..width {
            for channel in shot(x, y, (width, height)) {
                let word = (channel.clamp(0.0, 1.0) * 65535.0).round() as u16;
                luma.extend(word.to_le_bytes());
            }
            luma.extend(u16::MAX.to_le_bytes());
        }
    }
    Arc::new(NativePicture {
        frame: 0,
        width,
        height,
        format,
        bits: 16,
        matrix: YuvMatrix::Bt709,
        range: ColorRange::Full,
        chroma_location: 0,
        alpha: Alpha::Opaque,
        luma,
        chroma: Vec::new(),
        luma_stride: width as usize * format.pixel_bytes(),
        chroma_stride: 0,
        ..NativePicture::default()
    })
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "a keyer in a composition, alone and with each of its mattes and its mask"
)]
fn a_keyer_of_a_composition_keys_the_picture_of_its_clip_with_its_mattes_and_its_mask() {
    // The picture of a clip shot against a green screen, keyed in a frame that is not
    // square, over black and over white, which shows its alpha: what the processor makes of
    // the frame of the clip, and of the frames of the masks that are its mattes.
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let size = MASK;
    let picture = Source::Picture(shot_picture(size.0, size.1), 0);
    let under = [None, Some([1.0; 4])];
    let [black, white] = under.map(|under| {
        let clip = fused_of(picture.clone(), vec![FusedKind::MediaIn], Some(0), size);
        frame_of(&gpu, &mut compositor, under, clip, size)
    });
    let pixels = pixels_over(&black, &white);
    assert!(pixels.iter().all(|pixel| (pixel[3] - 1.0).abs() < 1e-3));
    // The masks: a Rectangle over the left of the picture, a soft Ellipse in its middle, and
    // another, lower and to the right. Each alone, its mask the red of its frame over black.
    let masks = vec![
        rectangle(
            masked(),
            RectangleShape {
                center: [0.3, 0.5],
                width: 0.4,
                height: 0.8,
                ..rectangle_shape()
            },
        ),
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
        ellipse(
            Masked {
                soft_edge: 0.1,
                ..masked()
            },
            EllipseShape {
                center: [0.6, 0.4],
                ..ellipse_shape()
            },
        ),
    ];
    let levels: Vec<Vec<f32>> = masks
        .iter()
        .map(|node| {
            let clip = clip_of(vec![node.clone()], 0, size);
            reds(&frame_of(&gpu, &mut compositor, None, clip, size))
        })
        .collect();
    // Which of the garbage matte (the Rectangle), the solid matte (the first Ellipse) and the
    // mask (the second) the keyer has, whether its garbage matte is read the other way
    // round, and its view.
    let cases: [(&str, [bool; 3], bool, KeyerView); 7] = [
        ("a keyer alone", [false; 3], false, KeyerView::FinalResult),
        (
            "a garbage matte",
            [true, false, false],
            false,
            KeyerView::FinalResult,
        ),
        (
            "a garbage matte the other way round",
            [true, false, false],
            true,
            KeyerView::FinalResult,
        ),
        (
            "a solid matte",
            [false, true, false],
            false,
            KeyerView::FinalResult,
        ),
        (
            "both mattes",
            [true, true, false],
            false,
            KeyerView::FinalResult,
        ),
        (
            "both mattes and a mask",
            [true, true, true],
            false,
            KeyerView::FinalResult,
        ),
        (
            "the matte, with both mattes and a mask",
            [true, true, true],
            false,
            KeyerView::Matte,
        ),
    ];
    let mut alone = Vec::new();
    for (what, linked, invert, view) in cases {
        let settings = staged(|keyer| {
            keyer.low = 0.1;
            keyer.high = 0.9;
            keyer.garbage_invert = invert;
            keyer.view = view;
        });
        let [garbage_matte, solid_matte, mask] = [0, 1, 2].map(|at| linked[at].then_some(at));
        let mut nodes = masks.clone();
        nodes.push(FusedKind::MediaIn);
        nodes.push(FusedKind::DeltaKeyer {
            input: Some(3),
            garbage_matte,
            solid_matte,
            mask,
            settings,
        });
        let [dark, light] = under.map(|under| {
            let clip = fused_of(picture.clone(), nodes.clone(), Some(4), size);
            frame_of(&gpu, &mut compositor, under, clip, size)
        });
        let incoming = |at: usize| Incoming {
            garbage: linked[0].then(|| levels[0][at]),
            solid: linked[1].then(|| levels[1][at]),
            mask: linked[2].then(|| levels[2][at]),
        };
        let wanted = keyed_of(&settings, &pixels, size, &incoming);
        assert_over(&dark, &light, &wanted, KEYED_TOLERANCE, what);
        if alone.is_empty() {
            // The keyer does something: the screen is gone, and the frame over white is not
            // the frame over black.
            assert!(differ(&dark, &black, 0.05), "{what}");
            assert!(differ(&dark, &light, 0.05), "{what}");
            alone = dark;
        } else {
            // And so does what comes into it.
            assert!(differ(&dark, &alone, 0.05), "{what}");
        }
    }
    assert_no_errors(&gpu);
}

/// A picture of colours that needs no source: a soft white Ellipse over a Background of pure
/// green, the screen of a keyer as it is at first, which keys it to the mask of the Ellipse.
/// The picture is that of the node at 2.
fn green_over() -> Vec<FusedKind> {
    let mut nodes = soft_over();
    nodes[0] = FusedKind::Background {
        rgba: [0.0, 1.0, 0.0, 1.0],
        mask: None,
    };
    nodes
}

#[test]
fn a_keyer_reads_the_same_share_of_the_picture_about_a_pixel_at_whatever_size_it_is_drawn() {
    // The controls that read the pixels about each one, through the textures of a
    // composition and the passes of a frame: the matte of the picture as it is drawn, in a
    // frame of its size and in one of half its size, is what the processor makes of the
    // frame of the picture there. 70 is some 2 pixels of a picture 32 high, and 4 of one 64
    // high.
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let sizes: [((u32, u32), (u32, u32)); 3] = [
        ((48, 32), (48, 32)),
        ((96, 64), (48, 32)),
        ((96, 64), (96, 64)),
    ];
    let cases: [KeyerCase; 4] = [
        (
            "a pre-blur",
            |keyer| keyer.pre_blur = 70.0,
            KEYER_FILTERED_TOLERANCE,
        ),
        (
            "a matte blur",
            |keyer| keyer.matte_blur = 70.0,
            KEYER_FILTERED_TOLERANCE,
        ),
        (
            "an erode",
            |keyer| keyer.erode_dilate = -70.0,
            KEYER_MORPHED_TOLERANCE,
        ),
        (
            "a clean background after a matte blur",
            |keyer| {
                keyer.matte_blur = 70.0;
                keyer.clean_background = 0.5;
            },
            KEYER_FILTERED_TOLERANCE,
        ),
    ];
    for (nominal, seen) in sizes {
        let clip = clip_of(green_over(), 2, nominal);
        let input = pixels_of(&frame_of(&gpu, &mut compositor, None, clip, seen));
        let mut matte = |set: fn(&mut DeltaKeyed)| {
            let settings = keyed(|keyer| {
                keyer.view = KeyerView::Matte;
                set(keyer);
            });
            let mut nodes = green_over();
            nodes.push(key(2, settings));
            let clip = clip_of(nodes, 3, nominal);
            let shown = frame_of(&gpu, &mut compositor, None, clip, seen);
            (settings, shown)
        };
        let (_, plain) = matte(|_| {});
        for (what, set, tolerance) in cases {
            let what = format!("{what}, of {nominal:?} in a frame of {seen:?}");
            let (settings, shown) = matte(set);
            let wanted = keyed_of(&settings, &input, seen, &|_| Incoming::default());
            let off = farthest(&shown, &wanted);
            assert!(off <= tolerance, "{what}: {off} off");
            assert!(differ(&shown, &plain, 0.02), "{what}");
        }
    }
    assert_no_errors(&gpu);
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "the passes of each keyer, one after another"
)]
fn a_keyer_that_changes_nothing_is_no_pass_and_its_mask_keeps_its_ends_to_the_bit() {
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
    let source = drawn(&mut compositor, vec![FusedKind::MediaIn], 0);
    assert_eq!(compositor.fusion_passes(), 0);
    // A keyer that keys nothing and suppresses no spill, with any control that reads the
    // pixels about each one: the clip as it is, to the bit, and no pass.
    let idle = keyed(|keyer| {
        keyer.gain = 0.0;
        keyer.spill_method = SpillMethod::None;
        keyer.pre_blur = 40.0;
        keyer.matte_blur = 40.0;
    });
    let shown = drawn(&mut compositor, vec![FusedKind::MediaIn, key(0, idle)], 1);
    assert_eq!(shown, source);
    assert_eq!(compositor.fusion_passes(), 0);
    // A composition without a keyer is drawn by the passes that it was drawn by, the source
    // alone and two flips; and with such a keyer after them by the same passes and to the
    // bit.
    let (nodes, _) = there_and_back();
    let twice = drawn(&mut compositor, nodes.clone(), 2);
    assert_eq!(compositor.fusion_passes(), 3);
    let mut longer = nodes;
    longer.push(key(2, idle));
    assert_eq!(drawn(&mut compositor, longer, 3), twice);
    assert_eq!(compositor.fusion_passes(), 6);
    // A keyer as it is at first: the source alone and the pass that keys it.
    let first = keyed(|_| {});
    let once = drawn(&mut compositor, vec![FusedKind::MediaIn, key(0, first)], 1);
    assert_ne!(once, source);
    assert_eq!(compositor.fusion_passes(), 8);
    // A Rectangle as large as the picture: a mask of 1 all over, and inverted one of 0.
    let limited = |invert: bool| {
        let whole = rectangle(
            Masked { invert, ..masked() },
            RectangleShape {
                width: 1.0,
                height: 1.0,
                ..rectangle_shape()
            },
        );
        vec![
            whole,
            FusedKind::MediaIn,
            with_mask(&key(1, first), Some(0)),
        ]
    };
    // Limited by a mask of 1: one pass more, that of the mask, and the same picture to the
    // bit.
    let all = drawn(&mut compositor, limited(false), 2);
    assert_eq!(all, once);
    assert_eq!(compositor.fusion_passes(), 11);
    // Limited by a mask of 0: the picture as it came in, which went through a texture of the
    // composition, and is within a step of the source.
    let none = drawn(&mut compositor, limited(true), 2);
    assert_within(&none, &source, 1, "a mask of 0");
    assert_eq!(compositor.fusion_passes(), 14);
    // With every control that reads the pixels about each one: the source alone; the two
    // passes of the Pre-Blur; the matte of the key; three passes for each of Erode/Dilate,
    // the Matte Blur and the Cleans; and the pass that makes the picture.
    let every = keyed(|keyer| {
        keyer.pre_blur = 300.0;
        keyer.erode_dilate = 200.0;
        keyer.matte_blur = 300.0;
        keyer.clean_foreground = 0.5;
    });
    let through = drawn(&mut compositor, vec![FusedKind::MediaIn, key(0, every)], 1);
    assert_ne!(through, once);
    assert_eq!(compositor.fusion_passes(), 28);
    assert_no_errors(&gpu);
}

#[test]
fn a_composition_with_a_keyer_is_shown_as_its_source_until_its_pipeline_is_made() {
    let Some(gpu) = gpu_for_tests() else { return };
    let picture = Source::Picture(pattern(), 0);
    let under = colour([0.2, 0.5, 0.3, 1.0]);
    let plain = scene(vec![
        under.clone(),
        layer(picture.clone(), ColorRange::Full),
    ]);
    // The picture of the clip keyed, with a Matte Blur: the passes of a keyer, by a pipeline
    // of its own, and those of a filter of the neighbourhoods, whose pipeline is made at
    // once.
    let settings = keyed(|keyer| keyer.matte_blur = 300.0);
    let nodes = vec![FusedKind::MediaIn, key(0, settings)];
    let cut = scene(vec![
        under,
        layer(fused(picture, nodes, Some(1)), ColorRange::Full),
    ]);
    // A compositor that waits for its pipelines draws each frame as its scene says.
    let mut waiting = Compositor::new(Arc::clone(&gpu));
    let source = halves(&gpu, &mut waiting, &plain);
    let pipelines = waiting.pipelines_made();
    let wanted = halves(&gpu, &mut waiting, &cut);
    assert_ne!(source, wanted);
    assert!(!waiting.is_provisional());
    assert_eq!(waiting.pipelines_made(), pipelines + 1);
    // The source alone; the matte of the key, the two passes of its filter and the matte
    // drawn anew; and the pass that makes the picture.
    assert_eq!(waiting.fusion_passes(), 6);
    // One that does not shows the clip without its composition at first, and says so.
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    compositor.compile_in_background(|| {});
    assert_eq!(halves(&gpu, &mut compositor, &cut), source);
    assert!(compositor.is_provisional());
    assert_eq!(compositor.fusion_passes(), 0);
    // Once the pipeline is made, it draws the frame as it is.
    compositor.finish_compiling();
    assert_eq!(halves(&gpu, &mut compositor, &cut), wanted);
    assert!(!compositor.is_provisional());
    assert_eq!(compositor.fusion_passes(), 6);
    assert_no_errors(&gpu);
}

/// What a viewer of one node of a composition shows: the composition made for that node, its
/// `nodes` those that the node depends on and `output` the node, of pictures of `size`
/// pixels, on a layer that is not moved, whole, opaque and not graded, alone in a frame of
/// `canvas` pixels.
fn viewer(
    source: Source,
    (nodes, output): (Vec<FusedKind>, Option<usize>),
    size: (u32, u32),
    canvas: (u32, u32),
) -> Scene {
    let clip = fused_of(source, nodes, output, size);
    scene_of(canvas.0, canvas.1, vec![layer(clip, ColorRange::Full)])
}

/// R, G, B and A of the pixel (x, y) of a frame `width` pixels wide.
fn float_pixel(frame: &[f32], width: u32, x: u32, y: u32) -> [f32; 4] {
    let at = ((y * width + x) * 4) as usize;
    [frame[at], frame[at + 1], frame[at + 2], frame[at + 3]]
}

#[test]
fn a_node_viewed_alone_is_drawn_by_the_passes_that_lead_to_it_and_none_after_it() {
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let picture = Source::Picture(pattern(), 0);
    let own = (WIDTH, HEIGHT);
    let plain = scene(vec![layer(picture.clone(), ColorRange::Full)]);
    let source = floats(&gpu, &mut compositor, &plain);
    // The picture of the clip over a Background by a Blend of a half, and that flipped.
    let whole = vec![
        FusedKind::MediaIn,
        FusedKind::Background {
            rgba: [0.3, 0.1, 0.2, 1.0],
            mask: None,
        },
        FusedKind::Merge {
            background: Some(1),
            foreground: Some(0),
            mask: None,
            settings: Merged {
                blend: 0.5,
                ..merge()
            },
        },
        flip(2),
    ];
    let passes = compositor.fusion_passes();
    let flipped = viewer(picture.clone(), (whole.clone(), Some(3)), own, own);
    let flipped = floats(&gpu, &mut compositor, &flipped);
    // The source alone, the Background, the Merge and the Transform.
    assert_eq!(compositor.fusion_passes(), passes + 4);
    // The Merge viewed, the composition made for it: what it reads, and itself. Its picture
    // is half the picture of the clip and half the Background, and not the Transform's.
    let merged = viewer(picture.clone(), (whole[..3].to_vec(), Some(2)), own, own);
    let middle = floats(&gpu, &mut compositor, &merged);
    assert_eq!(compositor.fusion_passes(), passes + 7);
    for (at, (shown, source)) in middle
        .as_chunks::<4>()
        .0
        .iter()
        .zip(source.as_chunks::<4>().0)
        .enumerate()
    {
        for (channel, behind) in [0.3_f32, 0.1, 0.2].into_iter().enumerate() {
            let wanted = 0.5 * source[channel] + 0.5 * behind;
            assert!(
                (shown[channel] - wanted).abs() <= 2e-3,
                "pixel {at}: {shown:?}, not {wanted} of {source:?}"
            );
        }
    }
    assert!(differ(&middle, &flipped, 1e-2));
    // With the nodes after it left in, it is the same picture to the bit, by the same
    // passes: a pass that nothing reads is not drawn.
    let before = halves(&gpu, &mut compositor, &merged);
    let kept = viewer(picture.clone(), (whole.clone(), Some(2)), own, own);
    assert_eq!(halves(&gpu, &mut compositor, &kept), before);
    assert_eq!(compositor.fusion_passes(), passes + 13);
    // The Background viewed: its colour over the whole picture, by its one pass.
    let backdrop = viewer(picture.clone(), (vec![whole[1].clone()], Some(0)), own, own);
    let shown = floats(&gpu, &mut compositor, &backdrop);
    for (at, pixel) in shown.as_chunks::<4>().0.iter().enumerate() {
        for (channel, wanted) in [0.3_f32, 0.1, 0.2, 1.0].into_iter().enumerate() {
            assert!(
                (pixel[channel] - wanted).abs() <= 1e-3,
                "pixel {at}: {pixel:?}"
            );
        }
    }
    assert_eq!(compositor.fusion_passes(), passes + 14);
    // The MediaIn viewed is the source of the clip, to the bit, and no pass.
    let media_in = viewer(
        picture.clone(),
        (vec![FusedKind::MediaIn], Some(0)),
        own,
        own,
    );
    assert_eq!(
        halves(&gpu, &mut compositor, &media_in),
        halves(&gpu, &mut compositor, &plain)
    );
    // And a node that gives no picture shows the black of the frame.
    let none = viewer(picture, (vec![FusedKind::MediaIn], None), own, own);
    assert_eq!(
        halves(&gpu, &mut compositor, &none),
        NOTHING.repeat((WIDTH * HEIGHT) as usize)
    );
    assert_eq!(compositor.fusion_passes(), passes + 14);
    assert_no_errors(&gpu);
}

#[test]
fn a_node_viewed_alone_is_fitted_whole_into_the_frame_over_black() {
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let rgba = [0.2, 0.4, 0.6, 1.0];
    // A Background, which covers the picture of its composition: the pixels that are not
    // black are the picture, and they lie where the fitting says, whatever the two shapes.
    let cases: [FittedCase; 4] = [
        (
            "a square in a wide frame",
            (32, 32),
            (64, 36),
            [14, 0, 36, 36],
        ),
        (
            "a wide one in a frame less wide",
            (64, 16),
            (64, 36),
            [0, 10, 64, 16],
        ),
        (
            "a wide one in a frame that stands",
            (32, 16),
            (36, 64),
            [0, 23, 36, 18],
        ),
        (
            "one of the shape of the frame",
            (32, 18),
            (64, 36),
            [0, 0, 64, 36],
        ),
    ];
    for (what, size, canvas, lies) in cases {
        let told = columbina_compose::geometry::fitted(
            (f64::from(canvas.0), f64::from(canvas.1)),
            (f64::from(size.0), f64::from(size.1)),
        );
        assert_eq!(told, Some(lies.map(f64::from)), "{what}");
        let alone = viewer(
            Source::Color([1.0, 0.0, 1.0, 1.0]),
            (vec![FusedKind::Background { rgba, mask: None }], Some(0)),
            size,
            canvas,
        );
        let shown = floats(&gpu, &mut compositor, &alone);
        for y in 0..canvas.1 {
            for x in 0..canvas.0 {
                let inside = (lies[0]..lies[0] + lies[2]).contains(&x)
                    && (lies[1]..lies[1] + lies[3]).contains(&y);
                let wanted: [f32; 4] = if inside {
                    [0.2, 0.4, 0.6, 1.0]
                } else {
                    [0.0, 0.0, 0.0, 1.0]
                };
                let pixel = float_pixel(&shown, canvas.0, x, y);
                for channel in 0..4 {
                    assert!(
                        (pixel[channel] - wanted[channel]).abs() <= 1e-3,
                        "{what}: ({x}, {y}) is {pixel:?}, not {wanted:?}"
                    );
                }
            }
        }
    }
    // The MediaIn of a picture twice as wide as high, in a frame of 16:9: the layer of the
    // clip as it is fitted, to the bit, black above and below it.
    let picture = Source::Picture(pattern(), 0);
    let plain = scene_of(64, 36, vec![layer(picture.clone(), ColorRange::Full)]);
    let media_in = viewer(
        picture,
        (vec![FusedKind::MediaIn], Some(0)),
        (WIDTH, HEIGHT),
        (64, 36),
    );
    let shown = halves(&gpu, &mut compositor, &media_in);
    assert_eq!(shown, halves(&gpu, &mut compositor, &plain));
    assert_eq!(
        columbina_compose::geometry::fitted((64.0, 36.0), (16.0, 8.0)),
        Some([0.0, 2.0, 64.0, 32.0])
    );
    let row = 64 * 4;
    let black = NOTHING.repeat(2 * 64);
    assert_eq!(&shown[..2 * row], &black[..]);
    assert_eq!(&shown[34 * row..], &black[..]);
    assert_ne!(&shown[2 * row..2 * row + 4], &NOTHING[..]);
    assert_no_errors(&gpu);
}

#[test]
fn a_mask_node_viewed_alone_is_white_by_its_mask_where_its_picture_lies() {
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    // A soft Ellipse of a square composition, in a frame twice as wide: the picture lies in
    // its middle, a pixel of it to a pixel of the frame.
    let (side, canvas) = (32_u32, (64_u32, 32_u32));
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
    let wanted = mask::mask(side, side, &shape, &common, None);
    assert_eq!(
        columbina_compose::geometry::fitted((64.0, 32.0), (32.0, 32.0)),
        Some([16.0, 0.0, 32.0, 32.0])
    );
    let alone = viewer(
        Source::Color([1.0, 0.0, 1.0, 1.0]),
        (vec![soft], Some(0)),
        (side, side),
        canvas,
    );
    let shown = floats(&gpu, &mut compositor, &alone);
    let mut most = 0.0_f32;
    for y in 0..canvas.1 {
        for x in 0..canvas.0 {
            let pixel = float_pixel(&shown, canvas.0, x, y);
            let mask = if (16..48).contains(&x) {
                wanted[(y * side + x - 16) as usize]
            } else {
                0.0
            };
            most = most.max(mask);
            for value in &pixel[..3] {
                assert!(
                    (value - mask).abs() <= MASK_TOLERANCE,
                    "({x}, {y}) is {pixel:?}, not {mask}"
                );
            }
        }
    }
    // The mask is somewhere full: the picture is not black all over.
    assert!(most > 0.9, "{most}");
    assert_eq!(compositor.fusion_passes(), 1);
    assert_no_errors(&gpu);
}

// ---------------------------------------------------------------------------------------------
// The Tracker
// ---------------------------------------------------------------------------------------------

/// The pixel of the picture of the tests that the Trackers of the tests follow: its column
/// and its row.
const MARK: (i32, i32) = (6, 3);

/// How far the picture that is tracked has gone at each frame of the tests of a Tracker, in
/// whole pixels to the right and down: nowhere at frame 0, the reference frame.
const SHIFTS: [(i32, i32); 4] = [(0, 0), (2, 1), (-3, 0), (4, -2)];

/// A pixel of a preview that is white and opaque, in half floats.
const WHITE: [u16; 4] = [0x3C00; 4];

/// The place, as the settings of a composition count one, of the point `x` pixels from the
/// left edge and `y` pixels from the top of a picture of the tests: a pixel reaches from its
/// number to the next, so that its middle is half a pixel on.
fn place(x: f64, y: f64) -> [f64; 2] {
    [x / f64::from(WIDTH), 1.0 - y / f64::from(HEIGHT)]
}

/// A Transform of the picture at `input` that puts it `by` pixels to the right and down.
fn shifted(input: usize, by: (i32, i32)) -> FusedKind {
    FusedKind::Transform {
        input: Some(input),
        mask: None,
        settings: Transformed {
            center: place(8.0 + f64::from(by.0), 4.0 + f64::from(by.1)),
            ..transform()
        },
    }
}

/// A Tracker that match moves and draws as `merge` says, whose one pattern follows the
/// middle of the pixel [`MARK`] of a picture that goes as [`SHIFTS`] says, a frame for each:
/// what the tracking of such a picture writes.
fn follower(merge: TrackerMerge) -> Tracker {
    let middle = (f64::from(MARK.0) + 0.5, f64::from(MARK.1) + 0.5);
    let path = (0_i64..)
        .zip(SHIFTS)
        .map(|(frame, (right, down))| {
            let at = place(middle.0 + f64::from(right), middle.1 + f64::from(down));
            (frame, at)
        })
        .collect();
    Tracker {
        operation: TrackerOperation::MatchMove,
        merge,
        trackers: vec![Pattern {
            path,
            ..Pattern::default()
        }],
        ..Tracker::default()
    }
}

/// The Tracker `tracker` as the model has it at `frame`, in pictures of the size of the
/// pictures of the tests: its `input`, its `foreground` and its `mask` reading `reads`.
fn tracked(tracker: &Tracker, frame: u32, reads: [Option<usize>; 3]) -> FusedKind {
    let [input, foreground, mask] = reads;
    FusedKind::Tracker {
        input,
        foreground,
        mask,
        settings: tracker.at(f64::from(frame), (WIDTH, HEIGHT)),
    }
}

/// What a Tracker that match moves and draws as `merge` says draws, the picture that it
/// moves put at `center`, as large as it is and not turned.
fn put_by(merge: TrackerMerge, center: [f64; 2]) -> Tracked {
    Tracked {
        operation: TrackerOperation::MatchMove,
        merge,
        center,
        size: 1.0,
        angle: 0.0,
    }
}

/// A Rectangle mask of two pixels by two whose top left pixel is [`MARK`]: a white block, in
/// a picture that is transparent about it.
fn block() -> FusedKind {
    let middle = (f64::from(MARK.0) + 1.0, f64::from(MARK.1) + 1.0);
    rectangle(
        masked(),
        RectangleShape {
            center: place(middle.0, middle.1),
            width: 2.0 / f64::from(WIDTH),
            height: 2.0 / f64::from(HEIGHT),
            ..rectangle_shape()
        },
    )
}

/// Whether the pixel (x, y) of a frame is of the block of [`block`] as it lies when it has
/// gone `by` pixels to the right and down.
fn on_the_block(x: i64, y: i64, by: (i32, i32)) -> bool {
    let left = i64::from(MARK.0 + by.0);
    let top = i64::from(MARK.1 + by.1);
    (left..left + 2).contains(&x) && (top..top + 2).contains(&y)
}

/// The frame `frame`, of the size of the pictures of the tests, with the pixels (x, y) for
/// which `on` holds made `colour`.
fn painted(frame: &[u16], colour: [u16; 4], on: impl Fn(i64, i64) -> bool) -> Vec<u16> {
    let width = i64::from(WIDTH);
    let mut out = Vec::with_capacity(frame.len());
    for (at, pixel) in (0_i64..).zip(frame.as_chunks::<4>().0) {
        let shown = if on(at % width, at / width) {
            colour
        } else {
            *pixel
        };
        out.extend(shown);
    }
    out
}

#[test]
fn a_tracker_holds_still_the_picture_that_it_was_tracked_in() {
    // A picture that goes by whole pixels, frame after frame, and a Tracker whose path
    // follows a mark of it: held still, each frame is the picture where it was at the
    // reference frame, to the pixel.
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let picture = Source::Picture(pattern(), 0);
    let plain = halves(
        &gpu,
        &mut compositor,
        &scene(vec![layer(picture.clone(), ColorRange::Full)]),
    );
    let (width, height) = (i64::from(WIDTH), i64::from(HEIGHT));
    let tracker = follower(TrackerMerge::BgOnly);
    for (frame, by) in (0_u32..).zip(SHIFTS) {
        let nodes = vec![
            FusedKind::MediaIn,
            shifted(0, by),
            tracked(&tracker, frame, [Some(1), None, None]),
        ];
        let passes = compositor.fusion_passes();
        let held = layer(fused(picture.clone(), nodes, Some(2)), ColorRange::Full);
        let shown = halves(&gpu, &mut compositor, &scene(vec![held]));
        // Each pixel is the one that it was at the reference frame, of those that the
        // picture as it has gone still has.
        let (right, down) = (i64::from(by.0), i64::from(by.1));
        let wanted = rearranged(&plain, NOTHING, |x, y| {
            let kept = (0..width).contains(&(x + right)) && (0..height).contains(&(y + down));
            kept.then_some((x, y))
        });
        assert_within(&shown, &wanted, 1, &format!("frame {frame}"));
        // The mark is back where it was.
        let mark = (i64::from(MARK.0), i64::from(MARK.1));
        assert_within(
            &pixel(&shown, mark.0, mark.1),
            &pixel(&plain, mark.0, mark.1),
            1,
            &format!("the mark at frame {frame}"),
        );
        // At the reference frame nothing is drawn for it; at another, the source alone, the
        // picture as it has gone, and that held still.
        let drawn = if frame == 0 { 0 } else { 3 };
        assert_eq!(compositor.fusion_passes(), passes + drawn, "frame {frame}");
    }
    assert_no_errors(&gpu);
}

#[test]
fn a_tracker_makes_its_foreground_follow_what_it_tracked() {
    // A white block that lies on the mark at the reference frame is laid, by a Tracker
    // whose path follows the mark, on the mark where it has gone.
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let picture = Source::Picture(pattern(), 0);
    let plain = halves(
        &gpu,
        &mut compositor,
        &scene(vec![layer(picture.clone(), ColorRange::Full)]),
    );
    let nothing = NOTHING.repeat((WIDTH * HEIGHT) as usize);
    for (frame, by) in (0_u32..).zip(SHIFTS) {
        let mut drawn = |nodes: Vec<FusedKind>| {
            let clip = layer(fused(picture.clone(), nodes, Some(3)), ColorRange::Full);
            halves(&gpu, &mut compositor, &scene(vec![clip]))
        };
        let (right, down) = (i64::from(by.0), i64::from(by.1));
        // The picture as it has gone, and the block on its mark.
        let gone = rearranged(&plain, NOTHING, |x, y| Some((x - right, y - down)));
        let on_the_mark = painted(&gone, WHITE, |x, y| on_the_block(x, y, by));
        // The picture is what was tracked, and the block the foreground: over the picture,
        // and alone.
        let over = |merge: TrackerMerge| {
            vec![
                FusedKind::MediaIn,
                shifted(0, by),
                block(),
                tracked(&follower(merge), frame, [Some(1), Some(2), None]),
            ]
        };
        let shown = drawn(over(TrackerMerge::FgOverBg));
        assert_within(&shown, &on_the_mark, 1, &format!("over, frame {frame}"));
        let shown = drawn(over(TrackerMerge::FgOnly));
        let alone = painted(&nothing, WHITE, |x, y| on_the_block(x, y, by));
        assert_within(&shown, &alone, 1, &format!("alone, frame {frame}"));
        // The block is what was tracked, and the picture the foreground: the block as it
        // has gone lies over the picture, which follows it; and held still, the block is
        // where it was at the reference frame.
        let under = |merge: TrackerMerge| {
            vec![
                FusedKind::MediaIn,
                block(),
                shifted(1, by),
                tracked(&follower(merge), frame, [Some(2), Some(0), None]),
            ]
        };
        let shown = drawn(under(TrackerMerge::BgOverFg));
        assert_within(&shown, &on_the_mark, 1, &format!("under, frame {frame}"));
        let shown = drawn(under(TrackerMerge::BgOnly));
        let back = painted(&nothing, WHITE, |x, y| on_the_block(x, y, (0, 0)));
        assert_within(&shown, &back, 1, &format!("held, frame {frame}"));
    }
    assert_no_errors(&gpu);
}

#[test]
fn a_tracker_of_two_patterns_brings_back_a_picture_that_was_turned() {
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let picture = Source::Picture(pattern(), 0);
    let plain = halves(
        &gpu,
        &mut compositor,
        &scene(vec![layer(picture.clone(), ColorRange::Full)]),
    );
    // The picture turned a quarter counter-clockwise about its middle, (8, 4): the point
    // (x, y) of it is then at (y + 4, 12 - x).
    let turned = FusedKind::Transform {
        input: Some(0),
        mask: None,
        settings: Transformed {
            angle: 90.0,
            ..transform()
        },
    };
    // Two patterns at the middles of two pixels at frame 0, and where the turn has put them
    // at frame 1.
    let trackers = [(6.5, 2.5), (9.5, 5.5)]
        .into_iter()
        .map(|(x, y)| Pattern {
            path: [(0, place(x, y)), (1, place(y + 4.0, 12.0 - x))]
                .into_iter()
                .collect(),
            ..Pattern::default()
        })
        .collect();
    let tracker = Tracker {
        trackers,
        ..follower(TrackerMerge::BgOnly)
    };
    // The motion is a quarter turn about the middle, with no shift and no size: held still,
    // the picture is turned a quarter the other way.
    let FusedKind::Tracker { settings, .. } = tracked(&tracker, 1, [Some(1), None, None]) else {
        panic!("a tracker")
    };
    assert!(
        (settings.angle + 90.0).abs() < 1e-9 && (settings.size - 1.0).abs() < 1e-9,
        "{settings:?}"
    );
    let off = [settings.center[0] - 0.5, settings.center[1] - 0.5];
    assert!(off[0].abs() < 1e-9 && off[1].abs() < 1e-9, "{settings:?}");
    let nodes = vec![
        FusedKind::MediaIn,
        turned,
        tracked(&tracker, 1, [Some(1), None, None]),
    ];
    let held = layer(fused(picture, nodes, Some(2)), ColorRange::Full);
    let shown = halves(&gpu, &mut compositor, &scene(vec![held]));
    // Brought back, each pixel is where it was, of those that the turned picture still had:
    // the columns from 4 to 11. The turn that the model found is a quarter but for the
    // rounding of its fit, so that a value may be a step further off than one that is read
    // at the middle of a pixel.
    let wanted = rearranged(&plain, NOTHING, |x, y| {
        (4..=11).contains(&x).then_some((x, y))
    });
    assert_within(&shown, &wanted, 2, "a quarter turn brought back");
    assert_no_errors(&gpu);
}

#[test]
fn the_effect_mask_of_a_tracker_mixes_what_it_draws_with_the_picture_that_was_tracked() {
    // Each way of a Tracker is drawn without its mask (`own`), with it, and so is the
    // picture of its `input` alone (`first`), over black and over white: the node with its
    // mask is `first + (own - first) * mask` of each of R, G and B of the frames, as of
    // every node that hands a picture on.
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let picture = Source::Picture(pattern(), 0);
    let size = (WIDTH, HEIGHT);
    // The picture that was tracked: that of the clip at half its size, transparent about
    // it. The foreground is the opaque Background.
    let small = FusedKind::Transform {
        input: Some(1),
        mask: None,
        settings: Transformed {
            size: 0.5,
            ..transform()
        },
    };
    let alone = fused(picture.clone(), limiting(), Some(0));
    let mask: Vec<f32> = frame_of(&gpu, &mut compositor, None, alone, size)
        .as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| pixel[0])
        .collect();
    assert!(mask.iter().any(|value| *value >= 0.999));
    assert!(mask.iter().any(|value| *value <= 0.001));
    for under in [None, Some([1.0; 4])] {
        for merge in [
            TrackerMerge::BgOnly,
            TrackerMerge::FgOnly,
            TrackerMerge::FgOverBg,
            TrackerMerge::BgOverFg,
        ] {
            // The picture that is moved is put two pixels to the right and one down.
            let moves = FusedKind::Tracker {
                input: Some(3),
                foreground: Some(2),
                mask: None,
                settings: put_by(merge, place(10.0, 5.0)),
            };
            let mut frame = |mask: Option<Option<usize>>, output: usize| {
                let mut nodes = limiting();
                nodes.push(small.clone());
                if let Some(mask) = mask {
                    nodes.push(with_mask(&moves, mask));
                }
                let clip = fused(picture.clone(), nodes, Some(output));
                frame_of(&gpu, &mut compositor, under, clip, size)
            };
            let own = frame(Some(None), 4);
            let limited = frame(Some(Some(0)), 4);
            let first = frame(None, 3);
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
                        "{merge:?}, over {under:?}: pixel {at} is {limited:?}, not {wanted} \
                         of {own:?} and {first:?} by {mask}"
                    );
                }
            }
            // The node draws something of its own, and the mask does something to it.
            for (what, other) in [("the node", &first), ("the mask", &limited)] {
                assert!(
                    own.iter()
                        .zip(other)
                        .any(|(own, other)| (own - other).abs() > 0.05),
                    "{what}: {merge:?}, over {under:?}"
                );
            }
        }
    }
    assert_no_errors(&gpu);
}
