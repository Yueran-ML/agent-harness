//! Layers of two pictures of one media, mixed: what a clip that is retimed by Frame Blend shows
//! at a frame whose place in its source lies between two pictures. The two are mixed as they
//! are stored, before they are made R'G'B', and all that is done to a layer is done to the
//! mixed picture as it is to one. Without an adapter the tests are skipped, unless
//! `COLUMBINA_REQUIRE_GPU` is 1.

use std::sync::Arc;

use columbina_compose::yuv;
use columbina_compose::{
    gpu_for_tests, ColorRange, ComposeError, Composite, Compositor, Crop, Gpu, LayerColor, Scene,
    SceneLayer, Source, Target, Transform, YuvCoding, YuvMatrix,
};
use columbina_media::{Alpha, NativePicture, PictureFormat};

const WIDTH: u32 = 16;
const HEIGHT: u32 = 8;

/// The formats that pictures are drawn in, with the bits of each: 4:2:0 in bytes and in words,
/// 4:2:2, 4:4:4, and R'G'B'A as a file stores it.
const FORMATS: [(PictureFormat, u8); 5] = [
    (PictureFormat::Nv12, 8),
    (PictureFormat::P010, 10),
    (PictureFormat::P216, 10),
    (PictureFormat::P416, 12),
    (PictureFormat::Rgba64, 16),
];

/// How the pictures of `format` here are coded: Y'CbCr by BT.709 in limited range, and R'G'B'A
/// in full range.
fn coding(format: PictureFormat) -> YuvCoding {
    YuvCoding {
        matrix: YuvMatrix::Bt709,
        range: if format.is_rgb() {
            ColorRange::Full
        } else {
            ColorRange::Limited
        },
    }
}

/// A picture of `format` in `bits` bits, of `width` × [`HEIGHT`], every sample of which has the
/// codes `pixel`: Y', Cb and Cr in the top bits of their bytes or words; or, of R'G'B'A, the
/// words of R', G' and B' at full scale. Its alpha, `alpha` of 65535, is straight where it is
/// not opaque: only R'G'B'A is given any here.
fn flat(format: PictureFormat, bits: u8, width: u32, pixel: [u16; 3], alpha: u16) -> NativePicture {
    let sample = |code: u16| -> Vec<u8> {
        if format.bytes() == 1 {
            vec![u8::try_from(code).unwrap()]
        } else {
            (code << (16 - bits)).to_le_bytes().to_vec()
        }
    };
    let pixels = (width * HEIGHT) as usize;
    let (luma, chroma, chroma_stride) = if format.is_rgb() {
        let one: Vec<u8> = [pixel[0], pixel[1], pixel[2], alpha]
            .into_iter()
            .flat_map(sample)
            .collect();
        (one.repeat(pixels), Vec::new(), 0)
    } else {
        let (across, down) = format.chroma_shift();
        let (chroma_width, chroma_height) = (width >> across, HEIGHT >> down);
        let one = [sample(pixel[1]), sample(pixel[2])].concat();
        (
            sample(pixel[0]).repeat(pixels),
            one.repeat((chroma_width * chroma_height) as usize),
            chroma_width as usize * one.len(),
        )
    };
    NativePicture {
        frame: 0,
        width,
        height: HEIGHT,
        format,
        bits,
        matrix: coding(format).matrix,
        range: coding(format).range,
        chroma_location: u8::from(!format.is_rgb()),
        alpha: if alpha == u16::MAX {
            Alpha::Opaque
        } else {
            Alpha::Straight
        },
        luma,
        chroma,
        luma_stride: width as usize * format.pixel_bytes(),
        chroma_stride,
        ..NativePicture::default()
    }
}

/// The codes of a picture of `format` in `bits` bits of the colour `rgb`, R'G'B' from 0 to 1,
/// rounded: Y', Cb and Cr, or the words of R'G'B'A.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "codes, which are whole and not below 0"
)]
fn codes(format: PictureFormat, bits: u8, rgb: [f64; 3]) -> [u16; 3] {
    if format.is_rgb() {
        return rgb.map(|value| (value * 65535.0).round() as u16);
    }
    let coding = coding(format);
    let (kr, kb) = coding.matrix.coefficients();
    let [r, g, b] = rgb;
    let y = kr * r + (1.0 - kr - kb) * g + kb * b;
    let cb = (b - y) / (2.0 * (1.0 - kb));
    let cr = (r - y) / (2.0 * (1.0 - kr));
    let [black, luma_range, middle, chroma_range] = yuv::levels(coding, bits);
    [
        black + luma_range * y,
        middle + chroma_range * cb,
        middle + chroma_range * cr,
    ]
    .map(|code| code.round() as u16)
}

