//! Automation Follows Edit (Reference Manual 20, pp. 3910–3911; reference C-A9): the automation
//! written over a clip of a track of sound "lives with" it, so an edit that moves the clip takes
//! the points over it along, one that takes a section away takes them away, and a ripple moves
//! what is after its place.
//!
//! It is worked out on the steps that every edit of clips is made of ([`super::work::Work`]), not
//! edit by edit, on copies of the lanes of the tracks that they touch, which are written back as
//! one `splice_lane` (or `set_lane`) for each lane that changed once the edit is done:
//!
//! - A clip taken away keeps what its lanes are over it, from its start to its end, and leaves
//!   them there going straight from the value that came to its start to the value at its end, as
//!   a range of a lane taken away does (p. 3898).
//! - The same clip put back, by a move, takes that to where it goes, on its new track if it went
//!   to another: the lanes there between its new ends are put in place of by it, the values
//!   before and after them staying as they were. A lane of the clip's track that the new track
//!   cannot have (an insert or a send that it has not) is not taken; one that it has not yet is
//!   made, from its setting as it is set.
//! - A ripple from a frame moves the points after it with what moves: a ripple that takes frames
//!   away takes their points away too, and the two sides meet with their values (a jump); one
//!   that puts frames in goes straight across them from the value that came to the place to the
//!   value that was there. Where a clip that stays lies across the place, the cut is made at its
//!   end instead, so that its lanes stay with it.
//! - A clip that is trimmed, rolled, slipped or cut in two changes no lane, nor does one that is
//!   put on the timeline from its media. The lanes of buses and VCAs, which have no clips, stay
//!   where they are.
//!
//! A frame `f` is at tick `⌊f · scale / rate⌋` of a lane (as the graph places a clip's first
//! sample), and the points over a clip move by as many ticks as its start does, so that a lane
//! stays with the samples of the clip exactly. Nothing is done while the timeline's
//! `follows_edit` is off.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::audio::automation::{check_lane, fixed_value, view, Owner, Shape};
use crate::audio::{Lane, LanePoint, Lanes};
use crate::ops::{LanesOf, Op};
use crate::project::{Clip, Id, Timeline, Track, TrackKind};
use crate::time::{Rate, Time};

/// The lanes that an edit changes, as it goes.
pub(super) struct Follow {
    /// Whether the timeline's automation follows its edits.
    on: bool,
    /// The lanes of the tracks touched so far, as they are now.
    lanes: BTreeMap<Id, Lanes>,
    /// What the lanes of each clip taken away were over it, to go where it is put back.
    carried: HashMap<Id, Vec<Piece>>,
}

/// What a lane was over a clip: its value at the clip's start, the points within it and the value
/// that came to its end, in ticks of the lane.
struct Piece {
    path: String,
    shape: Shape,
    scale: u32,
    from: i64,
    to: i64,
    first: f64,
    inner: Vec<LanePoint>,
    last: f64,
    /// The value at its last tick, where the clip has one before its end.
    near_end: Option<f64>,
}

/// The tick of `scale` that frame `frame` of `rate` starts on.
fn tick(rate: Rate, frame: i64, scale: u32) -> i64 {
    rate.ticks_of(frame, scale)
}

fn owner(track: &Track) -> Owner<'_> {
    Owner::Strip {
        strip: &track.audio.strip,
        mute: track.audio.mute,
        bus: false,
    }
}

fn sound<'t>(timeline: &'t Timeline, id: &str) -> Option<&'t Track> {
    timeline
        .tracks
        .iter()
        .find(|r| r.id == id && r.kind == TrackKind::Audio)
}

/// `value` within the least and the greatest values of `points`: a value that a lane comes to
/// between two of its points is between theirs, but for the rounding of the sum, which this takes
/// away, so that no edit gives a lane a value beyond those it had.
fn bounded(points: &[LanePoint], value: f64) -> f64 {
    let (least, most) = points
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
            (lo.min(p.value), hi.max(p.value))
        });
    if least <= most {
        value.clamp(least, most)
    } else {
        value
    }
}

