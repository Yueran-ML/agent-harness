//! The Blur palette of a corrector (pp. 3454–3456): its defaults, its ranges, how it is written
//! and read, and how it is set, keyframed and taken back by `set_node`.

use columbina_model::grade::{Blur, BlurMode, Interp, Key, Link, Node, Param, Port};
use columbina_model::{apply, apply_done, Op, OpError, Project, Target};
use serde_json::json;

fn c1() -> Target {
    Target::Clip {
        timeline: "t1".into(),
        clip: "c1".into(),
    }
}

fn rgb(from: &str, to: &str) -> Link {
    Link {
        from: from.into(),
        to: to.into(),
        kind: Port::Rgb,
        input: 0,
    }
}

/// Apply an operation, check that its inverse takes it back, and give back the inverse.
fn applied(project: &mut Project, op: Op) -> Op {
    let before = project.clone();
    let (_, inverse) = apply_done(project, op).unwrap();
    let mut undone = project.clone();
    apply(&mut undone, inverse.clone()).unwrap();
    let mut expected = before;
    expected.next_id = undone.next_id;
    assert_eq!(undone, expected);
    inverse
}

fn refused(project: &Project, op: Op) -> OpError {
    let mut copy = project.clone();
    let error = apply(&mut copy, op).unwrap_err();
    assert_eq!(&copy, project);
    error
}

/// The sample with c1 graded by one corrector, n3.
fn one() -> Project {
    let mut project = Project::sample();
    applied(
        &mut project,
        Op::AddNode {
            grade: c1(),
            node: Node::corrector(""),
            index: None,
            links: vec![rgb("source", ""), rgb("", "output")],
        },
    );
    project
}

fn set(params: serde_json::Value) -> Op {
    Op::SetNode {
        grade: c1(),
        id: "n3".into(),
        label: None,
        enabled: None,
        locked: None,
        pos: None,
        params: Some(params),
    }
}

fn blur_of(project: &Project) -> Blur {
    let clip = project.timelines[0]
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
        .find(|clip| clip.id == "c1")
        .unwrap();
    clip.grade.nodes[0].as_corrector().unwrap().blur.clone()
}

#[test]
fn a_new_corrector_has_the_blur_palette_the_pages_show_and_writes_it() {
    let blur = Blur::default();
    assert_eq!(blur.mode, BlurMode::Blur);
    assert!(!blur.reads_neighbourhood());
    let written = serde_json::to_value(Node::corrector("n1").kind.params()).unwrap();
    assert_eq!(
        written["blur"],
        json!({
            "mode": "blur",
            "radius": {"r": 0.5, "g": 0.5, "b": 0.5},
            "ratio": {"r": 0.5, "g": 0.5, "b": 0.5},
            "scaling": {"r": 0.25, "g": 0.25, "b": 0.25},
            "coring_softness": 0.0,
            "level": 0.0,
            "mix": 100.0,
            "linked": {"radius": true, "ratio": true, "scaling": true},
        })
    );
}

#[test]
fn a_corrector_written_before_the_blur_palette_reads_as_one_at_its_defaults() {
    let mut written = Node::corrector("n1").kind.params();
    written.as_object_mut().unwrap().remove("blur");
    let node = columbina_model::grade::NodeKind::from_params(
        columbina_model::grade::Kind::Corrector,
        written,
    )
    .unwrap();
    assert_eq!(node.as_corrector_kind().blur, Blur::default());
}

trait AsCorrector {
    fn as_corrector_kind(&self) -> &columbina_model::Corrector;
}

impl AsCorrector for columbina_model::grade::NodeKind {
    fn as_corrector_kind(&self) -> &columbina_model::Corrector {
        match self {
            columbina_model::grade::NodeKind::Corrector(corrector) => corrector,
            _ => panic!("a corrector"),
        }
    }
}

