//! Colour management on the card: OCIO's processors as operations of a layer's input transform
//! and of the output transform (an ACES output transform of 1.3 and of 2.0 among them), each
//! drawn by the compositor as OCIO's processor computes it on the processor; the DRTs drawn as
//! their references on the processor compute them; decoded Y'CbCr that keeps its superwhites
//! only where it is managed; and an export made Y'CbCr by the matrix of its output space.
//! Without a card the tests are skipped, unless `COLUMBINA_REQUIRE_GPU` is 1.

#![allow(
    clippy::cast_possible_truncation,
    reason = "floats of the card from doubles"
)]

use std::sync::Arc;

use columbina_compose::ocio::{self, CardOps};
use columbina_compose::tone::{self, Drt, Kind};
use columbina_compose::yuv::native_of;
use columbina_compose::{
    gpu_for_tests, Compositor, Gpu, LayerColor, Scene, SceneLayer, Source, Stage, Target,
    Transform, YuvCoding, YuvMatrix,
};
use columbina_lut::half;
use columbina_media::{ColorRange, NativePicture, Picture, PictureFormat};
use columbina_model::color_management::{AcesVersion, OdtOutput};

/// A picture of half floats (as of `OpenEXR`), `width` wide, of the colours `colours` in rows,
/// each as its half float.
fn floats(width: u32, colours: &[[f32; 3]]) -> NativePicture {
    let height = u32::try_from(colours.len().div_ceil(width as usize)).unwrap();
    let mut luma = Vec::new();
    for at in 0..(width * height) as usize {
        let rgb = colours.get(at).copied().unwrap_or([0.0; 3]);
        for value in [rgb[0], rgb[1], rgb[2], 1.0] {
            luma.extend_from_slice(&half::to_bits(value).to_le_bytes());
        }
    }
    NativePicture {
        width,
        height,
        format: PictureFormat::RgbaF16,
        bits: 16,
        range: ColorRange::Full,
        luma,
        luma_stride: width as usize * 8,
        ..NativePicture::default()
    }
}

/// The colours that a picture of [`floats`] holds: each made the half float nearest it.
fn halved(colours: &[[f32; 3]]) -> Vec<[f32; 3]> {
    colours
        .iter()
        .map(|rgb| rgb.map(|value| half::from_bits(half::to_bits(value))))
        .collect()
}

/// A scene of one picture filling it, of its size, with these stages.
fn scene_of(picture: NativePicture, input: Stage, output: Stage) -> Scene {
    let (width, height) = (picture.width, picture.height);
    Scene {
        width,
        height,
        layers: vec![SceneLayer {
            source: Source::Picture(Arc::new(picture), 0),
            transform: Transform::NONE,
            opacity: 1.0,
            color: LayerColor {
                coding: YuvCoding {
                    range: ColorRange::Full,
                    ..YuvCoding::default()
                },
                input,
                unclamped: true,
                ..LayerColor::default()
            },
        }],
        output,
        ..Scene::default()
    }
}

/// The canvas of a preview of `scene`, in floats: R, G, B of each pixel, unrounded.
fn canvas(gpu: &Arc<Gpu>, compositor: &mut Compositor, scene: &Scene) -> Vec<[f32; 3]> {
    let target = Target::preview(gpu, scene.width, scene.height).unwrap();
    compositor.render(scene, &target).unwrap();
    let texture = target.texture();
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
                half::from_bits(u16::from_le_bytes([
                    bytes[at + 2 * channel],
                    bytes[at + 2 * channel + 1],
                ]))
            }));
        }
    }
    pixels
}

/// Colours spread over 0 to `most`, greys and saturated ones, and a few beyond.
fn colours(most: f32, count: usize) -> Vec<[f32; 3]> {
    let mut out = Vec::new();
    let mut seed = 0x2545_f491_u32;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        f32::from(u16::try_from(seed >> 16).unwrap()) / 65535.0
    };
    for i in 0..count {
        #[allow(clippy::cast_precision_loss)]
        let grey = most * i as f32 / count as f32;
        out.push([grey, grey, grey]);
        out.push([next() * most, next() * most, next() * most]);
        out.push([next() * most, next() * 0.1 * most, next() * 0.05 * most]);
    }
    out
}

/// Whether `got` is `wanted` within `relative` of the larger of its largest channel and 1, for
/// each channel.
fn near(got: [f32; 3], wanted: [f32; 3], relative: f32) -> bool {
    let scale = wanted.iter().fold(1.0f32, |most, v| most.max(v.abs()));
    got.iter()
        .zip(wanted)
        .all(|(g, w)| (g - w).abs() <= relative * scale)
}

