//! Pictures of files, as the media crate reads them, drawn by the compositor: stills of R'G'B'
//! code for code (pure red 255, not 253), `OpenEXR` beyond 1.0 kept in the half floats of the
//! canvas, alpha of `ProRes` 4444 and of `OpenEXR` laid over a layer below it, and chroma sited
//! where each file says it lies. The files are made by the `ffmpeg` of the command line
//! (`media/tests/support`); without it, or without a card, the tests are skipped, unless
//! `COLUMBINA_REQUIRE_FFMPEG` and `COLUMBINA_REQUIRE_GPU` are 1.

#[path = "../../media/tests/support/mod.rs"]
mod support;

use std::path::Path;
use std::sync::Arc;

use columbina_compose::yuv;
use columbina_compose::{
    gpu_for_tests, Compositor, Gpu, LayerColor, Scene, SceneLayer, Source, Target, Transform,
    YuvCoding,
};
use columbina_media::{Alpha, Converter, Hardware, NativePicture, PictureFormat, VideoDecoder};
use support::{media, Sample};

/// The first frame of a sample as a native picture; None where the sample cannot be made and
/// is not required, or there is no card.
fn picture_of(sample: Sample) -> Option<NativePicture> {
    let made = media(sample)?;
    gpu_for_tests()?;
    Some(first(&made.path))
}

fn first(path: &Path) -> NativePicture {
    let mut opened = VideoDecoder::open(path, Hardware::Off).unwrap();
    let frame = opened.next_decoded().unwrap().expect("a first frame");
    Converter::new().native(&frame).unwrap()
}

/// A layer of a picture, decoded by what it says, filling a canvas of its size.
fn layer(picture: NativePicture) -> SceneLayer {
    let coding = YuvCoding::of_native(&picture);
    SceneLayer {
        source: Source::Picture(Arc::new(picture), 0),
        transform: Transform::NONE,
        opacity: 1.0,
        color: LayerColor {
            coding,
            ..LayerColor::default()
        },
        crop: columbina_compose::Crop::NONE,
        composite: columbina_compose::Composite::Normal,
    }
}

/// A scene of the size of `picture`, of `below` (if any) and the picture over it.
fn scene(picture: NativePicture, below: Option<[f32; 4]>) -> Scene {
    let (width, height) = (picture.width, picture.height);
    let mut layers: Vec<SceneLayer> = below
        .map(|colour| SceneLayer {
            source: Source::Color(colour),
            transform: Transform::NONE,
            opacity: 1.0,
            color: LayerColor::default(),
            crop: columbina_compose::Crop::NONE,
            composite: columbina_compose::Composite::Normal,
        })
        .into_iter()
        .collect();
    layers.push(layer(picture));
    Scene {
        width,
        height,
        layers,
        ..Scene::default()
    }
}

/// The value of a half float.
fn float_of_half(half: u16) -> f32 {
    let sign = if half & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exponent = i32::from((half >> 10) & 0x1f);
    let mantissa = f32::from(half & 0x03ff);
    match exponent {
        0 => sign * mantissa * 2f32.powi(-24),
        0x1f => f32::NAN,
        _ => sign * (1.0 + mantissa / 1024.0) * 2f32.powi(exponent - 15),
    }
}

/// The canvas of a preview of `scene` as it is, in half floats: R, G, B and A of each pixel,
/// unrounded and unclamped.
fn canvas(gpu: &Arc<Gpu>, compositor: &mut Compositor, scene: &Scene) -> Vec<[f32; 4]> {
    let target = Target::preview(gpu, scene.width, scene.height).unwrap();
    compositor.render(scene, &target).unwrap();
    let texture = target.texture();
    assert_eq!(texture.format(), wgpu::TextureFormat::Rgba16Float);
    let (width, height) = (scene.width, scene.height);
    let row = (width * 8).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = gpu.device().create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(row * height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(height),
            },
        },
        texture.size(),
    );
    gpu.queue().submit([encoder.finish()]);
    let (mapped, mapping) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            mapped.send(result).unwrap();
        });
    gpu.device()
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    mapping.recv().unwrap().unwrap();
    let bytes = buffer.slice(..).get_mapped_range().unwrap().to_vec();
    let mut pixels = Vec::new();
    for y in 0..height as usize {
        for x in 0..width as usize {
            let at = y * row as usize + x * 8;
            pixels.push(std::array::from_fn(|channel| {
                float_of_half(u16::from_le_bytes([
                    bytes[at + 2 * channel],
                    bytes[at + 2 * channel + 1],
                ]))
            }));
        }
    }
    pixels
}

