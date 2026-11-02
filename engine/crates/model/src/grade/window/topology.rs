//! Whole-animation Polygon and Curve topology: inserting and removing points.
//!
//! The point count is the same on every frame; adding or removing a point applies to the whole
//! animation and keeps every other point's keys. An inserted point splits its edge at one fixed
//! share (a Polygon) or cubic parameter (a Curve, by de Casteljau) for the whole animation. Its
//! coordinates, and a Curve's two shortened handles, are generated on the union of the existing
//! key times of the fields they are made from, so the shape is unchanged at every instant.
//! Where this module cannot prove that, the insertion is refused as a whole and says why: it
//! never bakes a static value, drops keys, approximates, or adds dense keys. These are project
//! rules; Resolve instead makes an animated Polygon's points pop on and off at keyframes
//! (RM20 p. 3194).

use serde::Serialize;

use super::{bounds, WindowBezierPoint, WindowPoint, WindowShape, MOST_POLYGON_POINTS};
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

/// Why a Polygon or Curve topology edit was refused; the shape is then unchanged.
#[derive(Clone, Debug, PartialEq)]
pub enum TopologyError {
    /// Not a Polygon/Curve, or an index or parameter out of range.
    Unsupported,
    /// A Polygon already has [`MOST_POLYGON_POINTS`] points.
    Maximum,
    /// Removing would leave fewer than `least` points: three on a Polygon, two on a closed Curve.
    Minimum { shape: &'static str, least: usize },
    /// The inserted point's `axis` coordinate (or a Curve handle it shortens) cannot be
    /// generated exactly.
    Shape {
        axis: &'static str,
        /// Point indices whose coordinates or handles cause the conflict.
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
            return Err(TopologyError::Minimum {
                shape: "polygon",
                least: 3,
            });
        }
        points.remove(index);
        Ok(())
    }

