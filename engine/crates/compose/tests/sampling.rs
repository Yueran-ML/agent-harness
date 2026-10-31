//! Pictures of 4:2:2 and 4:4:4 (`NativePicture`s of P216 and P416, in words of 16 bits whose top
//! bits matter): every sample of their chroma drawn where it lies, their codes read by their own
//! bits and levels, in a preview as in an export. Without an adapter the tests are skipped,
//! unless `COLUMBINA_REQUIRE_GPU` is 1.

use std::sync::Arc;
use std::time::Instant;

use columbina_compose::yuv;
use columbina_compose::{
    gpu_for_tests, ColorRange, Compositor, Gpu, LayerColor, Scene, SceneLayer, Source, Target,
    Transform, YuvCoding, YuvMatrix,
};
use columbina_media::{NativePicture, Picture, PictureFormat};

/// The codes of `bits` bits of R'G'B' from 0 to 1, made Y'CbCr by `coding`, unrounded.
fn exact_codes(rgb: [f64; 3], coding: YuvCoding, bits: u8) -> [f64; 3] {
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
}

/// As [`exact_codes`], rounded.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn codes(rgb: [f64; 3], coding: YuvCoding, bits: u8) -> [u16; 3] {
    exact_codes(rgb, coding, bits).map(|code| code.round() as u16)
}

