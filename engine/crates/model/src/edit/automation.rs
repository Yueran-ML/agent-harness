//! The edits of the automation of a timeline (Ch. 178, pp. 3893–3915): points of a lane added,
//! taken away and moved; a range of a lane taken away, raised or lowered, pasted over or thinned;
//! a lane drawn with the Pencil; the lanes of a track, a bus or a VCA cleared; and the inserts of
//! a strip changed so that no lane is left naming what is not there.
//!
//! Each is worked out into [`Op::SetLane`] and [`Op::SpliceLane`] (and the operations of
//! sound), as one transaction and one step of undo. A change to the points of a lane that is
//! there is one `splice_lane` of the points between the first and the last that change, so its
//! inverse holds only those.

use std::cell::Cell;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::work::{missing, Work};
use super::EditError;
use crate::audio::automation::{
    defaults_like, fits, fixed_value, held, kind_of, lanes_of_changed_inserts, no_lane, offset_by,
    paths, point, range_of, replaced, shifted, switched, thinned, ticks_in, tolerance_of, value_at,
    view, with_value, Owner, Set, Shape, LANE_RAMP_PER_SECOND,
};
use crate::audio::{
    AutomationArm, GroupControl, Insert, Lane, LaneKind, LanePoint, LaneScale, Lanes,
};
use crate::ops::{inserts_of, owner_of, InsertsOf, LanesOf, Op, OpError, LARGEST};
use crate::project::{Id, Project, Timeline, TrackKind};
use crate::time::Time;

/// What [`super::Edit::LanePoints`] does to points of a lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointAction {
    /// Add points (Alt+click, p. 3898), or set those there.
    Add,
    /// Take the points at times away (Ctrl+Alt+click, p. 3898).
    Remove,
    /// Move the points at times, in time and in value (p. 3911).
    Move,
}

/// What [`super::Edit::LaneRange`] does to a range of a lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RangeAction {
    /// Take its points away, the values at its ends kept (p. 3898).
    Delete,
    /// The same, for a client that keeps the points that it cut (p. 3898).
    Cut,
    /// Put points in place of it, the values at its ends kept (p. 3898).
    Paste,
    /// Raise or lower it, with bounding points at its ends (p. 3899).
    ShiftValue,
    /// Thin its points (C-A4).
    Thin,
}

/// What [`super::Edit::ClearLanes`] leaves a setting at once its lane is gone (Clear Track
/// Automation, p. 3912).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Then {
    /// As it was set.
    #[default]
    Keep,
    /// As a new strip has it (Reset).
    Reset,
    /// At the value of its lane at `at` (Hold).
    Hold,
}

/// What follows the end of a run of a pass of recording on a lane (p. 3905).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunEnd {
    /// Back to the lane as it was, on a straight line over the run's `glide` (Snap let go; Return
    /// at the stop); at once for a switch.
    Return,
    /// The last value, to the end of the timeline: the lane after it goes (Hold at the stop).
    Hold,
    /// The last value, to the next point of the lane after the end, and the lane as it was from
    /// there; to the end where there is none (Event at the stop).
    Event,
    /// The lane as it was, at once from the end (the end of the Active Range, p. 3908).
    Cut,
}

/// A run of a pass of recording on a lane: the values recorded from where a control was touched
/// (or from the start of the Active Range) to where it stopped being written, and what follows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedRun {
    /// The values recorded, at times of the timeline, in order: the first where the run starts.
    /// Of a Trim, the offsets in decibels.
    pub points: Vec<LanePoint>,
    /// Where the run ends, a time of the timeline at or after its last point: the last value is
    /// held to it.
    pub to: Time,
    /// What follows its end.
    pub then: RunEnd,
    /// How long a return takes (C-A1, C-A2); none, at once, where it is left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glide: Option<Time>,
}

/// What a pass of recording wrote on one lane: its runs, in the order of time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedLane {
    /// The track of sound, bus or VCA.
    pub of: LanesOf,
    /// The path of the setting of the lane.
    pub path: String,
    /// Whether the runs are offsets in decibels of the lane as it was (Trim, pp. 3900–3901), not
    /// its values (Write).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub trim: bool,
    /// The runs.
    pub runs: Vec<RecordedRun>,
}

/// A point to add: a time of the timeline and, where it is given, a value; where it is not, the
/// value that the lane has there.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewPoint {
    /// Its time.
    pub at: Time,
    /// Its value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
}

fn range(what: &'static str) -> EditError {
    EditError::Op(OpError::OutOfRange { what })
}

fn point_name(at: Time) -> String {
    format!("{}/{}", at.ticks, at.scale)
}

/// A lane as an edit changes it: of what and of which setting, its shape, the scale of its
/// points (that of the lane, or the sample rate of the timeline for one that is not there yet),
/// the value of its setting as it is set, and its points; and the range of its setting, found
/// once, where it is needed.
struct Target {
    of: LanesOf,
    path: String,
    shape: Shape,
    scale: u32,
    fixed: f64,
    points: Vec<LanePoint>,
    exists: bool,
    range: Cell<Option<(f64, f64)>>,
}

/// The owner of the lanes of `of` in the project of `work`, which an edit may change: a track
/// that is locked is refused.
fn open_owner<'w>(work: &'w Work, of: &LanesOf) -> Result<(Owner<'w>, &'w Lanes), EditError> {
    if let LanesOf::Track(id) = of {
        work.open(id)?;
    }
    Ok(owner_of(work.timeline(), of)?)
}

fn target(work: &Work, of: &LanesOf, path: &str) -> Result<Target, EditError> {
    let (owner, lanes) = open_owner(work, of)?;
    let view = view(owner);
    let refused = || range(no_lane(owner, path));
    let kind = kind_of(&view, path).ok_or_else(refused)?;
    let fixed = fixed_value(&view, path).ok_or_else(refused)?;
    let (scale, points, exists) = match lanes.get(path) {
        Some(lane) => (lane.points[0].at.scale, lane.points.clone(), true),
        None => (work.timeline().sample_rate, vec![], false),
    };
    Ok(Target {
        of: of.clone(),
        path: path.into(),
        shape: Shape {
            kind,
            scale: LaneScale::of_path(path),
        },
        scale,
        fixed,
        points,
        exists,
        range: Cell::new(None),
    })
}

impl Target {
    /// A time of the timeline as ticks of the lane, or `out_of_range` naming `what`.
    fn ticks(&self, at: Time, what: &'static str) -> Result<i64, EditError> {
        ticks_in(at, self.scale).ok_or_else(|| range(what))
    }

    /// The least and the greatest value of its setting, found the first time that it is asked
    /// for and kept for the rest of the edit.
    fn range(&self, work: &Work) -> (f64, f64) {
        if let Some(range) = self.range.get() {
            return range;
        }
        let range = owner_of(work.timeline(), &self.of)
            .map_or((-f64::MAX, f64::MAX), |(owner, _)| {
                range_of(owner, &self.path, self.fixed)
            });
        self.range.set(Some(range));
        range
    }

