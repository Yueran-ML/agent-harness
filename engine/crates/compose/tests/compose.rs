//! Frames composed on the graphics card and read back. Without an adapter the tests are skipped,
//! unless `COLUMBINA_REQUIRE_GPU` is 1.

use std::sync::Arc;
use std::time::Instant;

use columbina_compose::geometry::picture_stride;
use columbina_compose::{
    gpu_for_tests, ComposeError, Compositor, Gpu, Preference, Scene, SceneLayer, Source, Target,
    Text, Transform,
};
use columbina_media::Picture;
use columbina_model::project::{Rgba, TextAlign, TitleStyle};

/// The codes of BT.709 of limited range of R'G'B' from 0 to 1.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn codes(rgb: [f64; 3], ten: bool) -> [u16; 3] {
    let (kr, kb) = (0.2126, 0.0722);
    let [r, g, b] = rgb;
    let y = kr * r + (1.0 - kr - kb) * g + kb * b;
    let cb = (b - y) / (2.0 * (1.0 - kb));
    let cr = (r - y) / (2.0 * (1.0 - kr));
    let (black, luma_range, middle, chroma_range) = range(ten);
    [
        black + luma_range * y,
        middle + chroma_range * cb,
        middle + chroma_range * cr,
    ]
    .map(|code: f64| code.round() as u16)
}

fn range(ten: bool) -> (f64, f64, f64, f64) {
    if ten {
        (64.0, 876.0, 512.0, 896.0)
    } else {
        (16.0, 219.0, 128.0, 224.0)
    }
}

/// R'G'B' from 0 to 255 of codes of BT.709 of limited range.
fn rgb(codes: [u16; 3], ten: bool) -> [f64; 3] {
    let (black, luma_range, middle, chroma_range) = range(ten);
    let [y, cb, cr] = codes.map(f64::from);
    let y = (y - black) / luma_range;
    let (cb, cr) = ((cb - middle) / chroma_range, (cr - middle) / chroma_range);
    [
        y + 1.5748 * cr,
        y - 0.187_324 * cb - 0.468_124 * cr,
        y + 1.8556 * cb,
    ]
    .map(|value| value.clamp(0.0, 1.0) * 255.0)
}

/// A picture whose pixel at (x, y) has the codes that `pixel` gives; the chroma of each block of
/// 2 × 2 is that of its top left pixel. Rows are longer than they need to be, as a decoder's are.
fn picture(width: u32, height: u32, ten: bool, pixel: impl Fn(u32, u32) -> [u16; 3]) -> Picture {
    let bytes = if ten { 2 } else { 1 };
    let stride = (width as usize * bytes).next_multiple_of(64) + 64;
    let rows = height as usize;
    let mut luma = vec![0xEE; stride * rows];
    let mut chroma = vec![0xEE; stride * rows.div_ceil(2)];
    let put = |plane: &mut Vec<u8>, at: usize, code: u16| {
        if ten {
            plane[at..at + 2].copy_from_slice(&(code << 6).to_le_bytes());
        } else {
            plane[at] = u8::try_from(code).unwrap();
        }
    };
    for y in 0..height {
        for x in 0..width {
            let [luma_code, cb, cr] = pixel(x, y);
            put(
                &mut luma,
                y as usize * stride + x as usize * bytes,
                luma_code,
            );
            if x % 2 == 0 && y % 2 == 0 {
                let at = (y / 2) as usize * stride + (x / 2) as usize * 2 * bytes;
                put(&mut chroma, at, cb);
                put(&mut chroma, at + bytes, cr);
            }
        }
    }
    Picture {
        frame: 0,
        width,
        height,
        bits: if ten { 10 } else { 8 },
        luma,
        chroma,
        luma_stride: stride,
        chroma_stride: stride,
    }
}

/// The pixels of a frame read back, R'G'B'A.
struct Frame {
    width: u32,
    pixels: Vec<u8>,
}

impl Frame {
    fn at(&self, x: u32, y: u32) -> [u8; 4] {
        let at = (y * self.width + x) as usize * 4;
        self.pixels[at..at + 4].try_into().unwrap()
    }
}

fn preview(gpu: &Arc<Gpu>, compositor: &mut Compositor, scene: &Scene) -> Frame {
    let target = Target::preview(gpu, scene.width, scene.height).unwrap();
    compositor.render(scene, &target).unwrap();
    Frame {
        width: scene.width,
        pixels: target.read_rgba().unwrap(),
    }
}

fn layer(source: Source, transform: Transform, opacity: f32) -> SceneLayer {
    SceneLayer {
        source,
        transform,
        opacity,
    }
}

fn scene(width: u32, height: u32, layers: Vec<SceneLayer>) -> Scene {
    Scene {
        width,
        height,
        layers,
        ..Scene::default()
    }
}

/// Check that a pixel is within `most` of a colour of R'G'B' from 0 to 255, and opaque.
#[track_caller]
fn assert_near(pixel: [u8; 4], wanted: [f64; 3], most: f64, what: &str) {
    for channel in 0..3 {
        assert!(
            (f64::from(pixel[channel]) - wanted[channel]).abs() <= most,
            "{what}: {pixel:?}, not {wanted:?}"
        );
    }
    assert_eq!(pixel[3], 255, "{what}: {pixel:?}");
}

