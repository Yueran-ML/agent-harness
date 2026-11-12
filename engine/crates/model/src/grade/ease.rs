//! How a key is come to and left: straight, or eased (Reference Manual 20, pp. 1105 and 1117).
//!
//! The inspector and the Curve Editor of the Edit page give a key one of four ways: Linear, Ease
//! In, Ease Out, and Ease In and Out. They are no kinds of key of their own. A key says how its
//! value goes to the next one ([`Interp`]), and an ease is a curve whose handle lies flat at the
//! key: the value stands still there for a moment and then gathers speed, or loses it as it
//! comes in. So an ease is written as the handles of a Bezier, and a curve that somebody then
//! pulls by its handles is the same kind of thing as one that was eased by a menu.
//!
//! A key lies between two stretches. Ease In is of the stretch that comes to it, and is written
//! on the key before, in the handle that reaches back from this one; Ease Out is of the stretch
//! that leaves it, and is written on the key itself. The side that is not eased goes straight:
//! its handle points at the other key. Where both handles of a stretch point straight at each
//! other the stretch is written as linear, which it is, so that its values are those of a line
//! to the last bit.
//!
//! A handle reaches over a third of the frames between its two keys ([`EASE_SHARE`]), where the
//! handles of a straight line lie. A stretch that holds its value (a static key of the Color
//! page) is left as it is: there is nothing between its keys to ease.

#![allow(
    clippy::float_cmp,
    reason = "a handle is that of an ease or of a line when its numbers are those, as written"
)]

use super::{Interp, Key};

/// The share of the frames between two keys that the handle of an ease reaches over, and that
/// of a straight stretch written as a curve.
pub const EASE_SHARE: f64 = 1.0 / 3.0;

/// How a key is come to and left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ease {
    /// Straight in and straight out.
    Linear,
    /// The value slows as it comes to the key, and leaves it straight.
    In,
    /// The value comes to the key straight, and gathers speed as it leaves.
    Out,
    /// It slows as it comes and gathers speed as it leaves.
    InOut,
}

impl Ease {
    /// Every way, in the order of the menu.
    pub const ALL: [Ease; 4] = [Ease::Linear, Ease::In, Ease::Out, Ease::InOut];

    fn eases_in(self) -> bool {
        matches!(self, Ease::In | Ease::InOut)
    }

    fn eases_out(self) -> bool {
        matches!(self, Ease::Out | Ease::InOut)
    }
}

/// The two handles of the stretch from a key to the next, as they are or as a line has them:
/// the one that leaves `from`, and the one that reaches back from `to`.
fn handles(from: &Key, to: &Key) -> Option<([f64; 2], [f64; 2])> {
    let rise = to.value - from.value;
    match from.interp {
        Interp::Hold => None,
        Interp::Linear => Some(straight(rise)),
        Interp::Bezier { out, into } => Some((out, into)),
    }
}

/// The handles of a straight stretch that rises by `rise`.
fn straight(rise: f64) -> ([f64; 2], [f64; 2]) {
    (
        [EASE_SHARE, rise * EASE_SHARE],
        [EASE_SHARE, -rise * EASE_SHARE],
    )
}

/// The handle of an ease: flat at its key.
const FLAT: [f64; 2] = [EASE_SHARE, 0.0];

/// A stretch of these handles: linear where they are those of a line.
fn stretch(out: [f64; 2], into: [f64; 2], rise: f64) -> Interp {
    if (out, into) == straight(rise) {
        Interp::Linear
    } else {
        Interp::Bezier { out, into }
    }
}

