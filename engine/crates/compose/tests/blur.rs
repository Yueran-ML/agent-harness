//! The Blur palette (pp. 3454–3456) on the card: Blur, Sharpen and Mist held to the processor
//! (`grade::evaluate_picture`) in grades of every shape, with keys and windows, on layers and on
//! the canvas; a palette that does nothing draws the frame as before, to the bit; a palette
//! keyframed makes no shader from frame to frame; and what it costs at 4K. Without an adapter
//! the tests are skipped, unless `COLUMBINA_REQUIRE_GPU` is 1.

#![allow(
    clippy::cast_precision_loss,
    clippy::too_many_lines,
    reason = "places of pixels, as the card takes them; scenes and what they are to show"
)]

use std::sync::Arc;

use columbina_compose::grade::{evaluate_picture, stage_of, GradeOptions, NoLuts};
use columbina_compose::neighbourhood::half;
use columbina_compose::yuv::native_of;
use columbina_compose::{
    gpu_for_tests, ColorRange, Compositor, Gpu, LayerColor, Scene, SceneLayer, Source, Stage,
    Target, Transform,
};
use columbina_media::{NativePicture, Picture, PictureFormat};
use columbina_model::grade::{
    Blur, BlurMode, Corrector, Grade, Interp, Key, Link, Node, NodeKind, ParallelMixer, Param,
    Port, Qualifier, Window, WindowShape, OUTPUT, SOURCE,
};

/// The most that the card may be off the processor, and the most but for 1 in 1000 of the
/// values: the neighbourhoods go through textures of half floats (which the card may round
/// towards 0, the processor to the nearest), and sharpening multiplies what they are off.
/// Measured on an RTX 4080 under Direct3D 12: at most 3.5e-3, and 2.2e-3.
const MOST: f32 = 4e-3;
const NEARLY: f32 = 2.5e-3;
/// The same for Sharpen with Level and Coring Softness: the soft threshold of the detail is
/// steep (a slope of 37 over its band, 0.02 to 0.06 of detail), and the sharpening is 4.8 times
/// the detail, so that a half float off is some 20 times as far off. Measured: 1.5e-2, and
/// 9.9e-3.
const MOST_CORED: f32 = 1.6e-2;
const NEARLY_CORED: f32 = 1.1e-2;

fn fixed(value: f64) -> Param {
    Param::Fixed(value)
}

fn corrector(id: &str, set: impl FnOnce(&mut Corrector)) -> Node {
    let mut node = Node::corrector(id);
    if let NodeKind::Corrector(corrector) = &mut node.kind {
        set(corrector);
    }
    node
}

fn link(from: &str, to: &str, input: u32) -> Link {
    Link {
        from: from.into(),
        to: to.into(),
        kind: Port::Rgb,
        input,
    }
}

fn graph(nodes: Vec<Node>, links: Vec<Link>) -> Grade {
    let mut grade = Grade { nodes, links };
    grade.sort_links();
    assert!(grade.is_complete());
    grade
}

fn serial(nodes: Vec<Node>) -> Grade {
    let mut links = Vec::new();
    let mut from = SOURCE.to_owned();
    for node in &nodes {
        links.push(link(&from, &node.id, 0));
        from.clone_from(&node.id);
    }
    links.push(link(&from, OUTPUT, 0));
    graph(nodes, links)
}

fn radius(blur: &mut Blur, r: f64) {
    blur.radius.r = fixed(r);
    blur.radius.g = fixed(r);
    blur.radius.b = fixed(r);
}

fn layer(source: Source, transform: Transform, opacity: f32, grade: Stage) -> SceneLayer {
    SceneLayer {
        source,
        transform,
        opacity,
        color: LayerColor {
            grade,
            ..LayerColor::default()
        },
    }
}

fn whole_scene(width: u32, height: u32, layers: Vec<SceneLayer>) -> Scene {
    Scene {
        width,
        height,
        layers,
        ..Scene::default()
    }
}

