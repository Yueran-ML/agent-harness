//! Whole-animation Polygon topology: inserting and removing points.
//!
//! The point count is the same on every frame; adding or removing a point applies to the whole
//! animation and keeps every other point's keys. An inserted point splits its edge at one fixed
//! share for the whole animation. Its coordinates are generated on the union of the existing key
//! times of the two edge ends, so the shape is unchanged at every instant. Where this module
//! cannot prove that, the insertion is refused as a whole and says why: it never bakes a static
//! value, drops keys, approximates, or adds dense keys. These are project rules; Resolve instead
//! makes an animated Polygon's points pop on and off at keyframes (RM20 p. 3194).

use serde::Serialize;

use super::{bounds, WindowPoint, WindowShape, MOST_POLYGON_POINTS};
use crate::grade::{Bounds, Interp, Key, Param};

/// Floating-point slack when comparing generated values, in local image-height units.
const TOLERANCE: f64 = 1e-9;

/// Why an exact generated parameter could not be represented.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Why {
    /// A held jump coincides with a continuous change in the same interval.
    Jump,
    /// Non-linear easings with different time parameterizations cannot be summed exactly.
    Timing,
    /// An original curve leaves the coordinate range and is clamped when drawn.
    Overshoot,
    /// The generated coordinate leaves the coordinate range.
    Range,
    /// The exact split needs a time handle outside the 0–1 that keys can store.
    Encoding,
}

/// The terms (by position in the input) and key interval that prevent an exact sum.
#[derive(Clone, Debug, PartialEq)]
pub struct Conflict {
    pub why: Why,
    pub terms: Vec<usize>,
    /// The key interval, in the grade's own clock; equal ends name a single key.
    pub frames: Option<[i64; 2]>,
}

/// Why a Polygon topology edit was refused; the shape is then unchanged.
#[derive(Clone, Debug, PartialEq)]
pub enum TopologyError {
    /// Not a Polygon, or an index out of range.
    Unsupported,
    /// A Polygon already has [`MOST_POLYGON_POINTS`] points.
    Maximum,
    /// Removing would leave fewer than three points.
    Minimum,
    /// The inserted point's `axis` coordinate cannot be generated exactly.
    Shape {
        axis: &'static str,
        /// Polygon point indices whose coordinates cause the conflict.
        points: Vec<usize>,
        why: Why,
        frames: Option<[i64; 2]>,
    },
}

/// The weighted sum `Σ weight·param` of animated scalars as a single exact Param, or the
/// reason the current representation cannot store it.
///
/// # Errors
///
/// A [`Conflict`] names the contributing terms and the key interval.
pub fn blend(terms: &[(f64, &Param)], bounds: Bounds) -> Result<Param, Conflict> {
    let terms: Vec<_> = terms
        .iter()
        .enumerate()
        .filter(|(_, (weight, _))| *weight != 0.0)
        .map(|(i, &(weight, param))| (i, weight, param))
        .collect();
    let out = |value: f64| value < bounds.low - TOLERANCE || value > bounds.high + TOLERANCE;
    // Drawing clamps an evaluated coordinate; a clamped original cannot be summed linearly.
    for &(i, _, param) in &terms {
        if let Param::Keys(keys) = param {
            for pair in keys.windows(2) {
                if let Some((_, y)) = curve(&pair[0], &pair[1]) {
                    let (low, high) = extremes(y);
                    if out(low) || out(high) {
                        return Err(Conflict {
                            why: Why::Overshoot,
                            terms: vec![i],
                            frames: Some([pair[0].frame, pair[1].frame]),
                        });
                    }
                }
            }
        }
    }
    let mut frames: Vec<i64> = terms
        .iter()
        .filter_map(|(_, _, param)| match param {
            Param::Keys(keys) => Some(keys.iter().map(|key| key.frame)),
            Param::Fixed(_) => None,
        })
        .flatten()
        .collect();
    frames.sort_unstable();
    frames.dedup();
    #[allow(clippy::cast_precision_loss, reason = "frames are within 2^53")]
    let at = |frame: i64| -> f64 {
        terms
            .iter()
            .map(|(_, weight, param)| weight * param.value_at(frame as f64))
            .sum()
    };
    let everyone = || terms.iter().map(|(i, _, _)| *i).collect();
    if frames.is_empty() {
        let value = at(0);
        return if out(value) {
            Err(Conflict {
                why: Why::Range,
                terms: everyone(),
                frames: None,
            })
        } else {
            Ok(Param::Fixed(value.clamp(bounds.low, bounds.high)))
        };
    }
    let values: Vec<f64> = frames.iter().map(|&frame| at(frame)).collect();
    if let Some(k) = values.iter().position(|&value| out(value)) {
        return Err(Conflict {
            why: Why::Range,
            terms: everyone(),
            frames: Some([frames[k], frames[k]]),
        });
    }
    let mut keys = Vec::with_capacity(frames.len());
    for k in 0..frames.len() {
        let interp = if k + 1 == frames.len() {
            Interp::Linear
        } else {
            interval(&terms, [frames[k], frames[k + 1]], &out)?
        };
        keys.push(Key {
            frame: frames[k],
            value: values[k].clamp(bounds.low, bounds.high),
            interp,
        });
    }
    Ok(Param::Keys(keys))
}

