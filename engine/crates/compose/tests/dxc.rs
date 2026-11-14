//! Microsoft's DXC beside the executable, as the package of the desktop brings it beside
//! `columbina-engine.exe`: the card is made with it, and a grade of several nodes is compiled
//! and drawn with it.
//!
//! Ignored, since nothing puts the DLLs in `target/`: the tests of the engine compile with FXC.
//! To run it, by hand, under the lock of the card:
//!
//! 1. build it: `cargo test -p columbina-compose --test dxc --no-run`, which names the
//!    executable (`target\debug\deps\dxc-<hash>.exe`);
//! 2. copy `dxcompiler.dll` and `dxil.dll` of the Windows SDK (`C:\Program Files
//!    (x86)\Windows Kits\10\Redist\D3D\x64`, 1.8.2502 or later) beside that executable;
//! 3. `cargo test -p columbina-compose --test dxc -- --ignored --exact
//!    dxc_beside_the_executable_compiles_the_shaders`, without `COLUMBINA_DX12_COMPILER`;
//! 4. delete the two DLLs again.

use std::sync::Arc;
use std::time::Instant;

use columbina_compose::grade::{stage_of, GradeOptions, NoLuts};
use columbina_compose::{
    ChosenBy, CompilerKind, Compositor, Gpu, LayerColor, Preference, Scene, SceneLayer, Source,
    Target, Transform, COMPILER_VARIABLE,
};
use columbina_model::grade::{Corrector, Grade, Link, Node, NodeKind, Param, Port, OUTPUT, SOURCE};

const WIDTH: u32 = 64;
const HEIGHT: u32 = 36;

/// Correctors one after another, each with a gain and a curve of its own.
fn serial(count: usize) -> Grade {
    let ids: Vec<String> = (0..count).map(|i| format!("n{i}")).collect();
    let nodes = ids
        .iter()
        .map(|id| {
            let mut node = Node::corrector(id);
            if let NodeKind::Corrector(corrector) = &mut node.kind {
                set(corrector);
            }
            node
        })
        .collect();
    let link = |from: &str, to: &str| Link {
        from: from.into(),
        to: to.into(),
        kind: Port::Rgb,
        input: 0,
    };
    let mut links = Vec::new();
    let mut from = SOURCE.to_owned();
    for id in &ids {
        links.push(link(&from, id));
        from.clone_from(id);
    }
    links.push(link(&from, OUTPUT));
    let mut grade = Grade { nodes, links };
    grade.sort_links();
    assert!(grade.is_complete());
    grade
}

fn set(corrector: &mut Corrector) {
    corrector.primaries.gain.y = Param::Fixed(1.05);
    corrector.curves.y.points =
        columbina_model::grade::fixed_points(&[[0.0, 0.0], [0.4, 0.38], [1.0, 1.0]]);
    corrector.hsl.hue_vs_sat = columbina_model::grade::fixed_points(&[[0.0, 0.1], [0.5, -0.1]]);
}

#[test]
#[ignore = "needs dxcompiler.dll and dxil.dll beside the test's executable, put there by hand"]
fn dxc_beside_the_executable_compiles_the_shaders() {
    let executable = std::env::current_exe().unwrap();
    let folder = executable.parent().unwrap();
    for dll in ["dxcompiler.dll", "dxil.dll"] {
        assert!(
            folder.join(dll).is_file(),
            "put {dll} of the Windows SDK in {} first (see the head of tests/dxc.rs)",
            folder.display()
        );
    }
    assert!(
        std::env::var_os(COMPILER_VARIABLE).is_none(),
        "{COMPILER_VARIABLE} would name another compiler"
    );
    let started = Instant::now();
    let gpu: Arc<Gpu> = Gpu::new(Preference::HighPerformance).unwrap();
    eprintln!(
        "on {} ({}), made in {:.0} ms: {:?}",
        gpu.name(),
        gpu.backend(),
        started.elapsed().as_secs_f64() * 1000.0,
        gpu.shader_compiler()
    );
    assert_eq!(gpu.dxc_failure(), None);
    let compiler = gpu.shader_compiler().unwrap();
    assert_eq!(compiler.by, ChosenBy::BesideExecutable);
    assert_eq!(
        compiler.kind,
        CompilerKind::Dxc(folder.join("dxcompiler.dll"))
    );

    let target = Target::preview(&gpu, WIDTH, HEIGHT).unwrap();
    for nodes in [1, 4] {
        let scene = Scene {
            width: WIDTH,
            height: HEIGHT,
            layers: vec![SceneLayer {
                source: Source::Color([0.5, 0.5, 0.5, 1.0]),
                transform: Transform::NONE,
                opacity: 1.0,
                color: LayerColor {
                    grade: stage_of(&serial(nodes), 0.0, &GradeOptions::default(), &NoLuts),
                    ..LayerColor::default()
                },
                crop: columbina_compose::Crop::NONE,
                composite: columbina_compose::Composite::Normal,
            }],
            ..Scene::default()
        };
        let mut compositor = Compositor::new(Arc::clone(&gpu));
        let started = Instant::now();
        compositor.render(&scene, &target).unwrap();
        let pixels = target.read_rgba().unwrap();
        eprintln!(
            "{nodes} correctors: the first frame in {:.0} ms",
            started.elapsed().as_secs_f64() * 1000.0
        );
        // Grey, lifted a little by the gains: drawn, and not black.
        assert!(pixels[0] > 100 && pixels[0] < 255, "{:?}", &pixels[..4]);
    }
    assert_eq!(gpu.uncaught_errors(), (0, None));
}
