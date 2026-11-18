//! The Planar Tracker of a composition drawn on the card (`columbina_compose::fusion`, The
//! Planar Tracker): Steady puts the picture that was tracked back where the plane was at the
//! reference frame, a shift of whole pixels to the pixel; Corner Pin lays its foreground with its
//! corners on the corners moved by the path; its effect mask limits it to its picture as it came
//! in; and one that moves nothing draws the layer as one without it, to the bit, and no pass.
//! Without an adapter the tests are skipped, unless `COLUMBINA_REQUIRE_GPU` is 1.

use std::sync::Arc;

use columbina_compose::{
    card_for_tests, gpu_for_tests, ColorRange, Composite, Compositor, Crop, Fusion, Gpu,
    LayerColor, Scene, SceneLayer, Source, Target, Transform, YuvCoding, YuvMatrix,
};
use columbina_media::{Alpha, NativePicture, PictureFormat};
use columbina_model::fusion::{
    Corners, Fused, FusedKind, FusedNode, Homography, Masked, PaintMode, Place, PlanarOperation,
    PlanarTracked, PlanarTracker, RectangleShape, STILL,
};
use columbina_model::stabilize::IDENTITY;

/// The size of the picture of the tests: a share of it that is a whole number of pixels is a
/// binary fraction, which floats hold exactly.
const WIDTH: u32 = 32;
const HEIGHT: u32 = 32;

/// A pixel of a preview over which nothing is drawn, in half floats: black, and opaque.
const NOTHING: [u16; 4] = [0, 0, 0, 0x3C00];

/// The colour of the foreground that is laid on the plane: green, opaque.
const GREEN: [f64; 4] = [0.0, 1.0, 0.0, 1.0];