/// The value control points of a key's time Bezier with its time controls, if it is one.
fn curve(from: &Key, to: &Key) -> Option<([f64; 4], [f64; 4])> {
    match from.interp {
        Interp::Bezier { out, into } => Some((
            [0.0, out[0], 1.0 - into[0], 1.0],
            [
                from.value,
                from.value + out[1],
                to.value + into[1],
                to.value,
            ],
        )),
        _ => None,
    }
}

fn bernstein(p: [f64; 4], q: f64) -> f64 {
    let r = 1.0 - q;
    r * r * r * p[0] + 3.0 * r * r * q * p[1] + 3.0 * r * q * q * p[2] + q * q * q * p[3]
}

/// The exact range of a cubic over its whole parameter, from the roots of its derivative.
fn extremes(y: [f64; 4]) -> (f64, f64) {
    let (a, b, c) = (y[1] - y[0], y[2] - y[1], y[3] - y[2]);
    let (qa, qb, qc) = (a - 2.0 * b + c, 2.0 * (b - a), a);
    let mut candidates = vec![0.0, 1.0];
    if qa.abs() < f64::EPSILON {
        if qb != 0.0 {
            candidates.push(-qc / qb);
        }
    } else {
        let discriminant = qb * qb - 4.0 * qa * qc;
        if discriminant >= 0.0 {
            let root = discriminant.sqrt();
            candidates.extend([(-qb - root) / (2.0 * qa), (-qb + root) / (2.0 * qa)]);
        }
    }
    candidates
        .into_iter()
        .filter(|q| (0.0..=1.0).contains(q))
        .map(|q| bernstein(y, q))
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), v| {
            (low.min(v), high.max(v))
        })
}

/// The parameter at which a monotonic time cubic reaches `share`, as `Param::value_at` finds it.
fn solve(x: [f64; 4], share: f64) -> f64 {
    if share <= 0.0 {
        return 0.0;
    }
    if share >= 1.0 {
        return 1.0;
    }
    let (mut low, mut high) = (0.0_f64, 1.0_f64);
    for _ in 0..64 {
        let middle = f64::midpoint(low, high);
        if bernstein(x, middle) < share {
            low = middle;
        } else {
            high = middle;
        }
    }
    f64::midpoint(low, high)
}

#[allow(
    clippy::many_single_char_names,
    reason = "de Casteljau's intermediate control points"
)]
fn split(p: [f64; 4], q: f64) -> ([f64; 4], [f64; 4]) {
    let lerp = |a: f64, b: f64| a + (b - a) * q;
    let (a, b, c) = (lerp(p[0], p[1]), lerp(p[1], p[2]), lerp(p[2], p[3]));
    let (d, e) = (lerp(a, b), lerp(b, c));
    let f = lerp(d, e);
    ([p[0], a, d, f], [f, e, c, p[3]])
}

