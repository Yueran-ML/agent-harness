//! Curve Window geometry shared by the mask and the native controls: a path of cubic Bezier
//! edges flattened by adaptive de Casteljau subdivision. Each straight piece keeps its original
//! edge and parameter interval, so a control can find the exact parameter of a click on an edge.
//!
//! The mask flattens closed Curves only, in f64, until each piece's control hull lies within
//! [`ERROR`] image heights of its chord after the largest transform scale ([`tolerance`]): at
//! most .125 pixels on a picture 16384 high. The controls may flatten more coarsely for drawing,
//! never more finely than the mask, and an open draft is flattened without its closing edge.

#![allow(
    clippy::many_single_char_names,
    reason = "Bezier control points and de Casteljau levels in plane geometry notation"
)]

/// The mask's tessellation error in image heights at unit transform scale.
pub const ERROR: f64 = 1.0 / (8.0 * 16384.0);

/// One straight piece of edge `edge` (anchor `edge` to the next), covering cubic parameters `t`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurveSegment {
    pub a: [f64; 2],
    pub b: [f64; 2],
    pub edge: usize,
    pub t: [f64; 2],
}

/// An evaluated anchor: its position and its incoming and outgoing handles, relative to it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    pub position: [f64; 2],
    pub into: [f64; 2],
    pub out: [f64; 2],
}

/// The local tolerance the mask flattens with, for the transform's largest column scale.
#[must_use]
pub fn tolerance(scale: f64) -> f64 {
    ERROR / scale
}

pub(crate) fn distance(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let d = [b[0] - a[0], b[1] - a[1]];
    let n = d[0] * d[0] + d[1] * d[1];
    let t = if n == 0.0 {
        0.0
    } else {
        (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / n).clamp(0.0, 1.0)
    };
    (p[0] - a[0] - t * d[0]).hypot(p[1] - a[1] - t * d[1])
}

/// The control points of edge `j`: `[A, A + A.out, B + B.in, B]`, B the next anchor (wrapping).
#[must_use]
pub fn cubic(anchors: &[Anchor], j: usize) -> [[f64; 2]; 4] {
    let (a, b) = (anchors[j], anchors[(j + 1) % anchors.len()]);
    let (p, q) = (a.position, b.position);
    [
        p,
        [p[0] + a.out[0], p[1] + a.out[1]],
        [q[0] + b.into[0], q[1] + b.into[1]],
        q,
    ]
}

/// Split `cubic` by halving until each piece's hull is within `tolerance` of its chord.
pub fn flatten(cubic: [[f64; 2]; 4], tolerance: f64, edge: usize, out: &mut Vec<CurveSegment>) {
    let mut pending = vec![(cubic, [0.0, 1.0])];
    while let Some((p, t)) = pending.pop() {
        if distance(p[1], p[0], p[3]).max(distance(p[2], p[0], p[3])) <= tolerance {
            out.push(CurveSegment {
                a: p[0],
                b: p[3],
                edge,
                t,
            });
        } else {
            let mid = |a: [f64; 2], b: [f64; 2]| [a[0].midpoint(b[0]), a[1].midpoint(b[1])];
            let a = mid(p[0], p[1]);
            let b = mid(p[1], p[2]);
            let c = mid(p[2], p[3]);
            let d = mid(a, b);
            let e = mid(b, c);
            let f = mid(d, e);
            let tm = t[0].midpoint(t[1]);
            pending.push(([f, e, c, p[3]], [tm, t[1]]));
            pending.push(([p[0], a, d, f], [t[0], tm]));
        }
    }
}