/// R'G'B' from 0 to 1 of codes of `bits` bits coded as `coding` says, as the shader should make
/// it.
fn rgb(codes: [f64; 3], coding: YuvCoding, bits: u8) -> [f64; 3] {
    let [black, luma_range, middle, chroma_range] = yuv::levels(coding, bits);
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

/// What is put below the top `bits` of each word of a picture: not 0, as a converter of more
/// bits may leave it.
fn below(bits: u8) -> u16 {
    u16::try_from((1u32 << (16 - bits)) - 1).unwrap() & 0x5555
}

/// A code of `bits` bits as the shader reads it from its word: with what is below it, as a
/// fraction of a code.
fn read(code: u16, bits: u8) -> f64 {
    f64::from(code) + f64::from(below(bits)) / f64::from(1u32 << (16 - bits))
}

/// A picture of `format` (P216 or P416) of `bits` bits, said to be of `coding`, whose pixel at
/// (x, y) has the luma that `luma` gives, and whose sample of chroma at (x, y) of the plane of
/// chroma has the Cb and Cr that `chroma` gives. Rows are longer than they need to be, and what
/// is below the top bits of each word is not 0 ([`below`]).
fn native(
    width: u32,
    height: u32,
    format: PictureFormat,
    bits: u8,
    coding: YuvCoding,
    luma: impl Fn(u32, u32) -> u16,
    chroma: impl Fn(u32, u32) -> [u16; 2],
) -> NativePicture {
    let (across, down) = format.chroma_shift();
    let (chroma_width, chroma_height) = (width.div_ceil(1 << across), height.div_ceil(1 << down));
    let luma_stride = (width as usize * 2).next_multiple_of(64) + 64;
    let chroma_stride = (chroma_width as usize * 4).next_multiple_of(64) + 64;
    let mut luma_plane = vec![0xEE; luma_stride * height as usize];
    let mut chroma_plane = vec![0xEE; chroma_stride * chroma_height as usize];
    let word = |code: u16| {
        let top = u16::try_from(u32::from(code) << (16 - bits)).unwrap();
        (top | below(bits)).to_le_bytes()
    };
    for y in 0..height {
        for x in 0..width {
            let at = y as usize * luma_stride + x as usize * 2;
            luma_plane[at..at + 2].copy_from_slice(&word(luma(x, y)));
        }
    }
    for y in 0..chroma_height {
        for x in 0..chroma_width {
            let at = y as usize * chroma_stride + x as usize * 4;
            let [cb, cr] = chroma(x, y);
            chroma_plane[at..at + 2].copy_from_slice(&word(cb));
            chroma_plane[at + 2..at + 4].copy_from_slice(&word(cr));
        }
    }
    NativePicture {
        frame: 0,
        width,
        height,
        format,
        bits,
        matrix: coding.matrix,
        range: coding.range,
        chroma_location: 1,
        luma: luma_plane,
        chroma: chroma_plane,
        luma_stride,
        chroma_stride,
    }
}

/// A picture of one colour of codes `pixel` in each sample.
fn solid(
    width: u32,
    height: u32,
    format: PictureFormat,
    bits: u8,
    coding: YuvCoding,
    pixel: [u16; 3],
) -> NativePicture {
    native(
        width,
        height,
        format,
        bits,
        coding,
        |_, _| pixel[0],
        |_, _| [pixel[1], pixel[2]],
    )
}

/// The picture as the only layer of a scene of its size, decoded by what it says.
fn alone(picture: NativePicture) -> Scene {
    let coding = YuvCoding::of_native(&picture);
    Scene {
        width: picture.width,
        height: picture.height,
        layers: vec![SceneLayer {
            source: Source::Picture(Arc::new(picture), 0),
            transform: Transform::NONE,
            opacity: 1.0,
            color: LayerColor {
                coding,
                ..LayerColor::default()
            },
        }],
        ..Scene::default()
    }
}

/// R'G'B'A of a preview of `scene`, 8 bits each.
fn preview(gpu: &Arc<Gpu>, compositor: &mut Compositor, scene: &Scene) -> Vec<u8> {
    let target = Target::preview(gpu, scene.width, scene.height).unwrap();
    compositor.render(scene, &target).unwrap();
    target.read_rgba().unwrap()
}

fn at(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let at = (y * width + x) as usize * 4;
    pixels[at..at + 4].try_into().unwrap()
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

fn assert_no_errors(gpu: &Gpu) {
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

/// 75% colour bars: white, yellow, cyan, green, magenta, red, blue, black.
const BARS: [[f64; 3]; 8] = [
    [0.75, 0.75, 0.75],
    [0.75, 0.75, 0.0],
    [0.0, 0.75, 0.75],
    [0.0, 0.75, 0.0],
    [0.75, 0.0, 0.75],
    [0.75, 0.0, 0.0],
    [0.0, 0.0, 0.75],
    [0.0, 0.0, 0.0],
];

const CODINGS: [YuvCoding; 3] = [
    YuvCoding {
        matrix: YuvMatrix::Bt709,
        range: ColorRange::Limited,
    },
    YuvCoding {
        matrix: YuvMatrix::Bt601,
        range: ColorRange::Full,
    },
    YuvCoding {
        matrix: YuvMatrix::Bt2020,
        range: ColorRange::Limited,
    },
];

#[test]
fn bars_of_4_2_2_and_4_4_4_are_drawn_in_their_colours_whatever_their_bits() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let (width, height) = (64, 8);
    for (format, bits) in [
        (PictureFormat::P216, 10),
        (PictureFormat::P216, 12),
        (PictureFormat::P416, 8),
        (PictureFormat::P416, 12),
        (PictureFormat::P416, 16),
    ] {
        let (across, _) = format.chroma_shift();
        for coding in CODINGS {
            let bar = |x: u32| codes(BARS[(x / 8) as usize], coding, bits);
            let picture = native(
                width,
                height,
                format,
                bits,
                coding,
                |x, _| bar(x)[0],
                |x, _| {
                    let [_, cb, cr] = bar(x << across);
                    [cb, cr]
                },
            );
            let shown = preview(&gpu, &mut compositor, &alone(picture));
            for (index, nominal) in (0u32..).zip(BARS.iter()) {
                let exact = rgb(
                    codes(*nominal, coding, bits).map(|code| read(code, bits)),
                    coding,
                    bits,
                );
                let what = format!("{format:?} of {bits} bits, {coding:?}, bar {index}");
                for x in [index * 8 + 3, index * 8 + 4] {
                    // Within half a step of 8 bits of the colour of the codes, and of the
                    // colour they were rounded from by what rounding codes of `bits` bits makes
                    // of it (up to 1.75 for codes of 8 bits).
                    let rounding = if bits == 8 { 2.0 } else { 1.0 };
                    assert_near(at(&shown, width, x, 4), exact, 0.65, &what);
                    assert_near(at(&shown, width, x, 4), *nominal, rounding, &what);
                }
            }
        }
    }
    assert_no_errors(&gpu);
}

#[test]
fn every_sample_of_the_chroma_of_4_4_4_is_its_own_pixels() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let coding = YuvCoding::default();
    let bits = 12;
    // A checkerboard of red and blue a pixel each, of one luma: of 4:2:0 or 4:2:2 its chroma
    // would be grey.
    let red = codes([0.8, 0.1, 0.1], coding, bits);
    let blue = codes([0.1, 0.1, 0.8], coding, bits);
    let luma = red[0].midpoint(blue[0]);
    let picture = native(
        16,
        8,
        PictureFormat::P416,
        bits,
        coding,
        |_, _| luma,
        |x, y| {
            let [_, cb, cr] = if (x + y) % 2 == 0 { red } else { blue };
            [cb, cr]
        },
    );
    let shown = preview(&gpu, &mut compositor, &alone(picture));
    for y in 0..8 {
        for x in 0..16 {
            let [_, cb, cr] = if (x + y) % 2 == 0 { red } else { blue };
            let exact = rgb([luma, cb, cr].map(|code| read(code, bits)), coding, bits);
            assert_near(at(&shown, 16, x, y), exact, 0.65, &format!("({x}, {y})"));
        }
    }
    assert_no_errors(&gpu);
}

#[test]
fn the_chroma_of_4_2_2_is_of_each_row_and_lies_on_the_even_columns() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let coding = YuvCoding::default();
    let bits = 10;
    let (width, height) = (32, 8);
    // Cr rises by 24 from a sample of chroma to the next, and Cb is of the row: 400 on the even
    // rows, 624 on the odd ones. Of 4:2:0 the rows would be mixed.
    let cr = |x: u32| 320 + 24 * u16::try_from(x).unwrap();
    let cb = |y: u32| if y.is_multiple_of(2) { 400 } else { 624 };
    let picture = native(
        width,
        height,
        PictureFormat::P216,
        bits,
        coding,
        |_, _| 502,
        |x, y| [cb(y), cr(x)],
    );
    let shown = preview(&gpu, &mut compositor, &alone(picture));
    for y in 0..height {
        // Not the last column, which has no sample of chroma after it.
        for x in 0..width - 1 {
            // An even column is its sample; an odd one is halfway to the next.
            let red = if x % 2 == 0 {
                read(cr(x / 2), bits)
            } else {
                read(cr(x / 2), bits).midpoint(read(cr(x / 2 + 1), bits))
            };
            let exact = rgb([read(502, bits), read(cb(y), bits), red], coding, bits);
            assert_near(at(&shown, width, x, y), exact, 0.65, &format!("({x}, {y})"));
        }
    }
    assert_no_errors(&gpu);
}

