//! Layers of two pictures of one media each moved along the flows between them before they are
//! mixed: what a clip that is retimed by Optical Flow shows at a frame whose place in its source
//! lies between two pictures. The pictures are of a smooth texture of grey, moved or made
//! brighter, so that the picture that should lie between them is known. Without an adapter the
//! tests are skipped, unless `COLUMBINA_REQUIRE_GPU` is 1; the one that holds what is drawn to
//! that picture is skipped on the software adapter too.

use std::sync::Arc;

use columbina_compose::flow::{Pair, KEPT};
use columbina_compose::{
    card_for_tests, gpu_for_tests, ColorRange, Composite, Compositor, Crop, Gpu, LayerColor, Scene,
    SceneLayer, Source, Target, Transform, YuvCoding, YuvMatrix,
};
use columbina_media::{Alpha, NativePicture, PictureFormat};

const WIDTH: u32 = 256;
const HEIGHT: u32 = 192;

/// How far from the edges the pixels that are held to what should be drawn lie, at least: what
/// comes in from beyond an edge is in neither picture.
const INSIDE: u32 = 24;

/// A number from -1 to 1 for the corner `(i, j)` of the cells of noise `seed`, the same each
/// time.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "corners of cells of a few pixels, and the bits of a hash"
)]
fn corner(i: f64, j: f64, seed: u64) -> f64 {
    let mut hash = (i as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (j as i64 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ seed.wrapping_mul(0x1656_67B1_9E37_79F9);
    hash ^= hash >> 31;
    hash = hash.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    hash ^= hash >> 29;
    (hash >> 11) as f64 / (1_u64 << 53) as f64 * 2.0 - 1.0
}

/// Noise `seed` at `(x, y)`, in cells: the numbers of the corners of the cell about it, read
/// between them by smoothstep, from -1 to 1.
fn noise(x: f64, y: f64, seed: u64) -> f64 {
    let (left, up) = (x.floor(), y.floor());
    let smooth = |t: f64| t * t * (3.0 - 2.0 * t);
    let (across, down) = (smooth(x - left), smooth(y - up));
    let top = corner(left, up, seed) * (1.0 - across) + corner(left + 1.0, up, seed) * across;
    let bottom =
        corner(left, up + 1.0, seed) * (1.0 - across) + corner(left + 1.0, up + 1.0, seed) * across;
    top * (1.0 - down) + bottom * down
}

/// The texture at `(x, y)`, in pixels: grey from about 0.17 to 0.83, of noise in cells of 32,
/// 16 and 8 pixels, the larger the stronger.
fn texture(x: f64, y: f64) -> f64 {
    let octaves = [(32.0, 0.18, 1), (16.0, 0.10, 2), (8.0, 0.05, 3)];
    0.5 + octaves
        .into_iter()
        .map(|(cell, strength, seed)| strength * noise(x / cell, y / cell, seed))
        .sum::<f64>()
}

/// An opaque picture of R'G'B'A of 16 bits of [`WIDTH`] × [`HEIGHT`]: the texture moved `by`
/// pixels across and down, and `brighter` brighter.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "words from 0 to 65535"
)]
fn picture(by: (f64, f64), brighter: f64) -> Arc<NativePicture> {
    let mut luma = Vec::with_capacity((WIDTH * HEIGHT * 8) as usize);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let grey = texture(f64::from(x) - by.0, f64::from(y) - by.1) + brighter;
            let word = (grey.clamp(0.0, 1.0) * 65535.0).round() as u16;
            for value in [word, word, word, u16::MAX] {
                luma.extend_from_slice(&value.to_le_bytes());
            }
        }
    }
    Arc::new(NativePicture {
        frame: 0,
        width: WIDTH,
        height: HEIGHT,
        format: PictureFormat::Rgba64,
        bits: 16,
        matrix: YuvMatrix::Bt709,
        range: ColorRange::Full,
        chroma_location: 0,
        alpha: Alpha::Opaque,
        luma,
        chroma: Vec::new(),
        luma_stride: WIDTH as usize * PictureFormat::Rgba64.pixel_bytes(),
        chroma_stride: 0,
        ..NativePicture::default()
    })
}

/// What pictures `number` and `number + 1` of the texture are known by.
fn pair(number: i64) -> Pair {
    Pair {
        media: "the texture".into(),
        first: number,
        second: number + 1,
    }
}