fn assert_no_errors(gpu: &Gpu) {
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

#[test]
fn colours_are_laid_over_each_other_as_the_alpha_formula_says() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let colours = [
        ([0.2, 0.4, 0.6, 1.0], 1.0),
        ([1.0, 0.0, 0.0, 0.5], 0.8),
        ([0.0, 1.0, 0.5, 1.0], 0.25),
    ];
    let layers = colours
        .iter()
        .map(|(colour, opacity)| layer(Source::Color(*colour), Transform::NONE, *opacity))
        .collect();
    let frame = preview(&gpu, &mut compositor, &scene(32, 16, layers));
    // Over black: each over what is under it by its alpha, times its opacity.
    let mut wanted = [0.0_f64; 3];
    for (colour, opacity) in colours {
        let alpha = f64::from(colour[3]) * f64::from(opacity);
        for channel in 0..3 {
            wanted[channel] = f64::from(colour[channel]) * alpha + wanted[channel] * (1.0 - alpha);
        }
    }
    let wanted = wanted.map(|value| value * 255.0);
    for (x, y) in [(0, 0), (31, 15), (16, 8), (5, 12)] {
        assert_near(frame.at(x, y), wanted, 1.0, &format!("({x}, {y})"));
    }
    // A colour of the project, half opaque, over the same.
    let half = layer(Source::color(Rgba([0, 0, 255, 128])), Transform::NONE, 1.0);
    let base = layer(Source::Color([1.0, 1.0, 1.0, 1.0]), Transform::NONE, 1.0);
    let frame = preview(&gpu, &mut compositor, &scene(8, 8, vec![base, half]));
    let alpha = 128.0 / 255.0;
    assert_near(
        frame.at(4, 4),
        [255.0 * (1.0 - alpha), 255.0 * (1.0 - alpha), 255.0],
        1.0,
        "half blue over white",
    );
    // Nothing but black, without layers; and a layer of no opacity is not drawn.
    let none = layer(Source::Color([1.0; 4]), Transform::NONE, 0.0);
    let frame = preview(&gpu, &mut compositor, &scene(8, 8, vec![none]));
    assert!(frame.pixels.chunks(4).all(|pixel| pixel == [0, 0, 0, 255]));
    assert_no_errors(&gpu);
}

/// A grey picture of 64 × 36 with a block of another colour over x 16 to 32 and y 8 to 16.
fn block_picture(ten: bool) -> (Picture, [f64; 3], [f64; 3]) {
    let grey = codes([0.5, 0.5, 0.5], ten);
    let block = codes([0.8, 0.2, 0.1], ten);
    let picture = picture(64, 36, ten, |x, y| {
        if (16..32).contains(&x) && (8..16).contains(&y) {
            block
        } else {
            grey
        }
    });
    (picture, rgb(grey, ten), rgb(block, ten))
}

#[test]
fn a_block_of_a_picture_lands_where_the_picture_is_fitted_moved_scaled_and_turned() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    for ten in [false, true] {
        let (picture, grey, block) = block_picture(ten);
        let picture = Arc::new(picture);
        let none = [0.0; 3];
        // Fitted to 128 × 72 (twice as large), made half as large, moved a quarter of the width
        // right: 64 × 36 from (64, 18), the block over x 80 to 96 and y 26 to 34.
        let moved = Transform {
            x: 0.25,
            y: 0.0,
            scale: 0.5,
            rotation: 0.0,
        };
        let frame = preview(
            &gpu,
            &mut compositor,
            &scene(
                128,
                72,
                vec![layer(Source::Picture(Arc::clone(&picture), 0), moved, 1.0)],
            ),
        );
        for (x, y) in [(82, 28), (93, 31), (88, 30)] {
            assert_near(frame.at(x, y), block, 1.5, &format!("block at ({x}, {y})"));
        }
        for (x, y) in [(66, 20), (125, 51), (78, 30), (98, 30), (88, 24), (88, 36)] {
            assert_near(frame.at(x, y), grey, 1.5, &format!("grey at ({x}, {y})"));
        }
        for (x, y) in [(10, 10), (62, 36), (96, 16), (96, 55)] {
            assert_near(frame.at(x, y), none, 0.0, &format!("black at ({x}, {y})"));
        }

        // Turned half round about its middle: the block over x 64 to 96 and y 40 to 56.
        let turned = Transform {
            rotation: 180.0,
            ..Transform::NONE
        };
        let frame = preview(
            &gpu,
            &mut compositor,
            &scene(
                128,
                72,
                vec![layer(Source::Picture(Arc::clone(&picture), 0), turned, 1.0)],
            ),
        );
        assert_near(frame.at(80, 48), block, 1.5, "turned block");
        assert_near(frame.at(40, 24), grey, 1.5, "turned grey");
        assert_near(frame.at(48, 24), grey, 1.5, "where the block was");

        // Turned a quarter by its media, into a canvas that stands: shown 36 × 64, fitted to
        // 72 × 128, the block over x 40 to 56 and y 32 to 64.
        let frame = preview(
            &gpu,
            &mut compositor,
            &scene(
                72,
                128,
                vec![layer(
                    Source::Picture(Arc::clone(&picture), 90),
                    Transform::NONE,
                    1.0,
                )],
            ),
        );
        assert_near(frame.at(48, 48), block, 1.5, "a quarter turn, block");
        assert_near(frame.at(20, 48), grey, 1.5, "a quarter turn, grey");
        assert_near(frame.at(48, 100), grey, 1.5, "a quarter turn, below");
    }
    assert_no_errors(&gpu);
}