/// The R'G'B' from 0 to 1 of a pixel of an export of P010 (BT.709, limited range).
fn exported(read: &Picture, x: usize, y: usize) -> [f64; 3] {
    let word =
        |plane: &[u8], at: usize| f64::from(u16::from_le_bytes([plane[at], plane[at + 1]]) >> 6);
    let luma = word(&read.luma, y * read.luma_stride + 2 * x);
    let chroma = (y / 2) * read.chroma_stride + (x / 2) * 4;
    let (cb, cr) = (word(&read.chroma, chroma), word(&read.chroma, chroma + 2));
    rgb([luma, cb, cr], YuvCoding::default(), 10)
}

#[test]
fn a_preview_and_an_export_of_4_2_2_and_4_4_4_agree() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let (width, height) = (64, 32);
    for (format, bits, coding) in [
        (PictureFormat::P216, 10, CODINGS[0]),
        (PictureFormat::P416, 12, CODINGS[1]),
        (PictureFormat::P416, 16, CODINGS[2]),
    ] {
        // Blocks of 16 × 16 of the bars, so that every sample of chroma about a pixel inside a
        // block is of its colour.
        let block = |x: u32, y: u32| BARS[((x / 16 + 4 * (y / 16)) % 8) as usize];
        let (across, down) = format.chroma_shift();
        let picture = native(
            width,
            height,
            format,
            bits,
            coding,
            |x, y| codes(block(x, y), coding, bits)[0],
            |x, y| {
                let [_, cb, cr] = codes(block(x << across, y << down), coding, bits);
                [cb, cr]
            },
        );
        let scene = alone(picture);
        let shown = preview(&gpu, &mut compositor, &scene);
        let target = Target::export(&gpu, width, height, 10).unwrap();
        compositor.render(&scene, &target).unwrap();
        let mut read = Picture::default();
        target.read_picture(&mut read).unwrap();
        for y in [5, 21] {
            for x in [5, 21, 37, 53] {
                let what = format!("{format:?} of {bits} bits, {coding:?}, at ({x}, {y})");
                let from_export = exported(&read, x as usize, y as usize);
                // The export's codes of 10 bits, a step of a quarter of 8 bits, and the
                // preview's of 8: within 1.5 of each other, and of the colour of the block.
                assert_near(at(&shown, width, x, y), from_export, 1.5, &what);
                assert_near(at(&shown, width, x, y), block(x, y), 1.5, &what);
            }
        }
    }
    assert_no_errors(&gpu);
}

