//! Spatial window keys. These feather/combination formulas are project choices, not
//! published Resolve algorithms. Soft 100 spans .5 local image heights (half on each
//! side); Polygon/Curve inside and outside independently span .5 at 100.
//!
//! Cubics are split in f64 (`super::curve`, shared with the native controls) until their
//! control hull lies within 1/(8*16384) image heights of the chord after the largest
//! transform scale: <= .125 pixels at 16384 high. This is the tessellation error, separate
//! from f32 shader rounding. A threaded bounding-box tree prunes both even-odd ray tests
//! and nearest-edge searches. Neither window count nor Curve anchor/segment count has an
//! artificial limit.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    clippy::many_single_char_names,
    reason = "the shader parameter layout and its f32 reference"
)]

use columbina_model::grade::{
    window::bounds, Bounds, Param, Window, WindowBezierPoint, WindowCombine, WindowPoint,
    WindowShape,
};

use super::curve::{self, Anchor, CurveSegment};
use super::maths::smoothstep;

pub(crate) const LIBRARY: &str = include_str!("windows.wgsl");
const HEADER: usize = 4;
const TREE_NODE: usize = 3;

#[derive(Clone, Copy)]
pub(crate) struct Position {
    pub uv: [f32; 2],
    pub size: [f32; 2],
}

pub(crate) fn active(w: &Window) -> bool {
    w.enabled
        && match &w.shape {
            WindowShape::Curve { closed, points, .. } => *closed && points.len() >= 2,
            _ => true,
        }
}

fn value(p: &Param, bounds: Bounds, frame: f64) -> f64 {
    p.value_at(frame).clamp(bounds.low, bounds.high)
}

fn point(p: &WindowPoint, frame: f64) -> [f64; 2] {
    [
        value(&p.x, bounds::COORDINATE, frame),
        value(&p.y, bounds::COORDINATE, frame),
    ]
}

#[derive(Clone, Copy)]
struct Edge {
    a: [f32; 2],
    b: [f32; 2],
    soft: f32,
}

/// A closed Curve's flattened edges at `frame`, as the mask uses them.
pub(crate) fn curve_segments(
    points: &[WindowBezierPoint],
    frame: f64,
    scale: f64,
) -> Vec<CurveSegment> {
    if scale <= 0.0 {
        return Vec::new();
    }
    let anchors: Vec<Anchor> = points
        .iter()
        .map(|p| Anchor {
            position: point(&p.position, frame),
            into: point(&p.into, frame),
            out: point(&p.out, frame),
        })
        .collect();
    curve::segments(&anchors, true, curve::tolerance(scale))
}

fn tree(edges: &mut [Edge], out: &mut Vec<[f32; 4]>, relative: usize) {
    let mut bbox = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for edge in edges.iter() {
        for p in [edge.a, edge.b] {
            for i in 0..2 {
                bbox[i] = bbox[i].min(p[i]);
                bbox[i + 2] = bbox[i + 2].max(p[i]);
            }
        }
    }
    let at = out.len();
    out.extend([bbox, [0.0; 4], [0.0; 4]]);
    if edges.len() == 1 {
        let e = edges[0];
        out[at + 1][1] = 1.0;
        out[at + 1][2] = e.soft;
        out[at + 2] = [e.a[0], e.a[1], e.b[0], e.b[1]];
    } else {
        let axis = usize::from(bbox[3] - bbox[1] > bbox[2] - bbox[0]);
        edges.sort_unstable_by(|a, b| (a.a[axis] + a.b[axis]).total_cmp(&(b.a[axis] + b.b[axis])));
        let (left, right) = edges.split_at_mut(edges.len() / 2);
        tree(left, out, relative);
        tree(right, out, relative);
    }
    out[at + 1][0] = (relative + out.len()) as f32;
}