/// The part of a time Bezier between two shares of its keys' interval, renormalized to 0–1 in
/// time. The same input always splits identically, so shared parameterizations stay shared.
#[allow(
    clippy::float_cmp,
    reason = "only an exactly whole key interval keeps its stored controls"
)]
fn sub_curve(x: [f64; 4], y: [f64; 4], shares: [f64; 2]) -> ([f64; 4], [f64; 4]) {
    if shares == [0.0, 1.0] {
        return (x, y);
    }
    let (qa, qb) = (solve(x, shares[0]), solve(x, shares[1]));
    let (x, y) = (split(x, qb).0, split(y, qb).0);
    let r = if qb == 0.0 { 0.0 } else { qa / qb };
    let (x, y) = (split(x, r).1, split(y, r).1);
    let span = x[3] - x[0];
    let x = [0.0, (x[1] - x[0]) / span, (x[2] - x[0]) / span, 1.0];
    (x, y)
}

/// One input on one interval of the generated key grid.
#[derive(Clone, Copy)]
enum Piece {
    /// Constant on the half-open interval; `jump` is the change at its right end.
    Flat {
        value: f64,
        held: bool,
        jump: f64,
    },
    Line(f64, f64),
    Curve([f64; 4], [f64; 4]),
}

#[allow(clippy::cast_precision_loss, reason = "frames are within 2^53")]
fn piece(param: &Param, [start, end]: [i64; 2]) -> Piece {
    let flat = |value| Piece::Flat {
        value,
        held: false,
        jump: 0.0,
    };
    let keys = match param {
        Param::Fixed(value) => return flat(*value),
        Param::Keys(keys) => keys,
    };
    let next = keys.partition_point(|key| key.frame <= start);
    if next == 0 {
        return flat(keys[0].value);
    }
    if next == keys.len() {
        return flat(keys[next - 1].value);
    }
    // The grid holds every key, so this key interval contains [start, end].
    let (from, to) = (&keys[next - 1], &keys[next]);
    let span = (to.frame - from.frame) as f64;
    let shares = [
        (start - from.frame) as f64 / span,
        (end - from.frame) as f64 / span,
    ];
    match from.interp {
        Interp::Hold => Piece::Flat {
            value: from.value,
            held: true,
            jump: if end == to.frame {
                to.value - from.value
            } else {
                0.0
            },
        },
        Interp::Linear => {
            let lerp = |share: f64| from.value + (to.value - from.value) * share;
            Piece::Line(lerp(shares[0]), lerp(shares[1]))
        }
        Interp::Bezier { .. } => {
            let (times, values) = curve(from, to).expect("a Bezier key");
            let (times, values) = sub_curve(times, values, shares);
            Piece::Curve(times, values)
        }
    }
}

fn same(a: f64, b: f64) -> bool {
    (a - b).abs() <= TOLERANCE
}

/// Whether value controls are a straight line in time for these time controls.
fn straight(x: [f64; 4], y: [f64; 4]) -> bool {
    let delta = y[3] - y[0];
    same(y[1], y[0] + delta * x[1]) && same(y[2], y[0] + delta * x[2])
}