/// The value at tick `at`, the later of two points there holding.
fn at(points: &[LanePoint], shape: Shape, fixed: f64, at: i64) -> f64 {
    bounded(
        points,
        crate::audio::automation::value_at(points, shape, fixed, at),
    )
}

/// The value that comes to tick `at`, before a point there holds.
fn before(points: &[LanePoint], shape: Shape, fixed: f64, at: i64) -> f64 {
    bounded(
        points,
        crate::audio::automation::value_before(points, shape, fixed, at),
    )
}

fn point(ticks: i64, scale: u32, value: f64) -> LanePoint {
    LanePoint {
        at: Time { ticks, scale },
        value,
    }
}

/// Points without one that is at the time and of the value of the one before it.
fn without_twins(points: Vec<LanePoint>) -> Vec<LanePoint> {
    let mut kept: Vec<LanePoint> = Vec::with_capacity(points.len());
    for p in points {
        let twin = kept
            .last()
            .is_some_and(|q| q.at.ticks == p.at.ticks && q.value.to_bits() == p.value.to_bits());
        if !twin {
            kept.push(p);
        }
    }
    kept
}

/// The points of a lane with those from tick `from` to tick `to`, both in, put in place of by
/// `new` (from `from` to `to`): a point at `from` of the value that came to it and one at `to` of
/// the value that was there keep the lane as it was outside them.
fn spliced(
    points: &[LanePoint],
    (shape, fixed, scale): (Shape, f64, u32),
    (from, to): (i64, i64),
    new: &[LanePoint],
) -> Vec<LanePoint> {
    let left = before(points, shape, fixed, from);
    let right = at(points, shape, fixed, to);
    let all = points
        .iter()
        .filter(|p| p.at.ticks < from)
        .cloned()
        .chain([point(from, scale, left)])
        .chain(new.iter().cloned())
        .chain([point(to, scale, right)])
        .chain(points.iter().filter(|p| p.at.ticks > to).cloned())
        .collect();
    without_twins(all)
}

/// The points of a lane rippled from tick `place` to tick `to`: those after `place` moved by
/// `moved` (each to its new tick), those between `to` and `place` taken away where `to` is
/// before it, and the two sides meeting at `to`; or, where `to` is after it, the lane going
/// straight across from the one to the other. None where no point is so late as to change.
fn rippled(
    points: &[LanePoint],
    (shape, fixed, scale): (Shape, f64, u32),
    (place, to): (i64, i64),
    moved: impl Fn(i64) -> i64,
) -> Option<Vec<LanePoint>> {
    if !points.iter().any(|p| p.at.ticks >= place.min(to)) {
        return None;
    }
    let came = before(points, shape, fixed, to.min(place));
    let was = at(points, shape, fixed, place);
    let kept = points
        .iter()
        .filter(|p| p.at.ticks < to.min(place))
        .cloned();
    let after = points
        .iter()
        .filter(|p| p.at.ticks > place)
        .map(|p| point(moved(p.at.ticks), scale, p.value));
    let ends = if to < place {
        [point(to, scale, came), point(to, scale, was)]
    } else {
        [point(place, scale, came), point(to, scale, was)]
    };
    Some(without_twins(kept.chain(ends).chain(after).collect()))
}

/// The points of a lane with one at each of `ticks` that is between its first and its last and
/// has none, of the value there, so that what is between two of them goes as it went whatever
/// moves outside them.
fn pinned(
    points: &[LanePoint],
    (shape, fixed, scale): (Shape, f64, u32),
    ticks: impl Iterator<Item = i64>,
) -> Vec<LanePoint> {
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return points.to_vec();
    };
    let mut all = points.to_vec();
    for t in ticks {
        let i = all.partition_point(|p| p.at.ticks < t);
        let there = all.get(i).is_some_and(|p| p.at.ticks == t);
        if first.at.ticks < t && t < last.at.ticks && !there {
            all.insert(i, point(t, scale, at(points, shape, fixed, t)));
        }
    }
    all
}

