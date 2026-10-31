//! Random gestures, transactions, undos and redos, checked against a simple model of a history
//! with gestures: a line of projects, the place in it that the project is at, and the gesture
//! that is going on, whose transactions change the project at that place rather than add one. A
//! gesture ends as it says, or as anything else is done; one that ends with the project as it
//! was before it takes its place in the line away. Every transaction that is applied makes a
//! revision, a client that applies what each did keeps the engine's project, and an undo takes a
//! whole gesture back to the project before it.

mod common;

use columbina_model::{apply, Change, History, Op, Project, TransactionError};
use common::{check, messy, op_of, spec, tidy, Spec};
use proptest::prelude::*;

/// What a client asks of the engine.
#[derive(Clone, Debug)]
enum Action {
    /// A transaction of no gesture.
    Transaction(Vec<(Spec, bool)>),
    /// A transaction of the gesture of one of two keys, ending it or not.
    Gesture(u8, Vec<(Spec, bool)>, bool),
    /// The gesture of a key ends.
    End(u8),
    Undo,
    Redo,
}

fn makings() -> impl Strategy<Value = Vec<(Spec, bool)>> {
    prop::collection::vec((spec(), prop::bool::weighted(0.1)), 0..=3)
}

fn action() -> impl Strategy<Value = Action> {
    prop_oneof![
        2 => makings().prop_map(Action::Transaction),
        8 => (0u8..2, makings(), prop::bool::weighted(0.15))
            .prop_map(|(key, makings, end)| Action::Gesture(key, makings, end)),
        1 => (0u8..2).prop_map(Action::End),
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

/// `project` with the `next_id` of `other`.
fn with_next_id_of(project: &Project, other: &Project) -> Project {
    Project {
        next_id: other.next_id,
        ..project.clone()
    }
}

/// The model of a history with gestures.
struct Model {
    /// The projects of the steps, the start first.
    states: Vec<Project>,
    at: usize,
    undoable: usize,
    redoable: usize,
    most: usize,
    /// The key of the gesture going on.
    open: Option<String>,
}

impl Model {
    /// The gesture going on ends: its place in the line is taken away where it changed nothing.
    fn close(&mut self) {
        if self.open.take().is_some()
            && with_next_id_of(&self.states[self.at - 1], &self.states[self.at])
                == self.states[self.at]
        {
            self.states.truncate(self.at);
            self.at -= 1;
            self.undoable -= 1;
        }
    }

    /// A transaction made `project`, as part of the gesture `key` or of none.
    fn applied(&mut self, project: Project, key: Option<&str>) {
        if key.is_some() && self.open.as_deref() == key {
            self.states[self.at] = project;
            return;
        }
        self.close();
        self.states.truncate(self.at + 1);
        self.states.push(project);
        self.at += 1;
        self.undoable = (self.undoable + 1).min(self.most);
        self.redoable = 0;
        self.open = key.map(str::to_owned);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn a_gesture_is_one_step_however_many_transactions_it_has(
        start_messy in prop::bool::weighted(0.2),
        most in 1usize..6,
        actions in prop::collection::vec(action(), 1..50)
    ) {
        let start = if start_messy { messy() } else { tidy() };
        let mut history = History::with_most(start.clone(), most);
        let mut model = Model {
            states: vec![start.clone()],
            at: 0,
            undoable: 0,
            redoable: 0,
            most,
            open: None,
        };
        let mut client = start;

        for action in &actions {
            let before = history.project().clone();
            let revision = history.revision();
            let result = match action {
                Action::Transaction(makings) | Action::Gesture(_, makings, _) => {
                    let ops = transaction_of(&before, makings);
                    let (key, end) = match action {
                        Action::Gesture(key, _, end) => (Some(format!("1:g{key}")), *end),
                        _ => (None, false),
                    };
                    let result = match &key {
                        Some(key) => history
                            .change_in_gesture(
                                ops.iter().cloned().map(Change::Op).collect(),
                                "desktop",
                                &mut |change, _: &[Op]| change,
                                key,
                                end,
                            )
                            .map(|changed| changed.applied),
                        None => history.apply(ops.clone(), "agent"),
                    };
                    match &result {
                        Ok(_) => {
                            let mut expected = before.clone();
                            for op in &ops {
                                apply(&mut expected, op.clone()).unwrap();
                            }
                            prop_assert_eq!(history.project(), &expected);
                            model.applied(expected, key.as_deref());
                        }
                        // Refused, or empty: an operation that changes nothing, as setting
                        // media to what it is, is none of a step.
                        Err(TransactionError::Empty | TransactionError::Refused { .. }) => {}
                        Err(error) => {
                            return Err(TestCaseError::fail(format!("{error}")));
                        }
                    }
                    if end && model.open == key {
                        model.close();
                    }
                    Some(result)
                }
                Action::End(key) => {
                    if model.open.as_deref() == Some(&format!("1:g{key}")) {
                        history.end_gesture();
                    }
                    if model.open.as_deref() == Some(&format!("1:g{key}")) {
                        model.close();
                    }
                    None
                }
                Action::Undo => {
                    model.close();
                    let result = history.undo("agent");
                    prop_assert_eq!(result.is_some(), model.undoable > 0);
                    if let Some(Ok(_)) = &result {
                        model.at -= 1;
                        model.undoable -= 1;
                        model.redoable += 1;
                        let expected = with_next_id_of(&model.states[model.at], &before);
                        prop_assert_eq!(history.project(), &expected);
                    }
                    result
                }
                Action::Redo => {
                    model.close();
                    let result = history.redo("agent");
                    prop_assert_eq!(result.is_some(), model.redoable > 0);
                    if let Some(Ok(_)) = &result {
                        model.at += 1;
                        model.undoable += 1;
                        model.redoable -= 1;
                        let expected = with_next_id_of(&model.states[model.at], &before);
                        prop_assert_eq!(history.project(), &expected);
                    }
                    result
                }
            };
            if let Some(Ok(applied)) = result {
                prop_assert_eq!(applied.revision, revision + 1);
                prop_assert_eq!(history.revision(), revision + 1);
                client.next_id = applied.next_id;
                for op in applied.done {
                    apply(&mut client, op.clone())
                        .unwrap_or_else(|e| panic!("the client refused {op:?}: {e}"));
                }
                prop_assert_eq!(&client, history.project());
            } else {
                // Refused, or nothing to do: nothing changes, the revision included.
                prop_assert_eq!(history.project(), &before);
                prop_assert_eq!(history.revision(), revision);
            }
            prop_assert_eq!(history.gesture().map(|(key, _)| key.to_owned()), model.open.clone());
            prop_assert_eq!(history.can_undo(), model.undoable > 0);
            prop_assert_eq!(history.can_redo(), model.redoable > 0);
            check(history.project());
        }
        // However it went, every step undone walks back the line to its start.
        history.end_gesture();
        model.close();
        while model.undoable > 0 {
            let before = history.project().clone();
            history.undo("agent").unwrap().unwrap();
            model.at -= 1;
            model.undoable -= 1;
            prop_assert_eq!(history.project(), &with_next_id_of(&model.states[model.at], &before));
        }
        prop_assert!(!history.can_undo());
    }
}
