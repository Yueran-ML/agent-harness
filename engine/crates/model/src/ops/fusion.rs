//! The operations on the composition of a clip (see [`crate::fusion`]), each as the operation
//! of a grade that it is like ([`super::grade`]).
//!
//! A node that is added with an empty id is given `n` and a number from `next_id`, passing
//! over the ids that nodes of its composition hold. A node that is added with an id keeps it,
//! unless a node of its composition holds it, it has a dot in it, or it is of the shape of the
//! ids that the engine makes with a number that it has not made yet. The nodes of a
//! composition that is put in whole, by `set_fusion` or with a clip, have their ids, which are
//! checked so.
//!
//! Every change is checked on the composition as it would be, whole
//! ([`Composition::check`]): so a change that would leave it with no `media_out`, or with
//! two, is refused, as one that would make a loop is; and so is one that would have a centre
//! follow a node that is not there or is no Tracker. A Tracker that a centre follows is not
//! taken away (`in_use`): so every change is undone by its inverse, which puts back a
//! composition that was checked.

use std::collections::HashSet;

use super::{clip_at, missing, place, range, taken, timeline_at, Counter, Op, OpError};
use crate::catalog;
use crate::fusion::{check_node, check_node_id, Composition, Input, Link, Node, NodeKind};
use crate::grade::{changed, check_label, merge, Key, Param, Pos};
use crate::project::{Clip, Project, TrackKind};

/// Where clip `clip` of timeline `timeline` is: its timeline, its track and its place there.
fn clip_place(
    project: &Project,
    timeline: &str,
    clip: &str,
) -> Result<(usize, usize, usize), OpError> {
    let t = timeline_at(project, timeline)?;
    let (r, at) = clip_at(&project.timelines[t], clip)?;
    Ok((t, r, at))
}

/// The clip at a place that [`clip_place`] gave.
fn clip_mut(project: &mut Project, (t, r, at): (usize, usize, usize)) -> &mut Clip {
    &mut project.timelines[t].tracks[r].clips[at]
}

/// The composition of clip `clip` of timeline `timeline`, with where the clip is; a clip
/// that has none is refused.
fn composition_at<'a>(
    project: &'a Project,
    timeline: &str,
    clip: &str,
) -> Result<(&'a Composition, (usize, usize, usize)), OpError> {
    let (t, r, at) = clip_place(project, timeline, clip)?;
    let composition = project.timelines[t].tracks[r].clips[at]
        .fusion
        .as_ref()
        .ok_or_else(|| missing("fusion", clip))?;
    Ok((composition, (t, r, at)))
}

/// Check the ids of the nodes of a composition that is put in whole: each has one, and one
/// that the engine could make is one that it has made.
pub(super) fn check_ids(counter: &Counter, composition: &Composition) -> Result<(), OpError> {
    for node in &composition.nodes {
        check_node_id(&node.id)?;
        counter.check(&node.id)?;
    }
    Ok(())
}

/// Check the composition of a clip as it would be on a track of `kind`, where it has one: a
/// composition makes a picture, so a clip of sound has none.
pub(super) fn check_of(kind: TrackKind, clip: &Clip) -> Result<(), OpError> {
    let Some(composition) = &clip.fusion else {
        return Ok(());
    };
    if kind != TrackKind::Video {
        return Err(OpError::WrongTrack {
            what: "fusion",
            kind,
        });
    }
    composition.check()
}

/// Put the links that are told apart by their inputs in the order of their inputs.
fn sorted(links: Vec<Link>) -> Vec<Link> {
    let mut ordered = Composition {
        nodes: vec![],
        links,
    };
    ordered.sort_links();
    ordered.links
}

pub(super) fn set_fusion(
    project: &mut Project,
    timeline: &str,
    clip: &str,
    mut value: Option<Composition>,
) -> Result<Op, OpError> {
    let at = clip_place(project, timeline, clip)?;
    if let Some(composition) = &mut value {
        let kind = project.timelines[at.0].tracks[at.1].kind;
        // A composition makes a picture: a clip of sound takes none.
        if kind != TrackKind::Video {
            return Err(OpError::WrongTrack {
                what: "fusion",
                kind,
            });
        }
        composition.sort_links();
        check_ids(&Counter::of(project), composition)?;
        composition.check()?;
    }
    let old = std::mem::replace(&mut clip_mut(project, at).fusion, value);
    Ok(Op::SetFusion {
        timeline: timeline.into(),
        clip: clip.into(),
        value: old,
    })
}