impl Follow {
    pub fn new(timeline: &Timeline) -> Self {
        Self {
            on: timeline.audio.follows_edit,
            lanes: BTreeMap::new(),
            carried: HashMap::new(),
        }
    }

    /// The lanes of a track of sound as the edit has left them so far.
    fn lanes_of(&mut self, timeline: &Timeline, track: &str) -> Option<&mut Lanes> {
        let found = sound(timeline, track)?;
        Some(
            self.lanes
                .entry(track.into())
                .or_insert_with(|| found.audio.lanes.clone()),
        )
    }

    /// A clip of `track` is taken away: keep what its lanes are over it, and leave them going
    /// straight between the values at its ends.
    pub fn take(&mut self, timeline: &Timeline, track: &str, clip: &Clip) {
        if !self.on {
            return;
        }
        let Some(found) = sound(timeline, track) else {
            return;
        };
        let settings = view(owner(found));
        let rate = timeline.rate;
        let Some(lanes) = self.lanes_of(timeline, track) else {
            return;
        };
        let mut pieces = vec![];
        for (path, lane) in lanes.iter_mut() {
            let shape = Shape::of(lane);
            let scale = lane.points[0].at.scale;
            let fixed = fixed_value(&settings, path).unwrap_or(lane.points[0].value);
            let (from, to) = (tick(rate, clip.start, scale), tick(rate, clip.end(), scale));
            let points = &lane.points;
            pieces.push(Piece {
                path: path.clone(),
                shape,
                scale,
                from,
                to,
                first: at(points, shape, fixed, from),
                inner: points
                    .iter()
                    .filter(|p| from < p.at.ticks && p.at.ticks < to)
                    .cloned()
                    .collect(),
                last: before(points, shape, fixed, to),
                near_end: (to - 1 > from).then(|| at(points, shape, fixed, to - 1)),
            });
            if points
                .iter()
                .any(|p| from <= p.at.ticks && p.at.ticks <= to)
            {
                lane.points = spliced(points, (shape, fixed, scale), (from, to), &[]);
            }
        }
        self.carried.insert(clip.id.clone(), pieces);
    }

    /// A clip taken away is put on `track` as `clip`: what its lanes were over it goes where it
    /// is now.
    pub fn put(&mut self, timeline: &Timeline, track: &str, clip: &Clip) {
        let Some(pieces) = self.carried.remove(&clip.id) else {
            return;
        };
        let Some(found) = sound(timeline, track) else {
            return;
        };
        let (owner, rate) = (owner(found), timeline.rate);
        let settings = view(owner);
        let Some(lanes) = self.lanes_of(timeline, track) else {
            return;
        };
        for piece in pieces {
            let Some(fixed) = fixed_value(&settings, &piece.path) else {
                continue;
            };
            let (old, shape, scale) = match lanes.get(&piece.path) {
                Some(lane) => (
                    lane.points.as_slice(),
                    Shape::of(lane),
                    lane.points[0].at.scale,
                ),
                // A lane that the track cannot have is not made: see `check_lane` below.
                None => (&[][..], piece.shape, piece.scale),
            };
            let start = tick(rate, clip.start, scale);
            // A tick of the piece's scale on the lane, as far from the clip's start as it was.
            let place = |ticks: i64| {
                let (num, den) = (
                    i128::from(ticks - piece.from) * i128::from(scale),
                    i128::from(piece.scale),
                );
                let near = (2 * num + den).div_euclid(2 * den);
                start + i64::try_from(near).unwrap_or(i64::MAX - start)
            };
            // As far as the clip goes now: where its frames are not whole ticks, the piece may
            // come to a tick more than it, which is the first of what follows it.
            // There, its last tick is held by a point of the value that it had, so that what is
            // before it goes as it went.
            let end = place(piece.to);
            let short = tick(rate, clip.end(), scale).max(start);
            let mut new: Vec<LanePoint> = [point(start, scale, piece.first)]
                .into_iter()
                .chain(
                    piece
                        .inner
                        .iter()
                        .map(|p| point(place(p.at.ticks), scale, p.value)),
                )
                .collect();
            let end = match piece.near_end {
                Some(value) if end > short => {
                    let last = place(piece.to - 1);
                    if new.last().is_none_or(|p| p.at.ticks < last) {
                        new.push(point(last, scale, value));
                    }
                    short
                }
                _ => end.min(short),
            };
            new.push(point(end, scale, piece.last));
            let lane = Lane {
                points: spliced(old, (shape, fixed, scale), (start, end), &new),
                kind: shape.kind,
                scale: Some(shape.scale),
            };
            if check_lane(&piece.path, &lane, owner, &settings).is_ok() {
                lanes.insert(piece.path, lane);
            }
        }
    }

