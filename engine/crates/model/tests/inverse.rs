//! Random operations on a project: each one that is applied is taken back by its inverse and
//! done again by the inverse of that, and each one that is refused changes nothing. After every
//! operation the project is still one that `Project::validate` leaves as it is.

mod common;

use columbina_model::{apply, apply_done, Project};
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

#[test]
fn the_random_operations_on_sound_are_applied_as_well_as_refused() {
    use std::collections::BTreeMap;
    let mut project = tidy();
    let mut applied: BTreeMap<String, usize> = BTreeMap::new();
    let mut refused = 0;
    let mut n: usize = 0x9e37_79b9;
    for _ in 0..4000 {
        let mut next = || {
            n = n.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            n >> 7
        };
        let op = common::sounds::sound_op_of(&project, [next(), next(), next(), next()]);
        let name = serde_json::to_value(&op).unwrap()["op"]
            .as_str()
            .unwrap()
            .to_owned();
        let before = project.clone();
        if let Ok((done, inverse)) = apply_done(&mut project, op.clone()) {
            *applied.entry(name).or_default() += 1;
            // Taken back, and done again by the inverse of the inverse, which is `done`.
            let after = project.clone();
            let mut undone = after.clone();
            let again = apply(&mut undone, inverse).unwrap();
            assert_eq!(
                undone,
                Project {
                    next_id: after.next_id,
                    ..before
                },
                "{op:?}"
            );
            assert_eq!(again, done, "{op:?}");
            apply(&mut undone, done).unwrap();
            assert_eq!(undone, after, "{op:?}");
        } else {
            assert_eq!(project, before);
            refused += 1;
        }
        check(&project);
    }
    assert_eq!(applied.len(), 17, "{applied:?}");
    assert!(applied.values().all(|&n| n >= 5), "{applied:?}");
    assert!(refused >= 400, "{refused} refused");
}
