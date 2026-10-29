//! What the edits share: a project that an edit is being worked out on, with the operations done
//! to it so far, and the steps that many edits are made of: cutting clips in two, clearing
//! frames of a track, moving what follows a frame, and taking transitions off clips while they
//! change and putting them back where the clips still meet.

use std::collections::{HashMap, HashSet};

use super::{EditError, Expansion};
use crate::ops::{apply_done, in_source, Op, OpError, LARGEST};
use crate::project::{Clip, Cue, Id, Project, Source, Timeline, Track, TrackKind, Transition};
use crate::span::ticks_of_scaled;
use crate::time::{Rate, Time};

use super::Edge;

/// A project that an edit of one of its timelines is being worked out on, and what was done.
pub(super) struct Work<'a> {
    project: &'a mut Project,
    timeline: Id,
    done: Vec<Op>,
    inverse: Vec<Op>,
    clamped: bool,
    /// The transitions taken off clips that change, to be put back where they still meet.
    detached: Vec<Transition>,
}

impl<'a> Work<'a> {
    pub fn new(project: &'a mut Project, timeline: &str) -> Result<Self, EditError> {
        if !project.timelines.iter().any(|t| t.id == timeline) {
            return Err(missing("timeline", timeline));
        }
        Ok(Self {
            project,
            timeline: timeline.into(),
            done: vec![],
            inverse: vec![],
            clamped: false,
            detached: vec![],
        })
    }

    pub fn project(&self) -> &Project {
        self.project
    }

    pub fn timeline(&self) -> &Timeline {
        self.project
            .timelines
            .iter()
            .find(|t| t.id == self.timeline)
            .expect("no edit takes its timeline away")
    }

    pub fn timeline_id(&self) -> Id {
        self.timeline.clone()
    }

    pub fn rate(&self) -> Rate {
        self.timeline().rate
    }

    /// Say that the edit went less far than it was asked to.
    pub fn clamp(&mut self) {
        self.clamped = true;
    }

    pub fn track(&self, id: &str) -> Result<&Track, EditError> {
        self.timeline()
            .tracks
            .iter()
            .find(|r| r.id == id)
            .ok_or_else(|| missing("track", id))
    }

    /// A track that the edit may change: one that is not locked.
    pub fn open(&self, id: &str) -> Result<&Track, EditError> {
        let track = self.track(id)?;
        if track.locked {
            return Err(EditError::Locked { track: id.into() });
        }
        Ok(track)
    }

    /// A clip, and the track that it is on.
    pub fn clip(&self, id: &str) -> Result<(&Track, &Clip), EditError> {
        find_clip(self.timeline(), id).ok_or_else(|| missing("clip", id))
    }

    /// A clip as it is now, and the id of its track.
    pub fn owned(&self, id: &str) -> Result<(Id, Clip), EditError> {
        let (track, clip) = self.clip(id)?;
        Ok((track.id.clone(), clip.clone()))
    }

    /// The ids of the tracks that are not locked, of the kinds given.
    pub fn unlocked(&self, kinds: &[TrackKind]) -> Vec<Id> {
        self.timeline()
            .tracks
            .iter()
            .filter(|r| !r.locked && kinds.contains(&r.kind))
            .map(|r| r.id.clone())
            .collect()
    }

    /// The ids of every track that is not locked.
    pub fn all_unlocked(&self) -> Vec<Id> {
        self.unlocked(&[TrackKind::Video, TrackKind::Audio, TrackKind::Subtitle])
    }

    /// The clips that an edit of `clips` works on: those, which must be on tracks that are not
    /// locked, and with `linked` the clips in their groups on tracks that are not locked. Each
    /// once, the ones given first.
    pub fn group(&self, clips: &[Id], linked: bool) -> Result<Vec<Id>, EditError> {
        let mut all: Vec<Id> = vec![];
        for id in clips {
            let (track, _) = self.clip(id)?;
            if track.locked {
                return Err(EditError::Locked {
                    track: track.id.clone(),
                });
            }
            if !all.contains(id) {
                all.push(id.clone());
            }
        }
        if linked {
            let groups: HashSet<&Id> = all
                .iter()
                .filter_map(|id| self.clip(id).ok()?.1.link.as_ref())
                .collect();
            for track in self.timeline().tracks.iter().filter(|r| !r.locked) {
                for clip in &track.clips {
                    let linked = clip.link.as_ref().is_some_and(|g| groups.contains(g));
                    if linked && !all.contains(&clip.id) {
                        all.push(clip.id.clone());
                    }
                }
            }
        }
        Ok(all)
    }

