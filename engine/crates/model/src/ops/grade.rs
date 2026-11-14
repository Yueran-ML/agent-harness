//! The operations on grades and groups (see [`crate::grade`]).
//!
//! A node that is added with an empty id is given `n` and a number from `next_id`, passing over
//! the ids that nodes of its grade hold, and a group `k` and a number. A node that is added with
//! an id keeps it, unless a node of its grade holds it, it is `source` or `output`, or it is of
//! the shape of the ids that the engine makes with a number that it has not made yet. The nodes
//! of a grade that is put in whole, by `set_grade`, `add_group` or with a clip or a timeline,
//! have their ids, which are checked so.

use std::collections::HashSet;

use super::{clip_at, missing, place, range, taken, timeline_at, Counter, Op, OpError};
use crate::catalog;
use crate::grade::{
    changed, check_label, check_node, check_node_id, merge, Grade, Group, Key, Link, Node,
    NodeKind, Param, Port, Pos, Target,
};
use crate::project::{Id, Project};

/// The grade that `target` names.
pub(crate) fn grade_at<'a>(project: &'a Project, target: &Target) -> Result<&'a Grade, OpError> {
    match target {
        Target::Clip { timeline, clip } => {
            let t = &project.timelines[timeline_at(project, timeline)?];
            let (r, at) = clip_at(t, clip)?;
            Ok(&t.tracks[r].clips[at].grade)
        }
        Target::PreClip { group } => Ok(&group_of(project, group)?.pre),
        Target::PostClip { group } => Ok(&group_of(project, group)?.post),
        Target::Timeline { timeline } => {
            Ok(&project.timelines[timeline_at(project, timeline)?].grade)
        }
    }
}

/// The group that clip `clip` of timeline `timeline` is in, if it is in one.
pub(crate) fn group_of_clip(
    project: &Project,
    timeline: &str,
    clip: &str,
) -> Result<Option<Id>, OpError> {
    let t = &project.timelines[timeline_at(project, timeline)?];
    let (r, at) = clip_at(t, clip)?;
    Ok(t.tracks[r].clips[at].group.clone())
}

/// The grade that `target` names, to be changed.
fn grade_mut<'a>(project: &'a mut Project, target: &Target) -> Result<&'a mut Grade, OpError> {
    match target {
        Target::Clip { timeline, clip } => {
            let t = timeline_at(project, timeline)?;
            let (r, at) = clip_at(&project.timelines[t], clip)?;
            Ok(&mut project.timelines[t].tracks[r].clips[at].grade)
        }
        Target::PreClip { group } => Ok(&mut group_mut(project, group)?.pre),
        Target::PostClip { group } => Ok(&mut group_mut(project, group)?.post),
        Target::Timeline { timeline } => {
            let t = timeline_at(project, timeline)?;
            Ok(&mut project.timelines[t].grade)
        }
    }
}

fn group_of<'a>(project: &'a Project, id: &str) -> Result<&'a Group, OpError> {
    project
        .groups
        .iter()
        .find(|k| k.id == id)
        .ok_or_else(|| missing("group", id))
}

fn group_mut<'a>(project: &'a mut Project, id: &str) -> Result<&'a mut Group, OpError> {
    project
        .groups
        .iter_mut()
        .find(|k| k.id == id)
        .ok_or_else(|| missing("group", id))
}

/// Check the ids of the nodes of a grade that is put in whole: each has one, and one that the
/// engine could make is one that it has made.
pub(super) fn check_ids(counter: &Counter, grade: &Grade) -> Result<(), OpError> {
    for node in &grade.nodes {
        check_node_id(&node.id)?;
        counter.check(&node.id)?;
    }
    Ok(())
}

/// Check a group but for its id: its name and its grades, whose links are in order.
pub(super) fn check_group(group: &Group) -> Result<(), OpError> {
    check_label(&group.name)?;
    group.pre.check()?;
    group.post.check()
}

/// Whether a node of a grade that is put in whole may keep `id` in `project`: it is the id of
/// a node, and not one that the engine could make and has not made yet (as of a grade of
/// another project, a still of the `PowerGrades`).
pub(crate) fn keeps_node_id(project: &Project, id: &str) -> bool {
    check_node_id(id).is_ok() && Counter::of(project).check(id).is_ok()
}