    /// `value` as the setting takes it: a switch on or off, or a number kept within its range,
    /// saying in `moved` that it was kept.
    fn kept(&self, work: &Work, value: f64, moved: &Cell<bool>) -> f64 {
        match self.shape.kind {
            LaneKind::Switch => switched(value),
            LaneKind::Continuous => {
                let (least, most) = self.range(work);
                let kept = value.clamp(least, most);
                if kept.to_bits() != value.to_bits() {
                    moved.set(true);
                }
                kept
            }
        }
    }
}

/// Put `points` in place of the lane of a target: the lane added where it was not, taken away
/// where nothing is left of it, and otherwise one `splice_lane` of the points that change. An
/// edit that changes nothing is `empty_range`.
fn commit(work: &mut Work, target: Target, points: Vec<LanePoint>) -> Result<(), EditError> {
    let timeline = work.timeline_id();
    let Target {
        of, path, shape, ..
    } = target;
    if !target.exists {
        if points.is_empty() {
            return Err(EditError::EmptyRange);
        }
        work.op(Op::SetLane {
            timeline,
            of,
            path,
            lane: Some(Lane {
                points,
                kind: shape.kind,
                scale: Some(shape.scale),
            }),
        })?;
        return Ok(());
    }
    if points.is_empty() {
        work.op(Op::SetLane {
            timeline,
            of,
            path,
            lane: None,
        })?;
        return Ok(());
    }
    let old = &target.points;
    let same =
        |a: &LanePoint, b: &LanePoint| a.at == b.at && a.value.to_bits() == b.value.to_bits();
    let most = old.len().min(points.len());
    let head = (0..most).take_while(|&i| same(&old[i], &points[i])).count();
    let tail = (0..most - head)
        .take_while(|&i| same(&old[old.len() - 1 - i], &points[points.len() - 1 - i]))
        .count();
    if head == old.len() && head == points.len() {
        return Err(EditError::EmptyRange);
    }
    work.op(Op::SpliceLane {
        timeline,
        of,
        path,
        at: head,
        remove: old.len() - head - tail,
        points: points[head..points.len() - tail].to_vec(),
    })?;
    Ok(())
}

/// Add, take away or move points of the lane of the setting at `path` of `of` (pp. 3898–3900,
/// 3911–3912). Added points go where their times are, and one at the time of a point that is
/// there sets the value of the last there; one given no value takes the value that the lane has
/// there, so that nothing is heard to change; a lane that is not there is made, of the kind of its
/// setting. Points are named by their times, every point at a time at once; taking the last away
/// takes the lane away. A move shifts the points named by `by` and their values by `delta`, as far
/// as the points beside them and the range of the setting let it, and is `clamped` where they
/// stop it.
pub(super) fn lane_points(
    work: &mut Work,
    of: &LanesOf,
    path: &str,
    action: PointAction,
    new: &[NewPoint],
    at: &[Time],
    (by, delta): (Option<Time>, Option<f64>),
) -> Result<(), EditError> {
    let target = target(work, of, path)?;
    let moved = Cell::new(false);
    let points = match action {
        PointAction::Add => {
            if new.is_empty() {
                return Err(range("points"));
            }
            let mut points = target.points.clone();
            for p in new {
                let ticks = target.ticks(p.at, "at")?;
                let here = value_at(&points, target.shape, target.fixed, ticks);
                let value = match p.value {
                    Some(value) if value.is_finite() => target.kept(work, value, &moved),
                    Some(_) => return Err(range("value")),
                    None => here,
                };
                let after = points.partition_point(|q| q.at.ticks <= ticks);
                match points.get_mut(after.wrapping_sub(1)) {
                    Some(last) if last.at.ticks == ticks => last.value = value,
                    _ => points.insert(after, point(ticks, target.scale, value)),
                }
            }
            points
        }
        PointAction::Remove | PointAction::Move => {
            if !target.exists {
                return Err(missing("lane", path));
            }
            if at.is_empty() {
                return Err(range("at"));
            }
            let mut chosen = vec![false; target.points.len()];
            for &time in at {
                let ticks = target.ticks(time, "at")?;
                let mut found = false;
                for (p, chosen) in target.points.iter().zip(chosen.iter_mut()) {
                    if p.at.ticks == ticks {
                        *chosen = true;
                        found = true;
                    }
                }
                if !found {
                    return Err(missing("point", &point_name(time)));
                }
            }
            if action == PointAction::Remove {
                target
                    .points
                    .iter()
                    .zip(&chosen)
                    .filter(|(_, chosen)| !**chosen)
                    .map(|(p, _)| p.clone())
                    .collect()
            } else {
                moved_points(work, &target, &chosen, (by, delta), &moved)?
            }
        }
    };
    if moved.get() {
        work.clamp();
    }
    commit(work, target, points)
}

/// The points of a target with those `chosen` moved by `by` in time and `delta` in value, as far
/// as their neighbours, the ends of the timeline and the range of the setting let them.
fn moved_points(
    work: &Work,
    target: &Target,
    chosen: &[bool],
    (by, delta): (Option<Time>, Option<f64>),
    moved: &Cell<bool>,
) -> Result<Vec<LanePoint>, EditError> {
    let delta = delta.unwrap_or(0.0);
    if !delta.is_finite() {
        return Err(range("delta"));
    }
    let by = match by {
        None => 0,
        Some(by) => {
            let sign = if by.ticks < 0 { -1 } else { 1 };
            let size = Time {
                ticks: by.ticks.saturating_abs(),
                scale: by.scale,
            };
            sign * target.ticks(size, "by")?
        }
    };
    if by == 0 && delta == 0.0 {
        return Err(EditError::EmptyRange);
    }
    // How far the points chosen may go: each run of them stays between the points beside it.
    let points = &target.points;
    let (mut least, mut most) = (i64::MIN, i64::MAX);
    for (i, p) in points.iter().enumerate() {
        if !chosen[i] {
            continue;
        }
        let before = if i == 0 { 0 } else { points[i - 1].at.ticks };
        if i == 0 || !chosen[i - 1] {
            least = least.max(before - p.at.ticks);
        }
        let after = points.get(i + 1).map_or(LARGEST, |q| q.at.ticks);
        if i + 1 == points.len() || !chosen[i + 1] {
            most = most.min(after - p.at.ticks);
        }
    }
    let shift = by.clamp(least, most);
    if shift != by {
        moved.set(true);
    }
    Ok(points
        .iter()
        .zip(chosen)
        .map(|(p, &chosen)| {
            if chosen {
                let value = target.kept(work, p.value + delta, moved);
                point(p.at.ticks + shift, target.scale, value)
            } else {
                p.clone()
            }
        })
        .collect())
}