/// R'G'B'A of a preview of `scene`, 8 bits each.
fn preview(gpu: &Arc<Gpu>, compositor: &mut Compositor, scene: &Scene) -> Vec<[u8; 4]> {
    let target = Target::preview(gpu, scene.width, scene.height).unwrap();
    compositor.render(scene, &target).unwrap();
    target.read_rgba().unwrap().as_chunks::<4>().0.to_vec()
}

/// R'G'B' from 0 to 1 of codes of Y', Cb and Cr (as fractions, blended) of a picture, by what
/// it says.
fn rgb_of(codes: [f64; 3], picture: &NativePicture) -> [f64; 3] {
    let coding = YuvCoding::of_native(picture);
    let [black, luma_range, middle, chroma_range] = yuv::levels(coding, picture.bits);
    let weights = coding.coefficients();
    let y = (codes[0] - black) / luma_range;
    let (cb, cr) = (
        (codes[1] - middle) / chroma_range,
        (codes[2] - middle) / chroma_range,
    );
    [
        y + weights[0] * cr,
        y - weights[1] * cb - weights[2] * cr,
        y + weights[3] * cb,
    ]
    .map(|value| value.clamp(0.0, 1.0))
}

#[track_caller]
fn assert_near(pixel: [u8; 4], wanted: [f64; 3], most: f64, what: &str) {
    for channel in 0..3 {
        assert!(
            (f64::from(pixel[channel]) - wanted[channel] * 255.0).abs() <= most,
            "{what}: {pixel:?}, not {:?}",
            wanted.map(|value| value * 255.0)
        );
    }
}

