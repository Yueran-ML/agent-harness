//! The edits of grades: adding nodes as the keys of the Color page add them (Reference Manual
//! 20, p. 3318), taking a node out of its tree, resetting one, and copying grades, nodes and
//! single settings from one grade to others (pp. 3285–3297).

use std::collections::{HashMap, HashSet};

use super::{EditError, Expansion};
use crate::grade::{
    at_path, patch_at, Grade, Kind, Link, Node, NodeKind, Port, Target, OUTPUT, SOURCE,
};
use crate::ops::{apply_done, grade_at, reserve_node_ids, Op, OpError};
use crate::project::{Id, Project};

/// The operations that an edit has been worked out into so far.
#[derive(Default)]
struct Steps {
    done: Vec<Op>,
    inverse: Vec<Op>,
}

impl Steps {
    /// Apply an operation, and give it back as it was done.
    fn apply(&mut self, project: &mut Project, op: Op) -> Result<Op, EditError> {
        let (done, inverse) = apply_done(project, op)?;
        self.done.push(done.clone());
        self.inverse.push(inverse);
        Ok(done)
    }

    fn finish(self) -> Expansion {
        Expansion {
            done: self.done,
            inverse: self.inverse,
            clamped: false,
            created: vec![],
        }
    }
}

/// Where a node is to be added, beside the node that is chosen.
#[derive(Clone, Copy)]
pub(super) enum Beside {
    /// After it (Add Serial, Option-S).
    After,
    /// Before it (Add Before Current, Shift-S).
    Before,
    /// Beside it, into a parallel mixer (Add Parallel, Option-P).
    Parallel,
    /// Beside it, into a layer mixer, on top (Add Layer, Option-L).
    Layer,
}

/// Add a corrector beside `node` of `grade`.
pub(super) fn add(
    project: &mut Project,
    grade: &Target,
    node: &str,
    beside: Beside,
) -> Result<Expansion, EditError> {
    let mut steps = Steps::default();
    let g = grade_at(project, grade)?.clone();
    let corrector = Node::corrector("");
    match beside {
        Beside::After => {
            if !g.has_output(node, Port::Rgb).ok_or_else(|| missing(node))? {
                return Err(wrong(&g, node).into());
            }
            let mut links = vec![Link::rgb(node, "")];
            links.extend(g.links_from(node, Port::Rgb).map(|out| Link {
                from: String::new(),
                ..out.clone()
            }));
            steps.apply(project, add_node(grade, corrector, links))?;
        }
        Beside::Before => {
            serial_input(&g, node)?;
            let mut links = vec![Link::rgb("", node)];
            if let Some(up) = g.link_into(node, Port::Rgb, 0) {
                links.push(Link::rgb(up.from.clone(), ""));
            }
            steps.apply(project, add_node(grade, corrector, links))?;
        }
        Beside::Parallel | Beside::Layer => {
            let mixer = if matches!(beside, Beside::Parallel) {
                Kind::ParallelMixer
            } else {
                Kind::LayerMixer
            };
            beside_mixer(project, &mut steps, grade, &g, node, mixer)?;
        }
    }
    Ok(steps.finish())
}

/// Add a corrector beside `node` into a mixer of `kind`: into the one that `node` alone goes
/// into, as a new input below its others, or into a new one that takes the place of `node` in
/// what it went into (p. 3334).
fn beside_mixer(
    project: &mut Project,
    steps: &mut Steps,
    grade: &Target,
    g: &Grade,
    node: &str,
    kind: Kind,
) -> Result<(), EditError> {
    serial_input(g, node)?;
    if node == OUTPUT {
        return Err(wrong(g, node).into());
    }
    let up = g.link_into(node, Port::Rgb, 0).map(|l| l.from.clone());
    let outs: Vec<Link> = g.links_from(node, Port::Rgb).cloned().collect();
    let into = |from: Id| up.iter().map(move |up| Link::rgb(up.clone(), from.clone()));
    if let [out] = &outs[..] {
        if g.node(&out.to).is_some_and(|m| m.kind.kind() == kind) {
            let inputs = g.inputs_of(&out.to, Port::Rgb).unwrap_or(0);
            let more = Op::SetNode {
                grade: grade.clone(),
                id: out.to.clone(),
                label: None,
                enabled: None,
                params: Some(serde_json::json!({ "inputs": inputs + 1 })),
            };
            steps.apply(project, more)?;
            let mut links: Vec<Link> = into(String::new()).collect();
            links.push(Link {
                from: String::new(),
                to: out.to.clone(),
                kind: Port::Rgb,
                input: inputs,
            });
            steps.apply(project, add_node(grade, Node::corrector(""), links))?;
            return Ok(());
        }
    }
    let added = steps.apply(
        project,
        add_node(grade, Node::corrector(""), into(String::new()).collect()),
    )?;
    let Op::AddNode { node: added, .. } = added else {
        unreachable!("an added node is done as added")
    };
    let mut links = vec![
        Link::rgb(node, ""),
        Link {
            from: added.id,
            to: String::new(),
            kind: Port::Rgb,
            input: 1,
        },
    ];
    links.extend(outs.into_iter().map(|out| Link {
        from: String::new(),
        ..out
    }));
    steps.apply(project, add_node(grade, Node::new("", kind), links))?;
    Ok(())
}