/// `weight` of the way from `first` to `second`, moved along the flows between them where they
/// are known by `flow`, and else mixed as they are.
fn between(
    first: &Arc<NativePicture>,
    second: &Arc<NativePicture>,
    weight: f32,
    flow: Option<Pair>,
) -> Source {
    Source::Between {
        first: Arc::clone(first),
        second: Arc::clone(second),
        weight,
        rotation: 0,
        flow,
    }
}

/// A scene of the size of the pictures, of a layer of `source` as it is fitted, opaque, whole,
/// laid as it is, read as R'G'B'A of full range.
fn scene(source: Source) -> Scene {
    Scene {
        width: WIDTH,
        height: HEIGHT,
        layers: vec![SceneLayer {
            source,
            transform: Transform::NONE,
            opacity: 1.0,
            color: LayerColor {
                coding: YuvCoding {
                    matrix: YuvMatrix::Bt709,
                    range: ColorRange::Full,
                },
                ..LayerColor::default()
            },
            crop: Crop::NONE,
            composite: Composite::Normal,
        }],
        ..Scene::default()
    }
}

/// A preview of a layer of `source`: R'G'B'A of 8 bits.
fn drawn(gpu: &Arc<Gpu>, compositor: &mut Compositor, source: Source) -> Vec<u8> {
    let target = Target::preview(gpu, WIDTH, HEIGHT).unwrap();
    compositor.render(&scene(source), &target).unwrap();
    target.read_rgba().unwrap()
}

/// A preview of a layer of `source`: its half floats as they are.
fn halves(gpu: &Arc<Gpu>, compositor: &mut Compositor, source: Source) -> Vec<u16> {
    let target = Target::preview(gpu, WIDTH, HEIGHT).unwrap();
    compositor.render(&scene(source), &target).unwrap();
    target.read_halves().unwrap()
}

/// How far the red of the preview `shown` is from that of `wanted`, in codes of 8 bits, over the
/// pixels [`INSIDE`] or more from the edges: on average, and the share of them more than 8 codes
/// off.
fn off(shown: &[u8], wanted: &[u8]) -> (f64, f64) {
    let (mut sum, mut far, mut count) = (0.0, 0_u32, 0_u32);
    for y in INSIDE..HEIGHT - INSIDE {
        for x in INSIDE..WIDTH - INSIDE {
            let at = ((y * WIDTH + x) * 4) as usize;
            let apart = shown[at].abs_diff(wanted[at]);
            sum += f64::from(apart);
            far += u32::from(apart > 8);
            count += 1;
        }
    }
    (sum / f64::from(count), f64::from(far) / f64::from(count))
}