    /// What is on `track` from frame `from` on, but for `skip`, moves by `by` frames: its lanes
    /// move with it. The cut is made after a clip that stays across `from`, and no later than the
    /// first clip that moves.
    pub fn ripple(
        &mut self,
        timeline: &Timeline,
        track: &str,
        (from, by): (i64, i64),
        skip: &HashSet<Id>,
    ) {
        if !self.on || by == 0 {
            return;
        }
        let Some(found) = sound(timeline, track) else {
            return;
        };
        if found.audio.lanes.is_empty() && !self.lanes.contains_key(track) {
            return;
        }
        let settings = view(owner(found));
        let rate = timeline.rate;
        let moves = |c: &&Clip| c.start >= from && !skip.contains(&c.id);
        let mut spans: Vec<(i64, i64)> = found
            .clips
            .iter()
            .filter(moves)
            .map(|c| (c.start, c.end()))
            .collect();
        spans.sort_unstable();
        let stays = found.clips.iter().filter(|c| c.start < from && !moves(c));
        let mut place = stays
            .map(|c| if by > 0 { c.end() } else { c.end() - by })
            .fold(from, i64::max);
        if let Some(&(first, _)) = spans.first() {
            place = place.min(first);
        }
        let Some(lanes) = self.lanes_of(timeline, track) else {
            return;
        };
        for (path, lane) in lanes.iter_mut() {
            let shape = Shape::of(lane);
            let scale = lane.points[0].at.scale;
            let fixed = fixed_value(&settings, path).unwrap_or(lane.points[0].value);
            let ticks = |frame| tick(rate, frame, scale);
            let (p, to) = (ticks(place), ticks(place + by));
            // Each clip that moves moves by as many ticks as its start does; what is after it,
            // up to the next, as far as it.
            let steps: Vec<(i64, i64)> = spans
                .iter()
                .map(|&(s, _)| (ticks(s), ticks(s + by) - ticks(s)))
                .collect();
            // Where they do not all move as far, a line from a point before a clip to one over it
            // would be stretched or shrunk by a tick: the first and the last tick of each are held
            // by points first.
            if steps.iter().any(|&(_, d)| d != to - p) {
                let ends = spans.iter().flat_map(|&(s, e)| [ticks(s), ticks(e) - 1]);
                lane.points = pinned(&lane.points, (shape, fixed, scale), ends);
            }
            let tick_after = |t: i64| {
                let i = steps.partition_point(|&(s, _)| s <= t);
                t + if i == 0 { to - p } else { steps[i - 1].1 }
            };
            if let Some(points) = rippled(&lane.points, (shape, fixed, scale), (p, to), tick_after)
            {
                lane.points = points;
            }
        }
    }

