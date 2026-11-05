//! Transactions on a project, and the one line of undo that every client of the engine shares.
//!
//! The engine is the only writer of a project. Its clients, the desktop and the agent, send it
//! transactions, each a list of operations that is applied whole or not at all. Each
//! transaction that is applied makes a new revision of the project, and so does each undo and
//! each redo. The steps that can be undone are kept in the order in which they were done,
//! whoever made them: an undo takes back the latest of them, whether the desktop or the agent
//! made it.
//!
//! # Gestures
//!
//! A control dragged on the Color page changes the project many times a second, and each change
//! is to be seen at once, by every client, and yet the whole drag is one step to undo, as it is in
//! `DaVinci Resolve`. So a transaction may be sent as part of a **gesture**, named by a key
//! ([`History::change_in_gesture`]). The first transaction of a gesture is a step as any other;
//! each next one of the same gesture is applied, makes its revision as any other, and is put into
//! that step: its operations after the step's, its inverses after the step's inverses, so that an
//! undo takes back the whole gesture, last first, to the project before it, and a redo does it
//! again. A gesture ends when it says so (its last transaction, or [`History::end_gesture`]), and
//! whenever anything else is done to the history: another transaction (of anyone, of another
//! gesture or of none), an undo or a redo. A transaction of an ended gesture's key begins a new
//! gesture. A gesture that ends with the project as it was before it (a drag let go where it began,
//! or put back by Escape) leaves no step at all; `next_id` is not taken back.
//!
//! A change of the engine that is no step ([`History::apply_unrecorded`]) does not end a
//! gesture: it adds or takes away nothing that the gesture's step names.

use std::collections::VecDeque;
use std::ops::Range;

use serde::{Serialize, Serializer};

use crate::edit::{run_change, Change, ChangeError, Edit, EditError, Expansion};
use crate::ops::{apply_done, Op, OpError};
use crate::project::Project;

/// How many steps are kept to be undone. When one more is done, the oldest is let go.
pub const HISTORY_MOST: usize = 1000;

/// A project, its revision, and the steps that can be undone and redone.
#[derive(Clone, Debug)]
pub struct History {
    project: Project,
    revision: u64,
    /// The steps that can be undone, the latest last.
    undo: VecDeque<Step>,
    /// The steps that can be redone, the one that was undone last at the end.
    redo: Vec<Step>,
    /// How many steps are kept to be undone.
    most: usize,
    /// The gesture that is going on, whose step is the latest of `undo`.
    gesture: Option<Gesture>,
    /// The revision that the last change that was not the engine's own (a step, an undo, a
    /// redo) made: since it, only changes of the engine that are no step were made.
    recorded: u64,
}

/// A gesture that is going on: its key, the revision before its first transaction, and the
/// project then, to tell whether it ends where it began.
#[derive(Clone, Debug)]
struct Gesture {
    key: String,
    base: u64,
    before: Project,
}

/// A transaction that was done: what was done, with the ids that the engine gave, and what
/// takes it back.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    /// The operations as they were done, in order: each carries every id that the engine gave
    /// and the place that each thing was put at, so that doing them again makes the same things
    /// with the same ids.
    pub done: Vec<Op>,
    /// The inverses of `done`, in the same order. They are applied last first to take the
    /// step back.
    pub inverse: Vec<Op>,
    /// Who made the transaction.
    pub by: String,
}

/// What a transaction, an undo or a redo did: all that a client needs to bring its copy of the
/// project up to the new revision, and to tell whose change it was.
#[derive(Clone, Debug, PartialEq)]
pub struct Applied {
    /// The revision of the project that it made.
    pub revision: u64,
    /// The operations that were applied, in the order in which they were applied, each as it
    /// was done: a client that applies them to its copy of the revision before gets this one,
    /// with the same ids.
    pub done: Vec<Op>,
    /// Who asked for it.
    pub by: String,
    /// Who made the transaction: for a new one, who asked for it; for an undo or a redo, who
    /// made the transaction that it takes back or does again.
    pub made_by: String,
    /// The `next_id` of the project after it. A copy of the project that is given it before
    /// `done` is applied takes the ids in `done` as ids that the engine made, and does not
    /// refuse them as ids that it may make later.
    pub next_id: u64,
}

/// Why a transaction, an undo or a redo was refused. The project and its revision are then
/// exactly as they were.
///
/// It is written in JSON as [`OpError`] is, with the message of the whole error in `message`:
/// `{"reason": "empty", "message": "the transaction has no operations"}`, or, for an operation
/// that was refused, the place of the operation in `index` beside the reason and the fields of
/// [`OpError`]: `{"index": 2, "reason": "overlap", "with": "c1", "message": "operation 2 of the
/// transaction was refused: the clip covers frames that clip c1 covers"}`.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TransactionError {
    /// The transaction has no operations. Nothing is done, and no revision is made: a
    /// transaction that changes nothing is taken for a mistake of the client.
    #[error("the transaction has no operations")]
    Empty,
    /// An operation was refused, and those before it were taken back.
    ///
    /// `index` is the place of the operation in the list that was applied: for a transaction,
    /// the list that the client sent; for an undo, the inverses of the step, last first; for a
    /// redo, the operations of the step as they were done. An undo or a redo is refused only
    /// through a bug of the engine, since it applies what was applied to the same project
    /// before.
    #[error("operation {index} of the transaction was refused: {error}")]
    Refused {
        /// The place of the operation in the list.
        index: usize,
        /// Why it was refused.
        error: OpError,
    },
    /// An edit was refused, and what was done before it was taken back. It is written as
    /// `Refused` is, with the error of the edit (see [`EditError`]).
    #[error("operation {index} of the transaction was refused: {error}")]
    Edit {
        /// The place of the edit in the list.
        index: usize,
        /// Why it was refused.
        error: EditError,
    },
}

/// What a transaction of operations and edits did.
#[derive(Clone, Debug, PartialEq)]
pub struct Changed {
    /// What was done, as for a transaction of operations: the operations that the edits were
    /// worked out into are among those in `done`, in their place.
    pub applied: Applied,
    /// The places in the transaction of the edits that went less far than they were asked to,
    /// as far as they could.
    pub clamped: Vec<usize>,
    /// For each change of the transaction, in order, the places in `applied.done` of the
    /// operations that it was worked out into: one for an operation.
    pub parts: Vec<Range<usize>>,
    /// Whether it was put into the step of the gesture that it goes on, rather than made a step
    /// of its own.
    pub merged: bool,
}

/// Resolve transaction-local references before each change and observe its actual outputs.
/// Output checks run before the transaction is committed and can refuse it atomically.
pub trait ChangeMapper {
    /// Replace references using the changes already applied in this transaction.
    fn map(&mut self, change: Change, done: &[Op]) -> Change;

    /// Learn the change's outputs or refuse an output binding it could not fulfill.
    ///
    /// # Errors
    /// A requested output is absent. The entire transaction is rolled back.
    fn applied(&mut self, _expansion: &Expansion) -> Result<(), OpError> {
        Ok(())
    }
}

impl<F: FnMut(Change, &[Op]) -> Change> ChangeMapper for F {
    fn map(&mut self, change: Change, done: &[Op]) -> Change {
        self(change, done)
    }
}