/// Check that a node takes the picture at one input, as a corrector and the output do, so that
/// a node can go before it.
fn serial_input(g: &Grade, node: &str) -> Result<(), EditError> {
    if node == OUTPUT {
        return Ok(());
    }
    match g.node(node) {
        None if node == SOURCE => Err(wrong(g, node).into()),
        None => Err(missing(node).into()),
        Some(n) if matches!(n.kind, NodeKind::Corrector(_)) => Ok(()),
        Some(_) => Err(wrong(g, node).into()),
    }
}

/// Add a corrector at the end of the tree of `grade`: before its output (Append Node,
/// Option-K).
pub(super) fn append_node(project: &mut Project, grade: &Target) -> Result<Expansion, EditError> {
    add(project, grade, OUTPUT, Beside::Before)
}

/// Take `node` out of the tree of `grade` without taking it away, linking what was on either
/// side of it as taking it away does (Extract, p. 3319).
pub(super) fn extract(
    project: &mut Project,
    grade: &Target,
    node: &str,
) -> Result<Expansion, EditError> {
    let g = grade_at(project, grade)?;
    let at = g.position(node).ok_or_else(|| missing(node))?;
    let kept = g.nodes[at].clone();
    let mut steps = Steps::default();
    let remove = Op::RemoveNode {
        grade: grade.clone(),
        id: node.into(),
        links: None,
    };
    steps.apply(project, remove)?;
    let again = Op::AddNode {
        grade: grade.clone(),
        node: kept,
        index: Some(at),
        links: vec![],
    };
    steps.apply(project, again)?;
    Ok(steps.finish())
}

/// Set `node` of `grade` as a new node of its kind is set, keeping its label, whether it is
/// enabled and how many inputs it has.
pub(super) fn reset(
    project: &mut Project,
    grade: &Target,
    node: &str,
) -> Result<Expansion, EditError> {
    let g = grade_at(project, grade)?;
    let n = g.node(node).ok_or_else(|| missing(node))?;
    let params = match &n.kind {
        NodeKind::Corrector(_) => NodeKind::new(Kind::Corrector).params(),
        NodeKind::ParallelMixer(_) => serde_json::json!({}),
        NodeKind::LayerMixer(_) => {
            let default = NodeKind::new(Kind::LayerMixer).params();
            serde_json::json!({ "composite": default["composite"] })
        }
        NodeKind::KeyMixer(mixer) => {
            let default = crate::grade::KeyMixer::default();
            let inputs = vec![default.inputs[0].clone(); mixer.inputs.len()];
            serde_json::json!({ "inputs": inputs, "output": default.output })
        }
    };
    let mut steps = Steps::default();
    steps.apply(project, set_params(grade, node, params))?;
    Ok(steps.finish())
}

/// Put the grade `from` in place of each grade of `to`, as Copy Grade does (p. 3286).
pub(super) fn copy_grade(
    project: &mut Project,
    from: &Target,
    to: &[Target],
) -> Result<Expansion, EditError> {
    if to.is_empty() {
        return Err(OpError::OutOfRange { what: "to" }.into());
    }
    let value = grade_at(project, from)?.clone();
    let mut steps = Steps::default();
    for target in to {
        let set = Op::SetGrade {
            grade: target.clone(),
            value: value.clone(),
        };
        steps.apply(project, set)?;
    }
    Ok(steps.finish())
}