/// The words of R', G' and B' of the pixel at (x, y) of the picture of the tests: no two of its
/// pixels are of one colour.
fn words(x: u32, y: u32) -> [u16; 3] {
    [
        4096 + 1500 * x,
        8192 + 1500 * y,
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

/// A scene of the size of the picture of the tests, of the one layer of `source`, as it is
/// fitted, opaque, whole and laid as it is, its codes of BT.709 in full range.
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

/// The picture of the tests into a Planar Tracker that draws `settings`, its foreground a green
/// Background and its effect mask the node at `mask`, if any, among `more`, which come before
/// it after the `MediaIn` and the Background.
fn tracked(settings: PlanarTracked, mask: Option<usize>, more: Vec<FusedKind>) -> Source {
    let mut nodes = vec![
        FusedKind::MediaIn,
        FusedKind::Background {
            rgba: GREEN,
            mask: None,
        },
    ];
    nodes.extend(more);
    nodes.push(FusedKind::PlanarTracker {
        input: Some(0),
        foreground: Some(1),
        mask,
        settings,
    });
    let output = nodes.len() - 1;
    composed(nodes, output)
}

/// A shift of `x` pixels to the right and `y` down of the picture of the tests, as a path writes
/// it: in shares of the picture from its lower left corner, `y` up.
fn shifted(x: f64, y: f64) -> Homography {
    let mut shift = STILL;
    shift[2] = x / f64::from(WIDTH);
    shift[5] = -y / f64::from(HEIGHT);
    shift
}

/// A Planar Tracker that does `operation`, with the path `path`.
fn planar(operation: PlanarOperation, path: &[(i64, Homography)]) -> PlanarTracker {
    PlanarTracker {
        operation,
        path: path.iter().copied().collect(),
        ..PlanarTracker::default()
    }
}

/// The pixel at (x, y) of half floats of the size of the scene, R, G, B and A.
fn pixel(values: &[u16], x: u32, y: u32) -> [u16; 4] {
    let at = 4 * (y * WIDTH + x) as usize;
    [values[at], values[at + 1], values[at + 2], values[at + 3]]
}

/// Whether two pixels are one but for a step of a half float in each channel.
fn alike(one: [u16; 4], other: [u16; 4]) -> bool {
    one.iter().zip(other).all(|(a, b)| a.abs_diff(b) <= 1)
}

#[test]
fn a_planar_tracker_that_moves_nothing_draws_the_layer_as_one_without_it_to_the_bit() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let wanted = halves(&gpu, &mut compositor, &scene(Source::Picture(pattern(), 0)));
    let shift = planar(
        PlanarOperation::Steady,
        &[(0, STILL), (5, shifted(3.0, 2.0))],
    )
    .at(5.0);
    for settings in [
        // Held still at the reference frame, and with no path.
        planar(
            PlanarOperation::Steady,
            &[(4, STILL), (9, shifted(3.0, 2.0))],
        )
        .at(4.0),
        planar(PlanarOperation::Steady, &[]).at(7.0),
        // With an operation of None, whatever the path.
        PlanarTracked {
            operation: PlanarOperation::None,
            ..shift
        },
    ] {
        assert_eq!(
            settings.matrix == IDENTITY,
            settings.operation != PlanarOperation::None
        );
        let shown = halves(
            &gpu,
            &mut compositor,
            &scene(tracked(settings, None, vec![])),
        );
        assert_eq!(shown, wanted, "{settings:?}");
    }
    // Nothing is drawn into a texture: neither the source, nor the foreground that no pass
    // reads.
    assert_eq!(compositor.fusion_passes(), 0);
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

#[test]
fn steady_puts_each_pixel_of_a_plane_moved_by_whole_pixels_back_where_it_was() {
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let wanted = halves(&gpu, &mut compositor, &scene(Source::Picture(pattern(), 0)));
    for (dx, dy) in [(3_u32, 2_u32), (0, 1), (5, 0)] {
        // The plane has moved right and down since the reference frame, 0: each pixel at the
        // frame is read from where the plane has gone.
        let moved = shifted(f64::from(dx), f64::from(dy));
        let settings = planar(PlanarOperation::Steady, &[(0, STILL), (6, moved)]).at(6.0);
        let passes = compositor.fusion_passes();
        let shown = halves(
            &gpu,
            &mut compositor,
            &scene(tracked(settings, None, vec![])),
        );
        // The source drawn alone, and the pass of the node.
        assert_eq!(compositor.fusion_passes(), passes + 2);
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let seen = pixel(&shown, x, y);
                if x + dx >= WIDTH || y + dy >= HEIGHT {
                    // What the picture is not moved over shows what is under the layer.
                    assert_eq!(seen, NOTHING, "({x}, {y}) of a shift of ({dx}, {dy})");
                    continue;
                }
                let source = pixel(&wanted, x + dx, y + dy);
                assert!(
                    alike(seen, source),
                    "({x}, {y}) of a shift of ({dx}, {dy}): {seen:04x?}, not {source:04x?}"
                );
            }
        }
    }
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

/// The pixel of a picture of the tests in which the point `at`, in pixels from its top left
/// corner, lies.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a point within the picture"
)]
fn under(at: [f64; 2]) -> (u32, u32) {
    (at[0].floor() as u32, at[1].floor() as u32)
}

