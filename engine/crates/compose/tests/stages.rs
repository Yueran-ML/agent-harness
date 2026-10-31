//! The stages of colour of a frame: pictures decoded by their own matrix and range, and the input
//! transform, grade, timeline grade and output transform done where the order of `DaVinci
//! Resolve` puts them. Without an adapter the tests are skipped, unless `COLUMBINA_REQUIRE_GPU`
//! is 1.

use std::sync::Arc;
use std::time::Instant;

use columbina_compose::yuv::native_of;
use columbina_compose::{
    gpu_for_tests, ColorOp, ColorRange, ComposeError, Compositor, Gpu, LayerColor, OpTexture,
    Scene, SceneLayer, Source, Stage, Target, Text, TextureKind, Transform, YuvCoding, YuvMatrix,
};
use columbina_media::{NativePicture, Picture};
use columbina_model::project::{Rgba, TextAlign, TitleStyle};

/// The codes of R'G'B' from 0 to 1, made Y'CbCr by `coding`, in 8 bits or 10.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn codes(rgb: [f64; 3], coding: YuvCoding, ten: bool) -> [u16; 3] {
    let (kr, kb) = coding.matrix.coefficients();
    let [r, g, b] = rgb;
    let y = kr * r + (1.0 - kr - kb) * g + kb * b;
    let cb = (b - y) / (2.0 * (1.0 - kb));
    let cr = (r - y) / (2.0 * (1.0 - kr));
    let [black, luma_range, middle, chroma_range] = coding.levels(ten);
    [
        black + luma_range * y,
        middle + chroma_range * cb,
        middle + chroma_range * cr,
    ]
    .map(|code: f64| code.round() as u16)
}

/// A picture of one colour, of codes `pixel`.
fn picture(width: u32, height: u32, ten: bool, pixel: [u16; 3]) -> NativePicture {
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
    for y in 0..rows {
        for x in 0..width as usize {
            put(&mut luma, y * stride + x * bytes, pixel[0]);
        }
    }
    for y in 0..rows.div_ceil(2) {
        for x in 0..(width as usize).div_ceil(2) {
            put(&mut chroma, y * stride + x * 2 * bytes, pixel[1]);
            put(&mut chroma, y * stride + x * 2 * bytes + bytes, pixel[2]);
        }
    }
    native_of(Picture {
        frame: 0,
        width,
        height,
        bits: if ten { 10 } else { 8 },
        luma,
        chroma,
        luma_stride: stride,
        chroma_stride: stride,
    })
}