/// The half floats of the texture of a preview, R, G, B and A of each pixel, row after row.
fn halves(gpu: &Gpu, target: &Target) -> Vec<u16> {
    let texture = target.texture();
    let (width, height) = target.size();
    let row = (width * 8).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = gpu.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("blur test read"),
        size: u64::from(row) * u64::from(height),
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
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    gpu.device()
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let mapped = buffer.slice(..).get_mapped_range().unwrap();
    let mut out = Vec::with_capacity(width as usize * height as usize * 4);
    for line in mapped.chunks(row as usize) {
        for pair in line[..width as usize * 8].as_chunks::<2>().0 {
            out.push(u16::from_le_bytes(*pair));
        }
    }
    drop(mapped);
    buffer.unmap();
    out
}

/// The bits of the half float nearest `value`.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]
fn half_bits(value: f32) -> u16 {
    let value = half(value);
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    if value == 0.0 {
        return sign;
    }
    let exponent = ((bits >> 23) & 0xFF) as i32 - 127;
    if exponent < -14 {
        return sign | (value.abs() * 16_777_216.0) as u16;
    }
    sign | (((exponent + 15) as u16) << 10) | ((bits >> 13) & 0x3FF) as u16
}

/// The value of a half float.
fn half_value(bits: u16) -> f32 {
    let exponent = i32::from((bits >> 10) & 0x1F);
    let fraction = f32::from(bits & 0x3FF);
    let size = match exponent {
        0 => fraction * 2.0_f32.powi(-24),
        _ => (1.0 + fraction / 1024.0) * 2.0_f32.powi(exponent - 15),
    };
    if bits & 0x8000 == 0 {
        size
    } else {
        -size
    }
}

/// A picture of half floats, opaque, whose pixel at (x, y) is `pixel`; and its pixels as the
/// card holds them.
fn float_picture(
    width: u32,
    height: u32,
    pixel: impl Fn(u32, u32) -> [f32; 3],
) -> (NativePicture, Vec<[f32; 3]>) {
    let stride = width as usize * 8;
    let mut luma = Vec::with_capacity(stride * height as usize);
    let mut pixels = Vec::with_capacity(width as usize * height as usize);
    for y in 0..height {
        for x in 0..width {
            let rgb = pixel(x, y).map(half);
            pixels.push(rgb);
            for value in [rgb[0], rgb[1], rgb[2], 1.0] {
                luma.extend(half_bits(value).to_le_bytes());
            }
        }
    }
    let picture = NativePicture {
        width,
        height,
        format: PictureFormat::RgbaF16,
        bits: 16,
        range: ColorRange::Full,
        luma,
        luma_stride: stride,
        ..NativePicture::default()
    };
    (picture, pixels)
}

/// A picture with something of everything about its pixels: a slope, stripes, a disc with an
/// edge, a faint noise, of every colour.
fn busy(width: u32, height: u32) -> impl Fn(u32, u32) -> [f32; 3] {
    move |x, y| {
        let (u, v) = (x as f32 / width as f32, y as f32 / height as f32);
        let stripes = if (x / 3) % 2 == 0 { 0.12 } else { -0.12 };
        let fine = if (x / 9 + y / 7) % 2 == 0 {
            0.08
        } else {
            -0.08
        };
        let disc = if (u - 0.65).powi(2) + (v - 0.4).powi(2) < 0.04 {
            0.25
        } else {
            0.0
        };
        let noise = ((x.wrapping_mul(7919) ^ y.wrapping_mul(104_729)) % 97) as f32 / 97.0 * 0.02;
        let base = 0.2 + 0.5 * u + if v > 0.5 { stripes } else { fine } + disc + noise;
        [
            base.clamp(0.02, 0.98),
            (base * 0.8 + 0.1 * v).clamp(0.02, 0.98),
            (0.9 - base * 0.6).clamp(0.02, 0.98),
        ]
    }
}