/// Make `count` ids of new nodes for a grade whose nodes hold `held`, as an edit that puts
/// nodes into a grade whole needs them. `next_id` goes up past them.
pub(crate) fn reserve_node_ids(
    project: &mut Project,
    held: &HashSet<Id>,
    count: usize,
) -> Result<Vec<Id>, OpError> {
    let mut counter = Counter::of(project);
    let ids = (0..count)
        .map(|_| counter.make('n', |id| held.contains(id)))
        .collect::<Result<_, _>>()?;
    project.next_id = counter.next;
    Ok(ids)
}

/// Put the links that are told apart by their inputs in the order of their inputs.
fn sorted(mut links: Vec<Link>) -> Vec<Link> {
    links.sort_by(|a, b| a.target().cmp(&b.target()));
    links
}

pub(super) fn add_node(
    project: &mut Project,
    target: &Target,
    mut node: Node,
    index: Option<usize>,
    links: Vec<Link>,
) -> Result<Op, OpError> {
    let grade = grade_at(project, target)?;
    let at = place(index, grade.nodes.len())?;
    let given = node.id.clone();
    let mut counter = Counter::of(project);
    if node.id.is_empty() {
        node.id = counter.make('n', |id| grade.node(id).is_some())?;
    } else {
        check_node_id(&node.id)?;
        if grade.node(&node.id).is_some() {
            return Err(taken("node", &node.id));
        }
        counter.check(&node.id)?;
    }
    check_node(&node)?;
    let id = node.id.clone();
    let mut new = grade.clone();
    new.nodes.insert(at, node);
    let mut overwritten = vec![];
    let mut inputs = HashSet::new();
    for mut link in links {
        for end in [&mut link.from, &mut link.to] {
            if *end == given {
                end.clone_from(&id);
            }
        }
        let input = (link.to.clone(), link.kind, link.input);
        if !link.touches(&id) || !inputs.insert(input) {
            return Err(range("links"));
        }
        if let Some(had) = new.links.iter().position(|l| l.target() == link.target()) {
            overwritten.push(new.links.remove(had));
        }
        new.links.push(link);
    }
    new.sort_links();
    new.check_links()?;
    *grade_mut(project, target)? = new;
    project.next_id = counter.next;
    Ok(Op::RemoveNode {
        grade: target.clone(),
        id,
        links: Some(sorted(overwritten)),
    })
}

pub(super) fn remove_node(
    project: &mut Project,
    target: &Target,
    id: &str,
    links: Option<Vec<Link>>,
) -> Result<Op, OpError> {
    let grade = grade_at(project, target)?;
    let at = grade.position(id).ok_or_else(|| missing("node", id))?;
    let own: Vec<Link> = grade
        .links
        .iter()
        .filter(|l| l.touches(id))
        .cloned()
        .collect();
    let bridge = match links {
        None => grade.bridge(id),
        Some(links) => {
            let fed: HashSet<(&str, Port, u32)> = own
                .iter()
                .filter(|l| l.from == id)
                .map(Link::target)
                .collect();
            let mut inputs = HashSet::new();
            for link in &links {
                let fits = !link.touches(id) && fed.contains(&link.target());
                if !fits || !inputs.insert(link.target()) {
                    return Err(range("links"));
                }
            }
            links
        }
    };
    let mut new = grade.clone();
    let node = new.nodes.remove(at);
    new.links.retain(|l| !l.touches(id));
    new.links.extend(bridge);
    new.sort_links();
    new.check_links()?;
    *grade_mut(project, target)? = new;
    Ok(Op::AddNode {
        grade: target.clone(),
        node,
        index: Some(at),
        links: own,
    })
}

