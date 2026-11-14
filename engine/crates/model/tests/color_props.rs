//! Random operations of colour management, mixed with the other operations: each one that is
//! applied is taken back exactly by its inverse and done again by what it gave back as done,
//! each one that is refused changes nothing, and after each one the project is one that
//! `Project::validate` leaves as it is, and is read back as it is written.

mod common;

use std::collections::BTreeMap;

use columbina_model::{apply, apply_done, OpError, Project};
use common::colors::color_op_of;
use common::{check, messy, op_of, spec, tidy, Spec};
use proptest::prelude::*;

/// The makings of an operation: one of colour management, or now and then any other.
#[derive(Clone, Debug)]
enum Making {
    Color([usize; 4]),
    Other(Spec),
}

fn making() -> impl Strategy<Value = Making> {
    prop_oneof![
        6 => any::<[usize; 4]>().prop_map(Making::Color),
        1 => spec().prop_map(Making::Other),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn every_operation_of_colour_is_taken_back_or_changes_nothing(
        start_messy in prop::bool::weighted(0.2),
        makings in prop::collection::vec(making(), 1..30)
    ) {
        let mut project = if start_messy { messy() } else { tidy() };
        for making in &makings {
            let op = match making {
                Making::Color(n) => color_op_of(&project, *n),
                Making::Other(spec) => op_of(&project, spec),
            };
            let before = project.clone();
            let Ok((done, inverse)) = apply_done(&mut project, op.clone()) else {
                prop_assert_eq!(&project, &before, "{:?}", op);
                continue;
            };
            check(&project);
            let after = project.clone();
            let mut undone = after.clone();
            let again = apply(&mut undone, inverse.clone())
                .unwrap_or_else(|e| panic!("{inverse:?} of {op:?}: {e}"));
            let mut expected = before.clone();
            expected.next_id = after.next_id;
            prop_assert_eq!(&undone, &expected, "{:?} then {:?}", op, inverse);
            prop_assert_eq!(&done, &again, "{:?}", op);
            apply(&mut undone, again.clone())
                .unwrap_or_else(|e| panic!("{again:?} of {op:?}: {e}"));
            prop_assert_eq!(&undone, &after, "{:?} redone as {:?}", op, again);
            let read = Project::from_value(serde_json::to_value(&project).unwrap()).unwrap();
            prop_assert_eq!(&read, &project);
        }
    }
}

/// Numbers of a fixed sequence, for a test that is the same at every run.
struct Numbers(u64);

impl Numbers {
    fn next(&mut self) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        usize::try_from(self.0 >> 33).unwrap()
    }

    fn four(&mut self) -> [usize; 4] {
        [self.next(), self.next(), self.next(), self.next()]
    }
}

#[test]
fn the_random_operations_of_colour_are_applied_and_refused_for_each_reason() {
    let mut numbers = Numbers(11);
    let mut applied: BTreeMap<String, usize> = BTreeMap::new();
    let mut refused: BTreeMap<String, usize> = BTreeMap::new();
    for _ in 0..40 {
        let mut project = tidy();
        for _ in 0..60 {
            let op = color_op_of(&project, numbers.four());
            let name = serde_json::to_value(&op).unwrap()["op"]
                .as_str()
                .unwrap()
                .to_owned();
            match apply(&mut project, op) {
                Ok(_) => *applied.entry(name).or_default() += 1,
                Err(error) => {
                    let reason = match error {
                        OpError::OutOfRange { what } => format!("out_of_range {what}"),
                        error => serde_json::to_value(&error).unwrap()["reason"]
                            .as_str()
                            .unwrap()
                            .to_owned(),
                    };
                    *refused.entry(reason).or_default() += 1;
                    *refused.entry(name).or_default() += 1;
                }
            }
        }
    }
    for name in [
        "set_color_settings",
        "set_media_color",
        "unknown_space",
        "inapplicable",
        "color_settings",
        "missing",
        "out_of_range custom_luminance",
        "out_of_range input_lut",
        "out_of_range limit_gamut",
    ] {
        let (ok, no) = (
            applied.get(name).copied().unwrap_or(0),
            refused.get(name).copied().unwrap_or(0),
        );
        assert!(
            no >= 1 && (ok >= 50 || !name.starts_with("set_")),
            "{name}: {ok} applied, {no} refused; {refused:?}"
        );
    }
}
