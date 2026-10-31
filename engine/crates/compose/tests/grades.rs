//! Grades of the Color page through the compositor: layers and the canvas graded by graphs of
//! nodes, as the processor's reference says, and how long a frame of graded layers takes.
//! Without an adapter the tests are skipped, unless `COLUMBINA_REQUIRE_GPU` is 1.

use std::sync::Arc;
use std::time::Instant;

use columbina_compose::grade::{evaluate, stage_of, stage_of_grades, GradeOptions, NoLuts};
use columbina_compose::{
    gpu_for_tests, Compositor, Gpu, LayerColor, Scene, SceneLayer, Source, Target, Transform,
};
use columbina_media::{NativePicture, Picture};
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

fn mixer(id: &str, kind: NodeKind) -> Node {
    Node {
        id: id.into(),
        label: String::new(),
        enabled: true,
        kind,
    }
}

/// A warm, contrasty primary, then a parallel of a cooler and a desaturated node.
fn primary_and_parallel() -> Grade {
    graph(
        vec![
            corrector("n1", |c| {
                c.primaries.contrast = fixed(1.3);
                c.primaries.temperature = fixed(1200.0);
                c.primaries.gamma.y = fixed(0.1);
                c.curves.y.points = vec![[0.0, 0.05], [0.5, 0.55], [1.0, 0.95]];
            }),
            corrector("n2", |c| c.primaries.offset.b = fixed(30.0)),
            corrector("n3", |c| {
                c.primaries.saturation = fixed(20.0);
                c.key.output.gain = fixed(0.6);
            }),
            mixer("pm", NodeKind::ParallelMixer(ParallelMixer::default())),
        ],
        vec![
            link(SOURCE, "n1", 0),
            link("n1", "n2", 0),
            link("n1", "n3", 0),
            link("n2", "pm", 0),
            link("n3", "pm", 1),
            link("pm", OUTPUT, 0),
        ],
    )
}

/// Two nodes laid by a layer mixer in Screen, the lower one keyframed.
fn layered() -> Grade {
    graph(
        vec![
            corrector("a", |c| {
                c.hsl.hue_vs_sat = vec![[0.0, -0.6], [0.2, 0.0], [0.8, 0.0]];
                c.primaries.lift.r = fixed(0.05);
            }),
            corrector("b", |c| {
                c.primaries.gain.b = Param::Keys(vec![
                    Key {
                        frame: 0,
                        value: 0.5,
                        interp: Interp::Linear,
                    },
                    Key {
                        frame: 24,
                        value: 1.5,
                        interp: Interp::Linear,
                    },
                ]);
                c.key.output.gain = fixed(0.5);
            }),
            mixer(
                "lm",
                NodeKind::LayerMixer(LayerMixer {
                    inputs: 2,
                    composite: Composite::Screen,
                }),
            ),
        ],
        vec![
            link(SOURCE, "a", 0),
            link(SOURCE, "b", 0),
            link("a", "lm", 0),
            link("b", "lm", 1),
            link("lm", OUTPUT, 0),
        ],
    )
}

fn layer(source: Source, grade: columbina_compose::Stage) -> SceneLayer {
    SceneLayer {
        source,
        transform: Transform::NONE,
        opacity: 1.0,
        color: LayerColor {
            grade,
            ..LayerColor::default()
        },
    }
}

fn preview(gpu: &Arc<Gpu>, compositor: &mut Compositor, scene: &Scene) -> Vec<u8> {
    let target = Target::preview(gpu, scene.width, scene.height).unwrap();
    compositor.render(scene, &target).unwrap();
    target.read_rgba().unwrap()
}

#[track_caller]
fn assert_shown(pixels: &[u8], wanted: [f32; 3], what: &str) {
    for channel in 0..3 {
        let shown = f32::from(pixels[channel]);
        let want = wanted[channel].clamp(0.0, 1.0) * 255.0;
        assert!(
            (shown - want).abs() <= 1.0,
            "{what}: {:?}, not {:?}",
            &pixels[..3],
            wanted.map(|v| v * 255.0)
        );
    }
}

