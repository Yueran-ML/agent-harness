//! The keys of the settings of a clip, set by their paths: `transform.x`.
//!
//! The settings of a clip that take keys are those that the catalog lists
//! ([`crate::catalog::clip`]). They are set as the keys of a node of a grade are
//! ([`Op::SetKeys`], [`Op::SpliceKeys`]), but the clip itself holds them, and it is checked as
//! any change of a clip is.

use super::{check_clip, clip_at, missing, range, timeline_at, Op, OpError};
use crate::catalog;
use crate::grade::{Key, Param};
use crate::project::{Clip, Project};

/// The setting of a clip at a path that the catalog lists, or none.
fn setting<'c>(clip: &'c mut Clip, path: &str) -> Option<&'c mut Param> {
    catalog::of_clip(path)?;
    let (of, name) = path.split_once('.')?;
    match of {
        "transform" => clip.transform.setting_mut(name),
        _ => None,
    }
}

/// Put a setting in place of the one at `path` of a clip: `change` is given the setting as it
/// is and gives what it is to be. The clip is checked as any change of it is, and what the
/// setting was is given back.
fn with_setting(
    project: &mut Project,
    timeline_id: &str,
    id: &str,
    path: &str,
    change: impl FnOnce(&Param) -> Result<Param, OpError>,
) -> Result<Param, OpError> {
    let t = timeline_at(project, timeline_id)?;
    let timeline = &project.timelines[t];
    let (r, at) = clip_at(timeline, id)?;
    let track = &timeline.tracks[r];
    let mut new = track.clips[at].clone();
    let held = setting(&mut new, path).ok_or_else(|| missing("setting", path))?;
    let before = held.clone();
    *held = change(&before)?;
    check_clip(project, timeline.rate, track.kind, &new)?;
    project.timelines[t].tracks[r].clips[at] = new;
    Ok(before)
}

pub(super) fn set_clip_keys(
    project: &mut Project,
    timeline: &str,
    id: &str,
    path: String,
    param: Param,
) -> Result<Op, OpError> {
    let before = with_setting(project, timeline, id, &path, |_| Ok(param))?;
    Ok(Op::SetClipKeys {
        timeline: timeline.into(),
        id: id.into(),
        path,
        param: before,
    })
}

pub(super) fn splice_clip_keys(
    project: &mut Project,
    timeline: &str,
    id: &str,
    path: String,
    (at, remove): (usize, usize),
    keys: Vec<Key>,
) -> Result<Op, OpError> {
    let put = keys.len();
    let mut removed = Vec::new();
    let before = with_setting(project, timeline, id, &path, |before| {
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
            // A setting keeps one key at least: `set_clip_keys` puts a number there.
            return Err(range("keys"));
        }
        Ok(Param::Keys(all))
    })?;
    Ok(match before {
        // A setting that had no keys has its number back.
        Param::Fixed(_) => Op::SetClipKeys {
            timeline: timeline.into(),
            id: id.into(),
            path,
            param: before,
        },
        Param::Keys(_) => Op::SpliceClipKeys {
            timeline: timeline.into(),
            id: id.into(),
            path,
            at,
            remove: put,
            keys: removed,
        },
    })
}
