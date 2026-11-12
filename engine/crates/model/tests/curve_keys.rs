//! The points of the curves of a corrector take keys (T5 of phase 4; Reference Manual 20,
//! p. 3384): where the points are at a frame, what is checked of them, how they are written, and
//! how their keys are set.

use columbina_model::grade::{
    fixed_points, points_at, CurvePoint, Interp, Key, Kind, Link, Node, NodeKind, Param,
};
use columbina_model::{apply, Grade, Op, OpError, Project, Target};
use serde_json::json;

fn c1() -> Target {
    Target::Clip {
        timeline: "t1".into(),
        clip: "c1".into(),
    }
}

fn grade(project: &Project) -> &Grade {
    &project.timelines[0]
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
        .find(|clip| clip.id == "c1")
        .expect("the clip is there")
        .grade
}

/// The sample with a corrector in the grade of c1, and the id of the node.
fn with_corrector() -> (Project, String) {
    let mut project = Project::sample();
    let added = Op::AddNode {
        grade: c1(),
        node: Node::new("", Kind::Corrector),
        index: None,
        links: vec![Link::rgb("source", ""), Link::rgb("", "output")],
    };
    apply(&mut project, added).unwrap();
    let id = grade(&project).nodes[0].id.clone();
    (project, id)
}

fn set_node(id: &str, params: serde_json::Value) -> Op {
    Op::SetNode {
        grade: c1(),
        id: id.into(),
        label: None,
        enabled: None,
        locked: None,
        pos: None,
        params: Some(params),
    }
}

fn keys(list: &[(i64, f64)]) -> Param {
    Param::Keys(
        list.iter()
            .map(|&(frame, value)| Key {
                frame,
                value,
                interp: Interp::Linear,
            })
            .collect(),
    )
}

/// A curve of three points whose middle one goes up and to the right from frame 0 to frame 10.
fn moving() -> Vec<CurvePoint> {
    let mut points = fixed_points(&[[0.0, 0.0], [0.25, 0.25], [1.0, 1.0]]);
    points[1] = [
        keys(&[(0, 0.25), (10, 0.75)]),
        keys(&[(0, 0.25), (10, 0.5)]),
    ];
    points
}

#[test]
fn the_points_of_a_curve_are_where_their_keys_put_them_at_a_frame() {
    let still = fixed_points(&[[0.0, 0.1], [0.4, 0.5], [1.0, 0.9]]);
    assert_eq!(
        points_at(&still, 123.0),
        vec![[0.0, 0.1], [0.4, 0.5], [1.0, 0.9]]
    );
    let moving = moving();
    assert_eq!(
        points_at(&moving, -3.0),
        vec![[0.0, 0.0], [0.25, 0.25], [1.0, 1.0]]
    );
    assert_eq!(
        points_at(&moving, 5.0),
        vec![[0.0, 0.0], [0.5, 0.375], [1.0, 1.0]]
    );
    assert_eq!(
        points_at(&moving, 10.0),
        vec![[0.0, 0.0], [0.75, 0.5], [1.0, 1.0]]
    );
    assert_eq!(points_at(&[], 0.0), Vec::<[f64; 2]>::new());
}

