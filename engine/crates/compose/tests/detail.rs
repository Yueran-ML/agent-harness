//! Operations that read the pixels about each one (`color.rs`, neighbourhoods): a layer whose
//! grade has such an operation is drawn into a texture of its own first, at the size of its
//! picture, filtered, and then graded and laid on the canvas; a layer without one is drawn in one
//! pass as before, to the bit. Midtone Detail (p. 3063) is the first of them. Without an adapter
//! the tests are skipped, unless `COLUMBINA_REQUIRE_GPU` is 1.

#![allow(
    clippy::cast_precision_loss,
    clippy::too_many_lines,
    reason = "places of pixels, as the card takes them; scenes and what they are to show"
)]

use std::path::PathBuf;
use std::sync::Arc;

use columbina_compose::grade::{evaluate_picture, stage_of, GradeOptions, NoLuts};
use columbina_compose::neighbourhood::{self, half, Image};
use columbina_compose::yuv::native_of;
use columbina_compose::{
    gpu_for_tests, ColorOp, ColorRange, Compositor, Filter, Gpu, LayerColor, Neighbourhood, Scene,
    SceneLayer, Source, Stage, Target, Transform,
};
use columbina_media::{NativePicture, Picture, PictureFormat};
use columbina_model::grade::{
    Composite, Corrector, Grade, Interp, Key, LayerMixer, Link, Node, NodeKind, ParallelMixer,
    Param, Port, OUTPUT, SOURCE,
};

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

fn mixer(id: &str, kind: NodeKind) -> Node {
    Node {
        id: id.into(),
        label: String::new(),
        enabled: true,
        kind,
    }
}

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