/// The R'G'B' of a preview of `scene`, as half floats the card wrote, row after row.
fn rgb_of(gpu: &Arc<Gpu>, compositor: &mut Compositor, scene: &Scene) -> Vec<[f32; 3]> {
    let target = Target::preview(gpu, scene.width, scene.height).unwrap();
    compositor.render(scene, &target).unwrap();
    halves(gpu, &target)
        .as_chunks::<4>()
        .0
        .iter()
        .map(|[r, g, b, _]| [half_value(*r), half_value(*g), half_value(*b)])
        .collect()
}

/// How far apart two pictures are: the most, and the most but for the farthest 1 in 1000.
fn apart(card: &[[f32; 3]], processor: &[[f32; 3]]) -> (f32, f32) {
    assert_eq!(card.len(), processor.len());
    let mut offs: Vec<f32> = card
        .iter()
        .zip(processor)
        .flat_map(|(a, b)| [0, 1, 2].map(|i| (a[i] - b[i]).abs()))
        .collect();
    offs.sort_by(f32::total_cmp);
    let most = *offs.last().unwrap();
    let nearly = offs[offs.len() - 1 - offs.len() / 1000];
    (most, nearly)
}

/// Grades with the Blur palette in each mode, in branches, keyed and windowed; and what each is
/// called.
fn blurred_grades() -> Vec<(&'static str, Grade)> {
    let blur = serial(vec![
        corrector("n1", |c| c.primaries.contrast = fixed(1.2)),
        corrector("n2", |c| radius(&mut c.blur, 0.7)),
    ]);
    let sharpen = serial(vec![corrector("n1", |c| {
        c.blur.mode = BlurMode::Sharpen;
        radius(&mut c.blur, 0.1);
        c.blur.scaling.g = fixed(0.5);
        // A band of softness from 0.02 to 0.06 of detail (`CORING_REACH` 0.2).
        c.blur.level = fixed(0.3);
        c.blur.coring_softness = fixed(0.2);
        c.primaries.saturation = fixed(60.0);
    })]);
    let mist = serial(vec![
        corrector("n1", |c| {
            c.blur.mode = BlurMode::Mist;
            radius(&mut c.blur, 0.3);
            c.blur.mix = fixed(55.0);
        }),
        corrector("n2", |c| c.primaries.gain.r = fixed(1.1)),
    ]);
    let channels = serial(vec![corrector("n1", |c| {
        c.blur.linked.radius = false;
        c.blur.radius.r = fixed(0.8);
        c.blur.radius.g = fixed(0.35);
        c.blur.ratio.b = fixed(0.9);
        c.blur.radius.b = fixed(0.65);
    })]);
    let branch = graph(
        vec![
            corrector("n1", |c| c.primaries.lift.y = fixed(0.03)),
            corrector("n2", |c| {
                radius(&mut c.blur, 0.75);
                c.primaries.midtone_detail = fixed(40.0);
            }),
            corrector("n3", |c| c.primaries.saturation = fixed(30.0)),
            Node {
                id: "pm".into(),
                label: String::new(),
                enabled: true,
                locked: false,
                pos: None,
                kind: NodeKind::ParallelMixer(ParallelMixer { inputs: 2 }),
            },
        ],
        vec![
            link(SOURCE, "n1", 0),
            link("n1", "n2", 0),
            link("n1", "n3", 0),
            link("n2", "pm", 0),
            link("n3", "pm", 1),
            link("pm", OUTPUT, 0),
        ],
    );
    let windowed = serial(vec![corrector("n1", |c| {
        radius(&mut c.blur, 0.9);
        let mut window = Window::new("w", WindowShape::circular());
        window.transform.size = fixed(35.0);
        window.transform.pan = fixed(55.0);
        if let WindowShape::Circular { softness } = &mut window.shape {
            *softness = fixed(10.0);
        }
        c.windows.push(window);
    })]);
    let qualified = serial(vec![corrector("n1", |c| {
        radius(&mut c.blur, 0.2);
        let mut q = Qualifier::default();
        q.lum.low = fixed(40.0);
        q.lum.low_soft = fixed(10.0);
        c.qualifier = Some(q);
        c.key.output.gain = fixed(0.8);
    })]);
    // A blur, and a sharpening of what it made: the second neighbourhood filters the first.
    let two = serial(vec![
        corrector("n1", |c| radius(&mut c.blur, 0.7)),
        corrector("n2", |c| {
            c.blur.mode = BlurMode::Sharpen;
            radius(&mut c.blur, 0.2);
            c.primaries.gain.g = fixed(1.1);
        }),
    ]);
    // A blur, and downstream a node whose Midtone Detail reads the pixels about what it made.
    let detailed = serial(vec![
        corrector("n1", |c| {
            c.blur.mode = BlurMode::Mist;
            radius(&mut c.blur, 0.8);
            c.blur.mix = fixed(70.0);
        }),
        corrector("n2", |c| c.primaries.contrast = fixed(1.1)),
        corrector("n3", |c| c.primaries.midtone_detail = fixed(60.0)),
    ]);
    vec![
        ("blur", blur),
        ("sharpen", sharpen),
        ("two in series", two),
        ("then midtone detail", detailed),
        ("mist", mist),
        ("each channel", channels),
        ("in a parallel", branch),
        ("in a window", windowed),
        ("qualified", qualified),
    ]
}