#[test]
fn graded_layers_and_canvas_are_what_the_processor_makes_of_them() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let options = GradeOptions::default();
    let colours = [
        [0.7, 0.35, 0.2],
        [0.2, 0.45, 0.6],
        [0.5, 0.5, 0.5],
        [0.9, 0.85, 0.1],
    ];
    for frame in [0.0, 12.0, 30.0] {
        for (grade, what) in [
            (primary_and_parallel(), "a primary and a parallel"),
            (layered(), "layers"),
        ] {
            for colour in colours {
                let stage = stage_of(&grade, frame, &options, &NoLuts);
                // On a layer.
                let scene = Scene {
                    width: 8,
                    height: 8,
                    layers: vec![layer(
                        Source::Color([colour[0], colour[1], colour[2], 1.0]),
                        stage.clone(),
                    )],
                    ..Scene::default()
                };
                let wanted = evaluate(&grade, frame, &options, colour);
                assert_shown(
                    &preview(&gpu, &mut compositor, &scene),
                    wanted,
                    &format!("{what} on a layer at {frame}"),
                );
                // Over the canvas, as a timeline grade.
                let scene = Scene {
                    width: 8,
                    height: 8,
                    layers: vec![layer(
                        Source::Color([colour[0], colour[1], colour[2], 1.0]),
                        columbina_compose::Stage::default(),
                    )],
                    grade: stage,
                    ..Scene::default()
                };
                assert_shown(
                    &preview(&gpu, &mut compositor, &scene),
                    wanted,
                    &format!("{what} over the canvas at {frame}"),
                );
            }
        }
    }
    // Pre-clip, clip and post-clip grades one after another on a layer.
    let (pre, clip) = (primary_and_parallel(), layered());
    let post = graph(
        vec![corrector("p", |c| c.primaries.saturation = fixed(70.0))],
        vec![link(SOURCE, "p", 0), link("p", OUTPUT, 0)],
    );
    let stage = stage_of_grades(
        &[(&pre, 5.0), (&clip, 10.0), (&post, 0.0)],
        &options,
        &NoLuts,
    );
    let colour = [0.6, 0.3, 0.25];
    let wanted = evaluate(
        &post,
        0.0,
        &options,
        evaluate(&clip, 10.0, &options, evaluate(&pre, 5.0, &options, colour)),
    );
    let scene = Scene {
        width: 8,
        height: 8,
        layers: vec![layer(
            Source::Color([colour[0], colour[1], colour[2], 1.0]),
            stage,
        )],
        ..Scene::default()
    };
    assert_shown(
        &preview(&gpu, &mut compositor, &scene),
        wanted,
        "three grades",
    );
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

/// A picture of one colour, 8-bit 4:2:0 of BT.709 limited range.
fn picture(width: u32, height: u32, codes: [u8; 3]) -> NativePicture {
    let stride = (width as usize).next_multiple_of(64);
    let rows = height as usize;
    let luma = vec![codes[0]; stride * rows];
    let mut chroma = vec![0; stride * rows.div_ceil(2)];
    for pair in chroma.as_chunks_mut::<2>().0 {
        pair.copy_from_slice(&[codes[1], codes[2]]);
    }
    columbina_compose::yuv::native_of(Picture {
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

/// A grade of six nodes: three serial correctors, two of them in parallel, a layer mixer.
fn six_nodes(seed: f64) -> Grade {
    graph(
        vec![
            corrector("n1", |c| {
                c.primaries.contrast = fixed(1.1 + seed * 0.1);
                c.primaries.lift.y = fixed(0.02);
                c.curves.y.points = vec![[0.0, 0.0], [0.3, 0.25], [0.7, 0.75], [1.0, 1.0]];
            }),
            corrector("n2", |c| {
                c.primaries.saturation = fixed(60.0);
                c.hsl.hue_vs_sat = vec![[0.0, 0.2], [0.3, 0.0], [0.6, -0.2]];
                c.soft_clip.high_soft.r = fixed(20.0);
            }),
            corrector("n3", |c| c.log.shadow.b = fixed(0.1)),
            corrector("n4", |c| {
                c.primaries.temperature = fixed(-600.0);
                c.key.output.gain = fixed(0.7);
            }),
            mixer(
                "lm",
                NodeKind::LayerMixer(LayerMixer {
                    inputs: 2,
                    composite: Composite::Overlay,
                }),
            ),
            corrector("n6", |c| c.hsl.sat_vs_lum = vec![[0.0, 0.0], [1.0, 0.1]]),
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

#[test]
#[ignore = "a measure of time: run it on its own, in release, under the lock of the card"]
fn how_long_frames_of_graded_layers_take() {
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
    let full = Arc::new(picture(width, height, [120, 110, 140]));
    let half = Arc::new(picture(width / 2, height / 2, [150, 100, 160]));
    let grades: Vec<Grade> = (0..3).map(|i| six_nodes(f64::from(i))).collect();
    let layers = |graded: bool, frame: f64| -> Vec<SceneLayer> {
        let stage = |i: usize| {
            if graded {
                stage_of(&grades[i], frame, &options, &NoLuts)
            } else {
                columbina_compose::Stage::default()
            }
        };
        vec![
            layer(Source::Picture(Arc::clone(&full), 0), stage(0)),
            SceneLayer {
                source: Source::Picture(Arc::clone(&half), 0),
                transform: Transform {
                    x: 0.2,
                    y: 0.1,
                    scale: 0.5,
                    rotation: 10.0,
                },
                opacity: 0.8,
                color: LayerColor {
                    grade: stage(1),
                    ..LayerColor::default()
                },
            },
            SceneLayer {
                opacity: 0.5,
                ..layer(Source::Picture(Arc::clone(&full), 0), stage(2))
            },
        ]
    };
    for graded in [false, true] {
        let target = Target::preview(&gpu, width, height).unwrap();
        let scene = |frame: f64| Scene {
            width,
            height,
            layers: layers(graded, frame),
            ..Scene::default()
        };
        compositor.render(&scene(0.0), &target).unwrap();
        wait(&gpu);
        let frames = 30_u32;
        let started = Instant::now();
        for frame in 0..frames {
            // Each frame its own parameters, as keyframes would give, from the same graphs.
            compositor
                .render(&scene(f64::from(frame)), &target)
                .unwrap();
            wait(&gpu);
        }
        eprintln!(
            "{width}×{height}, three layers{}, into a preview: {:?} a frame",
            if graded {
                ", each graded by six nodes"
            } else {
                ""
            },
            started.elapsed() / frames
        );
    }
    assert_eq!(gpu.uncaught_errors(), (0, None));
}
