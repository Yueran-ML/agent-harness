//! Spatial keys, including the picture positions read by later neighbourhoods.
#![allow(
    clippy::cast_precision_loss,
    clippy::float_cmp,
    reason = "small test image dimensions and exact hard-mask zero/one values"
)]
use columbina_compose::grade::{
    evaluate, evaluate_picture, evaluate_picture_showing, GradeOptions, Look, NoLuts, Prepared,
    Show,
};
use columbina_model::grade::{
    Grade, Link, Node, NodeKind, Port, Window, WindowBezierPoint, WindowCombine, WindowPoint,
    WindowShape, OUTPUT, SOURCE,
};

fn grade(shape: WindowShape) -> Grade {
    let mut node = Node::corrector("w");
    let NodeKind::Corrector(c) = &mut node.kind else {
        unreachable!()
    };
    c.primaries.gain.r = 2.0.into();
    c.windows.push(Window::new("w1", shape));
    Grade {
        nodes: vec![node],
        links: vec![
            Link {
                from: SOURCE.into(),
                to: "w".into(),
                kind: Port::Rgb,
                input: 0,
            },
            Link {
                from: "w".into(),
                to: OUTPUT.into(),
                kind: Port::Rgb,
                input: 0,
            },
        ],
    }
}

fn corrector(g: &mut Grade) -> &mut columbina_model::grade::Corrector {
    let NodeKind::Corrector(c) = &mut g.nodes[0].kind else {
        unreachable!()
    };
    c
}

fn matte(g: &Grade, width: u32, height: u32) -> Vec<f32> {
    evaluate_picture_showing(
        g,
        0.0,
        &GradeOptions::default(),
        &NoLuts,
        &|_, c| c,
        (width, height),
        &vec![[0.25; 3]; (width * height) as usize],
        height as f32 / 1080.0,
        Some(&Look {
            node: "w".into(),
            show: Show::Matte,
        }),
    )
    .iter()
    .map(|p| p[0])
    .collect()
}

#[test]
fn ordered_opacity_subtraction_and_inverted_size_zero_are_spatial_keys() {
    let mut g = grade(WindowShape::Circular {
        softness: 0.0.into(),
    });
    corrector(&mut g).windows[0].opacity = 60.0.into();
    let mut cut = Window::new("cut", WindowShape::linear());
    cut.opacity = 20.0.into();
    cut.combine = WindowCombine::Subtract;
    corrector(&mut g).windows.push(cut);
    let out = matte(&g, 80, 40);
    assert!((out[20 * 80 + 40] - 0.4).abs() < 1e-6);
    corrector(&mut g).windows.reverse();
    assert!((matte(&g, 80, 40)[20 * 80 + 40] - 0.6).abs() < 1e-6);
    corrector(&mut g).windows.truncate(1);
    let w = &mut corrector(&mut g).windows[0];
    w.combine = WindowCombine::Add;
    w.opacity = 100.0.into();
    w.transform.size = 0.0.into();
    assert!(matte(&g, 80, 40).iter().all(|v| *v == 0.0));
    corrector(&mut g).windows[0].invert = true;
    assert!(matte(&g, 80, 40).iter().all(|v| *v == 1.0));
}

#[test]
fn drafts_disabled_windows_and_positionless_calls_preserve_the_previous_key() {
    let mut g = grade(WindowShape::curve());
    corrector(&mut g).windows[0].invert = true;
    let with_draft = Prepared::new(&g, &GradeOptions::default(), &NoLuts)
        .ops_at(0.0)
        .last()
        .unwrap();
    corrector(&mut g).windows.clear();
    let old = Prepared::new(&g, &GradeOptions::default(), &NoLuts)
        .ops_at(0.0)
        .last()
        .unwrap();
    assert_eq!(with_draft.body, old.body);
    assert_eq!(with_draft.library, old.library);
    assert_eq!(with_draft.params, old.params);
    let expected = evaluate(&g, 0.0, &GradeOptions::default(), [0.25; 3]);
    corrector(&mut g)
        .windows
        .push(Window::new("c", WindowShape::circular()));
    assert_eq!(
        evaluate(&g, 0.0, &GradeOptions::default(), [0.25; 3]),
        expected
    );
    corrector(&mut g).windows[0].enabled = false;
    assert!(matte(&g, 80, 40).iter().all(|v| *v == 1.0));
}

