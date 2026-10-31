//! Random operations and edits of grades and groups, mixed with the other operations: each one
//! that is applied is taken back exactly by its inverse and done again by what it gave back as
//! done, each one that is refused changes nothing, and after each one every grade still has
//! nodes of their own ids and links in order that go round in no loop, and the project is one
//! that `Project::validate` leaves as it is.

mod common;

use std::collections::BTreeMap;

use columbina_model::grade::{Node, Target};
use columbina_model::keyframes::Align;
use columbina_model::{apply, apply_done, expand, Edit, Group, Link, Op, Project};
use common::grades::{grade_of, grade_op_of, targets};
use common::{check, messy, op_of, spec, tidy, Spec};
use proptest::prelude::*;

/// The project that the tests start from: the tidy one with c1 in group k9, a node in the
/// grade of c1, of the timeline and of the pre-clip grade of the group, and a key link.
fn graded() -> Project {
    let mut project = tidy();
    let c1 = Target::Clip {
        timeline: "t1".into(),
        clip: "c1".into(),
    };
    let ops = vec![
        Op::AddGroup {
            group: Group::new("", "Interview"),
            index: None,
        },
        Op::SetClipGroup {
            timeline: "t1".into(),
            clip: "c1".into(),
            group: Some(format!("k{}", project.next_id)),
        },
    ];
    for op in ops {
        apply(&mut project, op).unwrap();
    }
    let group = project.groups[0].id.clone();
    for grade in [
        c1.clone(),
        c1,
        Target::Timeline {
            timeline: "t1".into(),
        },
        Target::PreClip { group },
    ] {
        let op = Op::AddNode {
            grade,
            node: Node::corrector(""),
            index: None,
            links: vec![Link::rgb("", "output")],
        };
        apply(&mut project, op).unwrap();
    }
    check(&project);
    project
}

/// The makings of an operation: one on grades, or now and then any other.
#[derive(Clone, Debug)]
enum Making {
    Grade([usize; 4]),
    Other(Spec),
}

fn making() -> impl Strategy<Value = Making> {
    prop_oneof![
        8 => any::<[usize; 4]>().prop_map(Making::Grade),
        1 => spec().prop_map(Making::Other),
    ]
}