impl Serialize for TransactionError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut value = match self {
            Self::Empty => serde_json::json!({ "reason": "empty" }),
            Self::Refused { index, error } => {
                let mut value = serde_json::to_value(error).map_err(serde::ser::Error::custom)?;
                if let Some(object) = value.as_object_mut() {
                    object.insert("index".into(), (*index).into());
                }
                value
            }
            Self::Edit { index, error } => {
                let mut value = serde_json::to_value(error).map_err(serde::ser::Error::custom)?;
                if let Some(object) = value.as_object_mut() {
                    object.insert("index".into(), (*index).into());
                }
                value
            }
        };
        if let Some(object) = value.as_object_mut() {
            object.insert("message".into(), self.to_string().into());
        }
        value.serialize(serializer)
    }
}

impl History {
    /// A history of `project` at revision 0, with nothing to undo or redo, that keeps
    /// [`HISTORY_MOST`] steps to undo.
    #[must_use]
    pub fn new(project: Project) -> Self {
        Self::with_most(project, HISTORY_MOST)
    }

    /// A history of `project` at revision 0, with nothing to undo or redo, that keeps `most`
    /// steps to undo. For tests, which let steps go without making a thousand of them.
    #[doc(hidden)]
    #[must_use]
    pub fn with_most(project: Project, most: usize) -> Self {
        Self {
            project,
            revision: 0,
            undo: VecDeque::new(),
            redo: Vec::new(),
            most,
            gesture: None,
            recorded: 0,
        }
    }

    /// The project as it is now.
    #[must_use]
    pub fn project(&self) -> &Project {
        &self.project
    }

    /// The revision of the project: 0 at first, and one more after each transaction, undo and
    /// redo that is applied.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Whether there is a step to undo.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether there is a step to redo.
    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Apply a transaction that `by` sent: its operations in order, all of them or none.
    ///
    /// A transaction that is applied makes a new revision and a step that can be undone, and
    /// the steps that could be redone are let go.
    ///
    /// # Errors
    /// The transaction is empty, or one of its operations is refused. Then the project is
    /// exactly as it was, `next_id` too, and so is the revision.
    pub fn apply(&mut self, ops: Vec<Op>, by: &str) -> Result<Applied, TransactionError> {
        self.apply_mapped(ops, by, |op, _| op)
    }

    /// Apply a transaction as [`History::apply`] does, with each operation passed through `map`
    /// just before it is applied. `map` is given the operation and those of the transaction that
    /// were applied before it, each as it was done, with the ids that the engine gave: so an
    /// operation can be made to name a thing that an earlier operation of the same transaction
    /// added, whose id was not known when the transaction was sent.
    ///
    /// The transaction is still applied whole or not at all, and what is undone and redone is
    /// the operations as `map` made them and as they were done.
    ///
    /// # Errors
    /// As [`History::apply`]: an operation as `map` made it is refused, and `index` is its place
    /// in `ops`.
    pub fn apply_mapped(
        &mut self,
        ops: Vec<Op>,
        by: &str,
        map: impl FnMut(Op, &[Op]) -> Op,
    ) -> Result<Applied, TransactionError> {
        if ops.is_empty() {
            return Err(TransactionError::Empty);
        }
        let mut ending = self.gesture.take();
        let nothing = ending
            .as_mut()
            .is_some_and(|gesture| left_nothing(gesture, &self.project));
        let (done, inverse) = match run(&mut self.project, ops, map) {
            Ok(ran) => ran,
            Err(error) => {
                self.gesture = ending;
                return Err(error);
            }
        };
        if nothing {
            self.undo.pop_back();
        }
        self.revision += 1;
        self.recorded = self.revision;
        self.push(Step {
            done: done.clone(),
            inverse,
            by: by.into(),
        });
        Ok(self.applied(done, by, by))
    }

    /// Apply a transaction of edits that `by` sent, as [`History::change`] does.
    ///
    /// # Errors
    /// As [`History::change`].
    pub fn edit(&mut self, edits: Vec<Edit>, by: &str) -> Result<Changed, TransactionError> {
        let changes = edits.into_iter().map(Change::Edit).collect();
        self.change(changes, by, |change, _| change)
    }

    /// Apply a transaction of operations and edits that `by` sent, in order, all of them or
    /// none, each passed through `map` first as [`History::apply_mapped`] passes operations.
    ///
    /// Each edit is worked out into operations at the moment that it is applied, against the
    /// project that the changes before it left. The step that is kept to be undone holds the
    /// operations only, as they were done: the transaction is undone and redone as one of
    /// operations is.
    ///
    /// # Errors
    /// The transaction is empty or does nothing, as a transaction of edits each of which finds
    /// nothing to do may; or one of its changes is refused, with its place in `changes`. Then
    /// the project is exactly as it was, `next_id` too, and so is the revision.
    pub fn change(
        &mut self,
        changes: Vec<Change>,
        by: &str,
        mut map: impl FnMut(Change, &[Op]) -> Change,
    ) -> Result<Changed, TransactionError> {
        self.change_with(changes, by, &mut map)
    }

    /// Apply changes as [`Self::change`], also checking their outputs before committing.
    ///
    /// # Errors
    /// As [`Self::change`], or a mapper refuses a change's outputs. All project and history
    /// state is preserved on refusal, including the id counter and redo steps.
    pub fn change_with(
        &mut self,
        changes: Vec<Change>,
        by: &str,
        mapper: &mut impl ChangeMapper,
    ) -> Result<Changed, TransactionError> {
        self.run_changes(changes, by, mapper, None)
    }

    /// Apply a transaction as [`History::change_with`] does, as part of the gesture `key` (see
    /// the module's docs): the first of a gesture is a step of its own, as any transaction is;
    /// one of the gesture that is going on is put into its step. Where `end` is set, the gesture
    /// ends with it.
    ///
    /// # Errors
    /// As [`History::change_with`]. A transaction that is refused changes nothing but this: a
    /// gesture `key` that is going on ends where `end` is set.
    pub fn change_in_gesture(
        &mut self,
        changes: Vec<Change>,
        by: &str,
        mapper: &mut impl ChangeMapper,
        key: &str,
        end: bool,
    ) -> Result<Changed, TransactionError> {
        let result = self.run_changes(changes, by, mapper, Some(key));
        if end
            && self
                .gesture
                .as_ref()
                .is_some_and(|gesture| gesture.key == key)
        {
            self.end_gesture();
        }
        result
    }

    /// The key of the gesture that is going on, and the revision before its first transaction.
    #[must_use]
    pub fn gesture(&self) -> Option<(&str, u64)> {
        self.gesture
            .as_ref()
            .map(|gesture| (gesture.key.as_str(), gesture.base))
    }

    /// Whether a transaction prepared by the project at `revision` is taken: one prepared at
    /// the revision that the project is at, or at one since which only the engine changed it by
    /// changes that are no step ([`History::apply_unrecorded`]: a proxy, the streams of media),
    /// which no transaction depends on; or, for a transaction of the gesture `gesture` that is
    /// going on, at any revision since the one before the gesture began, since only the gesture
    /// has changed the project since.
    #[must_use]
    pub fn takes_revision(&self, revision: u64, gesture: Option<&str>) -> bool {
        (self.recorded..=self.revision).contains(&revision)
            || self.gesture.as_ref().is_some_and(|open| {
                gesture == Some(open.key.as_str())
                    && (open.base..=self.revision).contains(&revision)
            })
    }