/// Change the range from `from` to `to` of the lane of the setting at `path` of `of` (pp.
/// 3898–3899): take its points away; paste `points` (at times from `from`) over it; raise or lower
/// it by `delta`, ramping in and out over 10 ms within it (a switch jumps); or thin its points by
/// `tolerance` (C-A4, where it is left out). The values before it and from its end on stay as they
/// were. A lane that is not there is pasted or raised from the value of its setting.
pub(super) fn lane_range(
    work: &mut Work,
    of: &LanesOf,
    path: &str,
    (from, to): (Time, Time),
    action: RangeAction,
    pasted: Option<&[LanePoint]>,
    (delta, tolerance): (Option<f64>, Option<f64>),
) -> Result<(), EditError> {
    let target = target(work, of, path)?;
    let (from, to) = (target.ticks(from, "from")?, target.ticks(to, "to")?);
    if from >= to {
        return Err(EditError::EmptyRange);
    }
    let needs_lane = matches!(
        action,
        RangeAction::Delete | RangeAction::Cut | RangeAction::Thin
    );
    if needs_lane && !target.exists {
        return Err(missing("lane", path));
    }
    let moved = Cell::new(false);
    let (shape, fixed, scale) = (target.shape, target.fixed, target.scale);
    let points = match action {
        RangeAction::Delete | RangeAction::Cut => {
            replaced(&target.points, shape, fixed, scale, (from, to), &[])
        }
        RangeAction::Paste => {
            let pasted = pasted.ok_or_else(|| range("points"))?;
            let mut new = Vec::with_capacity(pasted.len());
            for p in pasted {
                let at = target.ticks(p.at, "points")?.saturating_add(from);
                if at > to || new.last().is_some_and(|q: &LanePoint| q.at.ticks > at) {
                    return Err(range("points"));
                }
                if !p.value.is_finite() {
                    return Err(range("value"));
                }
                new.push(point(at, scale, target.kept(work, p.value, &moved)));
            }
            crate::audio::automation::pasted(&target.points, shape, fixed, scale, (from, to), &new)
        }
        RangeAction::ShiftValue => {
            let delta = delta.unwrap_or(0.0);
            if !delta.is_finite() {
                return Err(range("delta"));
            }
            if delta == 0.0 {
                return Err(EditError::EmptyRange);
            }
            let ramp = match shape.kind {
                LaneKind::Switch => 0,
                LaneKind::Continuous => i64::from(scale / LANE_RAMP_PER_SECOND),
            };
            let change = |value: f64| target.kept(work, value + delta, &moved);
            shifted(
                &target.points,
                shape,
                fixed,
                scale,
                (from, to),
                ramp,
                &change,
            )
        }
        RangeAction::Thin => {
            let tolerance = match tolerance {
                Some(t) if t.is_finite() && t >= 0.0 => t,
                Some(_) => return Err(range("tolerance")),
                None => tolerance_for(work, &target),
            };
            let first = target.points.partition_point(|p| p.at.ticks < from);
            let end = target.points.partition_point(|p| p.at.ticks <= to);
            let mut points = target.points[..first].to_vec();
            points.extend(thinned(&target.points[first..end], shape, tolerance));
            points.extend_from_slice(&target.points[end..]);
            points
        }
    };
    if moved.get() {
        work.clamp();
    }
    commit(work, target, points)
}

/// How far a lane of a target may go from what was drawn, where nothing is given (C-A4), of the
/// range of its setting as the edit has found it.
fn tolerance_for(work: &Work, target: &Target) -> f64 {
    tolerance_of(&target.path, target.range(work))
}

/// Draw the lane of the setting at `path` of `of` from `from` to `to` with the Pencil (pp. 3900,
/// 3911): `points`, at times of the timeline from `from` to `to` in order, thinned by `tolerance`
/// (C-A4, where it is left out; a switch keeps where it changes, each value on or off as it is
/// past the middle, C-A5), are put in place of what the range had, the values before it and from
/// its end on staying as they were. A lane that is not there is made.
pub(super) fn lane_draw(
    work: &mut Work,
    of: &LanesOf,
    path: &str,
    (from, to): (Time, Time),
    drawn: &[LanePoint],
    tolerance: Option<f64>,
) -> Result<(), EditError> {
    let target = target(work, of, path)?;
    let (from, to) = (target.ticks(from, "from")?, target.ticks(to, "to")?);
    if from >= to {
        return Err(EditError::EmptyRange);
    }
    if drawn.is_empty() {
        return Err(range("points"));
    }
    let moved = Cell::new(false);
    let mut new: Vec<LanePoint> = Vec::with_capacity(drawn.len());
    for p in drawn {
        let at = target.ticks(p.at, "points")?;
        let early = new.last().is_some_and(|q| q.at.ticks > at);
        if at < from || at > to || early {
            return Err(range("points"));
        }
        if !p.value.is_finite() {
            return Err(range("value"));
        }
        new.push(point(at, target.scale, target.kept(work, p.value, &moved)));
    }
    let tolerance = match tolerance {
        Some(t) if t.is_finite() && t >= 0.0 => t,
        Some(_) => return Err(range("tolerance")),
        None => tolerance_for(work, &target),
    };
    let new = thinned(&new, target.shape, tolerance);
    let (shape, fixed, scale) = (target.shape, target.fixed, target.scale);
    let points = replaced(&target.points, shape, fixed, scale, (from, to), &new);
    if moved.get() {
        work.clamp();
    }
    commit(work, target, points)
}

/// Write what a pass of the recording of automation recorded (T16, pp. 3905–3909) into the lanes
/// that it was of, as one transaction: each run, in the order of time, puts the values recorded
/// (thinned as a lane drawn is, C-A4; kept within the range of the setting, a switch on or off)
/// in place of the lane from where it starts, the value before it staying as it was, the last
/// value held to its end, and then what [`RunEnd`] says. A run of a Trim moves the lane as it was
/// before the edit by the offsets that it recorded (a fader's −∞ staying), and its return takes
/// the offset back to 0: the lanes of one setting given twice (passes of Preview punched in
/// together) each move the lane as it was before them all, the later taking the place of the
/// earlier where it writes. A number never steps where it was heard to ramp: a run comes in
/// from the lane over 10 ms, and gives it back over 10 ms at least (C-T1); a switch jumps. A
/// lane that is not there is made, from the value of its setting.
pub(super) fn record_pass(work: &mut Work, lanes: &[RecordedLane]) -> Result<(), EditError> {
    if lanes.is_empty() {
        return Err(range("lanes"));
    }
    let mut before: Vec<(&LanesOf, &str, Vec<LanePoint>)> = Vec::new();
    for recorded in lanes {
        let known = before
            .iter()
            .find(|(of, path, _)| **of == recorded.of && *path == recorded.path);
        let was = if let Some((_, _, was)) = known {
            was.clone()
        } else {
            let was = target(work, &recorded.of, &recorded.path)?.points;
            before.push((&recorded.of, &recorded.path, was.clone()));
            was
        };
        record_lane(work, recorded, &was)?;
    }
    Ok(())
}