#[test]
fn a_corner_pin_lays_its_foreground_with_its_corners_on_the_corners_moved_by_the_path() {
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let input = halves(&gpu, &mut compositor, &scene(Source::Picture(pattern(), 0)));
    let background = FusedKind::Background {
        rgba: GREEN,
        mask: None,
    };
    let green = halves(&gpu, &mut compositor, &scene(composed(vec![background], 0)));
    // A quadrilateral in perspective, its corners on the corners of pixels: in pixels from the
    // top left of the picture, the upper left, the upper right, the lower right and the lower
    // left.
    let quad: [[f64; 2]; 4] = [[6.0, 5.0], [26.0, 6.0], [25.0, 27.0], [5.0, 26.0]];
    let place = |[x, y]: [f64; 2]| Place {
        x: x / f64::from(WIDTH),
        y: 1.0 - y / f64::from(HEIGHT),
    };
    let middle = quad.iter().fold([0.0, 0.0], |sum, at| {
        [sum[0] + at[0] / 4.0, sum[1] + at[1] / 4.0]
    });
    // Without a path, and moved three pixels right and two down by it.
    for (dx, dy) in [(0.0, 0.0), (3.0, 2.0)] {
        let pinned = PlanarTracker {
            corners: Corners {
                tl: place(quad[0]),
                tr: place(quad[1]),
                br: place(quad[2]),
                bl: place(quad[3]),
            },
            ..planar(
                PlanarOperation::CornerPin,
                &[(0, STILL), (4, shifted(dx, dy))],
            )
        };
        let settings = pinned.at(4.0);
        assert_eq!(settings.operation, PlanarOperation::CornerPin);
        let shown = halves(
            &gpu,
            &mut compositor,
            &scene(tracked(settings, None, vec![])),
        );
        for corner in quad {
            // Two pixels from the corner towards the middle of the quadrilateral the foreground
            // is laid, and two pixels away from it the picture is as it was.
            let length = (middle[0] - corner[0]).hypot(middle[1] - corner[1]);
            let towards = [
                (middle[0] - corner[0]) / length,
                (middle[1] - corner[1]) / length,
            ];
            let inside = under([
                corner[0] + dx + 2.0 * towards[0],
                corner[1] + dy + 2.0 * towards[1],
            ]);
            let seen = pixel(&shown, inside.0, inside.1);
            let laid = pixel(&green, inside.0, inside.1);
            assert!(
                alike(seen, laid),
                "{corner:?} moved ({dx}, {dy}), {inside:?}: {seen:04x?}, not {laid:04x?}"
            );
            let outside = under([
                corner[0] + dx - 2.0 * towards[0],
                corner[1] + dy - 2.0 * towards[1],
            ]);
            let seen = pixel(&shown, outside.0, outside.1);
            let was = pixel(&input, outside.0, outside.1);
            assert!(
                alike(seen, was),
                "{corner:?} moved ({dx}, {dy}), {outside:?}: {seen:04x?}, not {was:04x?}"
            );
        }
        // The middle is the foreground, and a corner of the picture is the picture.
        let centre = under([middle[0] + dx, middle[1] + dy]);
        assert!(alike(
            pixel(&shown, centre.0, centre.1),
            pixel(&green, centre.0, centre.1)
        ));
        assert!(alike(pixel(&shown, 0, 0), pixel(&input, 0, 0)));
    }
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

#[test]
fn the_effect_mask_limits_a_planar_tracker_to_its_picture_as_it_came_in() {
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let wanted = halves(&gpu, &mut compositor, &scene(Source::Picture(pattern(), 0)));
    // A Rectangle over the left half of the picture, crisp.
    let left = FusedKind::Rectangle {
        mask: None,
        common: Masked {
            level: 1.0,
            soft_edge: 0.0,
            border_width: 0.0,
            invert: false,
            solid: true,
            paint_mode: PaintMode::Merge,
        },
        shape: RectangleShape {
            center: [0.25, 0.5],
            width: 0.5,
            height: 1.0,
            corner_radius: 0.0,
            angle: 0.0,
        },
    };
    let (dx, dy) = (2_u32, 1_u32);
    let moved = shifted(f64::from(dx), f64::from(dy));
    let settings = planar(PlanarOperation::Steady, &[(0, STILL), (3, moved)]).at(3.0);
    let shown = halves(
        &gpu,
        &mut compositor,
        &scene(tracked(settings, Some(2), vec![left])),
    );
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let seen = pixel(&shown, x, y);
            if x >= WIDTH / 2 {
                // Where the mask is 0, the picture as it came in.
                let source = pixel(&wanted, x, y);
                assert!(alike(seen, source), "({x}, {y}): {seen:04x?}");
            } else if y + dy < HEIGHT {
                // Where it is 1, the picture held still.
                let source = pixel(&wanted, x + dx, y + dy);
                assert!(alike(seen, source), "({x}, {y}): {seen:04x?}");
            } else {
                assert_eq!(seen, NOTHING, "({x}, {y})");
            }
        }
    }
    assert_eq!(gpu.uncaught_errors(), (0, None));
}
