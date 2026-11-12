//! Random operations on grades and groups: they name the grades, nodes, links and groups of the
//! project most of the time, and now and then ones that are not there, ids that are taken or
//! reserved, links that go round or into inputs that are not there, and settings out of their
//! bounds.

use columbina_model::grade::{
    Grade, Group, Interp, Key, Kind, Link, Node, Param, Port, Pos, Qualifier, Target,
};
use columbina_model::{Op, Project};

/// The grades of a project, each by its target, and one that is not there.
pub fn targets(project: &Project) -> Vec<Target> {
    let mut targets = vec![];
    for timeline in &project.timelines {
        targets.push(Target::Timeline {
            timeline: timeline.id.clone(),
        });
        for track in &timeline.tracks {
            for clip in &track.clips {
                targets.push(Target::Clip {
                    timeline: timeline.id.clone(),
                    clip: clip.id.clone(),
                });
            }
        }
    }
    for group in &project.groups {
        targets.push(Target::PreClip {
            group: group.id.clone(),
        });
        targets.push(Target::PostClip {
            group: group.id.clone(),
        });
    }
    targets
}

/// The grade that `target` names, if it is there.
pub fn grade_of<'a>(project: &'a Project, target: &Target) -> Option<&'a Grade> {
    match target {
        Target::Clip { timeline, clip } => project
            .timelines
            .iter()
            .find(|t| t.id == *timeline)?
            .tracks
            .iter()
            .flat_map(|r| &r.clips)
            .find(|c| c.id == *clip)
            .map(|c| &c.grade),
        Target::Timeline { timeline } => project
            .timelines
            .iter()
            .find(|t| t.id == *timeline)
            .map(|t| &t.grade),
        Target::PreClip { group } => project
            .groups
            .iter()
            .find(|k| k.id == *group)
            .map(|k| &k.pre),
        Target::PostClip { group } => project
            .groups
            .iter()
            .find(|k| k.id == *group)
            .map(|k| &k.post),
    }
}

/// One of `items`, or one time in sixteen `bogus`.
fn pick<T: Clone>(items: &[T], n: usize, bogus: T) -> T {
    if n.is_multiple_of(16) || items.is_empty() {
        bogus
    } else {
        items[n / 16 % items.len()].clone()
    }
}

fn kind_of(n: usize) -> Kind {
    match n % 8 {
        0 => Kind::ParallelMixer,
        1 => Kind::LayerMixer,
        2 => Kind::KeyMixer,
        _ => Kind::Corrector,
    }
}

fn port_of(n: usize) -> Port {
    if n.is_multiple_of(4) {
        Port::Key
    } else {
        Port::Rgb
    }
}

/// The ends that links may name in `grade`: its nodes, the source and the output, and one that
/// is not there.
fn ends(grade: Option<&Grade>) -> Vec<String> {
    let mut ends: Vec<String> = grade
        .map(|g| g.nodes.iter().map(|n| n.id.clone()).collect())
        .unwrap_or_default();
    ends.push("source".into());
    ends.push("output".into());
    ends
}

/// A link between two of `ends`, made of `n`.
fn link_of(ends: &[String], n: usize) -> Link {
    Link {
        from: pick(ends, n, "n999".into()),
        to: pick(ends, n / 7 + 3, "n999".into()),
        kind: port_of(n / 5),
        input: u32::try_from(n / 11 % 3).unwrap_or(0) * u32::from(n.is_multiple_of(3)),
    }
}