#[test]
fn the_edge_of_a_picture_that_falls_between_pixels_is_smoothed() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let white = codes([1.0, 1.0, 1.0], false);
    let picture = Arc::new(picture(16, 16, false, |_, _| white));
    // 16 × 16 made 0.5 + 1/16 as large on a canvas of 16: 9 pixels across, from 3.5 to 12.5.
    let transform = Transform {
        scale: 9.0 / 16.0,
        ..Transform::NONE
    };
    let frame = preview(
        &gpu,
        &mut compositor,
        &scene(
            16,
            16,
            vec![layer(Source::Picture(picture, 0), transform, 1.0)],
        ),
    );
    let at = |x| f64::from(frame.at(x, 8)[0]);
    assert!(at(2) <= 1.0, "{}", at(2));
    assert!((at(3) - 127.5).abs() <= 3.0, "half covered: {}", at(3));
    assert!(at(4) >= 254.0, "{}", at(4));
    assert!((at(12) - 127.5).abs() <= 3.0, "half covered: {}", at(12));
    assert!(at(13) <= 1.0, "{}", at(13));
}

#[test]
fn a_dissolve_half_way_mixes_the_two_pictures_half_and_half() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    for ten in [false, true] {
        let (left, right) = (codes([0.9, 0.3, 0.1], ten), codes([0.1, 0.5, 0.9], ten));
        let a = Arc::new(picture(32, 18, ten, |_, _| left));
        // Of another size: fitted alike.
        let b = Arc::new(picture(64, 36, ten, |_, _| right));
        let frame = preview(
            &gpu,
            &mut compositor,
            &scene(
                64,
                36,
                vec![
                    layer(Source::Picture(a, 0), Transform::NONE, 1.0),
                    layer(Source::Picture(b, 0), Transform::NONE, 0.5),
                ],
            ),
        );
        let (left, right) = (rgb(left, ten), rgb(right, ten));
        let wanted = [0, 1, 2].map(|channel| f64::midpoint(left[channel], right[channel]));
        for (x, y) in [(0, 0), (32, 18), (63, 35)] {
            assert_near(frame.at(x, y), wanted, 1.0, &format!("({x}, {y})"));
        }
    }
    assert_no_errors(&gpu);
}

fn style(size: u32, x: f64, y: f64, align: TextAlign) -> TitleStyle {
    TitleStyle {
        font: String::new(),
        size,
        color: Rgba([255, 255, 255, 255]),
        x,
        y,
        align,
    }
}

/// The smallest rectangle of the pixels of a frame brighter than `above`, as x0, y0, x1, y1
/// (the last ones beyond), and how many there are.
fn lit(frame: &Frame, height: u32, above: u8) -> ([u32; 4], usize) {
    let mut bounds = [u32::MAX, u32::MAX, 0, 0];
    let mut count = 0;
    for y in 0..height {
        for x in 0..frame.width {
            if frame.at(x, y)[..3].iter().any(|value| *value > above) {
                bounds = [
                    bounds[0].min(x),
                    bounds[1].min(y),
                    bounds[2].max(x + 1),
                    bounds[3].max(y + 1),
                ];
                count += 1;
            }
        }
    }
    (bounds, count)
}

#[test]
fn a_title_is_drawn_where_its_style_puts_it() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let title = |text: &str, style| {
        layer(
            Source::Title(Text {
                text: text.into(),
                style,
            }),
            Transform::NONE,
            1.0,
        )
    };
    let started = Instant::now();
    let centred = title("Columbina 你好", style(32, 0.5, 0.5, TextAlign::Center));
    let frame = preview(
        &gpu,
        &mut compositor,
        &scene(320, 180, vec![centred.clone()]),
    );
    eprintln!(
        "the fonts read and the first title drawn in {:?}",
        started.elapsed()
    );
    let ([x0, y0, x1, y1], count) = lit(&frame, 180, 128);
    assert!(count > 200, "{count} pixels lit");
    // About the middle.
    assert!((x0 + x1).abs_diff(320) <= 8, "{x0}..{x1}");
    assert!((y0 + y1).abs_diff(180) <= 16, "{y0}..{y1}");
    assert!(x1 - x0 > 120 && x1 - x0 < 300, "{x0}..{x1}");
    assert!(y1 - y0 > 16 && y1 - y0 < 48, "{y0}..{y1}");
    assert_eq!(frame.at(0, 0), [0, 0, 0, 255]);

    // Its anchor on its left, a tenth from the left and a fifth from the top.
    let left = title("Columbina", style(24, 0.1, 0.2, TextAlign::Left));
    let frame = preview(&gpu, &mut compositor, &scene(320, 180, vec![left]));
    let ([x0, y0, _, y1], _) = lit(&frame, 180, 128);
    assert!(x0.abs_diff(32) <= 4, "{x0}");
    assert!((y0 + y1).abs_diff(72) <= 12, "{y0}..{y1}");
    // On its right.
    let right = title("Columbina", style(24, 0.9, 0.8, TextAlign::Right));
    let frame = preview(&gpu, &mut compositor, &scene(320, 180, vec![right]));
    let ([_, y0, x1, y1], _) = lit(&frame, 180, 128);
    assert!(x1.abs_diff(288) <= 4, "{x1}");
    assert!((y0 + y1).abs_diff(288) <= 12, "{y0}..{y1}");

    // Drawn again from what was kept, and moved by its transform with the rest.
    let started = Instant::now();
    let mut moved = centred;
    moved.transform.x = -0.1;
    let frame = preview(&gpu, &mut compositor, &scene(320, 180, vec![moved]));
    eprintln!("a title kept, drawn again in {:?}", started.elapsed());
    let ([x0, _, x1, _], _) = lit(&frame, 180, 128);
    assert!((x0 + x1).abs_diff(256) <= 8, "{x0}..{x1}");
    assert_no_errors(&gpu);
}

