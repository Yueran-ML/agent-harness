//! Clip clipboard snapshots: capture without changing the project, paste through ordinary ops.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::follow::{Follow, Piece};
use super::work::{halves_of, missing, union, Work};
use super::EditError;
use crate::audio::GroupControl;
use crate::ops::{within, Op, OpError};
use crate::project::{Clip, Id, Timeline, TrackKind, Transition};
use crate::time::Rate;

/// A project-local snapshot, independent of the lifetime of the clips that were copied.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipClipboard {
    /// The source timeline's frame rate.
    pub rate: Rate,
    /// The source playhead, retained as the alignment reference.
    pub at: i64,
    /// Selected spans, in source track and frame order.
    pub clips: Vec<CopiedClip>,
    /// Transitions whose two sides are present in the captured spans.
    #[serde(default)]
    pub transitions: Vec<CopiedTransition>,
}

/// A transition and the indices of its two clips in the snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CopiedTransition {
    /// Index of the clip ending at the cut.
    pub left: usize,
    /// Index of the clip starting at the cut.
    pub right: usize,
    /// The original transition settings; ids are replaced when pasting.
    pub transition: Transition,
}

/// One captured clip or slice, with the automation over its original timeline span.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CopiedClip {
    /// The source track id, used by the destination mapping.
    pub track: Id,
    /// Its zero-based position among tracks of the same kind, for clipboard ghost routing.
    pub index: usize,
    /// Picture or sound; subtitle editing has its own clipboard.
    pub kind: TrackKind,
    /// All clip settings, including the source mapping after slicing.
    pub clip: Clip,
    /// The lanes that follow the captured slice, independent of later source edits.
    pub lanes: Vec<Piece>,
}

pub(super) fn editing_peers(timeline: &Timeline, track: &str) -> Vec<Id> {
    if timeline.audio.groups_bypassed {
        return vec![];
    }
    let groups = timeline.track_groups.iter().filter(|g| {
        g.active && g.linked.contains(&GroupControl::Editing) && g.contains(timeline, track)
    });
    let mut along = Vec::new();
    for group in groups {
        for member in timeline
            .tracks
            .iter()
            .filter(|row| !row.locked && group.contains(timeline, &row.id))
        {
            if member.id != track && !along.contains(&member.id) {
                along.push(member.id.clone());
            }
        }
    }
    along
}

fn copy_transitions(timeline: &Timeline, clips: &[CopiedClip]) -> Vec<CopiedTransition> {
    let mut result = Vec::new();
    for track in &timeline.tracks {
        for transition in &track.transitions {
            for (left, l) in clips
                .iter()
                .enumerate()
                .filter(|(_, c)| c.track == track.id && c.clip.id == transition.left)
            {
                if let Some((right, _)) = clips.iter().enumerate().find(|(_, r)| {
                    r.track == track.id
                        && r.clip.id == transition.right
                        && r.clip.start == l.clip.end()
                }) {
                    result.push(CopiedTransition {
                        left,
                        right,
                        transition: transition.clone(),
                    });
                }
            }
        }
    }
    result
}