fn layer(source: Source, opacity: f32, color: LayerColor) -> SceneLayer {
    SceneLayer {
        source,
        transform: Transform::NONE,
        opacity,
        color,
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

/// A stage of one operation of `body`, reading `params`.
fn stage(body: &str, params: Vec<[f32; 4]>) -> Stage {
    Stage {
        ops: vec![ColorOp {
            body: body.into(),
            params,
            ..ColorOp::default()
        }],
    }
}

/// R'G'B'A of a preview of `scene`, 8 bits each.
fn preview(gpu: &Arc<Gpu>, compositor: &mut Compositor, scene: &Scene) -> Vec<u8> {
    let target = Target::preview(gpu, scene.width, scene.height).unwrap();
    compositor.render(scene, &target).unwrap();
    target.read_rgba().unwrap()
}

/// The planes of an export of `scene` in `bits`.
fn export(gpu: &Arc<Gpu>, compositor: &mut Compositor, scene: &Scene, bits: u8) -> Picture {
    let target = Target::export(gpu, scene.width, scene.height, bits).unwrap();
    compositor.render(scene, &target).unwrap();
    let mut picture = Picture::default();
    target.read_picture(&mut picture).unwrap();
    picture
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

#[test]
fn pictures_are_decoded_by_their_own_matrix_and_range() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let colours = [
        [0.8, 0.2, 0.1],
        [0.1, 0.6, 0.9],
        [0.5, 0.5, 0.5],
        [0.0, 1.0, 0.3],
    ];
    let codings = [
        (YuvMatrix::Bt601, ColorRange::Limited),
        (YuvMatrix::Bt601, ColorRange::Full),
        (YuvMatrix::Bt709, ColorRange::Limited),
        (YuvMatrix::Bt709, ColorRange::Full),
        (YuvMatrix::Bt2020, ColorRange::Limited),
        (YuvMatrix::Bt2020, ColorRange::Full),
    ];
    for (matrix, range) in codings {
        let coding = YuvCoding { matrix, range };
        for ten in [false, true] {
            for colour in colours {
                let picture = Arc::new(picture(16, 8, ten, codes(colour, coding, ten)));
                let color = LayerColor {
                    coding,
                    ..LayerColor::default()
                };
                let shown = preview(
                    &gpu,
                    &mut compositor,
                    &scene(
                        16,
                        8,
                        vec![layer(Source::Picture(Arc::clone(&picture), 0), 1.0, color)],
                    ),
                );
                // Codes of 8 bits are rounded by a third of a step of 8 bits of R'G'B' at most,
                // and full range by less.
                let what = format!(
                    "{matrix:?} {range:?} in {} bits, {colour:?}",
                    8 + 2 * u8::from(ten)
                );
                assert_near(at(&shown, 16, 7, 3), colour, 1.5, &what);
            }
        }
    }
    // A picture of BT.2020, full range, decoded as BT.709 of limited range is of other colours.
    let coding = YuvCoding {
        matrix: YuvMatrix::Bt2020,
        range: ColorRange::Full,
    };
    let red = Arc::new(picture(16, 8, true, codes([0.8, 0.2, 0.1], coding, true)));
    let wrong = preview(
        &gpu,
        &mut compositor,
        &scene(
            16,
            8,
            vec![layer(Source::Picture(red, 0), 1.0, LayerColor::default())],
        ),
    );
    let pixel = at(&wrong, 16, 7, 3);
    assert!(
        (f64::from(pixel[0]) - 0.8 * 255.0).abs() > 10.0,
        "decoded alike: {pixel:?}"
    );
    assert_no_errors(&gpu);
}

#[test]
fn the_same_picture_of_another_coding_is_drawn_in_the_same_colour_into_an_export() {
    // Codes of BT.601 of full range and of BT.709 of limited range for the same colour come
    // out as the same codes of the export (BT.709, limited).
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let wanted_rgb = [0.7, 0.3, 0.2];
    let mut made = Vec::new();
    for coding in [
        YuvCoding::default(),
        YuvCoding {
            matrix: YuvMatrix::Bt601,
            range: ColorRange::Full,
        },
    ] {
        let picture = Arc::new(picture(16, 8, true, codes(wanted_rgb, coding, true)));
        let color = LayerColor {
            coding,
            ..LayerColor::default()
        };
        let read = export(
            &gpu,
            &mut compositor,
            &scene(16, 8, vec![layer(Source::Picture(picture, 0), 1.0, color)]),
            10,
        );
        let word = |plane: &[u8], at: usize| u16::from_le_bytes([plane[at], plane[at + 1]]) >> 6;
        made.push([
            word(&read.luma, 3 * read.luma_stride + 8),
            word(&read.chroma, read.chroma_stride + 8),
            word(&read.chroma, read.chroma_stride + 10),
        ]);
    }
    let wanted = codes(wanted_rgb, YuvCoding::default(), true);
    for codes in &made {
        for (code, want) in codes.iter().zip(wanted) {
            assert!(code.abs_diff(want) <= 2, "{made:?}, not {wanted:?}");
        }
    }
    assert_no_errors(&gpu);
}

/// A frame of each kind of layer: a picture of 10 bits over half the canvas, a colour half
/// opaque, a title, with subtitles in a preview.
fn busy(color: &LayerColor, grade: &Stage, output: &Stage) -> Scene {
    let coding = YuvCoding::default();
    let picture = Arc::new(picture(32, 36, true, codes([0.7, 0.4, 0.2], coding, true)));
    let style = TitleStyle {
        font: String::new(),
        size: 12,
        color: Rgba([255, 255, 255, 255]),
        x: 0.5,
        y: 0.3,
        align: TextAlign::Center,
    };
    let mut scene = scene(
        64,
        36,
        vec![
            SceneLayer {
                source: Source::Picture(picture, 0),
                transform: Transform {
                    x: -0.25,
                    scale: 1.0,
                    ..Transform::NONE
                },
                opacity: 0.9,
                color: color.clone(),
            },
            layer(Source::Color([0.2, 0.5, 0.9, 0.5]), 0.8, color.clone()),
            layer(
                Source::Title(Text {
                    text: "Columbina".into(),
                    style,
                }),
                1.0,
                color.clone(),
            ),
        ],
    );
    scene.subtitles = vec!["Hello".into()];
    scene.grade = grade.clone();
    scene.output = output.clone();
    scene
}

#[test]
fn stages_whose_operations_change_nothing_leave_the_frame_as_it_is() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let none = busy(&LayerColor::default(), &Stage::default(), &Stage::default());
    let same = stage("rgb = rgb * param($P).x;", vec![[1.0; 4]]);
    let every = LayerColor {
        input: same.clone(),
        grade: Stage {
            ops: vec![same.ops[0].clone(), same.ops[0].clone()],
        },
        ..LayerColor::default()
    };
    // Each stage alone, and all of them.
    let stages = [
        busy(&every, &Stage::default(), &Stage::default()),
        busy(&LayerColor::default(), &same, &Stage::default()),
        busy(&LayerColor::default(), &Stage::default(), &same),
        busy(&every, &same, &same),
    ];
    let wanted = preview(&gpu, &mut compositor, &none);
    let wanted_export = [8, 10].map(|bits| export(&gpu, &mut compositor, &none, bits));
    for (index, staged) in stages.iter().enumerate() {
        assert!(
            preview(&gpu, &mut compositor, staged) == wanted,
            "the preview of {index}"
        );
        for (bits, wanted) in [8, 10].into_iter().zip(&wanted_export) {
            let read = export(&gpu, &mut compositor, staged, bits);
            assert!(
                read.luma == wanted.luma && read.chroma == wanted.chroma,
                "the export of {index} in {bits} bits"
            );
        }
    }
    assert_no_errors(&gpu);
}

#[test]
fn a_layer_is_graded_before_it_is_laid_over_what_is_under_it() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let invert = LayerColor {
        grade: stage("rgb = vec3<f32>(1.0) - rgb;", Vec::new()),
        ..LayerColor::default()
    };
    // White half opaque over black, inverted: black, half over black. Inverted after it is laid
    // over, it would be grey.
    let shown = preview(
        &gpu,
        &mut compositor,
        &scene(
            8,
            8,
            vec![layer(Source::Color([1.0; 4]), 0.5, invert.clone())],
        ),
    );
    assert_near(at(&shown, 8, 4, 4), [0.0; 3], 0.0, "inverted white");
    // Dark grey half opaque over white: inverted, light grey half over white.
    let grey = [0.25, 0.25, 0.25, 1.0];
    let shown = preview(
        &gpu,
        &mut compositor,
        &scene(
            8,
            8,
            vec![
                layer(Source::Color([1.0; 4]), 1.0, LayerColor::default()),
                layer(Source::Color(grey), 0.5, invert.clone()),
            ],
        ),
    );
    assert_near(
        at(&shown, 8, 4, 4),
        [0.5 * 0.75 + 0.5; 3],
        1.0,
        "inverted grey over white",
    );
    // A picture: graded once decoded, before its opacity.
    let coding = YuvCoding::default();
    let picture = Arc::new(picture(8, 8, false, codes([0.2, 0.4, 0.6], coding, false)));
    let shown = preview(
        &gpu,
        &mut compositor,
        &scene(8, 8, vec![layer(Source::Picture(picture, 0), 0.5, invert)]),
    );
    assert_near(
        at(&shown, 8, 4, 4),
        [0.4, 0.3, 0.2],
        1.5,
        "a picture inverted, half",
    );
    assert_no_errors(&gpu);
}