    /// The operations that put the lanes as the edit left them in place of those of the project.
    pub fn ops(self, timeline: &Timeline) -> Vec<Op> {
        let mut ops = vec![];
        for (id, lanes) in self.lanes {
            let Some(track) = sound(timeline, &id) else {
                continue;
            };
            let old = &track.audio.lanes;
            let paths: BTreeSet<&String> = old.keys().chain(lanes.keys()).collect();
            for path in paths {
                let of = LanesOf::Track(id.clone());
                let set = |lane: Option<Lane>| Op::SetLane {
                    timeline: timeline.id.clone(),
                    of: of.clone(),
                    path: path.clone(),
                    lane,
                };
                match (old.get(path), lanes.get(path)) {
                    (Some(a), Some(b)) if a == b => {}
                    (Some(a), Some(b)) if (a.kind, a.scale) == (b.kind, b.scale) => {
                        ops.push(splice(timeline, of.clone(), path, &a.points, &b.points));
                    }
                    (_, b) => ops.push(set(b.cloned())),
                }
            }
        }
        ops
    }
}

/// One `splice_lane` of the points between the first and the last that differ.
fn splice(
    timeline: &Timeline,
    of: LanesOf,
    path: &str,
    old: &[LanePoint],
    new: &[LanePoint],
) -> Op {
    let same =
        |a: &LanePoint, b: &LanePoint| a.at == b.at && a.value.to_bits() == b.value.to_bits();
    let most = old.len().min(new.len());
    let head = (0..most).take_while(|&i| same(&old[i], &new[i])).count();
    let tail = (0..most - head)
        .take_while(|&i| same(&old[old.len() - 1 - i], &new[new.len() - 1 - i]))
        .count();
    Op::SpliceLane {
        timeline: timeline.id.clone(),
        of,
        path: path.into(),
        at: head,
        remove: old.len() - head - tail,
        points: new[head..new.len() - tail].to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{LaneKind, LaneScale};

    const LINE: Shape = Shape {
        kind: LaneKind::Continuous,
        scale: LaneScale::Linear,
    };

    fn pts(list: &[(i64, f64)]) -> Vec<LanePoint> {
        list.iter().map(|&(t, v)| point(t, 10, v)).collect()
    }

    fn plain(points: &[LanePoint]) -> Vec<(i64, f64)> {
        points.iter().map(|p| (p.at.ticks, p.value)).collect()
    }

    #[test]
    fn a_ripple_that_takes_ticks_away_joins_the_values_at_its_ends() {
        let lane = pts(&[(0, 0.0), (20, 10.0), (40, 0.0)]);
        let done = rippled(&lane, (LINE, 0.0, 10), (30, 10), |t| t - 20).unwrap();
        assert_eq!(plain(&done), vec![(0, 0.0), (10, 5.0), (20, 0.0)]);
    }

    #[test]
    fn a_ripple_that_puts_ticks_in_goes_straight_across_them() {
        let lane = pts(&[(0, 0.0), (20, 10.0), (40, 0.0)]);
        let done = rippled(&lane, (LINE, 0.0, 10), (10, 30), |t| t + 20).unwrap();
        assert_eq!(
            plain(&done),
            vec![(0, 0.0), (10, 5.0), (30, 5.0), (40, 10.0), (60, 0.0)]
        );
        // Nothing at or after the place: nothing changes.
        assert!(rippled(&lane, (LINE, 0.0, 10), (50, 70), |t| t + 20).is_none());
    }

    #[test]
    fn a_value_between_two_points_is_never_beyond_them() {
        // 0.2 + (-0.9 - 0.2) · 1 is -0.9000000000000001 in doubles.
        let lane = vec![point(0, 3, 0.2), point(1, 3, -0.9)];
        for t in 0..=1 {
            for v in [at(&lane, LINE, 0.0, t), before(&lane, LINE, 0.0, t)] {
                assert!((-0.9..=0.2).contains(&v), "{v}");
            }
        }
    }
}