/// Capture whole selected clips, or their intersections with a range. Linked selection and
/// direct Editing-group peers use the same source spans; source clips and lanes stay intact.
///
/// # Errors
/// Missing clips, invalid frame bounds, or a selection with no frames are refused.
pub fn copy_clips(
    timeline: &Timeline,
    clips: &[Id],
    at: i64,
    range: Option<(i64, i64)>,
    linked: bool,
    grouped: bool,
) -> Result<ClipClipboard, EditError> {
    within("at", i128::from(at))?;
    if at < 0 {
        return Err(OpError::OutOfTimeline { start: at }.into());
    }
    if let Some((from, to)) = range {
        within("from", i128::from(from))?;
        within("to", i128::from(to))?;
        if from < 0 || from >= to {
            return Err(EditError::EmptyRange);
        }
    }
    let located: Vec<_> = timeline
        .tracks
        .iter()
        .flat_map(|track| track.clips.iter().map(move |clip| (track, clip)))
        .collect();
    let mut selected = HashSet::new();
    let mut links = HashSet::new();
    for id in clips {
        let (_, clip) = located
            .iter()
            .find(|(_, clip)| clip.id == *id)
            .ok_or_else(|| missing("clip", id))?;
        selected.insert(id.clone());
        if linked {
            links.extend(clip.link.iter().cloned());
        }
    }
    let mut ranges: BTreeMap<Id, Vec<(i64, i64)>> = BTreeMap::new();
    for (track, clip) in &located {
        let along =
            linked && !track.locked && clip.link.as_ref().is_some_and(|id| links.contains(id));
        if !selected.contains(&clip.id) && !along {
            continue;
        }
        let (from, to) = range.map_or((clip.start, clip.end()), |(a, b)| {
            (a.max(clip.start), b.min(clip.end()))
        });
        if from >= to {
            continue;
        }
        ranges.entry(track.id.clone()).or_default().push((from, to));
        if grouped {
            for peer in editing_peers(timeline, &track.id) {
                ranges.entry(peer).or_default().push((from, to));
            }
        }
    }
    let mut copied = Vec::new();
    for (index, track) in timeline.tracks.iter().enumerate() {
        let Some(spans) = ranges.get(&track.id) else {
            continue;
        };
        for (from, to) in union(spans) {
            for clip in &track.clips {
                if clip.start >= to || clip.end() <= from {
                    continue;
                }
                let mut slice = clip.clone();
                // A title and a colour count their own frames: the keys of a slice of one stay
                // where they were on the timeline. (A still does too, but is not told from its
                // media here, where there is no project: its keys go with the head of its slice.)
                let own = clip.source_in().is_none();
                if slice.end() > to {
                    slice = halves_of(&slice, to, timeline.rate, own).0;
                }
                if slice.start < from {
                    slice = halves_of(&slice, from, timeline.rate, own).1;
                }
                // Slicing allocates no project clip here; retain the source identity so
                // transitions can find both captured sides. Paste allocates all fresh ids.
                slice.id.clone_from(&clip.id);
                let lanes = Follow::capture(timeline, &track.id, &slice);
                copied.push(CopiedClip {
                    track: track.id.clone(),
                    index: timeline.tracks[..index]
                        .iter()
                        .filter(|t| t.kind == track.kind)
                        .count(),
                    kind: track.kind,
                    clip: slice,
                    lanes,
                });
            }
        }
    }
    if copied.is_empty() {
        return Err(EditError::EmptyRange);
    }
    let clipboard = ClipClipboard {
        rate: timeline.rate,
        at,
        transitions: copy_transitions(timeline, &copied),
        clips: copied,
    };
    clipboard.check()?;
    Ok(clipboard)
}

impl ClipClipboard {
    pub(super) fn check(&self) -> Result<(), OpError> {
        let bad = || OpError::OutOfRange { what: "clipboard" };
        if self.rate.num == 0 || self.rate.den == 0 || self.at < 0 || self.clips.is_empty() {
            return Err(bad());
        }
        within("at", i128::from(self.at))?;
        for item in &self.clips {
            let clip = &item.clip;
            within("start", i128::from(clip.start))?;
            within("end", i128::from(clip.start) + i128::from(clip.length))?;
            if clip.start < 0 || clip.length <= 0 || item.kind == TrackKind::Subtitle {
                return Err(bad());
            }
            for lane in &item.lanes {
                lane.check_snapshot()?;
            }
        }
        for copied in &self.transitions {
            let (Some(left), Some(right)) =
                (self.clips.get(copied.left), self.clips.get(copied.right))
            else {
                return Err(bad());
            };
            if left.track != right.track
                || left.clip.end() != right.clip.start
                || left.clip.id != copied.transition.left
                || right.clip.id != copied.transition.right
                || copied.transition.length <= 0
            {
                return Err(bad());
            }
            within("transition length", i128::from(copied.transition.length))?;
        }
        Ok(())
    }
}

pub(super) fn cut(
    work: &mut Work,
    clips: &[Id],
    (from, to): (Option<i64>, Option<i64>),
    linked: bool,
    grouped: bool,
) -> Result<(), EditError> {
    let range = match (from, to) {
        (None, None) => None,
        (Some(from), Some(to)) => Some((from, to)),
        _ => return Err(OpError::OutOfRange { what: "range" }.into()),
    };
    let selected = work.group(clips, linked)?;
    let clipboard = copy_clips(work.timeline(), &selected, 0, range, false, grouped)?;
    for item in clipboard.clips {
        work.clear_clips(&[item.track], item.clip.start, item.clip.end())?;
    }
    Ok(())
}