/// One lane of [`record_pass`], `was` its points before the edit.
#[allow(
    clippy::too_many_lines,
    reason = "a run read, ramped in, ended and put in place, step by step"
)]
fn record_lane(
    work: &mut Work,
    recorded: &RecordedLane,
    was: &[LanePoint],
) -> Result<(), EditError> {
    let target = target(work, &recorded.of, &recorded.path)?;
    let (shape, fixed, scale) = (target.shape, target.fixed, target.scale);
    if recorded.runs.is_empty() {
        return Err(range("runs"));
    }
    let decibels = shape.kind == LaneKind::Continuous
        && shape.scale == LaneScale::Linear
        && recorded.path.ends_with("_db");
    if recorded.trim && !decibels {
        return Err(range("trim"));
    }
    let moved = Cell::new(false);
    let tolerance = tolerance_for(work, &target);
    let line = Shape {
        kind: LaneKind::Continuous,
        scale: LaneScale::Linear,
    };
    // The ramp of a number in and out (C-T1: 10 ms); none for a switch.
    let ramp = match shape.kind {
        LaneKind::Continuous => i64::from(scale / LANE_RAMP_PER_SECOND),
        LaneKind::Switch => 0,
    };
    let mut points = target.points.clone();
    let mut last_from = i64::MIN;
    for run in &recorded.runs {
        let mut values = Vec::with_capacity(run.points.len() + 1);
        for p in &run.points {
            let at = target.ticks(p.at, "points")?;
            if values.last().is_some_and(|q: &LanePoint| q.at.ticks > at) {
                return Err(range("points"));
            }
            if !p.value.is_finite() {
                return Err(range("value"));
            }
            let value = if recorded.trim {
                p.value
            } else {
                target.kept(work, p.value, &moved)
            };
            values.push(point(at, scale, value));
        }
        let (Some(first), Some(last)) = (values.first().cloned(), values.last().cloned()) else {
            return Err(range("points"));
        };
        let (from, to) = (first.at.ticks, target.ticks(run.to, "to")?);
        if last.at.ticks > to {
            return Err(range("points"));
        }
        if from < last_from {
            return Err(range("runs"));
        }
        last_from = from;
        let glide = match (run.glide, shape.kind) {
            (Some(glide), LaneKind::Continuous) => target.ticks(glide, "glide")?.max(ramp),
            _ => ramp,
        };
        let mut new = values;
        // It comes in over the ramp: its first value where the ramp ends, unless the next is
        // there already (of a Trim, from an offset of 0).
        if ramp > 0 && to > from {
            let ramped = (from + ramp).min(to);
            if new.get(1).is_some_and(|next| next.at.ticks <= ramped) {
                new.remove(0);
            } else {
                new[0].at.ticks = ramped;
            }
        }
        // Thin the trajectory after making its entry ramp. Moving the first point of an
        // already thinned line would also change its slope beyond the next recorded point.
        let mut new = thinned(&new, if recorded.trim { line } else { shape }, tolerance);
        // Keep the entry endpoint as thinning's first point. Adding Trim's zero anchor
        // before thinning could remove that endpoint and lengthen even a small entry ramp.
        if recorded.trim && ramp > 0 && to > from {
            new.insert(0, point(from, scale, 0.0));
        }
        // The last recorded value stays exact from its point until the stop.
        if last.at.ticks < to {
            new.push(point(to, scale, last.value));
        }
        // Where the run gives the lane back: none for one that holds to the end.
        let next = was.iter().find(|p| p.at.ticks > to).map(|p| p.at.ticks);
        let end = match run.then {
            RunEnd::Return => Some(to.saturating_add(glide).min(LARGEST)),
            RunEnd::Cut => Some(to.saturating_add(ramp).min(LARGEST)),
            RunEnd::Event => next,
            RunEnd::Hold => None,
        };
        if let (RunEnd::Event, Some(end)) = (run.then, end) {
            // Held to the next point of the lane, and into it over the ramp.
            new.push(point((end - ramp).max(to), scale, last.value));
        }
        if recorded.trim {
            if let Some(end) = end.filter(|_| run.then != RunEnd::Hold) {
                new.push(point(end, scale, 0.0));
            }
            new = offset_by(was, shape, fixed, scale, &new, end)
                .into_iter()
                .map(|p| {
                    let value = target.kept(work, p.value, &moved);
                    point(p.at.ticks, scale, value)
                })
                .collect();
        }
        points = match end {
            Some(end) => replaced(&points, shape, fixed, scale, (from, end), &new),
            None => held(&points, shape, fixed, scale, from, &new),
        };
    }
    if moved.get() {
        work.clamp();
    }
    commit(work, target, points)
}

// Fill Range, Glide Range, and Copy, Paste and Erase of automation (T16b, pp. 3908, 3913–3914).

/// A lane that Fill Range or Glide Range writes (pp. 3908, 3913): of what, which setting, and the
/// value that Preview holds it at.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilledLane {
    /// The track of sound, bus or VCA.
    pub of: LanesOf,
    /// The path of the setting of the lane.
    pub path: String,
    /// The value, in the units of the setting.
    pub value: f64,
}

/// A lane of automation as Fairlight › Automation › Copy takes it (p. 3913), and as Paste puts
/// it: of what, which setting, and either the points of a range, at times from its start (its
/// value at the start the first point, its value at the end the last), or, copied with no range,
/// the value at the playhead (a snapshot).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CopiedLane {
    /// The track of sound, bus or VCA.
    pub of: LanesOf,
    /// The path of the setting of the lane.
    pub path: String,
    /// The points of the range, at times from its start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<Vec<LanePoint>>,
    /// The value at the playhead, of a snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
}

/// The Automation Arm of `of` on `timeline` (T16b).
///
/// # Errors
/// It is not there (`missing` track, `unknown_bus`, `unknown_vca`, or a track not of sound).
pub fn arm_of(timeline: &Timeline, of: &LanesOf) -> Result<AutomationArm, OpError> {
    owner_of(timeline, of)?;
    Ok(match of {
        LanesOf::Track(id) => timeline
            .tracks
            .iter()
            .find(|t| t.id == *id)
            .map_or_else(AutomationArm::default, |t| t.audio.automation_arm),
        LanesOf::Bus(id) => timeline
            .buses
            .iter()
            .find(|b| b.id == *id)
            .map_or_else(AutomationArm::default, |b| b.automation_arm),
        LanesOf::Vca(id) => timeline
            .vcas
            .iter()
            .find(|v| v.id == *id)
            .map_or_else(AutomationArm::default, |v| v.automation_arm),
    })
}

/// Do `one` and say whether it changed anything: an edit of one lane among several that changes
/// nothing is passed over.
fn changed(one: Result<(), EditError>) -> Result<bool, EditError> {
    match one {
        Ok(()) => Ok(true),
        Err(EditError::EmptyRange) => Ok(false),
        Err(error) => Err(error),
    }
}