    /// Whether `clip` would show what is in its media, with `before` and `after` frames more
    /// beyond its ends.
    pub fn fits(&self, clip: &Clip, before: i64, after: i64) -> bool {
        let Some(media) = clip.media() else {
            return true;
        };
        let Some(media) = self.project.media.iter().find(|m| m.id == *media) else {
            return false;
        };
        in_source(media, self.rate(), clip, before, after).is_ok()
    }

    /// Apply an operation, and give it back as it was done.
    pub fn op(&mut self, op: Op) -> Result<Op, EditError> {
        let (done, inverse) = apply_done(self.project, op)?;
        self.done.push(done.clone());
        self.inverse.push(inverse);
        Ok(done)
    }

    /// Take the transitions at a clip off it, to be put back once it has changed.
    pub fn detach(&mut self, clip: &str) -> Result<(), EditError> {
        let Some((track, _)) = find_clip(self.timeline(), clip) else {
            return Ok(());
        };
        let at: Vec<Transition> = track
            .transitions
            .iter()
            .filter(|x| x.left == clip || x.right == clip)
            .cloned()
            .collect();
        for transition in at {
            self.op(Op::RemoveTransition {
                timeline: self.timeline_id(),
                id: transition.id.clone(),
            })?;
            self.detached.push(transition);
        }
        Ok(())
    }

    /// Put back the transitions that were taken off, between clips that still meet on one
    /// track, as long as they still fit, and give back what was done.
    pub fn finish(mut self) -> Result<Expansion, EditError> {
        for mut transition in std::mem::take(&mut self.detached) {
            let Some((track, left)) = find_clip(self.timeline(), &transition.left) else {
                continue;
            };
            let Some(right) = track
                .clips
                .iter()
                .find(|c| c.id == transition.right && c.start == left.end())
            else {
                continue;
            };
            let taken = track
                .transitions
                .iter()
                .any(|x| x.left == transition.left || x.right == transition.right);
            if taken {
                continue;
            }
            let length = match super::misc::check_fits(
                &self,
                track,
                (left, right),
                transition.length,
                transition.align,
                None,
            ) {
                Ok(()) => transition.length,
                Err(EditError::NoHandles {
                    longest: Some(length),
                    ..
                }) => length,
                Err(error) => return Err(error),
            };
            let track = track.id.clone();
            if length != transition.length {
                self.clamp();
                transition.length = length;
            }
            if length == 0 {
                continue;
            }
            self.op(Op::AddTransition {
                timeline: self.timeline_id(),
                track,
                transition,
            })?;
        }
        Ok(Expansion {
            done: self.done,
            inverse: self.inverse,
            clamped: self.clamped,
        })
    }

    /// Take a clip away.
    pub fn remove(&mut self, clip: &str) -> Result<(), EditError> {
        self.detach(clip)?;
        self.op(Op::RemoveClip {
            timeline: self.timeline_id(),
            id: clip.into(),
        })?;
        Ok(())
    }

    /// Put a clip on a track, and give it back with the ids that it was given.
    pub fn insert(&mut self, track: &str, clip: Clip) -> Result<Clip, EditError> {
        let done = self.op(Op::InsertClip {
            timeline: self.timeline_id(),
            track: track.into(),
            clip,
        })?;
        match done {
            Op::InsertClip { clip, .. } => Ok(clip),
            _ => unreachable!("an insert is done as an insert"),
        }
    }

    /// Move a clip to `start` on `track`.
    pub fn move_to(&mut self, clip: &str, track: &str, start: i64) -> Result<(), EditError> {
        self.detach(clip)?;
        self.op(Op::MoveClip {
            timeline: self.timeline_id(),
            id: clip.into(),
            track: track.into(),
            start,
        })?;
        Ok(())
    }