/// `grade` with each Blur palette as it is at first.
fn without_blur(grade: &Grade) -> Grade {
    let mut grade = grade.clone();
    for node in &mut grade.nodes {
        if let NodeKind::Corrector(c) = &mut node.kind {
            c.blur = Blur::default();
        }
    }
    grade
}

#[test]
fn blur_sharpen_and_mist_on_the_card_are_those_of_the_processor() {
    let Some(gpu) = gpu_for_tests() else { return };
    eprintln!("on {} ({})", gpu.name(), gpu.backend());
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let options = GradeOptions::default();
    // A picture 1080 high, a pixel of it a pixel of the reference frame, where the detail that
    // Sharpen takes is of a pixel; and one 324 high, where every filter is narrower than that.
    for (width, height) in [(384, 1080), (432, 324)] {
        let (picture, pixels) = float_picture(width, height, busy(width, height));
        let picture = Arc::new(picture);
        let units = height as f32 / 1080.0;
        for (what, grade) in blurred_grades() {
            let wanted = evaluate_picture(
                &grade,
                0.0,
                &options,
                &NoLuts,
                &|_, rgb| rgb,
                (width, height),
                &pixels,
                units,
            );
            let flat = evaluate_picture(
                &without_blur(&grade),
                0.0,
                &options,
                &NoLuts,
                &|_, rgb| rgb,
                (width, height),
                &pixels,
                units,
            );
            let (changed, _) = apart(&wanted, &flat);
            assert!(
                height < 1080 || changed > 0.02,
                "{what}: the palette changes only {changed}"
            );
            let stage = stage_of(&grade, 0.0, &options, &NoLuts);
            let on_layer = whole_scene(
                width,
                height,
                vec![layer(
                    Source::Picture(Arc::clone(&picture), 0),
                    Transform::NONE,
                    1.0,
                    stage.clone(),
                )],
            );
            let (most_off, nearly_off) = if what == "sharpen" {
                (MOST_CORED, NEARLY_CORED)
            } else {
                (MOST, NEARLY)
            };
            let (most, nearly) = apart(&rgb_of(&gpu, &mut compositor, &on_layer), &wanted);
            eprintln!("{what}, {width}×{height}, on a layer: at most {most} off, {nearly}");
            assert!(
                most < most_off && nearly < nearly_off,
                "{what}: {most}, {nearly}"
            );
            let on_canvas = Scene {
                grade: stage,
                ..whole_scene(
                    width,
                    height,
                    vec![layer(
                        Source::Picture(Arc::clone(&picture), 0),
                        Transform::NONE,
                        1.0,
                        Stage::default(),
                    )],
                )
            };
            let (most, nearly) = apart(&rgb_of(&gpu, &mut compositor, &on_canvas), &wanted);
            eprintln!("{what}, {width}×{height}, over the canvas: at most {most} off, {nearly}");
            assert!(
                most < most_off && nearly < nearly_off,
                "{what}: {most}, {nearly}"
            );
        }
    }
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

#[test]
fn the_card_sharpens_no_detail_out_of_a_flat_field_or_a_slow_gradient() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let options = GradeOptions::default();
    // A gain that takes the colours off the half floats that the neighbourhoods keep, and the
    // most sharpening there is: Radius 0, Scaling 1, twelve times the detail.
    let grade = |sharpened: bool| {
        serial(vec![corrector("n1", |c| {
            c.primaries.gain.r = fixed(1.0123);
            c.primaries.gain.g = fixed(0.9871);
            c.primaries.gain.b = fixed(1.0377);
            if sharpened {
                c.blur.mode = BlurMode::Sharpen;
                radius(&mut c.blur, 0.0);
                c.blur.scaling.r = fixed(1.0);
                c.blur.scaling.g = fixed(1.0);
                c.blur.scaling.b = fixed(1.0);
            }
        })])
    };
    let (width, height) = (256, 1080);
    let mut shown = |pixels: &(NativePicture, Vec<[f32; 3]>), sharpened: bool| {
        let scene = whole_scene(
            width,
            height,
            vec![layer(
                Source::Picture(Arc::new(pixels.0.clone()), 0),
                Transform::NONE,
                1.0,
                stage_of(&grade(sharpened), 0.0, &options, &NoLuts),
            )],
        );
        rgb_of(&gpu, &mut compositor, &scene)
    };
    // Flat fields, within the range and beyond it: as they are without the palette, to the bit.
    for colour in [
        [0.3, 0.55, 0.7],
        [0.123_456_7, 0.654_321, 0.9],
        [0.001_234, 0.017, 0.044_44],
        [1.11, 1.99, 2.02],
        [3.3, 7.77, 12.345],
        [100.3, 777.7, 4321.0],
    ] {
        let field = float_picture(width, height, |_, _| colour);
        assert_eq!(shown(&field, true), shown(&field, false), "{colour:?}");
    }
    // Slopes so slow that a half float keeps each of their values for some 20 pixels, within
    // the range and beyond it: no bands where the half floats step.
    for (from, step) in [(0.3, 1e-5), (5.0, 2e-4)] {
        let slope = float_picture(width, height, |x, _| {
            [from + step * x as f32, 0.456_789, 2.345_678]
        });
        assert_eq!(shown(&slope, true), shown(&slope, false), "{from}");
    }
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

/// The codes of BT.709 of limited range of R'G'B' from 0 to 1, of 8 bits.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn codes(rgb: [f64; 3]) -> [u16; 3] {
    let (kr, kb) = (0.2126, 0.0722);
    let [r, g, b] = rgb;
    let y = kr * r + (1.0 - kr - kb) * g + kb * b;
    let cb = (b - y) / (2.0 * (1.0 - kb));
    let cr = (r - y) / (2.0 * (1.0 - kr));
    [16.0 + 219.0 * y, 128.0 + 224.0 * cb, 128.0 + 224.0 * cr].map(|code: f64| code.round() as u16)
}