/// The generated interpolation of one interval of the key grid.
#[allow(
    clippy::too_many_lines,
    reason = "one interval's classification and each of its refusals read together"
)]
fn interval(
    terms: &[(usize, f64, &Param)],
    frames: [i64; 2],
    out: &impl Fn(f64) -> bool,
) -> Result<Interp, Conflict> {
    // Flat and straight parts sum to one straight line; curves are grouped by time controls.
    let mut line = [0.0_f64; 2];
    let mut jump = 0.0_f64;
    let mut jumping = Vec::new();
    let mut changing = Vec::new();
    let (mut held, mut continuous) = (false, false);
    let mut groups: Vec<([f64; 4], [f64; 4], Vec<usize>)> = Vec::new();
    for &(i, weight, param) in terms {
        match piece(param, frames) {
            Piece::Flat {
                value,
                held: was_held,
                jump: step,
            } => {
                held |= was_held;
                line[0] += weight * value;
                line[1] += weight * value;
                if step != 0.0 {
                    jump += weight * step;
                    jumping.push(i);
                }
            }
            Piece::Line(p, q) => {
                continuous = true;
                line[0] += weight * p;
                line[1] += weight * q;
                if !same(p, q) {
                    changing.push(i);
                }
            }
            Piece::Curve(x, y) => {
                continuous = true;
                if straight(x, y) {
                    line[0] += weight * y[0];
                    line[1] += weight * y[3];
                    if !same(y[0], y[3]) {
                        changing.push(i);
                    }
                    continue;
                }
                changing.push(i);
                let y = y.map(|v| weight * v);
                match groups
                    .iter_mut()
                    .find(|(gx, _, _)| same(gx[1], x[1]) && same(gx[2], x[2]))
                {
                    Some((_, sum, members)) => {
                        for (s, v) in sum.iter_mut().zip(y) {
                            *s += v;
                        }
                        members.push(i);
                    }
                    None => groups.push((x, y, vec![i])),
                }
            }
        }
    }
    // A group may cancel to a straight line, leaving room for another easing.
    groups.retain(|(x, y, _)| {
        if straight(*x, *y) {
            line[0] += y[0];
            line[1] += y[3];
            false
        } else {
            true
        }
    });
    let moves = !same(line[0], line[1]) || !groups.is_empty();
    if !same(jump, 0.0) {
        if moves {
            let mut involved: Vec<usize> = jumping.into_iter().chain(changing).collect();
            involved.sort_unstable();
            involved.dedup();
            return Err(Conflict {
                why: Why::Jump,
                terms: involved,
                frames: Some(frames),
            });
        }
        return Ok(Interp::Hold);
    }
    match groups.as_slice() {
        [] => Ok(if held && !continuous {
            Interp::Hold
        } else {
            Interp::Linear
        }),
        [(x, y, members)] => {
            if !(0.0..=1.0).contains(&x[1]) || !(0.0..=1.0).contains(&x[2]) {
                return Err(Conflict {
                    why: Why::Encoding,
                    terms: members.clone(),
                    frames: Some(frames),
                });
            }
            let delta = line[1] - line[0];
            let y = [
                y[0] + line[0],
                y[1] + line[0] + delta * x[1],
                y[2] + line[0] + delta * x[2],
                y[3] + line[1],
            ];
            let (low, high) = extremes(y);
            if out(low) || out(high) {
                let mut involved = members.clone();
                involved.extend(changing);
                involved.sort_unstable();
                involved.dedup();
                return Err(Conflict {
                    why: Why::Range,
                    terms: involved,
                    frames: Some(frames),
                });
            }
            Ok(Interp::Bezier {
                out: [x[1], y[1] - y[0]],
                into: [1.0 - x[2], y[2] - y[3]],
            })
        }
        _ => Err(Conflict {
            why: Why::Timing,
            terms: groups
                .iter()
                .flat_map(|(_, _, members)| members.iter().copied())
                .collect(),
            frames: Some(frames),
        }),
    }
}