    /// Make a clip as `new` is: where it starts, how long it lasts, where it begins in its
    /// media, and its fades, which may only be shorter.
    pub fn reshape(&mut self, new: &Clip) -> Result<(), EditError> {
        let (_, old) = self.owned(&new.id)?;
        self.detach(&new.id)?;
        if (old.fade_in, old.fade_out) != (new.fade_in, new.fade_out) {
            self.op(set_fades(
                &self.timeline_id(),
                &new.id,
                new.fade_in,
                new.fade_out,
            ))?;
        }
        if (old.start, old.length, old.source_in()) != (new.start, new.length, new.source_in()) {
            self.op(Op::TrimClip {
                timeline: self.timeline_id(),
                id: new.id.clone(),
                source_in: new.source_in(),
                start: new.start,
                length: new.length,
            })?;
        }
        Ok(())
    }

    /// Cut each of `clips`, which lie across `at`, in two there. The first half keeps the id of
    /// the clip and the transition at its start; the second half is a new clip, which takes the
    /// transition at its end. The second halves of the clips of a group are put in a new group
    /// together.
    pub fn blade(&mut self, clips: &[Id], at: i64) -> Result<(), EditError> {
        let mut groups: HashMap<Id, Id> = HashMap::new();
        for id in clips {
            let (track, clip) = self.owned(id)?;
            let (first, mut second) = halves_of(&clip, at, self.rate());
            second.link = clip
                .link
                .as_ref()
                .map(|group| groups.get(group).cloned().unwrap_or_default());
            self.reshape(&first)?;
            let second = self.insert(&track, second)?;
            if let (Some(group), Some(made)) = (&clip.link, &second.link) {
                groups.insert(group.clone(), made.clone());
            }
            for transition in &mut self.detached {
                if transition.left == *id {
                    transition.left.clone_from(&second.id);
                }
            }
        }
        Ok(())
    }

    /// The clips of a track that lie across `at`.
    pub fn across(&self, track: &str, at: i64) -> Result<Vec<Id>, EditError> {
        Ok(self
            .track(track)?
            .clips
            .iter()
            .filter(|c| c.start < at && at < c.end())
            .map(|c| c.id.clone())
            .collect())
    }

    /// Clear clips from frames `from` to `to`: what lies across either end is cut there, and
    /// what is between them is taken away. Subtitles are cut only when the gap is closed.
    pub fn clear_clips(&mut self, tracks: &[Id], from: i64, to: i64) -> Result<(), EditError> {
        for at in [from, to] {
            let mut across = vec![];
            for track in tracks {
                across.extend(self.across(track, at)?);
            }
            self.blade(&across, at)?;
        }
        for track in tracks {
            let inside: Vec<Id> = self
                .track(track)?
                .clips
                .iter()
                .filter(|c| from <= c.start && c.end() <= to)
                .map(|c| c.id.clone())
                .collect();
            for id in inside {
                self.remove(&id)?;
            }
        }
        Ok(())
    }

    /// Cut a gap out of subtitles before moving what follows it. A subtitle across both
    /// ends keeps its two remaining parts together, at its original start.
    fn cut_cues(&mut self, tracks: &[Id], from: i64, to: i64) -> Result<(), EditError> {
        for track in tracks {
            let cues = self.track(track)?.cues.clone();
            for cue in cues {
                let end = cue.start + cue.length;
                if end <= from || to <= cue.start {
                    continue;
                }
                let timeline = self.timeline_id();
                let op = if from <= cue.start && end <= to {
                    Op::RemoveCue {
                        timeline,
                        id: cue.id,
                    }
                } else {
                    let kept_before = (from - cue.start).max(0);
                    let kept_after = (end - to).max(0);
                    let start = if cue.start < from { cue.start } else { to };
                    Op::SetCue {
                        timeline,
                        id: cue.id,
                        start: Some(start),
                        length: Some(kept_before + kept_after),
                        text: None,
                    }
                };
                self.op(op)?;
            }
        }
        Ok(())
    }