    /// End the gesture that is going on, if one is. Gives none where none was, and else whether
    /// it left a step: none where it ended with the project as it was before it.
    pub fn end_gesture(&mut self) -> Option<bool> {
        let mut gesture = self.gesture.take()?;
        let nothing = left_nothing(&mut gesture, &self.project);
        if nothing {
            self.undo.pop_back();
        }
        Some(!nothing)
    }

    /// Keep a step to undo, letting go of what could be redone and of the oldest step beyond
    /// the most that are kept.
    fn push(&mut self, step: Step) {
        self.redo.clear();
        self.undo.push_back(step);
        while self.undo.len() > self.most {
            self.undo.pop_front();
        }
    }

    /// Apply a transaction of changes, as a step of its own, or as part of the gesture `key`.
    fn run_changes(
        &mut self,
        changes: Vec<Change>,
        by: &str,
        mapper: &mut impl ChangeMapper,
        key: Option<&str>,
    ) -> Result<Changed, TransactionError> {
        if changes.is_empty() {
            return Err(TransactionError::Empty);
        }
        let goes_on = key.is_some()
            && !self.undo.is_empty()
            && self
                .gesture
                .as_ref()
                .is_some_and(|gesture| Some(gesture.key.as_str()) == key);
        let before = self.project.clone();
        let mut done: Vec<Op> = vec![];
        let mut inverse = vec![];
        let mut clamped = vec![];
        let mut parts = vec![];
        for (index, change) in changes.into_iter().enumerate() {
            let change = mapper.map(change, &done);
            match run_change(&mut self.project, change) {
                Ok(expansion) => {
                    if let Err(error) = mapper.applied(&expansion) {
                        self.project = before;
                        return Err(TransactionError::Refused { index, error });
                    }
                    parts.push(done.len()..done.len() + expansion.done.len());
                    if expansion.clamped {
                        clamped.push(index);
                    }
                    done.extend(expansion.done);
                    inverse.extend(expansion.inverse);
                }
                Err(error) => {
                    self.project = before;
                    return Err(match error {
                        ChangeError::Op(error) => TransactionError::Refused { index, error },
                        ChangeError::Edit(error) => TransactionError::Edit { index, error },
                    });
                }
            }
        }
        if done.is_empty() {
            self.project = before;
            return Err(TransactionError::Empty);
        }
        self.revision += 1;
        self.recorded = self.revision;
        if goes_on {
            let step = self
                .undo
                .back_mut()
                .expect("a gesture that goes on has its step");
            step.done.extend(done.iter().cloned());
            step.inverse.extend(inverse);
        } else {
            if let Some(mut ending) = self.gesture.take() {
                if left_nothing(&mut ending, &before) {
                    self.undo.pop_back();
                }
            }
            self.push(Step {
                done: done.clone(),
                inverse,
                by: by.into(),
            });
            if let Some(key) = key {
                self.gesture = Some(Gesture {
                    key: key.into(),
                    base: self.revision - 1,
                    before,
                });
            }
        }
        Ok(Changed {
            applied: self.applied(done, by, by),
            clamped,
            parts,
            merged: goes_on,
        })
    }

    /// Apply a transaction that is no step of the history, as the engine does when it sets the
    /// proxy that it made: it makes a new revision, as every change does, but nothing to undo,
    /// and the steps that could be redone are kept. So a change that no one asked for is never
    /// the step that an undo takes back, and never lets go of what a client could redo.
    ///
    /// Only an operation that no step of the history can be refused for may be applied so: one
    /// that adds or takes away no thing and changes nothing that an operation checks, as
    /// [`Op::SetProxy`]. The steps are then never refused for it. They are not quite as they
    /// would be without it, though: a step that adds media back, or sets a proxy, carries the
    /// media or the proxy as it was when the step was done, so that undoing the removal of media
    /// (or a client's `set_proxy`) brings back the proxy that it had then, which may be older
    /// than one the engine set since. The engine then finds the proxy stale or missing at its
    /// next start, or when it is read, and makes it again.
    ///
    /// # Errors
    /// As [`History::apply`].
    pub fn apply_unrecorded(
        &mut self,
        ops: Vec<Op>,
        by: &str,
    ) -> Result<Applied, TransactionError> {
        if ops.is_empty() {
            return Err(TransactionError::Empty);
        }
        let (done, _) = run(&mut self.project, ops, |op, _| op)?;
        self.revision += 1;
        // A gesture that ends where it began leaves no step: the project before it is taken to
        // have had this change too, which no step of the gesture names.
        // Where it cannot be (it names what the gesture added), the project before is left as
        // it was, and the gesture leaves a step however it ends.
        if let Some(gesture) = &mut self.gesture {
            let _ = run(&mut gesture.before, done.clone(), |op, _| op);
        }
        Ok(self.applied(done, by, by))
    }

    /// Take back the latest step, whoever made it, at the asking of `by`. Gives none if there
    /// is nothing to undo.
    ///
    /// `next_id` is not taken back, so the ids that the step made are never made again.
    ///
    /// # Errors
    /// Only through a bug of the engine, since the inverses are applied to the project that
    /// the step left; then the project and the revision are as they were, and the step can
    /// still be undone.
    pub fn undo(&mut self, by: &str) -> Option<Result<Applied, TransactionError>> {
        self.end_gesture();
        let step = self.undo.pop_back()?;
        let ops = step.inverse.iter().rev().cloned().collect();
        match run(&mut self.project, ops, |op, _| op) {
            Ok((done, _)) => {
                self.revision += 1;
                self.recorded = self.revision;
                let applied = self.applied(done, by, &step.by);
                self.redo.push(step);
                Some(Ok(applied))
            }
            Err(error) => {
                self.undo.push_back(step);
                Some(Err(error))
            }
        }
    }

    /// Do again the step that was undone last, at the asking of `by`, with the same ids as the
    /// first time. Gives none if there is nothing to redo.
    ///
    /// # Errors
    /// Only through a bug of the engine, since the operations are applied as they were done to
    /// the project that they were done to, but for `next_id`; then the project and the revision
    /// are as they were, and the step can still be redone.
    pub fn redo(&mut self, by: &str) -> Option<Result<Applied, TransactionError>> {
        self.end_gesture();
        let step = self.redo.pop()?;
        match run(&mut self.project, step.done.clone(), |op, _| op) {
            Ok((done, _)) => {
                self.revision += 1;
                self.recorded = self.revision;
                let applied = self.applied(done, by, &step.by);
                self.undo.push_back(step);
                Some(Ok(applied))
            }
            Err(error) => {
                self.redo.push(step);
                Some(Err(error))
            }
        }
    }

    /// What was done at the revision that the project is at now.
    fn applied(&self, done: Vec<Op>, by: &str, made_by: &str) -> Applied {
        Applied {
            revision: self.revision,
            done,
            by: by.into(),
            made_by: made_by.into(),
            next_id: self.project.next_id,
        }
    }
}

/// Whether a gesture ends with the project as it was before it, `now` being the project as the
/// gesture ends: but for `next_id`, which does not go back.
fn left_nothing(gesture: &mut Gesture, now: &Project) -> bool {
    gesture.before.next_id = now.next_id;
    gesture.before == *now
}