#[test]
fn subtitles_are_drawn_at_the_bottom_of_a_preview_and_not_in_an_export() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let mut subtitled = scene(320, 180, vec![]);
    subtitled.subtitles = vec!["Hello there".into()];
    let frame = preview(&gpu, &mut compositor, &subtitled);
    let ([x0, y0, x1, y1], count) = lit(&frame, 180, 200);
    assert!(count > 20, "{count}");
    // Letters 9 pixels high, their block ending 9 above the bottom.
    assert!((x0 + x1).abs_diff(320) <= 6, "{x0}..{x1}");
    assert!(y0 > 140 && y1 <= 171, "{y0}..{y1}");
    // An outline: dark pixels beside the white ones, where the canvas is black anyway; over a
    // white canvas, the outline shows.
    subtitled.layers = vec![layer(Source::Color([1.0; 4]), Transform::NONE, 1.0)];
    let frame = preview(&gpu, &mut compositor, &subtitled);
    let dark = (140..180)
        .flat_map(|y| (0..320).map(move |x| (x, y)))
        .filter(|(x, y)| frame.at(*x, *y)[0] < 60)
        .count();
    assert!(dark > 30, "{dark} dark pixels of outline");

    // An export of the same draws none: all white.
    let target = Target::export(&gpu, 320, 180, 8).unwrap();
    compositor.render(&subtitled, &target).unwrap();
    let mut picture = Picture::default();
    target.read_picture(&mut picture).unwrap();
    for y in 0..180 {
        for x in 0..320 {
            assert_eq!(picture.luma[y * picture.luma_stride + x], 235, "({x}, {y})");
        }
    }
    assert_no_errors(&gpu);
}

/// Colour bars, a ramp of grey and a block of a title, composed on the card, made NV12 or P010,
/// read back, and made R'G'B' again on the processor: within 2 of 255 of what is composed, away
/// from the edges, where chroma is shared by pixels of two colours.
#[test]
#[allow(clippy::too_many_lines)]
fn a_picture_made_nv12_or_p010_on_the_card_is_what_was_composed() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let bars = [
        [0.75, 0.75, 0.75],
        [0.75, 0.75, 0.0],
        [0.0, 0.75, 0.75],
        [0.0, 0.75, 0.0],
        [0.75, 0.0, 0.75],
        [0.75, 0.0, 0.0],
        [0.0, 0.0, 0.75],
        [0.1, 0.2, 0.3],
    ];
    let (width, height) = (256, 64);
    let source = Arc::new(picture(width, height / 2, false, |x, _| {
        codes(bars[(x / 32) as usize], false)
    }));
    // The bars over the top half (fitted, then moved up a quarter), a ramp of grey below.
    let ramp = Arc::new(picture(width, height / 2, true, |x, _| {
        let level = f64::from(x) / f64::from(width - 1);
        codes([level, level, level], true)
    }));
    let layers = vec![
        layer(
            Source::Picture(Arc::clone(&ramp), 0),
            Transform {
                y: 0.25,
                ..Transform::NONE
            },
            1.0,
        ),
        layer(
            Source::Picture(source, 0),
            Transform {
                y: -0.25,
                ..Transform::NONE
            },
            1.0,
        ),
    ];
    let composed = scene(width, height, layers);
    let wanted = preview(&gpu, &mut compositor, &composed);
    for bits in [8, 10] {
        let ten = bits == 10;
        let target = Target::export(&gpu, width, height, bits).unwrap();
        assert_eq!(target.bits(), Some(bits));
        compositor.render(&composed, &target).unwrap();
        let mut picture = Picture {
            frame: 7,
            ..Picture::default()
        };
        target.read_picture(&mut picture).unwrap();
        assert_eq!(picture.frame, 7);
        assert_eq!(
            (picture.width, picture.height, picture.bits),
            (width, height, bits)
        );
        let stride = picture_stride(width, bits);
        assert_eq!(
            (picture.luma_stride, picture.chroma_stride),
            (stride, stride)
        );
        assert_eq!(picture.luma.len(), stride * height as usize);
        assert_eq!(picture.chroma.len(), stride * (height as usize).div_ceil(2));
        let bytes = if ten { 2 } else { 1 };
        let read = |plane: &[u8], at: usize| {
            if ten {
                let word = u16::from_le_bytes([plane[at], plane[at + 1]]);
                assert_eq!(word & 0x3F, 0, "the low bits of P010 are 0");
                word >> 6
            } else {
                u16::from(plane[at])
            }
        };
        let mut worst = 0.0_f64;
        for y in 0..height as usize {
            for x in 0..width as usize {
                // Away from the edges of the bars, and from the edge between the halves.
                if (x % 32) < 3 || (x % 32) > 28 || (30..34).contains(&y) {
                    continue;
                }
                let luma = read(&picture.luma, y * stride + x * bytes);
                let chroma_at = (y / 2) * stride + (x / 2) * 2 * bytes;
                let cb = read(&picture.chroma, chroma_at);
                let cr = read(&picture.chroma, chroma_at + bytes);
                let made = rgb([luma, cb, cr], ten);
                let composed = wanted.at(u32::try_from(x).unwrap(), u32::try_from(y).unwrap());
                for channel in 0..3 {
                    let off = (made[channel] - f64::from(composed[channel])).abs();
                    worst = worst.max(off);
                    // The ramp changes by a step a pixel: chroma of two pixels, and 8 bits of
                    // the preview, against the canvas of the export.
                    assert!(
                        off <= 2.0,
                        "{bits} bits at ({x}, {y}): {made:?}, not {composed:?}"
                    );
                }
            }
        }
        eprintln!("{bits} bits: at most {worst:.2} of 255 off");
    }
    // Pixels of its padding, beyond the width, are 0; one of an odd width too.
    let target = Target::export(&gpu, 33, 5, 8).unwrap();
    compositor
        .render(
            &scene(
                33,
                5,
                vec![layer(Source::Color([1.0; 4]), Transform::NONE, 1.0)],
            ),
            &target,
        )
        .unwrap();
    let mut picture = Picture::default();
    target.read_picture(&mut picture).unwrap();
    assert_eq!(picture.luma_stride, 64);
    assert_eq!(picture.luma[..33], [235; 33]);
    assert_eq!(picture.luma[33..64], [0; 31]);
    assert_eq!(picture.chroma[..34], [128; 34]);
    assert_eq!(picture.chroma.len(), 64 * 3);
    assert_no_errors(&gpu);
}

