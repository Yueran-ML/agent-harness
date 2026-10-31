//! The edits of grades: adding nodes as the keys of the Color page add them (Reference Manual
//! 20, p. 3318), taking a node out of its tree, resetting one, copying grades, nodes and single
//! settings from one grade to others (pp. 3285–3297), and putting the grades of a clip's group
//! into its own (pp. 3306–3307).

use std::collections::{HashMap, HashSet};

use super::{EditError, Expansion, MOST_PRESERVED_NODES};
use crate::grade::{
    at_path, patch_at, Grade, Kind, Link, Node, NodeKind, Port, Target, OUTPUT, SOURCE,
};
use crate::keyframes::{aligned, Align, KeyBase};
use crate::ops::{apply_done, grade_at, group_of_clip, reserve_node_ids, Op, OpError};
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

/// Put the grade `from` in place of each grade of `to`, as Copy Grade does (p. 3286), keeping
/// the first `preserve` nodes of each that has as many (Copy Grade: Preserve number of nodes,
/// p. 3285), its keys put on the frames of each as `align` puts them (p. 3287).
pub(super) fn copy_grade(
    project: &mut Project,
    from: &Target,
    to: &[Target],
    preserve: u32,
    align: Align,
) -> Result<Expansion, EditError> {
    if to.is_empty() {
        return Err(OpError::OutOfRange { what: "to" }.into());
    }
    if preserve > MOST_PRESERVED_NODES {
        return Err(OpError::OutOfRange {
            what: "preserve_nodes",
        }
        .into());
    }
    let copied = grade_at(project, from)?.clone();
    let base = KeyBase::of(project, from);
    let mut steps = Steps::default();
    for target in to {
        let had = grade_at(project, target)?;
        let copied = aligned(&copied, base, KeyBase::of(project, target), align);
        let kept = preserve as usize;
        let value = if kept == 0 || had.nodes.len() < kept {
            copied.clone()
        } else {
            let kept = had.nodes[..kept].to_vec();
            preserved(project, &copied, kept)?
        };
        let set = Op::SetGrade {
            grade: target.clone(),
            value,
        };
        steps.apply(project, set)?;
    }
    Ok(steps.finish())
}

/// The grade `copied` with `kept`, the first nodes of the grade it is put in place of, in the
/// places of its first nodes: each of them of the kind of the node it takes the place of, with
/// as many inputs, so that the links of `copied` fit it. A node of `copied` after them whose id
/// one of `kept` holds is given a new one.
fn preserved(project: &mut Project, copied: &Grade, kept: Vec<Node>) -> Result<Grade, EditError> {
    if copied.nodes.len() < kept.len() {
        return Err(OpError::OutOfRange {
            what: "preserve_nodes",
        }
        .into());
    }
    let mut names: HashMap<Id, Id> = HashMap::new();
    for (node, place) in kept.iter().zip(&copied.nodes) {
        let fits = node.kind.kind() == place.kind.kind()
            && [Port::Rgb, Port::Key]
                .iter()
                .all(|port| node.kind.inputs(*port) == place.kind.inputs(*port));
        if !fits {
            return Err(OpError::WrongNode {
                node: node.id.clone(),
                kind: node.kind.kind().name(),
            }
            .into());
        }
        names.insert(place.id.clone(), node.id.clone());
    }
    let rest = &copied.nodes[kept.len()..];
    let held: HashSet<Id> = kept
        .iter()
        .chain(rest)
        .map(|node| node.id.clone())
        .collect();
    let taken: Vec<Id> = rest
        .iter()
        .filter(|node| kept.iter().any(|k| k.id == node.id))
        .map(|node| node.id.clone())
        .collect();
    let ids = reserve_node_ids(project, &held, taken.len())?;
    names.extend(taken.into_iter().zip(ids));
    let mut value = renamed(copied, &names);
    value.nodes.splice(..kept.len(), kept);
    Ok(value)
}

/// `grade` with the nodes that `names` names given the names it gives them, in its links too.
fn renamed(grade: &Grade, names: &HashMap<Id, Id>) -> Grade {
    let name = |id: &Id| names.get(id).unwrap_or(id).clone();
    let mut value = Grade {
        nodes: grade
            .nodes
            .iter()
            .map(|node| Node {
                id: name(&node.id),
                ..node.clone()
            })
            .collect(),
        links: grade
            .links
            .iter()
            .map(|link| Link {
                from: name(&link.from),
                to: name(&link.to),
                ..link.clone()
            })
            .collect(),
    };
    value.sort_links();
    value
}