    /// Move what starts at `from` or later on `tracks` by `by` frames, but for `skip`. Where
    /// `by` is below 0, the frames from `from + by` to `from` must be clear of clips that are
    /// not moved, or the ripple is blocked. Subtitles in those frames are cut with the gap.
    pub fn shift(
        &mut self,
        tracks: &[Id],
        from: i64,
        by: i64,
        skip: &HashSet<Id>,
    ) -> Result<(), EditError> {
        if by == 0 {
            return Ok(());
        }
        if by < 0 {
            for id in tracks {
                if let Some(clip) = in_the_way(self.track(id)?, from + by, from, skip) {
                    return Err(EditError::RippleBlocked {
                        track: id.clone(),
                        clip,
                    });
                }
            }
            self.cut_cues(tracks, from + by, from)?;
        }
        for id in tracks {
            let track = self.track(id)?;
            let clips: Vec<Id> = track
                .clips
                .iter()
                .filter(|c| c.start >= from && !skip.contains(&c.id))
                .map(|c| c.id.clone())
                .collect();
            let selected: HashSet<&str> = clips.iter().map(String::as_str).collect();
            let boundaries: Vec<Transition> = track
                .transitions
                .iter()
                .filter(|x| {
                    selected.contains(x.left.as_str()) != selected.contains(x.right.as_str())
                })
                .cloned()
                .collect();
            let mut cues: Vec<(i64, Id)> = track
                .cues
                .iter()
                .filter(|q| q.start >= from && !skip.contains(&q.id))
                .map(|q| (q.start, q.id.clone()))
                .collect();
            // The one furthest along goes first, so that none is moved onto another.
            if by > 0 {
                cues.reverse();
            }
            for transition in boundaries {
                self.op(Op::RemoveTransition {
                    timeline: self.timeline_id(),
                    id: transition.id.clone(),
                })?;
                self.detached.push(transition);
            }
            if !clips.is_empty() {
                self.op(Op::ShiftClips {
                    timeline: self.timeline_id(),
                    track: id.clone(),
                    clips,
                    by,
                })?;
            }
            for (start, cue) in cues {
                self.op(Op::SetCue {
                    timeline: self.timeline_id(),
                    id: cue,
                    start: Some(start + by),
                    length: None,
                    text: None,
                })?;
            }
        }
        Ok(())
    }
}

impl Work<'_> {
    /// Make room for `length` frames at `at` on every track that is not locked: cut the clips
    /// that lie across it there, and move what starts there or later on.
    pub fn make_room(&mut self, at: i64, length: i64) -> Result<(), EditError> {
        if at < 0 {
            return Err(EditError::Op(OpError::OutOfTimeline { start: at }));
        }
        let mut across = vec![];
        for track in self.unlocked(&[TrackKind::Video, TrackKind::Audio]) {
            across.extend(self.across(&track, at)?);
        }
        self.blade(&across, at)?;
        let tracks = self.all_unlocked();
        for track in &tracks {
            let cues: Vec<Cue> = self
                .track(track)?
                .cues
                .iter()
                .filter(|cue| cue.start < at && at < cue.start + cue.length)
                .cloned()
                .collect();
            for cue in cues {
                self.op(Op::SetCue {
                    timeline: self.timeline_id(),
                    id: cue.id.clone(),
                    start: None,
                    length: Some(at - cue.start),
                    text: None,
                })?;
                self.op(Op::AddCue {
                    timeline: self.timeline_id(),
                    track: track.clone(),
                    cue: Cue {
                        id: String::new(),
                        start: at,
                        length: cue.start + cue.length - at,
                        text: cue.text,
                    },
                })?;
            }
        }
        self.shift(&tracks, at, length, &HashSet::new())
    }

    /// Close the gaps from the start to the end of each of `spans` on every track that is not
    /// locked, the last first.
    pub fn close(&mut self, spans: &[(i64, i64)]) -> Result<(), EditError> {
        let tracks = self.all_unlocked();
        for &(from, to) in union(spans).iter().rev() {
            self.shift(&tracks, to, from - to, &HashSet::new())?;
        }
        Ok(())
    }
}

/// The stretches of frames that `spans` cover, in order, those that meet or overlap made one.
pub(super) fn union(spans: &[(i64, i64)]) -> Vec<(i64, i64)> {
    let mut spans = spans.to_vec();
    spans.sort_unstable();
    let mut all: Vec<(i64, i64)> = vec![];
    for (from, to) in spans {
        match all.last_mut() {
            Some(last) if from <= last.1 => last.1 = last.1.max(to),
            _ => all.push((from, to)),
        }
    }
    all
}

