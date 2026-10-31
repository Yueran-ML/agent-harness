//! Frames of phase 1 kept as they were composed before colour came in (phase 2 split composing
//! into stages, each of which changes nothing yet): the same scenes are composed now, and must
//! be within 1 of 255 of what was kept, a preview pixel by pixel and an export code by code.
//!
//! The frames were written by the compositor of phase 1 with `COLUMBINA_WRITE_GOLDEN=1`, on the
//! card of the machine that wrote them (an NVIDIA one, by Direct3D 12). No text is drawn: what
//! letters look like depends on the fonts of the system.

use std::path::PathBuf;
use std::sync::Arc;

use columbina_compose::{gpu_for_tests, Compositor, Scene, SceneLayer, Source, Target, Transform};
use columbina_media::Picture;

/// The codes of BT.709 of limited range of R'G'B' from 0 to 1.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn codes(rgb: [f64; 3], ten: bool) -> [u16; 3] {
    let (kr, kb) = (0.2126, 0.0722);
    let [r, g, b] = rgb;
    let y = kr * r + (1.0 - kr - kb) * g + kb * b;
    let cb = (b - y) / (2.0 * (1.0 - kb));
    let cr = (r - y) / (2.0 * (1.0 - kr));
    let (black, luma_range, middle, chroma_range) = if ten {
        (64.0, 876.0, 512.0, 896.0)
    } else {
        (16.0, 219.0, 128.0, 224.0)
    };
    [
        black + luma_range * y,
        middle + chroma_range * cb,
        middle + chroma_range * cr,
    ]
    .map(|code: f64| code.round() as u16)
}

/// A picture whose pixel at (x, y) has the codes that `pixel` gives; the chroma of each block of
/// 2 × 2 is that of its top left pixel.
fn picture(width: u32, height: u32, ten: bool, pixel: impl Fn(u32, u32) -> [u16; 3]) -> Picture {
    let bytes = if ten { 2 } else { 1 };
    let stride = (width as usize * bytes).next_multiple_of(64);
    let rows = height as usize;
    let mut luma = vec![0; stride * rows];
    let mut chroma = vec![0; stride * rows.div_ceil(2)];
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

fn layer(source: Source, transform: Transform, opacity: f32) -> SceneLayer {
    SceneLayer {
        source,
        transform,
        opacity,
    }
}

/// Pictures of 8 and 10 bits fitted, made smaller than they are, moved, turned by their media
/// and by their transforms, and half opaque, with a colour over them.
fn pictures() -> Scene {
    let colours = [
        [0.9, 0.1, 0.1],
        [0.1, 0.8, 0.2],
        [0.2, 0.3, 0.9],
        [0.95, 0.9, 0.2],
    ];
    // Squares of four colours, and fine stripes over its lower third.
    let large = Arc::new(picture(256, 144, false, |x, y| {
        if y >= 96 {
            codes([f64::from(x % 2); 3], false)
        } else {
            codes(colours[((x / 32 + y / 32) % 4) as usize], false)
        }
    }));
    // A ramp of 10 bits, 4:3.
    let ramp = Arc::new(picture(64, 48, true, |x, y| {
        let level = f64::from(x + y) / 110.0;
        codes([level, 1.0 - level, 0.5], true)
    }));
    // Standing: its top half red, its bottom half blue, stored turned.
    let standing = Arc::new(picture(48, 64, false, |x, _| {
        codes(
            if x < 24 {
                [0.8, 0.1, 0.1]
            } else {
                [0.1, 0.1, 0.8]
            },
            false,
        )
    }));
    Scene {
        width: 128,
        height: 72,
        layers: vec![
            layer(Source::Picture(large, 0), Transform::NONE, 1.0),
            layer(
                Source::Picture(ramp, 0),
                Transform {
                    x: 0.2,
                    y: -0.1,
                    scale: 0.6,
                    rotation: 15.0,
                },
                0.7,
            ),
            layer(
                Source::Picture(standing, 90),
                Transform {
                    x: -0.3,
                    y: 0.05,
                    scale: 0.4,
                    rotation: 0.0,
                },
                0.9,
            ),
            layer(
                Source::Color([0.1, 0.3, 0.9, 0.5]),
                Transform {
                    x: 0.1,
                    y: 0.2,
                    scale: 0.3,
                    rotation: -20.0,
                },
                0.6,
            ),
        ],
        ..Scene::default()
    }
}

/// Colours laid over each other, partly opaque, over a ramp half opaque: where rounding to 8
/// bits between layers would show.
fn colours() -> Scene {
    let ramp = Arc::new(picture(128, 72, true, |x, _| {
        let level = f64::from(x) / 127.0;
        codes([level, level * 0.5, 1.0 - level], true)
    }));
    let colour = |rgba: [f32; 4], opacity: f32| layer(Source::Color(rgba), Transform::NONE, opacity);
    Scene {
        width: 128,
        height: 72,
        layers: vec![
            colour([0.2, 0.4, 0.6, 1.0], 1.0),
            layer(Source::Picture(ramp, 0), Transform::NONE, 0.5),
            colour([1.0, 0.0, 0.0, 0.5], 0.8),
            colour([0.0, 1.0, 0.5, 1.0], 0.25),
            colour([0.3, 0.3, 0.3, 0.7], 0.33),
        ],
        ..Scene::default()
    }
}

fn path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join(name)
}