/// Fill Range (p. 3908, 3913): each lane of `lanes` written over the range from `from` to `to`
/// with its value; or Glide Range (`glide`): from the value that the lane has at `from` on a
/// straight line to its value at `to` (a switch is filled). A number ramps in and back out over
/// 10 ms, as a pass of recording does (C-T1): in within the range, out after its end, the values
/// before it and after the ramp out staying as they were. A track, bus or VCA whose arm is Safe
/// is left alone (the interaction reference §11.4); a lane that is not there is made.
pub(super) fn lane_fill(
    work: &mut Work,
    lanes: &[FilledLane],
    (from, to): (Time, Time),
    glide: bool,
) -> Result<(), EditError> {
    if lanes.is_empty() {
        return Err(range("lanes"));
    }
    let mut done = false;
    for filled in lanes {
        // Neither Safe nor Off is written (T16b's review: Off is not written either).
        if !arm_of(work.timeline(), &filled.of)?.records() {
            continue;
        }
        if !filled.value.is_finite() {
            return Err(range("value"));
        }
        let target = target(work, &filled.of, &filled.path)?;
        let (start, end) = (target.ticks(from, "from")?, target.ticks(to, "to")?);
        if start >= end {
            return Err(EditError::EmptyRange);
        }
        let mut points = Vec::with_capacity(2);
        if glide && target.shape.kind == LaneKind::Continuous {
            let was = value_at(&target.points, target.shape, target.fixed, start);
            points.push(LanePoint {
                at: from,
                value: was,
            });
            points.push(LanePoint {
                at: to,
                value: filled.value,
            });
        } else {
            points.push(LanePoint {
                at: from,
                value: filled.value,
            });
        }
        let recorded = RecordedLane {
            of: filled.of.clone(),
            path: filled.path.clone(),
            trim: false,
            runs: vec![RecordedRun {
                points,
                to,
                then: RunEnd::Cut,
                glide: None,
            }],
        };
        done |= changed(record_lane(work, &recorded, &target.points))?;
    }
    if done {
        Ok(())
    } else {
        Err(EditError::EmptyRange)
    }
}

/// What Copy of automation takes (p. 3913) of `of` on `timeline`: of each, the lanes of the
/// categories of recording that are on (pp. 3905–3906); with a range, their points within it at
/// times from `from` (in the scale of each lane), its value at `from` the first and its value at
/// `to` the last; with none, the value of each at `at` (a snapshot), and the value as it is set
/// of each fader, mute, pan and send of a category that is on and has no lane, so that a static
/// mix is copied too. A Safe arm is copied from: it is only not written to.
///
/// # Errors
/// An owner is not there (as of [`arm_of`]); a `from` or `to` out of the timeline, or a range
/// that is empty, is `out_of_range` (`from`, `to`); neither a range nor a time, or both, is
/// `out_of_range` (`at`).
pub fn copy_lanes(
    timeline: &Timeline,
    of: &[LanesOf],
    range: Option<(Time, Time)>,
    at: Option<Time>,
) -> Result<Vec<CopiedLane>, OpError> {
    if range.is_some() && at.is_some() {
        return Err(OpError::OutOfRange { what: "at" });
    }
    let categories = timeline.audio.record.categories;
    let mut copied = Vec::new();
    for owner in of {
        let (model, lanes) = owner_of(timeline, owner)?;
        for (path, lane) in lanes.iter().filter(|(p, _)| categories.covers(p)) {
            let scale = lane.points[0].at.scale;
            let (points, value) = match (range, at) {
                (Some((from, to)), _) => {
                    let start =
                        ticks_in(from, scale).ok_or(OpError::OutOfRange { what: "from" })?;
                    let end = ticks_in(to, scale).ok_or(OpError::OutOfRange { what: "to" })?;
                    if start >= end {
                        return Err(OpError::OutOfRange { what: "to" });
                    }
                    let mut points =
                        vec![point(0, scale, lane.value_at(point(start, scale, 0.0).at))];
                    points.extend(
                        lane.points
                            .iter()
                            .filter(|p| p.at.ticks > start && p.at.ticks < end)
                            .map(|p| point(p.at.ticks - start, scale, p.value)),
                    );
                    points.push(point(
                        end - start,
                        scale,
                        lane.value_at(point(end, scale, 0.0).at),
                    ));
                    (Some(points), None)
                }
                (None, Some(at)) => (None, Some(lane.value_at(at))),
                (None, None) => return Err(OpError::OutOfRange { what: "at" }),
            };
            copied.push(CopiedLane {
                of: owner.clone(),
                path: path.clone(),
                points,
                value,
            });
        }
        if range.is_none() {
            // A snapshot of the mix as it is set too: the faders, mutes, pans and sends that
            // have no lane, so that a static mix can be pasted (T16b's review).
            let view = view(model);
            for path in paths(model) {
                let mixed = path == "fader_db"
                    || path == "mute"
                    || path.starts_with("pan.")
                    || path.starts_with("send.");
                if !mixed || lanes.contains_key(&path) || !categories.covers(&path) {
                    continue;
                }
                if let Some(value) = fixed_value(&view, &path) {
                    copied.push(CopiedLane {
                        of: owner.clone(),
                        path,
                        points: None,
                        value: Some(value),
                    });
                }
            }
        }
    }
    Ok(copied)
}

/// Paste of automation (pp. 3913–3914): each lane of `lanes` (to any track, bus or VCA, and a
/// level to another level: a fader to a send and back) put at `at`. With `to`, it fills the range
/// from `at` to `to`: a snapshot's value over all of it, points as far as they go (cut at `to`,
/// the last value held to it where they end before). With none, points are pasted whole from
/// `at`; a snapshot needs a range (`out_of_range: to`), and a lane of no length is passed over. What was in the range is put in place of,
/// the values at its ends kept (p. 3898). A Safe arm, and a lane of a category of recording that
/// is off (p. 3913), is left alone.
pub(super) fn automation_paste(
    work: &mut Work,
    lanes: &[CopiedLane],
    at: Time,
    to: Option<Time>,
) -> Result<(), EditError> {
    if lanes.is_empty() {
        return Err(range("lanes"));
    }
    if at.scale == 0 || at.ticks < 0 {
        return Err(range("at"));
    }
    if let Some(to) = to {
        if to.scale == 0 || to.ticks < 0 {
            return Err(range("to"));
        }
        // Compare actual times here; rounding to the timeline grid can collapse a valid
        // range on a finer destination lane. Its tick bounds and precision are checked below.
        if i128::from(to.ticks) * i128::from(at.scale)
            <= i128::from(at.ticks) * i128::from(to.scale)
        {
            return Err(EditError::EmptyRange);
        }
    }
    let categories = work.timeline().audio.record.categories;
    let mut done = false;
    for lane in lanes {
        if !arm_of(work.timeline(), &lane.of)?.pastes() || !categories.covers(&lane.path) {
            continue;
        }
        let target = target(work, &lane.of, &lane.path)?;
        let rate = target.scale;
        let start = target.ticks(at, "at")?;
        let end = to.map(|to| target.ticks(to, "to")).transpose()?;
        if end.is_some_and(|end| end <= start) {
            return Err(EditError::EmptyRange);
        }
        let one = match (&lane.points, lane.value, end) {
            (Some(points), None, _) => {
                let mut relative = Vec::with_capacity(points.len() + 1);
                for p in points {
                    let ticks = ticks_in(p.at, rate).ok_or_else(|| range("points"))?;
                    if ticks < 0
                        || relative
                            .last()
                            .is_some_and(|q: &LanePoint| q.at.ticks > ticks)
                    {
                        return Err(range("points"));
                    }
                    if !p.value.is_finite() {
                        return Err(range("value"));
                    }
                    let moved = Cell::new(false);
                    let value = target.kept(work, p.value, &moved);
                    if moved.get() {
                        work.clamp();
                    }
                    relative.push(point(ticks, rate, value));
                }
                let Some(last) = relative.last().map(|p| p.at.ticks) else {
                    return Err(range("points"));
                };
                let span = end.map_or(last, |end| end - start);
                if span <= 0 {
                    // Nothing of it to paste: passed over (T16b's review).
                    continue;
                }
                let held = value_at(&relative, target.shape, target.fixed, span);
                // Keep both sides of a source jump at the cut: its left point determines
                // the curve approaching the exit ramp, even though its right point is at to.
                relative.retain(|p| p.at.ticks <= span);
                if relative.last().is_none_or(|p| p.at.ticks < span) {
                    relative.push(point(span, rate, held));
                }
                let to = point(start + span, rate, 0.0).at;
                let from = point(start, rate, 0.0).at;
                lane_range(
                    work,
                    &lane.of,
                    &lane.path,
                    (from, to),
                    RangeAction::Paste,
                    Some(&relative),
                    (None, None),
                )
            }
            (None, Some(value), Some(end)) => {
                let (from, to) = (point(start, rate, value), point(end, rate, value));
                lane_range(
                    work,
                    &lane.of,
                    &lane.path,
                    (from.at, to.at),
                    RangeAction::Paste,
                    Some(&[point(0, rate, value), point(end - start, rate, value)]),
                    (None, None),
                )
            }
            // A snapshot fills a range; with none it would reshape the lane around a point.
            (None, Some(_), None) => return Err(range("to")),
            _ => return Err(range("points")),
        };
        done |= changed(one)?;
    }
    if done {
        Ok(())
    } else {
        Err(EditError::EmptyRange)
    }
}

