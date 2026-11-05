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
    point, range_of, replaced, shifted, switched, thinned, ticks_in, tolerance_of, value_at, view,
    with_value, Owner, Set, Shape, LANE_RAMP_PER_SECOND,
};
use crate::audio::{Insert, Lane, LaneKind, LanePoint, LaneScale, Lanes};
use crate::ops::{inserts_of, owner_of, InsertsOf, LanesOf, Op, OpError, LARGEST};
use crate::project::{Id, Project};
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
            replaced(&target.points, shape, fixed, scale, (from, to), &new)
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
        // Thinned as recorded (of a Trim, the offsets), the last value held to the end.
        let mut new = thinned(&values, if recorded.trim { line } else { shape }, tolerance);
        if last.at.ticks < to {
            new.push(point(to, scale, last.value));
        }
        // It comes in over the ramp: its first value where the ramp ends, unless the next is
        // there already (of a Trim, from an offset of 0).
        if ramp > 0 && to > from {
            let ramped = (from + ramp).min(to);
            if new.get(1).is_some_and(|next| next.at.ticks <= ramped) {
                new.remove(0);
            } else {
                new[0].at.ticks = ramped;
            }
            if recorded.trim {
                new.insert(0, point(from, scale, 0.0));
            }
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