#[test]
fn stills_of_r_g_b_are_drawn_code_for_code() {
    let Some(ramp) = picture_of(Sample::RampPng) else {
        return;
    };
    let gpu = gpu_for_tests().unwrap();
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    assert_eq!(ramp.format, PictureFormat::Rgba64);
    let shown = preview(&gpu, &mut compositor, &scene(ramp, None));
    for y in 0..4 {
        for x in 0..256 {
            let [red, green, blue] = support::ramp(x, y);
            assert_eq!(
                shown[(y * 256 + x) as usize],
                [red, green, blue, 255],
                "at ({x}, {y})"
            );
        }
    }
    // Pure red is 255: the 253 that a still made Y'CbCr came to is gone.
    let Some(red) = picture_of(Sample::RedPng) else {
        return;
    };
    let shown = preview(&gpu, &mut compositor, &scene(red, None));
    assert!(shown.iter().all(|&pixel| pixel == [255, 0, 0, 255]));
    // Of grey, and of 16 bits, rounded to 8 bits from the words of the file.
    let Some(grey) = picture_of(Sample::GreyPng) else {
        return;
    };
    let shown = preview(&gpu, &mut compositor, &scene(grey, None));
    for (x, pixel) in shown.iter().take(256).enumerate() {
        let code = u8::try_from(x).unwrap();
        assert_eq!(*pixel, [code, code, code, 255]);
    }
    let Some(deep) = picture_of(Sample::Rgb48Png) else {
        return;
    };
    let canvas = canvas(&gpu, &mut compositor, &scene(deep.clone(), None));
    for (at, pixel) in canvas.iter().enumerate() {
        let (x, y) = (
            u32::try_from(at % 256).unwrap(),
            u32::try_from(at / 256).unwrap(),
        );
        let words = deep.rgba(x, y);
        for channel in 0..3 {
            let wanted = f32::from(words[channel]) / 65535.0;
            // Within a step of a half float about it.
            assert!(
                (pixel[channel] - wanted).abs() <= wanted / 1024.0 + 1e-6,
                "{pixel:?} for {words:?} at ({x}, {y})"
            );
        }
    }
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

#[test]
fn open_exr_beyond_1_is_kept_on_the_canvas() {
    let Some(hot) = picture_of(Sample::HotExr) else {
        return;
    };
    let gpu = gpu_for_tests().unwrap();
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    assert_eq!(hot.format, PictureFormat::RgbaF16);
    let canvas = canvas(&gpu, &mut compositor, &scene(hot.clone(), None));
    let mut brightest = 0.0_f32;
    for (at, pixel) in canvas.iter().enumerate() {
        let (x, y) = (
            u32::try_from(at % 64).unwrap(),
            u32::try_from(at / 64).unwrap(),
        );
        // Each pixel read at its centre, from its own texel: the half float of the file, however
        // far beyond 1, within a step of a half float (a card may round toward 0 as it writes
        // a half float).
        let halves = hot.rgba(x, y).map(float_of_half);
        for c in 0..3 {
            assert!(
                (pixel[c] - halves[c]).abs() <= halves[c].abs() / 1024.0 + 1e-4,
                "{pixel:?}, not {halves:?} at ({x}, {y})"
            );
        }
        assert_eq!(pixel[3], 1.0);
        brightest = brightest.max(pixel[0]).max(pixel[1]).max(pixel[2]);
    }
    assert!(brightest > 3.9, "the brightest is {brightest}");
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

/// The colour of the layer below the pictures with alpha, R'G'B'A from 0 to 1: a green.
const BELOW: [f32; 4] = [0.1, 0.6, 0.2, 1.0];

#[test]
fn alpha_of_prores_4444_shows_the_layer_below_it() {
    let Some(prores) = picture_of(Sample::ProresAlpha) else {
        return;
    };
    let gpu = gpu_for_tests().unwrap();
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    assert_eq!(
        (prores.format, prores.alpha),
        (PictureFormat::P416, Alpha::Straight)
    );
    let shown = preview(&gpu, &mut compositor, &scene(prores.clone(), Some(BELOW)));
    let below = BELOW.map(f64::from);
    for y in (0..64).step_by(7) {
        for x in 0..256 {
            let alpha = prores.alpha_at(x, y);
            let codes = prores.ycc(x, y).map(f64::from);
            let colour = rgb_of(codes, &prores);
            // Straight: the colour of the picture as much as its alpha, the layer below the
            // rest.
            let wanted: [f64; 3] =
                std::array::from_fn(|c| colour[c] * alpha + below[c] * (1.0 - alpha));
            let pixel = shown[(y * 256 + x) as usize];
            assert_near(pixel, wanted, 1.0, &format!("at ({x}, {y}), alpha {alpha}"));
            assert_eq!(pixel[3], 255);
        }
    }
    // Transparent at the left, opaque at the right.
    assert_near(shown[0], [below[0], below[1], below[2]], 1.0, "the left");
    let right = rgb_of(prores.ycc(255, 0).map(f64::from), &prores);
    assert_near(shown[255], right, 1.0, "the right");
    // Without the layer below, over the black of the canvas: the colour as much as its alpha.
    let alone = canvas(&gpu, &mut compositor, &scene(prores.clone(), None));
    for x in [0, 64, 128, 255] {
        let alpha = prores.alpha_at(x, 0);
        let colour = rgb_of(prores.ycc(x, 0).map(f64::from), &prores);
        for c in 0..3 {
            let got = f64::from(alone[x as usize][c]);
            assert!((got - colour[c] * alpha).abs() < 2e-3, "{got} at {x}");
        }
    }
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

#[test]
fn premultiplied_alpha_of_open_exr_is_laid_over_as_it_is() {
    let Some(exr) = picture_of(Sample::AlphaExr) else {
        return;
    };
    let gpu = gpu_for_tests().unwrap();
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    assert_eq!(exr.alpha, Alpha::Premultiplied);
    let canvas = canvas(&gpu, &mut compositor, &scene(exr.clone(), Some(BELOW)));
    let [red, green, blue, alpha] = exr.rgba(10, 10).map(float_of_half);
    // Premultiplied: the colour as it is, the layer below as much as is not covered.
    let wanted = [red, green, blue].map(f64::from);
    let pixel = canvas[10 * 64 + 10];
    for c in 0..3 {
        let over = wanted[c] + f64::from(BELOW[c]) * (1.0 - f64::from(alpha));
        assert!(
            (f64::from(pixel[c]) - over).abs() < 2e-3,
            "{pixel:?}, not {over} in {c}"
        );
    }
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

/// What the shader should draw at the pixel `(x, y)` of a picture of the chroma checkerboard
/// ([`support::checkerboard`]) shown at its own size, its chroma sited as it says: Cb and Cr
/// blended between the four samples about where the pixel lies in the plane of chroma.
fn sited(picture: &NativePicture, x: u32, y: u32) -> [f64; 3] {
    let [sx, sy] = yuv::siting(picture.format, picture.chroma_location).map(f64::from);
    let (across, down) = picture.format.chroma_shift();
    let step = |shift: u32| if shift == 0 { 1.0 } else { 0.5 };
    let (width, height) = picture.chroma_size();
    let place = [
        (f64::from(x) - sx) * step(across),
        (f64::from(y) - sy) * step(down),
    ];
    let low = place.map(f64::floor);
    let blend = [place[0] - low[0], place[1] - low[1]];
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let index = |value: f64, size: u32| -> u32 { value.clamp(0.0, f64::from(size - 1)) as u32 };
    let sample = |dx: f64, dy: f64| -> [f64; 2] {
        let codes = picture.ycc(
            index(low[0] + dx, width) << across,
            index(low[1] + dy, height) << down,
        );
        [f64::from(codes[1]), f64::from(codes[2])]
    };
    let mix =
        |a: [f64; 2], b: [f64; 2], t: f64| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
    let top = mix(sample(0.0, 0.0), sample(1.0, 0.0), blend[0]);
    let bottom = mix(sample(0.0, 1.0), sample(1.0, 1.0), blend[0]);
    let [cb, cr] = mix(top, bottom, blend[1]);
    rgb_of([f64::from(picture.ycc(x, y)[0]), cb, cr], picture)
}

#[test]
fn chroma_is_drawn_where_each_file_says_it_lies() {
    let sitings = (1..=6)
        .map(Sample::Sited420)
        .chain((1..=2).map(Sample::Sited422));
    let Some(gpu) = gpu_for_tests() else {
        return;
    };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    for sample in sitings {
        let Some(picture) = picture_of(sample) else {
            return;
        };
        let shown = preview(&gpu, &mut compositor, &scene(picture.clone(), None));
        // Drawn as if sited left, each pixel would be otherwise somewhere: the test holds the
        // siting, not merely the colours.
        let mut left = picture.clone();
        left.chroma_location = 1;
        let mut told_apart = picture.chroma_location == 1;
        for y in 0..picture.height {
            for x in 0..picture.width {
                let pixel = shown[(y * picture.width + x) as usize];
                let wanted = sited(&picture, x, y);
                assert_near(pixel, wanted, 1.0, &format!("{sample:?} at ({x}, {y})"));
                let other = sited(&left, x, y);
                told_apart |= (0..3).any(|c| (other[c] - wanted[c]).abs() * 255.0 > 8.0);
            }
        }
        assert!(told_apart, "{sample:?} is drawn as if sited left");
    }
    assert_eq!(gpu.uncaught_errors(), (0, None));
}
