//! Automation Follows Edit (Reference Manual 20, pp. 3910–3911; reference C-A9): the automation
//! written over a clip of a track of sound "lives with" it, so an edit that moves the clip takes
//! the points over it along, one that takes a section away takes them away, a ripple moves what
//! is after its place, and a copy of a clip takes a copy of them.
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
//!   made, from its setting as it is set. A copy of a clip takes a copy of them in the same way.
//! - A ripple from a frame moves the points after it with what moves: a ripple that takes frames
//!   away takes their points away too, and the two sides meet with their values (a jump); one
//!   that puts frames in goes straight across them from the value that came to the place to the
//!   value that was there. Where a clip that stays lies across the place, or starts after it, the
//!   cut is made at its end instead, so that its lanes stay with it. A clip trimmed at its start
//!   with a ripple has its lanes rippled from its start (see [`Follow::ripple_with`]). A clip of
//!   the group of a ripple trim that starts after the place of the ripple stays where it starts
//!   while what is between the place and it moves: the lanes there move with that, as far as the
//!   clip only (see [`Follow::ripple_before`]), and the clip's own go as the trim changes it.
//! - A clip that is trimmed (without a ripple), rolled, slipped or cut in two changes no lane, nor
//!   does one that is put on the timeline from its media. The lanes of buses and VCAs, which
//!   have no clips, stay where they are.
//!
//! A frame `f` is at tick `⌊f · scale / rate⌋` of a lane (as the graph places a clip's first
//! sample), and the points over a clip move by as many ticks as its start does, so that a lane
//! stays with the samples of the clip exactly. A point that an edit adds and that lies on the
//! straight line between the points beside it is left out. Nothing is done while the timeline's
//! `follows_edit` is off.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::audio::automation::{
    bounded, check_lane, fixed_value, point, replaced, value_at, value_before, view, without_twins,
    Owner, Shape,
};
use crate::audio::{Lane, LaneKind, LanePoint, Lanes};
use crate::ops::{LanesOf, Op};
use crate::project::{Clip, Id, Timeline, Track, TrackKind};
use crate::time::Rate;

/// The lanes that an edit changes, as it goes.
pub(super) struct Follow {
    /// Whether the timeline's automation follows its edits.
    on: bool,
    /// The lanes of the tracks touched so far, as they are now.
    lanes: BTreeMap<Id, Lanes>,
    /// What the lanes of each clip taken away were over it, to go where it is put back.
    carried: HashMap<Id, Vec<Piece>>,
    /// What the lanes of each clip copied are over it, to go where its copies are put.
    copied: HashMap<Id, Vec<Piece>>,
    /// The tracks whose lanes the edit has rippled itself, which a ripple leaves alone.
    held: HashSet<Id>,
}

/// What a lane was over a clip: its value at the clip's start, the points within it and the value
/// that came to its end, in ticks of the lane.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Piece {
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