#[test]
fn what_cannot_be_composed_is_refused() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let target = Target::preview(&gpu, 16, 16).unwrap();
    let error = compositor
        .render(&scene(8, 8, vec![]), &target)
        .unwrap_err();
    assert_eq!(error.reason(), "target");
    // A picture whose planes are short: the layer, by its number.
    let mut short = picture(16, 8, false, |_, _| [16, 128, 128]);
    short.chroma.truncate(short.chroma_stride);
    let layers = vec![
        layer(Source::Color([1.0; 4]), Transform::NONE, 1.0),
        layer(Source::Picture(Arc::new(short), 0), Transform::NONE, 1.0),
    ];
    let error = compositor
        .render(&scene(16, 16, layers), &target)
        .unwrap_err();
    assert!(
        matches!(error, ComposeError::Layer { layer: 1, .. }),
        "{error}"
    );
    // It goes on after.
    let fine = layer(Source::Color([1.0; 4]), Transform::NONE, 1.0);
    compositor
        .render(&scene(16, 16, vec![fine]), &target)
        .unwrap();
    assert_eq!(target.read_rgba().unwrap()[..4], [255, 255, 255, 255]);
    assert_eq!(
        target
            .read_picture(&mut Picture::default())
            .unwrap_err()
            .reason(),
        "target"
    );
    assert!(Target::export(&gpu, 16, 16, 12).is_err());
    assert!(Target::preview(&gpu, 0, 16).is_err());
    assert!(Target::preview(&gpu, 1 << 20, 16).is_err());
    // A target of another card.
    let other = Gpu::new(Preference::HighPerformance).unwrap();
    let elsewhere = Target::preview(&other, 16, 16).unwrap();
    let error = compositor
        .render(&scene(16, 16, vec![]), &elsewhere)
        .unwrap_err();
    assert_eq!(error.reason(), "target");
    // A card that was lost.
    let mut lost = Compositor::new(Arc::clone(&other));
    other.mark_lost("as a test");
    let error = lost.render(&scene(16, 16, vec![]), &elsewhere).unwrap_err();
    assert_eq!(error, ComposeError::Lost("as a test".into()));
    assert_no_errors(&gpu);
}

#[test]
fn the_texture_of_a_preview_is_one_that_a_view_on_the_same_card_can_draw() {
    // The texture of a preview is the card's, of a format and use that a view on the same card
    // binds to draw it.
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let first = Target::preview(&gpu, 16, 16).unwrap();
    compositor
        .render(
            &scene(
                16,
                16,
                vec![layer(
                    Source::Color([0.0, 1.0, 0.0, 1.0]),
                    Transform::NONE,
                    1.0,
                )],
            ),
            &first,
        )
        .unwrap();
    let texture = Arc::clone(first.texture());
    assert_eq!(texture.format(), wgpu::TextureFormat::Rgba8Unorm);
    assert!(texture
        .usage()
        .contains(wgpu::TextureUsages::TEXTURE_BINDING));
    assert_eq!(texture.size().width, 16);
}