/// A clip of a timeline, and the track that it is on.
pub(super) fn find_clip<'t>(timeline: &'t Timeline, id: &str) -> Option<(&'t Track, &'t Clip)> {
    timeline
        .tracks
        .iter()
        .find_map(|r| r.clips.iter().find(|c| c.id == id).map(|c| (r, c)))
}

/// The first clip of a track, but for `skip`, that covers a frame from `from` to `to`.
fn in_the_way(track: &Track, from: i64, to: i64, skip: &HashSet<Id>) -> Option<Id> {
    track
        .clips
        .iter()
        .find(|c| c.start < to && from < c.end() && !skip.contains(&c.id))
        .map(|c| c.id.clone())
}

/// The transitions at the start and at the end of a clip of a track.
pub(super) fn ends<'t>(
    track: &'t Track,
    clip: &str,
) -> (Option<&'t Transition>, Option<&'t Transition>) {
    let start = track.transitions.iter().find(|x| x.right == clip);
    let end = track.transitions.iter().find(|x| x.left == clip);
    (start, end)
}

/// The frames of a clip of a track that the transitions at its start and end cover.
pub(super) fn covered(track: &Track, clip: &str) -> (i64, i64) {
    let (start, end) = ends(track, clip);
    (
        start.map_or(0, |x| x.handles().0),
        end.map_or(0, |x| x.handles().1),
    )
}

/// The frames of its media that the transitions at its start and end show of a clip of a track
/// beyond its ends.
pub(super) fn beyond(track: &Track, clip: &str) -> (i64, i64) {
    let (start, end) = ends(track, clip);
    (
        start.map_or(0, |x| x.handles().1),
        end.map_or(0, |x| x.handles().0),
    )
}

/// The two halves of a clip cut at `at`, which it lies across. Each shows what the clip shows
/// there, from where the other ends in the media: forward, the second begins where the first
/// ends; backward, the first begins where the second ends. The first keeps the fade in and the
/// second the fade out, each as far as it lasts. The second has no id.
pub(super) fn halves_of(clip: &Clip, at: i64, rate: Rate) -> (Clip, Clip) {
    let a = at - clip.start;
    let b = clip.length - a;
    let mut first = Clip {
        length: a,
        fade_in: clip.fade_in.min(a),
        fade_out: 0,
        ..clip.clone()
    };
    let mut second = Clip {
        id: String::new(),
        start: at,
        length: b,
        fade_in: 0,
        fade_out: clip.fade_out.min(b),
        ..clip.clone()
    };
    if let Some(begins) = clip.source_in() {
        let later = |frames| Time {
            ticks: begins.ticks + ticks_of_scaled(frames, clip.speed, rate, begins.scale),
            ..begins
        };
        if clip.reverse {
            set_source_in(&mut first, later(b));
        } else {
            set_source_in(&mut second, later(a));
        }
    }
    (first, second)
}

/// A clip with one edge moved `delta` frames later: its start, so that it is `delta` frames
/// shorter, or its end, so that it is `delta` frames longer. What it shows of its media stays
/// where it was on the timeline, but for a tick at speeds whose frames do not last whole ticks.
/// With `keep_start`, the clip still starts where it did, as in a ripple, and so does what it
/// shows.
pub(super) fn edged(clip: &Clip, edge: Edge, delta: i64, keep_start: bool, rate: Rate) -> Clip {
    let mut new = clip.clone();
    let scale = clip.source_in().map_or(1, |t| t.scale);
    let ticks = |frames| ticks_of_scaled(frames, clip.speed, rate, scale);
    // Forward, the start of a clip shows where it begins in its media; backward, its end does.
    let moves_begin = (edge == Edge::In) != clip.reverse;
    let shift = match edge {
        Edge::In => ticks(delta),
        Edge::Out => -ticks(delta),
    };
    match edge {
        Edge::In => {
            new.length = clip.length - delta;
            if !keep_start {
                new.start = clip.start + delta;
            }
        }
        Edge::Out => new.length = clip.length + delta,
    }
    if let (true, Some(begins)) = (moves_begin, clip.source_in()) {
        set_source_in(
            &mut new,
            Time {
                ticks: begins.ticks.saturating_add(shift),
                scale,
            },
        );
    }
    fit_fades(&mut new, edge);
    new
}