/// A picture of 4:2:0 of 8 bits whose pixel at (x, y) has the codes that `pixel` gives.
fn yuv_picture(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u16; 3]) -> NativePicture {
    let stride = (width as usize).next_multiple_of(64);
    let rows = height as usize;
    let mut luma = vec![0; stride * rows];
    let mut chroma = vec![0; stride * rows.div_ceil(2)];
    for y in 0..height {
        for x in 0..width {
            let [code, cb, cr] = pixel(x, y);
            luma[y as usize * stride + x as usize] = u8::try_from(code).unwrap();
            if x % 2 == 0 && y % 2 == 0 {
                let at = (y / 2) as usize * stride + x as usize;
                chroma[at] = u8::try_from(cb).unwrap();
                chroma[at + 1] = u8::try_from(cr).unwrap();
            }
        }
    }
    native_of(Picture {
        frame: 0,
        width,
        height,
        bits: 8,
        luma,
        chroma,
        luma_stride: stride,
        chroma_stride: stride,
    })
}

#[test]
fn a_blur_palette_that_does_nothing_draws_the_frame_as_before_to_the_bit() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let options = GradeOptions::default();
    let picture = Arc::new(yuv_picture(256, 144, |x, y| {
        codes([f64::from((x / 3 + y / 5) % 7) / 7.0, 0.4, 0.6])
    }));
    let (float, _) = float_picture(97, 61, busy(97, 61));
    let float = Arc::new(float);
    let grade = |quiet: bool| {
        serial(vec![
            corrector("n1", |c| {
                c.primaries.contrast = fixed(1.2);
                c.curves.y.points =
                    columbina_model::grade::fixed_points(&[[0.0, 0.0], [0.3, 0.25], [1.0, 1.0]]);
                if quiet {
                    c.blur.mode = BlurMode::Mist;
                    c.blur.linked.ratio = false;
                    c.blur.scaling.r = fixed(0.9);
                    c.blur.ratio.b = fixed(0.1);
                }
            }),
            corrector("n2", |c| {
                c.primaries.saturation = fixed(70.0);
                c.key.output.gain = fixed(0.7);
                if quiet {
                    c.blur.mode = BlurMode::Sharpen;
                    c.blur.level = fixed(0.6);
                    c.blur.coring_softness = fixed(0.3);
                    c.blur.mix = fixed(10.0);
                }
            }),
        ])
    };
    let scene = |quiet: bool| {
        let stage = stage_of(&grade(quiet), 0.0, &options, &NoLuts);
        Scene {
            grade: stage.clone(),
            ..whole_scene(
                128,
                72,
                vec![
                    layer(
                        Source::Picture(Arc::clone(&picture), 0),
                        Transform::NONE,
                        1.0,
                        stage.clone(),
                    ),
                    layer(
                        Source::Picture(Arc::clone(&float), 0),
                        Transform {
                            x: 0.2,
                            y: -0.1,
                            scale: 0.6,
                            rotation: 15.0,
                        },
                        0.7,
                        stage,
                    ),
                ],
            )
        }
    };
    let made_before = compositor.pipelines_made();
    let target = Target::preview(&gpu, 128, 72).unwrap();
    compositor.render(&scene(false), &target).unwrap();
    let plain = halves(&gpu, &target);
    let after_plain = compositor.pipelines_made();
    compositor.render(&scene(true), &target).unwrap();
    let quiet = halves(&gpu, &target);
    assert_eq!(plain, quiet);
    // The same shader: none is made for it.
    assert!(after_plain > made_before);
    assert_eq!(compositor.pipelines_made(), after_plain);
    assert_eq!(compositor.neighbourhood_bytes(), 0);
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