/// How long frames take to compose: run with `--ignored --nocapture`, in release for numbers that
/// mean something.
#[test]
#[ignore = "a measurement, not a test"]
fn how_long_frames_take() {
    let Some(gpu) = gpu_for_tests() else { return };
    eprintln!("on {} ({})", gpu.name(), gpu.backend());
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let wait = |gpu: &Gpu| {
        gpu.device()
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
    };
    for (width, height) in [(1920, 1080), (3840, 2160)] {
        let grey = codes([0.4, 0.5, 0.6], false);
        let full = Arc::new(picture(width, height, false, |x, y| {
            if (x / 64 + y / 64) % 2 == 0 {
                grey
            } else {
                [100, 120, 140]
            }
        }));
        let half = Arc::new(picture(width / 2, height / 2, true, |_, _| [500, 400, 600]));
        let scene = Scene {
            width,
            height,
            layers: vec![
                layer(Source::Picture(Arc::clone(&full), 0), Transform::NONE, 1.0),
                layer(
                    Source::Picture(Arc::clone(&half), 0),
                    Transform {
                        x: 0.2,
                        y: 0.1,
                        scale: 0.5,
                        rotation: 10.0,
                    },
                    0.8,
                ),
                layer(Source::Color([0.0, 0.0, 0.0, 0.3]), Transform::NONE, 1.0),
                layer(
                    Source::Title(Text {
                        text: "Columbina 你好".into(),
                        style: style(height / 10, 0.5, 0.8, TextAlign::Center),
                    }),
                    Transform::NONE,
                    1.0,
                ),
            ],
            ..Scene::default()
        };
        let target = Target::preview(&gpu, width, height).unwrap();
        compositor.render(&scene, &target).unwrap();
        wait(&gpu);
        let frames = 30;
        let started = Instant::now();
        for _ in 0..frames {
            compositor.render(&scene, &target).unwrap();
            wait(&gpu);
        }
        eprintln!(
            "{width}×{height}, two pictures, a colour and a title, into a preview: {:?} a frame",
            started.elapsed() / frames
        );
        for bits in [8, 10] {
            let export = Target::export(&gpu, width, height, bits).unwrap();
            let mut picture = Picture::default();
            compositor.render(&scene, &export).unwrap();
            export.read_picture(&mut picture).unwrap();
            let started = Instant::now();
            let mut reading = std::time::Duration::ZERO;
            for _ in 0..frames {
                compositor.render(&scene, &export).unwrap();
                wait(&gpu);
                let read = Instant::now();
                export.read_picture(&mut picture).unwrap();
                reading += read.elapsed();
            }
            eprintln!(
                "{width}×{height} into an export of {bits} bits, read back: {:?} a frame, of \
                 which {:?} reading back",
                started.elapsed() / frames,
                reading / frames
            );
        }
    }
}

#[test]
fn the_software_adapter_composes_and_converts_alike() {
    // WARP, on Windows, where there is no card: an export goes on there, slowly.
    let gpu = match Gpu::new(Preference::Software) {
        Ok(gpu) => gpu,
        Err(error) if cfg!(windows) => panic!("no WARP: {error}"),
        Err(error) => {
            eprintln!("skipped: no software adapter: {error}");
            return;
        }
    };
    assert!(gpu.is_software(), "{gpu:?}");
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let (picture, grey, block) = block_picture(true);
    let layers = vec![layer(
        Source::Picture(Arc::new(picture), 0),
        Transform::NONE,
        1.0,
    )];
    let composed = scene(64, 36, layers);
    let frame = preview(&gpu, &mut compositor, &composed);
    assert_near(frame.at(24, 12), block, 1.5, "block");
    assert_near(frame.at(4, 30), grey, 1.5, "grey");
    let target = Target::export(&gpu, 64, 36, 10).unwrap();
    compositor.render(&composed, &target).unwrap();
    let mut read = Picture::default();
    target.read_picture(&mut read).unwrap();
    let word = |at: usize| u16::from_le_bytes([read.luma[at], read.luma[at + 1]]) >> 6;
    let wanted = codes([0.8, 0.2, 0.1], true)[0];
    assert!(word(12 * read.luma_stride + 24 * 2).abs_diff(wanted) <= 2);
    assert_no_errors(&gpu);
}

/// Stripes a pixel wide, black and white, over a picture of 4K.
fn stripes() -> Arc<Picture> {
    let (white, black) = (codes([1.0; 3], false), codes([0.0; 3], false));
    Arc::new(picture(3840, 2160, false, |x, _| {
        if x % 2 == 0 {
            white
        } else {
            black
        }
    }))
}

#[test]
fn stripes_drawn_two_three_and_four_times_smaller_are_grey() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let stripes = stripes();
    let layers = || {
        vec![layer(
            Source::Picture(Arc::clone(&stripes), 0),
            Transform::NONE,
            1.0,
        )]
    };
    // Grey is 127.5; read as they were, the stripes would be 0 and 255.
    for (width, height) in [(1920, 1080), (1280, 720), (960, 540)] {
        let frame = preview(&gpu, &mut compositor, &scene(width, height, layers()));
        // Away from the first and the last column, where the picture is taken to go on as its
        // edge does.
        for x in (1..width - 1).step_by(7) {
            for y in [0, height / 2, height - 1] {
                let pixel = frame.at(x, y);
                assert!(
                    pixel[0].abs_diff(128) <= 8,
                    "{width}×{height} at ({x}, {y}): {pixel:?}"
                );
            }
        }
    }
    // Made smaller by the transform, as by the canvas.
    let half = Transform {
        scale: 0.5,
        ..Transform::NONE
    };
    let frame = preview(
        &gpu,
        &mut compositor,
        &scene(
            1920,
            1080,
            vec![layer(Source::Picture(Arc::clone(&stripes), 0), half, 1.0)],
        ),
    );
    for x in (500..1400).step_by(5) {
        assert!(frame.at(x, 540)[0].abs_diff(128) <= 8, "{x}");
    }
    // And into an export: codes of luma about 125.5, dithered.
    let target = Target::export(&gpu, 1280, 720, 8).unwrap();
    compositor
        .render(&scene(1280, 720, layers()), &target)
        .unwrap();
    let mut read = Picture::default();
    target.read_picture(&mut read).unwrap();
    for x in 1..1279 {
        let code = read.luma[360 * read.luma_stride + x];
        assert!(code.abs_diff(126) <= 3, "{x}: {code}");
    }
    assert_no_errors(&gpu);
}

