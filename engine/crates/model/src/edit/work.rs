//! What the edits share: a project that an edit is being worked out on, with the operations done
//! to it so far, and the transitions taken off clips while they change, to be put back where the
//! clips still meet.

use super::{EditError, Expansion};
use crate::ops::{apply_done, Op, OpError};
use crate::project::{Clip, Id, Project, Timeline, Track, Transition};

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
    /// track, and give back what was done.
    pub fn finish(mut self) -> Result<Expansion, EditError> {
        for transition in std::mem::take(&mut self.detached) {
            let Some((track, left)) = find_clip(self.timeline(), &transition.left) else {
                continue;
            };
            let right = track.clips.iter().find(|c| c.id == transition.right);
            let taken = track
                .transitions
                .iter()
                .any(|x| x.left == transition.left || x.right == transition.right);
            if right.is_none_or(|right| right.start != left.end()) || taken {
                continue;
            }
            let track = track.id.clone();
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
}

/// A clip of a timeline, and the track that it is on.
pub(super) fn find_clip<'t>(timeline: &'t Timeline, id: &str) -> Option<(&'t Track, &'t Clip)> {
    timeline
        .tracks
        .iter()
        .find_map(|r| r.clips.iter().find(|c| c.id == id).map(|c| (r, c)))
}

pub(super) fn missing(what: &'static str, id: &str) -> EditError {
    EditError::Op(OpError::Missing {
        what,
        id: id.into(),
    })
}