    /// Split Curve edge `edge` (anchor `edge` to the next, the closing edge included only when
    /// closed) at cubic parameter `t`, for the whole animation, by de Casteljau: the new anchor
    /// and its handles lie on the original cubic, the two neighbours' facing handles shorten, and
    /// every other Param is kept. An open draft splits only its drawn edges. Returns the new
    /// anchor's index.
    ///
    /// # Errors
    ///
    /// The shape is left unchanged when any generated field cannot be represented exactly.
    #[allow(
        clippy::many_single_char_names,
        reason = "de Casteljau's parameter and the edge ends A, B in the formula's notation"
    )]
    pub fn insert_curve_point(&mut self, edge: usize, t: f64) -> Result<usize, TopologyError> {
        let Self::Curve { points, closed, .. } = self else {
            return Err(TopologyError::Unsupported);
        };
        let n = points.len();
        let edges = if *closed { n } else { n.saturating_sub(1) };
        if edge >= edges || !(t > 0.0 && t < 1.0) {
            return Err(TopologyError::Unsupported);
        }
        let ends = [edge, (edge + 1) % n];
        let (a, b) = (&points[ends[0]], &points[ends[1]]);
        let (s, u) = (t, 1.0 - t);
        // Each generated field as weights of [A, A.out, B.in, B] for the control points
        // C = [A, A+A.out, B+B.in, B]: N = Σ Bernstein·C, R0/R1 the second de Casteljau level.
        let n_at = [
            u * u * u + 3.0 * u * u * s,
            3.0 * u * u * s,
            3.0 * u * s * s,
            3.0 * u * s * s + s * s * s,
        ];
        let r0 = [u * u + 2.0 * u * s, 2.0 * u * s, s * s, s * s];
        let r1 = [u * u, u * u, 2.0 * u * s, 2.0 * u * s + s * s];
        let less = |r: [f64; 4]| std::array::from_fn::<f64, 4, _>(|i| r[i] - n_at[i]);
        let owners = [ends[0], ends[0], ends[1], ends[1]];
        let generate = |weights: [f64; 4], axis: &'static str| {
            let field = |point: &'_ WindowPoint| {
                if axis == "x" {
                    point.x.clone()
                } else {
                    point.y.clone()
                }
            };
            let inputs = [
                field(&a.position),
                field(&a.out),
                field(&b.into),
                field(&b.position),
            ];
            let terms: Vec<(f64, &Param)> = weights.iter().copied().zip(inputs.iter()).collect();
            blend(&terms, bounds::COORDINATE).map_err(|conflict| {
                let mut points: Vec<usize> = conflict.terms.iter().map(|&k| owners[k]).collect();
                points.sort_unstable();
                points.dedup();
                TopologyError::Shape {
                    axis,
                    points,
                    why: conflict.why,
                    frames: conflict.frames,
                }
            })
        };
        let point = |weights: [f64; 4]| -> Result<WindowPoint, TopologyError> {
            Ok(WindowPoint {
                x: generate(weights, "x")?,
                y: generate(weights, "y")?,
            })
        };
        let inserted = WindowBezierPoint {
            position: point(n_at)?,
            into: point(less(r0))?,
            out: point(less(r1))?,
        };
        let a_out = point([0.0, s, 0.0, 0.0])?;
        let b_in = point([0.0, 0.0, u, 0.0])?;
        points[ends[0]].out = a_out;
        points[ends[1]].into = b_in;
        points.insert(edge + 1, inserted);
        Ok(edge + 1)
    }

    /// Remove Curve anchor `index` from the whole animation. A closed Curve keeps at least two
    /// anchors (it is never turned back into a draft); an open draft may become empty.
    ///
    /// # Errors
    ///
    /// The shape is left unchanged when the anchor cannot be removed.
    pub fn remove_curve_point(&mut self, index: usize) -> Result<(), TopologyError> {
        let Self::Curve { points, closed, .. } = self else {
            return Err(TopologyError::Unsupported);
        };
        if index >= points.len() {
            return Err(TopologyError::Unsupported);
        }
        if *closed && points.len() <= 2 {
            return Err(TopologyError::Minimum {
                shape: "curve",
                least: 2,
            });
        }
        points.remove(index);
        Ok(())
    }

    /// Remove several Curve anchors at once (a multi-point selection's Backspace, RM20
    /// p. 3195), from the whole animation: each anchor's Params go with it, every other Param is
    /// kept. A repeated index counts once.
    ///
    /// # Errors
    ///
    /// Nothing is removed when the list is empty or names a missing anchor
    /// ([`TopologyError::Unsupported`]), or when a closed Curve would keep fewer than two
    /// ([`TopologyError::Minimum`]); an open draft may become empty.
    pub fn remove_curve_points(&mut self, indices: &[usize]) -> Result<(), TopologyError> {
        let Self::Curve { points, closed, .. } = self else {
            return Err(TopologyError::Unsupported);
        };
        let mut gone = indices.to_vec();
        gone.sort_unstable();
        gone.dedup();
        if gone.is_empty() || gone.last().is_some_and(|&i| i >= points.len()) {
            return Err(TopologyError::Unsupported);
        }
        if *closed && points.len() - gone.len() < 2 {
            return Err(TopologyError::Minimum {
                shape: "curve",
                least: 2,
            });
        }
        for &i in gone.iter().rev() {
            points.remove(i);
        }
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

    fn anchor(p: [f64; 2], into: [f64; 2], out: [f64; 2]) -> WindowBezierPoint {
        WindowBezierPoint {
            position: WindowPoint::new(p[0], p[1]),
            into: WindowPoint::new(into[0], into[1]),
            out: WindowPoint::new(out[0], out[1]),
        }
    }
    fn curve(points: Vec<WindowBezierPoint>, closed: bool) -> WindowShape {
        WindowShape::Curve {
            points,
            closed,
            inside: 0.0.into(),
            outside: 0.0.into(),
        }
    }
    fn at(p: &WindowPoint, frame: f64) -> [f64; 2] {
        [p.x.value_at(frame), p.y.value_at(frame)]
    }
    /// The cubic of edge `j` at `frame`, as compose draws it: [A, A+out, B+in, B].
    #[allow(clippy::many_single_char_names, reason = "control point notation")]
    fn cubic(points: &[WindowBezierPoint], j: usize, frame: f64) -> [[f64; 2]; 4] {
        let (a, b) = (&points[j], &points[(j + 1) % points.len()]);
        let (p, h, k, q) = (
            at(&a.position, frame),
            at(&a.out, frame),
            at(&b.into, frame),
            at(&b.position, frame),
        );
        [p, [p[0] + h[0], p[1] + h[1]], [q[0] + k[0], q[1] + k[1]], q]
    }
    /// An independent Bernstein evaluation, not de Casteljau.
    fn eval(c: [[f64; 2]; 4], t: f64) -> [f64; 2] {
        let u = 1.0 - t;
        [0, 1].map(|i| {
            u * u * u * c[0][i]
                + 3.0 * u * u * t * c[1][i]
                + 3.0 * u * t * t * c[2][i]
                + t * t * t * c[3][i]
        })
    }
    /// Edge `j` split at `t` into edges `j` and `j+1` of `after` traces the same curve, and the
    /// other fields are untouched, at every sampled frame.
    fn assert_split(
        before: &[WindowBezierPoint],
        after: &[WindowBezierPoint],
        j: usize,
        t: f64,
        frames: &[f64],
    ) {
        assert_eq!(after.len(), before.len() + 1);
        for &frame in frames {
            let whole = cubic(before, j, frame);
            let (left, right) = (cubic(after, j, frame), cubic(after, j + 1, frame));
            for k in 0..=20 {
                let sigma = f64::from(k) / 20.0;
                for (got, want) in [
                    (eval(left, sigma), eval(whole, sigma * t)),
                    (eval(right, sigma), eval(whole, t + sigma * (1.0 - t))),
                ] {
                    assert!(
                        (got[0] - want[0]).abs() < 1e-9 && (got[1] - want[1]).abs() < 1e-9,
                        "frame {frame} sigma {sigma}: {got:?} vs {want:?}"
                    );
                }
            }
        }
        for (i, point) in after.iter().enumerate() {
            if i == j + 1 {
                continue;
            }
            let old = if i <= j { i } else { i - 1 };
            let mut kept = before[old].clone();
            if old == j {
                kept.out = point.out.clone();
            }
            if old == (j + 1) % before.len() {
                kept.into = point.into.clone();
            }
            assert_eq!(point, &kept, "point {i}");
        }
    }

    fn lens() -> Vec<WindowBezierPoint> {
        vec![
            anchor([-0.25, 0.0], [0.0, 0.1], [0.0, -0.2]),
            anchor([0.25, 0.0], [0.05, -0.15], [-0.1, 0.25]),
            anchor([0.0, 0.3], [0.2, 0.0], [-0.2, 0.0]),
        ]
    }

    #[test]
    fn curve_insertion_splits_the_original_cubic_exactly_and_keeps_other_points() {
        for (edge, t) in [(0, 0.3), (1, 0.5), (2, 0.85)] {
            let before = lens();
            let mut shape = curve(before.clone(), true);
            assert_eq!(shape.insert_curve_point(edge, t), Ok(edge + 1));
            let WindowShape::Curve { points, closed, .. } = &shape else {
                panic!()
            };
            assert!(closed);
            assert_split(&before, points, edge, t, &[0.0]);
            assert!(shape.check().is_ok());
        }
        // A corner edge stays straight: the new handles lie along the chord.
        let corner = |x: f64| anchor([x, 0.0], [0.0; 2], [0.0; 2]);
        let mut shape = curve(vec![corner(0.0), corner(1.0)], true);
        assert_eq!(shape.insert_curve_point(0, 0.25), Ok(1));
        let WindowShape::Curve { points, .. } = &shape else {
            panic!()
        };
        // A cubic with corner handles is not uniform in t: 3·.75·.25² + .25³.
        assert!((at(&points[1].position, 0.0)[0] - 0.156_25).abs() < 1e-12);
        assert_eq!(at(&points[1].into, 0.0)[1], 0.0);
    }

    #[test]
    fn an_open_draft_splits_only_its_drawn_edges() {
        let before = lens();
        let mut shape = curve(before.clone(), false);
        let unchanged = shape.clone();
        assert_eq!(
            shape.insert_curve_point(2, 0.5),
            Err(TopologyError::Unsupported)
        );
        assert_eq!(shape, unchanged);
        assert_eq!(shape.insert_curve_point(1, 0.4), Ok(2));
        let WindowShape::Curve { points, closed, .. } = &shape else {
            panic!()
        };
        assert!(!closed, "a draft stays a draft");
        assert_split(&before, points, 1, 0.4, &[0.0]);
        // A one-point draft has no edge.
        let mut one = curve(vec![before[0].clone()], false);
        assert_eq!(
            one.insert_curve_point(0, 0.5),
            Err(TopologyError::Unsupported)
        );
    }

    #[test]
    fn an_animated_curve_edge_splits_on_its_key_union_and_stays_exact_between_keys() {
        let mut before = lens();
        let ease = |dv: f64| bez([0.3, dv], [0.4, -dv]);
        before[0].position.x = keys(&[(0, -0.25, L), (20, -0.1, L), (40, -0.3, L)]);
        before[0].out.y = keys(&[(0, -0.2, ease(0.1)), (30, -0.05, L)]);
        before[1].into.x = keys(&[(0, 0.05, ease(-0.2)), (30, 0.2, L)]);
        before[1].position.y = keys(&[(10, 0.0, L), (30, 0.1, L)]);
        // Fields this edge does not use may hold anything: A.in and B.out.
        before[0].into.x = keys(&[(0, 0.0, H), (5, 0.4, L), (9, 0.0, L)]);
        before[1].out.y = keys(&[(0, 0.0, bez([0.9, 1.0], [0.05, 0.0])), (7, 0.2, L)]);
        let mut shape = curve(before.clone(), true);
        assert_eq!(shape.insert_curve_point(0, 0.35), Ok(1));
        let WindowShape::Curve { points, .. } = &shape else {
            panic!()
        };
        let frames: Vec<f64> = (-8..=200).map(|k| f64::from(k) * 0.25).collect();
        assert_split(&before, points, 0, 0.35, &frames);
        assert!(shape.check().is_ok());
    }

    #[test]
    fn a_curve_insertion_that_cannot_keep_its_shape_names_its_anchors_and_changes_nothing() {
        let mut before = lens();
        before[1].position.x = keys(&[(0, 0.25, H), (10, 0.3, L)]);
        before[0].out.x = keys(&[(0, 0.0, L), (10, 0.1, L)]);
        let mut shape = curve(before, true);
        let unchanged = shape.clone();
        let error = shape.insert_curve_point(0, 0.5).unwrap_err();
        let TopologyError::Shape {
            axis,
            points,
            why,
            frames,
        } = error
        else {
            panic!("{error:?}")
        };
        assert_eq!(
            (axis, points, why, frames),
            ("x", vec![0, 1], Why::Jump, Some([0, 10]))
        );
        assert_eq!(shape, unchanged);
        // Edge 2 (from anchor 2 to anchor 0) uses neither anchor 1 nor anchor 0's out handle.
        assert_eq!(shape.insert_curve_point(2, 0.5), Ok(3));
    }

    #[test]
    fn curve_removal_keeps_two_closed_anchors_but_lets_a_draft_empty() {
        let before = lens();
        let mut shape = curve(before.clone(), true);
        shape.remove_curve_point(1).unwrap();
        let WindowShape::Curve { points, closed, .. } = &shape else {
            panic!()
        };
        assert!(closed);
        assert_eq!(points[..], [before[0].clone(), before[2].clone()]);
        let two = shape.clone();
        assert_eq!(
            shape.remove_curve_point(0),
            Err(TopologyError::Minimum {
                shape: "curve",
                least: 2
            })
        );
        assert_eq!(shape, two);
        let mut draft = curve(before[..2].to_vec(), false);
        draft.remove_curve_point(0).unwrap();
        draft.remove_curve_point(0).unwrap();
        assert_eq!(draft, curve(vec![], false));
        assert_eq!(draft.remove_curve_point(0), Err(TopologyError::Unsupported));
    }

    #[test]
    fn removing_selected_curve_points_is_all_or_nothing_for_the_whole_animation() {
        // Four anchors, the third animated: removal takes its every key with it.
        let mut before = lens();
        before.push(anchor([-0.1, -0.3], [0.0; 2], [0.0; 2]));
        before[2].position.x = keys(&[(0, 0.0, L), (10, 0.2, L)]);
        let mut shape = curve(before.clone(), true);
        shape.remove_curve_points(&[2, 0]).unwrap();
        let WindowShape::Curve { points, closed, .. } = &shape else {
            panic!()
        };
        assert!(closed);
        assert_eq!(points[..], [before[1].clone(), before[3].clone()]);
        // A closed Curve keeps two: removing either of the two left refuses, nothing changes.
        let two = shape.clone();
        assert_eq!(
            shape.remove_curve_points(&[1]),
            Err(TopologyError::Minimum {
                shape: "curve",
                least: 2
            })
        );
        assert_eq!(shape, two);
        let mut whole = curve(before.clone(), true);
        assert_eq!(
            whole.remove_curve_points(&[0, 1, 2]),
            Err(TopologyError::Minimum {
                shape: "curve",
                least: 2
            })
        );
        assert_eq!(whole, curve(before.clone(), true));
        // A missing index or none refuses as a whole too; a repeated one counts once; a draft
        // may become empty.
        assert_eq!(
            whole.remove_curve_points(&[1, 4]),
            Err(TopologyError::Unsupported)
        );
        assert_eq!(
            whole.remove_curve_points(&[]),
            Err(TopologyError::Unsupported)
        );
        assert_eq!(whole, curve(before.clone(), true));
        whole.remove_curve_points(&[1, 1]).unwrap();
        assert_eq!(
            whole,
            curve(
                vec![before[0].clone(), before[2].clone(), before[3].clone()],
                true
            )
        );
        let mut draft = curve(before[..3].to_vec(), false);
        draft.remove_curve_points(&[0, 1, 2]).unwrap();
        assert_eq!(draft, curve(vec![], false));
        assert_eq!(
            WindowShape::polygon().remove_curve_points(&[0]),
            Err(TopologyError::Unsupported)
        );
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
        assert_eq!(
            shape.remove_polygon_point(0),
            Err(TopologyError::Minimum {
                shape: "polygon",
                least: 3
            })
        );
        assert_eq!(shape, three);
        assert_eq!(
            WindowShape::linear().remove_polygon_point(0),
            Err(TopologyError::Unsupported)
        );
    }
}