#[test]
fn a_picture_made_nv12_or_p010_keeps_each_row_and_each_column() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    // Luma that says the row and the column of each pixel, and chroma that changes with them.
    let (width, height) = (64, 48);
    // Of colours that R'G'B' holds, so that they come back as they were.
    let code = |x: u32, y: u32| 60 + u16::try_from((y * 4 + x % 4) % 120).unwrap();
    let chroma = |x: u32, y: u32| {
        [
            124 + u16::try_from(x / 2 % 8).unwrap(),
            124 + u16::try_from(y / 2 % 8).unwrap(),
        ]
    };
    let source = Arc::new(picture(width, height, false, |x, y| {
        let [cb, cr] = chroma(x, y);
        [code(x, y), cb, cr]
    }));
    let composed = scene(
        width,
        height,
        vec![layer(Source::Picture(source, 0), Transform::NONE, 1.0)],
    );
    for bits in [8, 10] {
        let target = Target::export(&gpu, width, height, bits).unwrap();
        compositor.render(&composed, &target).unwrap();
        let mut read = Picture::default();
        target.read_picture(&mut read).unwrap();
        let sample = |plane: &[u8], at: usize| {
            if bits > 8 {
                u16::from_le_bytes([plane[2 * at], plane[2 * at + 1]]) >> 8
            } else {
                u16::from(plane[at])
            }
        };
        let stride = read.luma_stride / if bits > 8 { 2 } else { 1 };
        for y in 0..height {
            for x in 0..width {
                let luma = sample(&read.luma, y as usize * stride + x as usize);
                assert!(
                    luma.abs_diff(code(x, y)) <= 1,
                    "{bits} bits, luma at ({x}, {y}): {luma}, not {}",
                    code(x, y)
                );
            }
        }
        // Chroma, away from the columns and rows where it changes.
        for j in 1..height / 2 - 1 {
            for i in 1..width / 2 - 1 {
                let [cb, cr] = chroma(2 * i, 2 * j);
                if chroma(2 * i - 2, 2 * j - 2) != [cb, cr]
                    || chroma(2 * i + 2, 2 * j + 2) != [cb, cr]
                {
                    continue;
                }
                let at = j as usize * stride + 2 * i as usize;
                let (first, second) = (sample(&read.chroma, at), sample(&read.chroma, at + 1));
                assert!(
                    first.abs_diff(cb) <= 1 && second.abs_diff(cr) <= 1,
                    "{bits} bits, chroma at ({i}, {j}): {first}, {second}, not {cb}, {cr}"
                );
            }
        }
    }
    assert_no_errors(&gpu);
}

#[test]
fn two_layers_that_meet_at_a_pixel_leave_no_seam_between_them() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let white = codes([1.0; 3], false);
    let picture = Arc::new(picture(1920, 1080, false, |_, _| white));
    // Split screen: each half as large, a quarter of the width to the left and to the right.
    for (width, height) in [(1920, 1080), (1280, 720), (640, 360)] {
        let half = |x: f64| Transform {
            x,
            scale: 0.5,
            ..Transform::NONE
        };
        let layers = vec![
            layer(Source::Picture(Arc::clone(&picture), 0), half(-0.25), 1.0),
            layer(Source::Picture(Arc::clone(&picture), 0), half(0.25), 1.0),
            layer(Source::Color([1.0; 4]), half(-0.25), 1.0),
        ];
        let frame = preview(&gpu, &mut compositor, &scene(width, height, layers));
        let middle = width / 2;
        for x in middle - 3..middle + 3 {
            assert_eq!(
                frame.at(x, height / 2),
                [255, 255, 255, 255],
                "{width}×{height} at {x}"
            );
        }
        assert_eq!(frame.at(middle, height / 8), [0, 0, 0, 255]);
    }
    assert_no_errors(&gpu);
}

fn title(text: &str, style: TitleStyle, transform: Transform) -> SceneLayer {
    layer(
        Source::Title(Text {
            text: text.into(),
            style,
        }),
        transform,
        1.0,
    )
}

#[test]
fn a_title_of_letters_4096_high_many_lines_long_is_drawn_without_running_out() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let text = "Columbina\nsecond line\nthird line\nfourth line\nfifth";
    let started = Instant::now();
    let huge = title(
        text,
        style(4096, 0.5, 0.5, TextAlign::Center),
        Transform::NONE,
    );
    let frame = preview(&gpu, &mut compositor, &scene(1920, 1080, vec![huge]));
    eprintln!("drawn in {:?}", started.elapsed());
    // A part of a letter fills much of the canvas.
    let (_, count) = lit(&frame, 1080, 128);
    assert!(count > 1920 * 1080 / 20, "{count} pixels lit");
    // Made smaller by its transform, the whole of it is seen, drawn at its scale.
    let small = title(
        text,
        style(4096, 0.5, 0.5, TextAlign::Center),
        Transform {
            scale: 0.02,
            ..Transform::NONE
        },
    );
    let frame = preview(&gpu, &mut compositor, &scene(1920, 1080, vec![small]));
    let ([x0, y0, x1, y1], count) = lit(&frame, 1080, 128);
    assert!(count > 1000, "{count}");
    assert!((x0 + x1).abs_diff(1920) <= 40, "{x0}..{x1}");
    assert!((y0 + y1).abs_diff(1080) <= 40, "{y0}..{y1}");
    // Five lines of 4096 × 1.25, a fiftieth as large: some 500 high.
    assert!((300..600).contains(&(y1 - y0)), "{y0}..{y1}");
    assert_no_errors(&gpu);
}