/// A change to the settings of a node, made of `n`, which is out of bounds or of another kind
/// of node now and then.
fn params_of(n: usize) -> serde_json::Value {
    let value = f64::from(u32::try_from(n % 101).unwrap_or(0));
    match n % 21 {
        0 => serde_json::json!({"primaries": {"saturation": value}}),
        1 => serde_json::json!({"primaries": {"lift": {"r": value / 100.0 - 0.5, "y": 0.1}}}),
        2 => serde_json::to_value(serde_json::json!({"primaries": {"contrast": {"keys": [
            Key { frame: 0, value: 1.0, interp: Interp::Linear },
            Key { frame: i64::try_from(n % 50).unwrap_or(0) + 1, value: value / 50.0,
                  interp: Interp::Bezier { out: [0.3, 0.0], into: [0.3, 0.0] } },
        ]}}}))
        .unwrap(),
        3 => {
            serde_json::json!({"lut": if n.is_multiple_of(2) { serde_json::json!("LUTs/film.cube") } else { serde_json::Value::Null }})
        }
        4 => {
            serde_json::json!({"curves": {"ganged": false, "y": {"points": [[0.0, 0.0], [0.5, value / 100.0], [1.0, 1.0]]}}})
        }
        5 => serde_json::json!({"hsl": {"hue_vs_sat": [[0.1, 0.2], [0.6, -0.4]]}}),
        6 => serde_json::json!({"inputs": n % 5}),
        7 => serde_json::json!({"composite": "screen"}),
        8 => serde_json::json!({"key": {"output": {"gain": value / 50.0}}}),
        9 => serde_json::json!({"primaries": {"saturation": 500}}),
        10 => serde_json::json!({"bogus": 1}),
        11 => {
            let input = serde_json::json!({"invert": true, "matte": false, "gain": 1, "offset": 0});
            serde_json::json!({ "inputs": vec![input; 3] })
        }
        12 => serde_json::json!({"primaries": {"saturation": {"keys": []}}}),
        // A shot match given whole, or taken away.
        13 => {
            if n.is_multiple_of(4) {
                serde_json::json!({"shot_match": null})
            } else {
                serde_json::json!({"shot_match": {
                    "gamma": {"r": 0.9, "g": 1.0, "b": value / 50.0 + 0.5},
                    "gain": {"r": 1.1, "g": 1.0, "b": 0.95},
                    "offset": {"r": -0.02, "g": 0.0, "b": 0.01},
                    "contrast": value / 40.0,
                    "pivot": 0.4,
                }})
            }
        }
        // A part of one, which a node without one refuses, and one out of its bounds.
        14 => serde_json::json!({"shot_match": {"gain": {"g": value / 50.0}, "pivot": 0.5}}),
        15 => serde_json::json!({"shot_match": {"gamma": {"r": 0.0}}}),
        // A qualifier given whole, or taken away.
        16 => {
            if n.is_multiple_of(4) {
                serde_json::json!({"qualifier": null})
            } else {
                let mut whole = serde_json::to_value(Qualifier::default()).unwrap();
                whole["mode"] = ["hsl", "rgb", "lum"][n / 3 % 3].into();
                whole["hue"]["center"] = value.into();
                whole["hue"]["width"] = (value / 4.0).into();
                whole["sat"]["on"] = n.is_multiple_of(3).into();
                whole["finesse"]["blur_radius"] = (value * 3.0).into();
                whole["finesse"]["morph"] =
                    ["shrink", "grow", "opening", "closing"][n / 7 % 4].into();
                serde_json::json!({ "qualifier": whole })
            }
        }
        // A part of one, which a node without one refuses; and keys of a range.
        17 => serde_json::json!({"qualifier": {"lum": {"low": value / 2.0, "high_soft": 10}}}),
        18 => serde_json::to_value(
            serde_json::json!({"qualifier": {"finesse": {"clean_black": {"keys": [
                Key { frame: 0, value: 0.0, interp: Interp::Linear },
                Key { frame: i64::try_from(n % 40).unwrap_or(0) + 1, value, interp: Interp::Hold },
            ]}}}}),
        )
        .unwrap(),
        // Out of its bounds.
        19 => serde_json::json!({"qualifier": {"finesse": {"in_out_ratio": -101}}}),
        _ => serde_json::json!({"key": {
            "qualifier": {"invert": n.is_multiple_of(2), "gain": value / 50.0},
            "input_blur": {"radius": value * 30.0, "ratio": value / 100.0},
        }}),
    }
}

/// Where a node lies, made of `n`: now and then further than a node may lie.
fn pos_of(n: usize) -> Pos {
    let step = f64::from(u32::try_from(n % 997).unwrap_or(0));
    match n % 41 {
        0 => Pos { x: 2.0e6, y: 0.0 },
        1 => Pos { x: 0.0, y: -3.0e6 },
        _ => Pos {
            x: step * 7.25 - 900.0,
            y: step * 0.5,
        },
    }
}

