//! Random operations on a project: each one that is applied is taken back by its inverse and
//! done again by the inverse of that, and each one that is refused changes nothing. After every
//! operation the project is still one that `Project::validate` leaves as it is.

mod common;

use columbina_model::{apply, apply_done};
use common::{check, messy, op_of, spec, tidy};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn every_operation_is_taken_back_or_changes_nothing(
        start_messy in any::<bool>(),
        specs in prop::collection::vec(spec(), 1..30)
    ) {
        let mut project = if start_messy { messy() } else { tidy() };
        check(&project);
        for spec in &specs {
            let op = op_of(&project, spec);
            let before = project.clone();
            let Ok((done, inverse)) = apply_done(&mut project, op.clone()) else {
                // A refused operation changes nothing, not even next_id.
                prop_assert_eq!(&project, &before, "{:?}", op);
                continue;
            };
            check(&project);
            prop_assert!(project.next_id >= before.next_id);
            let after = project.clone();

            let mut undone = after.clone();
            let again = apply(&mut undone, inverse.clone())
                .unwrap_or_else(|e| panic!("{inverse:?} of {op:?}: {e}"));
            prop_assert_eq!(undone.next_id, after.next_id);
            let mut expected = before.clone();
            expected.next_id = after.next_id;
            prop_assert_eq!(&undone, &expected, "{:?} then {:?}", op, inverse);

            // The operation as it was done is the inverse of the inverse.
            prop_assert_eq!(&done, &again, "{:?}", op);

            // Doing it again with the inverse of the inverse makes the same things, with the
            // same ids.
            apply(&mut undone, again.clone())
                .unwrap_or_else(|e| panic!("{again:?} of {op:?}: {e}"));
            prop_assert_eq!(&undone, &after, "{:?} redone as {:?}", op, again);
        }
    }
}