/// Shorten the fades of a clip that would last longer than it does: the one at `edge` first.
pub(super) fn fit_fades(clip: &mut Clip, edge: Edge) {
    let over = clip.fade_in + clip.fade_out - clip.length.max(0);
    if over <= 0 {
        return;
    }
    let (first, second) = match edge {
        Edge::In => (&mut clip.fade_in, &mut clip.fade_out),
        Edge::Out => (&mut clip.fade_out, &mut clip.fade_in),
    };
    let cut = over.min(*first);
    *first -= cut;
    *second -= (over - cut).min(*second);
}

pub(super) fn set_source_in(clip: &mut Clip, time: Time) {
    if let Source::Media { source_in, .. } = &mut clip.source {
        *source_in = time;
    }
}

/// An operation that sets the fades of a clip.
pub(super) fn set_fades(timeline: &str, clip: &str, fade_in: i64, fade_out: i64) -> Op {
    Op::SetClip {
        timeline: timeline.into(),
        id: clip.into(),
        enabled: None,
        speed: None,
        reverse: None,
        link: None,
        transform: None,
        gain_db: None,
        pan: None,
        fade_in: Some(fade_in),
        fade_out: Some(fade_out),
        source: None,
    }
}

/// The delta from 0 toward `want`, as far as `ok` allows, which it allows of 0 and of fewer
/// frames than one that it allows.
pub(super) fn most(want: i64, ok: impl Fn(i64) -> bool) -> i64 {
    if ok(want) {
        return want;
    }
    let sign = want.signum();
    let (mut good, mut bad) = (0, want.abs());
    while bad - good > 1 {
        let middle = good + (bad - good) / 2;
        if ok(sign * middle) {
            good = middle;
        } else {
            bad = middle;
        }
    }
    sign * good
}

/// The deltas that an edit can go by, and the error to give where it can go nowhere.
pub(super) struct Limit {
    lo: i64,
    hi: i64,
    below: Option<EditError>,
    above: Option<EditError>,
}

impl Limit {
    pub fn new() -> Self {
        Self {
            lo: -LARGEST,
            hi: LARGEST,
            below: None,
            above: None,
        }
    }

    pub fn at_least(&mut self, lo: i64, why: impl FnOnce() -> EditError) {
        if lo > self.lo {
            self.lo = lo.min(0);
            self.below = Some(why());
        }
    }

    pub fn at_most(&mut self, hi: i64, why: impl FnOnce() -> EditError) {
        if hi < self.hi {
            self.hi = hi.max(0);
            self.above = Some(why());
        }
    }

    /// The delta as near `want` as the limit lets it be, and why it is no nearer if it is not.
    pub fn clamp(&self, want: i64) -> (i64, Option<EditError>) {
        if want < self.lo {
            (self.lo, self.below.clone())
        } else if want > self.hi {
            (self.hi, self.above.clone())
        } else {
            (want, None)
        }
    }
}

/// Go by `want`, or as near it as `limit` and then `fits` let, and say whether that is less;
/// refuse if it is nothing. `fits` tells whether a delta keeps every clip within its media, and
/// names a clip that it does not keep.
pub(super) fn reach(
    want: i64,
    limit: &Limit,
    fits: impl Fn(i64) -> Result<(), Id>,
) -> Result<(i64, bool), EditError> {
    if want == 0 {
        return Err(EditError::EmptyRange);
    }
    let (near, mut why) = limit.clamp(want);
    let far = most(near, |d| fits(d).is_ok());
    if far != near {
        why = Some(EditError::NoHandles {
            clip: fits(near).err().unwrap_or_default(),
            need: want.abs(),
            have: far.abs(),
            longest: None,
        });
    }
    if far == 0 {
        return Err(why.unwrap_or(EditError::EmptyRange));
    }
    Ok((far, far != want))
}

pub(super) fn missing(what: &'static str, id: &str) -> EditError {
    EditError::Op(OpError::Missing {
        what,
        id: id.into(),
    })
}