/// A grade of a few nodes in a row, now and then with a link back or of an id that is
/// reserved, made of `n` for a project whose `next_id` is `next`.
fn grade_made_of(n: usize, next: u64) -> Grade {
    let count = n % 4;
    let mut nodes: Vec<Node> = (0..count)
        .map(|k| {
            let mut node = Node::new(format!("a{k}"), kind_of(n / 3 + k));
            node.locked = (n + k) % 5 == 2;
            node.pos = (n / 5 + k).is_multiple_of(2).then(|| pos_of(n + k * 31));
            node
        })
        .collect();
    if n % 9 == 1 && !nodes.is_empty() {
        nodes[0].id = format!("n{next}");
    }
    let mut ids: Vec<String> = vec!["source".into()];
    ids.extend(nodes.iter().map(|node| node.id.clone()));
    ids.push("output".into());
    let mut links: Vec<Link> = ids.windows(2).map(|w| Link::rgb(&w[0], &w[1])).collect();
    if n % 7 == 1 && count > 1 {
        links.push(Link {
            from: ids[count].clone(),
            to: ids[1].clone(),
            kind: Port::Key,
            input: 0,
        });
    }
    Grade { nodes, links }
}

/// An operation on the keys of a setting of `node`, made of `n`: of a corrector or of a Key
/// Mixer, of a palette that the node may not have, or of nothing that takes keys.
fn keys_op_of(grade: Target, node: String, n: [usize; 4]) -> Op {
    let paths = [
        "primaries.lift.r",
        "primaries.saturation",
        "key.output.gain",
        "blur.radius.g",
        "inputs.0.gain",
        "output.offset",
        "qualifier.hue.center",
        "windows.0.opacity",
        "curves.y.points.0.0",
        "nothing.here",
    ];
    let path = paths[n[2] % paths.len()].to_owned();
    #[allow(clippy::cast_possible_wrap, reason = "small numbers")]
    let first = (n[1] % 40) as i64 - 10;
    let key = |frame: i64, value: f64, held: bool| Key {
        frame,
        value,
        interp: if held {
            Interp::Hold
        } else {
            Interp::Bezier {
                out: [0.25, 0.125],
                into: [0.5, -0.25],
            }
        },
    };
    // Values that most of the settings take, one that few do, and frames out of order.
    let value = [0.5, 0.25, 0.75, 250.0][n[3] / 4 % 4];
    let step = [12, 1, 30, -3][n[3] / 16 % 4];
    let keys = vec![
        key(first, value, n[3].is_multiple_of(3)),
        key(first + step, 0.5, n[3].is_multiple_of(5)),
    ];
    match n[1] % 4 {
        0 => Op::SetKeys {
            grade,
            id: node,
            path,
            param: Param::Fixed(value),
        },
        1 => Op::SetKeys {
            grade,
            id: node,
            path,
            param: Param::Keys(keys),
        },
        _ => Op::SpliceKeys {
            grade,
            id: node,
            path,
            at: n[3] / 64 % 3,
            remove: n[3] / 192 % 3,
            keys: keys[..n[3] / 576 % 3].to_vec(),
        },
    }
}