/// Have the key at `index` come to and left in a way. The stretches on its two sides are
/// changed, each in its handle at this key; a stretch that holds is left alone, and so is
/// everything where there is no such key.
pub fn ease(keys: &mut [Key], index: usize, ease: Ease) {
    if index >= keys.len() {
        return;
    }
    if index > 0 {
        let (before, here) = (&keys[index - 1], &keys[index]);
        let rise = here.value - before.value;
        if let Some((out, _)) = handles(before, here) {
            let into = if ease.eases_in() {
                FLAT
            } else {
                straight(rise).1
            };
            keys[index - 1].interp = stretch(out, into, rise);
        }
    }
    if index + 1 < keys.len() {
        let (here, after) = (&keys[index], &keys[index + 1]);
        let rise = after.value - here.value;
        if let Some((_, into)) = handles(here, after) {
            let out = if ease.eases_out() {
                FLAT
            } else {
                straight(rise).0
            };
            keys[index].interp = stretch(out, into, rise);
        }
    }
}

/// The way that the key at `index` is come to and left, if it is one of the four: none where a
/// handle at the key was pulled to somewhere else, or where there is no such key.
///
/// A side that has no stretch, or one that holds, agrees with every way: the first key of a
/// curve that leaves it eased is eased out, and a key alone is linear.
#[must_use]
pub fn ease_of(keys: &[Key], index: usize) -> Option<Ease> {
    let here = keys.get(index)?;
    // Whether a side is eased, straight, or of no say.
    let side = |handle: Option<[f64; 2]>, line: [f64; 2]| match handle {
        None => Some(None),
        Some(handle) if handle == FLAT && line != FLAT => Some(Some(true)),
        Some(handle) if handle == line => Some(Some(false)),
        Some(_) => None,
    };
    let coming = index.checked_sub(1).and_then(|before| {
        let before = &keys[before];
        handles(before, here).map(|(_, into)| (into, straight(here.value - before.value).1))
    });
    let leaving = keys.get(index + 1).and_then(|after| {
        handles(here, after).map(|(out, _)| (out, straight(after.value - here.value).0))
    });
    let coming = side(
        coming.map(|(handle, _)| handle),
        coming.map_or(FLAT, |(_, line)| line),
    )?;
    let leaving = side(
        leaving.map(|(handle, _)| handle),
        leaving.map_or(FLAT, |(_, line)| line),
    )?;
    Some(match (coming.unwrap_or(false), leaving.unwrap_or(false)) {
        (false, false) => Ease::Linear,
        (true, false) => Ease::In,
        (false, true) => Ease::Out,
        (true, true) => Ease::InOut,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grade::Param;

    fn key(frame: i64, value: f64) -> Key {
        Key {
            frame,
            value,
            interp: Interp::Linear,
        }
    }

    fn three() -> Vec<Key> {
        vec![key(0, 0.0), key(30, 12.0), key(90, -6.0)]
    }

    #[test]
    fn an_ease_is_a_flat_handle_at_the_key_and_a_straight_one_at_the_other() {
        let mut keys = three();
        ease(&mut keys, 1, Ease::InOut);
        // The stretch that comes: straight out of the first key, flat into the second.
        assert_eq!(
            keys[0].interp,
            Interp::Bezier {
                out: [EASE_SHARE, 12.0 * EASE_SHARE],
                into: FLAT
            }
        );
        // The stretch that leaves: flat out of the second key, straight into the third.
        assert_eq!(
            keys[1].interp,
            Interp::Bezier {
                out: FLAT,
                into: [EASE_SHARE, 18.0 * EASE_SHARE]
            }
        );
        assert_eq!(keys[2].interp, Interp::Linear);
    }

    #[test]
    fn each_way_is_read_back_as_it_was_set_and_linear_is_written_as_linear() {
        for at in 0..3 {
            for way in Ease::ALL {
                let mut keys = three();
                ease(&mut keys, at, way);
                // The first key has nothing coming to it, and the last nothing leaving.
                let read = match (at, way) {
                    (0, Ease::In) | (2, Ease::Out) => Ease::Linear,
                    (0, Ease::InOut) => Ease::Out,
                    (2, Ease::InOut) => Ease::In,
                    _ => way,
                };
                assert_eq!(ease_of(&keys, at), Some(read), "{way:?} at {at}");
                ease(&mut keys, at, Ease::Linear);
                assert_eq!(keys, three(), "{way:?} at {at}, and linear again");
            }
        }
    }

    #[test]
    fn the_ways_of_two_keys_share_the_stretch_between_them() {
        let mut keys = three();
        ease(&mut keys, 0, Ease::Out);
        ease(&mut keys, 1, Ease::In);
        assert_eq!(
            keys[0].interp,
            Interp::Bezier {
                out: FLAT,
                into: FLAT
            }
        );
        assert_eq!(ease_of(&keys, 0), Some(Ease::Out));
        assert_eq!(ease_of(&keys, 1), Some(Ease::In));
        // Taking the ease of one away leaves that of the other.
        ease(&mut keys, 1, Ease::Linear);
        assert_eq!(ease_of(&keys, 0), Some(Ease::Out));
        assert_eq!(ease_of(&keys, 1), Some(Ease::Linear));
    }

    #[test]
    fn an_eased_value_stands_still_at_its_key_and_is_never_beyond_its_two_keys() {
        let mut keys = three();
        ease(&mut keys, 1, Ease::InOut);
        let param = Param::Keys(keys);
        // Near the key the value has moved far less than a line would have.
        assert!((param.value_at(29.0) - 12.0).abs() < 0.05);
        assert!((param.value_at(31.0) - 12.0).abs() < 0.05);
        assert!((12.0 - param.value_at(29.0)).abs() < 12.0 / 30.0 / 4.0);
        let mut last = 0.0;
        for step in 0..=300 {
            let value = param.value_at(f64::from(step) / 10.0);
            assert!((0.0..=12.0).contains(&value) && value >= last, "{step}");
            last = value;
        }
        assert_eq!(param.value_at(0.0), 0.0);
        assert_eq!(param.value_at(30.0), 12.0);
        assert_eq!(param.value_at(90.0), -6.0);
    }

    #[test]
    fn a_stretch_that_holds_and_a_handle_that_was_pulled_are_left_as_they_are() {
        let mut keys = three();
        keys[0].interp = Interp::Hold;
        ease(&mut keys, 1, Ease::In);
        assert_eq!(keys[0].interp, Interp::Hold);
        // Its side has no say: the key is linear, or eased out, by its other side alone.
        assert_eq!(ease_of(&keys, 1), Some(Ease::Linear));
        ease(&mut keys, 1, Ease::InOut);
        assert_eq!(ease_of(&keys, 1), Some(Ease::Out));

        let pulled = Interp::Bezier {
            out: [0.5, 2.0],
            into: [0.25, -1.0],
        };
        let mut keys = three();
        keys[1].interp = pulled;
        assert_eq!(ease_of(&keys, 1), None);
        assert_eq!(ease_of(&keys, 2), None);
        // Easing the third key in moves the handle at it, and keeps the one that was pulled.
        ease(&mut keys, 2, Ease::In);
        assert_eq!(
            keys[1].interp,
            Interp::Bezier {
                out: [0.5, 2.0],
                into: FLAT
            }
        );
        assert_eq!(ease_of(&keys, 2), Some(Ease::In));
        assert_eq!(ease_of(&keys, 1), None);
    }

    #[test]
    fn where_there_is_no_such_key_nothing_is_done() {
        let mut keys = three();
        ease(&mut keys, 3, Ease::InOut);
        assert_eq!(keys, three());
        assert_eq!(ease_of(&keys, 3), None);
        let mut alone = vec![key(5, 1.0)];
        ease(&mut alone, 0, Ease::InOut);
        assert_eq!(alone, vec![key(5, 1.0)]);
        assert_eq!(ease_of(&alone, 0), Some(Ease::Linear));
        // Two keys of one value: a flat handle is the handle of the line between them.
        let mut level = vec![key(0, 4.0), key(10, 4.0)];
        ease(&mut level, 0, Ease::Out);
        assert_eq!(level[0].interp, Interp::Linear);
        assert_eq!(ease_of(&level, 0), Some(Ease::Linear));
    }
}