#[test]
fn chinese_letters_are_drawn_from_a_font_that_has_them() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let drawn = |compositor: &mut Compositor, text: &str| {
        let frame = preview(
            &gpu,
            compositor,
            &scene(
                128,
                64,
                vec![title(
                    text,
                    style(48, 0.5, 0.5, TextAlign::Center),
                    Transform::NONE,
                )],
            ),
        );
        frame
            .pixels
            .chunks(4)
            .map(|pixel| pixel[0] > 128)
            .collect::<Vec<bool>>()
    };
    let differ = |a: &[bool], b: &[bool]| a.iter().zip(b).filter(|(a, b)| a != b).count();
    let (ni, hao, square) = (
        drawn(&mut compositor, "你"),
        drawn(&mut compositor, "好"),
        drawn(&mut compositor, "\u{25A1}"),
    );
    // Letters missing from every font would be drawn as the same box.
    assert!(ni.iter().filter(|lit| **lit).count() > 100);
    assert!(differ(&ni, &hao) > 100, "你 and 好 are drawn alike");
    assert!(differ(&ni, &square) > 100, "你 is drawn as a box");
    assert!(differ(&hao, &square) > 100, "好 is drawn as a box");
}

#[test]
fn a_title_of_a_colour_half_opaque_is_laid_over_as_the_alpha_formula_says() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let mut red = style(64, 0.5, 0.5, TextAlign::Center);
    red.color = Rgba([255, 0, 0, 128]);
    let layers = vec![
        layer(Source::Color([1.0; 4]), Transform::NONE, 1.0),
        title("IIII", red, Transform::NONE),
    ];
    let frame = preview(&gpu, &mut compositor, &scene(256, 128, layers));
    // Within the letters, red half over white: 255, 127, 127; nowhere darker, as it would be if
    // the alpha were taken twice.
    let alpha = 128.0 / 255.0;
    let inside = 255.0 * (1.0 - alpha);
    let mut full = 0;
    for pixel in frame.pixels.chunks(4) {
        assert_eq!(pixel[0], 255, "{pixel:?}");
        assert!(f64::from(pixel[1]) >= inside - 2.0, "{pixel:?}");
        if (f64::from(pixel[1]) - inside).abs() <= 2.0 && pixel[1] == pixel[2] {
            full += 1;
        }
    }
    assert!(full > 200, "{full} pixels of the letters");
}

#[test]
fn titles_are_sized_in_pixels_of_the_timeline() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let layers = || {
        vec![title(
            "Columbina",
            style(40, 0.5, 0.5, TextAlign::Center),
            Transform::NONE,
        )]
    };
    let full = preview(&gpu, &mut compositor, &scene(640, 360, layers()));
    let mut half = scene(320, 180, layers());
    half.units = 0.5;
    let half = preview(&gpu, &mut compositor, &half);
    let ([a0, b0, a1, b1], _) = lit(&full, 360, 128);
    let ([c0, d0, c1, d1], _) = lit(&half, 180, 128);
    assert!(
        (a1 - a0).abs_diff(2 * (c1 - c0)) <= 4,
        "{a0}..{a1}, {c0}..{c1}"
    );
    assert!(
        (b1 - b0).abs_diff(2 * (d1 - d0)) <= 4,
        "{b0}..{b1}, {d0}..{d1}"
    );
}

/// How long a picture of 4K drawn three times smaller and subtitles of 4K take: run with
/// `--ignored --nocapture`, in release.
#[test]
#[ignore = "a measurement, not a test"]
fn how_long_smaller_pictures_and_subtitles_take() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let wait = |gpu: &Gpu| {
        gpu.device()
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
    };
    let stripes = stripes();
    let third = scene(
        1280,
        720,
        vec![layer(Source::Picture(stripes, 0), Transform::NONE, 1.0)],
    );
    let frames = 30;
    let target = Target::preview(&gpu, 1280, 720).unwrap();
    compositor.render(&third, &target).unwrap();
    wait(&gpu);
    let started = Instant::now();
    for _ in 0..frames {
        compositor.render(&third, &target).unwrap();
        wait(&gpu);
    }
    eprintln!(
        "4K drawn a third as large into a preview of 1280×720: {:?} a frame",
        started.elapsed() / frames
    );
    let export = Target::export(&gpu, 1280, 720, 8).unwrap();
    let mut read = Picture::default();
    let started = Instant::now();
    for _ in 0..frames {
        compositor.render(&third, &export).unwrap();
        export.read_picture(&mut read).unwrap();
    }
    eprintln!(
        "the same into an export of 8 bits, read back: {:?} a frame",
        started.elapsed() / frames
    );
    // Subtitles of 4K, each of another text, so that each is laid out and drawn anew.
    let target = Target::preview(&gpu, 3840, 2160).unwrap();
    let mut subtitled = scene(3840, 2160, vec![]);
    subtitled.subtitles = vec!["字幕 the first line of the subtitles".into(), "0".into()];
    compositor.render(&subtitled, &target).unwrap();
    wait(&gpu);
    let mut worst = std::time::Duration::ZERO;
    let started = Instant::now();
    for frame in 0..frames {
        subtitled.subtitles[1] = format!("and a second line, number {frame}");
        let one = Instant::now();
        compositor.render(&subtitled, &target).unwrap();
        worst = worst.max(one.elapsed());
    }
    wait(&gpu);
    eprintln!(
        "subtitles of two lines on 3840×2160, new each frame: {:?} a frame, at most {worst:?}",
        started.elapsed() / frames
    );
}