/// R'G'B' from 0 to 1 of the codes `pixel` of a picture of `format` in `bits` bits, as the
/// shader makes it, unrounded. The codes need not be whole: those of two pictures mixed are not.
fn rgb_of(format: PictureFormat, bits: u8, pixel: [f64; 3]) -> [f64; 3] {
    if format.is_rgb() {
        return pixel.map(|word| word / 65535.0);
    }
    let coding = coding(format);
    let [black, luma_range, middle, chroma_range] = yuv::levels(coding, bits);
    let weights = coding.coefficients();
    let y = (pixel[0] - black) / luma_range;
    let (cb, cr) = (
        (pixel[1] - middle) / chroma_range,
        (pixel[2] - middle) / chroma_range,
    );
    [
        y + weights[0] * cr,
        y - weights[1] * cb - weights[2] * cr,
        y + weights[3] * cb,
    ]
}

/// `weight` of the way from `first` to `second`.
fn mixed(first: [u16; 3], second: [u16; 3], weight: f64) -> [f64; 3] {
    [0, 1, 2].map(|at| f64::from(first[at]) * (1.0 - weight) + f64::from(second[at]) * weight)
}

/// A layer of `source` as it is fitted, opaque, whole, laid as it is, decoded as a picture of
/// `format` here says it is.
fn layer(source: Source, format: PictureFormat) -> SceneLayer {
    SceneLayer {
        source,
        transform: Transform::NONE,
        opacity: 1.0,
        color: LayerColor {
            coding: coding(format),
            ..LayerColor::default()
        },
        crop: Crop::NONE,
        composite: Composite::Normal,
    }
}

/// A scene of the size of the pictures, of `layers`.
fn scene(layers: Vec<SceneLayer>) -> Scene {
    Scene {
        width: WIDTH,
        height: HEIGHT,
        layers,
        ..Scene::default()
    }
}

/// `weight` of the way from `first` to `second`, as a layer shows it.
fn between(first: &Arc<NativePicture>, second: &Arc<NativePicture>, weight: f32) -> Source {
    Source::Between {
        first: Arc::clone(first),
        second: Arc::clone(second),
        weight,
        rotation: 0,
    }
}

/// A preview of `scene`: R'G'B'A of 8 bits, and its half floats as they are.
fn preview(gpu: &Arc<Gpu>, compositor: &mut Compositor, scene: &Scene) -> (Vec<u8>, Vec<u16>) {
    let target = Target::preview(gpu, scene.width, scene.height).unwrap();
    compositor.render(scene, &target).unwrap();
    (target.read_rgba().unwrap(), target.read_halves().unwrap())
}

/// Check that every pixel of a preview is within a code of 8 bits of the colour `wanted`,
/// R'G'B' from 0 to 1, and opaque.
#[track_caller]
fn assert_all_near(pixels: &[u8], wanted: [f64; 3], what: &str) {
    assert_eq!(pixels.len(), (WIDTH * HEIGHT) as usize * 4, "{what}");
    for (at, pixel) in pixels.as_chunks::<4>().0.iter().enumerate() {
        for channel in 0..3 {
            let code = wanted[channel] * 255.0;
            assert!(
                (f64::from(pixel[channel]) - code).abs() <= 1.0,
                "{what}: pixel {at} is {pixel:?}, not {:?}",
                wanted.map(|value| value * 255.0)
            );
        }
        assert_eq!(pixel[3], 255, "{what}: pixel {at}");
    }
}