#[test]
fn gradient_rotation_translation_and_softness_follow_local_y() {
    let mut g = grade(WindowShape::Gradient {
        softness: 0.0.into(),
    });
    let out = matte(&g, 80, 40);
    assert_eq!((out[0], out[39 * 80]), (1.0, 0.0));
    corrector(&mut g).windows[0].transform.rotate = 90.0.into();
    let out = matte(&g, 80, 40);
    assert_eq!((out[20 * 80], out[20 * 80 + 79]), (0.0, 1.0));
    corrector(&mut g).windows[0].transform.pan = 75.0.into();
    let out = matte(&g, 80, 40);
    assert_eq!((out[20 * 80 + 50], out[20 * 80 + 70]), (0.0, 1.0));
    corrector(&mut g).windows[0].shape = WindowShape::Gradient {
        softness: 100.0.into(),
    };
    let out = matte(&g, 80, 40);
    assert!(out[20 * 80 + 60] > 0.5 && out[20 * 80 + 60] < 0.7);
}

#[test]
fn polygon_even_odd_and_independent_feather_apply_to_pinned_points() {
    let mut g = grade(WindowShape::Polygon {
        points: vec![
            WindowPoint::new(-0.3, -0.3),
            WindowPoint::new(0.3, 0.3),
            WindowPoint::new(-0.3, 0.3),
            WindowPoint::new(0.3, -0.3),
        ],
        inside: 0.0.into(),
        outside: 0.0.into(),
    });
    let out = matte(&g, 100, 100);
    assert_eq!((out[30 * 100 + 50], out[50 * 100 + 25]), (1.0, 0.0));
    corrector(&mut g).windows[0].shape = WindowShape::Polygon {
        points: vec![
            WindowPoint::new(-0.25, -0.25),
            WindowPoint::new(0.25, -0.25),
            WindowPoint::new(0.25, 0.25),
            WindowPoint::new(-0.25, 0.25),
        ],
        inside: 20.0.into(),
        outside: 40.0.into(),
    };
    let out = matte(&g, 100, 100);
    assert!(out[50 * 100 + 20] > 0.0 && out[50 * 100 + 20] < out[50 * 100 + 25]);
    assert!(out[50 * 100 + 25] < out[50 * 100 + 35]);
}

#[test]
fn two_anchor_cubic_uses_relative_handles_and_geometry_edits_keep_shader() {
    let shape = WindowShape::Curve {
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
        inside: 0.0.into(),
        outside: 0.0.into(),
    };
    let mut g = grade(shape);
    let out = matte(&g, 100, 100);
    assert_eq!((out[50 * 100 + 50], out[10 * 100 + 50]), (1.0, 0.0));
    let old = Prepared::new(&g, &GradeOptions::default(), &NoLuts)
        .ops_at(0.0)
        .last()
        .unwrap();
    corrector(&mut g).windows[0].shape = WindowShape::polygon();
    corrector(&mut g).windows[0].transform.rotate = 25.0.into();
    let new = Prepared::new(&g, &GradeOptions::default(), &NoLuts)
        .ops_at(0.0)
        .last()
        .unwrap();
    assert_eq!(old.body, new.body);
    assert_eq!(old.library, new.library);
    assert_ne!(old.params, new.params);
}

#[test]
fn qualifier_window_then_external_key_use_their_separate_matte_switches() {
    let mut g = grade(WindowShape::Circular {
        softness: 0.0.into(),
    });
    let c = corrector(&mut g);
    c.windows[0].opacity = 50.0.into();
    c.qualifier = Some(columbina_model::Qualifier::default());
    c.key.qualifier.gain = 0.25.into();
    let mut external = Node::corrector("ext");
    let NodeKind::Corrector(c) = &mut external.kind else {
        unreachable!()
    };
    c.key.output.gain = 0.4.into();
    g.nodes.push(external);
    g.links[0].from = "ext".into();
    g.links.push(Link {
        from: SOURCE.into(),
        to: "ext".into(),
        kind: Port::Rgb,
        input: 0,
    });
    g.links.push(Link {
        from: "ext".into(),
        to: "w".into(),
        kind: Port::Key,
        input: 0,
    });
    g.sort_links();
    for (qualifier_matte, input_matte, inside, outside) in [
        (true, true, 0.05, 0.0),
        (false, true, 0.3, 0.1),
        (true, false, 0.525, 0.4),
        (false, false, 1.0, 0.65),
    ] {
        let c = corrector(&mut g);
        c.key.qualifier.matte = qualifier_matte;
        c.key.input.matte = input_matte;
        let mask = matte(&g, 80, 40);
        assert!((mask[20 * 80 + 40] - inside).abs() < 1e-6);
        assert!((mask[0] - outside).abs() < 1e-6);
    }
}