/// Erase of automation (p. 3914): the points of each lane of `of`, of the categories of
/// recording that are on, from `from` to `to` taken away, the values at its ends kept (p. 3898).
/// A Safe arm is left alone.
pub(super) fn automation_erase(
    work: &mut Work,
    of: &[LanesOf],
    (from, to): (Time, Time),
) -> Result<(), EditError> {
    if of.is_empty() {
        return Err(range("of"));
    }
    let categories = work.timeline().audio.record.categories;
    let mut done = false;
    for owner in of {
        if !arm_of(work.timeline(), owner)?.pastes() {
            continue;
        }
        let (_, lanes) = open_owner(work, owner)?;
        let paths: Vec<String> = lanes
            .keys()
            .filter(|path| categories.covers(path))
            .cloned()
            .collect();
        for path in paths {
            let one = lane_range(
                work,
                owner,
                &path,
                (from, to),
                RangeAction::Delete,
                None,
                (None, None),
            );
            done |= changed(one)?;
        }
    }
    if done {
        Ok(())
    } else {
        Err(EditError::EmptyRange)
    }
}

// Track Groups that link Automation (T16b, §4.2, §11.6).

/// The control of a group that links the setting at `path`, if one does: the fader, the mute,
/// the pans and the sends (a plugin's controls are never in a group, p. 3917).
#[must_use]
pub fn group_control(path: &str) -> Option<GroupControl> {
    match path {
        "fader_db" => Some(GroupControl::Fader),
        "mute" => Some(GroupControl::Mute),
        _ if path.starts_with("pan.") => Some(GroupControl::Panning),
        _ if path.starts_with("send.") => Some(GroupControl::Sends),
        _ => None,
    }
}

/// The other tracks of sound, not locked, that an edit of the lane at `path` of `of` is made to
/// as well (§11.6): each in an active group with it that links Automation and the control of the
/// setting, once, in the order of the tracks. None for a bus or a VCA.
#[must_use]
pub fn grouped_with(timeline: &Timeline, of: &LanesOf, path: &str) -> Vec<Id> {
    let (LanesOf::Track(track), Some(control)) = (of, group_control(path)) else {
        return Vec::new();
    };
    timeline
        .tracks
        .iter()
        .filter(|other| other.id != *track && other.kind == TrackKind::Audio && !other.locked)
        .filter(|other| {
            timeline.track_groups.iter().any(|group| {
                group.active
                    && group.linked.contains(&GroupControl::Automation)
                    && group.linked.contains(&control)
                    && group.members.contains(track)
                    && group.members.contains(&other.id)
            })
        })
        .map(|other| other.id.clone())
        .collect()
}

/// The value of the setting at `path` of `of` at `at`: its lane's, or as it is set.
fn setting_at(work: &Work, of: &LanesOf, path: &str, at: Time) -> Option<f64> {
    let target = target(work, of, path).ok()?;
    let ticks = target.ticks(at, "at").ok()?;
    Some(value_at(&target.points, target.shape, target.fixed, ticks))
}

/// How a value given to the setting of one track of a group is given to another's (§4.2, §11.6).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Follow {
    /// As it is: the track's own, or a switch, which follows to the same state.
    Same,
    /// Moved by how far the other's setting is from this one's: a number that is not a level.
    By(f64),
    /// A level (a fader, or another setting in decibels), moved by the same change in decibels
    /// from where this one's is (`ours`) to the value, from where the other's is (`theirs`): as
    /// T2's faders of a group follow, −∞ (−100 dB and below) counted as −100, and one at −∞ that
    /// would go down staying; no higher than +10 dB.
    Level { theirs: f64, ours: f64 },
}

impl Follow {
    /// `value` as it is given to the other track.
    pub(super) fn apply(self, value: f64) -> f64 {
        match self {
            Self::Same => value,
            Self::By(offset) => value + offset,
            Self::Level { theirs, ours } => super::audio::moved_level(
                theirs,
                super::audio::level_change(ours, value),
                LEVEL_MOST,
            ),
        }
    }
}

/// The highest a level of a group's follower goes, in decibels (§4.2, as T2's faders).
const LEVEL_MOST: f64 = 10.0;

/// How the setting at `path` of `member` follows that of `of`, as they are at `at` (§4.2): a
/// switch to the same state, a level by the same change in decibels, any other number by the
/// same amount.
pub(super) fn follow_at(
    work: &Work,
    of: &LanesOf,
    member: &LanesOf,
    path: &str,
    at: Time,
) -> Follow {
    let switch = target(work, of, path).is_ok_and(|t| t.shape.kind == LaneKind::Switch);
    if switch {
        return Follow::Same;
    }
    let level = path == "fader_db" || path.ends_with("_db");
    match (
        setting_at(work, member, path, at),
        setting_at(work, of, path, at),
    ) {
        (Some(theirs), Some(ours)) if level => Follow::Level { theirs, ours },
        (Some(theirs), Some(ours)) => Follow::By(theirs - ours),
        _ => Follow::Same,
    }
}

