use super::*;
use columbina_model::grade::{Window, WindowBezierPoint, WindowPoint, WindowShape};

/// Manual wall-clock evidence, with shader warm-up excluded. No CI latency assertion.
#[test]
#[ignore = "manual 4K steady-state timing; run explicitly when validating spatial rendering"]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "reading nonnegative integer addresses from the packed shader parameters"
)]
fn windows_gpu_4k_steady_state_timing() {
    use columbina_compose::grade::Prepared;
    use std::time::{Duration, Instant};
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let target = Target::preview(&gpu, 3840, 2160).unwrap();
    eprintln!(
        "T23 4K timing GPU {} compiler {:?}; 2 warmup + 6 measured frames",
        gpu.name(),
        gpu.shader_compiler()
    );
    for (label, shapes) in [
        ("no windows", vec![]),
        ("one cubic", vec![shapes().remove(3)]),
        ("five shapes", shapes()),
    ] {
        let grade = serial(vec![corrector("w", |c| {
            c.primaries.gain.r = 1.7.into();
            c.windows = shapes
                .into_iter()
                .enumerate()
                .map(|(i, shape)| Window::new(format!("w{i}"), shape))
                .collect();
        })]);
        let prepared = Prepared::new(&grade, &GradeOptions::default(), &NoLuts);
        let mut scene = whole_scene(
            3840,
            2160,
            vec![layer(
                Source::Color([0.25, 0.4, 0.6, 1.0]),
                Transform::NONE,
                1.0,
                Stage::default(),
            )],
        );
        let mut packed = Duration::ZERO;
        let mut draw = Duration::ZERO;
        let mut leaves = Vec::new();
        for frame in 0..8 {
            let begin = Instant::now();
            let ops: Vec<_> = prepared.ops_at(f64::from(frame)).collect();
            let pack_time = begin.elapsed();
            if frame == 0 && label != "no windows" {
                let params = &ops.last().unwrap().params;
                let block = columbina_compose::grade::layout::CORRECTOR as usize;
                let first = block + params[block][0] as usize;
                for i in 0..params[block][1] as usize {
                    let tree = params[first + i * 4 + 3];
                    let nodes = ((tree[2] - tree[1]) / 3.0) as usize;
                    leaves.push(nodes.div_ceil(2));
                }
            }
            scene.layers[0].color.grade = Stage { ops };
            let begin = Instant::now();
            compositor.render(&scene, &target).unwrap();
            gpu.device()
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            let draw_time = begin.elapsed();
            if frame >= 2 {
                packed += pack_time;
                draw += draw_time;
            }
        }
        eprintln!("T23 {label}: 3840x2160, edge_counts={leaves:?}, ops_at={:.3}ms/frame, submit+GPUwait={:.3}ms/frame, total={:.3}ms/frame",
            packed.as_secs_f64()*1000.0/6.0,draw.as_secs_f64()*1000.0/6.0,(packed+draw).as_secs_f64()*1000.0/6.0);
    }
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

fn shapes() -> Vec<WindowShape> {
    vec![
        WindowShape::linear(),
        WindowShape::circular(),
        WindowShape::polygon(),
        WindowShape::Curve {
            points: vec![
                WindowBezierPoint {
                    position: WindowPoint::new(-0.25, 0.0),
                    into: WindowPoint::new(0.0, 0.4),
                    out: WindowPoint::new(0.0, -0.4),
                },
                WindowBezierPoint {
                    position: WindowPoint::new(0.25, 0.0),
                    into: WindowPoint::new(0.0, -0.4),
                    out: WindowPoint::new(0.0, 0.4),
                },
            ],
            closed: true,
            inside: 5.0.into(),
            outside: 7.0.into(),
        },
        WindowShape::gradient(),
    ]
}

fn window_grade(shape: WindowShape, detail: bool) -> Grade {
    let mut nodes = vec![corrector("w", |c| {
        c.primaries.gain.r = fixed(1.7);
        let mut w = Window::new("window", shape);
        w.transform.pan = 57.0.into();
        w.transform.tilt = 44.0.into();
        w.transform.rotate = 24.0.into();
        w.opacity = 83.0.into();
        c.windows.push(w);
    })];
    if detail {
        nodes.push(corrector("after", |c| {
            c.primaries.midtone_detail = 50.0.into();
        }));
    }
    serial(nodes)
}

#[test]
fn windows_gpu_shapes_match_cpu_in_clip_and_timeline_and_keep_the_pipeline() {
    let Some(gpu) = gpu_for_tests() else { return };
    eprintln!(
        "T23 GPU {} compiler {:?}",
        gpu.name(),
        gpu.shader_compiler()
    );
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let (w, h) = (96, 64);
    let (p, pixels) = float_picture(w, h, |_, _| [0.25, 0.4, 1.5]);
    let source = Arc::new(p);
    let mut pipelines = None;
    for shape in shapes() {
        let grade = window_grade(shape, false);
        let wanted = evaluate_picture(
            &grade,
            0.0,
            &GradeOptions::default(),
            &NoLuts,
            &|_, c| c,
            (w, h),
            &pixels,
            h as f32 / 1080.0,
        );
        for timeline in [false, true] {
            let op = stage_of(&grade, 0.0, &GradeOptions::default(), &NoLuts);
            let mut scene = whole_scene(
                w,
                h,
                vec![layer(
                    Source::Picture(Arc::clone(&source), 0),
                    Transform::NONE,
                    1.0,
                    if timeline {
                        Stage::default()
                    } else {
                        op.clone()
                    },
                )],
            );
            if timeline {
                scene.grade = op;
            }
            let got = rgb_of(&gpu, &mut compositor, &scene);
            let (error, _) = apart(&got, &wanted);
            assert!(error < 0.0015, "timeline={timeline} error={error}");
        }
        if let Some(count) = pipelines {
            assert_eq!(compositor.pipelines_made(), count);
        }
        pipelines = Some(compositor.pipelines_made());
    }
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

#[test]
fn windows_gpu_are_in_shown_source_space_after_rgb_and_yuv_metadata_rotation() {
    let Some(gpu) = gpu_for_tests() else { return };
    eprintln!("T23 compiler {:?}", gpu.shader_compiler());
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    for yuv in [false, true] {
        let (w, h) = (96, 48);
        let picture = if yuv {
            yuv_picture(w, h, false, |_, _| codes([0.25; 3], false))
        } else {
            float_picture(w, h, |_, _| [0.25; 3]).0
        };
        let source = Arc::new(picture);
        for rotation in [0, 90, 180, 270] {
            let (sw, sh) = if rotation % 180 == 0 { (w, h) } else { (h, w) };
            let base = whole_scene(
                sw,
                sh,
                vec![layer(
                    Source::Picture(Arc::clone(&source), rotation),
                    Transform::NONE,
                    1.0,
                    Stage::default(),
                )],
            );
            let pixels = rgb_of(&gpu, &mut compositor, &base);
            let grade = window_grade(WindowShape::circular(), true);
            let wanted = evaluate_picture(
                &grade,
                0.0,
                &GradeOptions::default(),
                &NoLuts,
                &|_, c| c,
                (sw, sh),
                &pixels,
                sh as f32 / 1080.0,
            );
            let mut scene = base;
            scene.layers[0].color.grade = stage_of(&grade, 0.0, &GradeOptions::default(), &NoLuts);
            let got = rgb_of(&gpu, &mut compositor, &scene);
            let (error, _) = apart(&got, &wanted);
            assert!(
                error < 0.0015,
                "yuv={yuv}, rotation={rotation}, error={error}"
            );
        }
    }
}

#[test]
fn windows_gpu_uniform_colour_neighbourhood_reads_the_preceding_spatial_result() {
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let (w, h) = (96, 64);
    let grade = window_grade(WindowShape::circular(), true);
    let pixels = vec![[0.25; 3]; (w * h) as usize];
    let wanted = evaluate_picture(
        &grade,
        0.0,
        &GradeOptions::default(),
        &NoLuts,
        &|_, c| c,
        (w, h),
        &pixels,
        h as f32 / 1080.0,
    );
    let scene = whole_scene(
        w,
        h,
        vec![layer(
            Source::Color([0.25, 0.25, 0.25, 1.0]),
            Transform::NONE,
            1.0,
            stage_of(&grade, 0.0, &GradeOptions::default(), &NoLuts),
        )],
    );
    let got = rgb_of(&gpu, &mut compositor, &scene);
    let (error, _) = apart(&got, &wanted);
    assert!(error < 0.0015, "spatial constant source error={error}");
}

#[test]
fn windows_gpu_qualifier_external_key_mixers_and_highlight_share_the_same_mask() {
    use columbina_compose::grade::{evaluate_picture_showing, Look, Prepared, Show};
    use columbina_model::grade::{KeyMixer, Qualifier};
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let (w, h) = (80, 48);
    let (pic, pixels) = float_picture(w, h, |_, _| [0.25; 3]);
    let source = Arc::new(pic);
    for matte in [false, true] {
        let grade = graph(
            vec![
                corrector("w", |c| {
                    c.primaries.gain.r = 1.7.into();
                    c.qualifier = Some(Qualifier::default());
                    c.key.qualifier.gain = 0.3.into();
                    c.key.qualifier.matte = matte;
                    c.windows.push(Window::new("a", WindowShape::circular()));
                }),
                corrector("ext", |c| {
                    c.key.output.gain = 0.4.into();
                }),
                mixer("km", NodeKind::KeyMixer(KeyMixer::default())),
                corrector("apply", |c| {
                    c.key.input.matte = matte;
                    c.primaries.gain.b = 1.5.into();
                    c.windows.push(Window::new("b", WindowShape::gradient()));
                }),
                mixer(
                    "lm",
                    NodeKind::LayerMixer(LayerMixer {
                        inputs: 2,
                        composite: Composite::Normal,
                    }),
                ),
            ],
            vec![
                link(SOURCE, "w", 0),
                link(SOURCE, "ext", 0),
                link("ext", "apply", 0),
                Link {
                    from: "w".into(),
                    to: "km".into(),
                    kind: Port::Key,
                    input: 0,
                },
                Link {
                    from: "ext".into(),
                    to: "km".into(),
                    kind: Port::Key,
                    input: 1,
                },
                Link {
                    from: "km".into(),
                    to: "apply".into(),
                    kind: Port::Key,
                    input: 0,
                },
                link("w", "lm", 0),
                link("apply", "lm", 1),
                link("lm", OUTPUT, 0),
            ],
        );
        for look in [
            None,
            Some(Look {
                node: "w".into(),
                show: Show::Highlight,
            }),
            Some(Look {
                node: "apply".into(),
                show: Show::Matte,
            }),
        ] {
            let wanted = evaluate_picture_showing(
                &grade,
                0.0,
                &GradeOptions::default(),
                &NoLuts,
                &|_, c| c,
                (w, h),
                &pixels,
                h as f32 / 1080.0,
                look.as_ref(),
            );
            let prepared = Prepared::new(&grade, &GradeOptions::default(), &NoLuts);
            let ops = if let Some(look) = &look {
                prepared.ops_showing(0.0, look).unwrap()
            } else {
                prepared.ops_at(0.0).collect()
            };
            let scene = whole_scene(
                w,
                h,
                vec![layer(
                    Source::Picture(Arc::clone(&source), 0),
                    Transform::NONE,
                    1.0,
                    Stage { ops },
                )],
            );
            let got = rgb_of(&gpu, &mut compositor, &scene);
            let (error, _) = apart(&got, &wanted);
            assert!(error < 0.001, "matte={matte} look={look:?} error={error}");
        }
    }
}

#[test]
fn windows_gpu_follow_clip_placement_and_animated_values_without_compiling() {
    use columbina_compose::geometry::{fit, placement};
    use columbina_compose::grade::Prepared;
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let (w, h) = (128, 80);
    let source = Arc::new(float_picture(80, 40, |_, _| [0.25; 3]).0);
    let grade = serial(vec![corrector("w", |c| {
        c.primaries.offset.r = 50.0.into();
        let mut window = Window::new(
            "a",
            WindowShape::Circular {
                softness: 0.0.into(),
            },
        );
        window.transform.pan = Param::Keys(vec![
            Key {
                frame: 0,
                value: 35.0,
                interp: Interp::Linear,
            },
            Key {
                frame: 10,
                value: 65.0,
                interp: Interp::Linear,
            },
        ]);
        c.windows.push(window);
    })]);
    let prepared = Prepared::new(&grade, &GradeOptions::default(), &NoLuts);
    let mut pipelines = None;
    for frame in [0.0, 3.0, 10.0] {
        let transform = Transform {
            x: 0.13,
            y: -0.07,
            scale: 0.6,
            rotation: 31.0,
        };
        let scene = whole_scene(
            w,
            h,
            vec![layer(
                Source::Picture(Arc::clone(&source), 0),
                transform,
                0.7,
                Stage {
                    ops: prepared.ops_at(frame).collect(),
                },
            )],
        );
        let got = rgb_of(&gpu, &mut compositor, &scene);
        let canvas = (f64::from(w), f64::from(h));
        let fitted = fit(canvas, (80.0, 40.0));
        let inverse = placement(canvas, fitted, &transform).inverse().unwrap();
        let mut checked = 0;
        for y in 0..h {
            for x in 0..w {
                let (u, v) = inverse.apply((f64::from(x) + 0.5, f64::from(y) + 0.5));
                let (u, v) = (u / fitted.0, v / fitted.1);
                if !(0.03..0.97).contains(&u) || !(0.03..0.97).contains(&v) {
                    continue;
                }
                let radius = ((u - (0.35 + 0.03 * frame)) * 2.0).hypot(v - 0.5);
                if (radius - 0.25).abs() < 0.003 {
                    continue;
                }
                let expected = if radius < 0.25 { 0.5 * 0.7 } else { 0.25 * 0.7 };
                assert!((f64::from(got[(y * w + x) as usize][0]) - expected).abs() < 0.001);
                checked += 1;
            }
        }
        assert!(checked > 500);
        if let Some(count) = pipelines {
            assert_eq!(compositor.pipelines_made(), count);
        }
        pipelines = Some(compositor.pipelines_made());
    }
}

#[test]
fn windows_gpu_mix_with_windowless_pre_and_post_clip_grades() {
    use columbina_compose::grade::stage_of_grades;
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let (w, h) = (80, 48);
    let (picture, pixels) = float_picture(w, h, |_, _| [0.25; 3]);
    let pre = serial(vec![corrector("pre", |c| c.primaries.gain.r = 1.2.into())]);
    let window = window_grade(WindowShape::circular(), false);
    let post = serial(vec![corrector("post", |c| c.primaries.gain.b = 1.1.into())]);
    let mut wanted = pixels;
    for grade in [&pre, &window, &post] {
        wanted = evaluate_picture(
            grade,
            0.0,
            &GradeOptions::default(),
            &NoLuts,
            &|_, c| c,
            (w, h),
            &wanted,
            h as f32 / 1080.0,
        );
    }
    let stage = stage_of_grades(
        &[(&pre, 0.0), (&window, 0.0), (&post, 0.0)],
        &GradeOptions::default(),
        &NoLuts,
    );
    let scene = whole_scene(
        w,
        h,
        vec![layer(
            Source::Picture(Arc::new(picture), 0),
            Transform::NONE,
            1.0,
            stage,
        )],
    );
    let got = rgb_of(&gpu, &mut compositor, &scene);
    assert!(apart(&got, &wanted).0 < 0.001);
}

#[test]
fn windows_gpu_title_uses_its_transparent_canvas_before_visible_texture_cropping() {
    use columbina_compose::Text;
    use columbina_model::project::{Rgba, Styled, TextAlign};
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let grade = serial(vec![corrector("w", |c| {
        c.primaries.offset.r = 50.0.into();
        let mut window = Window::new(
            "w1",
            WindowShape::Gradient {
                softness: 0.0.into(),
            },
        );
        window.transform.rotate = 90.0.into();
        c.windows.push(window);
    })]);
    let (w, h) = (256, 128);
    for text in [
        "IIIIIIIIIIIIIIIIIIII",
        "IIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIII",
    ] {
        for x in [0.0, -0.4] {
            let title = Source::Title(Text {
                text: text.into(),
                style: Styled {
                    font: String::new(),
                    size: 28.0,
                    color: Rgba([64, 64, 64, 128]),
                    x: 0.5,
                    y: 0.5,
                    align: TextAlign::Center,
                },
            });
            let transform = Transform {
                x,
                ..Transform::NONE
            };
            let mut scene = whole_scene(w, h, vec![layer(title, transform, 1.0, Stage::default())]);
            let baseline = rgb_of(&gpu, &mut compositor, &scene);
            let mut global = grade.clone();
            let NodeKind::Corrector(c) = &mut global.nodes[0].kind else {
                unreachable!()
            };
            c.windows.clear();
            scene.layers[0].color.grade = stage_of(&global, 0.0, &GradeOptions::default(), &NoLuts);
            let fully_graded = rgb_of(&gpu, &mut compositor, &scene);
            scene.layers[0].color.grade = stage_of(&grade, 0.0, &GradeOptions::default(), &NoLuts);
            let got = rgb_of(&gpu, &mut compositor, &scene);
            let mut checked = 0;
            for (i, (base, pixel)) in baseline.iter().zip(&got).enumerate() {
                if base[0] < 0.02 {
                    continue;
                }
                let u = (i % w as usize) as f64 / f64::from(w) + 0.5 / f64::from(w) - x;
                if (u - 0.5).abs() < 0.005 {
                    continue;
                }
                let expected = if u > 0.5 { fully_graded[i][0] } else { base[0] };
                assert!(
                    (pixel[0] - expected).abs() < 0.001,
                    "x={x}, source_u={u}, pixel={pixel:?}, wanted_red={expected}"
                );
                assert!(
                    (pixel[1] - base[1]).abs() < 0.001,
                    "the title's alpha must be retained"
                );
                checked += 1;
            }
            assert!(checked > 100);
        }
    }
}

#[test]
fn windows_gpu_title_neighbourhood_keeps_window_canvas_mapping_when_cropped() {
    use columbina_compose::Text;
    use columbina_model::project::{Rgba, Styled, TextAlign};
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let grade = serial(vec![
        corrector("w", |c| {
            c.primaries.offset.r = 50.0.into();
            let mut window = Window::new(
                "w1",
                WindowShape::Gradient {
                    softness: 30.0.into(),
                },
            );
            window.transform.rotate = 90.0.into();
            c.windows.push(window);
        }),
        corrector("detail", |c| c.primaries.midtone_detail = 70.0.into()),
    ]);
    let title = Source::Title(Text {
        text: "MMMMMMMMMM".into(),
        style: Styled {
            font: String::new(),
            size: 28.0,
            color: Rgba([64, 64, 64, 128]),
            x: 0.5,
            y: 0.5,
            align: TextAlign::Center,
        },
    });
    let mut scene = whole_scene(
        320,
        160,
        vec![layer(
            title,
            Transform::NONE,
            1.0,
            stage_of(&grade, 0.0, &GradeOptions::default(), &NoLuts),
        )],
    );
    let full = rgb_of(&gpu, &mut compositor, &scene);
    scene.layers[0].transform.x = -0.3; // Exactly 96 pixels, changing the visible texture crop.
    let cropped = rgb_of(&gpu, &mut compositor, &scene);
    let mut checked = 0;
    for y in 0..160 {
        for x in 8..224 {
            let a = cropped[y * 320 + x];
            let b = full[y * 320 + x + 96];
            if b[1] < 0.01 {
                continue;
            }
            assert!(
                a.iter().zip(b).all(|(a, b)| (*a - b).abs() < 0.002),
                "({x},{y}) {a:?} != {b:?}"
            );
            checked += 1;
        }
    }
    assert!(checked > 200);
}

#[test]
fn windows_gpu_mixed_grades_keep_pipelines_when_prior_window_tails_change() {
    use columbina_compose::grade::{stage_of_grades, Luts};
    struct ParameterLut;
    impl Luts for ParameterLut {
        fn lut(&self, name: &str) -> Option<ColorOp> {
            (name == "parameters.cube").then(|| ColorOp {
                body: "rgb = rgb * param($P).rgb + param($P + 1u).rgb;".into(),
                library: "fn t23_parameter_lut_library() {}".into(),
                params: vec![[0.9, 1.1, 0.95, 0.0], [0.03, 0.02, 0.01, 0.0]],
                ..ColorOp::default()
            })
        }
    }
    let lut = |_: &str, c: [f32; 3]| [c[0] * 0.9 + 0.03, c[1] * 1.1 + 0.02, c[2] * 0.95 + 0.01];
    let Some(gpu) = gpu_for_tests() else { return };
    eprintln!(
        "T23 mixed parameter bases compiler {:?}",
        gpu.shader_compiler()
    );
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let (width, height) = (96, 64);
    let (picture, pixels) = float_picture(width, height, |_, _| [0.25, 0.4, 0.6]);
    let source = Arc::new(picture);
    let mut before_grade = window_grade(shapes().remove(3), false);
    let NodeKind::Corrector(c) = &mut before_grade.nodes[0].kind else {
        unreachable!()
    };
    c.windows[0].transform.size = Param::Keys(vec![
        Key {
            frame: 0,
            value: 0.0,
            interp: Interp::Linear,
        },
        Key {
            frame: 10,
            value: 50.0,
            interp: Interp::Linear,
        },
    ]);
    let after_grade = serial(vec![corrector("after", |c| {
        c.primaries.midtone_detail = 50.0.into();
        c.lut = Some("parameters.cube".into());
    })]);
    let last = serial(vec![corrector("post", |c| {
        c.primaries.gain.b = 1.3.into();
    })]);
    let mut cases = vec![
        (before_grade.clone(), 0.0),
        (before_grade.clone(), 5.0),
        (before_grade.clone(), 10.0),
    ];
    let NodeKind::Corrector(c) = &mut before_grade.nodes[0].kind else {
        unreachable!()
    };
    c.windows
        .push(Window::new("second", WindowShape::circular()));
    cases.push((before_grade.clone(), 10.0));
    for count in [3, 7] {
        let NodeKind::Corrector(c) = &mut before_grade.nodes[0].kind else {
            unreachable!()
        };
        c.windows[0].shape = WindowShape::Polygon {
            points: (0..count)
                .map(|i| {
                    let angle = f64::from(i) * std::f64::consts::TAU / f64::from(count);
                    WindowPoint::new(0.25 * angle.cos(), 0.25 * angle.sin())
                })
                .collect(),
            inside: 4.0.into(),
            outside: 7.0.into(),
        };
        cases.push((before_grade.clone(), 10.0));
    }
    let mut pipelines = None;
    let mut previous: Option<Vec<[f32; 3]>> = None;
    let mut changed = 0;
    for (case, (before_grade, frame)) in cases.iter().enumerate() {
        let mut wanted = pixels.clone();
        for (grade, at) in [(before_grade, *frame), (&after_grade, 0.0), (&last, 0.0)] {
            wanted = evaluate_picture(
                grade,
                at,
                &GradeOptions::default(),
                &ParameterLut,
                &lut,
                (width, height),
                &wanted,
                height as f32 / 1080.0,
            );
        }
        if let Some(before) = &previous {
            if apart(&wanted, before).0 > 0.001 {
                changed += 1;
            }
        }
        previous = Some(wanted.clone());
        for split in [false, true] {
            let (input, grade) = if split {
                (
                    stage_of(
                        before_grade,
                        *frame,
                        &GradeOptions::default(),
                        &ParameterLut,
                    ),
                    stage_of_grades(
                        &[(&after_grade, 0.0), (&last, 0.0)],
                        &GradeOptions::default(),
                        &ParameterLut,
                    ),
                )
            } else {
                (
                    Stage::default(),
                    stage_of_grades(
                        &[(before_grade, *frame), (&after_grade, 0.0), (&last, 0.0)],
                        &GradeOptions::default(),
                        &ParameterLut,
                    ),
                )
            };
            let mut scene = whole_scene(
                width,
                height,
                vec![layer(
                    Source::Picture(Arc::clone(&source), 0),
                    Transform::NONE,
                    1.0,
                    grade,
                )],
            );
            scene.layers[0].color.input = input;
            let got = rgb_of(&gpu, &mut compositor, &scene);
            assert!(apart(&got, &wanted).0 < 0.002, "case={case} split={split}");
            if let Some(count) = pipelines {
                assert_eq!(
                    compositor.pipelines_made(),
                    count,
                    "case={case} split={split}: only parameter data changed"
                );
            }
            pipelines = Some(compositor.pipelines_made());
        }
    }
    assert!(
        changed >= 4,
        "the animated/edited masks must actually change the output"
    );
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

/// T24: a tracked window, Perspective 3D included, keys the same on the card as on the
/// processor, with the homography in the parameters and no pipeline of its own.
#[test]
fn windows_gpu_tracked_windows_match_cpu_through_their_homography() {
    use columbina_model::grade::window::tracking::{Motion, TrackData, WindowTracking};
    let Some(gpu) = gpu_for_tests() else { return };
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    let (w, h) = (96, 64);
    let (p, pixels) = float_picture(w, h, |_, _| [0.25, 0.4, 1.5]);
    let source = Arc::new(p);
    let mut pipelines = None;
    for (i, shape) in shapes().into_iter().enumerate() {
        let mut grade = window_grade(shape, false);
        let NodeKind::Corrector(c) = &mut grade.nodes[0].kind else {
            unreachable!()
        };
        let mut data = TrackData::new([40.0, 30.0], [192, 128]);
        data.set_sample(
            3,
            Motion {
                pan: -9.0,
                tilt: 6.0,
                zoom: 1.2,
                rotate: 0.3,
                aspect: 1.1,
                shear: 0.05,
                yaw: if i % 2 == 0 { 0.003 } else { -0.004 },
                pitch: 0.002,
            },
            1.0,
        );
        c.windows[0].tracking = Some(WindowTracking {
            data,
            ..WindowTracking::default()
        });
        let wanted = evaluate_picture(
            &grade,
            3.0,
            &GradeOptions::default(),
            &NoLuts,
            &|_, c| c,
            (w, h),
            &pixels,
            h as f32 / 1080.0,
        );
        let op = stage_of(&grade, 3.0, &GradeOptions::default(), &NoLuts);
        let scene = whole_scene(
            w,
            h,
            vec![layer(
                Source::Picture(Arc::clone(&source), 0),
                Transform::NONE,
                1.0,
                op,
            )],
        );
        let got = rgb_of(&gpu, &mut compositor, &scene);
        let (error, _) = apart(&got, &wanted);
        assert!(error < 0.0015, "shape {i}: error={error}");
        if let Some(count) = pipelines {
            assert_eq!(compositor.pipelines_made(), count);
        }
        pipelines = Some(compositor.pipelines_made());
    }
    assert_eq!(gpu.uncaught_errors(), (0, None));
}