pub(super) fn connect(project: &mut Project, target: &Target, link: Link) -> Result<Op, OpError> {
    let grade = grade_at(project, target)?;
    let mut new = grade.clone();
    let old = new
        .links
        .iter()
        .position(|l| l.target() == link.target())
        .map(|at| new.links.remove(at));
    new.links.push(link.clone());
    new.sort_links();
    new.check_links()?;
    *grade_mut(project, target)? = new;
    Ok(match old {
        Some(old) => Op::Connect {
            grade: target.clone(),
            link: old,
        },
        None => Op::Disconnect {
            grade: target.clone(),
            to: link.to,
            kind: link.kind,
            input: link.input,
        },
    })
}

pub(super) fn disconnect(
    project: &mut Project,
    target: &Target,
    to: &str,
    kind: Port,
    input: u32,
) -> Result<Op, OpError> {
    let grade = grade_mut(project, target)?;
    let at = grade
        .links
        .iter()
        .position(|l| l.target() == (to, kind, input))
        .ok_or_else(|| missing("link", to))?;
    let link = grade.links.remove(at);
    Ok(Op::Connect {
        grade: target.clone(),
        link,
    })
}

/// What `set_node` changes of a node: each field that is given.
pub(crate) struct NodeChange<'a> {
    pub(crate) label: Option<String>,
    pub(crate) enabled: Option<bool>,
    pub(crate) locked: Option<bool>,
    #[allow(clippy::option_option, reason = "left out and null are told apart")]
    pub(crate) pos: Option<Option<Pos>>,
    pub(crate) params: Option<&'a serde_json::Value>,
}

/// Set what is given of a node. Its `params` are not set where it is locked both before and
/// after the change: a node is unlocked and set in one change, or set and locked, and so the
/// inverse of a change is never refused for the lock.
pub(super) fn set_node(
    project: &mut Project,
    target: &Target,
    id: &str,
    change: NodeChange<'_>,
) -> Result<Op, OpError> {
    let NodeChange {
        label,
        enabled,
        locked,
        pos,
        params,
    } = change;
    let grade = grade_at(project, target)?;
    let at = grade.position(id).ok_or_else(|| missing("node", id))?;
    let old = &grade.nodes[at];
    let mut node = old.clone();
    if let Some(label) = &label {
        check_label(label)?;
        node.label.clone_from(label);
    }
    if let Some(enabled) = enabled {
        node.enabled = enabled;
    }
    if let Some(locked) = locked {
        node.locked = locked;
    }
    if let Some(pos) = pos {
        if let Some(pos) = pos {
            pos.check()?;
        }
        node.pos = pos;
    }
    let mut back = None;
    if let Some(patch) = params {
        if old.locked && node.locked {
            return Err(OpError::NodeLocked { node: id.into() });
        }
        let before = old.kind.params();
        let mut value = before.clone();
        merge(&mut value, patch);
        node.kind =
            NodeKind::from_params(old.kind.kind(), value).map_err(|error| OpError::Params {
                node: id.into(),
                why: error.to_string(),
            })?;
        node.kind.check()?;
        back = Some(changed(&before, patch));
    }
    let mut new = grade.clone();
    new.nodes[at] = node;
    // A mixer with fewer inputs has none for the links into those that it had.
    new.check_links()?;
    let inverse = Op::SetNode {
        grade: target.clone(),
        id: id.into(),
        label: label.map(|_| old.label.clone()),
        enabled: enabled.map(|_| old.enabled),
        locked: locked.map(|_| old.locked),
        pos: pos.map(|_| old.pos),
        params: back,
    };
    *grade_mut(project, target)? = new;
    Ok(inverse)
}