pub(super) fn add_node(
    project: &mut Project,
    timeline: &str,
    clip: &str,
    mut node: Node,
    index: Option<usize>,
    links: Vec<Link>,
) -> Result<Op, OpError> {
    let (composition, spot) = composition_at(project, timeline, clip)?;
    let at = place(index, composition.nodes.len())?;
    let given = node.id.clone();
    let mut counter = Counter::of(project);
    if node.id.is_empty() {
        node.id = counter.make('n', |id| composition.node(id).is_some())?;
    } else {
        check_node_id(&node.id)?;
        if composition.node(&node.id).is_some() {
            return Err(taken("node", &node.id));
        }
        counter.check(&node.id)?;
    }
    check_node(&node)?;
    let id = node.id.clone();
    let mut new = composition.clone();
    new.nodes.insert(at, node);
    let mut overwritten = vec![];
    let mut inputs = HashSet::new();
    for mut link in links {
        for end in [&mut link.from, &mut link.to] {
            if *end == given {
                end.clone_from(&id);
            }
        }
        let input = (link.to.clone(), link.input);
        if !link.touches(&id) || !inputs.insert(input) {
            return Err(range("links"));
        }
        if let Some(had) = new.links.iter().position(|l| l.target() == link.target()) {
            overwritten.push(new.links.remove(had));
        }
        new.links.push(link);
    }
    new.sort_links();
    new.check()?;
    clip_mut(project, spot).fusion = Some(new);
    project.next_id = counter.next;
    Ok(Op::RemoveFusionNode {
        timeline: timeline.into(),
        clip: clip.into(),
        id,
        links: Some(sorted(overwritten)),
    })
}

pub(super) fn remove_node(
    project: &mut Project,
    timeline: &str,
    clip: &str,
    id: &str,
    links: Option<Vec<Link>>,
) -> Result<Op, OpError> {
    let (composition, spot) = composition_at(project, timeline, clip)?;
    let at = composition
        .position(id)
        .ok_or_else(|| missing("node", id))?;
    // A Tracker that the centre of another node follows stays: the centre lets go of it
    // first.
    let follows = |node: &&Node| node.kind.follow().is_some_and(|follow| follow.node == id);
    if let Some(by) = composition.nodes.iter().find(follows) {
        return Err(OpError::InUse {
            what: "node",
            id: id.into(),
            by: by.id.clone(),
        });
    }
    let own: Vec<Link> = composition
        .links
        .iter()
        .filter(|l| l.touches(id))
        .cloned()
        .collect();
    let put = links.unwrap_or_default();
    {
        // Each link that is put goes into an input that the node went into, and none
        // touches the node.
        let fed: HashSet<(&str, Input)> = own
            .iter()
            .filter(|l| l.from == id)
            .map(Link::target)
            .collect();
        let mut inputs = HashSet::new();
        for link in &put {
            let fits = !link.touches(id) && fed.contains(&link.target());
            if !fits || !inputs.insert(link.target()) {
                return Err(range("links"));
            }
        }
    }
    let mut new = composition.clone();
    let node = new.nodes.remove(at);
    new.links.retain(|l| !l.touches(id));
    new.links.extend(put);
    new.sort_links();
    new.check()?;
    clip_mut(project, spot).fusion = Some(new);
    Ok(Op::AddFusionNode {
        timeline: timeline.into(),
        clip: clip.into(),
        node,
        index: Some(at),
        links: own,
    })
}

pub(super) fn connect(
    project: &mut Project,
    timeline: &str,
    clip: &str,
    link: Link,
) -> Result<Op, OpError> {
    let (composition, spot) = composition_at(project, timeline, clip)?;
    let mut new = composition.clone();
    let old = new
        .links
        .iter()
        .position(|l| l.target() == link.target())
        .map(|at| new.links.remove(at));
    new.links.push(link.clone());
    new.sort_links();
    new.check()?;
    clip_mut(project, spot).fusion = Some(new);
    Ok(match old {
        Some(old) => Op::ConnectFusion {
            timeline: timeline.into(),
            clip: clip.into(),
            link: old,
        },
        None => Op::DisconnectFusion {
            timeline: timeline.into(),
            clip: clip.into(),
            to: link.to,
            input: link.input,
        },
    })
}