fn assert_no_errors(gpu: &Gpu) {
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

/// Two colours well inside what every format holds, and far from each other.
const COLOURS: [[f64; 3]; 2] = [[0.8, 0.2, 0.4], [0.1, 0.6, 0.9]];

#[test]
fn two_pictures_are_mixed_by_how_much_of_the_second_is_shown() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    for (format, bits) in FORMATS {
        let [one, other] = COLOURS.map(|colour| codes(format, bits, colour));
        let first = Arc::new(flat(format, bits, WIDTH, one, u16::MAX));
        let second = Arc::new(flat(format, bits, WIDTH, other, u16::MAX));
        for weight in [0.5_f32, 0.25, 0.75] {
            let what = format!("{format:?} in {bits} bits, {weight} of the second");
            let wanted = rgb_of(format, bits, mixed(one, other, f64::from(weight)));
            let source = between(&first, &second, weight);
            let (shown, _) = preview(&gpu, &mut compositor, &scene(vec![layer(source, format)]));
            assert_all_near(&shown, wanted, &what);
            // The other way round it is the rest of the way: the weight is of the second.
            let source = between(&second, &first, 1.0 - weight);
            let (shown, _) = preview(&gpu, &mut compositor, &scene(vec![layer(source, format)]));
            assert_all_near(&shown, wanted, &format!("{what}, the other way round"));
        }
    }
    assert_no_errors(&gpu);
}

#[test]
fn all_of_one_of_two_pictures_is_that_picture_as_it_is_drawn_alone_to_the_bit() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    for (format, bits) in FORMATS {
        let [one, other] = COLOURS.map(|colour| codes(format, bits, colour));
        let first = Arc::new(flat(format, bits, WIDTH, one, u16::MAX));
        let second = Arc::new(flat(format, bits, WIDTH, other, u16::MAX));
        let mut drawn =
            |source: Source| preview(&gpu, &mut compositor, &scene(vec![layer(source, format)])).1;
        let alone = [&first, &second].map(|picture| drawn(Source::Picture(Arc::clone(picture), 0)));
        assert_ne!(alone[0], alone[1], "{format:?}");
        // None of the second, and less than none, is the first; all of it, and more, the
        // second. A weight that is no number shows the first.
        for (weight, picture) in [(0.0, 0), (-0.5, 0), (f32::NAN, 0), (1.0, 1), (1.5, 1)] {
            assert_eq!(
                drawn(between(&first, &second, weight)),
                alone[picture],
                "{format:?} in {bits} bits, {weight} of the second"
            );
        }
    }
    assert_no_errors(&gpu);
}

#[test]
fn a_picture_mixed_with_itself_is_that_picture() {
    // The shader of two pictures reads each as the shader of one reads it: half of a picture
    // and half of the same picture again is the picture, to a step of a half float.
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    for (format, bits) in FORMATS {
        let pixel = codes(format, bits, COLOURS[0]);
        let picture = Arc::new(flat(format, bits, WIDTH, pixel, u16::MAX));
        let again = Arc::new(flat(format, bits, WIDTH, pixel, u16::MAX));
        // Made smaller and moved, so that it is read between its texels too.
        let placed = |source: Source| SceneLayer {
            transform: Transform {
                x: 0.125,
                y: -0.0625,
                scale: 0.75,
                rotation: 10.0,
            },
            ..layer(source, format)
        };
        let alone = scene(vec![placed(Source::Picture(Arc::clone(&picture), 0))]);
        let twice = scene(vec![placed(between(&picture, &again, 0.5))]);
        let (_, wanted) = preview(&gpu, &mut compositor, &alone);
        let (_, shown) = preview(&gpu, &mut compositor, &twice);
        assert_eq!(shown.len(), wanted.len());
        // The layer is drawn: not every value of the frame is that of the black under it.
        assert!(wanted.iter().any(|half| *half != wanted[0]), "{format:?}");
        for (at, (shown, wanted)) in shown.iter().zip(&wanted).enumerate() {
            assert!(
                shown.abs_diff(*wanted) <= 1,
                "{format:?} in {bits} bits: value {at} is {shown:#06x}, not {wanted:#06x}"
            );
        }
    }
    assert_no_errors(&gpu);
}

