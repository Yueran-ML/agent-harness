//! The stabilizer of a clip drawn on the card (`columbina_compose::fusion`, The stabilizer):
//! what comes out of the composition of the clip (its source, where it has none of its own)
//! put where the matrix of the stabilizer says, before the layer of the clip is laid on the
//! canvas. A shift by whole pixels moves the picture by those pixels exactly, and what it is
//! not moved over is transparent; it moves what a composition made of the source, not the
//! source before the composition; an enlargement about the middle is the Transform of that
//! size, to the bit; and a stabilizer that moves nothing draws the layer as one without it, to
//! the bit, by the same passes. Without an adapter the tests are skipped, unless
//! `COLUMBINA_REQUIRE_GPU` is 1.

use std::sync::Arc;

use columbina_compose::{
    card_for_tests, gpu_for_tests, ColorRange, Composite, Compositor, Crop, Fusion, Gpu,
    LayerColor, Scene, SceneLayer, Source, Target, Transform, YuvCoding, YuvMatrix,
};
use columbina_media::{Alpha, NativePicture, PictureFormat};
use columbina_model::fusion::{Fused, FusedKind, FusedNode, Transformed};
use columbina_model::stabilize::{Matrix, IDENTITY};

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

/// A layer of `source` as it is fitted, opaque, whole and laid as it is, its codes of BT.709 in
/// full range.
fn layer(source: Source) -> SceneLayer {
    SceneLayer {
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
    }
}

/// A scene of the size of the picture of the tests, of the one layer of `source`.
fn scene(source: Source) -> Scene {
    Scene {
        width: WIDTH,
        height: HEIGHT,
        layers: vec![layer(source)],
        ..Scene::default()
    }
}

/// The half floats of a preview of `scene`, R, G, B and A of each pixel.
fn halves(gpu: &Arc<Gpu>, compositor: &mut Compositor, scene: &Scene) -> Vec<u16> {
    let target = Target::preview(gpu, scene.width, scene.height).unwrap();
    compositor.render(scene, &target).unwrap();
    target.read_halves().unwrap()
}

/// The picture of the tests in a composition of `nodes`, the `MediaOut` reading the one at
/// `output`.
fn composed(nodes: Vec<FusedKind>, output: usize) -> Source {
    let nodes = nodes
        .into_iter()
        .enumerate()
        .map(|(at, kind)| FusedNode {
            id: format!("n{at}"),
            kind,
        })
        .collect();
    Source::Fused(Box::new(Fusion {
        source: Source::Picture(pattern(), 0),
        composition: Fused {
            nodes,
            output: Some(output),
        },
        size: (WIDTH, HEIGHT),
    }))
}

/// The picture of the tests stabilized by `matrix`, as the model gives a clip without a
/// composition of its own: its `MediaIn`, and the stabilizer after it.
fn stabilized(matrix: Matrix) -> Source {
    stabilized_after(vec![FusedKind::MediaIn], 0, matrix)
}

/// The picture of the tests in a composition of `nodes`, the `MediaOut` reading the one at
/// `output`, stabilized by `matrix` as the model gives it: the stabilizer between that node
/// and the `MediaOut`.
fn stabilized_after(nodes: Vec<FusedKind>, output: usize, matrix: Matrix) -> Source {
    let nodes = nodes
        .into_iter()
        .enumerate()
        .map(|(at, kind)| FusedNode {
            id: format!("n{at}"),
            kind,
        })
        .collect();
    let composition = Fused {
        nodes,
        output: Some(output),
    };
    let steady = composition.stabilized(matrix);
    let nodes = steady.nodes.into_iter().map(|node| node.kind).collect();
    composed(nodes, steady.output.unwrap())
}

/// A composition of the picture of the tests flipped left for right by a Transform: each pixel
/// is exactly that of the other side of the picture.
fn flipped() -> Vec<FusedKind> {
    vec![
        FusedKind::MediaIn,
        FusedKind::Transform {
            input: Some(0),
            mask: None,
            settings: Transformed {
                center: [0.5, 0.5],
                size: 1.0,
                angle: 0.0,
                flip_horizontal: true,
                flip_vertical: false,
            },
        },
    ]
}

/// A shift by `x` and `y` pixels of the picture of the tests, as a matrix of shares of it.
fn shift(x: f64, y: f64) -> Matrix {
    [
        [1.0, 0.0, x / f64::from(WIDTH)],
        [0.0, 1.0, y / f64::from(HEIGHT)],
        [0.0, 0.0, 1.0],
    ]
}

/// The pixel at (x, y) of half floats of the size of the scene, R, G, B and A.
fn pixel(values: &[u16], x: u32, y: u32) -> [u16; 4] {
    let at = 4 * (y * WIDTH + x) as usize;
    [values[at], values[at + 1], values[at + 2], values[at + 3]]
}

#[test]
fn a_stabilizer_that_moves_nothing_draws_the_layer_as_one_without_it_to_the_bit() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let wanted = halves(&gpu, &mut compositor, &scene(Source::Picture(pattern(), 0)));
    let shown = halves(&gpu, &mut compositor, &scene(stabilized(IDENTITY)));
    assert_eq!(shown, wanted);
    // Nothing is put after the `MediaIn`, and nothing is drawn into a texture.
    assert_eq!(compositor.fusion_passes(), 0);
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