pub(super) fn disconnect(
    project: &mut Project,
    timeline: &str,
    clip: &str,
    to: &str,
    input: Input,
) -> Result<Op, OpError> {
    let (composition, spot) = composition_at(project, timeline, clip)?;
    let at = composition
        .links
        .iter()
        .position(|l| l.target() == (to, input))
        .ok_or_else(|| missing("link", to))?;
    let mut new = composition.clone();
    let link = new.links.remove(at);
    clip_mut(project, spot).fusion = Some(new);
    Ok(Op::ConnectFusion {
        timeline: timeline.into(),
        clip: clip.into(),
        link,
    })
}

/// What `set_fusion_node` changes of a node: each field that is given.
pub(crate) struct NodeChange<'a> {
    pub(crate) label: Option<String>,
    pub(crate) pass_through: Option<bool>,
    #[allow(clippy::option_option, reason = "left out and null are told apart")]
    pub(crate) pos: Option<Option<Pos>>,
    pub(crate) params: Option<&'a serde_json::Value>,
}

pub(super) fn set_node(
    project: &mut Project,
    timeline: &str,
    clip: &str,
    id: &str,
    change: NodeChange<'_>,
) -> Result<Op, OpError> {
    let NodeChange {
        label,
        pass_through,
        pos,
        params,
    } = change;
    let (composition, spot) = composition_at(project, timeline, clip)?;
    let at = composition
        .position(id)
        .ok_or_else(|| missing("node", id))?;
    let old = &composition.nodes[at];
    let mut node = old.clone();
    if let Some(label) = &label {
        check_label(label)?;
        node.label.clone_from(label);
    }
    if let Some(pass_through) = pass_through {
        node.pass_through = pass_through;
    }
    if let Some(pos) = pos {
        if let Some(pos) = pos {
            pos.check()?;
        }
        node.pos = pos;
    }
    let mut back = None;
    if let Some(patch) = params {
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
    let inverse = Op::SetFusionNode {
        timeline: timeline.into(),
        clip: clip.into(),
        id: id.into(),
        label: label.map(|_| old.label.clone()),
        pass_through: pass_through.map(|_| old.pass_through),
        pos: pos.map(|_| old.pos),
        params: back,
    };
    let mut new = composition.clone();
    new.nodes[at] = node;
    // What its centre is set to follow is a Tracker of the composition.
    if params.is_some() {
        new.check_follows()?;
    }
    clip_mut(project, spot).fusion = Some(new);
    Ok(inverse)
}

/// Put a setting in place of the one at `path` of the composition of a clip, the id of a
/// node, a dot, and the path of the setting in the `params` of the node: `change` is given
/// the setting as it is and gives what it is to be. The node is checked as any change of it
/// is, and what the setting was is given back.
fn with_keys(
    project: &mut Project,
    timeline: &str,
    clip: &str,
    path: &str,
    change: impl FnOnce(&str, &Param) -> Result<Param, OpError>,
) -> Result<Param, OpError> {
    let (composition, spot) = composition_at(project, timeline, clip)?;
    // The id of a node has no dot in it: the path of its setting begins after the first.
    let (id, setting) = path
        .split_once('.')
        .ok_or_else(|| missing("setting", path))?;
    let at = composition
        .position(id)
        .ok_or_else(|| missing("node", id))?;
    let mut kind = composition.nodes[at].kind.clone();
    if catalog::of_fusion(kind.kind(), setting).is_none() {
        return Err(missing("setting", path));
    }
    let held = kind
        .setting_mut(setting)
        .ok_or_else(|| missing("setting", path))?;
    let before = held.clone();
    *held = change(id, &before)?;
    kind.check()?;
    let mut new = composition.clone();
    new.nodes[at].kind = kind;
    clip_mut(project, spot).fusion = Some(new);
    Ok(before)
}

pub(super) fn set_keys(
    project: &mut Project,
    timeline: &str,
    clip: &str,
    path: String,
    param: Param,
) -> Result<Op, OpError> {
    let before = with_keys(project, timeline, clip, &path, |_, _| Ok(param))?;
    Ok(Op::SetFusionKeys {
        timeline: timeline.into(),
        clip: clip.into(),
        path,
        param: before,
    })
}

pub(super) fn splice_keys(
    project: &mut Project,
    timeline: &str,
    clip: &str,
    path: String,
    (at, remove): (usize, usize),
    keys: Vec<Key>,
) -> Result<Op, OpError> {
    let put = keys.len();
    let mut removed = Vec::new();
    let before = with_keys(project, timeline, clip, &path, |node, before| {
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
                node: node.into(),
                why: format!("{path} would have no key left: set_fusion_keys puts a number there"),
            });
        }
        Ok(Param::Keys(all))
    })?;
    Ok(match before {
        // A setting that had no keys has its number back.
        Param::Fixed(_) => Op::SetFusionKeys {
            timeline: timeline.into(),
            clip: clip.into(),
            path,
            param: before,
        },
        Param::Keys(_) => Op::SpliceFusionKeys {
            timeline: timeline.into(),
            clip: clip.into(),
            path,
            at,
            remove: put,
            keys: removed,
        },
    })
}