#[test]
fn points_that_would_pass_each_other_between_their_keys_are_held_in_their_order() {
    // In their order at frame 0 and at frame 10, and past each other in between: the second
    // goes right early, by a curve, and the third comes left late.
    let early = Interp::Bezier {
        out: [0.0, 0.5],
        into: [1.0, 0.0],
    };
    let mut points = fixed_points(&[[0.0, 0.0], [0.2, 0.3], [0.3, 0.6], [1.0, 1.0]]);
    points[1][0] = Param::Keys(vec![
        Key {
            frame: 0,
            value: 0.2,
            interp: early,
        },
        Key {
            frame: 10,
            value: 0.6,
            interp: Interp::Linear,
        },
    ]);
    points[2][0] = keys(&[(0, 0.3), (10, 0.7)]);
    for frame in 0..=40 {
        let at = points_at(&points, f64::from(frame) / 4.0);
        assert!(
            at.windows(2).all(|pair| pair[0][0] < pair[1][0]),
            "frame {}: {at:?}",
            f64::from(frame) / 4.0
        );
        assert!(at.iter().all(|[x, _]| (0.0..=1.0).contains(x)));
    }
    // And they did pass each other, as their keys have them.
    let raw = |at: f64| (points[1][0].value_at(at), points[2][0].value_at(at));
    let (second, third) = raw(3.0);
    assert!(second > third, "{second} and {third}");
    // Points at the end of the scale make room for one another.
    let mut crowded = fixed_points(&[[0.0, 0.0], [0.5, 0.5], [1.0, 1.0]]);
    crowded[1][0] = keys(&[(0, 0.5), (10, 1.0)]);
    crowded[0][0] = keys(&[(0, 0.0), (10, 0.0), (20, 1.0)]);
    let at = points_at(&crowded, 15.0);
    assert!(at.windows(2).all(|pair| pair[0][0] < pair[1][0]), "{at:?}");
    assert!(at[2][0] <= 1.0);
}

#[test]
fn a_point_is_written_as_two_numbers_or_with_the_keys_of_either() {
    let (mut project, id) = with_corrector();
    let moving = serde_json::to_value(moving()).unwrap();
    assert_eq!(
        moving,
        json!([
            [0.0, 0.0],
            [
                {"keys": [{"frame": 0, "value": 0.25, "interp": "linear"},
                          {"frame": 10, "value": 0.75, "interp": "linear"}]},
                {"keys": [{"frame": 0, "value": 0.25, "interp": "linear"},
                          {"frame": 10, "value": 0.5, "interp": "linear"}]},
            ],
            [1.0, 1.0],
        ])
    );
    apply(
        &mut project,
        set_node(&id, json!({"curves": {"y": {"points": moving}}})),
    )
    .unwrap();
    let NodeKind::Corrector(corrector) = &grade(&project).nodes[0].kind else {
        unreachable!("a corrector was added");
    };
    assert_eq!(corrector.curves.y.points, self::moving());
    assert!(!corrector.curves.y.is_diagonal());
    assert!(corrector.curves.r.is_diagonal());
    assert!(grade(&project).has_keys());
    // The project is read back as it is written, and one of numbers alone as it always was.
    let read = Project::from_value(serde_json::to_value(&project).unwrap()).unwrap();
    assert_eq!(read, project);
}