#[test]
fn the_blur_palette_is_set_keyframed_and_taken_back_by_set_node() {
    let mut project = one();
    let inverse = applied(
        &mut project,
        set(json!({"blur": {
            "mode": "mist",
            "radius": {"r": 0.3, "g": 0.3, "b": 0.3},
            "mix": 40,
            "linked": {"radius": false},
        }})),
    );
    let blur = blur_of(&project);
    assert_eq!(blur.mode, BlurMode::Mist);
    assert_eq!(blur.radius.g, Param::Fixed(0.3));
    assert_eq!(blur.mix, Param::Fixed(40.0));
    assert!(!blur.linked.radius && blur.linked.ratio);
    assert!(blur.reads_neighbourhood() && blur.mist_ever());
    assert_eq!(
        inverse,
        set(json!({"blur": {
            "mode": "blur",
            "radius": {"r": 0.5, "g": 0.5, "b": 0.5},
            "mix": 100.0,
            "linked": {"radius": true},
        }}))
    );
    // Keyframed through 0.5: it reads the pixels about each one at every frame.
    applied(
        &mut project,
        set(json!({"blur": {"mode": "blur", "radius": {"b": {"keys": [
            {"frame": 0, "value": 0.5, "interp": "linear"},
            {"frame": 10, "value": 0.8, "interp": "linear"},
        ]}}}})),
    );
    let blur = blur_of(&project);
    assert_eq!(
        blur.radius.b,
        Param::Keys(vec![
            Key {
                frame: 0,
                value: 0.5,
                interp: Interp::Linear
            },
            Key {
                frame: 10,
                value: 0.8,
                interp: Interp::Linear
            },
        ])
    );
    assert!(blur.radius_ever(2) && blur.radius_ever(0));
    // Mix tells only in the Mist mode.
    let mut only_mix = Blur {
        mix: Param::Fixed(20.0),
        ..Blur::default()
    };
    assert!(!only_mix.reads_neighbourhood());
    only_mix.mode = BlurMode::Mist;
    assert!(only_mix.reads_neighbourhood());
    // Scaling, Coring Softness, Level and H/V Ratio alone change nothing.
    let mut quiet = Blur {
        mode: BlurMode::Sharpen,
        ..Blur::default()
    };
    quiet.scaling.r = Param::Fixed(1.0);
    quiet.ratio.g = Param::Fixed(0.9);
    quiet.level = Param::Fixed(0.5);
    quiet.coring_softness = Param::Fixed(0.5);
    assert!(!quiet.reads_neighbourhood());
}

#[test]
fn settings_of_the_blur_palette_beyond_their_ranges_are_refused() {
    let project = one();
    for (patch, what) in [
        (json!({"radius": {"r": 1.2}}), "blur.radius.r"),
        (json!({"radius": {"g": -0.1}}), "blur.radius.g"),
        (json!({"ratio": {"b": 1.5}}), "blur.ratio.b"),
        (json!({"scaling": {"r": 2}}), "blur.scaling.r"),
        (json!({"coring_softness": 1.1}), "blur.coring_softness"),
        (json!({"level": -0.5}), "blur.level"),
        (json!({"mix": 101}), "blur.mix"),
    ] {
        assert_eq!(
            refused(&project, set(json!({ "blur": patch }))),
            OpError::OutOfRange { what },
            "{what}"
        );
    }
    for bogus in [
        json!({"mode": "defocus"}),
        json!({"denoise": 1}),
        json!({"linked": {"mix": true}}),
    ] {
        assert!(
            matches!(
                refused(&project, set(json!({ "blur": bogus }))),
                OpError::Params { .. }
            ),
            "{bogus}"
        );
    }
}

#[test]
fn keys_of_the_blur_palette_move_with_the_keys_of_the_grade() {
    let mut project = one();
    applied(
        &mut project,
        set(json!({"blur": {"mix": {"keys": [
            {"frame": 4, "value": 100, "interp": "linear"},
            {"frame": 12, "value": 30, "interp": "hold"},
        ]}}})),
    );
    let clip = project.timelines[0]
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
        .find(|clip| clip.id == "c1")
        .unwrap();
    assert!(clip.grade.has_keys());
    let moved = clip.grade.with_key_frames(|frame| frame + 10);
    let blur = &moved.nodes[0].as_corrector().unwrap().blur;
    let Param::Keys(keys) = &blur.mix else {
        panic!("keys")
    };
    assert_eq!(
        keys.iter().map(|key| key.frame).collect::<Vec<_>>(),
        [14, 22]
    );
}
