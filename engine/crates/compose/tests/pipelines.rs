//! The pipelines of programs of colour, made on threads of their own: a compositor that does not
//! wait for them draws a frame with the grade before, or with none, until they are made; one
//! that waits draws each frame as its scene says. And how long the pipelines of grades take to
//! make. Without an adapter the tests are skipped, unless `COLUMBINA_REQUIRE_GPU` is 1.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use columbina_compose::grade::{stage_of, GradeOptions, NoLuts};
use columbina_compose::{
    gpu_for_tests, ColorOp, ComposeError, Compositor, Gpu, LayerColor, Scene, SceneLayer, Source,
    Stage, Target, Transform,
};
use columbina_model::grade::{
    Composite, Corrector, Grade, LayerMixer, Link, Node, NodeKind, Param, Port, OUTPUT, SOURCE,
};

const WIDTH: u32 = 64;
const HEIGHT: u32 = 36;

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

/// Correctors one after another, each setting its gain as `gains` say.
fn serial(gains: &[f64]) -> Grade {
    let ids: Vec<String> = (0..gains.len()).map(|i| format!("n{i}")).collect();
    let nodes = ids
        .iter()
        .zip(gains)
        .map(|(id, &gain)| {
            corrector(id, |c| {
                c.primaries.gain.y = Param::Fixed(gain);
                c.curves.y.points = vec![[0.0, 0.0], [0.4, 0.38], [1.0, 1.0]];
            })
        })
        .collect();
    let mut links = Vec::new();
    let mut from = SOURCE.to_owned();
    for id in &ids {
        links.push(link(&from, id, 0));
        from.clone_from(id);
    }
    links.push(link(&from, OUTPUT, 0));
    let mut grade = Grade { nodes, links };
    grade.sort_links();
    assert!(grade.is_complete());
    grade
}

/// A grade of six nodes: three serial correctors, two of them in parallel and laid by a layer
/// mixer, and one after, each with what makes its code its own.
fn six_nodes() -> Grade {
    let mixer = Node {
        id: "lm".into(),
        label: String::new(),
        enabled: true,
        locked: false,
        pos: None,
        kind: NodeKind::LayerMixer(LayerMixer {
            inputs: 2,
            composite: Composite::Overlay,
        }),
    };
    let mut grade = Grade {
        nodes: vec![
            corrector("n1", |c| {
                c.primaries.contrast = Param::Fixed(1.2);
                c.curves.y.points = vec![[0.0, 0.0], [0.3, 0.25], [0.7, 0.75], [1.0, 1.0]];
            }),
            corrector("n2", |c| {
                c.primaries.saturation = Param::Fixed(60.0);
                c.hsl.hue_vs_sat = vec![[0.0, 0.2], [0.3, 0.0], [0.6, -0.2]];
                c.soft_clip.high_soft.r = Param::Fixed(20.0);
            }),
            corrector("n3", |c| c.log.shadow.b = Param::Fixed(0.1)),
            corrector("n4", |c| {
                c.primaries.temperature = Param::Fixed(-600.0);
                c.key.output.gain = Param::Fixed(0.7);
            }),
            mixer,
            corrector("n6", |c| c.hsl.sat_vs_lum = vec![[0.0, 0.0], [1.0, 0.1]]),
        ],
        links: vec![
            link(SOURCE, "n1", 0),
            link("n1", "n2", 0),
            link("n2", "n3", 0),
            link("n2", "n4", 0),
            link("n3", "lm", 0),
            link("n4", "lm", 1),
            link("lm", "n6", 0),
            link("n6", OUTPUT, 0),
        ],
    };
    grade.sort_links();
    assert!(grade.is_complete());
    grade
}

fn stage(grade: &Grade) -> Stage {
    stage_of(grade, 0.0, &GradeOptions::default(), &NoLuts)
}

/// An input transform that multiplies by `k` (its code says so: two of them are of two codes).
fn input(k: f32) -> Stage {
    Stage {
        ops: vec![ColorOp {
            body: format!("rgb = rgb * {k:?};").into(),
            ..ColorOp::default()
        }],
    }
}