impl WindowShape {
    /// Insert a Polygon point on edge `edge` (from point `edge` to the next) at `share` of the
    /// way along it, for the whole animation. Returns the new point's index.
    ///
    /// # Errors
    ///
    /// The shape is left unchanged when the point cannot be inserted exactly.
    pub fn insert_polygon_point(
        &mut self,
        edge: usize,
        share: f64,
    ) -> Result<usize, TopologyError> {
        let Self::Polygon { points, .. } = self else {
            return Err(TopologyError::Unsupported);
        };
        let n = points.len();
        if edge >= n || !(share > 0.0 && share < 1.0) {
            return Err(TopologyError::Unsupported);
        }
        if n >= MOST_POLYGON_POINTS {
            return Err(TopologyError::Maximum);
        }
        let ends = [edge, (edge + 1) % n];
        let (a, b) = (&points[ends[0]], &points[ends[1]]);
        let generate = |axis: &'static str, a: &Param, b: &Param| {
            blend(&[(1.0 - share, a), (share, b)], bounds::COORDINATE).map_err(|conflict| {
                TopologyError::Shape {
                    axis,
                    points: conflict.terms.iter().map(|&t| ends[t]).collect(),
                    why: conflict.why,
                    frames: conflict.frames,
                }
            })
        };
        let point = WindowPoint {
            x: generate("x", &a.x, &b.x)?,
            y: generate("y", &a.y, &b.y)?,
        };
        points.insert(edge + 1, point);
        Ok(edge + 1)
    }

    /// Remove Polygon point `index` from the whole animation, keeping at least three points.
    ///
    /// # Errors
    ///
    /// The shape is left unchanged when the point cannot be removed.
    pub fn remove_polygon_point(&mut self, index: usize) -> Result<(), TopologyError> {
        let Self::Polygon { points, .. } = self else {
            return Err(TopologyError::Unsupported);
        };
        if index >= points.len() {
            return Err(TopologyError::Unsupported);
        }
        if points.len() <= 3 {
            return Err(TopologyError::Minimum);
        }
        points.remove(index);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(frame: i64, value: f64, interp: Interp) -> Key {
        Key {
            frame,
            value,
            interp,
        }
    }
    fn keys(list: &[(i64, f64, Interp)]) -> Param {
        Param::Keys(list.iter().map(|&(f, v, i)| key(f, v, i)).collect())
    }
    fn bez(out: [f64; 2], into: [f64; 2]) -> Interp {
        Interp::Bezier { out, into }
    }
    const L: Interp = Interp::Linear;
    const H: Interp = Interp::Hold;

    /// The generated Param equals the weighted sum at every sampled instant, including
    /// fractional frames, key frames, and frames outside all keys.
    fn assert_exact(terms: &[(f64, &Param)], got: &Param) {
        for step in -40..=2400 {
            let frame = f64::from(step) * 0.25;
            let want: f64 = terms.iter().map(|(w, p)| w * p.value_at(frame)).sum();
            let have = got.value_at(frame);
            assert!(
                (want - have).abs() < 1e-9,
                "frame {frame}: want {want}, got {have} from {got:?}"
            );
        }
        assert!(got.check("test", -16.0, 16.0).is_ok(), "{got:?}");
    }

    fn accepts(a: &Param, b: &Param, s: f64) -> Param {
        let terms = [(1.0 - s, a), (s, b)];
        let got = blend(&terms, bounds::COORDINATE).expect("an exact split");
        assert_exact(&terms, &got);
        got
    }

    #[test]
    fn static_points_and_window_level_animation_split_statically() {
        let got = accepts(&Param::Fixed(-0.25), &Param::Fixed(0.25), 0.25);
        assert_eq!(got, Param::Fixed(-0.125));
    }

    #[test]
    fn linear_and_hold_on_different_key_grids_keep_every_existing_key_time() {
        let a = keys(&[(0, 0.0, L), (10, 2.0, L), (40, -1.0, H)]);
        let b = keys(&[(5, 4.0, L), (20, 0.0, L)]);
        let got = accepts(&a, &b, 0.3);
        let Param::Keys(k) = &got else { panic!() };
        assert_eq!(
            k.iter().map(|k| k.frame).collect::<Vec<_>>(),
            [0, 5, 10, 20, 40]
        );
        let a = keys(&[(0, 0.0, H), (10, 2.0, H), (30, 1.0, H)]);
        let b = keys(&[(5, 4.0, H), (20, 0.0, L)]);
        accepts(&a, &b, 0.6);
        // A Fixed end embeds into either.
        accepts(&a, &Param::Fixed(3.0), 0.5);
    }

    #[test]
    fn shared_time_bezier_with_different_values_and_embedded_linear_are_exact() {
        let ease = |dv: f64| bez([0.4, dv], [0.3, -dv]);
        let a = keys(&[(0, 0.0, ease(1.0)), (30, 2.0, L)]);
        let b = keys(&[(0, 1.0, ease(-3.0)), (30, -1.0, L)]);
        let got = accepts(&a, &b, 0.5);
        let Param::Keys(k) = &got else { panic!() };
        assert!(matches!(k[0].interp, Interp::Bezier { .. }));
        // A Bezier plus Linear/Fixed on an overlapping grid.
        let c = keys(&[(0, 0.0, L), (30, 3.0, L)]);
        accepts(&a, &c, 0.25);
        // A Bezier that is actually straight in time joins any other easing.
        let straight = keys(&[(0, 0.0, bez([0.1, 0.2], [0.6, -1.2])), (30, 2.0, L)]);
        accepts(&straight, &a, 0.5);
        // Curves of one time parameterization that cancel leave room for another easing.
        let up = keys(&[(0, 0.0, bez([0.2, 3.0], [0.7, -1.0])), (30, 1.0, L)]);
        let down = keys(&[(0, 0.0, bez([0.2, -3.0], [0.7, 1.0])), (30, -1.0, L)]);
        let terms = [(0.25, &up), (0.25, &down), (0.5, &a)];
        assert_exact(&terms, &blend(&terms, bounds::COORDINATE).unwrap());
        // Another term's keys split a shared easing identically for both curves.
        let grid = keys(&[(10, 0.0, L), (20, 1.0, H)]);
        let terms = [(0.25, &a), (0.25, &b), (0.5, &grid)];
        let got = blend(&terms, bounds::COORDINATE).unwrap();
        assert_exact(&terms, &got);
        let Param::Keys(k) = &got else { panic!() };
        assert_eq!(
            k.iter().map(|k| k.frame).collect::<Vec<_>>(),
            [0, 10, 20, 30]
        );
    }

    #[test]
    fn a_held_jump_with_a_constant_continuous_part_becomes_hold() {
        let a = keys(&[(0, 0.0, H), (10, 2.0, L)]);
        let b = keys(&[(10, 0.0, L), (20, 4.0, L)]);
        let got = accepts(&a, &b, 0.5);
        let Param::Keys(k) = &got else { panic!() };
        assert_eq!(k[0].interp, Interp::Hold);
        // Holds whose jumps cancel are continuous.
        let up = keys(&[(0, 0.0, H), (10, 2.0, L)]);
        let down = keys(&[(0, 2.0, H), (10, 0.0, L)]);
        let c = keys(&[(0, 0.0, L), (10, 1.0, L)]);
        let terms = [(0.25, &up), (0.25, &down), (0.5, &c)];
        assert_exact(&terms, &blend(&terms, bounds::COORDINATE).unwrap());
    }

    #[test]
    fn a_hold_jump_against_a_continuous_change_is_refused_with_its_interval() {
        let a = keys(&[(0, 0.0, H), (10, 2.0, L)]);
        let b = keys(&[(0, 0.0, L), (10, 2.0, L)]);
        let error = blend(&[(0.5, &a), (0.5, &b)], bounds::COORDINATE).unwrap_err();
        assert_eq!(error.why, Why::Jump);
        assert_eq!(error.frames, Some([0, 10]));
        assert_eq!(error.terms, [0, 1]);
    }

    #[test]
    fn different_nonlinear_time_parameterizations_are_refused() {
        let a = keys(&[(0, 0.0, bez([0.4, 1.0], [0.3, -1.0])), (30, 2.0, L)]);
        let b = keys(&[(0, 0.0, bez([0.1, 1.0], [0.6, -1.0])), (30, 2.0, L)]);
        let error = blend(&[(0.5, &a), (0.5, &b)], bounds::COORDINATE).unwrap_err();
        assert_eq!((error.why, error.frames), (Why::Timing, Some([0, 30])));
    }

    #[test]
    fn a_clamped_overshoot_is_refused_rather_than_scaled() {
        let wild = keys(&[
            (0, 0.0, bez([1.0 / 3.0, 40.0], [1.0 / 3.0, 40.0])),
            (10, 0.0, L),
        ]);
        let error = blend(
            &[(0.5, &wild), (0.5, &Param::Fixed(0.0))],
            bounds::COORDINATE,
        )
        .unwrap_err();
        assert_eq!(
            (error.why, error.terms, error.frames),
            (Why::Overshoot, vec![0], Some([0, 10]))
        );
    }

    #[test]
    fn a_generated_value_outside_the_coordinate_range_is_refused() {
        let error = blend(
            &[(1.0, &Param::Fixed(15.0)), (1.0, &Param::Fixed(15.0))],
            bounds::COORDINATE,
        )
        .unwrap_err();
        assert_eq!(error.why, Why::Range);
    }

    #[test]
    fn a_split_whose_time_handle_cannot_be_stored_is_refused() {
        let ease = keys(&[(0, 0.0, bez([1.0, 0.0], [1.0, 0.0])), (500, 1.0, L)]);
        let other = keys(&[(234, 0.0, L), (252, 1.0, L)]);
        let error = blend(&[(0.5, &ease), (0.5, &other)], bounds::COORDINATE).unwrap_err();
        assert_eq!((error.why, error.frames), (Why::Encoding, Some([234, 252])));
    }

    fn polygon(points: Vec<WindowPoint>) -> WindowShape {
        WindowShape::Polygon {
            points,
            inside: 0.0.into(),
            outside: 0.0.into(),
        }
    }

    #[test]
    fn polygon_insertion_splits_its_edge_and_keeps_every_other_point() {
        let mut shape = WindowShape::polygon();
        let WindowShape::Polygon { points: before, .. } = shape.clone() else {
            panic!()
        };
        assert_eq!(shape.insert_polygon_point(3, 0.5), Ok(4));
        let WindowShape::Polygon { points, .. } = &shape else {
            panic!()
        };
        assert_eq!(points.len(), 5);
        assert_eq!(points[..4], before[..]);
        assert_eq!(points[4], WindowPoint::new(-0.25, 0.0));
        // Animated ends split their key grid; the other point keeps its keys untouched.
        let mut animated = before.clone();
        animated[1].x = keys(&[(0, 0.25, L), (10, 0.5, H)]);
        animated[2].y = keys(&[(5, 0.25, H), (15, 0.75, L)]);
        let mut shape = polygon(animated.clone());
        assert_eq!(shape.insert_polygon_point(1, 0.25), Ok(2));
        let WindowShape::Polygon { points, .. } = &shape else {
            panic!()
        };
        assert_eq!(points[1], animated[1]);
        assert_eq!(points[3], animated[2]);
        assert_exact(
            &[(0.75, &animated[1].x), (0.25, &animated[2].x)],
            &points[2].x,
        );
        assert_exact(
            &[(0.75, &animated[1].y), (0.25, &animated[2].y)],
            &points[2].y,
        );
    }

    #[test]
    fn polygon_insertion_refusals_name_points_and_leave_the_shape_unchanged() {
        let mut points: Vec<_> = (0..4)
            .map(|i| WindowPoint::new(f64::from(i), 0.0))
            .collect();
        points[0].y = keys(&[(0, 0.0, H), (10, 2.0, L)]);
        points[1].y = keys(&[(0, 0.0, L), (10, 2.0, L)]);
        let mut shape = polygon(points);
        let before = shape.clone();
        assert_eq!(
            shape.insert_polygon_point(0, 0.5),
            Err(TopologyError::Shape {
                axis: "y",
                points: vec![0, 1],
                why: Why::Jump,
                frames: Some([0, 10]),
            })
        );
        assert_eq!(shape, before);
        // Another edge is independent of that animation.
        assert_eq!(shape.insert_polygon_point(2, 0.5), Ok(3));
        let mut full = polygon(
            (0..128)
                .map(|i| WindowPoint::new(f64::from(i) / 128.0, f64::from(i % 2)))
                .collect(),
        );
        let before = full.clone();
        assert_eq!(
            full.insert_polygon_point(0, 0.5),
            Err(TopologyError::Maximum)
        );
        assert_eq!(full, before);
    }

    #[test]
    fn polygon_removal_keeps_three_points_and_every_other_param() {
        let mut shape = WindowShape::polygon();
        let WindowShape::Polygon { points: before, .. } = shape.clone() else {
            panic!()
        };
        shape.remove_polygon_point(1).unwrap();
        let WindowShape::Polygon { points, .. } = &shape else {
            panic!()
        };
        assert_eq!(
            points[..],
            [before[0].clone(), before[2].clone(), before[3].clone()]
        );
        let three = shape.clone();
        assert_eq!(shape.remove_polygon_point(0), Err(TopologyError::Minimum));
        assert_eq!(shape, three);
        assert_eq!(
            WindowShape::linear().remove_polygon_point(0),
            Err(TopologyError::Unsupported)
        );
    }
}
