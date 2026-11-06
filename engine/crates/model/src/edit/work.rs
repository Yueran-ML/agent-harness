//! What the edits share: a project that an edit is being worked out on, with the operations done
//! to it so far, and the steps that many edits are made of: cutting clips in two, clearing
//! frames of a track, moving what follows a frame, and taking transitions off clips while they
//! change and putting them back where the clips still meet.

use std::collections::{HashMap, HashSet};

use super::follow::Follow;
use super::{Created, EditError, Expansion};
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
    created: Vec<Created>,
    /// The transitions taken off clips that change, to be put back where they still meet.
    detached: Vec<Transition>,
    /// The lanes that follow the edit (Automation Follows Edit).
    follow: Follow,
}

/// The timeline `id` of a project, which no edit takes away.
fn timeline_in<'p>(project: &'p Project, id: &str) -> &'p Timeline {
    project
        .timelines
        .iter()
        .find(|t| t.id == id)
        .expect("no edit takes its timeline away")
}

impl<'a> Work<'a> {
    pub fn new(project: &'a mut Project, timeline: &str) -> Result<Self, EditError> {
        let Some(found) = project.timelines.iter().find(|t| t.id == timeline) else {
            return Err(missing("timeline", timeline));
        };
        let follow = Follow::new(found);
        Ok(Self {
            project,
            timeline: timeline.into(),
            done: vec![],
            inverse: vec![],
            clamped: false,
            created: vec![],
            detached: vec![],
            follow,
        })
    }

    pub fn project(&self) -> &Project {
        self.project
    }