/// An operation on the grades or the groups of `project`, made of `n`.
#[allow(clippy::too_many_lines, reason = "an arm for each operation")]
pub fn grade_op_of(project: &Project, n: [usize; 4]) -> Op {
    let targets = targets(project);
    let bogus = Target::Clip {
        timeline: "t1".into(),
        clip: "c99".into(),
    };
    let target = pick(&targets, n[1], bogus);
    let grade = grade_of(project, &target);
    let ends = ends(grade);
    let nodes: Vec<String> = grade
        .map(|g| g.nodes.iter().map(|node| node.id.clone()).collect())
        .unwrap_or_default();
    let node = pick(&nodes, n[2], "n999".into());
    let groups: Vec<String> = project.groups.iter().map(|k| k.id.clone()).collect();
    let group = pick(&groups, n[2], "k999".into());
    // Some of the changes of a node are those of the keys of one of its settings.
    if n[0] % 14 == 9 && n[3] % 4 == 1 {
        return keys_op_of(target, node, n);
    }
    match n[0] % 14 {
        0..=2 => {
            let mut new = Node::new("", kind_of(n[3]));
            new.id = match n[3] % 16 {
                0 => node.clone(),
                1 => "source".into(),
                2 => "$x".into(),
                3 => format!("n{}", project.next_id),
                4 => "mine".into(),
                _ => String::new(),
            };
            if n[3] % 23 == 5 {
                new.label = "x".repeat(101);
            }
            new.locked = n[3] % 5 == 3;
            if n[3] % 3 == 1 {
                new.pos = Some(pos_of(n[3] / 3));
            }
            let given = new.id.clone();
            let links = (0..n[2] % 4)
                .map(|k| {
                    let mut link = link_of(&ends, n[3] / 3 + k * 17);
                    if (n[1] + k).is_multiple_of(2) {
                        link.from.clone_from(&given);
                    } else {
                        link.to.clone_from(&given);
                    }
                    link
                })
                .collect();
            let len = grade.map_or(0, |g| g.nodes.len());
            Op::AddNode {
                grade: target,
                node: new,
                index: match n[3] % 8 {
                    0 => Some(len + 1),
                    1 => Some(n[2] % (len + 1)),
                    _ => None,
                },
                links,
            }
        }
        3 | 4 => Op::RemoveNode {
            grade: target,
            id: node,
            links: n[3].is_multiple_of(5).then(|| {
                (0..n[2] % 3)
                    .map(|k| link_of(&ends, n[3] / 5 + k * 13))
                    .collect()
            }),
        },
        5 | 6 => Op::Connect {
            grade: target,
            link: link_of(&ends, n[3]),
        },
        7 => {
            let links = grade.map(|g| g.links.clone()).unwrap_or_default();
            let link = pick(&links, n[3], link_of(&ends, n[2]));
            Op::Disconnect {
                grade: target,
                to: link.to,
                kind: link.kind,
                input: link.input,
            }
        }
        8 | 9 => Op::SetNode {
            grade: target,
            id: node,
            label: n[3].is_multiple_of(3).then(|| format!("L{}", n[2] % 100)),
            enabled: n[3].is_multiple_of(5).then_some(n[2].is_multiple_of(2)),
            locked: (n[3] % 7 < 2).then_some(n[2].is_multiple_of(3)),
            pos: match n[3] % 6 {
                1 => Some(None),
                2 | 3 => Some(Some(pos_of(n[2] / 5))),
                _ => None,
            },
            params: (n[3] % 4 != 1).then(|| params_of(n[2] / 3)),
        },
        10 => {
            let value = if n[3].is_multiple_of(3) {
                let others = targets.clone();
                let from = pick(&others, n[2], target.clone());
                grade_of(project, &from).cloned().unwrap_or_default()
            } else {
                grade_made_of(n[3] / 3, project.next_id)
            };
            Op::SetGrade {
                grade: target,
                value,
            }
        }
        11 => {
            let mut new = Group::new(
                match n[3] % 8 {
                    0 => group.clone(),
                    1 => format!("k{}", project.next_id),
                    _ => String::new(),
                },
                format!("Group {}", n[2] % 10),
            );
            if n[3] % 5 == 2 {
                new.pre = grade_made_of(n[2], project.next_id);
            }
            Op::AddGroup {
                group: new,
                index: n[3].is_multiple_of(3).then_some(n[2] % (groups.len() + 2)),
            }
        }
        12 => {
            if n[3].is_multiple_of(2) {
                Op::RemoveGroup { id: group }
            } else {
                Op::SetGroup {
                    id: group,
                    name: Some(format!("Renamed {}", n[3] % 7)),
                }
            }
        }
        _ => {
            let clips: Vec<(String, String)> = project
                .timelines
                .iter()
                .flat_map(|t| {
                    t.tracks
                        .iter()
                        .flat_map(|r| &r.clips)
                        .map(|c| (t.id.clone(), c.id.clone()))
                })
                .collect();
            let (timeline, clip) = pick(&clips, n[3], ("t1".into(), "c99".into()));
            Op::SetClipGroup {
                timeline,
                clip,
                group: (!n[2].is_multiple_of(3)).then_some(group),
            }
        }
    }
}