/// Make an edit of the lane at `path` of `of` (`edit`, given the owner and the offset that its
/// values are moved by) and, `grouped`, the same edit of that lane of each track grouped with it
/// (§11.6, [`grouped_with`]): a number moved by how far that track's setting is from this one's at
/// `at` before the edit (a relative move, as a fader of a group follows another, §4.2), a switch
/// to the same state. A track that the edit cannot be made to (it has no such setting, no such
/// lane or point, or it would change nothing) is passed over.
pub(super) fn with_group(
    work: &mut Work,
    (of, path): (&LanesOf, &str),
    (grouped, at): (bool, Option<Time>),
    edit: impl Fn(&mut Work, &LanesOf, Follow) -> Result<(), EditError>,
) -> Result<(), EditError> {
    let members = if grouped {
        grouped_with(work.timeline(), of, path)
    } else {
        Vec::new()
    };
    let follows: Vec<(LanesOf, Follow)> = members
        .into_iter()
        .map(|id| {
            let member = LanesOf::Track(id);
            let follow = at.map_or(Follow::Same, |at| follow_at(work, of, &member, path, at));
            (member, follow)
        })
        .collect();
    edit(work, of, Follow::Same)?;
    for (member, follow) in follows {
        // A track that cannot follow is passed over (see above).
        let _ = edit(work, &member, follow);
    }
    Ok(())
}