/// The value at a path of dotted names in the `params` of a node: each name a field of an
/// object, or a place in a list.
fn slot<'v>(params: &'v mut serde_json::Value, path: &str) -> Option<&'v mut serde_json::Value> {
    let mut at = params;
    for part in path.split('.') {
        at = match at {
            serde_json::Value::Object(fields) => fields.get_mut(part)?,
            serde_json::Value::Array(items) => items.get_mut(part.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(at)
}

/// Put a setting in place of the one at `path` of a node: `change` is given the setting as
/// it is and gives what it is to be. The node is checked as any change of it is, and what
/// the setting was is given back.
fn with_keys(
    project: &mut Project,
    target: &Target,
    id: &str,
    path: &str,
    change: impl FnOnce(&Param) -> Result<Param, OpError>,
) -> Result<Param, OpError> {
    let refused = |error: serde_json::Error| OpError::Params {
        node: id.into(),
        why: error.to_string(),
    };
    let grade = grade_at(project, target)?;
    let at = grade.position(id).ok_or_else(|| missing("node", id))?;
    let old = &grade.nodes[at];
    if old.locked {
        return Err(OpError::NodeLocked { node: id.into() });
    }
    let kind = old.kind.kind();
    // A path that is not listed may lead to a number all the same, one that takes no keys.
    if catalog::of_node(kind, path).is_none() {
        return Err(missing("setting", path));
    }
    let mut params = old.kind.params();
    // Nor is there a setting of a palette that the node has not, or of a window beyond its last.
    let held = slot(&mut params, path).ok_or_else(|| missing("setting", path))?;
    let before: Param = serde_json::from_value(held.clone()).map_err(refused)?;
    let after = change(&before)?;
    *held = serde_json::to_value(&after).map_err(refused)?;
    let new = NodeKind::from_params(kind, params).map_err(refused)?;
    new.check()?;
    grade_mut(project, target)?.nodes[at].kind = new;
    Ok(before)
}

pub(super) fn set_keys(
    project: &mut Project,
    target: &Target,
    id: &str,
    path: String,
    param: Param,
) -> Result<Op, OpError> {
    let before = with_keys(project, target, id, &path, |_| Ok(param))?;
    Ok(Op::SetKeys {
        grade: target.clone(),
        id: id.into(),
        path,
        param: before,
    })
}

pub(super) fn splice_keys(
    project: &mut Project,
    target: &Target,
    id: &str,
    path: String,
    (at, remove): (usize, usize),
    keys: Vec<Key>,
) -> Result<Op, OpError> {
    let put = keys.len();
    let mut removed = Vec::new();
    let before = with_keys(project, target, id, &path, |before| {
        let mut all = match before {
            Param::Fixed(_) => Vec::new(),
            Param::Keys(keys) => keys.clone(),
        };
        let len = all.len();
        let Some(end) = at.checked_add(remove).filter(|&end| end <= len) else {
            return Err(OpError::OutOfList {
                at: at.saturating_add(remove),
                len,
            });
        };
        removed = all.splice(at..end, keys).collect();
        if all.is_empty() {
            return Err(OpError::Params {
                node: id.into(),
                why: format!("{path} would have no key left: set_keys puts a number there"),
            });
        }
        Ok(Param::Keys(all))
    })?;
    Ok(match before {
        // A setting that had no keys has its number back.
        Param::Fixed(_) => Op::SetKeys {
            grade: target.clone(),
            id: id.into(),
            path,
            param: before,
        },
        Param::Keys(_) => Op::SpliceKeys {
            grade: target.clone(),
            id: id.into(),
            path,
            at,
            remove: put,
            keys: removed,
        },
    })
}

pub(super) fn set_grade(
    project: &mut Project,
    target: &Target,
    mut value: Grade,
) -> Result<Op, OpError> {
    grade_at(project, target)?;
    value.sort_links();
    check_ids(&Counter::of(project), &value)?;
    value.check()?;
    let old = std::mem::replace(grade_mut(project, target)?, value);
    Ok(Op::SetGrade {
        grade: target.clone(),
        value: old,
    })
}

pub(super) fn add_group(
    project: &mut Project,
    mut group: Group,
    index: Option<usize>,
) -> Result<Op, OpError> {
    let at = place(index, project.groups.len())?;
    let mut counter = Counter::of(project);
    group.pre.sort_links();
    group.post.sort_links();
    check_ids(&counter, &group.pre)?;
    check_ids(&counter, &group.post)?;
    check_group(&group)?;
    let held = |id: &str| project.groups.iter().any(|k| k.id == id);
    if group.id.is_empty() {
        group.id = counter.make('k', held)?;
    } else if held(&group.id) {
        return Err(taken("group", &group.id));
    } else {
        counter.check(&group.id)?;
    }
    let id = group.id.clone();
    project.groups.insert(at, group);
    project.next_id = counter.next;
    Ok(Op::RemoveGroup { id })
}

pub(super) fn remove_group(project: &mut Project, id: &str) -> Result<Op, OpError> {
    let at = project
        .groups
        .iter()
        .position(|k| k.id == id)
        .ok_or_else(|| missing("group", id))?;
    let mut clips = project
        .timelines
        .iter()
        .flat_map(|t| &t.tracks)
        .flat_map(|r| &r.clips);
    if let Some(clip) = clips.find(|c| c.group.as_deref() == Some(id)) {
        return Err(OpError::Grouped {
            group: id.into(),
            clip: clip.id.clone(),
        });
    }
    let group = project.groups.remove(at);
    Ok(Op::AddGroup {
        group,
        index: Some(at),
    })
}

pub(super) fn set_group(
    project: &mut Project,
    id: &str,
    name: Option<String>,
) -> Result<Op, OpError> {
    if let Some(name) = &name {
        check_label(name)?;
    }
    let group = group_mut(project, id)?;
    let old = name.map(|name| std::mem::replace(&mut group.name, name));
    Ok(Op::SetGroup {
        id: id.into(),
        name: old,
    })
}

pub(super) fn set_clip_group(
    project: &mut Project,
    timeline_id: &str,
    clip_id: &str,
    group: Option<Id>,
) -> Result<Op, OpError> {
    let t = timeline_at(project, timeline_id)?;
    let (r, at) = clip_at(&project.timelines[t], clip_id)?;
    if let Some(group) = &group {
        group_of(project, group)?;
    }
    let clip = &mut project.timelines[t].tracks[r].clips[at];
    let old = std::mem::replace(&mut clip.group, group);
    Ok(Op::SetClipGroup {
        timeline: timeline_id.into(),
        clip: clip_id.into(),
        group: old,
    })
}

/// An operation on a grade as it was done, read back from the project that it was applied to
/// and the inverse that it gave: a node with the id that the engine gave it, the place that it
/// was put at, and its links with that id; the links that took the place of a node; the
/// settings of a node as they were set; and a grade with its links in order.
///
/// # Panics
/// If the grade or the node that was changed is not in the project, which would be a bug.
pub(super) fn done(project: &Project, op: Op, inverse: &Op) -> Op {
    const THERE: &str = "a grade that was changed is in the project";
    match (op, inverse) {
        (
            Op::AddNode {
                grade, node, links, ..
            },
            Op::RemoveNode { id, .. },
        ) => {
            let g = grade_at(project, &grade).expect(THERE);
            let at = g.position(id).expect(THERE);
            let links = links
                .into_iter()
                .map(|mut link| {
                    for end in [&mut link.from, &mut link.to] {
                        if *end == node.id {
                            end.clone_from(id);
                        }
                    }
                    link
                })
                .collect();
            Op::AddNode {
                grade,
                node: g.nodes[at].clone(),
                index: Some(at),
                links: sorted(links),
            }
        }
        (Op::RemoveNode { grade, id, .. }, Op::AddNode { links: own, .. }) => {
            let g = grade_at(project, &grade).expect(THERE);
            let bridge = own
                .iter()
                .filter(|l| l.from == id)
                .filter_map(|l| g.link_into(&l.to, l.kind, l.input).cloned())
                .collect();
            Op::RemoveNode {
                grade,
                id,
                links: Some(sorted(bridge)),
            }
        }
        (
            Op::SetNode {
                grade,
                id,
                label,
                enabled,
                locked,
                pos,
                params,
            },
            _,
        ) => {
            let g = grade_at(project, &grade).expect(THERE);
            let node = g.node(&id).expect(THERE);
            let params = params.map(|patch| changed(&node.kind.params(), &patch));
            Op::SetNode {
                grade,
                id,
                label,
                enabled,
                locked,
                pos,
                params,
            }
        }
        (Op::SetGrade { grade, .. }, _) => {
            let value = grade_at(project, &grade).expect(THERE).clone();
            Op::SetGrade { grade, value }
        }
        (op, _) => op,
    }
}