#[test]
fn a_shift_by_whole_pixels_moves_the_picture_by_those_pixels_and_uncovers_nothing_else() {
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let wanted = halves(&gpu, &mut compositor, &scene(Source::Picture(pattern(), 0)));
    for (dx, dy) in [(2_u32, 0_u32), (0, 1), (3, 2)] {
        let passes = compositor.fusion_passes();
        let shown = halves(
            &gpu,
            &mut compositor,
            &scene(stabilized(shift(f64::from(dx), f64::from(dy)))),
        );
        // The source drawn alone, and the pass of the stabilizer.
        assert_eq!(compositor.fusion_passes(), passes + 2);
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let seen = pixel(&shown, x, y);
                if x < dx || y < dy {
                    // What the picture is not moved over shows what is under the layer.
                    assert_eq!(seen, NOTHING, "({x}, {y}) of a shift of ({dx}, {dy})");
                    continue;
                }
                // Each value goes through one half float more than that of the layer drawn at
                // once, and is within a step of it.
                let source = pixel(&wanted, x - dx, y - dy);
                for (channel, (seen, source)) in seen.iter().zip(source).enumerate() {
                    assert!(
                        seen.abs_diff(source) <= 1,
                        "({x}, {y}) of a shift of ({dx}, {dy}), channel {channel}: \
                         {seen:#06x}, not {source:#06x}"
                    );
                }
            }
        }
    }
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

#[test]
fn the_stabilizer_moves_what_the_composition_made_of_the_source() {
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    // The picture flipped by its composition, not stabilized.
    let wanted = halves(&gpu, &mut compositor, &scene(composed(flipped(), 1)));
    for (dx, dy) in [(2_u32, 0_u32), (3, 1)] {
        let passes = compositor.fusion_passes();
        let matrix = shift(f64::from(dx), f64::from(dy));
        let shown = halves(
            &gpu,
            &mut compositor,
            &scene(stabilized_after(flipped(), 1, matrix)),
        );
        // The source drawn alone, the pass of the Transform, and that of the stabilizer.
        assert_eq!(compositor.fusion_passes(), passes + 3);
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let seen = pixel(&shown, x, y);
                // The flipped picture is moved right and down: what it is not moved over is at
                // the left and the top. (Stabilized before the flip, the source would be moved
                // right and then flipped, and that would be at the right.)
                if x < dx || y < dy {
                    assert_eq!(seen, NOTHING, "({x}, {y}) of a shift of ({dx}, {dy})");
                    continue;
                }
                let made = pixel(&wanted, x - dx, y - dy);
                for (channel, (seen, made)) in seen.iter().zip(made).enumerate() {
                    assert!(
                        seen.abs_diff(made) <= 1,
                        "({x}, {y}) of a shift of ({dx}, {dy}), channel {channel}: \
                         {seen:#06x}, not {made:#06x}"
                    );
                }
            }
        }
    }
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

#[test]
fn an_enlargement_about_the_middle_is_the_transform_of_that_size_to_the_bit() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let transform = Transformed {
        center: [0.5, 0.5],
        size: 2.0,
        angle: 0.0,
        flip_horizontal: false,
        flip_vertical: false,
    };
    let wanted = halves(
        &gpu,
        &mut compositor,
        &scene(composed(
            vec![
                FusedKind::MediaIn,
                FusedKind::Transform {
                    input: Some(0),
                    mask: None,
                    settings: transform,
                },
            ],
            1,
        )),
    );
    let twice = [[2.0, 0.0, -0.5], [0.0, 2.0, -0.5], [0.0, 0.0, 1.0]];
    let shown = halves(&gpu, &mut compositor, &scene(stabilized(twice)));
    assert_eq!(shown, wanted);
    // The middle of the picture stays where it was, and its corners are cut away: every pixel
    // is of the picture.
    assert!(shown
        .as_chunks::<4>()
        .0
        .iter()
        .all(|pixel| *pixel != NOTHING));
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

#[test]
fn a_picture_put_in_perspective_is_drawn_and_reads_where_the_matrix_puts_it() {
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let wanted = halves(&gpu, &mut compositor, &scene(Source::Picture(pattern(), 0)));
    // A shift of 2 pixels right, with a little perspective across: a point of the picture at
    // share u across and v down goes to (u + 2/16, v) divided by (1 + k u). Over the pixels of
    // the left half the divisor is so near 1 that the picture there is within a few steps of
    // the shift alone.
    let k = 1e-4;
    let matrix = [[1.0, 0.0, 2.0 / 16.0], [0.0, 1.0, 0.0], [k, 0.0, 1.0]];
    let shown = halves(&gpu, &mut compositor, &scene(stabilized(matrix)));
    for y in 0..HEIGHT {
        for x in 0..2 {
            assert_eq!(pixel(&shown, x, y), NOTHING, "({x}, {y})");
        }
        for x in 2..8 {
            let (seen, source) = (pixel(&shown, x, y), pixel(&wanted, x - 2, y));
            for (seen, source) in seen.iter().zip(source) {
                assert!(
                    seen.abs_diff(source) <= 16,
                    "({x}, {y}): {seen:#06x}, not {source:#06x}"
                );
            }
        }
    }
    assert_eq!(gpu.uncaught_errors(), (0, None));
}