#[test]
fn an_input_transform_of_ocio_is_drawn_as_its_processor_computes_it_and_made_once() {
    let Some(gpu) = gpu_for_tests() else {
        return;
    };
    let config = ocio::config().unwrap();
    let ops = CardOps::new(Arc::clone(&gpu));
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    // Encoded values of the media's space, to linear light of the timeline's, and the reverse:
    // camera logs, HLG and PQ, sRGB.
    for (from, to, most) in [
        ("Apple Log", "DaVinci Intermediate WideGamut", 1.0),
        ("D-Log D-Gamut", "ACEScct", 1.0),
        ("S-Log3 S-Gamut3.Cine", "Linear Rec.2020", 1.0),
        ("Rec.2100-HLG - Display", "Linear Rec.2020", 1.0),
        (
            "Rec.2100-PQ - Display",
            "DaVinci Intermediate WideGamut",
            1.0,
        ),
        ("Linear Rec.2020", "Rec.2100-PQ - Display", 20.0),
        ("sRGB Encoded Rec.709 (sRGB)", "ACEScct", 1.0),
    ] {
        let processor = config.processor(from, to).unwrap();
        let op = ops
            .op(&processor)
            .unwrap()
            .expect("a transform that does something");
        let values = colours(most, 64);
        let picture = floats(32, &values);
        let drawn = canvas(
            &gpu,
            &mut compositor,
            &scene_of(picture, Stage { ops: vec![op] }, Stage::default()),
        );
        let mut wanted = halved(&values);
        processor.apply(&mut wanted).unwrap();
        let mut worst = 0.0f32;
        for (got, want) in drawn.iter().zip(&wanted) {
            let scale = want.iter().fold(1.0f32, |m, v| m.max(v.abs()));
            for c in 0..3 {
                worst = worst.max((got[c] - want[c]).abs() / scale);
            }
        }
        // Half floats of the canvas: within about two of their steps.
        assert!(worst < 2e-3, "{from} → {to}: {worst}");
        eprintln!("{from} → {to}: worst {worst:.2e}");
    }
    // The same transform again, of another processor that does the same: made once.
    let made = ops.count();
    let again = config
        .processor("Apple Log", "DaVinci Intermediate WideGamut")
        .unwrap();
    assert!(ops.op(&again).unwrap().is_some());
    assert_eq!(ops.count(), made);
    // A space into itself does nothing, and is no operation.
    assert!(ops
        .op(&config.processor("ACEScct", "ACEScct").unwrap())
        .unwrap()
        .is_none());
    assert_eq!(gpu.uncaught_errors().0, 0);
}