fn frames(value: i64, source: Rate, destination: Rate) -> Result<i64, OpError> {
    let n = i128::from(value) * i128::from(source.den) * i128::from(destination.num);
    let d = i128::from(source.num) * i128::from(destination.den);
    let result = (2 * n + d).div_euclid(2 * d);
    within("clipboard frame", result)?;
    i64::try_from(result).map_err(|_| OpError::OutOfRange {
        what: "clipboard frame",
    })
}

pub(super) fn paste(
    work: &mut Work,
    clipboard: &ClipClipboard,
    at: i64,
    tracks: &BTreeMap<Id, Id>,
) -> Result<(), EditError> {
    if at < 0 {
        return Err(OpError::OutOfTimeline { start: at }.into());
    }
    let mut copies = Vec::new();
    for (index, item) in clipboard.clips.iter().enumerate() {
        let track = tracks.get(&item.track).unwrap_or(&item.track);
        let target = work.open(track)?;
        if target.kind != item.kind {
            return Err(OpError::WrongKind {
                track: track.clone(),
                kind: target.kind,
            }
            .into());
        }
        let mut clip = item.clip.clone();
        // The speed points of a clip are at frames of the rate that it was copied at: they
        // are not put on the frames of another rate.
        if clip.retime.is_some() && clipboard.rate != work.rate() {
            return Err(OpError::OutOfRange { what: "retimed" }.into());
        }
        let start = at + frames(clip.start - clipboard.at, clipboard.rate, work.rate())?;
        let end = at + frames(clip.end() - clipboard.at, clipboard.rate, work.rate())?;
        if start < 0 {
            return Err(OpError::OutOfTimeline { start }.into());
        }
        clip.start = start;
        clip.length = (end - start).max(1);
        clip.fade_in = frames(clip.fade_in, clipboard.rate, work.rate())?.min(clip.length);
        clip.fade_out = frames(clip.fade_out, clipboard.rate, work.rate())?.min(clip.length);
        let key = format!("clipboard:{index}");
        work.keep_snapshot_lanes(&key, &item.lanes);
        copies.push((track.clone(), clip, key));
    }
    for (track, clip, _) in &copies {
        work.clear_clips(std::slice::from_ref(track), clip.start, clip.end())?;
    }
    let mut counts = HashMap::new();
    for (_, clip, _) in &copies {
        if let Some(link) = &clip.link {
            *counts.entry(link.clone()).or_insert(0) += 1;
        }
    }
    let mut links: HashMap<Id, Id> = HashMap::new();
    let mut inserted_clips = Vec::new();
    for (track, mut clip, key) in copies {
        let previous = clip.link.clone();
        clip.id.clear();
        clip.link = previous
            .as_ref()
            .filter(|id| counts[*id] > 1)
            .map(|id| links.get(id).cloned().unwrap_or_default());
        let inserted = work.insert_copy(&track, clip, &key)?;
        if let (Some(old), Some(new)) = (previous, &inserted.link) {
            links.insert(old, new.clone());
        }
        inserted_clips.push((track, inserted));
    }
    paste_transitions(work, clipboard, &inserted_clips)?;
    Ok(())
}

fn paste_transitions(
    work: &mut Work,
    clipboard: &ClipClipboard,
    clips: &[(Id, Clip)],
) -> Result<(), EditError> {
    for copied in &clipboard.transitions {
        let (track, left) = &clips[copied.left];
        let (_, right) = &clips[copied.right];
        let mut transition = copied.transition.clone();
        transition.id.clear();
        transition.left.clone_from(&left.id);
        transition.right.clone_from(&right.id);
        transition.length = frames(transition.length, clipboard.rate, work.rate())?.max(1);
        // Range copies can shorten either side. Use the same fitting rule as edits that
        // reattach transitions after trimming; retain the shape and level parameters.
        match super::misc::check_fits(
            work,
            work.open(track)?,
            (left, right),
            transition.length,
            transition.align,
            None,
        ) {
            Ok(()) => {}
            Err(EditError::NoHandles {
                longest: Some(length),
                ..
            }) => {
                work.clamp();
                transition.length = length;
            }
            Err(error) => return Err(error),
        }
        if transition.length > 0 {
            work.op(Op::AddTransition {
                timeline: work.timeline_id(),
                track: track.clone(),
                transition,
            })?;
        }
    }
    Ok(())
}
