//! The Text+ of a composition drawn on the card (`columbina_compose::fusion`, Text+): its picture
//! is what the processor draws, over nothing; its effect mask limits it to nothing where the
//! mask is 0; one that does not change is drawn on the processor once; and one that writes
//! nothing is no picture and no pass. Without an adapter the tests are skipped, unless
//! `COLUMBINA_REQUIRE_GPU` is 1.

use std::sync::Arc;

use columbina_compose::{
    card_for_tests, ColorRange, Composite, Compositor, Crop, Fusion, Gpu, LayerColor, Scene,
    SceneLayer, Source, Target, Transform, YuvCoding, YuvMatrix,
};
use columbina_model::fusion::{
    Color, Fused, FusedKind, FusedNode, Masked, PaintMode, RectangleShape, TextBorder, TextPlus,
    TextPlused,
};
use columbina_model::grade::Param;

/// The size of the picture of the tests: letters of a quarter of its height are 8 pixels high.
const WIDTH: u32 = 64;
const HEIGHT: u32 = 32;

/// A pixel of a preview over which nothing is drawn, in half floats: black, and opaque.
const NOTHING: [u16; 4] = [0, 0, 0, 0x3C00];

/// A scene of the size of the picture of the tests, of the one layer of `source`, as it is
/// fitted, opaque, whole and laid as it is.
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

/// A composition of `nodes`, the `MediaOut` reading the one at `output`, of a clip whose source
/// is a colour that no node reads.
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
        source: Source::Color([0.0, 0.0, 0.0, 0.0]),
        composition: Fused {
            nodes,
            output: Some(output),
        },
        size: (WIDTH, HEIGHT),
    }))
}

/// A Text+ that says `text`, its em a quarter of the height of the picture, white over a green
/// border that reaches a quarter of the height beyond the cells of its glyphs.
fn boxed(text: &str) -> TextPlused {
    let fixed = Param::Fixed;
    TextPlus {
        text: text.into(),
        size: fixed(0.25),
        border: Some(TextBorder {
            color: Color {
                r: fixed(0.0),
                g: fixed(1.0),
                b: fixed(0.0),
                a: fixed(1.0),
            },
            extend: fixed(0.25),
            round: fixed(0.0),
        }),
        ..TextPlus::default()
    }
    .at(0.0)
}

/// A Text+ node of `settings`, its effect mask the node at `mask`.
fn lettered(settings: TextPlused, mask: Option<usize>) -> FusedKind {
    FusedKind::TextPlus {
        mask,
        settings: Box::new(settings),
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
fn a_text_plus_is_its_picture_over_nothing_and_its_effect_mask_limits_it() {
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let background = FusedKind::Background {
        rgba: [0.0, 1.0, 0.0, 1.0],
        mask: None,
    };
    let green = halves(&gpu, &mut compositor, &scene(composed(vec![background], 0)));
    let shown = halves(
        &gpu,
        &mut compositor,
        &scene(composed(vec![lettered(boxed("I"), None)], 0)),
    );
    // The centre is the middle of the picture; the line of the letter is ten pixels high, from
    // row 11, and the border reaches eight beyond it. Above the letter, within the border, the
    // border alone, as a Background of its colour is.
    for (x, y) in [(32, 5), (32, 7), (32, 26)] {
        let (seen, wanted) = (pixel(&shown, x, y), pixel(&green, x, y));
        assert!(
            alike(seen, wanted),
            "({x}, {y}): {seen:04x?}, not {wanted:04x?}"
        );
    }
    // Beyond the border, nothing: what is under the layer.
    for (x, y) in [(2, 16), (61, 2), (32, 0), (32, 31)] {
        assert_eq!(pixel(&shown, x, y), NOTHING, "({x}, {y})");
    }
    // Limited by a Rectangle over the left half: nothing right of it, and as it was left of it.
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
    let masked = halves(
        &gpu,
        &mut compositor,
        &scene(composed(vec![left, lettered(boxed("I"), Some(0))], 1)),
    );
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let seen = pixel(&masked, x, y);
            if x >= WIDTH / 2 {
                assert_eq!(seen, NOTHING, "({x}, {y})");
            } else {
                let unmasked = pixel(&shown, x, y);
                assert!(alike(seen, unmasked), "({x}, {y}): {seen:04x?}");
            }
        }
    }
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

#[test]
fn a_text_plus_that_does_not_change_is_drawn_on_the_processor_once() {
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let still = scene(composed(vec![lettered(boxed("Hi"), None)], 0));
    let first = halves(&gpu, &mut compositor, &still);
    let drawn = compositor.texts_drawn();
    assert_eq!(drawn, 1);
    // Drawn again, it is the same picture, from the card.
    assert_eq!(halves(&gpu, &mut compositor, &still), first);
    assert_eq!(compositor.texts_drawn(), drawn);
    // Another text is another picture.
    let other = scene(composed(vec![lettered(boxed("Ho"), None)], 0));
    assert_ne!(halves(&gpu, &mut compositor, &other), first);
    assert_eq!(compositor.texts_drawn(), drawn + 1);
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

#[test]
fn a_text_plus_that_writes_nothing_is_no_picture_and_no_pass() {
    let Some(gpu) = card_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let mut unwritten = boxed("I");
    unwritten.written = [1, 1];
    let shown = halves(
        &gpu,
        &mut compositor,
        &scene(composed(vec![lettered(unwritten, None)], 0)),
    );
    assert!(shown.chunks(4).all(|pixel| pixel == NOTHING));
    assert_eq!(compositor.fusion_passes(), 0);
    assert_eq!(compositor.texts_drawn(), 0);
    assert_eq!(gpu.uncaught_errors(), (0, None));
}