#[test]
fn the_output_transforms_of_aces_1_3_and_2_0_are_drawn_as_ocio_computes_them() {
    let Some(gpu) = gpu_for_tests() else {
        return;
    };
    let ops = CardOps::new(Arc::clone(&gpu));
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    // ACEScct values of AP1 (none below 0.0729, its 0 of linear light), from deep shadow to
    // about 100 times white (0.85), greys and colours. Beyond, where a channel is negative in AP1
    // or far brighter, the card and OCIO's processor part by up to 2.4e-2 in ACES 1.3's darkest
    // channel of the most saturated colours (as the ocio crate found of ACES 2.0, 6.7e-3).
    let values: Vec<[f32; 3]> = colours(1.0, 96)
        .into_iter()
        .map(|rgb| rgb.map(|v| 0.0729 + v * 0.78))
        .collect();
    for version in [AcesVersion::V1_3, AcesVersion::V2_0] {
        let config = ocio::builtin_config(version.config()).unwrap();
        let mut done = 0;
        for odt in version.odts() {
            let OdtOutput::View { display, view } = odt.output else {
                continue;
            };
            // SDR and HDR of each version: Rec.709, sRGB and PQ of 1000 nits.
            if !["Rec.709", "sRGB", "Rec.2020 ST2084 (1000 nits)"]
                .iter()
                .any(|name| odt.name.starts_with(name))
            {
                continue;
            }
            let processor = config
                .display_view("ACEScct", display, view, ocio::Direction::Forward)
                .unwrap();
            let op = ops.op(&processor).unwrap().unwrap();
            let picture = floats(32, &values);
            let drawn = canvas(
                &gpu,
                &mut compositor,
                &scene_of(picture, Stage::default(), Stage { ops: vec![op] }),
            );
            let mut wanted = halved(&values);
            processor.apply(&mut wanted).unwrap();
            if std::env::var("T7_DEBUG").is_ok() {
                for ((got, want), value) in drawn.iter().zip(&wanted).zip(halved(&values)) {
                    if (0..3).any(|c| (got[c] - want[c]).abs() > 5e-3) {
                        eprintln!("{} {value:?}: {got:?} vs {want:?}", odt.name);
                    }
                }
            }
            let mut errors: Vec<f32> = drawn
                .iter()
                .zip(&wanted)
                .flat_map(|(got, want)| (0..3).map(move |c| (got[c] - want[c]).abs()))
                .collect();
            errors.sort_by(f32::total_cmp);
            let (p99, worst) = (errors[errors.len() * 99 / 100], errors[errors.len() - 1]);
            // Display code values, in half floats: the 99th within 3e-3 of ACES 2.0 (whose codes
            // end at 1) and 1e-2 of ACES 1.3 (whose SDR codes go on to about 1.2, and the steep
            // foot of whose sRGB magnifies its darkest channels); every one within 5e-2. The
            // worst are the darkest channel of the most saturated colours of AP1 (the third of
            // `colours`, a red some ten times white with green and blue near 0), where the card's
            // arithmetic and OCIO's on the processor part in the steepest part of each version's
            // compression of gamut (ACES 2.0) or its glow and red modifier (1.3), as the ocio crate
            // found of ACES 2.0 at the most saturated colours beyond every gamut.
            let (most_99, most) = match version {
                AcesVersion::V2_0 => (3e-3, 5e-2),
                AcesVersion::V1_3 => (1e-2, 5e-2),
            };
            assert!(
                p99 < most_99 && worst < most,
                "{version:?} {}: 99% {p99}, worst {worst}",
                odt.name
            );
            eprintln!(
                "ACES {version:?} {}: 99% {p99:.2e}, worst {worst:.2e}",
                odt.name
            );
            done += 1;
        }
        assert!(done >= 2, "{version:?}: {done} output transforms");
    }
    assert_eq!(gpu.uncaught_errors().0, 0);
}

#[test]
fn every_drt_is_drawn_as_its_reference_computes_it() {
    let Some(gpu) = gpu_for_tests() else {
        return;
    };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let luma = [0.2627, 0.6780, 0.0593];
    for kind in [
        Kind::Simple,
        Kind::Luminance,
        Kind::Davinci,
        Kind::SaturationPreserving,
    ] {
        for (from, to) in [(1000.0, 100.0), (10000.0, 1000.0), (100.0, 1000.0)] {
            let drt = Drt {
                kind,
                from,
                to,
                luma,
                rolloff: [100.0, 1000.0],
            };
            let values = colours(from / 100.0 * 1.2, 96);
            let picture = floats(32, &values);
            let op = drt.op().unwrap();
            let drawn = canvas(
                &gpu,
                &mut compositor,
                &scene_of(picture, Stage { ops: vec![op] }, Stage::default()),
            );
            for (got, value) in drawn.iter().zip(halved(&values)) {
                let wanted = drt.apply(value);
                assert!(
                    near(*got, wanted, 2e-3),
                    "{kind:?} {from}→{to}: {value:?} → {got:?}, not {wanted:?}"
                );
            }
        }
    }
    // A matrix, a clamp and a limit of gamut, as on the processor.
    let matrix = [[0.6, 0.3, 0.1], [0.2, 0.7, 0.1], [-0.05, 0.1, 0.95]];
    let back = [[1.8, -0.8, 0.0], [-0.5, 1.5, 0.0], [0.0, 0.0, 1.0]];
    let values = colours(2.0, 32);
    let drawn = canvas(
        &gpu,
        &mut compositor,
        &scene_of(
            floats(32, &values),
            Stage {
                ops: vec![
                    tone::matrix_op(&matrix),
                    tone::limit_op(&matrix, &back),
                    tone::clamp_op(0.0, 1.5),
                ],
            },
            Stage::default(),
        ),
    );
    for (got, value) in drawn.iter().zip(halved(&values)) {
        let value = value.map(f64::from);
        let wanted = tone::apply_limit(&matrix, &back, tone::apply_matrix(&matrix, value))
            .map(|v| v.clamp(0.0, 1.5) as f32);
        assert!(near(*got, wanted, 2e-3), "{got:?} {wanted:?}");
    }
    assert_eq!(gpu.uncaught_errors().0, 0);
}