#[test]
fn layers_graded_alike_read_their_own_parameters_and_change_them_from_frame_to_frame() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let gain = |value: f32| LayerColor {
        grade: stage(
            "rgb = rgb * param($P).rgb;",
            vec![[value, value, value, 0.0]],
        ),
        ..LayerColor::default()
    };
    let half = |x: f64, value| SceneLayer {
        source: Source::Color([1.0; 4]),
        transform: Transform {
            x,
            scale: 0.5,
            ..Transform::NONE
        },
        opacity: 1.0,
        color: gain(value),
    };
    for (left, right) in [(0.5, 0.25), (0.75, 1.0)] {
        let shown = preview(
            &gpu,
            &mut compositor,
            &scene(64, 32, vec![half(-0.25, left), half(0.25, right)]),
        );
        let left_value = f64::from(left);
        let right_value = f64::from(right);
        assert_near(at(&shown, 64, 16, 16), [left_value; 3], 1.0, "left");
        assert_near(at(&shown, 64, 48, 16), [right_value; 3], 1.0, "right");
    }
    assert_no_errors(&gpu);
}

#[test]
fn the_timeline_grade_is_under_the_subtitles_and_the_output_transform_over_them() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let halve = stage("rgb = rgb * 0.5;", Vec::new());
    let mut subtitled = scene(
        640,
        360,
        vec![layer(
            Source::Color([0.0, 0.0, 0.0, 1.0]),
            1.0,
            LayerColor::default(),
        )],
    );
    subtitled.subtitles = vec!["IIIIIIIIII".into()];
    // The brightest pixel, of the letters, and one far from them, of the canvas.
    let measure = |pixels: &[u8]| {
        let brightest = pixels.chunks(4).map(|pixel| pixel[1]).max().unwrap();
        (brightest, at(pixels, 640, 10, 10)[1])
    };
    // Over black, which halving leaves black: the letters as they are drawn.
    let (letters, _) = measure(&preview(&gpu, &mut compositor, &subtitled));
    assert!(letters > 240, "{letters}");
    let halved = |value: u8| u8::try_from(u16::from(value).div_ceil(2)).unwrap();
    subtitled.grade = halve.clone();
    let graded = measure(&preview(&gpu, &mut compositor, &subtitled));
    assert_eq!(graded.0, letters, "the subtitles are not graded");
    subtitled.output = halve.clone();
    let output = measure(&preview(&gpu, &mut compositor, &subtitled));
    assert!(
        output.0.abs_diff(halved(letters)) <= 1,
        "the subtitles are halved by the output transform: {output:?}"
    );
    // White under them: halved twice, the canvas; halved once, the letters.
    subtitled.layers = vec![layer(Source::Color([1.0; 4]), 1.0, LayerColor::default())];
    let output = measure(&preview(&gpu, &mut compositor, &subtitled));
    assert_eq!(output.1, 64, "the canvas is halved twice");
    assert!(output.0.abs_diff(halved(letters)) <= 1, "{output:?}");
    subtitled.grade = Stage::default();
    let output = measure(&preview(&gpu, &mut compositor, &subtitled));
    // 127.5, which the card rounds either way.
    assert!(
        output.1.abs_diff(128) <= 1,
        "only the output transform: {output:?}"
    );
    // An export, which has no subtitles, goes through both: a quarter of white.
    subtitled.grade = halve;
    let read = export(&gpu, &mut compositor, &subtitled, 10);
    let luma = u16::from_le_bytes([read.luma[0], read.luma[1]]) >> 6;
    assert_eq!(luma, 64 + 219);
    assert_no_errors(&gpu);
}