/// Append the windows and their trees to the existing curve tail. All addresses are relative
/// to the window descriptor block, so another operation may put the grade anywhere in params.
pub(crate) fn pack(
    windows: &[Window],
    frame: f64,
    block: usize,
    fixed: usize,
    tail: &mut Vec<[f32; 4]>,
) -> [f32; 4] {
    let windows: Vec<_> = windows.iter().filter(|w| active(w)).collect();
    let start = tail.len();
    tail.resize(start + windows.len() * HEADER, [0.0; 4]);
    let relative = fixed - block;
    for (i, w) in windows.iter().enumerate() {
        let at = start + i * HEADER;
        let matrix = w.transform.affine_at(frame, 1.0).unwrap_or([0.0; 6]);
        let inverse = w.transform.inverse_at(frame, 1.0);
        if let Some(m) = inverse {
            tail[at] = [m[0] as f32, m[2] as f32, m[1] as f32, m[3] as f32];
        }
        tail[at + 1] = [
            matrix[4] as f32,
            matrix[5] as f32,
            0.0,
            f32::from(inverse.is_some()),
        ];
        tail[at + 2] = [
            (value(&w.opacity, bounds::OPACITY, frame) / 100.0) as f32,
            f32::from(w.invert),
            f32::from(w.combine == WindowCombine::Subtract),
            0.0,
        ];
        let soft = |p: &Param| (value(p, bounds::SOFTNESS, frame) / 200.0) as f32;
        let feather = |p: &Param| (value(p, bounds::FEATHER, frame) / 200.0) as f32;
        let mut edges = Vec::new();
        let mut outline = |points: &[WindowPoint], softness: Option<&[Param; 4]>| {
            for (j, a) in points.iter().enumerate() {
                edges.push(Edge {
                    a: point(a, frame).map(|v| v as f32),
                    b: point(&points[(j + 1) % points.len()], frame).map(|v| v as f32),
                    soft: softness.map_or(0.0, |s| soft(&s[j]) * 0.5),
                });
            }
        };
        match &w.shape {
            WindowShape::Circular { softness } => {
                tail[at + 1][2] = 1.0;
                tail[at + 2][3] = soft(softness) * 0.5;
                tail[at + 3][0] = soft(softness) * 0.5;
            }
            WindowShape::Gradient { softness } => {
                tail[at + 1][2] = 2.0;
                tail[at + 2][3] = soft(softness) * 0.5;
                tail[at + 3][0] = soft(softness) * 0.5;
            }
            WindowShape::Linear { points, softness } => {
                tail[at + 1][2] = 3.0;
                outline(points.as_ref(), Some(softness));
            }
            WindowShape::Polygon {
                points,
                inside,
                outside,
            } => {
                outline(points, None);
                tail[at + 2][3] = feather(inside);
                tail[at + 3][0] = feather(outside);
            }
            WindowShape::Curve {
                points,
                inside,
                outside,
                ..
            } => {
                tail[at + 2][3] = feather(inside);
                tail[at + 3][0] = feather(outside);
                let scale = (matrix[0].hypot(matrix[1])).max(matrix[2].hypot(matrix[3]))
                    * w.tracking.as_ref().map_or(1.0, |t| t.scale_at(frame));
                edges.extend(
                    curve_segments(points, frame, scale)
                        .into_iter()
                        .map(|s| Edge {
                            a: s.a.map(|v| v as f32),
                            b: s.b.map(|v| v as f32),
                            soft: 0.0,
                        }),
                );
            }
        }
        tail[at + 3][1] = (relative + tail.len()) as f32;
        if !edges.is_empty() {
            tree(&mut edges, tail, relative);
        }
        tail[at + 3][2] = (relative + tail.len()) as f32;
        // A tracked window (T24): the homography from the picture back to where the window is
        // placed by hand, three rows after its tree; `.w` of the last header row addresses it.
        if let Some(m) = w.tracking.as_ref().and_then(|t| t.picture_inverse(frame)) {
            tail[at + 3][3] = (relative + tail.len()) as f32;
            tail.extend(m.map(|r| [r[0] as f32, r[1] as f32, r[2] as f32, 0.0]));
        }
    }
    [(relative + start) as f32, windows.len() as f32, 0.0, 0.0]
}