/// A picture of 4:2:0 whose pixel at (x, y) has the codes that `pixel` gives; the chroma of each
/// block of 2 × 2 is that of its top left pixel.
fn yuv_picture(
    width: u32,
    height: u32,
    ten: bool,
    pixel: impl Fn(u32, u32) -> [u16; 3],
) -> NativePicture {
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

/// The half floats of the texture of a preview, R, G, B and A of each pixel, row after row.
fn halves(gpu: &Gpu, target: &Target) -> Vec<u16> {
    let texture = target.texture();
    let (width, height) = target.size();
    let row = (width * 8).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = gpu.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("detail test read"),
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

fn golden(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join(name)
}

/// A grade of six nodes with no Midtone Detail: curves, HSL curves, a key, a layer mixer.
fn six_nodes() -> Grade {
    graph(
        vec![
            corrector("n1", |c| {
                c.primaries.contrast = fixed(1.2);
                c.primaries.lift.y = fixed(0.03);
                c.curves.y.points = vec![[0.0, 0.0], [0.3, 0.25], [0.7, 0.78], [1.0, 1.0]];
            }),
            corrector("n2", |c| {
                c.primaries.saturation = fixed(65.0);
                c.hsl.hue_vs_lum = vec![[0.0, -0.3], [0.3, 0.0], [0.6, 0.2]];
                c.soft_clip.high_soft.r = fixed(20.0);
            }),
            corrector("n3", |c| c.log.shadow.b = fixed(0.1)),
            corrector("n4", |c| {
                c.primaries.temperature = fixed(-700.0);
                c.key.output.gain = fixed(0.7);
            }),
            mixer(
                "lm",
                NodeKind::LayerMixer(LayerMixer {
                    inputs: 2,
                    composite: Composite::Overlay,
                }),
            ),
            corrector("n6", |c| c.hsl.sat_vs_lum = vec![[0.0, 0.0], [1.0, 0.15]]),
        ],
        vec![
            link(SOURCE, "n1", 0),
            link("n1", "n2", 0),
            link("n2", "n3", 0),
            link("n2", "n4", 0),
            link("n3", "lm", 0),
            link("n4", "lm", 1),
            link("lm", "n6", 0),
            link("n6", OUTPUT, 0),
        ],
    )
}

/// Graded layers of every kind (a picture made smaller than it is, a picture of 10 bits turned
/// and half opaque, a colour), and a timeline grade: none reads the pixels about another.
fn graded_scene() -> Scene {
    let options = GradeOptions::default();
    let colours = [
        [0.9, 0.1, 0.1],
        [0.1, 0.8, 0.2],
        [0.2, 0.3, 0.9],
        [0.95, 0.9, 0.2],
    ];
    let large = Arc::new(yuv_picture(256, 144, false, |x, y| {
        if y >= 96 {
            codes([f64::from(x % 2); 3], false)
        } else {
            codes(colours[((x / 32 + y / 32) % 4) as usize], false)
        }
    }));
    let ramp = Arc::new(yuv_picture(64, 48, true, |x, y| {
        let level = f64::from(x + y) / 110.0;
        codes([level, 1.0 - level, 0.5], true)
    }));
    let second = serial(vec![corrector("a", |c| {
        c.primaries.contrast = fixed(1.4);
        c.primaries.saturation = fixed(80.0);
        c.soft_clip.high.g = fixed(90.0);
    })]);
    let third = serial(vec![corrector("t", |c| {
        c.primaries.temperature = fixed(1500.0);
    })]);
    let timeline = serial(vec![corrector("g", |c| {
        c.primaries.gain.r = fixed(1.1);
        c.primaries.lift.b = fixed(0.02);
    })]);
    Scene {
        width: 128,
        height: 72,
        layers: vec![
            layer(
                Source::Picture(large, 0),
                Transform::NONE,
                1.0,
                stage_of(&six_nodes(), 0.0, &options, &NoLuts),
            ),
            layer(
                Source::Picture(ramp, 0),
                Transform {
                    x: 0.2,
                    y: -0.1,
                    scale: 0.6,
                    rotation: 15.0,
                },
                0.7,
                stage_of(&second, 0.0, &options, &NoLuts),
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
                stage_of(&third, 0.0, &options, &NoLuts),
            ),
        ],
        grade: stage_of(&timeline, 0.0, &options, &NoLuts),
        ..Scene::default()
    }
}

/// A graded frame without operations on neighbourhoods is the frame that the compositor made
/// before they came in, to the bit: kept in `tests/golden/graded.rgba16f` (written with
/// `COLUMBINA_WRITE_GOLDEN=1` by the compositor of `26c4e96`, on the card of the machine that
/// wrote it, an NVIDIA one, by Direct3D 12 and FXC; and written again once Lum Mix took the
/// master ring as one move of Y, R, G and B, which its Y-only Lift changes).
#[test]
fn a_frame_graded_without_neighbourhoods_is_drawn_as_it_was_to_the_bit() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let scene = graded_scene();
    let target = Target::preview(&gpu, scene.width, scene.height).unwrap();
    compositor.render(&scene, &target).unwrap();
    let values = halves(&gpu, &target);
    let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    let file = golden("graded.rgba16f");
    if std::env::var("COLUMBINA_WRITE_GOLDEN").as_deref() == Ok("1") {
        std::fs::write(&file, &bytes).unwrap();
        return;
    }
    let kept = std::fs::read(&file).unwrap_or_else(|error| panic!("{}: {error}", file.display()));
    assert_eq!(kept.len(), bytes.len());
    let off = kept
        .as_chunks::<2>()
        .0
        .iter()
        .zip(&values)
        .filter(|(was, is)| u16::from_le_bytes(**was) != **is)
        .count();
    assert_eq!(off, 0, "{off} half floats of {} differ", values.len());
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

// ---------------------------------------------------------------------------------------------
// Neighbourhoods on the card, held to the processor
// ---------------------------------------------------------------------------------------------

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

/// A picture of half floats (as of an EXR), opaque, whose pixel at (x, y) is `pixel`, as it is
/// stored; and its pixels as the card holds them.
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

/// A picture with something of everything about its pixels: a slope across, stripes of a few
/// widths, a disc with an edge, and a faint noise, of every colour; and a quiet part at the top
/// right, of a noise of little detail.
#[allow(clippy::cast_precision_loss)]
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
        let base = if u > 0.75 && v < 0.35 {
            0.45 + 3.0 * noise
        } else {
            0.2 + 0.5 * u + if v > 0.5 { stripes } else { fine } + disc + noise
        };
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

/// How far apart two pictures are: the most, and the most but for the farthest 1 in 1000 of
/// their values.
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

fn whole_scene(width: u32, height: u32, layers: Vec<SceneLayer>) -> Scene {
    Scene {
        width,
        height,
        layers,
        ..Scene::default()
    }
}

/// A stage that shows what its one neighbourhood filters of the picture, as it is.
fn filtered_stage(filter: Filter) -> Stage {
    Stage {
        ops: vec![ColorOp {
            body: "rgb = $N0;".into(),
            neighbourhoods: vec![Neighbourhood {
                body: "".into(),
                filter,
            }],
            ..ColorOp::default()
        }],
    }
}

#[test]
fn filters_on_the_card_are_those_of_the_processor_on_layers_and_on_the_canvas() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let (width, height) = (97, 61);
    let (picture, pixels) = float_picture(width, height, busy(width, height));
    let picture = Arc::new(picture);
    // Pixels of the picture to a pixel of the reference: it fills a canvas of its height.
    let units = height as f32 / 1080.0;
    let image = Image {
        width,
        height,
        pixels: pixels.iter().map(|[r, g, b]| [*r, *g, *b, 1.0]).collect(),
    };
    let reference = |pixels_of_picture: f32| pixels_of_picture / units;
    for filter in [
        Filter::Gaussian {
            sigma: [reference(0.7); 2],
        },
        Filter::Gaussian {
            sigma: [reference(3.0); 2],
        },
        // Taken at a pixel of 4 across and of 2 down.
        Filter::Gaussian {
            sigma: [reference(9.0), reference(5.0)],
        },
        Filter::Box {
            radius: [reference(2.0); 2],
        },
        Filter::Box {
            radius: [reference(12.0), reference(1.0)],
        },
    ] {
        let filtered = neighbourhood::filtered(&image, filter, units);
        let wanted: Vec<[f32; 3]> = (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .map(|(x, y)| {
                neighbourhood::colour_at(
                    &filtered,
                    [
                        (x as f32 + 0.5) / width as f32,
                        (y as f32 + 0.5) / height as f32,
                    ],
                )
            })
            .collect();
        let on_layer = whole_scene(
            width,
            height,
            vec![layer(
                Source::Picture(Arc::clone(&picture), 0),
                Transform::NONE,
                1.0,
                filtered_stage(filter),
            )],
        );
        let (most, _) = apart(&rgb_of(&gpu, &mut compositor, &on_layer), &wanted);
        eprintln!("{filter:?} on a layer: at most {most} off");
        assert!(most < 2e-3, "{filter:?} on a layer: {most}");
        let on_canvas = Scene {
            grade: filtered_stage(filter),
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
        let (most, _) = apart(&rgb_of(&gpu, &mut compositor, &on_canvas), &wanted);
        eprintln!("{filter:?} over the canvas: at most {most} off");
        assert!(most < 2e-3, "{filter:?} over the canvas: {most}");
    }
    // A picture turned to be shown is filtered as it is shown.
    let (stored, stored_pixels) = float_picture(height, width, busy(height, width));
    let shown: Vec<[f32; 3]> = (0..height)
        .flat_map(|y| (0..width).map(move |x| (x, y)))
        .map(|(x, y)| stored_pixels[((width - 1 - x) * height + y) as usize])
        .collect();
    let filter = Filter::Gaussian {
        sigma: [reference(6.0), reference(1.5)],
    };
    let image = Image {
        width,
        height,
        pixels: shown.iter().map(|[r, g, b]| [*r, *g, *b, 1.0]).collect(),
    };
    let filtered = neighbourhood::filtered(&image, filter, units);
    let wanted: Vec<[f32; 3]> = (0..height)
        .flat_map(|y| (0..width).map(move |x| (x, y)))
        .map(|(x, y)| {
            neighbourhood::colour_at(
                &filtered,
                [
                    (x as f32 + 0.5) / width as f32,
                    (y as f32 + 0.5) / height as f32,
                ],
            )
        })
        .collect();
    let turned = whole_scene(
        width,
        height,
        vec![layer(
            Source::Picture(Arc::new(stored), 90),
            Transform::NONE,
            1.0,
            filtered_stage(filter),
        )],
    );
    let (most, _) = apart(&rgb_of(&gpu, &mut compositor, &turned), &wanted);
    assert!(most < 2e-3, "turned: {most}");
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

fn detail(amount: f64) -> impl FnOnce(&mut Corrector) {
    move |c| c.primaries.midtone_detail = fixed(amount)
}

/// Grades with Midtone Detail early, in the middle and late, in branches, twice; and what each
/// is called.
fn detailed_grades() -> Vec<(&'static str, Grade)> {
    let early = serial(vec![
        corrector("n1", |c| {
            detail(80.0)(c);
            c.primaries.contrast = fixed(1.2);
        }),
        corrector("n2", |c| {
            c.primaries.saturation = fixed(70.0);
            c.hsl.hue_vs_sat = vec![[0.0, -0.3], [0.5, 0.2]];
        }),
        corrector("n3", |c| {
            c.primaries.gain.r = fixed(1.1);
            c.primaries.lift.b = fixed(0.03);
        }),
        corrector("n4", |c| c.soft_clip.high_soft.g = fixed(30.0)),
    ]);
    let parallel = graph(
        vec![
            corrector("n1", |c| c.primaries.contrast = fixed(1.15)),
            corrector("n2", |c| {
                detail(-100.0)(c);
                c.primaries.temperature = fixed(900.0);
            }),
            corrector("n3", |c| c.primaries.saturation = fixed(30.0)),
            mixer("pm", NodeKind::ParallelMixer(ParallelMixer { inputs: 2 })),
            corrector("n5", |c| {
                c.curves.y.points = vec![[0.0, 0.02], [0.5, 0.55], [1.0, 0.97]];
            }),
        ],
        vec![
            link(SOURCE, "n1", 0),
            link("n1", "n2", 0),
            link("n1", "n3", 0),
            link("n2", "pm", 0),
            link("n3", "pm", 1),
            link("pm", "n5", 0),
            link("n5", OUTPUT, 0),
        ],
    );
    let layered = graph(
        vec![
            corrector("n1", |c| c.primaries.gamma.y = fixed(0.05)),
            corrector("n2", |c| c.primaries.saturation = fixed(40.0)),
            corrector("n3", |c| {
                detail(100.0)(c);
                c.key.output.gain = fixed(0.6);
            }),
            mixer(
                "lm",
                NodeKind::LayerMixer(LayerMixer {
                    inputs: 2,
                    composite: Composite::Overlay,
                }),
            ),
        ],
        vec![
            link(SOURCE, "n1", 0),
            link("n1", "n2", 0),
            link("n1", "n3", 0),
            link("n2", "lm", 0),
            link("n3", "lm", 1),
            link("lm", OUTPUT, 0),
        ],
    );
    let late = serial(vec![
        corrector("n1", |c| {
            c.primaries.contrast = fixed(1.3);
            c.curves.y.points = vec![[0.0, 0.0], [0.4, 0.35], [1.0, 1.0]];
        }),
        corrector("n2", |c| c.log.midtone.r = fixed(0.08)),
        corrector("n3", |c| {
            detail(100.0)(c);
            c.primaries.shadows = fixed(30.0);
            c.primaries.saturation = fixed(60.0);
        }),
    ]);
    let twice = serial(vec![
        corrector("n1", detail(50.0)),
        corrector("n2", |c| c.primaries.gain.y = fixed(1.15)),
        corrector("n3", detail(-50.0)),
    ]);
    vec![
        ("early", early),
        ("in a parallel", parallel),
        ("in a layer", layered),
        ("late", late),
        ("twice", twice),
    ]
}

/// `grade` with no Midtone Detail anywhere.
fn without_detail(grade: &Grade) -> Grade {
    let mut grade = grade.clone();
    for node in &mut grade.nodes {
        if let NodeKind::Corrector(c) = &mut node.kind {
            c.primaries.midtone_detail = fixed(0.0);
        }
    }
    grade
}

#[test]
fn midtone_detail_on_the_card_is_that_of_the_processor_wherever_it_is_in_a_graph() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let options = GradeOptions::default();
    // 324 rows: Midtone Detail at 3 pixels of the picture, taken at every pixel; 540: at 5,
    // taken at a pixel of 2.
    for (width, height) in [(432, 324), (720, 540)] {
        let (picture, pixels) = float_picture(width, height, busy(width, height));
        let picture = Arc::new(picture);
        let units = height as f32 / 1080.0;
        for (what, grade) in detailed_grades() {
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
                &without_detail(&grade),
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
                changed > 5e-3,
                "{what}: Midtone Detail changes only {changed}"
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
            let (most, nearly) = apart(&rgb_of(&gpu, &mut compositor, &on_layer), &wanted);
            eprintln!(
                "{what}, {width}×{height}, on a layer: at most {most} off, {nearly} but for 1 in \
                 1000"
            );
            assert!(most < 1e-2 && nearly < 3e-3, "{what}: {most}, {nearly}");
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
            assert!(most < 1e-2 && nearly < 3e-3, "{what}: {most}, {nearly}");
        }
    }
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

#[test]
fn keyframed_midtone_detail_makes_no_shader_from_frame_to_frame() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let options = GradeOptions::default();
    let (width, height) = (432, 324);
    let (picture, pixels) = float_picture(width, height, busy(width, height));
    let picture = Arc::new(picture);
    let units = height as f32 / 1080.0;
    let grade = serial(vec![
        corrector("n1", |c| c.primaries.contrast = fixed(1.1)),
        corrector("n2", |c| {
            c.primaries.midtone_detail = Param::Keys(vec![
                Key {
                    frame: 0,
                    value: 0.0,
                    interp: Interp::Linear,
                },
                Key {
                    frame: 10,
                    value: 60.0,
                    interp: Interp::Linear,
                },
                Key {
                    frame: 20,
                    value: -40.0,
                    interp: Interp::Linear,
                },
            ]);
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
    let mut kept = None;
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
        assert!(most < 1e-2 && nearly < 3e-3, "at {frame}: {most}, {nearly}");
        // The pipelines and the textures of the first frame do for all.
        let now = (
            compositor.pipelines_made(),
            compositor.neighbourhood_bytes(),
        );
        assert!(now.1 > 0);
        assert_eq!(*made.get_or_insert(now.0), now.0, "at {frame}");
        assert_eq!(*kept.get_or_insert(now.1), now.1, "at {frame}");
    }
    // At 0, it is the grade without it, but for what goes through the textures in half floats.
    let without = stage_of(&without_detail(&grade), 0.0, &options, &NoLuts);
    let plain = rgb_of(
        &gpu,
        &mut compositor,
        &whole_scene(
            width,
            height,
            vec![layer(
                Source::Picture(Arc::clone(&picture), 0),
                Transform::NONE,
                1.0,
                without,
            )],
        ),
    );
    let (most, _) = apart(&rgb_of(&gpu, &mut compositor, &scene(0.0)), &plain);
    assert!(most < 1e-3, "{most}");
    // Textures not drawn into for a while are let go.
    for _ in 0..62 {
        let _ = rgb_of(&gpu, &mut compositor, &whole_scene(8, 8, vec![]));
    }
    assert_eq!(compositor.neighbourhood_bytes(), 0);
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

#[test]
fn layers_of_every_kind_read_their_neighbourhoods_at_their_own_size() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let options = GradeOptions::default();
    // A colour: a pixel alone, which Midtone Detail leaves; the same graded without it.
    let grade = serial(vec![corrector("n1", |c| {
        c.primaries.midtone_detail = fixed(100.0);
        c.primaries.saturation = fixed(70.0);
    })]);
    let colour = |grade: &Grade| {
        whole_scene(
            64,
            36,
            vec![layer(
                Source::Color([0.6, 0.4, 0.3, 0.8]),
                Transform {
                    x: 0.1,
                    y: 0.0,
                    scale: 0.5,
                    rotation: 10.0,
                },
                0.9,
                stage_of(grade, 0.0, &options, &NoLuts),
            )],
        )
    };
    let (most, _) = apart(
        &rgb_of(&gpu, &mut compositor, &colour(&grade)),
        &rgb_of(&gpu, &mut compositor, &colour(&without_detail(&grade))),
    );
    assert!(most < 2e-3, "a colour: {most}");
    // A picture drawn smaller and turned, over another, and a picture of Y'CbCr: drawn, and
    // changed by its Midtone Detail.
    let (width, height) = (432, 324);
    let picture = Arc::new(float_picture(width, height, busy(width, height)).0);
    let yuv = Arc::new(yuv_picture(256, 144, false, |x, y| {
        codes([f64::from((x / 3 + y / 5) % 2) * 0.4 + 0.3; 3], false)
    }));
    let scene = |grade: &Grade| {
        let stage = stage_of(grade, 0.0, &options, &NoLuts);
        whole_scene(
            256,
            144,
            vec![
                layer(
                    Source::Picture(Arc::clone(&yuv), 0),
                    Transform::NONE,
                    1.0,
                    stage.clone(),
                ),
                layer(
                    Source::Picture(Arc::clone(&picture), 0),
                    Transform {
                        x: -0.2,
                        y: 0.1,
                        scale: 0.4,
                        rotation: 20.0,
                    },
                    0.8,
                    stage,
                ),
            ],
        )
    };
    let (changed, _) = apart(
        &rgb_of(&gpu, &mut compositor, &scene(&grade)),
        &rgb_of(&gpu, &mut compositor, &scene(&without_detail(&grade))),
    );
    assert!(changed > 0.05, "{changed}");
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

/// The stage of a layer at a frame.
type StageAt<'a> = Box<dyn Fn(f64) -> Stage + 'a>;

#[test]
#[ignore = "a measure of time: run it on its own, in release, under the lock of the card"]
fn how_long_frames_with_midtone_detail_take() {
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
    let picture = Arc::new(yuv_picture(width, height, false, |x, y| {
        codes([f64::from((x / 5 + y / 3) % 7) / 7.0; 3], false)
    }));
    let graded = |amount: f64| {
        serial(vec![corrector("n1", |c| {
            c.primaries.contrast = fixed(1.1);
            c.primaries.midtone_detail = fixed(amount);
        })])
    };
    // Gaussians of 20 and of 2 pixels of the picture alone, as T25 will take them.
    let units = 2.0;
    let stages: Vec<(&str, StageAt)> = vec![
        ("not graded", Box::new(|_| Stage::default())),
        (
            "graded, Midtone Detail 0",
            Box::new(|frame| stage_of(&graded(0.0), frame, &options, &NoLuts)),
        ),
        (
            "graded, Midtone Detail 50",
            Box::new(|frame| stage_of(&graded(50.0), frame, &options, &NoLuts)),
        ),
        (
            "a Gaussian of 20 pixels alone",
            Box::new(|_| filtered_stage(Filter::Gaussian { sigma: [10.0; 2] })),
        ),
        (
            "a Gaussian of 2 pixels alone",
            Box::new(move |_| {
                filtered_stage(Filter::Gaussian {
                    sigma: [2.0 / units; 2],
                })
            }),
        ),
    ];
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