/// A picture of P010 of one colour, of 10-bit codes `pixel` (Y', Cb, Cr).
fn p010(pixel: [u16; 3]) -> NativePicture {
    let (width, height) = (16u32, 8u32);
    let stride = (width as usize * 2).next_multiple_of(64);
    let mut luma = vec![0; stride * height as usize];
    let mut chroma = vec![0; stride * height.div_ceil(2) as usize];
    for y in 0..height as usize {
        for x in 0..width as usize {
            luma[y * stride + x * 2..][..2].copy_from_slice(&(pixel[0] << 6).to_le_bytes());
        }
    }
    for y in 0..height.div_ceil(2) as usize {
        for x in 0..width.div_ceil(2) as usize {
            chroma[y * stride + x * 4..][..2].copy_from_slice(&(pixel[1] << 6).to_le_bytes());
            chroma[y * stride + x * 4 + 2..][..2].copy_from_slice(&(pixel[2] << 6).to_le_bytes());
        }
    }
    native_of(Picture {
        frame: 0,
        width,
        height,
        bits: 10,
        luma,
        chroma,
        luma_stride: stride,
        chroma_stride: stride,
    })
}

#[test]
fn superwhites_and_superblacks_are_kept_only_where_colour_is_managed() {
    let Some(gpu) = gpu_for_tests() else {
        return;
    };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    // Y' 1019 (above white at 940), 4 (below black at 64), and a red whose B' is below 0.
    for (pixel, managed) in [
        (
            [1019u16, 512, 512],
            [1019.0 - 64.0, 1019.0 - 64.0, 1019.0 - 64.0].map(|c| c / 876.0),
        ),
        ([4, 512, 512], [-60.0 / 876.0; 3]),
    ] {
        for unclamped in [false, true] {
            let scene = Scene {
                width: 16,
                height: 8,
                layers: vec![SceneLayer {
                    source: Source::Picture(Arc::new(p010(pixel)), 0),
                    transform: Transform::NONE,
                    opacity: 1.0,
                    color: LayerColor {
                        unclamped,
                        ..LayerColor::default()
                    },
                }],
                ..Scene::default()
            };
            let drawn = canvas(&gpu, &mut compositor, &scene)[3 * 16 + 8];
            let wanted = if unclamped {
                managed.map(|v: f64| v as f32)
            } else {
                managed.map(|v: f64| v.clamp(0.0, 1.0) as f32)
            };
            assert!(
                near(drawn, wanted, 2e-3),
                "{pixel:?} {unclamped}: {drawn:?}"
            );
        }
    }
    assert_eq!(gpu.uncaught_errors().0, 0);
}

#[test]
fn an_export_is_made_y_cb_cr_by_the_matrix_of_its_output_space() {
    let Some(gpu) = gpu_for_tests() else {
        return;
    };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    // A colour of linear-ish R'G'B', drawn as a picture of half floats.
    let rgb = [0.8f32, 0.3, 0.1];
    for (matrix, (kr, kb)) in [
        (YuvMatrix::Bt709, (0.2126, 0.0722)),
        (YuvMatrix::Bt2020, (0.2627, 0.0593)),
        (YuvMatrix::Bt601, (0.299, 0.114)),
    ] {
        let picture = floats(16, &[rgb; 16 * 8]);
        let scene = scene_of(picture, Stage::default(), Stage::default());
        let target = Target::export(&gpu, 16, 8, 10).unwrap();
        target.set_matrix(matrix);
        compositor.render(&scene, &target).unwrap();
        let mut read = Picture::default();
        target.read_picture(&mut read).unwrap();
        let word = |plane: &[u8], at: usize| u16::from_le_bytes([plane[at], plane[at + 1]]) >> 6;
        let [r, g, b] = halved(&[rgb])[0].map(f64::from);
        let y = kr * r + (1.0 - kr - kb) * g + kb * b;
        let wanted = [
            64.0 + 876.0 * y,
            512.0 + 896.0 * (b - y) / (2.0 * (1.0 - kb)),
            512.0 + 896.0 * (r - y) / (2.0 * (1.0 - kr)),
        ];
        let got = [
            word(&read.luma, 2 * read.luma_stride + 8),
            word(&read.chroma, read.chroma_stride + 8),
            word(&read.chroma, read.chroma_stride + 10),
        ];
        for c in 0..3 {
            assert!(
                (f64::from(got[c]) - wanted[c]).abs() <= 1.0,
                "{matrix:?}: {got:?}, not {wanted:?}"
            );
        }
    }
}