#[test]
fn pictures_with_alpha_are_mixed_premultiplied() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let format = PictureFormat::Rgba64;
    let solid = codes(format, 16, COLOURS[0]);
    // The second picture covers nothing, and its colour, which is not seen, is another.
    let unseen = codes(format, 16, COLOURS[1]);
    let first = Arc::new(flat(format, 16, WIDTH, solid, u16::MAX - 1));
    let second = Arc::new(flat(format, 16, WIDTH, unseen, 0));
    let source = between(&first, &second, 0.5);
    let (shown, _) = preview(&gpu, &mut compositor, &scene(vec![layer(source, format)]));
    // Half of the first over black, with nothing of the colour of the second in it.
    let wanted = COLOURS[0].map(|value| value * 0.5);
    assert_all_near(&shown, wanted, "half of a picture and half of nothing");
    assert_no_errors(&gpu);
}

#[test]
fn what_is_done_to_a_layer_is_done_to_the_mixed_picture_as_to_one() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    // Codes of 12 bits whose means are whole, so that a picture of the means is what the two
    // mixed by halves are.
    let (format, bits) = (PictureFormat::P416, 12);
    let (one, other, mean) = ([1000, 2100, 1900], [3000, 1900, 2300], [2000, 2000, 2100]);
    assert_eq!(mixed(one, other, 0.5), mean.map(f64::from));
    let first = Arc::new(flat(format, bits, WIDTH, one, u16::MAX));
    let second = Arc::new(flat(format, bits, WIDTH, other, u16::MAX));
    let middle = Arc::new(flat(format, bits, WIDTH, mean, u16::MAX));
    // Over a colour: made smaller, moved, cut at its left, half seen, and blended by a mode.
    let under = layer(Source::Color([0.2, 0.5, 0.3, 1.0]), format);
    for composite in [Composite::Normal, Composite::Screen, Composite::Multiply] {
        let over = |source: Source| SceneLayer {
            transform: Transform {
                x: 0.1,
                y: 0.0,
                scale: 0.8,
                rotation: 0.0,
            },
            opacity: 0.5,
            crop: Crop {
                left: 0.25,
                ..Crop::NONE
            },
            composite,
            ..layer(source, format)
        };
        let two = scene(vec![under.clone(), over(between(&first, &second, 0.5))]);
        let one_picture = scene(vec![
            under.clone(),
            over(Source::Picture(Arc::clone(&middle), 0)),
        ]);
        let (shown, _) = preview(&gpu, &mut compositor, &two);
        let (wanted, _) = preview(&gpu, &mut compositor, &one_picture);
        // The layer is seen: the frame is not the colour under it alone.
        let (alone, _) = preview(&gpu, &mut compositor, &scene(vec![under.clone()]));
        assert_ne!(wanted, alone, "{composite:?}");
        for (at, (shown, wanted)) in shown.iter().zip(&wanted).enumerate() {
            assert!(
                shown.abs_diff(*wanted) <= 1,
                "{composite:?}: value {at} is {shown}, not {wanted}"
            );
        }
    }
    assert_no_errors(&gpu);
}

#[test]
fn two_pictures_that_are_not_of_one_shape_are_not_mixed() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let pixel = codes(PictureFormat::Nv12, 8, COLOURS[0]);
    let first = Arc::new(flat(PictureFormat::Nv12, 8, WIDTH, pixel, u16::MAX));
    let wider = Arc::new(flat(PictureFormat::Nv12, 8, WIDTH * 2, pixel, u16::MAX));
    let ten = codes(PictureFormat::P010, 10, COLOURS[0]);
    let deeper = Arc::new(flat(PictureFormat::P010, 10, WIDTH, ten, u16::MAX));
    let target = Target::preview(&gpu, WIDTH, HEIGHT).unwrap();
    for second in [&wider, &deeper] {
        let source = between(&first, second, 0.5);
        let two = scene(vec![layer(source, PictureFormat::Nv12)]);
        let error = compositor.render(&two, &target).unwrap_err();
        assert!(
            matches!(error, ComposeError::Layer { layer: 0, .. }),
            "{error}"
        );
        // All of the first is the first alone, whatever the second is.
        let source = between(&first, second, 0.0);
        let one = scene(vec![layer(source, PictureFormat::Nv12)]);
        compositor.render(&one, &target).unwrap();
    }
    assert_no_errors(&gpu);
}