/// Add the nodes of the grade `from` to the end of each grade of `to`, as Append Node Graph
/// does (p. 3290): what went into the output of the grade goes into what the source went into
/// in `from`, and what went into the output of `from` goes into the output. The nodes added are
/// given new ids, in their order.
pub(super) fn append_grade(
    project: &mut Project,
    from: &Target,
    to: &[Target],
) -> Result<Expansion, EditError> {
    if to.is_empty() {
        return Err(OpError::OutOfRange { what: "to" }.into());
    }
    let added = grade_at(project, from)?.clone();
    let mut steps = Steps::default();
    for target in to {
        let mut value = grade_at(project, target)?.clone();
        let held: HashSet<Id> = value.nodes.iter().map(|n| n.id.clone()).collect();
        let ids = reserve_node_ids(project, &held, added.nodes.len())?;
        let names: HashMap<&str, &Id> = added
            .nodes
            .iter()
            .map(|n| n.id.as_str())
            .zip(&ids)
            .collect();
        let last = value
            .link_into(OUTPUT, Port::Rgb, 0)
            .map(|l| l.from.clone());
        value.links.retain(|l| l.target() != (OUTPUT, Port::Rgb, 0));
        value.nodes.extend(added.nodes.iter().map(|n| Node {
            id: names[n.id.as_str()].clone(),
            ..n.clone()
        }));
        for link in &added.links {
            let from = if link.from == SOURCE {
                match &last {
                    Some(last) => last.clone(),
                    None => continue,
                }
            } else {
                names[link.from.as_str()].clone()
            };
            let to = if link.to == OUTPUT {
                link.to.clone()
            } else {
                names[link.to.as_str()].clone()
            };
            value.links.push(Link {
                from,
                to,
                ..link.clone()
            });
        }
        let set = Op::SetGrade {
            grade: target.clone(),
            value,
        };
        steps.apply(project, set)?;
    }
    Ok(steps.finish())
}

/// Set the setting at `param`, a path of fields written with dots, of `to_node` of the grade
/// `to` to what it is in `from_node` of the grade `from`, as Paste Value does (p. 3286).
pub(super) fn paste_value(
    project: &mut Project,
    (from, from_node): (&Target, &str),
    (to, to_node): (&Target, &str),
    param: &str,
) -> Result<Expansion, EditError> {
    let source = grade_at(project, from)?
        .node(from_node)
        .ok_or_else(|| missing(from_node))?;
    let target = grade_at(project, to)?
        .node(to_node)
        .ok_or_else(|| missing(to_node))?;
    let value = at_path(&source.kind.params(), param)
        .filter(|_| at_path(&target.kind.params(), param).is_some())
        .cloned()
        .ok_or(OpError::OutOfRange { what: "param" })?;
    let mut steps = Steps::default();
    steps.apply(project, set_params(to, to_node, patch_at(param, value)))?;
    Ok(steps.finish())
}

/// Set `to_node` of the grade `to` as `from_node` of the grade `from` is set, as pasting a node
/// does (p. 3320). The nodes are of one kind.
pub(super) fn paste_node(
    project: &mut Project,
    (from, from_node): (&Target, &str),
    (to, to_node): (&Target, &str),
) -> Result<Expansion, EditError> {
    let source = grade_at(project, from)?
        .node(from_node)
        .ok_or_else(|| missing(from_node))?;
    let target = grade_at(project, to)?
        .node(to_node)
        .ok_or_else(|| missing(to_node))?;
    if source.kind.kind() != target.kind.kind() {
        return Err(OpError::WrongNode {
            node: to_node.into(),
            kind: target.kind.kind().name(),
        }
        .into());
    }
    let params = source.kind.params();
    let mut steps = Steps::default();
    steps.apply(project, set_params(to, to_node, params))?;
    Ok(steps.finish())
}

fn add_node(grade: &Target, node: Node, links: Vec<Link>) -> Op {
    Op::AddNode {
        grade: grade.clone(),
        node,
        index: None,
        links,
    }
}

fn set_params(grade: &Target, node: &str, params: serde_json::Value) -> Op {
    Op::SetNode {
        grade: grade.clone(),
        id: node.into(),
        label: None,
        enabled: None,
        params: Some(params),
    }
}

fn missing(node: &str) -> OpError {
    OpError::Missing {
        what: "node",
        id: node.into(),
    }
}

/// Why a change cannot be made to or about `node`, which is of the wrong kind.
fn wrong(g: &Grade, node: &str) -> OpError {
    let kind = match g.node(node) {
        Some(n) => n.kind.kind().name(),
        None if node == SOURCE => SOURCE,
        None => OUTPUT,
    };
    OpError::WrongNode {
        node: node.into(),
        kind,
    }
}