/// Apply `ops` to a project in order, each as `map` makes it from itself and those done before
/// it, and give back each as it was done and the inverse of each, in the same order. If one is
/// refused, the project is put back as it was before the first, from a copy that was kept.
///
/// So `next_id` goes back too, which is the one place where it does, and it is safe: the ids
/// that the operations before the refused one made were in no revision of the project, so no
/// client holds them as the ids of things. A refusal may name one of them, as the clip that a
/// later clip of the same transaction overlaps, but that clip was never in the project that
/// anyone was sent.
fn run(
    project: &mut Project,
    ops: Vec<Op>,
    mut map: impl FnMut(Op, &[Op]) -> Op,
) -> Result<(Vec<Op>, Vec<Op>), TransactionError> {
    let before = project.clone();
    let mut done = Vec::with_capacity(ops.len());
    let mut inverse = Vec::with_capacity(ops.len());
    for (index, op) in ops.into_iter().enumerate() {
        let op = map(op, &done);
        match apply_done(project, op) {
            Ok((did, undoes)) => {
                done.push(did);
                inverse.push(undoes);
            }
            Err(error) => {
                *project = before;
                return Err(TransactionError::Refused { index, error });
            }
        }
    }
    Ok((done, inverse))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::apply;
    use crate::project::{Clip, Timeline, Track, TrackKind};
    use crate::time::{Rate, Time};

    // The sample: timeline t1 at 25/1, with c1 on V1 and c2 on A1, each on frames [0, 100) and
    // cut from m1, which lasts 250 frames; next_id is 3.

    /// A clip with no id, cut from m1 from its start, at `start` for `length` frames.
    fn clip(start: i64, length: i64) -> Clip {
        let source_in = Time {
            ticks: 0,
            scale: 15360,
        };
        Clip::of_media("", "m1", source_in, start, length)
    }

    /// Put a clip with no id on `track` of t1.
    fn insert(track: &str, start: i64, length: i64) -> Op {
        Op::InsertClip {
            timeline: "t1".into(),
            track: track.into(),
            clip: clip(start, length),
        }
    }

    /// Move clip `id` of t1 to `start` on `track`.
    fn move_to(id: &str, track: &str, start: i64) -> Op {
        Op::MoveClip {
            timeline: "t1".into(),
            id: id.into(),
            track: track.into(),
            start,
        }
    }

    /// A timeline with no ids, with a track of picture with two clips on it.
    fn timeline() -> Op {
        Op::AddTimeline {
            timeline: Timeline {
                tracks: vec![
                    Track::new("", TrackKind::Video).with_clips(vec![clip(50, 10), clip(0, 10)])
                ],
                ..Timeline::new("", "Second", Rate { num: 25, den: 1 })
            },
            index: None,
        }
    }

    /// `project` with the `next_id` of `other`, to compare them across an undo.
    fn with_next_id_of(project: &Project, other: &Project) -> Project {
        Project {
            next_id: other.next_id,
            ..project.clone()
        }
    }

    #[test]
    fn a_transaction_whose_third_operation_is_refused_changes_nothing() {
        let mut history = History::new(Project::sample());
        let before = history.project().clone();
        let ops = vec![
            // Makes c3, and next_id 4.
            insert("V1", 100, 50),
            // Makes t4 with V1, c5 and c6, and next_id 7.
            timeline(),
            // Overlaps c1.
            insert("V1", 20, 10),
        ];
        let error = history.apply(ops, "agent").unwrap_err();
        assert_eq!(
            error,
            TransactionError::Refused {
                index: 2,
                error: OpError::Overlap { with: "c1".into() },
            }
        );
        assert_eq!(
            error.to_string(),
            "operation 2 of the transaction was refused: the clip covers frames that clip c1 covers"
        );
        assert_eq!(history.project(), &before);
        assert_eq!(history.project().next_id, 3);
        assert_eq!(history.revision(), 0);
        assert!(!history.can_undo());
        // The ids that the refused transaction made are made again by the next one.
        let applied = history.apply(vec![insert("V1", 100, 50)], "agent").unwrap();
        let Op::InsertClip { clip, .. } = &applied.done[0] else {
            panic!("{applied:?}");
        };
        assert_eq!(clip.id, "c3");
    }

    #[test]
    fn an_empty_transaction_is_refused_and_makes_no_revision() {
        let mut history = History::new(Project::sample());
        assert_eq!(
            history.apply(vec![], "desktop"),
            Err(TransactionError::Empty)
        );
        assert_eq!(history.project(), &Project::sample());
        assert_eq!(history.revision(), 0);
        assert!(!history.can_undo());
    }

    #[test]
    fn an_error_is_written_with_its_index_reason_message_and_fields() {
        let empty = serde_json::to_value(TransactionError::Empty).unwrap();
        assert_eq!(
            empty,
            serde_json::json!({
                "reason": "empty",
                "message": "the transaction has no operations",
            })
        );
        let refused = TransactionError::Refused {
            index: 2,
            error: OpError::Overlap { with: "c1".into() },
        };
        assert_eq!(
            serde_json::to_value(refused).unwrap(),
            serde_json::json!({
                "index": 2,
                "reason": "overlap",
                "with": "c1",
                "message": "operation 2 of the transaction was refused: the clip covers frames \
                            that clip c1 covers",
            })
        );
    }

    #[test]
    fn a_transaction_gives_back_what_it_did_with_the_ids_that_the_engine_gave() {
        let mut history = History::new(Project::sample());
        let applied = history
            .apply(vec![insert("V1", 100, 50), timeline()], "agent")
            .unwrap();
        assert_eq!(applied.revision, 1);
        assert_eq!(applied.by, "agent");
        assert_eq!(applied.made_by, "agent");
        let Op::InsertClip { clip, .. } = &applied.done[0] else {
            panic!("{applied:?}");
        };
        assert_eq!(clip.id, "c3");
        let Op::AddTimeline { timeline, index } = &applied.done[1] else {
            panic!("{applied:?}");
        };
        assert_eq!(*index, Some(1));
        assert_eq!(timeline.id, "t4");
        assert_eq!(timeline.tracks[0].id, "V1");
        let ids: Vec<&str> = timeline.tracks[0]
            .clips
            .iter()
            .map(|c| c.id.as_str())
            .collect();
        // Given in the order of the list, and then put in the order of their start.
        assert_eq!(ids, ["c6", "c5"]);

        // A client that is given next_id and applies what was done to its copy gets the same
        // project. Without next_id, c3 would be refused as an id that the engine may make later.
        assert_eq!(applied.next_id, 7);
        let mut refused = Project::sample();
        assert_eq!(
            apply(&mut refused, applied.done[0].clone()),
            Err(OpError::Reserved { id: "c3".into() })
        );
        let mut copy = Project::sample();
        copy.next_id = applied.next_id;
        for op in applied.done {
            apply(&mut copy, op).unwrap();
        }
        assert_eq!(&copy, history.project());
    }

    #[test]
    fn undo_and_then_redo_give_back_the_same_projects_with_the_same_ids() {
        let mut history = History::new(Project::sample());
        let before = history.project().clone();
        let first = history
            .apply(
                vec![insert("V1", 100, 50), timeline(), move_to("c1", "V1", 160)],
                "desktop",
            )
            .unwrap();
        let after = history.project().clone();
        assert_eq!(after.next_id, 7);

        let undone = history.undo("desktop").unwrap().unwrap();
        assert_eq!(undone.revision, 2);
        assert_eq!(history.project(), &with_next_id_of(&before, &after));
        // What the undo did is the inverses, last first.
        assert_eq!(
            undone.done,
            [
                move_to("c1", "V1", 0),
                Op::RemoveTimeline { id: "t4".into() },
                Op::RemoveClip {
                    timeline: "t1".into(),
                    id: "c3".into(),
                },
            ]
        );
        assert!(history.can_redo());

        let redone = history.redo("agent").unwrap().unwrap();
        assert_eq!(redone.revision, 3);
        assert_eq!(redone.done, first.done);
        assert_eq!(redone.by, "agent");
        assert_eq!(redone.made_by, "desktop");
        assert_eq!(history.project(), &after);
        assert!(!history.can_redo());

        // And once more, to be sure that a step that was redone is undone as the first time.
        history.undo("agent").unwrap().unwrap();
        assert_eq!(history.project(), &with_next_id_of(&before, &after));
        history.redo("agent").unwrap().unwrap();
        assert_eq!(history.project(), &after);
        assert_eq!(history.revision(), 5);
    }

    /// What a test's `map` is given: the operation, and the operations done before it.
    type Seen = (Op, Vec<Op>);

    #[test]
    fn a_mapped_operation_can_name_what_an_earlier_one_of_its_transaction_added() {
        let mut history = History::new(Project::sample());
        let mut seen: Vec<Seen> = Vec::new();
        // The second operation moves "the clip that the first one added", whose id the client
        // could not know.
        let ops = vec![insert("V1", 100, 50), move_to("?", "V1", 300)];
        let applied = history
            .apply_mapped(ops, "agent", |op, done| {
                seen.push((op.clone(), done.to_vec()));
                match op {
                    Op::MoveClip {
                        timeline,
                        track,
                        start,
                        ..
                    } => {
                        let Op::InsertClip { clip, .. } = &done[0] else {
                            panic!("{done:?}");
                        };
                        Op::MoveClip {
                            timeline,
                            id: clip.id.clone(),
                            track,
                            start,
                        }
                    }
                    op => op,
                }
            })
            .unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0], (insert("V1", 100, 50), vec![]));
        assert_eq!(seen[1].1, applied.done[..1]);
        assert_eq!(applied.done[1], move_to("c3", "V1", 300));
        assert_eq!(history.project().timelines[0].tracks[0].clips[1].start, 300);

        // What is undone and redone is the operations as they were mapped.
        let undone = history.undo("agent").unwrap().unwrap();
        assert_eq!(undone.done[0], move_to("c3", "V1", 100));
        let redone = history.redo("agent").unwrap().unwrap();
        assert_eq!(redone.done, applied.done);
    }

    #[test]
    fn a_mapped_transaction_that_is_refused_changes_nothing() {
        let mut history = History::new(Project::sample());
        let before = history.project().clone();
        let mut calls = 0;
        // The mapping makes the second operation overlap c1, so it is refused after the first
        // was applied.
        let error = history
            .apply_mapped(
                vec![insert("V1", 100, 50), insert("V1", 300, 10)],
                "agent",
                |op, done| {
                    calls += 1;
                    if done.is_empty() {
                        op
                    } else {
                        insert("V1", 20, 10)
                    }
                },
            )
            .unwrap_err();
        assert_eq!(calls, 2);
        assert_eq!(
            error,
            TransactionError::Refused {
                index: 1,
                error: OpError::Overlap { with: "c1".into() },
            }
        );
        assert_eq!(history.project(), &before);
        assert_eq!(history.revision(), 0);
        assert!(!history.can_undo());
    }

    #[test]
    fn a_new_transaction_after_an_undo_lets_go_of_what_could_be_redone() {
        let mut history = History::new(Project::sample());
        history.apply(vec![insert("V1", 100, 50)], "agent").unwrap();
        history.undo("agent").unwrap().unwrap();
        assert!(history.can_redo());
        history
            .apply(vec![insert("A1", 100, 50)], "desktop")
            .unwrap();
        assert!(!history.can_redo());
        assert_eq!(history.redo("agent"), None);
        assert_eq!(history.revision(), 3);
        // The clip on A1 is c4, not the c3 that was undone.
        let clips = &history.project().timelines[0].tracks[1].clips;
        assert_eq!(clips[1].id, "c4");
    }

    #[test]
    fn the_oldest_step_is_let_go_beyond_the_most_that_are_kept() {
        let mut history = History::with_most(Project::sample(), 2);
        history.apply(vec![insert("V1", 100, 10)], "agent").unwrap();
        let first = history.project().clone();
        history.apply(vec![insert("V1", 110, 10)], "agent").unwrap();
        history.apply(vec![insert("V1", 120, 10)], "agent").unwrap();
        history.undo("agent").unwrap().unwrap();
        history.undo("agent").unwrap().unwrap();
        assert!(!history.can_undo());
        assert_eq!(history.undo("agent"), None);
        assert_eq!(
            history.project(),
            &with_next_id_of(&first, history.project())
        );
        assert_eq!(history.revision(), 5);
    }

    #[test]
    fn as_many_as_the_most_steps_are_kept() {
        let mut history = History::new(Project::sample());
        for at in 0..=HISTORY_MOST {
            let start = 100 + i64::try_from(at).unwrap();
            history
                .apply(vec![insert("A1", start, 1)], "agent")
                .unwrap();
        }
        let mut undone = 0;
        while let Some(result) = history.undo("agent") {
            result.unwrap();
            undone += 1;
        }
        assert_eq!(undone, HISTORY_MOST);
        // The first step was let go, so its clip is still there.
        assert_eq!(history.project().timelines[0].tracks[1].clips.len(), 2);
    }

    #[test]
    fn a_change_keeps_as_many_as_the_most_steps_and_lets_the_oldest_go() {
        let mut history = History::with_most(Project::sample(), 2);
        for start in [100, 110, 120] {
            let change = Change::Op(insert("V1", start, 10));
            history
                .change(vec![change], "agent", |change, _| change)
                .unwrap();
        }
        history.undo("agent").unwrap().unwrap();
        history.undo("agent").unwrap().unwrap();
        assert_eq!(history.undo("agent"), None);
        // The first step was let go, so its clip is still there.
        assert_eq!(history.project().timelines[0].tracks[0].clips.len(), 2);
    }

    #[test]
    fn a_change_that_sets_media_to_something_new_is_a_step_and_one_that_sets_it_as_it_is_none() {
        let mut history = History::new(Project::sample());
        let rename = |name: &str| {
            Change::Op(Op::SetMedia {
                id: "m1".into(),
                still: None,
                name: Some(name.into()),
                bin: None,
                mark_in: None,
                mark_out: None,
            })
        };
        let named = history.project().media[0].name.clone();
        history
            .change(vec![rename("Beach")], "agent", |change, _| change)
            .unwrap();
        assert_eq!(history.project().media[0].name, "Beach");
        assert_eq!(
            history.change(vec![rename("Beach")], "agent", |change, _| change),
            Err(TransactionError::Empty)
        );
        history.undo("agent").unwrap().unwrap();
        assert_eq!(history.project().media[0].name, named);
    }

    #[test]
    fn undo_all_the_way_back_and_then_nothing() {
        let mut history = History::new(Project::sample());
        assert_eq!(history.undo("agent"), None);
        history.apply(vec![insert("V1", 100, 50)], "agent").unwrap();
        history.apply(vec![timeline()], "desktop").unwrap();
        assert!(history.undo("agent").is_some());
        assert!(history.undo("agent").is_some());
        assert_eq!(history.undo("agent"), None);
        assert_eq!(history.revision(), 4);
        assert_eq!(
            history.project(),
            &with_next_id_of(&Project::sample(), history.project())
        );
        // Nothing to undo makes no revision.
        assert_eq!(history.undo("desktop"), None);
        assert_eq!(history.revision(), 4);
    }

    #[test]
    fn the_steps_of_the_agent_and_of_the_desktop_are_undone_latest_first() {
        let mut history = History::new(Project::sample());
        history.apply(vec![insert("V1", 100, 10)], "agent").unwrap();
        let after_agent = history.project().clone();
        history
            .apply(vec![insert("A1", 100, 10)], "desktop")
            .unwrap();
        let after_desktop = history.project().clone();
        history
            .apply(vec![move_to("c1", "V1", 200)], "agent")
            .unwrap();

        // The desktop's undo takes back the agent's latest change, not its own.
        let undone = history.undo("desktop").unwrap().unwrap();
        assert_eq!(
            (undone.by.as_str(), undone.made_by.as_str()),
            ("desktop", "agent")
        );
        assert_eq!(history.project(), &after_desktop);
        // The agent's undo takes back the desktop's change.
        let undone = history.undo("agent").unwrap().unwrap();
        assert_eq!(
            (undone.by.as_str(), undone.made_by.as_str()),
            ("agent", "desktop")
        );
        assert_eq!(
            history.project(),
            &with_next_id_of(&after_agent, &after_desktop)
        );
        // Redo does them again in the order in which they were made.
        let redone = history.redo("agent").unwrap().unwrap();
        assert_eq!(redone.made_by, "desktop");
        let redone = history.redo("agent").unwrap().unwrap();
        assert_eq!(redone.made_by, "agent");
        assert_eq!(history.revision(), 7);
    }

    #[test]
    fn a_change_of_the_engine_is_no_step_and_lets_go_of_nothing_to_redo() {
        let mut history = History::new(Project::sample());
        history.apply(vec![insert("V1", 100, 10)], "agent").unwrap();
        history
            .apply(vec![move_to("c1", "V1", 200)], "desktop")
            .unwrap();
        history.undo("desktop").unwrap().unwrap();
        let proxy = crate::project::Proxy {
            path: "/cache/m1.mov".into(),
            width: 960,
            height: 540,
            codec: "hevc".into(),
            digest_of_source: "0".repeat(64),
        };
        let set = Op::SetProxy {
            media: "m1".into(),
            proxy: Some(proxy.clone()),
        };
        let applied = history
            .apply_unrecorded(vec![set.clone()], "engine")
            .unwrap();
        assert_eq!(applied.revision, 4);
        assert_eq!(applied.done, [set]);
        assert_eq!(
            (applied.by.as_str(), applied.made_by.as_str()),
            ("engine", "engine")
        );
        // The move can still be redone, and the undo takes back the agent's clip, not the proxy.
        assert!(history.can_redo());
        history.redo("desktop").unwrap().unwrap();
        assert_eq!(history.project().timelines[0].tracks[0].clips[1].start, 200);
        history.undo("desktop").unwrap().unwrap();
        history.undo("desktop").unwrap().unwrap();
        assert!(!history.can_undo());
        assert_eq!(history.project().media[0].proxy, Some(proxy));
        assert_eq!(
            history.apply_unrecorded(vec![], "engine"),
            Err(TransactionError::Empty)
        );
    }

    #[test]
    fn the_streams_that_the_engine_reads_are_no_step_and_setting_them_again_is_none() {
        let mut history = History::new(Project::sample());
        history.apply(vec![insert("V1", 100, 10)], "agent").unwrap();
        let read = Op::SetMediaStreams {
            id: "m1".into(),
            layout: Some("stereo".into()),
            streams: Some(vec![crate::project::AudioStream {
                index: 0,
                channels: 2,
                layout: Some("stereo".into()),
            }]),
        };
        let applied = history
            .apply_unrecorded(vec![read.clone()], "engine")
            .unwrap();
        assert_eq!(applied.done, std::slice::from_ref(&read));
        assert_eq!(applied.made_by, "engine");
        // The undo takes back the agent's clip, and the streams stay read.
        history.undo("desktop").unwrap().unwrap();
        assert!(!history.can_undo());
        let audio = history.project().media[0].facts.audio.clone().unwrap();
        assert_eq!(audio.layout.as_deref(), Some("stereo"));
        assert_eq!(audio.streams.map(|s| s.len()), Some(1));
        // Set to what they are by a client, they change nothing and make no step.
        assert_eq!(
            history.change(vec![Change::Op(read)], "agent", |change, _| change),
            Err(TransactionError::Empty)
        );
    }

    /// The streams of m1, as the engine reads them of an older project.
    fn streams_read() -> Op {
        Op::SetMediaStreams {
            id: "m1".into(),
            layout: Some("stereo".into()),
            streams: Some(vec![]),
        }
    }

    #[test]
    fn a_transaction_prepared_before_changes_of_the_engine_alone_is_taken() {
        let mut history = History::new(Project::sample());
        history.apply(vec![insert("V1", 100, 10)], "agent").unwrap();
        let prepared = history.revision();
        history
            .apply_unrecorded(vec![streams_read()], "engine")
            .unwrap();
        // Only the engine changed the project since: what was prepared is taken.
        assert!(history.takes_revision(prepared, None));
        assert!(history.takes_revision(prepared + 1, None));
        // Not one prepared before a step of a client.
        assert!(!history.takes_revision(prepared - 1, None));
        history
            .apply(vec![move_to("c1", "V1", 300)], "desktop")
            .unwrap();
        assert!(!history.takes_revision(prepared, None));
        assert!(history.takes_revision(history.revision(), None));
    }

    mod gestures {
        use super::super::*;
        use super::{insert, move_to, streams_read, with_next_id_of};

        #[test]
        fn a_gesture_put_back_leaves_no_step_though_the_engine_changed_the_project_meanwhile() {
            let mut history = History::new(Project::sample());
            history.apply(vec![insert("A1", 100, 10)], "agent").unwrap();
            let start = start_of_c1(&history);
            drag(&mut history, "1:g1", 200, false).unwrap();
            history
                .apply_unrecorded(vec![streams_read()], "engine")
                .unwrap();
            // Put back where it began (Escape): no step, though the streams were read.
            drag(&mut history, "1:g1", start, true).unwrap();
            let undone = history.undo("desktop").unwrap().unwrap();
            assert!(undone
                .done
                .iter()
                .all(|op| !matches!(op, Op::MoveClip { .. })));
            assert!(!history.can_undo());
        }

        /// Move c1 on V1 to `start`, as part of the gesture `key`.
        fn drag(
            history: &mut History,
            key: &str,
            start: i64,
            end: bool,
        ) -> Result<Changed, TransactionError> {
            history.change_in_gesture(
                vec![Change::Op(move_to("c1", "V1", start))],
                "desktop",
                &mut |change, _: &[Op]| change,
                key,
                end,
            )
        }

        fn start_of_c1(history: &History) -> i64 {
            let clips = &history.project().timelines[0].tracks[0].clips;
            clips.iter().find(|clip| clip.id == "c1").unwrap().start
        }

        #[test]
        fn thirty_changes_of_a_gesture_are_one_step_that_an_undo_takes_back_whole() {
            let mut history = History::new(Project::sample());
            history.apply(vec![insert("A1", 100, 10)], "agent").unwrap();
            let before = history.project().clone();
            for (at, start) in (200..230).enumerate() {
                let changed = drag(&mut history, "1:g1", start, false).unwrap();
                // Each is seen at once: a revision of its own, the project as it says.
                assert_eq!(changed.applied.revision, 2 + at as u64);
                assert_eq!(changed.merged, at > 0);
                assert_eq!(start_of_c1(&history), start);
            }
            assert_eq!(history.gesture(), Some(("1:g1", 1)));
            assert_eq!(history.end_gesture(), Some(true));
            let after = history.project().clone();
            let undone = history.undo("agent").unwrap().unwrap();
            assert_eq!(undone.revision, 32);
            assert_eq!(undone.made_by, "desktop");
            // The inverses, last first, back to the project before the gesture.
            assert_eq!(undone.done.len(), 30);
            assert_eq!(history.project(), &with_next_id_of(&before, &after));
            let redone = history.redo("agent").unwrap().unwrap();
            assert_eq!(redone.done.len(), 30);
            assert_eq!(history.project(), &after);
            // One more undo takes the gesture back, and the next the agent's clip.
            history.undo("agent").unwrap().unwrap();
            history.undo("agent").unwrap().unwrap();
            assert!(!history.can_undo());
        }

        #[test]
        fn anything_else_done_ends_a_gesture() {
            let mut history = History::new(Project::sample());
            drag(&mut history, "1:g1", 200, false).unwrap();
            drag(&mut history, "1:g1", 210, false).unwrap();
            // Another client's transaction in between.
            history.apply(vec![insert("A1", 100, 10)], "agent").unwrap();
            assert_eq!(history.gesture(), None);
            // The same key again begins another gesture, and another step.
            let changed = drag(&mut history, "1:g1", 220, false).unwrap();
            assert!(!changed.merged);
            history.undo("desktop").unwrap().unwrap();
            assert_eq!(start_of_c1(&history), 210);
            history.undo("desktop").unwrap().unwrap();
            assert_eq!(history.project().timelines[0].tracks[1].clips.len(), 1);
            history.undo("desktop").unwrap().unwrap();
            assert_eq!(start_of_c1(&history), 0);

            // An undo ends one too, and so do a redo and another gesture.
            let mut history = History::new(Project::sample());
            drag(&mut history, "1:g1", 200, false).unwrap();
            history.undo("desktop").unwrap().unwrap();
            assert!(!drag(&mut history, "1:g1", 300, false).unwrap().merged);
            assert!(!drag(&mut history, "1:g2", 310, false).unwrap().merged);
            assert_eq!(history.gesture(), Some(("1:g2", 3)));
            history.undo("desktop").unwrap().unwrap();
            assert_eq!(start_of_c1(&history), 300);
            history.redo("desktop").unwrap().unwrap();
            assert_eq!(history.gesture(), None);
        }

        #[test]
        fn a_gesture_that_ends_where_it_began_leaves_no_step() {
            let mut history = History::new(Project::sample());
            history.apply(vec![insert("A1", 100, 10)], "agent").unwrap();
            history.undo("agent").unwrap().unwrap();
            assert!(history.can_redo());
            drag(&mut history, "1:g1", 200, false).unwrap();
            let last = drag(&mut history, "1:g1", 0, true).unwrap();
            // Seen as any change, but no step to undo, and what could be redone is let go.
            assert_eq!(last.applied.revision, 4);
            assert_eq!(history.gesture(), None);
            assert!(!history.can_undo());
            assert!(!history.can_redo());
            // One ended by another transaction leaves none either.
            drag(&mut history, "1:g2", 50, false).unwrap();
            drag(&mut history, "1:g2", 0, false).unwrap();
            history.apply(vec![insert("A1", 100, 10)], "agent").unwrap();
            history.undo("agent").unwrap().unwrap();
            assert!(!history.can_undo());
            assert_eq!(history.end_gesture(), None);
        }

        #[test]
        fn a_gesture_takes_a_revision_since_it_began_and_others_take_only_the_latest() {
            let mut history = History::new(Project::sample());
            history.apply(vec![insert("A1", 100, 10)], "agent").unwrap();
            for start in [200, 210, 220] {
                drag(&mut history, "1:g1", start, false).unwrap();
            }
            assert_eq!(history.revision(), 4);
            for revision in 1..=4 {
                assert!(history.takes_revision(revision, Some("1:g1")));
            }
            assert!(!history.takes_revision(0, Some("1:g1")));
            assert!(!history.takes_revision(5, Some("1:g1")));
            assert!(!history.takes_revision(3, Some("1:g2")));
            assert!(!history.takes_revision(3, None));
            assert!(history.takes_revision(4, None));
            history.end_gesture();
            assert!(!history.takes_revision(3, Some("1:g1")));
        }

        #[test]
        fn a_refused_transaction_of_a_gesture_changes_nothing_and_it_goes_on() {
            let mut history = History::new(Project::sample());
            drag(&mut history, "1:g1", 200, false).unwrap();
            let before = history.project().clone();
            let refused = history.change_in_gesture(
                vec![Change::Op(move_to("c9", "V1", 300))],
                "desktop",
                &mut |change, _: &[Op]| change,
                "1:g1",
                false,
            );
            assert!(refused.is_err());
            assert_eq!(history.project(), &before);
            assert_eq!(history.revision(), 1);
            assert!(drag(&mut history, "1:g1", 210, false).unwrap().merged);
            // A refused transaction of another gesture does not end this one.
            assert!(history
                .change_in_gesture(vec![], "agent", &mut |c, _: &[Op]| c, "2:g", false)
                .is_err());
            assert!(drag(&mut history, "1:g1", 220, true).unwrap().merged);
            assert_eq!(history.gesture(), None);
            history.undo("desktop").unwrap().unwrap();
            assert_eq!(start_of_c1(&history), 0);
            assert!(!history.can_undo());
        }

        #[test]
        fn a_change_of_the_engine_does_not_end_a_gesture() {
            let mut history = History::new(Project::sample());
            drag(&mut history, "1:g1", 200, false).unwrap();
            let proxy = Op::SetProxy {
                media: "m1".into(),
                proxy: None,
            };
            history.apply_unrecorded(vec![proxy], "engine").unwrap();
            assert!(drag(&mut history, "1:g1", 210, false).unwrap().merged);
            history.undo("desktop").unwrap().unwrap();
            assert_eq!(start_of_c1(&history), 0);
        }
    }

    mod edits {
        use super::super::*;
        use super::with_next_id_of;
        use crate::edit::fixture::{self, cues, lock};
        use crate::edit::{Change, CueItem, Edit, EditError};
        use crate::project::TrackKind;

        fn add_cue(start: i64) -> Edit {
            Edit::AddCue {
                timeline: "t1".into(),
                track: "S1".into(),
                start,
                length: 10,
                text: "again".into(),
            }
        }

        #[test]
        fn a_transaction_of_edits_is_one_step_that_is_undone_and_redone() {
            let start = fixture::project();
            let mut history = History::new(start.clone());
            let item = CueItem {
                start: 0,
                length: 5,
                text: "one".into(),
            };
            let edits = vec![
                Edit::CuesFrom {
                    timeline: "t1".into(),
                    track: "S1".into(),
                    items: vec![item],
                },
                Edit::RemoveTrack {
                    timeline: "t1".into(),
                    track: "A2".into(),
                },
            ];
            let changed = history.edit(edits, "agent").unwrap();
            let after = history.project().clone();
            assert_eq!(cues(&after, "S1"), [("q10".into(), 0, 5)]);
            // Two subtitles taken away and one put on, then the track taken away.
            assert_eq!(changed.parts, [0..3, 3..4]);
            assert_eq!(changed.applied.done.len(), 4);
            assert_eq!(history.revision(), 1);
            history.undo("desktop").unwrap().unwrap();
            assert_eq!(history.project(), &with_next_id_of(&start, &after));
            assert!(!history.can_undo());
            history.redo("desktop").unwrap().unwrap();
            assert_eq!(history.project(), &after);
        }

        #[test]
        fn a_later_edit_names_what_an_earlier_one_made() {
            let mut history = History::new(fixture::project());
            let set = Edit::SetCue {
                timeline: "t1".into(),
                id: "q10".into(),
                start: Some(60),
                length: None,
                text: None,
            };
            history.edit(vec![add_cue(50), set], "agent").unwrap();
            assert_eq!(cues(history.project(), "S1")[1], ("q10".into(), 60, 10));
        }

        #[test]
        fn operations_and_edits_are_applied_in_one_transaction() {
            let mut history = History::new(fixture::project());
            let track = Edit::AddTrack {
                timeline: "t1".into(),
                kind: TrackKind::Subtitle,
                index: None,
                name: None,
                language: None,
            };
            let both = vec![Change::Edit(track), Change::Op(lock("S2"))];
            let changed = history
                .change(both, "desktop", |change, done| {
                    assert!(done.len() <= 1);
                    change
                })
                .unwrap();
            assert_eq!(changed.parts, [0..1, 1..2]);
            assert!(matches!(changed.applied.done[1], Op::SetTrack { .. }));
        }

        #[test]
        fn a_refused_edit_takes_back_the_whole_transaction_and_names_its_place() {
            let start = fixture::project();
            let mut history = History::new(start.clone());
            let changes = vec![
                Change::Edit(add_cue(50)),
                Change::Op(lock("S1")),
                Change::Edit(add_cue(70)),
            ];
            let error = history
                .change(changes, "agent", |change, _| change)
                .unwrap_err();
            assert_eq!(
                error,
                TransactionError::Edit {
                    index: 2,
                    error: EditError::Locked { track: "S1".into() },
                }
            );
            let written = serde_json::to_value(&error).unwrap();
            assert_eq!(written["index"], 2);
            assert_eq!(written["reason"], "locked");
            assert_eq!(
                written["message"],
                "operation 2 of the transaction was refused: track S1 is locked"
            );
            assert_eq!(history.project(), &start);
            assert_eq!(history.revision(), 0);
            assert_eq!(history.edit(vec![], "agent"), Err(TransactionError::Empty));
        }
    }

    mod ripples {
        use super::super::*;
        use super::with_next_id_of;
        use crate::edit::fixture::{self, clips, lock, with};
        use crate::edit::{Change, Edge, Edit, EditError};

        fn ripple_delete(clip: &str) -> Edit {
            Edit::RippleDelete {
                timeline: "t1".into(),
                clips: vec![clip.into()],
                linked: true,
                grouped: false,
            }
        }

        fn trim(clip: &str, delta: i64) -> Edit {
            Edit::Trim {
                timeline: "t1".into(),
                clip: clip.into(),
                edge: Edge::Out,
                delta,
                ripple: false,
                linked: false,
            }
        }

        #[test]
        fn a_transaction_of_edits_is_one_step_of_operations_that_is_undone_and_redone() {
            let start = with(fixture::project(), vec![lock("V2"), lock("S1")]);
            let mut history = History::new(start.clone());
            let changed = history
                .edit(vec![ripple_delete("c1"), ripple_delete("c3")], "agent")
                .unwrap();
            let after = history.project().clone();
            assert_eq!(clips(&after, "V1"), []);
            // Two removals and two moves for the first edit, two removals for the second.
            assert_eq!(changed.parts, [0..4, 4..6]);
            assert_eq!(changed.applied.done.len(), 6);
            assert_eq!(changed.clamped, Vec::<usize>::new());
            assert_eq!(history.revision(), 1);
            history.undo("desktop").unwrap().unwrap();
            assert_eq!(history.project(), &with_next_id_of(&start, &after));
            assert!(!history.can_undo());
            history.redo("desktop").unwrap().unwrap();
            assert_eq!(history.project(), &after);
        }

        #[test]
        fn a_later_edit_is_worked_out_against_what_the_earlier_ones_left() {
            let mut history = History::new(fixture::project());
            // The first trim takes c3 to 170; the second goes from there to the end of m1.
            let changed = history
                .edit(vec![trim("c3", 20), trim("c3", 50)], "agent")
                .unwrap();
            assert_eq!(
                clips(history.project(), "V1")[1],
                ("c3".into(), 100, 100, 150)
            );
            assert_eq!(changed.clamped, [1]);
        }

        #[test]
        fn operations_and_edits_are_applied_in_one_transaction() {
            let mut history = History::new(fixture::project());
            let locks = [lock("V2"), lock("S1")].map(Change::Op);
            let delete = Change::Edit(ripple_delete("c3"));
            // c3 would be blocked by c5 on V2 and q7 on S1, but they are locked first.
            let all = [locks.to_vec(), vec![delete]].concat();
            let changed = history.change(all, "desktop", |change, _| change).unwrap();
            assert_eq!(changed.parts[..2], [0..1, 1..2]);
            assert!(matches!(changed.applied.done[0], Op::SetTrack { .. }));
        }

        #[test]
        fn a_refused_edit_takes_back_the_whole_transaction_and_names_its_place() {
            let start = fixture::project();
            let mut history = History::new(start.clone());
            let error = history
                .edit(vec![trim("c3", 20), ripple_delete("c3")], "agent")
                .unwrap_err();
            assert_eq!(
                error,
                TransactionError::Edit {
                    index: 1,
                    error: EditError::RippleBlocked {
                        track: "V2".into(),
                        clip: "c5".into(),
                    },
                }
            );
            let written = serde_json::to_value(&error).unwrap();
            assert_eq!(written["index"], 1);
            assert_eq!(written["reason"], "ripple_blocked");
            assert_eq!(
                written["message"],
                "operation 1 of the transaction was refused: c5 on track V2 is in the way of the \
                 ripple"
            );
            assert_eq!(history.project(), &start);
            assert_eq!(history.revision(), 0);
            assert_eq!(history.edit(vec![], "agent"), Err(TransactionError::Empty));
        }
    }
}