#[test]
fn a_blur_palette_keyframed_makes_no_shader_from_frame_to_frame() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let options = GradeOptions::default();
    let (width, height) = (432, 324);
    let (picture, pixels) = float_picture(width, height, busy(width, height));
    let picture = Arc::new(picture);
    let units = height as f32 / 1080.0;
    let keys = |values: [f64; 3]| {
        Param::Keys(
            values
                .iter()
                .enumerate()
                .map(|(i, value)| Key {
                    frame: 10 * i64::try_from(i).unwrap(),
                    value: *value,
                    interp: Interp::Linear,
                })
                .collect(),
        )
    };
    let grade = serial(vec![
        corrector("n1", |c| c.primaries.contrast = fixed(1.1)),
        corrector("n2", |c| {
            c.blur.mode = BlurMode::Mist;
            c.blur.radius.r = keys([0.1, 0.5, 0.9]);
            c.blur.radius.g = keys([0.1, 0.5, 0.9]);
            c.blur.radius.b = keys([0.1, 0.5, 0.9]);
            c.blur.mix = keys([100.0, 40.0, 80.0]);
        }),
    ]);
    let scene = |frame: f64| {
        whole_scene(
            width,
            height,
            vec![layer(
                Source::Picture(Arc::clone(&picture), 0),
                Transform::NONE,
                1.0,
                stage_of(&grade, frame, &options, &NoLuts),
            )],
        )
    };
    let mut made = None;
    for frame in [0.0, 5.0, 10.0, 15.0, 20.0, 3.0] {
        let shown = rgb_of(&gpu, &mut compositor, &scene(frame));
        let wanted = evaluate_picture(
            &grade,
            frame,
            &options,
            &NoLuts,
            &|_, rgb| rgb,
            (width, height),
            &pixels,
            units,
        );
        let (most, nearly) = apart(&shown, &wanted);
        eprintln!("at {frame}: at most {most} off, {nearly}");
        assert!(
            most < MOST && nearly < NEARLY,
            "at {frame}: {most}, {nearly}"
        );
        let now = compositor.pipelines_made();
        assert_eq!(*made.get_or_insert(now), now, "at {frame}");
    }
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

