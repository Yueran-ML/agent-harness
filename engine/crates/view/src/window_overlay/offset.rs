//! Exact softness guides of Linear and Polygon Windows, in local image-height units.
//!
//! The compose mask of both shapes measures the distance to the nearest outline edge and
//! decides inside by even-odd. Its opaque contour (inside) and its zero contour (outside) are
//! the boundary of the points on that side lying at least `radii[k]` from every edge `k`:
//! parallel edge offsets, mitred where two offsets meet, and arcs about vertices where the
//! outline turns away. A piece nearer to any edge than that edge's extent is dropped, so a
//! guide never lies on the wrong side and disappears when the shape cannot hold it.

#![allow(
    clippy::many_single_char_names,
    clippy::float_cmp,
    reason = "plane geometry notation; exact equality marks shared vertices and run ends"
)]

use std::collections::HashSet;
use std::f64::consts::TAU;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Piece {
    Line([f64; 2], [f64; 2]),
    /// About `centre`, counter-clockwise in local axes from `start` through `sweep` radians.
    Arc {
        centre: [f64; 2],
        radius: f64,
        start: f64,
        sweep: f64,
    },
}

/// A capsule boundary: an edge offset (parameter 0–1) or a circle about a vertex (angle).
#[derive(Clone, Copy, Debug)]
enum Curve {
    Line([f64; 2], [f64; 2]),
    Circle([f64; 2], f64),
}

/// Parameters closer than this are one breakpoint.
const TINY: f64 = 1e-12;
/// Pieces shorter than this many image heights are slivers of a tangency, not drawn.
const SHORTEST: f64 = 1e-7;

fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn cross(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn dot(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

/// The distance to a segment, as the compose mask measures it.
pub(crate) fn distance(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let d = sub(b, a);
    let n = dot(d, d);
    let t = if n == 0.0 {
        0.0
    } else {
        (dot(sub(p, a), d) / n).clamp(0.0, 1.0)
    };
    (p[0] - a[0] - t * d[0]).hypot(p[1] - a[1] - t * d[1])
}

/// Even-odd inside, the same crossing rule as the compose mask and the body hit.
pub(crate) fn inside(points: &[[f64; 2]], p: [f64; 2]) -> bool {
    let mut inside = false;
    for (j, a) in points.iter().enumerate() {
        let b = points[(j + 1) % points.len()];
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            inside = !inside;
        }
    }
    inside
}

impl Curve {
    fn at(self, s: f64) -> [f64; 2] {
        match self {
            Self::Line(a, b) => [a[0] + s * (b[0] - a[0]), a[1] + s * (b[1] - a[1])],
            Self::Circle(c, r) => [c[0] + r * s.cos(), c[1] + r * s.sin()],
        }
    }

    fn bounds(self) -> [f64; 4] {
        match self {
            Self::Line(a, b) => [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[0].max(b[0]),
                a[1].max(b[1]),
            ],
            Self::Circle(c, r) => [c[0] - r, c[1] - r, c[0] + r, c[1] + r],
        }
    }

    /// The parameter of a point on this curve.
    fn parameter(self, p: [f64; 2]) -> f64 {
        match self {
            Self::Line(a, b) => {
                let d = sub(b, a);
                dot(sub(p, a), d) / dot(d, d)
            }
            Self::Circle(c, _) => (p[1] - c[1]).atan2(p[0] - c[0]).rem_euclid(TAU),
        }
    }

    /// Where two capsule boundaries cross, within both edge offsets' extents.
    fn meet(self, other: Self) -> Vec<[f64; 2]> {
        match (self, other) {
            (Self::Line(a, b), Self::Line(c, e)) => {
                let (d, f) = (sub(b, a), sub(e, c));
                let den = cross(d, f);
                if den.abs() <= 1e-15 * dot(d, d).sqrt() * dot(f, f).sqrt() {
                    return vec![];
                }
                let s = cross(sub(c, a), f) / den;
                let t = cross(sub(c, a), d) / den;
                if (0.0..=1.0).contains(&s) && (0.0..=1.0).contains(&t) {
                    vec![self.at(s)]
                } else {
                    vec![]
                }
            }
            (Self::Line(a, b), Self::Circle(c, r)) => {
                let (d, f) = (sub(b, a), sub(a, c));
                let qa = dot(d, d);
                let qb = 2.0 * dot(f, d);
                let disc = qb * qb - 4.0 * qa * (dot(f, f) - r * r);
                if qa == 0.0 || disc < 0.0 {
                    return vec![];
                }
                let root = disc.sqrt();
                [(-qb - root) / (2.0 * qa), (-qb + root) / (2.0 * qa)]
                    .into_iter()
                    .filter(|s| (0.0..=1.0).contains(s))
                    .map(|s| self.at(s))
                    .collect()
            }
            (Self::Circle(..), Self::Line(..)) => other.meet(self),
            (Self::Circle(c, r), Self::Circle(e, q)) => {
                let d = sub(e, c);
                let l = d[0].hypot(d[1]);
                if l == 0.0 || l > r + q || l < (r - q).abs() {
                    return vec![];
                }
                let along = (r * r - q * q + l * l) / (2.0 * l);
                let h = (r * r - along * along).max(0.0).sqrt();
                let m = [c[0] + along * d[0] / l, c[1] + along * d[1] / l];
                vec![
                    [m[0] - h * d[1] / l, m[1] + h * d[0] / l],
                    [m[0] + h * d[1] / l, m[1] - h * d[0] / l],
                ]
            }
        }
    }
}

/// The guide on one side of the outline: `inside` for the opaque contour, otherwise the zero
/// contour. `radii[k]` is edge `k`'s extent on that side; zero or invalid extents have none.
#[allow(
    clippy::too_many_lines,
    reason = "one construction: candidates, breakpoints, kept runs"
)]
pub(crate) fn guide(points: &[[f64; 2]], radii: &[f64], inside_side: bool) -> Vec<Piece> {
    let n = points.len();
    if n < 2 || points.iter().flatten().any(|v| !v.is_finite()) {
        return vec![];
    }
    let radius = |k: usize| {
        radii
            .get(k)
            .copied()
            .filter(|r| r.is_finite() && *r > 0.0)
            .unwrap_or(0.0)
    };
    let edge = |k: usize| (points[k], points[(k + 1) % n]);
    let mut curves = vec![];
    let mut circles = HashSet::new();
    for k in 0..n {
        let r = radius(k);
        if r == 0.0 {
            continue;
        }
        let (a, b) = edge(k);
        let d = sub(b, a);
        let length = d[0].hypot(d[1]);
        if length > 0.0 {
            let normal = [-d[1] / length * r, d[0] / length * r];
            for side in [1.0, -1.0] {
                let shift = [normal[0] * side, normal[1] * side];
                curves.push(Curve::Line(
                    [a[0] + shift[0], a[1] + shift[1]],
                    [b[0] + shift[0], b[1] + shift[1]],
                ));
            }
        }
        for c in [a, b] {
            if circles.insert([c[0].to_bits(), c[1].to_bits(), r.to_bits()]) {
                curves.push(Curve::Circle(c, r));
            }
        }
    }
    // Breakpoints: crossings with every other capsule boundary, found by a sweep over x.
    let bounds: Vec<_> = curves.iter().map(|c| c.bounds()).collect();
    let mut order: Vec<usize> = (0..curves.len()).collect();
    order.sort_by(|&i, &j| bounds[i][0].total_cmp(&bounds[j][0]));
    let mut cuts = vec![vec![]; curves.len()];
    // A circle meets the offsets of its own edges tangentially, where a crossing is unstable:
    // cut it exactly at those edges' normals instead.
    for (curve, cuts) in curves.iter().zip(&mut cuts) {
        let Curve::Circle(c, _) = *curve else {
            continue;
        };
        for k in 0..n {
            let (a, b) = edge(k);
            if a == c || b == c {
                let d = sub(b, a);
                if d != [0.0, 0.0] {
                    let normal = d[0].atan2(-d[1]);
                    cuts.extend([normal.rem_euclid(TAU), (normal + TAU / 2.0).rem_euclid(TAU)]);
                }
            }
        }
    }
    for (at, &i) in order.iter().enumerate() {
        for &j in &order[at + 1..] {
            if bounds[j][0] > bounds[i][2] {
                break;
            }
            if bounds[j][1] > bounds[i][3] || bounds[i][1] > bounds[j][3] {
                continue;
            }
            for p in curves[i].meet(curves[j]) {
                cuts[i].push(curves[i].parameter(p));
                cuts[j].push(curves[j].parameter(p));
            }
        }
    }
    // Each edge's box grown by its extent: outside it, that edge is certainly not nearer.
    let reach: Vec<_> = (0..n)
        .filter(|&k| radius(k) > 0.0)
        .map(|k| {
            let ((a, b), r) = (edge(k), radius(k));
            (
                k,
                [
                    a[0].min(b[0]) - r,
                    a[1].min(b[1]) - r,
                    a[0].max(b[0]) + r,
                    a[1].max(b[1]) + r,
                ],
            )
        })
        .collect();
    let overlaps =
        |p: [f64; 4], q: [f64; 4]| p[0] <= q[2] && q[0] <= p[2] && p[1] <= q[3] && q[1] <= p[3];
    // On a curve only the edges whose grown box meets the curve's box can be nearer.
    let kept = |p: [f64; 2], near: &[usize]| {
        near.iter().all(|&k| {
            let (a, b) = edge(k);
            distance(p, a, b) >= radius(k) * (1.0 - 1e-9)
        }) && inside(points, p) == inside_side
    };
    let mut out = vec![];
    for ((curve, mut cuts), bounds) in curves.into_iter().zip(cuts).zip(bounds) {
        let near: Vec<usize> = reach
            .iter()
            .filter(|(_, grown)| overlaps(*grown, bounds))
            .map(|(k, _)| *k)
            .collect();
        let (low, high) = match curve {
            Curve::Line(..) => (0.0, 1.0),
            Curve::Circle(..) => {
                // Start at a breakpoint, so a kept run never straddles the angle origin.
                let start = cuts.iter().copied().fold(f64::INFINITY, f64::min);
                if start.is_finite() {
                    (start, start + TAU)
                } else {
                    (0.0, TAU)
                }
            }
        };
        cuts.retain(|s| *s > low && *s < high);
        cuts.extend([low, high]);
        cuts.sort_by(f64::total_cmp);
        cuts.dedup_by(|b, a| *b - *a <= TINY);
        let mut runs: Vec<(f64, f64)> = vec![];
        for pair in cuts.windows(2) {
            let (from, to) = (pair[0], pair[1]);
            if kept(curve.at(from.midpoint(to)), &near) {
                match runs.last_mut() {
                    Some(run) if run.1 == from => run.1 = to,
                    _ => runs.push((from, to)),
                }
            }
        }
        // A circle's last run continues into its first one through the start angle.
        if let (Curve::Circle(..), [first, .., last]) = (curve, runs.as_slice()) {
            if first.0 == low && last.1 == high {
                let merged = (last.0, first.1 + TAU);
                runs.pop();
                runs[0] = merged;
            }
        }
        for (from, to) in runs {
            let piece = match curve {
                Curve::Line(a, b) => {
                    let length = (to - from) * (b[0] - a[0]).hypot(b[1] - a[1]);
                    (length > SHORTEST).then(|| Piece::Line(curve.at(from), curve.at(to)))
                }
                Curve::Circle(centre, radius) => {
                    ((to - from) * radius > SHORTEST).then_some(Piece::Arc {
                        centre,
                        radius,
                        start: from,
                        sweep: to - from,
                    })
                }
            };
            out.extend(piece);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rectangle() -> Vec<[f64; 2]> {
        vec![[-0.25, -0.25], [0.25, -0.25], [0.25, 0.25], [-0.25, 0.25]]
    }

    /// Points along every piece, with its ends.
    fn samples(pieces: &[Piece]) -> Vec<[f64; 2]> {
        let mut out = vec![];
        for piece in pieces {
            for i in 0..=32 {
                let s = f64::from(i) / 32.0;
                out.push(match *piece {
                    Piece::Line(a, b) => [a[0] + s * (b[0] - a[0]), a[1] + s * (b[1] - a[1])],
                    Piece::Arc {
                        centre,
                        radius,
                        start,
                        sweep,
                    } => {
                        let t = start + s * sweep;
                        [centre[0] + radius * t.cos(), centre[1] + radius * t.sin()]
                    }
                });
            }
        }
        out
    }

    fn nearest(points: &[[f64; 2]], p: [f64; 2]) -> f64 {
        (0..points.len())
            .map(|k| distance(p, points[k], points[(k + 1) % points.len()]))
            .fold(f64::INFINITY, f64::min)
    }

    /// Every guide point is on its side at exactly the uniform extent from the outline.
    fn exact(points: &[[f64; 2]], r: f64, side: bool, pieces: &[Piece]) {
        for p in samples(pieces) {
            assert!((nearest(points, p) - r).abs() < 1e-9, "{p:?}");
            assert_eq!(inside(points, p), side, "{p:?}");
        }
    }

    #[test]
    fn rectangle_inner_guide_is_mitred_and_outer_guide_has_corner_arcs() {
        let points = rectangle();
        let inner = guide(&points, &[0.05; 4], true);
        exact(&points, 0.05, true, &inner);
        assert_eq!(inner.len(), 4);
        for piece in &inner {
            let Piece::Line(a, b) = *piece else {
                panic!("{piece:?}")
            };
            // Each side of the inner rectangle ends exactly at its mitred corners (±0.2).
            for v in a.into_iter().chain(b) {
                assert!((v.abs() - 0.2).abs() < 1e-12, "{piece:?}");
            }
        }
        let outer = guide(&points, &[0.05; 4], false);
        exact(&points, 0.05, false, &outer);
        let arcs: Vec<_> = outer
            .iter()
            .filter_map(|p| match *p {
                Piece::Arc { sweep, centre, .. } => Some((sweep, centre)),
                Piece::Line(..) => None,
            })
            .collect();
        assert_eq!(arcs.len(), 4, "{outer:?}");
        for (sweep, centre) in arcs {
            assert!((sweep - TAU / 4.0).abs() < 1e-6);
            assert!(points.contains(&centre));
        }
        let lines = outer.len() - 4;
        assert_eq!(lines, 4, "{outer:?}");
    }

    #[test]
    fn reflex_corner_rounds_the_inner_guide_and_mitres_the_outer_guide() {
        // An L: the corner (0, 0) is reflex.
        let points = vec![
            [-0.3, -0.3],
            [0.3, -0.3],
            [0.3, 0.0],
            [0.0, 0.0],
            [0.0, 0.3],
            [-0.3, 0.3],
        ];
        let inner = guide(&points, &[0.05; 6], true);
        exact(&points, 0.05, true, &inner);
        let reflex: Vec<_> = inner
            .iter()
            .filter(|p| matches!(p, Piece::Arc { centre, .. } if *centre == [0.0, 0.0]))
            .collect();
        assert_eq!(reflex.len(), 1, "{inner:?}");
        assert!(matches!(reflex[0], Piece::Arc { sweep, .. } if (sweep - TAU / 4.0).abs() < 1e-6));
        let outer = guide(&points, &[0.05; 6], false);
        exact(&points, 0.05, false, &outer);
        // Outside the reflex corner the two offsets meet at the mitre (0.05, 0.05), no arc.
        assert!(!outer
            .iter()
            .any(|p| matches!(p, Piece::Arc { centre, .. } if *centre == [0.0, 0.0])));
        let mitre = [0.05, 0.05];
        let ends: Vec<_> = outer
            .iter()
            .filter_map(|p| match *p {
                Piece::Line(a, b) => Some([a, b]),
                Piece::Arc { .. } => None,
            })
            .flatten()
            .filter(|e| (e[0] - mitre[0]).hypot(e[1] - mitre[1]) < 1e-12)
            .collect();
        assert_eq!(ends.len(), 2, "{outer:?}");
    }

    #[test]
    fn a_narrow_polygon_holds_no_inner_guide_and_no_guide_lies_on_the_wrong_side() {
        let points = vec![[-0.4, -0.05], [0.4, -0.05], [0.4, 0.05], [-0.4, 0.05]];
        // Inside 40 is .2 image heights, more than the half height .05 the shape holds.
        assert_eq!(guide(&points, &[0.2; 4], true), vec![]);
        let outer = guide(&points, &[0.2; 4], false);
        exact(&points, 0.2, false, &outer);
        // At .04 the inner guide is a thin mitred rectangle, still inside.
        let inner = guide(&points, &[0.04; 4], true);
        exact(&points, 0.04, true, &inner);
        assert_eq!(inner.len(), 4);
        // At .05 exactly the inner region is a line of zero width; nothing that is drawn
        // can lie outside the window.
        for p in samples(&guide(&points, &[0.05; 4], true)) {
            assert!(inside(&points, p) && nearest(&points, p) >= 0.05 - 1e-9);
        }
    }

    #[test]
    fn self_intersecting_and_degenerate_outlines_keep_the_distance_and_side() {
        let star: Vec<_> = (0..5)
            .map(|k| {
                let a = (f64::from(k) * 144.0 - 90.0).to_radians();
                [0.25 * a.cos(), 0.25 * a.sin()]
            })
            .collect();
        for side in [true, false] {
            let pieces = guide(&star, &[0.02; 5], side);
            assert_ne!(pieces, vec![]);
            exact(&star, 0.02, side, &pieces);
        }
        // Repeated points and a zero-length edge.
        let points = vec![[0.0, 0.0], [0.2, 0.0], [0.2, 0.0], [0.0, 0.2]];
        for side in [true, false] {
            exact(&points, 0.01, side, &guide(&points, &[0.01; 4], side));
        }
        assert_eq!(guide(&points, &[0.0; 4], false), vec![]);
        assert_eq!(guide(&points, &[f64::NAN; 4], false), vec![]);
    }

    #[test]
    fn per_edge_extents_bound_the_linear_contours() {
        // Linear: each side has its own extent; a zero side contributes no offset.
        let points = rectangle();
        let radii = [0.02, 0.0, 0.08, 0.04];
        for side in [true, false] {
            let pieces = guide(&points, &radii, side);
            for p in samples(&pieces) {
                let gaps: Vec<_> = (0..4)
                    .filter(|&k| radii[k] > 0.0)
                    .map(|k| distance(p, points[k], points[(k + 1) % 4]) - radii[k])
                    .collect();
                assert!(gaps.iter().all(|g| *g >= -1e-9), "{p:?} {gaps:?}");
                assert!(gaps.iter().any(|g| g.abs() < 1e-9), "{p:?} {gaps:?}");
                // A guide may end on a side without softness; off the outline it is on its side.
                if nearest(&points, p) > 1e-9 {
                    assert_eq!(inside(&points, p), side, "{p:?}");
                }
            }
        }
    }
}