#[test]
fn an_operation_reads_a_lut_of_three_dimensions_from_its_textures() {
    // A cube of 2 × 2 × 2 that inverts, read where the code of the operation says, as a LUT of
    // phase 2 will be.
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let cube = gpu.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("inverting cube"),
        size: wgpu::Extent3d {
            width: 2,
            height: 2,
            depth_or_array_layers: 2,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut texels = Vec::new();
    for b in 0..2_u8 {
        for g in 0..2_u8 {
            for r in 0..2_u8 {
                texels.extend([255 * (1 - r), 255 * (1 - g), 255 * (1 - b), 255]);
            }
        }
    }
    gpu.queue().write_texture(
        cube.as_image_copy(),
        &texels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(8),
            rows_per_image: Some(2),
        },
        cube.size(),
    );
    let lut = ColorOp {
        // From 0..1 to the centres of the first and last texels.
        body: "let size = param($P).x;\n\
               let at = (clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * (size - 1.0) + 0.5) / size;\n\
               rgb = textureSampleLevel($T0, $S, at, 0.0).rgb;"
            .into(),
        params: vec![[2.0, 0.0, 0.0, 0.0]],
        textures: vec![OpTexture {
            kind: TextureKind::D3,
            view: cube.create_view(&wgpu::TextureViewDescriptor::default()),
        }],
        ..ColorOp::default()
    };
    let graded = Stage { ops: vec![lut] };
    let mut colour = scene(
        8,
        8,
        vec![layer(
            Source::Color([0.2, 0.4, 0.6, 1.0]),
            1.0,
            LayerColor {
                grade: graded.clone(),
                ..LayerColor::default()
            },
        )],
    );
    let shown = preview(&gpu, &mut compositor, &colour);
    assert_near(at(&shown, 8, 4, 4), [0.8, 0.6, 0.4], 1.0, "a layer");
    // And over the canvas: inverted twice.
    colour.output = graded;
    let shown = preview(&gpu, &mut compositor, &colour);
    assert_near(at(&shown, 8, 4, 4), [0.2, 0.4, 0.6], 1.0, "and the canvas");
    assert_no_errors(&gpu);
}