/// The stage of a layer at a frame.
type StageAt<'a> = Box<dyn Fn(f64) -> Stage + 'a>;

#[test]
#[ignore = "a measure of time: run it on its own, in release, under the lock of the card"]
fn how_long_frames_with_the_blur_palette_take() {
    let Some(gpu) = gpu_for_tests() else { return };
    eprintln!("on {} ({})", gpu.name(), gpu.backend());
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let wait = |gpu: &Gpu| {
        gpu.device()
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
    };
    let options = GradeOptions::default();
    let (width, height) = (3840, 2160);
    let picture = Arc::new(yuv_picture(width, height, |x, y| {
        codes([f64::from((x / 5 + y / 3) % 7) / 7.0, 0.5, 0.4])
    }));
    let graded = |set: &dyn Fn(&mut Blur)| {
        serial(vec![corrector("n1", |c| {
            c.primaries.contrast = fixed(1.1);
            set(&mut c.blur);
        })])
    };
    let cases: Vec<(&str, Grade)> = vec![
        ("graded, Blur at its defaults", graded(&|_| {})),
        ("Blur, Radius 0.60", graded(&|b| radius(b, 0.6))),
        ("Blur, Radius 0.80", graded(&|b| radius(b, 0.8))),
        ("Blur, Radius 1.00", graded(&|b| radius(b, 1.0))),
        (
            "Sharpen, Radius 0.20, Level 0.2",
            graded(&|b| {
                b.mode = BlurMode::Sharpen;
                radius(b, 0.2);
                b.level = fixed(0.2);
            }),
        ),
        (
            "Mist, Radius 0.30, Mix 50",
            graded(&|b| {
                b.mode = BlurMode::Mist;
                radius(b, 0.3);
                b.mix = fixed(50.0);
            }),
        ),
        (
            "Blur, R G B apart",
            graded(&|b| {
                b.radius.r = fixed(0.7);
                b.radius.g = fixed(0.3);
                b.radius.b = fixed(0.6);
            }),
        ),
    ];
    let stages: Vec<(&str, StageAt)> =
        std::iter::once(("not graded", Box::new(|_: f64| Stage::default()) as StageAt))
            .chain(cases.iter().map(|(what, grade)| {
                let options = &options;
                (
                    *what,
                    Box::new(move |frame: f64| stage_of(grade, frame, options, &NoLuts)) as StageAt,
                )
            }))
            .collect();
    for (what, stage) in &stages {
        let target = Target::preview(&gpu, width, height).unwrap();
        let scene = |frame: f64| {
            whole_scene(
                width,
                height,
                vec![layer(
                    Source::Picture(Arc::clone(&picture), 0),
                    Transform::NONE,
                    1.0,
                    stage(frame),
                )],
            )
        };
        compositor.render(&scene(0.0), &target).unwrap();
        wait(&gpu);
        let frames = 30_u32;
        let started = std::time::Instant::now();
        for frame in 0..frames {
            compositor
                .render(&scene(f64::from(frame)), &target)
                .unwrap();
            wait(&gpu);
        }
        eprintln!(
            "{width}×{height}, a picture {what}, into a preview: {:?} a frame",
            started.elapsed() / frames
        );
    }
    assert_eq!(gpu.uncaught_errors(), (0, None));
}