#[test]
fn the_points_of_a_curve_are_checked_where_they_have_keys() {
    let (mut project, id) = with_corrector();
    let before = project.clone();
    let refused =
        |project: &mut Project, params: serde_json::Value| apply(project, set_node(&id, params));
    let custom = Err(OpError::OutOfRange {
        what: "curves.y.points",
    });
    let key = |frame: i64, value: f64| json!({"frame": frame, "value": value, "interp": "linear"});
    // A value of a key beyond the scale.
    let beyond = json!([[0.0, 0.0], [{"keys": [key(0, 0.5), key(10, 1.5)]}, 0.5], [1.0, 1.0]]);
    assert_eq!(
        refused(&mut project, json!({"curves": {"y": {"points": beyond}}})),
        custom
    );
    // Two points at one place at a frame that has a key, and out of their order there.
    let met = json!([[0.0, 0.0], [{"keys": [key(0, 0.5), key(10, 1.0)]}, 0.5], [1.0, 1.0]]);
    assert_eq!(
        refused(&mut project, json!({"curves": {"y": {"points": met}}})),
        custom
    );
    let crossed = json!([
        [0.0, 0.0],
        [{"keys": [key(0, 0.2), key(10, 0.8)]}, 0.3],
        [{"keys": [key(0, 0.5), key(20, 0.9)]}, 0.6],
        [1.0, 1.0],
    ]);
    assert_eq!(
        refused(&mut project, json!({"curves": {"y": {"points": crossed}}})),
        custom
    );
    // Keys out of the order of their frames.
    let unordered = json!([[0.0, 0.0], [0.5, {"keys": [key(10, 0.5), key(0, 0.6)]}], [1.0, 1.0]]);
    assert_eq!(
        refused(
            &mut project,
            json!({"curves": {"y": {"points": unordered}}})
        ),
        custom
    );
    // The `out` of a point of an HSL curve goes from −1 to 1, with keys as without.
    let low = json!([[0.3, {"keys": [key(0, 0.0), key(10, -1.5)]}]]);
    assert_eq!(
        refused(&mut project, json!({"hsl": {"hue_vs_sat": low}})),
        Err(OpError::OutOfRange {
            what: "hsl.hue_vs_sat"
        })
    );
    assert_eq!(project, before);
    // In their order at each of their keys, they are taken.
    let taken = json!([
        [0.0, 0.0],
        [{"keys": [key(0, 0.2), key(10, 0.4)]}, 0.3],
        [{"keys": [key(0, 0.5), key(20, 0.9)]}, 0.6],
        [1.0, 1.0],
    ]);
    refused(&mut project, json!({"curves": {"y": {"points": taken}}})).unwrap();
    let down = json!([[0.3, {"keys": [key(0, 0.0), key(10, -1.0)]}]]);
    refused(&mut project, json!({"hsl": {"hue_vs_sat": down}})).unwrap();
}

#[test]
fn the_keys_of_a_point_are_set_by_its_path() {
    let (mut project, id) = with_corrector();
    apply(
        &mut project,
        set_node(
            &id,
            json!({"curves": {"y": {"points": [[0.0, 0.0], [0.5, 0.5], [1.0, 1.0]]}},
                   "hsl": {"lum_vs_sat": [[0.5, 0.0]]}}),
        ),
    )
    .unwrap();
    let set = |path: &str, param: Param| Op::SetKeys {
        grade: c1(),
        id: id.clone(),
        path: path.into(),
        param,
    };
    let up = keys(&[(0, 0.5), (24, 0.8)]);
    let inverse = apply(&mut project, set("curves.y.points.1.1", up.clone())).unwrap();
    assert_eq!(inverse, set("curves.y.points.1.1", Param::Fixed(0.5)));
    let spliced = Op::SpliceKeys {
        grade: c1(),
        id: id.clone(),
        path: "hsl.lum_vs_sat.0.1".into(),
        at: 0,
        remove: 0,
        keys: vec![Key {
            frame: 12,
            value: -0.5,
            interp: Interp::Hold,
        }],
    };
    apply(&mut project, spliced).unwrap();
    let NodeKind::Corrector(corrector) = &grade(&project).nodes[0].kind else {
        unreachable!("a corrector was added");
    };
    assert_eq!(corrector.curves.y.points[1], [Param::Fixed(0.5), up]);
    assert_eq!(
        points_at(&corrector.hsl.lum_vs_sat, 100.0),
        vec![[0.5, -0.5]]
    );
    let half_way = corrector.curves.y.at(12.0)[1];
    assert!(
        (half_way[0] - 0.5).abs() < 1e-12 && (half_way[1] - 0.65).abs() < 1e-12,
        "{half_way:?}"
    );
    // A point that the curve has not, and a number of a point that no point has.
    for path in [
        "curves.y.points.3.0",
        "curves.y.points.1.2",
        "hsl.lum_vs_sat.1.0",
    ] {
        assert_eq!(
            apply(&mut project, set(path, Param::Fixed(0.5))),
            Err(OpError::Missing {
                what: "setting",
                id: path.into()
            }),
            "{path}"
        );
    }
    // The `in` of a point may not be set past its neighbour.
    assert_eq!(
        apply(&mut project, set("curves.y.points.1.0", Param::Fixed(1.0))),
        Err(OpError::OutOfRange {
            what: "curves.y.points"
        })
    );
}
