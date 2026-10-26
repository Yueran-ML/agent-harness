//! Transactions on a project, and the one line of undo that every client of the engine shares.
//!
//! The engine is the only writer of a project. Its clients, the desktop and the agent, send it
//! transactions, each a list of operations that is applied whole or not at all. Each
//! transaction that is applied makes a new revision of the project, and so does each undo and
//! each redo. The steps that can be undone are kept in the order in which they were done,
//! whoever made them: an undo takes back the latest of them, whether the desktop or the agent
//! made it.

use std::collections::VecDeque;

use crate::ops::{apply, apply_done, Op, OpError};
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
}

/// A transaction that was done: what was done, with the ids that the engine gave, and what
/// takes it back.
#[derive(Clone, Debug, PartialEq, Eq)]
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
#[derive(Clone, Debug, PartialEq, Eq)]
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
}

/// Why a transaction, an undo or a redo was refused. The project and its revision are then
/// exactly as they were.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TransactionError {
    /// The transaction has no operations. Nothing is done, and no revision is made: a
    /// transaction that changes nothing is taken for a mistake of the client.
    #[error("the transaction has no operations")]
    Empty,
    /// An operation was refused, and those before it were taken back. `index` is its place in
    /// the list that was applied. An undo or a redo is refused only through a bug of the
    /// engine, since it applies what was applied to the same project before.
    #[error("operation {index} of the transaction was refused: {error}")]
    Refused {
        /// The place of the operation in the list.
        index: usize,
        /// Why it was refused.
        error: OpError,
    },
}

impl History {
    /// A history of `project` at revision 0, with nothing to undo or redo, that keeps
    /// [`HISTORY_MOST`] steps to undo.
    #[must_use]
    pub fn new(project: Project) -> Self {
        Self::with_most(project, HISTORY_MOST)
    }

    /// A history of `project` at revision 0, with nothing to undo or redo, that keeps `most`
    /// steps to undo.
    #[must_use]
    pub fn with_most(project: Project, most: usize) -> Self {
        Self {
            project,
            revision: 0,
            undo: VecDeque::new(),
            redo: Vec::new(),
            most,
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
        if ops.is_empty() {
            return Err(TransactionError::Empty);
        }
        let (done, inverse) = run(&mut self.project, ops)?;
        self.revision += 1;
        self.redo.clear();
        self.undo.push_back(Step {
            done: done.clone(),
            inverse,
            by: by.into(),
        });
        while self.undo.len() > self.most {
            self.undo.pop_front();
        }
        Ok(Applied {
            revision: self.revision,
            done,
            by: by.into(),
            made_by: by.into(),
        })
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
        let step = self.undo.pop_back()?;
        let ops = step.inverse.iter().rev().cloned().collect();
        match run(&mut self.project, ops) {
            Ok((done, _)) => {
                self.revision += 1;
                let made_by = step.by.clone();
                self.redo.push(step);
                Some(Ok(Applied {
                    revision: self.revision,
                    done,
                    by: by.into(),
                    made_by,
                }))
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
        let step = self.redo.pop()?;
        match run(&mut self.project, step.done.clone()) {
            Ok((done, _)) => {
                self.revision += 1;
                let made_by = step.by.clone();
                self.undo.push_back(step);
                Some(Ok(Applied {
                    revision: self.revision,
                    done,
                    by: by.into(),
                    made_by,
                }))
            }
            Err(error) => {
                self.redo.push(step);
                Some(Err(error))
            }
        }
    }
}

/// Apply `ops` to a project in order, and give back each as it was done and the inverse of
/// each, in the same order. If one is refused, those before it are taken back, last first, and
/// the project is left exactly as it was.
///
/// The inverses do not take `next_id` back, so it is set back to what it was. This is the one
/// place where `next_id` goes back, and it is safe: the ids that the operations before the
/// refused one made were in no revision of the project, so no client holds them as the ids of
/// things. A refusal may name one of them, as the clip that a later clip of the same
/// transaction overlaps, but that clip was never in the project that anyone was sent.
///
/// # Panics
/// If the inverse of an operation that was just applied is refused on the project that the
/// operation left. Every operation promises that it is not, so this is a bug; and the project
/// would then be one that no client knows of, which is not to be written or sent.
fn run(project: &mut Project, ops: Vec<Op>) -> Result<(Vec<Op>, Vec<Op>), TransactionError> {
    let next_id = project.next_id;
    let mut done = Vec::with_capacity(ops.len());
    let mut inverse = Vec::with_capacity(ops.len());
    for (index, op) in ops.into_iter().enumerate() {
        match apply_done(project, op) {
            Ok((did, undoes)) => {
                done.push(did);
                inverse.push(undoes);
            }
            Err(error) => {
                for (at, op) in inverse.into_iter().enumerate().rev() {
                    if let Err(refusal) = apply(project, op.clone()) {
                        panic!(
                            "the inverse {op:?} of operation {at}, which was just applied, was \
                             refused as a transaction was taken back: {refusal}"
                        );
                    }
                }
                project.next_id = next_id;
                return Err(TransactionError::Refused { index, error });
            }
        }
    }
    Ok((done, inverse))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{Clip, Timeline, Track, TrackKind};
    use crate::time::{Rate, Time};

    // The sample: timeline t1 at 25/1, with c1 on V1 and c2 on A1, each on frames [0, 100) and
    // cut from m1, which lasts 250 frames; next_id is 3.

    /// A clip with no id, cut from m1 from its start, at `start` for `length` frames.
    fn clip(start: i64, length: i64) -> Clip {
        Clip {
            id: String::new(),
            media: "m1".into(),
            source_in: Time {
                ticks: 0,
                scale: 15360,
            },
            start,
            length,
        }
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
                id: String::new(),
                name: "Second".into(),
                rate: Rate { num: 25, den: 1 },
                tracks: vec![Track {
                    id: String::new(),
                    kind: TrackKind::Video,
                    clips: vec![clip(50, 10), clip(0, 10)],
                }],
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

        // A client that applies what was done to its copy gets the same project.
        let mut copy = Project::sample();
        copy.next_id = history.project().next_id;
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
}
