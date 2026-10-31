//! Qualifiers and their Matte Finesse on the card, through the compositor: the keys that read the
//! pixels about each (Pre-Filter, Clean Black and White, Blur Radius and In/Out Ratio, Morph
//! Operation, the blur of a key coming in) held to the processor, wherever they are in a graph,
//! on a layer and over the canvas; what the Highlight modes show; and keyframed settings that
//! make no shader from frame to frame. Without an adapter the tests are skipped, unless
//! `COLUMBINA_REQUIRE_GPU` is 1.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::too_many_lines,
    clippy::cast_possible_wrap,
    clippy::similar_names,
    clippy::field_reassign_with_default,
    reason = "places of pixels and bits of half floats; scenes; settings set one by one"
)]

use std::sync::Arc;

use columbina_compose::grade::{
    evaluate_picture_showing, stage_of, GradeOptions, Look, NoLuts, Prepared, Show,
};
use columbina_compose::neighbourhood::half;
use columbina_compose::{
    gpu_for_tests, ColorRange, Compositor, Gpu, LayerColor, Scene, SceneLayer, Source, Stage,
    Target, Transform,
};
use columbina_media::{NativePicture, PictureFormat};
use columbina_model::grade::{
    Corrector, Grade, Interp, Key, KeyMixer, Link, Morph, Node, NodeKind, Param, Port, Qualifier,
    QualifierMode, OUTPUT, SOURCE,
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

fn link(from: &str, to: &str, kind: Port, input: u32) -> Link {
    Link {
        from: from.into(),
        to: to.into(),
        kind,
        input,
    }
}

fn graph(nodes: Vec<Node>, links: Vec<Link>) -> Grade {
    let mut grade = Grade { nodes, links };
    grade.sort_links();
    assert!(grade.is_complete());
    grade
}

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

/// A picture of half floats, opaque, whose pixel at (x, y) is `pixel`; and its pixels as the card
/// holds them.
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

/// Something to key: a disc of orange on a slope of blue to grey, specks of orange about it,
/// holes in the disc, a band of skin, and a faint noise over all.
fn scene_to_key(width: u32, height: u32) -> impl Fn(u32, u32) -> [f32; 3] {
    move |x, y| {
        let (u, v) = (x as f32 / width as f32, y as f32 / height as f32);
        let noise = ((x.wrapping_mul(7919) ^ y.wrapping_mul(104_729)) % 97) as f32 / 97.0 * 0.03;
        let in_disc = (u - 0.4).powi(2) + (v - 0.5).powi(2) < 0.06;
        let hole = in_disc && x % 23 < 2 && y % 19 < 2;
        let speck = !in_disc && x % 31 < 2 && y % 29 < 2;
        let band = (0.75..0.85).contains(&u);
        let rgb = if (in_disc && !hole) || speck {
            [0.85, 0.45 + 0.1 * v, 0.1]
        } else if band {
            [0.7, 0.5, 0.4]
        } else {
            [0.2 + 0.3 * u, 0.25 + 0.3 * u, 0.6 - 0.2 * v]
        };
        rgb.map(|c| (c + noise).clamp(0.0, 1.0))
    }
}

fn layer(picture: &Arc<NativePicture>, grade: Stage) -> SceneLayer {
    SceneLayer {
        source: Source::Picture(Arc::clone(picture), 0),
        transform: Transform::NONE,
        opacity: 1.0,
        color: LayerColor {
            grade,
            ..LayerColor::default()
        },
    }
}

fn halves(gpu: &Gpu, target: &Target) -> Vec<u16> {
    let texture = target.texture();
    let (width, height) = target.size();
    let row = (width * 8).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = gpu.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("keys test read"),
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

/// How far apart two pictures are: the most but for the farthest 1 in 200 of their values, and
/// how many of their values are more than 0.01 apart. A key that Morph Operation takes through
/// its edge ([`columbina_compose::grade::layout::MORPH_EDGE`]) at almost exactly that edge goes
/// one way on the card and the other on the processor, as the half float of its filtered key
/// falls; such pixels are few, and alone.
fn apart(card: &[[f32; 3]], processor: &[[f32; 3]]) -> (f32, usize) {
    assert_eq!(card.len(), processor.len());
    let mut offs: Vec<f32> = card
        .iter()
        .zip(processor)
        .flat_map(|(a, b)| [0, 1, 2].map(|i| (a[i] - b[i]).abs()))
        .collect();
    offs.sort_by(f32::total_cmp);
    let nearly = offs[offs.len() - 1 - offs.len() / 200];
    let flipped = offs.iter().filter(|off| **off > 0.01).count();
    (nearly, flipped)
}

/// Whether two pictures are as near as the card and the processor are to be: within 5e-3 but
/// for 1 in 200 of their values, and fewer than 1 in 2000 more than 0.01 apart.
fn near_enough((nearly, flipped): (f32, usize), values: usize) -> bool {
    nearly < 5e-3 && flipped * 2000 < values
}

fn whole_scene(width: u32, height: u32, layers: Vec<SceneLayer>) -> Scene {
    Scene {
        width,
        height,
        layers,
        ..Scene::default()
    }
}

/// An HSL qualifier of the orange, refined by `set`.
fn orange(set: impl FnOnce(&mut Qualifier)) -> Qualifier {
    let mut q = Qualifier::default();
    q.hue.center = fixed(8.0);
    q.hue.width = fixed(10.0);
    q.hue.soft = fixed(5.0);
    q.sat.low = fixed(40.0);
    q.sat.low_soft = fixed(10.0);
    set(&mut q);
    q
}

/// Grades that read the pixels about each for their keys, each in a place of its own.
fn keyed_grades() -> Vec<(&'static str, Grade)> {
    let all = |q: &mut Qualifier| {
        q.finesse.pre_filter = fixed(30.0);
        q.finesse.clean_black = fixed(30.0);
        q.finesse.clean_white = fixed(25.0);
        q.finesse.black_clip = fixed(10.0);
        q.finesse.white_clip = fixed(90.0);
        q.finesse.blur_radius = fixed(25.0);
        q.finesse.in_out_ratio = fixed(30.0);
        q.finesse.morph = Morph::Closing;
        q.finesse.morph_radius = fixed(3.0);
    };
    let serial = |nodes: Vec<Node>| {
        let mut links = vec![];
        let mut from = SOURCE.to_owned();
        for node in &nodes {
            links.push(link(&from, &node.id, Port::Rgb, 0));
            from.clone_from(&node.id);
        }
        links.push(link(&from, OUTPUT, Port::Rgb, 0));
        graph(nodes, links)
    };
    let mut outside = serial(vec![
        corrector("n1", |c| {
            c.primaries.saturation = fixed(90.0);
            c.qualifier = Some(orange(all));
        }),
        corrector("n2", |c| {
            c.primaries.saturation = fixed(0.0);
            c.key.input.invert = true;
            c.key.input_blur.radius = fixed(20.0);
            c.key.input_blur.ratio = fixed(0.7);
        }),
    ]);
    outside.links.push(link("n1", "n2", Port::Key, 0));
    outside.sort_links();
    let mut mixed = serial(vec![
        corrector("n1", |c| {
            c.qualifier = Some(orange(|q| {
                q.finesse.blur_radius = fixed(15.0);
                q.finesse.morph = Morph::Grow;
                q.finesse.morph_radius = fixed(2.0);
            }));
        }),
        corrector("n2", |c| {
            c.qualifier = Some({
                let mut q = Qualifier::default();
                q.mode = QualifierMode::Lum;
                q.lum.low = fixed(45.0);
                q.lum.low_soft = fixed(10.0);
                q.finesse.clean_black = fixed(20.0);
                q.finesse.in_out_ratio = fixed(-50.0);
                q
            });
        }),
        corrector("n3", |c| {
            c.primaries.offset.b = fixed(60.0);
            c.qualifier = Some({
                let mut q = Qualifier::default();
                q.mode = QualifierMode::Rgb;
                q.blue.high = fixed(50.0);
                q.blue.high_soft = fixed(20.0);
                q.finesse.morph = Morph::Opening;
                q.finesse.morph_radius = fixed(2.5);
                q
            });
        }),
    ]);
    let mut mixer = KeyMixer::default();
    mixer.inputs[1].matte = true;
    mixed.nodes.push(Node {
        id: "km".into(),
        label: String::new(),
        enabled: true,
        locked: false,
        pos: None,
        kind: NodeKind::KeyMixer(mixer),
    });
    mixed.links.extend([
        link("n1", "km", Port::Key, 0),
        link("n2", "km", Port::Key, 1),
        link("km", "n3", Port::Key, 0),
    ]);
    mixed.sort_links();
    vec![
        (
            "one qualifier with all of Matte Finesse",
            serial(vec![corrector("n1", |c| {
                c.primaries.gain.b = fixed(1.6);
                c.qualifier = Some(orange(all));
            })]),
        ),
        ("an Outside node on a blurred key", outside),
        ("three qualifiers through a key mixer", mixed),
    ]
}

/// The node of each grade of [`keyed_grades`] whose key the Highlight modes are shown of.
fn shown_node(what: &str) -> &'static str {
    if what.starts_with("three") {
        "n3"
    } else if what.starts_with("an Outside") {
        "n2"
    } else {
        "n1"
    }
}

#[test]
fn keys_on_the_card_are_those_of_the_processor_wherever_they_are_in_a_graph() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let options = GradeOptions::default();
    let (width, height) = (432, 324);
    let (picture, pixels) = float_picture(width, height, scene_to_key(width, height));
    let picture = Arc::new(picture);
    let units = height as f32 / 1080.0;
    for (what, grade) in keyed_grades() {
        let prepared = Prepared::new(&grade, &options, &NoLuts);
        let node = shown_node(what);
        let looks = [
            None,
            Some(Look {
                node: node.into(),
                show: Show::Matte,
            }),
            Some(Look {
                node: node.into(),
                show: Show::Highlight,
            }),
        ];
        for look in &looks {
            let wanted = evaluate_picture_showing(
                &grade,
                0.0,
                &options,
                &NoLuts,
                &|_, rgb| rgb,
                (width, height),
                &pixels,
                units,
                look.as_ref(),
            );
            let stage = Stage {
                ops: match look {
                    None => prepared.ops_at(0.0).collect(),
                    Some(look) => prepared.ops_showing(0.0, look).unwrap(),
                },
            };
            let shown = look.as_ref().map(|look| look.show);
            let values = wanted.len() * 3;
            let on_layer = whole_scene(width, height, vec![layer(&picture, stage.clone())]);
            let off = apart(&rgb_of(&gpu, &mut compositor, &on_layer), &wanted);
            eprintln!(
                "{what}, {shown:?}, on a layer: {} off but for 1 in 200, {} of {values} over 0.01",
                off.0, off.1
            );
            assert!(near_enough(off, values), "{what}, {shown:?}: {off:?}");
            let on_canvas = Scene {
                grade: stage,
                ..whole_scene(width, height, vec![layer(&picture, Stage::default())])
            };
            let off = apart(&rgb_of(&gpu, &mut compositor, &on_canvas), &wanted);
            eprintln!(
                "{what}, {shown:?}, over the canvas: {} off, {} over 0.01",
                off.0, off.1
            );
            assert!(near_enough(off, values), "{what}, {shown:?}: {off:?}");
        }
        // The key is something: neither all nor nothing of the picture.
        let matte = evaluate_picture_showing(
            &grade,
            0.0,
            &options,
            &NoLuts,
            &|_, rgb| rgb,
            (width, height),
            &pixels,
            units,
            looks[1].as_ref(),
        );
        let keyed = matte.iter().filter(|rgb| rgb[0] > 0.5).count();
        assert!(
            keyed > pixels.len() / 50 && keyed < pixels.len() * 49 / 50,
            "{what}: {keyed} of {} keyed",
            pixels.len()
        );
    }
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

#[test]
fn keyframed_qualifiers_make_no_shader_from_frame_to_frame() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let options = GradeOptions::default();
    let (width, height) = (432, 324);
    let (picture, pixels) = float_picture(width, height, scene_to_key(width, height));
    let picture = Arc::new(picture);
    let units = height as f32 / 1080.0;
    let keys = |values: [f64; 3]| {
        Param::Keys(
            values
                .iter()
                .zip([0, 10, 20])
                .map(|(value, frame)| Key {
                    frame,
                    value: *value,
                    interp: Interp::Linear,
                })
                .collect(),
        )
    };
    let grade = graph(
        vec![corrector("n1", |c| {
            c.primaries.saturation = fixed(0.0);
            c.key.output.gain = keys([1.0, 0.5, 1.0]);
            c.qualifier = Some(orange(|q| {
                q.hue.center = keys([5.0, 10.0, 8.0]);
                q.hue.width = keys([8.0, 20.0, 12.0]);
                q.sat.low = keys([30.0, 50.0, 40.0]);
                q.finesse.clean_black = keys([0.0, 40.0, 10.0]);
                q.finesse.blur_radius = keys([0.0, 30.0, 10.0]);
                q.finesse.in_out_ratio = keys([0.0, -40.0, 60.0]);
                q.finesse.morph = Morph::Grow;
                q.finesse.morph_radius = keys([0.0, 3.0, 1.0]);
            }));
        })],
        vec![
            link(SOURCE, "n1", Port::Rgb, 0),
            link("n1", OUTPUT, Port::Rgb, 0),
        ],
    );
    let mut made = None;
    for frame in [0.0, 5.0, 10.0, 15.0, 20.0, 3.0] {
        let scene = whole_scene(
            width,
            height,
            vec![layer(&picture, stage_of(&grade, frame, &options, &NoLuts))],
        );
        let shown = rgb_of(&gpu, &mut compositor, &scene);
        let wanted = evaluate_picture_showing(
            &grade,
            frame,
            &options,
            &NoLuts,
            &|_, rgb| rgb,
            (width, height),
            &pixels,
            units,
            None,
        );
        let off = apart(&shown, &wanted);
        assert!(near_enough(off, wanted.len() * 3), "at {frame}: {off:?}");
        let now = compositor.pipelines_made();
        assert_eq!(*made.get_or_insert(now), now, "at {frame}");
    }
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

#[test]
#[ignore = "a measure of time: run it on its own, in release, under the lock of the card"]
fn how_long_frames_with_qualifiers_take() {
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
    let (picture, _) = float_picture(width, height, scene_to_key(width, height));
    let picture = Arc::new(picture);
    let graded = |set: &dyn Fn(&mut Qualifier)| {
        graph(
            vec![corrector("n1", |c| {
                c.primaries.saturation = fixed(80.0);
                c.qualifier = Some(orange(set));
            })],
            vec![
                link(SOURCE, "n1", Port::Rgb, 0),
                link("n1", OUTPUT, Port::Rgb, 0),
            ],
        )
    };
    let grades: Vec<(&str, Grade)> = vec![
        ("no qualifier", {
            let mut grade = graded(&|_| {});
            if let NodeKind::Corrector(c) = &mut grade.nodes[0].kind {
                c.qualifier = None;
            }
            grade
        }),
        ("a qualifier, no Matte Finesse", graded(&|_| {})),
        (
            "a qualifier, Clean Black and a blur of 20",
            graded(&|q| {
                q.finesse.clean_black = fixed(20.0);
                q.finesse.blur_radius = fixed(200.0);
            }),
        ),
        (
            "a qualifier, all of Matte Finesse",
            graded(&|q| {
                q.finesse.pre_filter = fixed(30.0);
                q.finesse.clean_black = fixed(20.0);
                q.finesse.clean_white = fixed(20.0);
                q.finesse.blur_radius = fixed(100.0);
                q.finesse.in_out_ratio = fixed(20.0);
                q.finesse.morph = Morph::Closing;
                q.finesse.morph_radius = fixed(4.0);
            }),
        ),
    ];
    for (what, grade) in &grades {
        let target = Target::preview(&gpu, width, height).unwrap();
        let scene = |frame: f64| {
            whole_scene(
                width,
                height,
                vec![layer(&picture, stage_of(grade, frame, &options, &NoLuts))],
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
            "{width}×{height}, {what}, into a preview: {:?} a frame",
            started.elapsed() / frames
        );
    }
    assert_eq!(gpu.uncaught_errors(), (0, None));
}