#[test]
fn linear_softness_is_independent_on_each_boundary_edge() {
    let mut shape = WindowShape::linear();
    let WindowShape::Linear { softness, .. } = &mut shape else {
        unreachable!()
    };
    *softness = [100.0.into(), 0.0.into(), 0.0.into(), 0.0.into()];
    let mask = matte(&grade(shape), 100, 100);
    assert!(mask[20 * 100 + 50] > 0.0 && mask[20 * 100 + 50] < 0.5);
    assert_eq!(mask[50 * 100 + 80], 0.0);
    assert_eq!(mask[80 * 100 + 50], 0.0);
    assert_eq!(mask[50 * 100 + 20], 0.0);
}

#[test]
fn linear_soft_edges_have_no_diagonal_nearest_edge_seam() {
    let mut shape = WindowShape::linear();
    let WindowShape::Linear { softness, .. } = &mut shape else {
        unreachable!()
    };
    *softness = [100.0.into(), 0.0.into(), 0.0.into(), 0.0.into()];
    let mask = matte(&grade(shape), 100, 100);
    let a = mask[29 * 100 + 30];
    let b = mask[30 * 100 + 29];
    assert!(a > 0.6 && a < 0.7);
    assert!(
        b > 0.6 && b < 0.7,
        "the same top feather must continue across the nearest-edge bisector: {b}"
    );
    assert!((a - b).abs() < 0.04);
}

fn picture(grade: &Grade, width: u32, height: u32) -> Vec<[f32; 3]> {
    evaluate_picture(
        grade,
        0.0,
        &GradeOptions::default(),
        &NoLuts,
        &|_, c| c,
        (width, height),
        &vec![[0.25; 3]; (width * height) as usize],
        height as f32 / 1080.0,
    )
}

#[test]
fn circular_window_limits_the_grade_in_landscape_and_portrait() {
    let g = grade(WindowShape::Circular {
        softness: 0.0.into(),
    });
    for (width, height) in [(80, 40), (40, 80)] {
        let out = picture(&g, width, height);
        assert_eq!(out[0], [0.25; 3], "outside the circle must keep its input");
        assert!(out[((height / 2) * width + width / 2) as usize][0] > 0.3);
        let row = &out[((height / 2) * width) as usize..((height / 2 + 1) * width) as usize];
        let across = row.iter().filter(|c| c[0] > 0.3).count();
        let down = (0..height)
            .filter(|y| out[(y * width + width / 2) as usize][0] > 0.3)
            .count();
        assert_eq!(across, down, "one local unit is an input-image height");
    }
}