/// The composition of clip `clip` of timeline `timeline`, if the clip is there and has one.
fn read<'a>(project: &'a Project, timeline: &str, clip: &str) -> Option<&'a Composition> {
    let (t, r, at) = clip_place(project, timeline, clip).ok()?;
    project.timelines[t].tracks[r].clips[at].fusion.as_ref()
}

/// An operation on the composition of a clip as it was done, read back from the project that
/// it was applied to and the inverse that it gave: a node with the id that the engine gave
/// it, the place that it was put at, and its links with that id; the links that were put in
/// the place of a node; the settings of a node as they were set; and a composition with its
/// links in order.
///
/// # Panics
/// If the composition or the node that was changed is not in the project, which would be a
/// bug.
/// What a patch of the params of a node did, as they are written after it: each field that it
/// names, as it is now; and whole, as the engine filled it in, each field that it brought where
/// there was none (`back`, the patch that takes it back, has `null` there). A part of a Text+
/// added as `{}` is so written with its defaults, which is what taking its inverse back gives.
fn written(
    after: &serde_json::Value,
    patch: &serde_json::Value,
    back: Option<&serde_json::Value>,
) -> serde_json::Value {
    match (after, patch) {
        (serde_json::Value::Object(after), serde_json::Value::Object(patch)) => patch
            .iter()
            .map(|(name, value)| {
                let was_back = back.and_then(|back| back.get(name));
                let now = match after.get(name) {
                    None => serde_json::Value::Null,
                    Some(field) if was_back.is_some_and(serde_json::Value::is_null) => {
                        field.clone()
                    }
                    Some(field) => written(field, value, was_back),
                };
                (name.clone(), now)
            })
            .collect::<serde_json::Map<_, _>>()
            .into(),
        (after, _) => after.clone(),
    }
}

pub(super) fn done(project: &Project, op: Op, inverse: &Op) -> Op {
    const THERE: &str = "a composition that was changed is in the project";
    match (op, inverse) {
        (
            Op::AddFusionNode {
                timeline,
                clip,
                node,
                links,
                ..
            },
            Op::RemoveFusionNode { id, .. },
        ) => {
            let composition = read(project, &timeline, &clip).expect(THERE);
            let at = composition.position(id).expect(THERE);
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
            Op::AddFusionNode {
                node: composition.nodes[at].clone(),
                index: Some(at),
                links: sorted(links),
                timeline,
                clip,
            }
        }
        (
            Op::RemoveFusionNode {
                timeline, clip, id, ..
            },
            Op::AddFusionNode { links: own, .. },
        ) => {
            let composition = read(project, &timeline, &clip).expect(THERE);
            let put = own
                .iter()
                .filter(|l| l.from == id)
                .filter_map(|l| composition.link_into(&l.to, l.input).cloned())
                .collect();
            Op::RemoveFusionNode {
                links: Some(sorted(put)),
                timeline,
                clip,
                id,
            }
        }
        (
            Op::SetFusionNode {
                timeline,
                clip,
                id,
                label,
                pass_through,
                pos,
                params,
            },
            inverse,
        ) => {
            let composition = read(project, &timeline, &clip).expect(THERE);
            let node = composition.node(&id).expect(THERE);
            let back = match inverse {
                Op::SetFusionNode { params, .. } => params.as_ref(),
                _ => None,
            };
            let params = params.map(|patch| written(&node.kind.params(), &patch, back));
            Op::SetFusionNode {
                timeline,
                clip,
                id,
                label,
                pass_through,
                pos,
                params,
            }
        }
        (Op::SetFusion { timeline, clip, .. }, _) => {
            let value = read(project, &timeline, &clip).cloned();
            Op::SetFusion {
                timeline,
                clip,
                value,
            }
        }
        (op, _) => op,
    }
}