/// Compare `bytes` with the frame kept as `name`, as values of `size` bytes (1, or 2 little-endian),
/// within `most`; or keep them, with `COLUMBINA_WRITE_GOLDEN=1`. Gives the largest difference.
fn against(name: &str, bytes: &[u8], size: usize, most: u16) -> u16 {
    let file = path(name);
    if std::env::var("COLUMBINA_WRITE_GOLDEN").as_deref() == Ok("1") {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, bytes).unwrap();
        return 0;
    }
    let kept = std::fs::read(&file).unwrap_or_else(|error| panic!("{}: {error}", file.display()));
    assert_eq!(kept.len(), bytes.len(), "{name}: of another length");
    let value = |data: &[u8], at: usize| {
        if size == 2 {
            u16::from_le_bytes([data[at], data[at + 1]])
        } else {
            u16::from(data[at])
        }
    };
    let mut worst = 0;
    for at in (0..bytes.len()).step_by(size) {
        let (was, is) = (value(&kept, at), value(bytes, at));
        let off = was.abs_diff(is);
        assert!(off <= most, "{name}: at byte {at}, {is}, not {was}");
        worst = worst.max(off);
    }
    worst
}

#[test]
fn frames_composed_by_stages_that_change_nothing_are_those_of_phase_1() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    for (name, scene) in [("pictures", pictures()), ("colours", colours())] {
        let preview = Target::preview(&gpu, scene.width, scene.height).unwrap();
        compositor.render(&scene, &preview).unwrap();
        let rgba = preview.read_rgba().unwrap();
        let worst = against(&format!("{name}.rgba"), &rgba, 1, 1);
        eprintln!("{name}, a preview: at most {worst} of 255 off");
        for bits in [8, 10] {
            let export = Target::export(&gpu, scene.width, scene.height, bits).unwrap();
            compositor.render(&scene, &export).unwrap();
            let mut read = Picture::default();
            export.read_picture(&mut read).unwrap();
            // P010 keeps its 10 bits high in 16: a code of 10 bits is 64 of them, and 1 of 255
            // is 4 codes.
            let (size, most) = if bits == 8 { (1, 1) } else { (2, 4 << 6) };
            let luma = against(&format!("{name}.{bits}.luma"), &read.luma, size, most);
            let chroma = against(&format!("{name}.{bits}.chroma"), &read.chroma, size, most);
            eprintln!(
                "{name}, an export of {bits} bits: luma at most {luma}, chroma at most {chroma} \
                 off"
            );
        }
    }
    assert_eq!(gpu.uncaught_errors(), (0, None));
}