/// A tracked window's key is its own shape moved by the track (T24): the same as the window
/// placed by hand where the track takes it, here a pan of a tenth of the picture's width, and
/// a turn about the anchor that a rotated window of another aspect follows.
#[test]
fn a_tracked_window_keys_where_the_track_moves_it_after_its_own_transform() {
    use columbina_model::grade::window::tracking::{Motion, TrackData, WindowTracking};
    let (w, h) = (80, 40);
    let mut g = grade(WindowShape::Circular {
        softness: 0.0.into(),
    });
    let mut data = TrackData::new([39.5, 19.5], [w as usize, h as usize]);
    data.set_sample(
        0,
        Motion {
            pan: 8.0,
            ..Motion::IDENTITY
        },
        1.0,
    );
    corrector(&mut g).windows[0].tracking = Some(WindowTracking {
        data,
        ..WindowTracking::default()
    });
    let mut moved = grade(WindowShape::Circular {
        softness: 0.0.into(),
    });
    corrector(&mut moved).windows[0].transform.pan = 60.0.into();
    assert_eq!(matte(&g, w, h), matte(&moved, w, h));

    // A turn of a quarter about the anchor: a window drawn wide is shown tall.
    let mut g = grade(WindowShape::linear());
    corrector(&mut g).windows[0].transform.aspect = 80.0.into();
    let mut data = TrackData::new([39.5, 19.5], [w as usize, h as usize]);
    data.set_sample(
        0,
        Motion {
            rotate: std::f64::consts::FRAC_PI_2,
            ..Motion::IDENTITY
        },
        1.0,
    );
    let plain = matte(&g, w, h);
    corrector(&mut g).windows[0].tracking = Some(WindowTracking {
        data,
        ..WindowTracking::default()
    });
    let turned = matte(&g, w, h);
    let extent = |m: &[f32], across: bool| {
        (0..if across { w } else { h })
            .filter(|&i| {
                let at = if across {
                    (h / 2) * w + i
                } else {
                    i * w + w / 2
                };
                m[at as usize] > 0.5
            })
            .count()
    };
    assert!(extent(&plain, true) > extent(&plain, false));
    assert!(extent(&turned, true) < extent(&turned, false));
    assert_eq!(extent(&plain, true), extent(&turned, false));
}

/// Perspective 3D: the key of a tracked homography is the window's shape through its inverse;
/// points behind the vanishing line take nothing.
#[test]
fn a_perspective_track_keys_through_the_homography() {
    use columbina_model::grade::window::tracking::{Motion, TrackData, WindowTracking};
    let (w, h) = (80, 40);
    let mut g = grade(WindowShape::Gradient {
        softness: 0.0.into(),
    });
    // A yaw of -0.04 a pixel puts the vanishing line 25 pixels left of the anchor.
    let mut data = TrackData::new([39.5, 19.5], [w as usize, h as usize]);
    data.set_sample(
        0,
        Motion {
            yaw: -0.04,
            ..Motion::IDENTITY
        },
        1.0,
    );
    corrector(&mut g).windows[0].tracking = Some(WindowTracking {
        data,
        ..WindowTracking::default()
    });
    let out = matte(&g, w, h);
    // The homography keeps the anchor and the horizontal line through it: above it is in.
    for x in [20, 39, 79] {
        assert_eq!(out[(5 * w + x) as usize], 1.0, "{x}");
        assert_eq!(out[(35 * w + x) as usize], 0.0, "{x}");
    }
    // Left of x = 39.5 - 25 the plane is behind the eye: nothing is keyed there at all.
    assert_eq!(out[(5 * w) as usize], 0.0);
    assert_eq!(out[(5 * w + 14) as usize], 0.0);
}

#[test]
fn an_open_curve_of_any_number_of_anchors_is_a_draft_that_keys_nothing() {
    let anchor = |x: f64, y: f64| WindowBezierPoint {
        position: WindowPoint::new(x, y),
        into: WindowPoint::new(0.0, 0.0),
        out: WindowPoint::new(0.0, 0.0),
    };
    let points = vec![anchor(-0.2, -0.2), anchor(0.2, -0.2), anchor(0.0, 0.2)];
    let mut g = grade(WindowShape::Curve {
        points: points.clone(),
        closed: false,
        inside: 0.0.into(),
        outside: 0.0.into(),
    });
    corrector(&mut g).windows[0].invert = true;
    // Ignored, even inverted: the whole picture keyed, as without a window.
    assert!(matte(&g, 80, 40).iter().all(|v| *v == 1.0));
    // Closed, it keys outside the triangle where inverted.
    if let WindowShape::Curve { closed, .. } = &mut corrector(&mut g).windows[0].shape {
        *closed = true;
    }
    let closed = matte(&g, 80, 40);
    assert_eq!(closed[20 * 80 + 40], 0.0);
    assert_eq!(closed[2 * 80 + 2], 1.0);
}

/// The place in a window's local units (image heights from the centre) of pixel `at` of a
/// picture of `width` × `height`, with the transform as it is made.
fn local(at: usize, width: u32, height: u32) -> [f64; 2] {
    let (x, y) = ((at % width as usize) as f64, (at / width as usize) as f64);
    let (w, h) = (f64::from(width), f64::from(height));
    [((x + 0.5) / w - 0.5) * w / h, (y + 0.5) / h - 0.5]
}