#[test]
fn planes_of_another_sampling_are_not_those_of_a_picture_of_the_same_size() {
    // Pictures of one size and three samplings, one after another, then again: each is drawn
    // with planes of its own shape (a chroma plane of 4:2:0 would not hold one of 4:4:4).
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let coding = YuvCoding::default();
    let red = [0.75, 0.0, 0.0];
    let [y, cb, cr] = codes(red, coding, 8).map(|code| u8::try_from(code).unwrap());
    let pictures = [
        solid(
            16,
            8,
            PictureFormat::P416,
            12,
            coding,
            codes(red, coding, 12),
        ),
        solid(
            16,
            8,
            PictureFormat::P216,
            10,
            coding,
            codes(red, coding, 10),
        ),
        yuv::native_of(Picture {
            frame: 0,
            width: 16,
            height: 8,
            bits: 8,
            luma: vec![y; 64 * 8],
            chroma: [cb, cr].repeat(32 * 4),
            luma_stride: 64,
            chroma_stride: 64,
        }),
    ];
    for _ in 0..2 {
        for picture in &pictures {
            let shown = preview(&gpu, &mut compositor, &alone(picture.clone()));
            for (x, y) in [(0, 0), (15, 7), (8, 4)] {
                assert_near(
                    at(&shown, 16, x, y),
                    red,
                    1.5,
                    &format!("{:?} at ({x}, {y})", picture.format),
                );
            }
        }
    }
    assert_no_errors(&gpu);
}

/// How long pictures of 4K of 4:2:2 and 4:4:4 take, uploaded each frame (as a new picture is)
/// and drawn into a preview of 4K and an export of 10 bits, against 4:2:0 of 10 bits: run with
/// `--ignored --nocapture`, in release.
#[test]
#[ignore = "a measurement, not a test"]
fn how_long_pictures_of_4k_of_4_2_2_and_4_4_4_take() {
    let Some(gpu) = gpu_for_tests() else { return };
    eprintln!("on {} ({})", gpu.name(), gpu.backend());
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let wait = |gpu: &Gpu| {
        gpu.device()
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
    };
    let (width, height) = (3840, 2160);
    let coding = YuvCoding::default();
    let grey = codes([0.4, 0.5, 0.6], coding, 10);
    for (format, bits) in [
        (PictureFormat::P010, 10),
        (PictureFormat::P216, 10),
        (PictureFormat::P416, 12),
    ] {
        let picture = if format == PictureFormat::P010 {
            let mut picture = solid(width, height, PictureFormat::P216, 10, coding, grey);
            // The planes of P010: chroma of half the rows, rows as long as those of luma.
            picture.format = PictureFormat::P010;
            picture.chroma_stride = picture.luma_stride;
            picture.chroma = vec![0x80; picture.luma_stride * (height as usize / 2)];
            picture
        } else {
            let pixel = codes([0.4, 0.5, 0.6], coding, bits);
            solid(width, height, format, bits, coding, pixel)
        };
        #[allow(clippy::cast_precision_loss, reason = "a size of some 50 MB")]
        let megabytes = (picture.luma.len() + picture.chroma.len()) as f64 / 1e6;
        // Each frame a picture of its own, as each frame decoded is: the planes are uploaded
        // anew every time.
        let scene = alone(picture);
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
            "{format:?} of {bits} bits, {width}×{height} ({megabytes:.1} MB), into a preview: \
             {:?} a frame",
            started.elapsed() / frames
        );
        let export = Target::export(&gpu, width, height, 10).unwrap();
        let mut read = Picture::default();
        compositor.render(&scene, &export).unwrap();
        export.read_picture(&mut read).unwrap();
        let started = Instant::now();
        for _ in 0..frames {
            compositor.render(&scene, &export).unwrap();
            export.read_picture(&mut read).unwrap();
        }
        eprintln!(
            "the same into an export of 10 bits, read back: {:?} a frame",
            started.elapsed() / frames
        );
    }
}