fn assert_no_errors(gpu: &Gpu) {
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

#[test]
fn a_picture_that_moves_is_shown_where_it_is_between_its_two_pictures() {
    // The texture moved by (6, -4) pixels from the first picture to the second: halfway it is
    // moved by (3, -2), and a quarter of the way by (1.5, -1). Optical Flow shows it there, to
    // 2 codes of 8 bits on average over the inside of the frame, with no more than 2% of its
    // pixels more than 8 codes off; Frame Blend, which shows the two over each other, is
    // three times as far off at least.
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let first = picture((0.0, 0.0), 0.0);
    let second = picture((6.0, -4.0), 0.0);
    for (weight, by) in [(0.5, (3.0, -2.0)), (0.25, (1.5, -1.0))] {
        let wanted = drawn(&gpu, &mut compositor, Source::Picture(picture(by, 0.0), 0));
        let flowed = drawn(
            &gpu,
            &mut compositor,
            between(&first, &second, weight, Some(pair(0))),
        );
        let mixed = drawn(
            &gpu,
            &mut compositor,
            between(&first, &second, weight, None),
        );
        assert_eq!(compositor.flow_failure(), None);
        let (flow_off, flow_far) = off(&flowed, &wanted);
        let (mix_off, _) = off(&mixed, &wanted);
        assert!(
            flow_off <= 2.0 && flow_far <= 0.02,
            "{weight} of the way: {flow_off} codes off on average, and {flow_far} of the pixels \
             more than 8 codes off"
        );
        assert!(
            mix_off >= 3.0 * flow_off.max(0.5),
            "{weight} of the way: Frame Blend is {mix_off} codes off, Optical Flow {flow_off}"
        );
    }
    // Both frames lie between the same two pictures: their flows are found once.
    assert_eq!(compositor.flows_computed(), 1);
    assert_no_errors(&gpu);
}

#[test]
fn a_picture_between_itself_and_itself_is_that_picture() {
    // The flows between two pictures that are the same are none: each is read where it is, and
    // the two mixed are the picture, to a step of a half float.
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let (one, again) = (picture((0.0, 0.0), 0.0), picture((0.0, 0.0), 0.0));
    let alone = halves(&gpu, &mut compositor, Source::Picture(Arc::clone(&one), 0));
    for weight in [0.5, 0.25] {
        let flowed = halves(
            &gpu,
            &mut compositor,
            between(&one, &again, weight, Some(pair(0))),
        );
        assert_eq!(flowed.len(), alone.len());
        for (at, (shown, wanted)) in flowed.iter().zip(&alone).enumerate() {
            assert!(
                shown.abs_diff(*wanted) <= 1,
                "{weight}: value {at} is {shown:#06x}, not {wanted:#06x}"
            );
        }
    }
    assert_eq!(compositor.flow_failure(), None);
    assert_eq!(compositor.flows_computed(), 1);
    assert_no_errors(&gpu);
}

#[test]
fn a_change_of_brightness_alone_moves_nothing_and_is_mixed_as_frame_blend_mixes_it() {
    // The second picture is the first a tenth brighter: nothing moves between them, and what is
    // shown between them is what Frame Blend shows, to a code of 8 bits, at every pixel.
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let (darker, brighter) = (picture((0.0, 0.0), 0.0), picture((0.0, 0.0), 0.1));
    let alone = drawn(
        &gpu,
        &mut compositor,
        Source::Picture(Arc::clone(&darker), 0),
    );
    for weight in [0.5, 0.25] {
        let mixed = drawn(
            &gpu,
            &mut compositor,
            between(&darker, &brighter, weight, None),
        );
        assert_ne!(mixed, alone, "{weight}: the frame lies between the two");
        let flowed = drawn(
            &gpu,
            &mut compositor,
            between(&darker, &brighter, weight, Some(pair(0))),
        );
        for (at, (shown, wanted)) in flowed.iter().zip(&mixed).enumerate() {
            assert!(
                shown.abs_diff(*wanted) <= 1,
                "{weight}: value {at} is {shown}, not {wanted} as Frame Blend shows it"
            );
        }
    }
    assert_eq!(compositor.flow_failure(), None);
    assert_no_errors(&gpu);
}

#[test]
fn the_flows_between_two_pictures_are_found_once_and_kept_for_a_few_pairs() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let pictures = [(0.0, 0.0), (6.0, -4.0), (12.0, -8.0)].map(|by| picture(by, 0.0));
    let found = |compositor: &mut Compositor, at: usize, weight: f32, flow: Option<Pair>| {
        let source = between(&pictures[at], &pictures[at + 1], weight, flow);
        drawn(&gpu, compositor, source);
        compositor.flows_computed()
    };
    // Three frames between the first two pictures, as of a clip at a quarter of its speed:
    // their flows are found at the first of them.
    for weight in [0.25, 0.5, 0.75] {
        assert_eq!(
            found(&mut compositor, 0, weight, Some(pair(0))),
            1,
            "{weight}"
        );
    }
    // Between the next two: found once more; and between the first two again, which are kept,
    // not again.
    assert_eq!(found(&mut compositor, 1, 0.5, Some(pair(1))), 2);
    assert_eq!(found(&mut compositor, 0, 0.5, Some(pair(0))), 2);
    // All of one picture, and two mixed as they are, find none.
    assert_eq!(found(&mut compositor, 0, 0.0, Some(pair(7))), 2);
    assert_eq!(found(&mut compositor, 0, 1.0, Some(pair(7))), 2);
    assert_eq!(found(&mut compositor, 0, 0.5, None), 2);
    // A few pairs are kept, the last shown: as many others let go of the first two, whose flows
    // are then found again.
    let kept = i64::try_from(KEPT).unwrap();
    for number in 10..10 + kept {
        found(&mut compositor, 0, 0.5, Some(pair(number)));
    }
    assert_eq!(compositor.flows_computed(), 2 + KEPT as u64);
    assert_eq!(
        found(&mut compositor, 0, 0.5, Some(pair(0))),
        3 + KEPT as u64
    );
    assert_eq!(compositor.flow_failure(), None);
    assert_no_errors(&gpu);
}