fn op_made(project: &Project, making: &Making) -> Op {
    match making {
        Making::Grade(n) => grade_op_of(project, *n),
        Making::Other(spec) => op_of(project, spec),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn every_operation_on_grades_is_taken_back_or_changes_nothing(
        start_messy in prop::bool::weighted(0.2),
        makings in prop::collection::vec(making(), 1..30)
    ) {
        let mut project = if start_messy { messy() } else { graded() };
        for making in &makings {
            let op = op_made(&project, making);
            let before = project.clone();
            let Ok((done, inverse)) = apply_done(&mut project, op.clone()) else {
                prop_assert_eq!(&project, &before, "{:?}", op);
                continue;
            };
            check(&project);
            let after = project.clone();
            let mut undone = after.clone();
            let again = apply(&mut undone, inverse.clone())
                .unwrap_or_else(|e| panic!("{inverse:?} of {op:?}: {e}"));
            let mut expected = before.clone();
            expected.next_id = after.next_id;
            prop_assert_eq!(&undone, &expected, "{:?} then {:?}", op, inverse);
            prop_assert_eq!(&done, &again, "{:?}", op);
            apply(&mut undone, again.clone())
                .unwrap_or_else(|e| panic!("{again:?} of {op:?}: {e}"));
            prop_assert_eq!(&undone, &after, "{:?} redone as {:?}", op, again);
            // What the project is written as is read back as it is.
            let read = Project::from_value(serde_json::to_value(&project).unwrap()).unwrap();
            prop_assert_eq!(&read, &project);
        }
    }
}

/// The settings that `paste_value` is given, one of which no node has.
const PARAMS: [&str; 6] = [
    "primaries.saturation",
    "primaries.lift",
    "curves",
    "lut",
    "inputs",
    "nothing",
];

/// How the keys of a copied grade are put on the clip it is copied to: as `DaVinci` does at
/// first, or either way named.
fn align(k: usize) -> Option<Align> {
    match k % 3 {
        0 => None,
        1 => Some(Align::SourceTimecode),
        _ => Some(Align::StartFrame),
    }
}

/// An edit of grades made of `n` against `project`.
fn edit_of(project: &Project, n: [usize; 5]) -> Edit {
    let targets = targets(project);
    let pick_target = |k: usize| {
        if targets.is_empty() || k.is_multiple_of(16) {
            Target::Clip {
                timeline: "t1".into(),
                clip: "c99".into(),
            }
        } else {
            targets[k / 16 % targets.len()].clone()
        }
    };
    let grade = pick_target(n[1]);
    let node_of = |target: &Target, k: usize| {
        let mut ids: Vec<String> = grade_of(project, target)
            .map(|g| g.nodes.iter().map(|node| node.id.clone()).collect())
            .unwrap_or_default();
        ids.extend(["source".to_string(), "output".to_string()]);
        if k.is_multiple_of(13) {
            "n999".into()
        } else if k % 7 == 1 {
            ids[ids.len() - 1 - k / 7 % 2].clone()
        } else {
            ids[k / 3 % ids.len()].clone()
        }
    };
    let node = node_of(&grade, n[2]);
    let others = |k: usize| (0..k % 3).map(|j| pick_target(n[4] / (j + 1))).collect();
    let clips: Vec<String> = project
        .timelines
        .iter()
        .filter(|t| t.id == "t1")
        .flat_map(|t| &t.tracks)
        .flat_map(|r| &r.clips)
        .map(|c| c.id.clone())
        .collect();
    let pick_clip = |k: usize| {
        if clips.is_empty() || k.is_multiple_of(16) {
            "c99".to_string()
        } else {
            clips[k / 16 % clips.len()].clone()
        }
    };
    match n[0] % 12 {
        0 => Edit::AddSerial { grade, node },
        1 => Edit::AddSerialBefore { grade, node },
        2 => Edit::AppendNode { grade },
        3 => Edit::AddParallel { grade, node },
        4 => Edit::AddLayer { grade, node },
        5 => Edit::ExtractNode { grade, node },
        6 => Edit::ResetNode { grade, node },
        7 => Edit::CopyGrade {
            from: grade,
            to: others(n[3]),
            preserve_nodes: (!n[4].is_multiple_of(3)).then_some(u32::try_from(n[4] % 13).unwrap()),
            keyframes: align(n[2] / 5),
        },
        8 => Edit::AppendNodeGraph {
            from: grade,
            to: others(n[3]),
            keyframes: align(n[2] / 5),
        },
        9 => {
            let to = pick_target(n[3]);
            let to_node = node_of(&to, n[4]);
            Edit::PasteValue {
                from: grade,
                from_node: node,
                to,
                to_node,
                param: PARAMS[n[4] % PARAMS.len()].into(),
            }
        }
        11 => Edit::CollapseGroupGrades {
            timeline: if n[1].is_multiple_of(17) {
                "t9".into()
            } else {
                "t1".into()
            },
            clips: (0..n[3] % 3).map(|j| pick_clip(n[4] / (j + 1))).collect(),
        },
        _ => {
            let to = pick_target(n[3]);
            let to_node = node_of(&to, n[4]);
            Edit::PasteNode {
                from: grade,
                from_node: node,
                to,
                to_node,
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    #[test]
    fn every_edit_of_grades_is_undone_and_redone_exactly_or_changes_nothing(
        makings in prop::collection::vec((any::<[usize; 5]>(), any::<[usize; 4]>()), 1..20)
    ) {
        let mut project = graded();
        for (n, m) in &makings {
            // Now and then an operation first, so that the grades are of many shapes.
            if m[0].is_multiple_of(3) {
                let op = grade_op_of(&project, *m);
                let _ = apply(&mut project, op);
            }
            let edit = edit_of(&project, *n);
            let before = project.clone();
            let Ok(expansion) = expand(&mut project, edit.clone()) else {
                prop_assert_eq!(&project, &before, "{:?}", edit);
                continue;
            };
            check(&project);
            let after = project.clone();
            let mut undone = after.clone();
            for inverse in expansion.inverse.iter().rev() {
                apply(&mut undone, inverse.clone())
                    .unwrap_or_else(|e| panic!("{inverse:?} of {edit:?}: {e}"));
            }
            let mut expected = before;
            expected.next_id = after.next_id;
            prop_assert_eq!(&undone, &expected, "{:?}", edit);
            for done in &expansion.done {
                apply(&mut undone, done.clone())
                    .unwrap_or_else(|e| panic!("{done:?} of {edit:?}: {e}"));
            }
            prop_assert_eq!(&undone, &after, "{:?} redone", edit);
        }
    }
}

/// A sequence of numbers that is the same on every run.
struct Numbers(u64);

impl Numbers {
    fn next(&mut self) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        usize::try_from(self.0 >> 33).unwrap()
    }

    fn four(&mut self) -> [usize; 4] {
        [self.next(), self.next(), self.next(), self.next()]
    }
}

#[test]
fn the_random_operations_and_edits_of_grades_are_applied_as_well_as_refused() {
    let mut numbers = Numbers(7);
    let mut applied: BTreeMap<String, usize> = BTreeMap::new();
    let mut refused: BTreeMap<String, usize> = BTreeMap::new();
    for _ in 0..40 {
        let mut project = graded();
        for _ in 0..60 {
            let op = grade_op_of(&project, numbers.four());
            let name = serde_json::to_value(&op).unwrap()["op"]
                .as_str()
                .unwrap()
                .to_owned();
            match apply(&mut project, op) {
                Ok(_) => *applied.entry(name).or_default() += 1,
                Err(_) => *refused.entry(name).or_default() += 1,
            }
            let n = [
                numbers.next(),
                numbers.next(),
                numbers.next(),
                numbers.next(),
                numbers.next(),
            ];
            let edit = edit_of(&project, n);
            let name = serde_json::to_value(&edit).unwrap()["edit"]
                .as_str()
                .unwrap()
                .to_owned();
            match expand(&mut project, edit) {
                Ok(_) => *applied.entry(name).or_default() += 1,
                Err(_) => *refused.entry(name).or_default() += 1,
            }
        }
    }
    for name in [
        "add_node",
        "remove_node",
        "connect",
        "disconnect",
        "set_node",
        "set_grade",
        "add_group",
        "remove_group",
        "set_group",
        "set_clip_group",
        "add_serial",
        "add_serial_before",
        "append_node",
        "add_parallel",
        "add_layer",
        "extract_node",
        "reset_node",
        "copy_grade",
        "append_node_graph",
        "collapse_group_grades",
        "paste_value",
        "paste_node",
    ] {
        let (ok, no) = (
            applied.get(name).copied().unwrap_or(0),
            refused.get(name).copied().unwrap_or(0),
        );
        assert!(ok >= 5 && no >= 1, "{name}: {ok} applied, {no} refused");
    }
}