fn smoothstep(low: f64, high: f64, x: f64) -> f64 {
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn square(inside: f64, outside: f64) -> WindowShape {
    WindowShape::Polygon {
        points: vec![
            WindowPoint::new(-0.25, -0.25),
            WindowPoint::new(0.25, -0.25),
            WindowPoint::new(0.25, 0.25),
            WindowPoint::new(-0.25, 0.25),
        ],
        inside: inside.into(),
        outside: outside.into(),
    }
}

#[test]
fn polygon_feathers_fall_off_by_the_distance_to_the_nearest_edge_each_side() {
    // Inside and outside of 100 span 0.5 each; 0 is a hard edge on that side.
    for (inside, outside) in [(50.0, 50.0), (100.0, 0.0), (0.0, 100.0), (30.0, 70.0)] {
        let g = grade(square(inside, outside));
        let (i, o) = (inside / 200.0, outside / 200.0);
        for (at, got) in matte(&g, 100, 100).iter().enumerate() {
            let [x, y] = local(at, 100, 100);
            let (ax, ay) = (x.abs() - 0.25, y.abs() - 0.25);
            let d = if ax < 0.0 && ay < 0.0 {
                -ax.max(ay)
            } else {
                -ax.max(0.0).hypot(ay.max(0.0))
            };
            let want = if d >= 0.0 && i == 0.0 {
                1.0
            } else if d < 0.0 && o == 0.0 {
                0.0
            } else {
                smoothstep(-o, i, d)
            };
            assert!(
                (f64::from(*got) - want).abs() < 2e-4,
                "{inside} {outside} at {x}, {y}: {got} not {want}"
            );
        }
    }
}

#[test]
fn circular_softness_100_spans_a_quarter_each_side_of_its_edge() {
    let g = grade(WindowShape::Circular {
        softness: 100.0.into(),
    });
    for (at, got) in matte(&g, 100, 100).iter().enumerate() {
        let [x, y] = local(at, 100, 100);
        let want = smoothstep(-0.25, 0.25, 0.25 - x.hypot(y));
        assert!(
            (f64::from(*got) - want).abs() < 2e-4,
            "{x}, {y}: {got} not {want}"
        );
    }
    // Inverted: none in the middle, all far out.
    let mut inverted = g;
    corrector(&mut inverted).windows[0].invert = true;
    let out = matte(&inverted, 100, 100);
    assert!(out[50 * 100 + 50] < 1e-3, "{}", out[50 * 100 + 50]);
    assert_eq!(out[0], 1.0);
}

#[test]
fn a_turned_polygon_is_the_polygon_of_its_points_turned() {
    let points = [[0.1, -0.2], [0.3, 0.1], [-0.2, 0.15]];
    let shape = |points: &[[f64; 2]]| WindowShape::Polygon {
        points: points
            .iter()
            .map(|p| WindowPoint::new(p[0], p[1]))
            .collect(),
        inside: 20.0.into(),
        outside: 20.0.into(),
    };
    let mut turned = grade(shape(&points));
    corrector(&mut turned).windows[0].transform.rotate = 90.0.into();
    // Clockwise a quarter: (x, y) to (-y, x), y down.
    let moved: Vec<[f64; 2]> = points.iter().map(|p| [-p[1], p[0]]).collect();
    let at_once = grade(shape(&moved));
    for (a, b) in matte(&turned, 100, 60).iter().zip(matte(&at_once, 100, 60)) {
        assert!((a - b).abs() < 1e-4, "{a} {b}");
    }
}

#[test]
fn the_window_of_a_node_before_another_is_its_own() {
    let alone = grade(square(30.0, 30.0));
    let mut before = alone.clone();
    before.nodes.push(Node::corrector("x"));
    before.links.retain(|l| l.to != OUTPUT);
    before.links.push(Link {
        from: "w".into(),
        to: "x".into(),
        kind: Port::Rgb,
        input: 0,
    });
    before.links.push(Link {
        from: "x".into(),
        to: OUTPUT.into(),
        kind: Port::Rgb,
        input: 0,
    });
    before.sort_links();
    assert_eq!(matte(&before, 80, 60), matte(&alone, 80, 60));
}