fn distance(p: [f32; 2], edge: [f32; 4]) -> f32 {
    let a = [edge[0], edge[1]];
    let d = [edge[2] - a[0], edge[3] - a[1]];
    let n = d[0] * d[0] + d[1] * d[1];
    let t = if n == 0.0 {
        0.0
    } else {
        (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / n).clamp(0.0, 1.0)
    };
    let x = p[0] - a[0] - t * d[0];
    let y = p[1] - a[1] - t * d[1];
    (x * x + y * y).sqrt()
}

fn feather(d: f32, inside: f32, outside: f32) -> f32 {
    if inside + outside == 0.0 {
        f32::from(d >= 0.0)
    } else {
        smoothstep(-outside, inside, d)
    }
}

/// The same packed geometry and operations as windows.wgsl. No position means no spatial
/// constraint; the graph therefore omits this key instead of sampling an invented location.
pub(crate) fn key(params: &[[f32; 4]], block: usize, position: Position) -> f32 {
    let descriptor = params[block];
    let start = block + descriptor[0] as usize;
    let mut mask = 0.0_f32;
    for i in 0..descriptor[1] as usize {
        let at = start + i * HEADER;
        let m = params[at];
        let info = params[at + 1];
        let mix = params[at + 2];
        let shape = params[at + 3];
        // A tracked window: the picture point taken back to where the window is placed by hand;
        // beyond the vanishing line of a perspective track none of the window is there.
        let (uv, seen) = if shape[3] == 0.0 {
            (position.uv, true)
        } else {
            let m = block + shape[3] as usize;
            let row = |r: [f32; 4]| r[0] * position.uv[0] + r[1] * position.uv[1] + r[2];
            let z = row(params[m + 2]);
            if z > 0.0 {
                ([row(params[m]) / z, row(params[m + 1]) / z], true)
            } else {
                (position.uv, false)
            }
        };
        let delta = [
            (uv[0] - info[0]) * position.size[0] / position.size[1],
            uv[1] - info[1],
        ];
        let p = [
            m[0] * delta[0] + m[1] * delta[1],
            m[2] * delta[0] + m[3] * delta[1],
        ];
        let mut raw = 0.0;
        if info[3] != 0.0 && seen {
            raw = match info[2] as u32 {
                1 => feather(0.25 - (p[0] * p[0] + p[1] * p[1]).sqrt(), mix[3], shape[0]),
                2 => {
                    if mix[3] + shape[0] == 0.0 {
                        f32::from(p[1] < 0.0)
                    } else {
                        feather(-p[1], mix[3], shape[0])
                    }
                }
                _ => {
                    let mut n = block + shape[1] as usize;
                    let end = block + shape[2] as usize;
                    let mut inside = false;
                    // Distances farther than the whole feather cannot change the mask.
                    let mut nearest = mix[3].max(shape[0]);
                    let mut inner = 1.0_f32;
                    let mut outer = 0.0_f32;
                    while n < end {
                        let bbox = params[n];
                        let link = params[n + 1];
                        let dx = (bbox[0] - p[0]).max(0.0).max(p[0] - bbox[2]);
                        let dy = (bbox[1] - p[1]).max(0.0).max(p[1] - bbox[3]);
                        let ray = p[1] >= bbox[1] && p[1] < bbox[3] && p[0] <= bbox[2];
                        if info[2] != 3.0 && !ray && dx * dx + dy * dy > nearest * nearest {
                            n = block + link[0] as usize;
                            continue;
                        }
                        if link[1] != 0.0 {
                            let e = params[n + 2];
                            let d = distance(p, e);
                            nearest = nearest.min(d);
                            if info[2] == 3.0 {
                                inner = inner.min(feather(d, link[2], link[2]));
                                outer = outer.max(feather(-d, link[2], link[2]));
                            }
                            if (e[1] > p[1]) != (e[3] > p[1])
                                && p[0] < (e[2] - e[0]) * (p[1] - e[1]) / (e[3] - e[1]) + e[0]
                            {
                                inside = !inside;
                            }
                        }
                        n += TREE_NODE;
                    }
                    let d = if inside { nearest } else { -nearest };
                    if info[2] == 3.0 {
                        if inside {
                            inner
                        } else {
                            outer
                        }
                    } else if mix[3] + shape[0] == 0.0 {
                        f32::from(inside)
                    } else {
                        feather(d, mix[3], shape[0])
                    }
                }
            };
        }
        if mix[1] != 0.0 {
            raw = 1.0 - raw;
        }
        let amount = raw * mix[0];
        mask = if mix[2] == 0.0 {
            (mask + amount).min(1.0)
        } else {
            (mask - amount).max(0.0)
        };
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_tree_prunes_large_outlines_and_agrees_with_an_independent_winding_reference() {
        // A 1024-anchor straight-handle Curve is legal (the Polygon limit is not applied).
        let points: Vec<_> = (0..1024)
            .map(|i| {
                let a = f64::from(i) * std::f64::consts::TAU / 1024.0;
                WindowBezierPoint {
                    position: WindowPoint::new(0.25 * a.cos(), 0.25 * a.sin()),
                    into: WindowPoint::new(0.0, 0.0),
                    out: WindowPoint::new(0.0, 0.0),
                }
            })
            .collect();
        let window = Window::new(
            "many",
            WindowShape::Curve {
                points,
                closed: true,
                inside: 0.0.into(),
                outside: 0.0.into(),
            },
        );
        let mut tail = Vec::new();
        let header = pack(&[window], 0.0, 0, 1, &mut tail);
        let params: Vec<_> = std::iter::once(header).chain(tail).collect();
        let mut visits = 0;
        let mut samples = 0;
        for y in 0..41 {
            for x in 0..41 {
                let p = [
                    (x as f32 + 0.37) / 41.0 - 0.5,
                    (y as f32 + 0.23) / 41.0 - 0.5,
                ];
                let radius = (p[0] * p[0] + p[1] * p[1]).sqrt();
                if (radius - 0.25).abs() < 0.00001 {
                    continue;
                }
                let got = key(
                    &params,
                    0,
                    Position {
                        uv: [p[0] + 0.5, p[1] + 0.5],
                        size: [100.0; 2],
                    },
                );
                assert_eq!(got, f32::from(radius < 0.25));
                // Count the actual packed threaded tree's ray/nearest traversal, not a leaf-count
                // estimate. The circle is an independent membership oracle away from its edge.
                let shape = params[1 + 3];
                let mut node = shape[1] as usize;
                let mut nearest = 0.0_f32;
                while node < shape[2] as usize {
                    visits += 1;
                    let b = params[node];
                    let l = params[node + 1];
                    let dx = (b[0] - p[0]).max(0.0).max(p[0] - b[2]);
                    let dy = (b[1] - p[1]).max(0.0).max(p[1] - b[3]);
                    let ray = p[1] >= b[1] && p[1] < b[3] && p[0] <= b[2];
                    if !ray && dx * dx + dy * dy > nearest * nearest {
                        node = l[0] as usize;
                        continue;
                    }
                    if l[1] != 0.0 {
                        nearest = nearest.min(distance(p, params[node + 2]));
                    }
                    node += 3;
                }
                samples += 1;
            }
        }
        let average = f64::from(visits) / f64::from(samples);
        eprintln!("T23 1024-anchor Curve: {samples} pixels, average {average:.2} BVH nodes visited vs 1024 leaf scans");
        assert!(average < 180.0, "{average}");
    }
}