fn scene(input: Stage, grade: Stage, over: Stage) -> Scene {
    Scene {
        width: WIDTH,
        height: HEIGHT,
        layers: vec![SceneLayer {
            source: Source::Color([0.5, 0.5, 0.5, 1.0]),
            transform: Transform::NONE,
            opacity: 1.0,
            color: LayerColor {
                input,
                grade,
                ..LayerColor::default()
            },
        }],
        grade: over,
        ..Scene::default()
    }
}

/// The red of the middle of the frame drawn of `scene`.
fn red(gpu: &Arc<Gpu>, compositor: &mut Compositor, scene: &Scene) -> u8 {
    let target = Target::preview(gpu, WIDTH, HEIGHT).unwrap();
    compositor.render(scene, &target).unwrap();
    let pixels = target.read_rgba().unwrap();
    pixels[((HEIGHT / 2 * WIDTH + WIDTH / 2) * 4) as usize]
}

/// Wait until the compositor has made what it was asked for, as a monitor that is still does
/// (without being told to).
fn made(compositor: &Compositor) {
    let started = Instant::now();
    while compositor.is_compiling() {
        assert!(
            started.elapsed() < Duration::from_secs(300),
            "still compiling"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_frame_is_drawn_with_the_grade_before_until_the_pipeline_of_its_own_is_made() {
    let Some(gpu) = gpu_for_tests() else { return };
    // What each scene is drawn as by a compositor that waits for its pipelines, as an export.
    let mut waits = Compositor::new(Arc::clone(&gpu));
    let mut drawn = |scene: &Scene| red(&gpu, &mut waits, scene);
    let darker = stage(&serial(&[0.5]));
    let brighter = stage(&serial(&[1.6, 1.0]));
    let first = scene(Stage::default(), darker.clone(), Stage::default());
    let second = scene(Stage::default(), brighter.clone(), Stage::default());
    let as_input = scene(input(0.5), brighter.clone(), Stage::default());
    let over_dark = scene(Stage::default(), Stage::default(), darker);
    let over_bright = scene(Stage::default(), Stage::default(), brighter);
    let ungraded = drawn(&scene(Stage::default(), Stage::default(), Stage::default()));
    let (dark, bright, halved) = (drawn(&first), drawn(&second), drawn(&as_input));
    let (over_dark_drawn, over_bright_drawn) = (drawn(&over_dark), drawn(&over_bright));
    assert!(!waits.is_provisional() && !waits.is_compiling());
    assert!(
        dark < ungraded && ungraded < bright,
        "{dark} {ungraded} {bright}"
    );
    assert!(halved < bright, "{halved} {bright}");
    assert!(over_dark_drawn < ungraded && ungraded < over_bright_drawn);

    let woken = Arc::new(AtomicUsize::new(0));
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    compositor.compile_in_background({
        let woken = Arc::clone(&woken);
        move || {
            woken.fetch_add(1, Ordering::SeqCst);
        }
    });
    // Nothing drawn before: the layer is drawn with no colour until its pipeline is made.
    assert_eq!(red(&gpu, &mut compositor, &first), ungraded);
    assert!(compositor.is_provisional());
    made(&compositor);
    let started = Instant::now();
    while woken.load(Ordering::SeqCst) == 0 {
        assert!(started.elapsed() < Duration::from_secs(10), "never woken");
        std::thread::yield_now();
    }
    assert_eq!(red(&gpu, &mut compositor, &first), dark);
    assert!(!compositor.is_provisional());
    // A grade of another shape: drawn as before (by the program and the parameters of the last
    // frame) until its own is made, and then as it is.
    assert_eq!(red(&gpu, &mut compositor, &second), dark);
    assert!(compositor.is_provisional());
    made(&compositor);
    assert_eq!(red(&gpu, &mut compositor, &second), bright);
    assert!(!compositor.is_provisional());
    // Another input transform is taken for another layer: drawn with no colour of its own,
    // not with the grade of what was there.
    assert_eq!(red(&gpu, &mut compositor, &as_input), ungraded);
    assert!(compositor.is_provisional());
    compositor.finish_compiling();
    assert_eq!(red(&gpu, &mut compositor, &as_input), halved);
    // The grade of the timeline, over the canvas, likewise: not done until it is made the
    // first time, and done as before while another shape of it is made.
    assert_eq!(red(&gpu, &mut compositor, &over_dark), ungraded);
    assert!(compositor.is_provisional());
    compositor.finish_compiling();
    assert_eq!(red(&gpu, &mut compositor, &over_dark), over_dark_drawn);
    assert_eq!(red(&gpu, &mut compositor, &over_bright), over_dark_drawn);
    assert!(compositor.is_provisional());
    compositor.finish_compiling();
    assert_eq!(red(&gpu, &mut compositor, &over_bright), over_bright_drawn);
    assert!(!compositor.is_provisional());
    // Made once, a pipeline is not made again.
    assert_eq!(red(&gpu, &mut compositor, &first), dark);
    assert!(!compositor.is_provisional() && !compositor.is_compiling());
    assert_eq!(gpu.uncaught_errors(), (0, None));
}

#[test]
fn a_program_that_is_not_wgsl_is_an_error_of_its_layer_once_its_pipeline_has_failed() {
    let Some(gpu) = gpu_for_tests() else { return };
    let wrong = Stage {
        ops: vec![ColorOp {
            body: "rgb = nothing_that_is;".into(),
            ..ColorOp::default()
        }],
    };
    let scene = scene(wrong, Stage::default(), Stage::default());
    let target = Target::preview(&gpu, WIDTH, HEIGHT).unwrap();
    let mut compositor = Compositor::new(Arc::clone(&gpu));
    compositor.compile_in_background(|| {});
    compositor.render(&scene, &target).unwrap();
    assert!(compositor.is_provisional());
    compositor.finish_compiling();
    for _ in 0..2 {
        match compositor.render(&scene, &target) {
            Err(ComposeError::Layer { layer: 0, message }) => {
                assert!(message.contains("its colour cannot be done"), "{message}");
            }
            other => panic!("{other:?}"),
        }
    }
    let mut waits = Compositor::new(Arc::clone(&gpu));
    assert!(matches!(
        waits.render(&scene, &target),
        Err(ComposeError::Layer { layer: 0, .. })
    ));
}

/// How long the pipeline of a grade takes to make, by the shape of the grade: the first frame
/// of a compositor that waits for it, less a frame after.
#[test]
#[ignore = "a measure of time: run it on its own, in release, under the lock of the card"]
fn how_long_the_pipelines_of_grades_take_to_make() {
    let Some(gpu) = gpu_for_tests() else { return };
    eprintln!("on {} ({})", gpu.name(), gpu.backend());
    let target = Target::preview(&gpu, WIDTH, HEIGHT).unwrap();
    let time = |what: &str, scene: &Scene| {
        let mut compositor = Compositor::new(Arc::clone(&gpu));
        let plain = self::scene(Stage::default(), Stage::default(), Stage::default());
        compositor.render(&plain, &target).unwrap();
        let started = Instant::now();
        compositor.render(scene, &target).unwrap();
        target.read_rgba().unwrap();
        let first = started.elapsed();
        let started = Instant::now();
        compositor.render(scene, &target).unwrap();
        target.read_rgba().unwrap();
        let again = started.elapsed();
        eprintln!(
            "{what}: {:.0} ms",
            (first.saturating_sub(again)).as_secs_f64() * 1000.0
        );
    };
    for nodes in [1, 2, 4] {
        let grade = stage(&serial(&vec![1.1; nodes]));
        time(
            &format!("a layer graded by {nodes} serial correctors"),
            &scene(Stage::default(), grade.clone(), Stage::default()),
        );
        time(
            &format!("the timeline graded by {nodes} serial correctors"),
            &scene(Stage::default(), Stage::default(), grade),
        );
    }
    time(
        "a layer graded by six nodes (a layer mixer of two)",
        &scene(Stage::default(), stage(&six_nodes()), Stage::default()),
    );
}