#[test]
fn code_of_colour_that_the_card_does_not_take_is_refused_and_the_compositor_goes_on() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let broken = stage("rgb = not_a_function(rgb);", Vec::new());
    let target = Target::preview(&gpu, 8, 8).unwrap();
    let mut frame = scene(
        8,
        8,
        vec![
            layer(Source::Color([1.0; 4]), 1.0, LayerColor::default()),
            layer(
                Source::Color([0.5; 4]),
                1.0,
                LayerColor {
                    grade: broken.clone(),
                    ..LayerColor::default()
                },
            ),
        ],
    );
    let error = compositor.render(&frame, &target).unwrap_err();
    assert!(
        matches!(error, ComposeError::Layer { layer: 1, .. }),
        "{error}"
    );
    frame.layers.truncate(1);
    frame.grade = broken;
    let error = compositor.render(&frame, &target).unwrap_err();
    assert_eq!(error.reason(), "color", "{error}");
    frame.grade = Stage::default();
    compositor.render(&frame, &target).unwrap();
    assert_eq!(target.read_rgba().unwrap()[..4], [255, 255, 255, 255]);
    assert_no_errors(&gpu);
}

/// How long frames take with stages of colour that do something, against none: run with
/// `--ignored --nocapture`, in release. (`how_long_frames_take` of `tests/compose.rs` measures
/// the same scene without them, as phase 1 did.)
#[test]
#[ignore = "a measurement, not a test"]
fn how_long_frames_with_stages_of_colour_take() {
    let Some(gpu) = gpu_for_tests() else { return };
    eprintln!("on {} ({})", gpu.name(), gpu.backend());
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let wait = |gpu: &Gpu| {
        gpu.device()
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
    };
    // A grade of a few operations, as a primary correction would have.
    let lift_gamma_gain = stage(
        "rgb = max(rgb * param($P).rgb + param($P + 1u).rgb, vec3<f32>(0.0));\n\
         rgb = pow(rgb, param($P + 2u).rgb);",
        vec![
            [1.1, 1.0, 0.95, 0.0],
            [0.01, 0.0, -0.01, 0.0],
            [0.9, 1.0, 1.05, 0.0],
        ],
    );
    let graded = LayerColor {
        grade: lift_gamma_gain.clone(),
        ..LayerColor::default()
    };
    let coding = YuvCoding::default();
    for (width, height) in [(1920, 1080), (3840, 2160)] {
        let full = Arc::new(picture(
            width,
            height,
            false,
            codes([0.4, 0.5, 0.6], coding, false),
        ));
        let half = Arc::new(picture(width / 2, height / 2, true, [500, 400, 600]));
        let layers = |color: &LayerColor| {
            vec![
                layer(Source::Picture(Arc::clone(&full), 0), 1.0, color.clone()),
                SceneLayer {
                    source: Source::Picture(Arc::clone(&half), 0),
                    transform: Transform {
                        x: 0.2,
                        y: 0.1,
                        scale: 0.5,
                        rotation: 10.0,
                    },
                    opacity: 0.8,
                    color: color.clone(),
                },
                layer(Source::Color([0.0, 0.0, 0.0, 0.3]), 1.0, color.clone()),
            ]
        };
        for (what, color, stages) in [
            ("no colour", LayerColor::default(), false),
            ("each layer graded", graded.clone(), false),
            (
                "each layer graded, a timeline grade and an output transform",
                graded.clone(),
                true,
            ),
        ] {
            let mut frame = scene(width, height, layers(&color));
            if stages {
                frame.grade = lift_gamma_gain.clone();
                frame.output = lift_gamma_gain.clone();
            }
            let target = Target::preview(&gpu, width, height).unwrap();
            compositor.render(&frame, &target).unwrap();
            wait(&gpu);
            let frames = 30;
            let started = Instant::now();
            for _ in 0..frames {
                compositor.render(&frame, &target).unwrap();
                wait(&gpu);
            }
            eprintln!(
                "{width}×{height}, three layers, {what}, into a preview: {:?} a frame",
                started.elapsed() / frames
            );
            let export_target = Target::export(&gpu, width, height, 10).unwrap();
            let mut read = Picture::default();
            compositor.render(&frame, &export_target).unwrap();
            export_target.read_picture(&mut read).unwrap();
            let started = Instant::now();
            for _ in 0..frames {
                compositor.render(&frame, &export_target).unwrap();
                export_target.read_picture(&mut read).unwrap();
            }
            eprintln!(
                "{width}×{height}, {what}, into an export of 10 bits, read back: {:?} a frame",
                started.elapsed() / frames
            );
        }
    }
}