/// `then` after `first`, as one grade: the nodes of both, in their order, what went into the
/// output of `first` going into what the source went into in `then`, and what went into the
/// output of `then` into the output. Their nodes are of other ids. Where nothing went into the
/// output of `first`, nothing goes into what the source went into in `then`.
fn chain(first: Grade, then: &Grade) -> Grade {
    let mut value = first;
    let last = value
        .link_into(OUTPUT, Port::Rgb, 0)
        .map(|l| l.from.clone());
    value.links.retain(|l| l.target() != (OUTPUT, Port::Rgb, 0));
    value.nodes.extend(then.nodes.iter().cloned());
    for link in &then.links {
        let from = if link.from == SOURCE {
            match &last {
                Some(last) => last.clone(),
                None => continue,
            }
        } else {
            link.from.clone()
        };
        value.links.push(Link {
            from,
            ..link.clone()
        });
    }
    value.sort_links();
    value
}

/// `grade` with new ids for its nodes, of the shape of those the engine makes, in their order,
/// for a grade whose nodes hold `held`.
fn anew(project: &mut Project, grade: &Grade, held: &HashSet<Id>) -> Result<Grade, EditError> {
    let ids = reserve_node_ids(project, held, grade.nodes.len())?;
    let names = grade.nodes.iter().map(|n| n.id.clone()).zip(ids).collect();
    Ok(renamed(grade, &names))
}

/// The ids of the nodes of `grade`.
fn ids_of(grade: &Grade) -> HashSet<Id> {
    grade.nodes.iter().map(|n| n.id.clone()).collect()
}

/// Add the nodes of the grade `from` to the end of each grade of `to`, as Append Node Graph
/// does (p. 3290): what went into the output of the grade goes into what the source went into
/// in `from`, and what went into the output of `from` goes into the output. The nodes added are
/// given new ids, in their order, and their keys are put on the frames of each as `align` puts
/// them (p. 3287).
pub(super) fn append_node_graph(
    project: &mut Project,
    from: &Target,
    to: &[Target],
    align: Align,
) -> Result<Expansion, EditError> {
    if to.is_empty() {
        return Err(OpError::OutOfRange { what: "to" }.into());
    }
    let added = grade_at(project, from)?.clone();
    let base = KeyBase::of(project, from);
    let mut steps = Steps::default();
    for target in to {
        let had = grade_at(project, target)?.clone();
        let added = aligned(&added, base, KeyBase::of(project, target), align);
        let added = anew(project, &added, &ids_of(&had))?;
        let set = Op::SetGrade {
            grade: target.clone(),
            value: chain(had, &added),
        };
        steps.apply(project, set)?;
    }
    Ok(steps.finish())
}

/// Put the nodes of the pre-clip grade of the group of each of `clips` before the nodes of its
/// grade, and those of the post-clip grade after them, and take it out of the group, as
/// Collapse Group Grades does (pp. 3306–3307). The group keeps its grades, and its other clips.
///
/// The picture goes through the nodes in the order that it went through the three grades
/// (p. 3330), and their keyframes are at the frames they were at, as the three count the frames
/// of the clip's source alike (see [`crate::keyframes`]). The nodes put in are given new ids, those of the pre-clip grade first;
/// the clip's own keep theirs. A grade of the group without nodes puts none in; one that is not
/// complete is put in as it is, which leaves the clip's grade not complete, as `DaVinci` copies
/// "all nodes".
pub(super) fn collapse_group_grades(
    project: &mut Project,
    timeline: &str,
    clips: &[Id],
) -> Result<Expansion, EditError> {
    if clips.is_empty() {
        return Err(OpError::OutOfRange { what: "clips" }.into());
    }
    let mut steps = Steps::default();
    for clip in clips {
        let group = group_of_clip(project, timeline, clip)?
            .ok_or_else(|| OpError::Ungrouped { clip: clip.clone() })?;
        let target = Target::Clip {
            timeline: timeline.into(),
            clip: clip.clone(),
        };
        let own = grade_at(project, &target)?.clone();
        let pre_clip = Target::PreClip {
            group: group.clone(),
        };
        let pre = grade_at(project, &pre_clip)?.clone();
        let post = grade_at(project, &Target::PostClip { group })?.clone();
        if !pre.nodes.is_empty() || !post.nodes.is_empty() {
            let mut held = ids_of(&own);
            let mut value = own;
            if !pre.nodes.is_empty() {
                let pre = anew(project, &pre, &held)?;
                held.extend(ids_of(&pre));
                value = chain(pre, &value);
            }
            if !post.nodes.is_empty() {
                let post = anew(project, &post, &held)?;
                value = chain(value, &post);
            }
            let set = Op::SetGrade {
                grade: target,
                value,
            };
            steps.apply(project, set)?;
        }
        let leave = Op::SetClipGroup {
            timeline: timeline.into(),
            clip: clip.clone(),
            group: None,
        };
        steps.apply(project, leave)?;
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