    pub fn timeline(&self) -> &Timeline {
        timeline_in(self.project, &self.timeline)
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

    /// Record the edit's intended output for a caller to bind a transaction name to.
    pub fn created(&mut self, output: Created) {
        self.created.push(output);
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

    /// The other tracks that an edit of `track` takes along: those, not locked, of the active
    /// groups of tracks that link Editing and that it is in (Ch. 179, p. 3917).
    pub fn edited_with(&self, track: &str) -> Vec<Id> {
        super::clipboard::editing_peers(self.timeline(), track)
    }

    /// `tracks`, each once, with the tracks that an edit of each takes along.
    pub fn with_groups(&self, tracks: Vec<Id>) -> Vec<Id> {
        let mut all = tracks;
        for track in all.clone() {
            for other in self.edited_with(&track) {
                if !all.contains(&other) {
                    all.push(other);
                }
            }
        }
        all
    }

    /// For an edit of `clips` told `grouped`: the frames of each of them, from its start to its
    /// end, on each other track that an edit of its track takes along.
    pub fn group_ranges(&self, clips: &[Id]) -> Result<Vec<(Id, i64, i64)>, EditError> {
        let mut ranges = vec![];
        for id in clips {
            let (track, clip) = self.clip(id)?;
            for other in self.edited_with(&track.id) {
                ranges.push((other, clip.start, clip.end()));
            }
        }
        Ok(ranges)
    }

    /// Cut the clips of `track` but for `keep` that lie across `from` or `to` there, and give the
    /// ids of those, but for `keep`, that are then from `from` to `to`.
    pub fn cut_out(
        &mut self,
        track: &str,
        (from, to): (i64, i64),
        keep: &[Id],
    ) -> Result<Vec<Id>, EditError> {
        for at in [from, to] {
            let mut across = self.across(track, at)?;
            across.retain(|id| !keep.contains(id));
            self.blade(&across, at)?;
        }
        Ok(self
            .track(track)?
            .clips
            .iter()
            .filter(|c| from <= c.start && c.end() <= to && !keep.contains(&c.id))
            .map(|c| c.id.clone())
            .collect())
    }

    /// Keep what the lanes of `track` are over `clip`, which is copied, for its copies.
    pub fn keep_lanes(&mut self, track: &str, clip: &Clip) {
        self.follow
            .keep(timeline_in(self.project, &self.timeline), track, clip);
    }

    pub fn keep_snapshot_lanes(&mut self, key: &str, pieces: &[super::follow::Piece]) {
        self.follow.keep_snapshot(key, pieces);
    }

    /// Put a copy of the clip `source` on a track, with a copy of the lanes over `source` (see
    /// [`Work::keep_lanes`]), and give it back with the ids that it was given.
    pub fn insert_copy(
        &mut self,
        track: &str,
        clip: Clip,
        source: &str,
    ) -> Result<Clip, EditError> {
        let done = self.op(Op::InsertClip {
            timeline: self.timeline_id(),
            track: track.into(),
            clip,
        })?;
        let Op::InsertClip { clip, .. } = done else {
            unreachable!("an insert is done as an insert")
        };
        self.follow.put_copy(
            timeline_in(self.project, &self.timeline),
            track,
            &clip,
            source,
        );
        Ok(clip)
    }

    /// Ripple the lanes of `track` at frame `place` by `by` frames, `spans` moving with them, and
    /// leave them alone in the ripples of the rest of the edit.
    pub fn ripple_lanes(&mut self, track: &str, (place, by): (i64, i64), spans: &[(i64, i64)]) {
        let timeline = timeline_in(self.project, &self.timeline);
        self.follow.ripple_with(timeline, track, (place, by), spans);
        self.follow.hold(track);
    }

    /// Ripple the lanes of `track` for what starts from frame `from` on and before frame `until`,
    /// but for `skip`, moving by `by` frames, leaving them from frame `resume` on as they are
    /// (see [`Follow::ripple_before`]).
    pub fn ripple_lanes_before(
        &mut self,
        track: &str,
        (from, by): (i64, i64),
        (until, resume): (i64, i64),
        skip: &HashSet<Id>,
    ) {
        let timeline = timeline_in(self.project, &self.timeline);
        self.follow
            .ripple_before(timeline, track, (from, by), (until, resume), skip);
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
        let fresh = Follow::new(timeline_in(self.project, &self.timeline));
        let follow = std::mem::replace(&mut self.follow, fresh);
        for op in follow.ops(self.timeline()) {
            self.op(op)?;
        }
        Ok(Expansion {
            done: self.done,
            inverse: self.inverse,
            clamped: self.clamped,
            created: self.created,
        })
    }

    /// Take a clip away, and the automation over it (Automation Follows Edit), which goes with it
    /// where it is put back.
    pub fn remove(&mut self, clip: &str) -> Result<(), EditError> {
        if let Some((track, found)) = find_clip(self.timeline(), clip) {
            let (track, found) = (track.id.clone(), found.clone());
            self.follow
                .take(timeline_in(self.project, &self.timeline), &track, &found);
        }
        self.remove_alone(clip)
    }

    /// Take a clip away, leaving the automation over it as it is: for a clip put back where it
    /// was, or on a track that goes too.
    pub fn remove_alone(&mut self, clip: &str) -> Result<(), EditError> {
        self.detach(clip)?;
        self.op(Op::RemoveClip {
            timeline: self.timeline_id(),
            id: clip.into(),
        })?;
        Ok(())
    }

    /// Put a clip on a track, and give it back with the ids that it was given.
    /// A clip taken away by [`Work::remove`] and put back takes its automation along.
    pub fn insert(&mut self, track: &str, clip: Clip) -> Result<Clip, EditError> {
        let done = self.op(Op::InsertClip {
            timeline: self.timeline_id(),
            track: track.into(),
            clip,
        })?;
        match done {
            Op::InsertClip { clip, .. } => {
                self.follow
                    .put(timeline_in(self.project, &self.timeline), track, &clip);
                Ok(clip)
            }
            _ => unreachable!("an insert is done as an insert"),
        }
    }

    /// Move a clip to `start` on `track`, with the automation over it.
    pub fn move_to(&mut self, clip: &str, track: &str, start: i64) -> Result<(), EditError> {
        let (from, old) = self.owned(clip)?;
        self.follow
            .take(timeline_in(self.project, &self.timeline), &from, &old);
        self.detach(clip)?;
        self.op(Op::MoveClip {
            timeline: self.timeline_id(),
            id: clip.into(),
            track: track.into(),
            start,
        })?;
        let (_, new) = self.owned(clip)?;
        self.follow
            .put(timeline_in(self.project, &self.timeline), track, &new);
        Ok(())
    }

    /// Make a clip as `new` is: where it starts, how long it lasts, where it begins in its
    /// media, and its fades, which may only be shorter.
    pub fn reshape(&mut self, new: &Clip) -> Result<(), EditError> {
        let (_, old) = self.owned(&new.id)?;
        self.detach(&new.id)?;
        let sound =
            (old.audio_fade_in, old.audio_fade_out) != (new.audio_fade_in, new.audio_fade_out);
        if sound || (old.fade_in, old.fade_out) != (new.fade_in, new.fade_out) {
            let mut op = set_fades(&self.timeline_id(), &new.id, new.fade_in, new.fade_out);
            if let (
                true,
                Op::SetClip {
                    audio_fade_in,
                    audio_fade_out,
                    ..
                },
            ) = (sound, &mut op)
            {
                (*audio_fade_in, *audio_fade_out) =
                    (Some(new.audio_fade_in), Some(new.audio_fade_out));
            }
            self.op(op)?;
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

    /// Refuse a ripple from `place` that ends a split edit, of which `group` holds the clips that
    /// change and their tracks, while something starts between the end of one of them and
    /// `place` on its track. That would stay where it is while what follows `place` moves, and
    /// so part the picture of an L or J cut that follows from its sound; and where the group
    /// gets longer, its clip would run into it.
    pub fn split_ripple(&self, group: &[(Id, Clip)], place: i64) -> Result<(), EditError> {
        let members: HashSet<&str> = group.iter().map(|(_, c)| c.id.as_str()).collect();
        for (track, c) in group {
            let blocker = self.track(track)?.clips.iter().find(|o| {
                !members.contains(o.id.as_str()) && c.end() <= o.start && o.start < place
            });
            if let Some(blocker) = blocker {
                return Err(EditError::RippleBlocked {
                    track: track.clone(),
                    clip: blocker.id.clone(),
                });
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
    pub fn cut_cues(&mut self, tracks: &[Id], from: i64, to: i64) -> Result<(), EditError> {
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

    /// Cut each subtitle of `tracks` that is shown across `at` in two there: the first part
    /// keeps its id, the second is a new subtitle with the same text.
    pub fn split_cues(&mut self, tracks: &[Id], at: i64) -> Result<(), EditError> {
        for track in tracks {
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
        Ok(())
    }

    /// Move what starts at `from` or later on `tracks` by `by` frames, but for `skip`. Where
    /// `by` is below 0, the frames from `from + by` to `from` must be clear of clips that are
    /// not moved, or the ripple is blocked. Subtitles in those frames are cut with the gap.
    /// Where `by` is above 0, a subtitle shown across `from` is cut in two there, and its
    /// second part moves with what follows.
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
        } else {
            self.split_cues(tracks, from)?;
        }
        self.move_after(tracks, from, by, skip)
    }

    /// Move what starts at `from` or later on `tracks` by `by` frames, but for `skip`, as it is:
    /// nothing in the way is looked for, and no subtitle is cut.
    pub fn move_after(
        &mut self,
        tracks: &[Id],
        from: i64,
        by: i64,
        skip: &HashSet<Id>,
    ) -> Result<(), EditError> {
        for id in tracks {
            self.follow.ripple(
                timeline_in(self.project, &self.timeline),
                id,
                (from, by),
                skip,
            );
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
            let cues: Vec<Id> = track
                .cues
                .iter()
                .filter(|q| q.start >= from && !skip.contains(&q.id))
                .map(|q| q.id.clone())
                .collect();
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
            if !cues.is_empty() {
                self.op(Op::ShiftCues {
                    timeline: self.timeline_id(),
                    track: id.clone(),
                    cues,
                    by,
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
/// second the fade out, each as far as it lasts, of the picture and of the sound: no fade is
/// left at the cut. The second has no id.
pub(super) fn halves_of(clip: &Clip, at: i64, rate: Rate) -> (Clip, Clip) {
    let a = at - clip.start;
    let b = clip.length - a;
    let mut first = Clip {
        length: a,
        fade_in: clip.fade_in.min(a),
        fade_out: 0,
        audio_fade_out: None,
        ..clip.clone()
    };
    let mut second = Clip {
        id: String::new(),
        start: at,
        length: b,
        fade_in: 0,
        fade_out: clip.fade_out.min(b),
        audio_fade_in: None,
        ..clip.clone()
    };
    fit_sound_fades(&mut first, Edge::In, rate);
    fit_sound_fades(&mut second, Edge::Out, rate);
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
    fit_fades(&mut new, edge, rate);
    new
}

/// Shorten the fades of a clip, on a timeline of `rate`, that would last longer than it does:
/// the one at `edge` first; those of its picture in frames, and those of its sound in time.
pub(super) fn fit_fades(clip: &mut Clip, edge: Edge, rate: Rate) {
    let over = clip.fade_in + clip.fade_out - clip.length.max(0);
    if over > 0 {
        let (first, second) = match edge {
            Edge::In => (&mut clip.fade_in, &mut clip.fade_out),
            Edge::Out => (&mut clip.fade_out, &mut clip.fade_in),
        };
        let cut = over.min(*first);
        *first -= cut;
        *second -= (over - cut).min(*second);
    }
    fit_sound_fades(clip, edge, rate);
}

/// Shorten the fades of the sound of a clip, on a timeline of `rate`, that would last longer
/// together than it does: the one at `edge` first, each in its own scale, rounded down. A fade
/// of its frames (`None`) is as its picture's, which fits already.
fn fit_sound_fades(clip: &mut Clip, edge: Edge, rate: Rate) {
    // Seconds as fractions: a fade of sound, or of frames where it has none.
    let frames = |count: i64| {
        (
            i128::from(count) * i128::from(rate.den),
            i128::from(rate.num),
        )
    };
    let lasts = |fade: Option<crate::audio::AudioFade>, count: i64| {
        fade.map_or(frames(count), |f| {
            (
                i128::from(f.length.ticks),
                i128::from(f.length.scale.max(1)),
            )
        })
    };
    let (rises, rise_scale) = lasts(clip.audio_fade_in, clip.fade_in);
    let (falls, fall_scale) = lasts(clip.audio_fade_out, clip.fade_out);
    let (whole, whole_scale) = frames(clip.length.max(0));
    // How far over, as p/q seconds: the two fades less the clip, over the product of their
    // scales; each product checked, as one of three scales of 32 bits may pass i128.
    let (Some(mut q), Some(mut p)) = (
        rise_scale
            .checked_mul(fall_scale)
            .and_then(|both| both.checked_mul(whole_scale)),
        (|| {
            rises
                .checked_mul(fall_scale)?
                .checked_mul(whole_scale)?
                .checked_add(falls.checked_mul(rise_scale)?.checked_mul(whole_scale)?)?
                .checked_sub(whole.checked_mul(rise_scale)?.checked_mul(fall_scale)?)
        })(),
    ) else {
        return;
    };
    let order = match edge {
        Edge::In => [true, false],
        Edge::Out => [false, true],
    };
    for fade_in in order {
        if p <= 0 {
            return;
        }
        let fade = if fade_in {
            &mut clip.audio_fade_in
        } else {
            &mut clip.audio_fade_out
        };
        let Some(fade) = fade else {
            continue;
        };
        let (ticks, scale) = (
            i128::from(fade.length.ticks),
            i128::from(fade.length.scale.max(1)),
        );
        // The longest it may last now, ticks/scale - p/q, as one fraction in its lowest terms
        // where its denominator is a scale; otherwise in its own scale, rounded down.
        let (Some(whole), Some(cut), Some(under)) = (
            ticks.checked_mul(q),
            p.checked_mul(scale),
            q.checked_mul(scale),
        ) else {
            return;
        };
        let left = (whole - cut).max(0);
        let common = gcd(left, under).max(1);
        let exact = (i64::try_from(left / common), u32::try_from(under / common));
        fade.length = match exact {
            (Ok(ticks), Ok(scale)) if left < whole => Time { ticks, scale },
            _ => Time {
                ticks: i64::try_from((left / q).clamp(0, ticks)).unwrap_or(0),
                scale: fade.length.scale,
            },
        };
        // What is left over: p/q - (what was cut)/scale, at least nothing.
        let now = i128::from(fade.length.ticks) * under / i128::from(fade.length.scale.max(1));
        (p, q) = (cut - (whole - now), under);
    }
}

/// The greatest common divisor of two numbers of no less than 0.
fn gcd(mut a: i128, mut b: i128) -> i128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
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
        audio_fade_in: None,
        audio_fade_out: None,
        eq: None,
        effects: None,
        pitch_correction: None,
        pitch: None,
        channels: None,
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