/// The top fields of `after` that differ from those of `before`, both objects.
fn differing(before: &Value, after: &Value) -> Map<String, Value> {
    let (Some(before), Some(after)) = (before.as_object(), after.as_object()) else {
        return Map::new();
    };
    after
        .iter()
        .filter(|(key, value)| before.get(*key) != Some(value))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

/// Take the lanes of `of` away (Clear Track Automation, p. 3912): those of `paths`, or all of
/// them, each setting then left as it was set (`keep`), as a new strip has it (`reset`), or at
/// the value of its lane at `at` (`hold`).
pub(super) fn clear_lanes(
    work: &mut Work,
    of: &LanesOf,
    paths: Option<&[String]>,
    then: Then,
    at: Option<Time>,
) -> Result<(), EditError> {
    let (owner, lanes) = open_owner(work, of)?;
    let cleared: Vec<(String, Lane)> = match paths {
        None => lanes.iter().map(|(p, l)| (p.clone(), l.clone())).collect(),
        Some(paths) => {
            if paths.is_empty() {
                return Err(range("paths"));
            }
            let mut list = Vec::with_capacity(paths.len());
            for path in paths {
                let lane = lanes.get(path).ok_or_else(|| missing("lane", path))?;
                if !list.iter().any(|(p, _): &(String, Lane)| p == path) {
                    list.push((path.clone(), lane.clone()));
                }
            }
            list
        }
    };
    if then == Then::Hold && at.is_none() {
        return Err(range("at"));
    }
    // What each setting is left at, where it changes.
    let mut set: Option<Set> = None;
    if then != Then::Keep {
        let defaults = match owner {
            Owner::Strip { strip, .. } => Some(defaults_like(strip)),
            Owner::Vca(_) => None,
        };
        let default_view = defaults.as_ref().map(|strip| {
            view(Owner::Strip {
                strip,
                mute: false,
                bus: false,
            })
        });
        for (path, lane) in &cleared {
            let value = match (then, at) {
                (Then::Hold, Some(at)) => lane.value_at(at),
                _ => match &default_view {
                    Some(view) => fixed_value(view, path).unwrap_or(0.0),
                    None => 0.0,
                },
            };
            let current = match &set {
                None => owner,
                Some(Set::Strip(strip, mute)) => Owner::Strip {
                    strip: strip.as_ref(),
                    mute: *mute,
                    bus: matches!(owner, Owner::Strip { bus: true, .. }),
                },
                Some(Set::Vca(vca)) => Owner::Vca(vca),
            };
            if !fits(current, path, value) {
                return Err(range("lane"));
            }
            set = with_value(current, path, value).or(set);
        }
    }
    let before = match owner {
        Owner::Strip { strip, mute, .. } => (serde_json::to_value(strip), mute),
        Owner::Vca(vca) => (serde_json::to_value(vca), vca.mute),
    };
    let timeline = work.timeline_id();
    for (path, _) in &cleared {
        work.op(Op::SetLane {
            timeline: timeline.clone(),
            of: of.clone(),
            path: path.clone(),
            lane: None,
        })?;
    }
    let (Some(set), (Ok(before), was_muted)) = (set, before) else {
        return Ok(());
    };
    if let Some(op) = settings_left(of, timeline, (&before, was_muted), set) {
        work.op(op)?;
    }
    Ok(())
}

/// The operation that sets what `of` was (`before`, as it is written, and its mute) to `set`:
/// of what differs, none where nothing does.
fn settings_left(of: &LanesOf, timeline: Id, before: (&Value, bool), set: Set) -> Option<Op> {
    let (before_json, was_muted) = before;
    let op = match (set, of) {
        (Set::Strip(strip, mute), LanesOf::Track(_) | LanesOf::Bus(_)) => {
            let after = serde_json::to_value(&strip).unwrap_or_default();
            let mut patch = Map::new();
            let strip_patch = differing(before_json, &after);
            if !strip_patch.is_empty() {
                patch.insert("strip".into(), Value::Object(strip_patch));
            }
            if mute != was_muted {
                patch.insert("mute".into(), mute.into());
            }
            if patch.is_empty() {
                return None;
            }
            match of {
                LanesOf::Track(id) => Op::SetTrackAudio {
                    timeline,
                    track: id.clone(),
                    patch: Value::Object(patch),
                },
                _ => Op::SetBus {
                    timeline,
                    id: of_id(of).clone(),
                    patch: Value::Object(patch),
                },
            }
        }
        (Set::Vca(vca), LanesOf::Vca(id)) => {
            let after = serde_json::to_value(&vca).unwrap_or_default();
            let patch = differing(before_json, &after);
            if patch.is_empty() {
                return None;
            }
            Op::SetVca {
                timeline,
                id: id.clone(),
                patch: Value::Object(patch),
            }
        }
        _ => return None,
    };
    Some(op)
}

fn of_id(of: &LanesOf) -> &Id {
    match of {
        LanesOf::Track(id) | LanesOf::Bus(id) | LanesOf::Vca(id) => id,
    }
}

// Inserts and the lanes that name them.

/// The owner of the lanes of a chain of inserts: a track or a bus; a clip has no lanes.
fn lanes_of_chain(of: &InsertsOf) -> Option<LanesOf> {
    match of {
        InsertsOf::Track(id) => Some(LanesOf::Track(id.clone())),
        InsertsOf::Bus(id) => Some(LanesOf::Bus(id.clone())),
        InsertsOf::Clip(_) => None,
    }
}

/// Take away the lanes of `of` whose paths `stale` picks.
fn take_lanes_away(
    work: &mut Work,
    of: &LanesOf,
    stale: impl Fn(&str, &Lane) -> bool,
) -> Result<(), EditError> {
    let (_, lanes) = owner_of(work.timeline(), of)?;
    let paths: Vec<String> = lanes
        .iter()
        .filter(|(path, lane)| stale(path, lane))
        .map(|(path, _)| path.clone())
        .collect();
    let timeline = work.timeline_id();
    for path in paths {
        work.op(Op::SetLane {
            timeline: timeline.clone(),
            of: of.clone(),
            path,
            lane: None,
        })?;
    }
    Ok(())
}

/// Take away the lanes of the insert `id` of `of` (`insert.<id>.…`).
fn take_insert_lanes_away(work: &mut Work, of: &LanesOf, id: &str) -> Result<(), EditError> {
    let prefix = format!("insert.{id}.");
    take_lanes_away(work, of, |path, _| path.starts_with(&prefix))
}

/// Take away every lane of an insert of `of` (`insert.…`), before its inserts are put in place
/// of by copies, which a strip gives ids of its own that may be those of the inserts gone.
pub(super) fn take_all_insert_lanes_away(work: &mut Work, of: &InsertsOf) -> Result<(), EditError> {
    match lanes_of_chain(of) {
        Some(of) => take_lanes_away(work, &of, |path, _| path.starts_with("insert.")),
        None => Ok(()),
    }
}

/// The project of `work` with the lanes of `of` taken away: to see what an operation on its
/// strip makes, without the lanes that it would leave naming what is not there.
fn without_lanes(work: &Work, of: &LanesOf) -> Project {
    let mut project = work.project().clone();
    if let Some(timeline) = project
        .timelines
        .iter_mut()
        .find(|t| t.id == work.timeline_id())
    {
        let lanes = match of {
            LanesOf::Track(id) => timeline
                .tracks
                .iter_mut()
                .find(|r| r.id == *id)
                .map(|r| &mut r.audio.lanes),
            LanesOf::Bus(id) => timeline
                .buses
                .iter_mut()
                .find(|b| b.id == *id)
                .map(|b| &mut b.lanes),
            LanesOf::Vca(id) => timeline
                .vcas
                .iter_mut()
                .find(|v| v.id == *id)
                .map(|v| &mut v.lanes),
        };
        if let Some(lanes) = lanes {
            lanes.clear();
        }
    }
    project
}

/// Apply an operation that sets the strip of a track or a bus (`set_track_audio`, `set_bus`),
/// first taking away the lanes that it would leave naming a setting that is not there, or one of
/// another kind: of a send, an output or an insert that it takes away, of a Ducker gone, or of an
/// insert whose id stays with an effect of another kind.
pub(super) fn strip_op(work: &mut Work, op: Op) -> Result<Op, EditError> {
    let of = match &op {
        Op::SetTrackAudio { track, .. } => LanesOf::Track(track.clone()),
        Op::SetBus { id, .. } => LanesOf::Bus(id.clone()),
        _ => return work.op(op),
    };
    let lanes = owner_of(work.timeline(), &of).map(|(owner, lanes)| (owner, lanes.clone()));
    if let (Ok((Owner::Strip { strip, .. }, lanes)), true) = (lanes, structural(&op)) {
        let before = strip.clone();
        let mut trial = without_lanes(work, &of);
        if !lanes.is_empty() && crate::ops::apply(&mut trial, op.clone()).is_ok() {
            let timeline = trial.timelines.iter().find(|t| t.id == work.timeline_id());
            if let Some(Ok((owner @ Owner::Strip { strip: after, .. }, _))) =
                timeline.map(|t| owner_of(t, &of))
            {
                let changed = lanes_of_changed_inserts(&before, after, &lanes);
                let view = view(owner);
                take_lanes_away(work, &of, |path, lane| {
                    kind_of(&view, path) != Some(lane.kind) || changed.iter().any(|p| p == path)
                })?;
            }
        }
    }
    work.op(op)
}

/// Whether an operation on a strip may take a setting away that a lane may be of: a patch of
/// its sends, outputs, inserts or Ducker. Nothing else of a strip comes and goes.
fn structural(op: &Op) -> bool {
    let (Op::SetTrackAudio { patch, .. } | Op::SetBus { patch, .. }) = op else {
        return false;
    };
    patch
        .get("strip")
        .and_then(Value::as_object)
        .is_some_and(|strip| {
            ["sends", "outputs", "inserts", "track_fx"]
                .iter()
                .any(|field| strip.contains_key(*field))
        })
}

/// The next id of an insert of `inserts` that none of them has, nor `also`: `fx` and a number
/// above all theirs.
fn fresh_id(inserts: &[Insert], also: &str) -> Id {
    let highest = inserts
        .iter()
        .map(|i| i.id.as_str())
        .chain([also])
        .filter_map(|id| id.strip_prefix("fx")?.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    format!("fx{}", highest.saturating_add(1))
}

/// Whether a chain of inserts may be changed: a track that is locked, or a clip on one, may not.
fn open_chain(work: &Work, of: &InsertsOf) -> Result<(), EditError> {
    match of {
        InsertsOf::Track(id) => {
            work.open(id)?;
        }
        InsertsOf::Clip(id) => {
            let (track, _) = work.owned(id)?;
            work.open(&track)?;
        }
        InsertsOf::Bus(_) => {}
    }
    Ok(())
}

/// Put an insert in place of the one at `slot` of a chain, or take that one away, as the
/// operation `set_insert` does, and take away the lanes of the one that goes, in the same
/// transaction. An insert of a strip that is put in place of another is a new insert: it is
/// given an id that no insert of the strip has had there, so that no lane of the old one is
/// taken for its.
pub(super) fn set_insert(
    work: &mut Work,
    of: &InsertsOf,
    slot: usize,
    insert: Option<&Insert>,
) -> Result<(), EditError> {
    open_chain(work, of)?;
    let timeline = work.timeline_id();
    let inserts = inserts_of(work.project(), &timeline, of)?.to_vec();
    let mut insert = insert.cloned();
    if let (Some(old), Some(lanes_of)) = (inserts.get(slot), lanes_of_chain(of)) {
        take_insert_lanes_away(work, &lanes_of, &old.id)?;
        if let Some(insert) = &mut insert {
            insert.id = fresh_id(&inserts, &old.id);
        }
    }
    work.op(Op::SetInsert {
        timeline,
        of: of.clone(),
        slot,
        insert,
    })?;
    Ok(())
}

/// Move the insert at `from` of a chain to `to`: within the chain, as the operation `move_insert`
/// does, its id and so its lanes going with it; or to the chain `to_of`, where it is a new
/// insert, at `to`: taken away from the one, its lanes with it, and added to the other with no
/// id, so that the strip gives it the next of its own.
pub(super) fn move_insert(
    work: &mut Work,
    of: &InsertsOf,
    (from, to): (usize, usize),
    to_of: Option<&InsertsOf>,
) -> Result<(), EditError> {
    open_chain(work, of)?;
    let timeline = work.timeline_id();
    let other = to_of.filter(|other| *other != of);
    let Some(other) = other else {
        work.op(Op::MoveInsert {
            timeline,
            of: of.clone(),
            from,
            to,
        })?;
        return Ok(());
    };
    open_chain(work, other)?;
    let inserts = inserts_of(work.project(), &timeline, of)?;
    let len = inserts.len();
    let insert = inserts
        .get(from)
        .cloned()
        .ok_or(OpError::OutOfList { at: from, len })?;
    if let Some(lanes_of) = lanes_of_chain(of) {
        take_insert_lanes_away(work, &lanes_of, &insert.id)?;
    }
    work.op(Op::SetInsert {
        timeline: timeline.clone(),
        of: of.clone(),
        slot: from,
        insert: None,
    })?;
    work.op(Op::AddInsert {
        timeline,
        of: other.clone(),
        insert: Insert {
            id: String::new(),
            ..insert
        },
        index: Some(to),
    })?;
    Ok(())
}