/// Every edge of the path in order: all `n` edges when `closed`, else the `n - 1` drawn ones.
/// A non-positive or non-finite tolerance flattens nothing.
#[must_use]
pub fn segments(anchors: &[Anchor], closed: bool, tolerance: f64) -> Vec<CurveSegment> {
    let mut out = Vec::new();
    if !(tolerance > 0.0 && tolerance.is_finite()) {
        return out;
    }
    let edges = if closed {
        anchors.len()
    } else {
        anchors.len().saturating_sub(1)
    };
    for j in 0..edges {
        flatten(cubic(anchors, j), tolerance, j, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(
        clippy::cast_possible_truncation,
        reason = "f32 packing error measured"
    )]
    fn curve_tessellation_meets_dense_bernstein_reference_and_keeps_original_intervals() {
        let cubics = [
            [[-0.25, 0.0], [-0.25, -0.4], [0.25, -0.4], [0.25, 0.0]],
            [[0.0, 0.0], [16.0, 16.0], [-16.0, 16.0], [0.0, 0.0]],
            [[-16.0, -16.0], [0.0, 0.0], [32.0, 32.0], [16.0, 16.0]],
        ];
        for scale in [1.0, 40.0] {
            for cubic in cubics {
                let mut segments = Vec::new();
                flatten(cubic, tolerance(scale), 7, &mut segments);
                let mut next = 0;
                let mut most = 0.0_f64;
                let mut rounded = 0.0_f64;
                for sample in 0..=20000 {
                    let t = f64::from(sample) / 20000.0;
                    while next + 1 < segments.len() && segments[next].t[1] < t {
                        next += 1;
                    }
                    let s = segments[next];
                    assert_eq!(s.edge, 7);
                    assert!(s.t[0] <= t && t <= s.t[1]);
                    // Independent Bernstein evaluation, not the subdivision algorithm.
                    let q = [0, 1].map(|j| {
                        (1.0 - t).powi(3) * cubic[0][j]
                            + 3.0 * (1.0 - t).powi(2) * t * cubic[1][j]
                            + 3.0 * (1.0 - t) * t * t * cubic[2][j]
                            + t.powi(3) * cubic[3][j]
                    });
                    most = most.max(distance(q, s.a, s.b) * scale * 16384.0);
                    rounded = rounded.max(
                        distance(
                            q,
                            s.a.map(|v| f64::from(v as f32)),
                            s.b.map(|v| f64::from(v as f32)),
                        ) * scale
                            * 16384.0,
                    );
                }
                assert!(
                    most <= 0.125 + 1e-7,
                    "error={most}, segments={}",
                    segments.len()
                );
                eprintln!("T23 cubic scale={scale} segments={} double_error_px={most:.6} packed_error_px={rounded:.6}",segments.len());
            }
        }
    }

    #[test]
    fn the_handles_of_an_edge_are_offsets_from_its_anchors() {
        let anchors = [
            Anchor {
                position: [1.0, 2.0],
                into: [-0.5, -0.25],
                out: [0.5, 0.25],
            },
            Anchor {
                position: [3.0, -1.0],
                into: [-0.75, 0.5],
                out: [0.25, 0.125],
            },
        ];
        assert_eq!(
            cubic(&anchors, 0),
            [[1.0, 2.0], [1.5, 2.25], [2.25, -0.5], [3.0, -1.0]]
        );
        assert_eq!(
            cubic(&anchors, 1),
            [[3.0, -1.0], [3.25, -0.875], [0.5, 1.75], [1.0, 2.0]]
        );
    }

    #[test]
    fn an_open_path_has_no_closing_edge_and_a_closed_one_wraps() {
        let anchor = |x: f64, y: f64| Anchor {
            position: [x, y],
            into: [0.0; 2],
            out: [0.0; 2],
        };
        let path = [anchor(0.0, 0.0), anchor(1.0, 0.0), anchor(1.0, 1.0)];
        let edges = |closed| {
            let mut found: Vec<usize> = segments(&path, closed, 0.01)
                .iter()
                .map(|s| s.edge)
                .collect();
            found.dedup();
            found
        };
        assert_eq!(edges(false), [0, 1]);
        assert_eq!(edges(true), [0, 1, 2]);
        let closing = segments(&path, true, 0.01);
        assert_eq!(closing.last().unwrap().b, [0.0, 0.0]);
        assert_eq!(segments(&path[..1], false, 0.01), []);
        assert_eq!(segments(&path, true, 0.0), []);
    }
}
