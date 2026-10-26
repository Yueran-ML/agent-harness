//! Random transactions, undos and redos of the agent and the desktop, checked against a simple
//! model of a history: a list of the projects that the transactions made, and the place in it
//! that the project is at. A transaction that is applied leaves the project that its operations
//! make one by one, and one that is refused changes nothing. An undo goes back to the project
//! before, but for `next_id`, and a redo forward to the project after, with the same ids. Each
//! of them makes one revision, and a client that applies what each did to its copy of the
//! project keeps the same project as the engine.

mod common;

use columbina_model::{apply, History, Op, OpError, Project, TransactionError};
use common::{check, messy, op_of, spec, tidy, Spec};
use proptest::prelude::*;

/// What a client asks of the engine.
#[derive(Clone, Debug)]
enum Action {
    /// A transaction of the operations that these makings give, each made against the project
    /// as the operations before it leave it. One that would be refused is left out unless its
    /// flag is set, so that some transactions are applied and some are refused.
    Transaction(Vec<(Spec, bool)>),
    Undo,
    Redo,
}

fn action() -> impl Strategy<Value = Action> {
    prop_oneof![
        6 => prop::collection::vec((spec(), prop::bool::weighted(0.1)), 1..=5)
            .prop_map(Action::Transaction),
        2 => Just(Action::Undo),
        2 => Just(Action::Redo),
    ]
}

/// The operations of a transaction made of `makings` against `project`.
fn transaction_of(project: &Project, makings: &[(Spec, bool)]) -> Vec<Op> {
    let mut scratch = project.clone();
    let mut ops = vec![];
    for (spec, keep_refused) in makings {
        let op = op_of(&scratch, spec);
        if apply(&mut scratch, op.clone()).is_ok() || *keep_refused {
            ops.push(op);
        }
    }
    ops
}

/// The project that `ops` make of `project` one by one, or the first that is refused and why.
fn reference(project: &Project, ops: &[Op]) -> Result<Project, (usize, OpError)> {
    let mut project = project.clone();
    for (index, op) in ops.iter().enumerate() {
        apply(&mut project, op.clone()).map_err(|error| (index, error))?;
    }
    Ok(project)
}

/// `project` with the `next_id` of `other`.
fn with_next_id_of(project: &Project, other: &Project) -> Project {
    Project {
        next_id: other.next_id,
        ..project.clone()
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn a_history_is_a_line_of_projects_that_undo_and_redo_walk(
        start_messy in any::<bool>(),
        most in 1usize..6,
        actions in prop::collection::vec((action(), any::<bool>()), 1..40)
    ) {
        let start = if start_messy { messy() } else { tidy() };
        let mut history = History::with_most(start.clone(), most);
        // The model: the projects that the transactions made and who made each, the place of
        // the project among them, and how many steps can be undone and redone. `makers[k]`
        // made `states[k]` of `states[k - 1]`; the start was made by no one.
        let mut states = vec![start.clone()];
        let mut makers = vec![String::new()];
        let mut at = 0;
        let mut undoable = 0;
        let mut redoable = 0;
        // A client's copy, kept only from what the engine says it did.
        let mut client = start;

        for (action, by_agent) in &actions {
            let by = if *by_agent { "agent" } else { "desktop" };
            let before = history.project().clone();
            let revision = history.revision();
            let result = match action {
                Action::Transaction(makings) => {
                    let ops = transaction_of(&before, makings);
                    let result = history.apply(ops.clone(), by);
                    match (reference(&before, &ops), &result) {
                        _ if ops.is_empty() => {
                            prop_assert_eq!(&result, &Err(TransactionError::Empty));
                        }
                        (Err((index, error)), _) => {
                            let expected = Err(TransactionError::Refused { index, error });
                            prop_assert_eq!(&result, &expected);
                        }
                        (Ok(expected), Ok(applied)) => {
                            prop_assert_eq!(history.project(), &expected);
                            prop_assert_eq!(applied.done.len(), ops.len());
                            prop_assert_eq!(&applied.made_by, by);
                            states.truncate(at + 1);
                            states.push(expected);
                            makers.truncate(at + 1);
                            makers.push(by.into());
                            at += 1;
                            undoable = (undoable + 1).min(most);
                            redoable = 0;
                        }
                        (Ok(_), Err(error)) => {
                            return Err(TestCaseError::fail(format!("refused: {error}")));
                        }
                    }
                    Some(result)
                }
                Action::Undo => {
                    let result = history.undo(by);
                    prop_assert_eq!(result.is_some(), undoable > 0);
                    if let Some(Ok(applied)) = &result {
                        // It takes back the latest step, whoever made it.
                        prop_assert_eq!(&applied.made_by, &makers[at]);
                        at -= 1;
                        undoable -= 1;
                        redoable += 1;
                        // Back to the project before, but for next_id, which does not go back.
                        let expected = with_next_id_of(&states[at], &before);
                        prop_assert_eq!(history.project(), &expected);
                    }
                    result
                }
                Action::Redo => {
                    let result = history.redo(by);
                    prop_assert_eq!(result.is_some(), redoable > 0);
                    if let Some(Ok(applied)) = &result {
                        at += 1;
                        prop_assert_eq!(&applied.made_by, &makers[at]);
                        undoable += 1;
                        redoable -= 1;
                        // Forward to the project after, with the same ids.
                        let expected = with_next_id_of(&states[at], &before);
                        prop_assert_eq!(history.project(), &expected);
                    }
                    result
                }
            };

            match result {
                Some(Ok(applied)) => {
                    prop_assert_eq!(applied.revision, revision + 1);
                    prop_assert_eq!(history.revision(), revision + 1);
                    prop_assert_eq!(&applied.by, by);
                    // next_id never goes back, and only a transaction raises it.
                    prop_assert!(history.project().next_id >= before.next_id);
                    if !matches!(action, Action::Transaction(_)) {
                        prop_assert_eq!(history.project().next_id, before.next_id);
                    }
                    // A client that takes next_id from what the engine sent and applies what
                    // was done keeps up.
                    prop_assert_eq!(applied.next_id, history.project().next_id);
                    client.next_id = applied.next_id;
                    for op in applied.done {
                        apply(&mut client, op.clone())
                            .unwrap_or_else(|e| panic!("the client refused {op:?}: {e}"));
                    }
                    prop_assert_eq!(&client, history.project());
                }
                Some(Err(error)) => {
                    // Refused: nothing changes, next_id and the revision included.
                    prop_assert!(
                        matches!(action, Action::Transaction(_)),
                        "an undo or a redo was refused: {}", error
                    );
                    prop_assert_eq!(history.project(), &before);
                    prop_assert_eq!(history.revision(), revision);
                }
                None => {
                    prop_assert_eq!(history.project(), &before);
                    prop_assert_eq!(history.revision(), revision);
                }
            }
            prop_assert_eq!(history.can_undo(), undoable > 0);
            prop_assert_eq!(history.can_redo(), redoable > 0);
            check(history.project());
        }
    }
}