impl Piece {
    pub(super) fn check_snapshot(&self) -> Result<(), crate::ops::OpError> {
        let bad = || crate::ops::OpError::OutOfRange {
            what: "clipboard_lane",
        };
        if self.scale == 0
            || self.from < 0
            || self.to <= self.from
            || !self.first.is_finite()
            || !self.last.is_finite()
            || self.near_end.is_some_and(|value| !value.is_finite())
        {
            return Err(bad());
        }
        let mut previous = self.from;
        for point in &self.inner {
            if point.at.scale != self.scale
                || point.at.ticks <= self.from
                || point.at.ticks >= self.to
                || point.at.ticks < previous
                || !point.value.is_finite()
            {
                return Err(bad());
            }
            previous = point.at.ticks;
        }
        Ok(())
    }
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

/// The value at tick `at`, the later of two points there holding, within the values of the lane.
fn at(points: &[LanePoint], shape: Shape, fixed: f64, at: i64) -> f64 {
    bounded(points, value_at(points, shape, fixed, at))
}

/// The value that comes to tick `at`, before a point there holds, within the values of the lane.
fn before(points: &[LanePoint], shape: Shape, fixed: f64, at: i64) -> f64 {
    bounded(points, value_before(points, shape, fixed, at))
}

/// The points of a lane without those at ticks `added` that it does not need: one alone at its
/// tick, between two at other ticks, of the value that the straight line between them has there
/// (of a switch, the value of the one before it), to the bit.
fn pruned(mut points: Vec<LanePoint>, shape: Shape, added: &[i64]) -> Vec<LanePoint> {
    let mut i = 1;
    while i + 1 < points.len() {
        let (prev, p, next) = (&points[i - 1], &points[i], &points[i + 1]);
        let t = p.at.ticks;
        let alone = prev.at.ticks < t && t < next.at.ticks;
        let on_line = match shape.kind {
            LaneKind::Switch => p.value.to_bits() == prev.value.to_bits(),
            LaneKind::Continuous => {
                let line = value_at(&[prev.clone(), next.clone()], shape, p.value, t);
                p.value.to_bits() == line.to_bits()
            }
        };
        if alone && on_line && added.contains(&t) {
            points.remove(i);
        } else {
            i += 1;
        }
    }
    points
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

/// The ticks about `boundaries` at which a lane needs a point of the value there before it is
/// rippled, so that a line from before a boundary to after it, which the ripple stretches or
/// shrinks by a tick, changes nothing on either side of it: the tick before the boundary and the
/// boundary itself, where a sloped line crosses there with no point at it.
fn pins(points: &[LanePoint], shape: Shape, boundaries: &[i64]) -> Vec<i64> {
    if shape.kind == LaneKind::Switch {
        return vec![];
    }
    let mut ticks = vec![];
    for &b in boundaries {
        for t in [b - 1, b] {
            let i = points.partition_point(|p| p.at.ticks < t);
            let there = points.get(i).is_some_and(|p| p.at.ticks == t);
            let (Some(p), Some(q)) = (i.checked_sub(1).map(|i| &points[i]), points.get(i)) else {
                continue;
            };
            let sloped = p.value.to_bits() != q.value.to_bits();
            if !there && sloped && p.at.ticks < b && b <= q.at.ticks && !ticks.contains(&t) {
                ticks.push(t);
            }
        }
    }
    ticks.sort_unstable();
    ticks
}

impl Follow {
    pub fn capture(timeline: &Timeline, track: &str, clip: &Clip) -> Vec<Piece> {
        let mut follow = Self::new(timeline);
        if follow.on {
            follow.pieces(timeline, track, clip, false)
        } else {
            vec![]
        }
    }

    pub fn keep_snapshot(&mut self, key: &str, pieces: &[Piece]) {
        if self.on {
            self.copied.insert(key.into(), pieces.to_vec());
        }
    }

    pub fn new(timeline: &Timeline) -> Self {
        Self {
            on: timeline.audio.follows_edit,
            lanes: BTreeMap::new(),
            carried: HashMap::new(),
            copied: HashMap::new(),
            held: HashSet::new(),
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

    /// What the lanes of `track` are over `clip`, and, with `clear`, leave them going straight
    /// between the values at its ends.
    fn pieces(&mut self, timeline: &Timeline, track: &str, clip: &Clip, clear: bool) -> Vec<Piece> {
        let Some(found) = sound(timeline, track) else {
            return vec![];
        };
        let settings = view(owner(found));
        let rate = timeline.rate;
        let Some(lanes) = self.lanes_of(timeline, track) else {
            return vec![];
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
            let over = points
                .iter()
                .any(|p| from <= p.at.ticks && p.at.ticks <= to);
            if clear && over {
                let cleared = replaced(points, shape, fixed, scale, (from, to), &[]);
                lane.points = pruned(cleared, shape, &[from, to]);
            }
        }
        pieces
    }

    /// A clip of `track` is taken away: keep what its lanes are over it, and leave them going
    /// straight between the values at its ends.
    pub fn take(&mut self, timeline: &Timeline, track: &str, clip: &Clip) {
        if self.on {
            let pieces = self.pieces(timeline, track, clip, true);
            self.carried.insert(clip.id.clone(), pieces);
        }
    }

    /// A clip of `track` is copied: keep what its lanes are over it for its copies.
    pub fn keep(&mut self, timeline: &Timeline, track: &str, clip: &Clip) {
        if self.on && !self.copied.contains_key(&clip.id) {
            let pieces = self.pieces(timeline, track, clip, false);
            self.copied.insert(clip.id.clone(), pieces);
        }
    }

    /// A clip taken away is put on `track` as `clip`: what its lanes were over it goes where it
    /// is now.
    pub fn put(&mut self, timeline: &Timeline, track: &str, clip: &Clip) {
        if let Some(pieces) = self.carried.remove(&clip.id) {
            self.paste(timeline, track, clip, pieces);
        }
    }

    /// A copy of `source` is put on `track` as `clip`: a copy of what the lanes of `source` were
    /// over it goes where the copy is.
    pub fn put_copy(&mut self, timeline: &Timeline, track: &str, clip: &Clip, source: &str) {
        if let Some(pieces) = self.copied.get(source).cloned() {
            self.paste(timeline, track, clip, pieces);
        }
    }

    fn paste(&mut self, timeline: &Timeline, track: &str, clip: &Clip, pieces: Vec<Piece>) {
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
                i64::try_from(i128::from(start) + near).unwrap_or(i64::MAX)
            };
            let mut new: Vec<LanePoint> = [point(start, scale, piece.first)]
                .into_iter()
                .chain(
                    piece
                        .inner
                        .iter()
                        .map(|p| point(place(p.at.ticks), scale, p.value)),
                )
                .collect();
            // As far as the clip goes now: where its frames are not whole ticks, the piece may
            // come to a tick more than it, the first of what follows it. There, the clip's last
            // tick is held by a point of the value that it had, so that what is before it goes as
            // it went, and what follows takes its own value from that tick.
            let (end, short) = (place(piece.to), tick(rate, clip.end(), scale).max(start));
            let end = match piece.near_end {
                Some(value) if end > short => {
                    if new.last().is_none_or(|p| p.at.ticks < short) {
                        new.push(point(short, scale, value));
                    }
                    short
                }
                _ => {
                    let end = end.min(short);
                    new.push(point(end, scale, piece.last));
                    end
                }
            };
            let points = replaced(old, shape, fixed, scale, (start, end), &new);
            let lane = Lane {
                points: pruned(points, shape, &[start, end]),
                kind: shape.kind,
                scale: Some(shape.scale),
            };
            if check_lane(&piece.path, &lane, owner, &settings).is_ok() {
                lanes.insert(piece.path, lane);
            }
        }
    }

    /// The lanes of `track` are rippled by the edit itself (see [`Follow::ripple_with`]): a
    /// ripple of the track leaves them alone.
    pub fn hold(&mut self, track: &str) {
        self.held.insert(track.into());
    }

    /// What is on `track` from frame `from` on, but for `skip`, moves by `by` frames: its lanes
    /// move with it. The cut is made after a clip that does not move and starts before the first
    /// that does, and no later than that.
    pub fn ripple(
        &mut self,
        timeline: &Timeline,
        track: &str,
        (from, by): (i64, i64),
        skip: &HashSet<Id>,
    ) {
        if !self.on || by == 0 || self.held.contains(track) {
            return;
        }
        let Some(found) = sound(timeline, track) else {
            return;
        };
        let (place, spans) = Self::ripple_place(found, (from, by), i64::MAX, skip);
        self.ripple_lanes(timeline, track, (place, by), &spans, None);
    }

    /// Where [`Follow::ripple`] cuts the lanes of `track` for what starts from frame `from` on
    /// and before frame `until`, but for `skip`, moving by `by` frames; and the spans of what
    /// moves, in order.
    fn ripple_place(
        found: &Track,
        (from, by): (i64, i64),
        until: i64,
        skip: &HashSet<Id>,
    ) -> (i64, Vec<(i64, i64)>) {
        let moves = |c: &&Clip| c.start >= from && c.start < until && !skip.contains(&c.id);
        let mut spans: Vec<(i64, i64)> = found
            .clips
            .iter()
            .filter(moves)
            .map(|c| (c.start, c.end()))
            .collect();
        spans.sort_unstable();
        let first = spans.first().map_or(until, |&(start, _)| start);
        let stays = found.clips.iter().filter(|c| !moves(c) && c.start < first);
        let place = stays
            .map(|c| if by > 0 { c.end() } else { c.end() - by })
            .fold(from, i64::max)
            .min(first);
        (place, spans)
    }

    /// What is on `track` from frame `from` on and before frame `until`, but for `skip`, moves
    /// by `by` frames, while what is after it is where the edit has put it (a clip of a group
    /// that a ripple trim changes, which starts after the place of the ripple): the lanes before
    /// `until` move with it as [`Follow::ripple`] moves them, as far as frame `resume`, where the
    /// lanes as the edit left them go on; from where they come to, they go straight to the value
    /// there, or, moved past it, are cut there. This is done whether the track is held or not.
    pub fn ripple_before(
        &mut self,
        timeline: &Timeline,
        track: &str,
        (from, by): (i64, i64),
        (until, resume): (i64, i64),
        skip: &HashSet<Id>,
    ) {
        if !self.on || by == 0 || from >= until {
            return;
        }
        let Some(found) = sound(timeline, track) else {
            return;
        };
        let (place, spans) = Self::ripple_place(found, (from, by), until, skip);
        if place < until {
            let bound = Some((until, resume.max(until)));
            self.ripple_lanes(timeline, track, (place, by), &spans, bound);
        }
    }

    /// The lanes of `track` rippled at frame `place` by `by` frames, `spans` (in order) moving
    /// with them: each by as many ticks as its start, and what is after it, up to the next, as
    /// far as it.
    pub fn ripple_with(
        &mut self,
        timeline: &Timeline,
        track: &str,
        (place, by): (i64, i64),
        spans: &[(i64, i64)],
    ) {
        self.ripple_lanes(timeline, track, (place, by), spans, None);
    }

    /// [`Follow::ripple_with`], and, with `(until, resume)`, only for what is before frame
    /// `until`: what the lanes are from frame `resume` on stays, and what moved goes straight
    /// from where it ends to the value there (a jump, where it moved as far or past it).
    fn ripple_lanes(
        &mut self,
        timeline: &Timeline,
        track: &str,
        (place, by): (i64, i64),
        spans: &[(i64, i64)],
        bound: Option<(i64, i64)>,
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
        let Some(lanes) = self.lanes_of(timeline, track) else {
            return;
        };
        for (path, lane) in lanes.iter_mut() {
            let shape = Shape::of(lane);
            let scale = lane.points[0].at.scale;
            let fixed = fixed_value(&settings, path).unwrap_or(lane.points[0].value);
            let ticks = |frame| tick(rate, frame, scale);
            let (p, to) = (ticks(place), ticks(place + by));
            let steps: Vec<(i64, i64)> = spans
                .iter()
                .map(|&(s, _)| (ticks(s), ticks(s + by) - ticks(s)))
                .collect();
            let tick_after = |t: i64| {
                let i = steps.partition_point(|&(s, _)| s <= t);
                t + if i == 0 { to - p } else { steps[i - 1].1 }
            };
            // Where two pieces next to each other do not move as far, a line across the start of
            // the second would be stretched or shrunk by a tick: it is held there first.
            let mut moving = to - p;
            let mut boundaries = vec![];
            for &(s, d) in &steps {
                if d != moving && s > p {
                    boundaries.push(s);
                }
                moving = d;
            }
            let held = pins(&lane.points, shape, &boundaries);
            let mut points = lane.points.clone();
            for &t in &held {
                let i = points.partition_point(|q| q.at.ticks < t);
                points.insert(i, point(t, scale, at(&lane.points, shape, fixed, t)));
            }
            if let Some(done) = rippled(&points, (shape, fixed, scale), (p, to), tick_after) {
                let mut added: Vec<i64> = held.iter().map(|&t| tick_after(t)).collect();
                added.extend([p, to]);
                let done = match bound {
                    None => done,
                    Some((until, resume)) => {
                        // What moved ends where the tick of `until` went, or at `resume`; from
                        // `resume` on the lane is as it was.
                        let r = ticks(resume);
                        let a = tick_after(ticks(until)).min(r);
                        added.extend([a, r]);
                        let ends = [
                            point(a, scale, before(&done, shape, fixed, a)),
                            point(r, scale, at(&lane.points, shape, fixed, r)),
                        ];
                        let moved = done.iter().filter(|q| q.at.ticks < a).cloned();
                        let kept = lane.points.iter().filter(|q| q.at.ticks > r).cloned();
                        without_twins(moved.chain(ends).chain(kept).collect())
                    }
                };
                lane.points = pruned(done, shape, &added);
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
    use crate::audio::LaneScale;

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
        // A jump where the two sides meet.
        let done = rippled(&lane, (LINE, 0.0, 10), (20, 10), |t| t - 10).unwrap();
        assert_eq!(
            plain(&done),
            vec![(0, 0.0), (10, 5.0), (10, 10.0), (30, 0.0)]
        );
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

    #[test]
    fn a_range_replaced_keeps_the_values_at_its_ends_within_those_of_the_lane() {
        let lane = vec![point(0, 3, 0.2), point(1, 3, -0.9), point(2, 3, 0.0)];
        let done = replaced(&lane, LINE, 0.0, 3, (1, 2), &[]);
        assert!(
            done.iter().all(|p| (-0.9..=0.2).contains(&p.value)),
            "{done:?}"
        );
    }

    #[test]
    fn a_point_added_on_the_line_between_its_neighbours_is_left_out_and_others_kept() {
        let lane = pts(&[
            (0, 0.0),
            (10, 5.0),
            (20, 10.0),
            (30, 10.0),
            (30, 0.0),
            (40, 7.0),
        ]);
        // 10 is on the line and was added; 20 is not on it; 30 is a jump.
        let kept = pruned(lane.clone(), LINE, &[10, 20, 30]);
        assert_eq!(
            plain(&kept),
            vec![(0, 0.0), (20, 10.0), (30, 10.0), (30, 0.0), (40, 7.0)]
        );
        // One on the line that was there before stays.
        assert_eq!(pruned(lane.clone(), LINE, &[]), lane);
    }

    #[test]
    fn a_lane_is_held_only_where_a_slope_crosses_a_boundary() {
        let lane = pts(&[(0, 0.0), (100, 10.0), (200, 10.0)]);
        // 50 is on the slope; 150 is on the flat; 100 has a point.
        assert_eq!(pins(&lane, LINE, &[50, 150, 100]), vec![49, 50, 99]);
    }

    /// A piece that a snapshot may hold: from 0 to 100 in ticks of 10, twins and a point within.
    fn piece() -> Piece {
        Piece {
            path: "fader_db".into(),
            shape: LINE,
            scale: 10,
            from: 0,
            to: 100,
            first: 0.0,
            inner: pts(&[(40, 1.0), (40, 2.0), (60, 3.0)]),
            last: 4.0,
            near_end: Some(3.5),
        }
    }

    #[test]
    fn a_snapshot_of_a_lane_is_refused_for_each_thing_that_it_may_not_be() {
        assert!(piece().check_snapshot().is_ok());
        let refused = |edit: fn(&mut Piece)| {
            let mut piece = piece();
            edit(&mut piece);
            piece.check_snapshot().is_err()
        };
        assert!(refused(|p| p.scale = 0));
        assert!(refused(|p| p.from = -1));
        // No longer than nothing, with no point that would be out of it.
        assert!(refused(|p| {
            p.to = 0;
            p.inner.clear();
        }));
        assert!(refused(|p| p.first = f64::NAN));
        assert!(refused(|p| p.last = f64::INFINITY));
        assert!(refused(|p| p.near_end = Some(f64::NAN)));
        // A point in other ticks, at the start, at the end, before the one before it, of no value.
        assert!(refused(|p| p.inner[0].at.scale = 25));
        assert!(refused(|p| p.inner[0].at.ticks = 0));
        assert!(refused(|p| p.inner[2].at.ticks = 100));
        assert!(refused(|p| p.inner[2].at.ticks = 39));
        assert!(refused(|p| p.inner[1].value = f64::NAN));
    }
}
