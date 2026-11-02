//! Evaluated Window controls of one composed frame. Coordinates are canvas and device pixels,
//! never CSS pixels. The producer chooses the active OSC and supplies no overlay for Split.

use crate::ViewportMap;
use columbina_compose::grade::curve::{self, Anchor, CurveSegment};
use columbina_compose::wipe::Wipe;

mod offset;
pub(crate) mod render;

/// Local to canvas pixels: `x = xx*u + xy*v + tx`, `y = yx*u + yy*v + ty`.
/// Named coefficients deliberately avoid the differing array orders of Window and layers.
/// The canvas and decoded picture are already oriented; apply no metadata rotation here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    pub xx: f64,
    pub xy: f64,
    pub yx: f64,
    pub yy: f64,
    pub tx: f64,
    pub ty: f64,
}

/// A supported shape about the local origin, evaluated at the frame being displayed.
#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub id: String,
    pub radius: f64,
    pub to_canvas: Affine,
    pub kind: ShapeKind,
    /// Half of the feather band in local image-height units.
    pub feather: f64,
    /// Linear/Polygon local outline points, or Curve anchors, in boundary order; empty for the
    /// other shapes.
    pub points: Vec<[f64; 2]>,
    /// Per outline edge `j` (point `j` to the next): the inside and outside feather extents.
    /// A Curve has one entry, its uniform Inside/Outside Softness extents.
    pub feathers: Vec<[f64; 2]>,
    /// A Curve's Bezier handles and whether it is closed; none for the other shapes.
    pub curve: Option<Bezier>,
}

/// The Bezier part of a Curve shape, evaluated at the displayed frame like its anchors.
#[derive(Clone, Debug, PartialEq)]
pub struct Bezier {
    /// Per anchor, its `[in, out]` handles relative to the anchor; zero is a corner side.
    pub handles: Vec<[[f64; 2]; 2]>,
    /// An open Curve is an unfinished drawing: no body, no mask, and while selected every
    /// click that hits nothing else adds a point (RM20 p. 3193).
    pub closed: bool,
    /// The local tolerance the mask flattens this Curve with (`curve::tolerance`): drawing is
    /// never flattened more finely than the mask actually is.
    pub tolerance: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeKind {
    Circular,
    Gradient,
    Linear,
    Polygon,
    Curve,
}

/// Indices are stable local corner/axis/point/edge identifiers, also used to break
/// equal-distance ties. `Insert(j)` is a selected Polygon's or Curve's edge `j` outline, below
/// handles. `HandleIn/HandleOut(i)` are a Curve anchor's Bezier handle ends; `Close` is an open
/// Curve's first anchor once it has two (clicking it closes the shape, p. 3193); `Append` is
/// anywhere else while an open Curve is selected. A Curve's `Softness(-1)`/`Softness(1)` are its
/// magenta inside/outside softness handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Move,
    Size(u8),
    Aspect(u8),
    Rotate,
    Softness(i8),
    Point(u32),
    Edge(u8),
    EdgeSoftness(u8),
    Insert(u32),
    HandleIn(u32),
    HandleOut(u32),
    Close,
    Append,
}

/// Where a Linear side's softness handle sits along its side, as a share from its first point.
/// Away from the white middle side handle, so both stay grabbable at small softness.
pub const EDGE_SOFT_SHARE: f64 = 0.75;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit<'a> {
    pub id: &'a str,
    pub control: Control,
}

pub const CORNERS: [[f64; 2]; 4] = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
pub const AXES: [[f64; 2]; 4] = [[-1.0, 0.0], [1.0, 0.0], [0.0, -1.0], [0.0, 1.0]];
pub const SOFT_NORMAL: [f64; 2] = [0.866_025_403_784_438_6, 0.5];

/// SDR UI colours and stroke width in device pixels, independent of content colour space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub selected: [f32; 3],
    pub unselected: [f32; 3],
    pub width: f32,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            selected: [1.0; 3],
            unselected: [0.5; 3],
            width: 2.0,
        }
    }
}

/// Immutable while shared with a queued/displayed texture. `None` coverage means all current;
/// `Some(wipe)` is the actual comparison of that frame, current A against reference B.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowOverlay {
    pub shapes: Box<[Shape]>,
    pub selected: Option<String>,
    pub style: Style,
    pub coverage: Option<Wipe>,
}

impl WindowOverlay {
    /// Hit at a device pixel centre. Selected wins overlaps, then the last supported shape in the list.
    /// This is a pointer-down query; a captured drag continues across a comparison boundary.
    #[must_use]
    pub fn hit(&self, map: ViewportMap, device: [f32; 2]) -> Option<&str> {
        self.hit_control(map, device).map(|hit| hit.id)
    }

    #[must_use]
    pub fn hit_control(&self, map: ViewportMap, device: [f32; 2]) -> Option<Hit<'_>> {
        if !self.visible(map, device) {
            return None;
        }
        for shape in self.ordered().rev() {
            if !shape.drawable(map) {
                continue;
            }
            // Softness guides are display only: they are not even made for a hit.
            let primitives = shape.layers(
                map,
                self.style,
                self.selected.as_deref() == Some(shape.id.as_str()),
                false,
            );
            // Explicit handles first, then a Polygon/Curve edge insertion, then the body (with a
            // closed Curve's margin just outside its outline).
            for edge in [false, true] {
                let control = primitives
                    .iter()
                    .filter(|p| {
                        p.control.is_some()
                            && p.edge == edge
                            && p.alpha(device, self.style.width) > 0.0
                    })
                    .min_by(|a, b| a.distance(device).total_cmp(&b.distance(device)))
                    .and_then(|p| p.control);
                if let Some(control) = control {
                    return Some(Hit {
                        id: &shape.id,
                        control,
                    });
                }
            }
            if shape.contains(map, device)
                || primitives
                    .iter()
                    .any(|p| !p.guide && p.alpha(device, self.style.width) > 0.0)
                || shape.grips(&primitives, device)
            {
                return Some(Hit {
                    id: &shape.id,
                    control: Control::Move,
                });
            }
            // A selected unfinished Curve takes every other click to draw (RM20 p. 3193).
            if self.selected.as_deref() == Some(shape.id.as_str()) && shape.drafting() {
                return Some(Hit {
                    id: &shape.id,
                    control: Control::Append,
                });
            }
        }
        None
    }

    /// Stroke coverage at a device pixel centre, including its antialiased half-pixel fringe.
    #[must_use]
    pub fn outline(&self, map: ViewportMap, device: [f32; 2]) -> Option<(&str, f32)> {
        if !self.visible(map, device) {
            return None;
        }
        self.ordered().rev().find_map(|shape| {
            self.primitives(shape, map).iter().rev().find_map(|p| {
                let a = p.alpha(device, self.style.width);
                (a > 0.0).then_some((shape.id.as_str(), a))
            })
        })
    }

    /// Only the selected window carries controls; the others are a grey outline whose body
    /// and stroke still select or move them, as before any window had controls.
    pub(crate) fn primitives(&self, shape: &Shape, map: ViewportMap) -> Vec<Primitive> {
        shape.primitives(
            map,
            self.style,
            self.selected.as_deref() == Some(shape.id.as_str()),
        )
    }

    pub(crate) fn ordered(&self) -> impl DoubleEndedIterator<Item = &Shape> {
        self.shapes
            .iter()
            .filter(|s| self.selected.as_deref() != Some(s.id.as_str()))
            .chain(
                self.shapes
                    .iter()
                    .filter(|s| self.selected.as_deref() == Some(s.id.as_str())),
            )
    }

    #[allow(clippy::cast_precision_loss)]
    fn visible(&self, map: ViewportMap, device: [f32; 2]) -> bool {
        let [x, y] = device;
        let [left, top, width, height] = map.dest;
        let [dx, dy] = [x - left, y - top];
        if !(x >= 0.0
            && y >= 0.0
            && x < map.surface.0 as f32
            && y < map.surface.1 as f32
            && dx >= 0.0
            && dy >= 0.0
            && dx < width
            && dy < height)
        {
            return false;
        }
        let ratio = sample_scale(map);
        let canvas = [dx * ratio[0], dy * ratio[1]];
        self.coverage
            .is_none_or(|wipe| wipe.sample_has_current(map.picture, canvas))
    }
}

/// Quantize canvas/device scale once for CPU and WGSL. Computing it in the shader can
/// introduce a positive neighbour weight even at unit zoom (GPU division is approximate).
#[allow(clippy::cast_precision_loss, reason = "bounded picture dimensions")]
pub(crate) fn sample_scale(map: ViewportMap) -> [f32; 2] {
    [
        map.picture.0 as f32 / map.dest[2],
        map.picture.1 as f32 / map.dest[3],
    ]
}

/// The inverse of a unit circle in device pixels, quantized once for CPU input and WGSL.
#[derive(Clone, Copy)]
pub(crate) struct DeviceCircle {
    pub inverse: [f32; 4],
    pub centre: [f32; 2],
    pub scale: f32,
}

impl Shape {
    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn device(&self, map: ViewportMap) -> Option<DeviceCircle> {
        let a = self.to_canvas;
        let sx = f64::from(map.dest[2]) / f64::from(map.picture.0);
        let sy = f64::from(map.dest[3]) / f64::from(map.picture.1);
        let [xx, xy, yx, yy] =
            [a.xx * sx, a.xy * sx, a.yx * sy, a.yy * sy].map(|v| v * self.radius);
        let scale = xx.abs().max(xy.abs()).max(yx.abs()).max(yy.abs());
        let [xx, xy, yx, yy] = [xx, xy, yx, yy].map(|v| v / scale);
        let determinant = xx * yy - xy * yx;
        if self.radius <= 0.0 || !determinant.is_finite() || determinant == 0.0 {
            return None;
        }
        let inverse = [yy, -xy, -yx, xx].map(|v| (v / determinant) as f32);
        let centre = [
            (f64::from(map.dest[0]) + a.tx * sx) as f32,
            (f64::from(map.dest[1]) + a.ty * sy) as f32,
        ];
        // A singular/nonfinite inverse cannot form a drawable stroke or a valid hit region.
        let scale = (scale as f32).max(f32::MIN_POSITIVE);
        (scale.is_finite() && inverse.iter().chain(&centre).all(|v| v.is_finite())).then_some(
            DeviceCircle {
                inverse,
                centre,
                scale,
            },
        )
    }
}

/// The same quantized primitive is consumed by pointer hit and the GPU. Kind: ellipse,
/// infinite line, filled device disk, or device segment. Every fragment uses A coverage.
pub(crate) struct Primitive {
    pub geometry: DeviceCircle,
    pub kind: f32,
    pub colour: [f32; 3],
    control: Option<Control>,
    anchor: [f32; 2],
    /// An outline edge control, ranked below every handle.
    edge: bool,
    /// A softness guide: display only, never hit.
    pub(crate) guide: bool,
}
impl Primitive {
    /// Handles compare their centres; edges compare the device distance to the segment.
    fn distance(&self, p: [f32; 2]) -> f32 {
        if self.edge {
            return self.along(p);
        }
        (p[0] - self.anchor[0]).hypot(p[1] - self.anchor[1])
    }
    /// The device distance to a segment primitive's centre line.
    fn along(&self, p: [f32; 2]) -> f32 {
        let segment = self.geometry;
        let [x, y] = [p[0] - segment.centre[0], p[1] - segment.centre[1]];
        let [dx, dy, _, _] = segment.inverse;
        let share = ((x * dx + y * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        (x - share * dx).hypot(y - share * dy)
    }
    pub(crate) fn alpha(&self, p: [f32; 2], width: f32) -> f32 {
        let geometry = self.geometry;
        let [x, y] = [p[0] - geometry.centre[0], p[1] - geometry.centre[1]];
        let distance = if self.kind < 0.5 {
            return geometry.alpha(p, width);
        } else if self.kind < 1.5 {
            (x * geometry.inverse[0] + y * geometry.inverse[1]).abs()
        } else if self.kind < 2.5 {
            return (geometry.scale + 0.5 - x.hypot(y)).clamp(0.0, 1.0);
        } else {
            let [dx, dy, _, _] = geometry.inverse;
            let t = ((x * dx + y * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
            (x - t * dx).hypot(y - t * dy)
        };
        (width * 0.5 + 0.5 - distance).clamp(0.0, 1.0)
    }

    fn into_guide(self) -> Self {
        Self {
            guide: true,
            ..self
        }
    }

    /// The device quad `[origin, first axis, second axis]` the GPU rasterizes for this
    /// primitive. It holds every pixel centre where `alpha` is positive, with a pixel to
    /// spare, so fragment work follows the covered pixels; none when nothing can be covered.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::many_single_char_names,
        reason = "bounded device quads in plane geometry notation"
    )]
    pub(crate) fn quad(&self, map: ViewportMap, width: f32) -> Option<[f32; 6]> {
        let g = self.geometry;
        let c = g.centre.map(f64::from);
        let reach = f64::from(width) * 0.5 + 0.5;
        // The drawn picture on the surface; the fragment discards every other pixel.
        let [left, top, w, h] = map.dest.map(f64::from);
        let clip = [
            left.max(0.0) - 1.0,
            top.max(0.0) - 1.0,
            (left + w).min(f64::from(map.surface.0)) + 1.0,
            (top + h).min(f64::from(map.surface.1)) + 1.0,
        ];
        let boxed = |b: [f64; 4]| {
            let b = if b.iter().all(|v| v.is_finite()) {
                b
            } else {
                clip
            };
            let b = [
                b[0].max(clip[0]),
                b[1].max(clip[1]),
                b[2].min(clip[2]),
                b[3].min(clip[3]),
            ];
            (b[0] < b[2] && b[1] < b[3]).then_some([b[0], b[1], b[2] - b[0], 0.0, 0.0, b[3] - b[1]])
        };
        let quad = if self.kind < 0.5 {
            // The estimate is |(|Kx| - S)| / |K^T u| with |K^T u| <= |K|_F, so a covered pixel
            // has |Kx| < S + reach |K|_F: an ellipse whose box is that bound times K^-1's rows.
            let k = g.inverse.map(f64::from);
            let det = (k[0] * k[3] - k[1] * k[2]).abs();
            let t = f64::from(g.scale) + reach * k.iter().map(|v| v * v).sum::<f64>().sqrt();
            let half = [t * k[3].hypot(k[1]) / det, t * k[2].hypot(k[0]) / det];
            boxed([
                c[0] - half[0] - 1.0,
                c[1] - half[1] - 1.0,
                c[0] + half[0] + 1.0,
                c[1] + half[1] + 1.0,
            ])?
        } else if self.kind < 1.5 {
            boxed(clip)?
        } else if self.kind < 2.5 {
            let e = f64::from(g.scale) + 1.5;
            boxed([c[0] - e, c[1] - e, c[0] + e, c[1] + e])?
        } else {
            // A segment's stadium within its rectangle grown by the reach.
            let e = reach + 1.0;
            let d = [f64::from(g.inverse[0]), f64::from(g.inverse[1])];
            let length = d[0].hypot(d[1]);
            let u = if length > 0.0 {
                [d[0] / length, d[1] / length]
            } else {
                [1.0, 0.0]
            };
            let n = [-u[1], u[0]];
            [
                c[0] - e * (u[0] + n[0]),
                c[1] - e * (u[1] + n[1]),
                d[0] + 2.0 * e * u[0],
                d[1] + 2.0 * e * u[1],
                2.0 * e * n[0],
                2.0 * e * n[1],
            ]
        };
        let quad = quad.map(|v| v as f32);
        quad.iter().all(|v| v.is_finite()).then_some(quad)
    }
}

impl Shape {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "quantize once for CPU and GPU device geometry"
    )]
    fn point(&self, map: ViewportMap, local: [f64; 2]) -> [f32; 2] {
        let a = self.to_canvas;
        [
            (f64::from(map.dest[0])
                + (a.xx * local[0] + a.xy * local[1] + a.tx) * f64::from(map.dest[2])
                    / f64::from(map.picture.0)) as f32,
            (f64::from(map.dest[1])
                + (a.yx * local[0] + a.yy * local[1] + a.ty) * f64::from(map.dest[3])
                    / f64::from(map.picture.1)) as f32,
        ]
    }

    /// The main boundary, and on the selected window its two softness rings when `guides`:
    /// like Linear/Polygon guides those are display only and never hit (author decision 2026-11-02).
    fn boundaries(
        &self,
        map: ViewportMap,
        style: Style,
        selected: bool,
        guides: bool,
    ) -> Vec<Primitive> {
        let Some(body) = self.device(map) else {
            return vec![];
        };
        let mut out = vec![];
        let r = self.radius;
        let f = self.feather;
        let mut boundary = |offset: f64, colour, guide: bool| {
            let geometry = if self.kind == ShapeKind::Circular {
                let circle = Self {
                    radius: offset,
                    ..self.clone()
                };
                let Some(circle) = circle.device(map) else {
                    return;
                };
                circle
            } else {
                let [_, _, x, y] = body.inverse;
                let norm = x.hypot(y);
                DeviceCircle {
                    inverse: [x / norm, y / norm, 0.0, 0.0],
                    centre: self.point(map, [0.0, offset]),
                    scale: 1.0,
                }
            };
            out.push(Primitive {
                geometry,
                kind: if self.kind == ShapeKind::Circular {
                    0.0
                } else {
                    1.0
                },
                colour,
                control: None,
                anchor: geometry.centre,
                edge: false,
                guide,
            });
        };
        if f > 0.0 && selected && guides {
            let base = if self.kind == ShapeKind::Circular {
                r
            } else {
                0.0
            };
            boundary(base - f, style.unselected, true);
            boundary(base + f, style.unselected, true);
        }
        boundary(
            if self.kind == ShapeKind::Circular {
                r
            } else {
                0.0
            },
            if selected {
                style.selected
            } else {
                style.unselected
            },
            false,
        );
        out
    }

    pub(crate) fn primitives(
        &self,
        map: ViewportMap,
        style: Style,
        selected: bool,
    ) -> Vec<Primitive> {
        self.layers(map, style, selected, true)
    }

    /// Everything drawn, or with `guides` false only what can be hit: softness guides and
    /// rings of every shape are display only.
    fn layers(
        &self,
        map: ViewportMap,
        style: Style,
        selected: bool,
        guides: bool,
    ) -> Vec<Primitive> {
        if matches!(self.kind, ShapeKind::Linear | ShapeKind::Polygon) {
            return self.path_primitives(map, style, selected, guides);
        }
        if self.kind == ShapeKind::Curve {
            return self.curve_primitives(map, style, selected, guides);
        }
        if self.device(map).is_none() {
            return vec![];
        }
        let mut out = self.boundaries(map, style, selected, guides);
        if !selected {
            return out;
        }
        let r = self.radius;
        let f = self.feather;
        let mut handle = |point, control, colour| {
            let centre = self.point(map, point);
            out.push(Primitive {
                geometry: DeviceCircle {
                    inverse: [0.0; 4],
                    centre,
                    scale: 4.0,
                },
                kind: 2.0,
                colour,
                control: Some(control),
                anchor: centre,
                edge: false,
                guide: false,
            });
        };
        if self.kind == ShapeKind::Circular {
            for (i, corner) in CORNERS.iter().enumerate() {
                #[allow(clippy::cast_possible_truncation)]
                handle(
                    corner.map(|x| x * r),
                    Control::Size(i as u8),
                    [0.15, 0.45, 1.0],
                );
            }
            for (i, axis) in AXES.iter().enumerate() {
                #[allow(clippy::cast_possible_truncation)]
                handle(
                    axis.map(|x| x * r),
                    Control::Aspect(i as u8),
                    [0.15, 0.45, 1.0],
                );
            }
            handle([0.0, -0.75 * r], Control::Rotate, style.selected);
            handle(
                SOFT_NORMAL.map(|x| x * (r + f)),
                Control::Softness(1),
                [1.0, 0.1, 0.8],
            );
        } else {
            handle([0.0, 0.0], Control::Move, style.selected);
            handle([-r, -f], Control::Softness(-1), [1.0, 0.1, 0.8]);
            handle([r, f], Control::Softness(1), [1.0, 0.1, 0.8]);
            let centre = self.point(map, [0.0, r]);
            // Direction uses the affine column, avoiding subtraction of huge device centres.
            let x = self.to_canvas.xy * f64::from(map.dest[2]) / f64::from(map.picture.0);
            let y = self.to_canvas.yy * f64::from(map.dest[3]) / f64::from(map.picture.1);
            #[allow(clippy::cast_possible_truncation)]
            let [dx, dy] = [(x / x.hypot(y)) as f32, (y / x.hypot(y)) as f32];
            for side in [-1.0, 1.0] {
                out.push(Primitive {
                    geometry: DeviceCircle {
                        inverse: [
                            -6.0 * dx - side * 4.0 * dy,
                            -6.0 * dy + side * 4.0 * dx,
                            0.0,
                            0.0,
                        ],
                        centre,
                        scale: 1.0,
                    },
                    kind: 3.0,
                    colour: style.selected,
                    control: Some(Control::Rotate),
                    anchor: centre,
                    edge: false,
                    guide: false,
                });
            }
        }
        out
    }
}

/// Device pixels kept around the surface when clipping outline segments and handles. Clipping
/// in f64 first keeps every quantized coordinate near the surface at any legal zoom.
const MARGIN: f64 = 16.0;
const SOFT_COLOUR: [f32; 3] = [1.0, 0.1, 0.8];
/// Device pixels a guide arc's chords may depart from the arc.
const ARC_TOLERANCE: f64 = 1.0 / 16.0;

impl Shape {
    /// Inside test shared by pointer down for every shape. Linear and Polygon use even-odd on
    /// the same device points as their drawn outline, without reordering self-intersections.
    pub(crate) fn contains(&self, map: ViewportMap, p: [f32; 2]) -> bool {
        match self.kind {
            ShapeKind::Circular => self.device(map).is_some_and(|body| body.contains(p)),
            ShapeKind::Gradient => self
                .device(map)
                .is_some_and(|body| body.local(p).0[1] <= 0.0),
            ShapeKind::Curve => {
                let closed = self.curve.as_ref().is_some_and(|c| c.closed);
                if !closed || !self.invertible(map) {
                    return false;
                }
                let ring: Vec<_> = self
                    .curve_segments(map)
                    .iter()
                    .map(|s| self.at(map, s.a))
                    .collect();
                offset::inside(&ring, p.map(f64::from))
            }
            ShapeKind::Linear | ShapeKind::Polygon => {
                let [x, y] = p.map(f64::from);
                let device: Vec<_> = self.points.iter().map(|&q| self.at(map, q)).collect();
                let mut inside = false;
                for (j, a) in device.iter().enumerate() {
                    let b = device[(j + 1) % device.len()];
                    if (a[1] > y) != (b[1] > y)
                        && x < (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]) + a[0]
                    {
                        inside = !inside;
                    }
                }
                inside
            }
        }
    }

    /// Whether the shape has invertible device geometry, even if all its strokes are clipped.
    fn drawable(&self, map: ViewportMap) -> bool {
        match self.kind {
            ShapeKind::Circular | ShapeKind::Gradient => self.device(map).is_some(),
            ShapeKind::Linear | ShapeKind::Polygon => {
                self.points.len() >= 2 && self.invertible(map)
            }
            // Even an empty draft is drawable: it takes clicks to draw while selected.
            ShapeKind::Curve => self.curve.is_some() && self.invertible(map),
        }
    }

    /// An unfinished Curve, which draws on every click that hits nothing else while selected.
    fn drafting(&self) -> bool {
        self.kind == ShapeKind::Curve && self.curve.as_ref().is_some_and(|c| !c.closed)
    }

    /// A local point in f64 device pixels.
    fn at(&self, map: ViewportMap, local: [f64; 2]) -> [f64; 2] {
        let a = self.to_canvas;
        [
            f64::from(map.dest[0])
                + (a.xx * local[0] + a.xy * local[1] + a.tx) * f64::from(map.dest[2])
                    / f64::from(map.picture.0),
            f64::from(map.dest[1])
                + (a.yx * local[0] + a.yy * local[1] + a.ty) * f64::from(map.dest[3])
                    / f64::from(map.picture.1),
        ]
    }

    /// Size 0 or another singular placement has no drawable outline and no controls.
    fn invertible(&self, map: ViewportMap) -> bool {
        let a = self.to_canvas;
        let sx = f64::from(map.dest[2]) / f64::from(map.picture.0);
        let sy = f64::from(map.dest[3]) / f64::from(map.picture.1);
        let m = [a.xx * sx, a.xy * sx, a.yx * sy, a.yy * sy];
        let scale = m.iter().fold(0.0_f64, |s, v| s.max(v.abs()));
        let [xx, xy, yx, yy] = m.map(|v| v / scale);
        let determinant = xx * yy - xy * yx;
        scale > 0.0 && scale.is_finite() && determinant.is_finite() && determinant != 0.0
    }

    /// The local unit normal of each edge towards the inside of the outline's winding.
    fn inward(&self, edge: usize) -> Option<[f64; 2]> {
        let count = self.points.len();
        let twice_area: f64 = (0..count)
            .map(|i| {
                let (from, to) = (self.points[i], self.points[(i + 1) % count]);
                from[0] * to[1] - to[0] * from[1]
            })
            .sum();
        let sign = if twice_area < 0.0 { -1.0 } else { 1.0 };
        let (from, to) = (self.points[edge], self.points[(edge + 1) % count]);
        let along = [to[0] - from[0], to[1] - from[1]];
        let length = along[0].hypot(along[1]);
        (length > 0.0 && length.is_finite())
            .then(|| [-sign * along[1] / length, sign * along[0] / length])
    }

    /// A device segment clipped to the surface and its margin, or none when nothing is left.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "clipped device coordinates are quantized once for CPU and GPU"
    )]
    fn segment(
        map: ViewportMap,
        a: [f64; 2],
        b: [f64; 2],
        colour: [f32; 3],
        control: Option<Control>,
    ) -> Option<Primitive> {
        let d = [b[0] - a[0], b[1] - a[1]];
        let (mut low, mut high) = (0.0_f64, 1.0_f64);
        let limits = [f64::from(map.surface.0), f64::from(map.surface.1)];
        for axis in 0..2 {
            for (p, q) in [
                (-d[axis], a[axis] + MARGIN),
                (d[axis], limits[axis] + MARGIN - a[axis]),
            ] {
                if p == 0.0 {
                    if q < 0.0 {
                        return None;
                    }
                } else if p < 0.0 {
                    low = low.max(q / p);
                } else {
                    high = high.min(q / p);
                }
            }
        }
        if low >= high || low.is_nan() || high.is_nan() {
            return None;
        }
        let start = [a[0] + low * d[0], a[1] + low * d[1]];
        let vector = [(high - low) * d[0], (high - low) * d[1]];
        if !vector[0].hypot(vector[1]).is_finite() || vector[0].hypot(vector[1]) < 1e-3 {
            return None;
        }
        let centre = start.map(|v| v as f32);
        Some(Primitive {
            geometry: DeviceCircle {
                inverse: [vector[0] as f32, vector[1] as f32, 0.0, 0.0],
                centre,
                scale: 1.0,
            },
            kind: 3.0,
            colour,
            control,
            anchor: centre,
            edge: control.is_some(),
            guide: false,
        })
    }

    #[allow(
        clippy::cast_possible_truncation,
        reason = "device coordinates near the surface are quantized once for CPU and GPU"
    )]
    fn disk(
        map: ViewportMap,
        at: [f64; 2],
        colour: [f32; 3],
        control: Control,
    ) -> Option<Primitive> {
        let inside = (-MARGIN..f64::from(map.surface.0) + MARGIN).contains(&at[0])
            && (-MARGIN..f64::from(map.surface.1) + MARGIN).contains(&at[1]);
        let centre = at.map(|v| v as f32);
        inside.then_some(Primitive {
            geometry: DeviceCircle {
                inverse: [0.0; 4],
                centre,
                scale: 4.0,
            },
            kind: 2.0,
            colour,
            control: Some(control),
            anchor: centre,
            edge: false,
            guide: false,
        })
    }

    /// A local circular arc as device chords: the image of a local circle is an ellipse
    /// `C + M (cos t, sin t)`. Only its part within the surface and margin is split, into
    /// chords at most `ARC_TOLERANCE` device pixels from it, each clipped like the outline.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::many_single_char_names,
        reason = "a bounded positive chord count; ellipse notation"
    )]
    fn arc(
        &self,
        map: ViewportMap,
        (centre, radius): ([f64; 2], f64),
        [from, to]: [f64; 2],
        colour: [f32; 3],
        out: &mut Vec<Primitive>,
    ) {
        let a = self.to_canvas;
        let sx = f64::from(map.dest[2]) / f64::from(map.picture.0);
        let sy = f64::from(map.dest[3]) / f64::from(map.picture.1);
        let m = [a.xx * sx, a.xy * sx, a.yx * sy, a.yy * sy].map(|v| v * radius);
        let c = self.at(map, centre);
        if !m.iter().chain(&c).all(|v| v.is_finite()) {
            return;
        }
        let at = |t: f64| {
            let (sin, cos) = t.sin_cos();
            [
                c[0] + m[0] * cos + m[1] * sin,
                c[1] + m[2] * cos + m[3] * sin,
            ]
        };
        let low = [-MARGIN, -MARGIN];
        let high = [
            f64::from(map.surface.0) + MARGIN,
            f64::from(map.surface.1) + MARGIN,
        ];
        // Where the ellipse crosses each side of the margin box: `A cos(t - phase) = k`.
        let mut cuts = vec![from, to];
        for axis in 0..2 {
            let (p, q) = (m[2 * axis], m[2 * axis + 1]);
            let amplitude = p.hypot(q);
            if amplitude == 0.0 {
                continue;
            }
            let phase = q.atan2(p);
            for line in [low[axis], high[axis]] {
                let k = (line - c[axis]) / amplitude;
                if k.abs() <= 1.0 {
                    for t in [phase + k.acos(), phase - k.acos()] {
                        let t = from + (t - from).rem_euclid(std::f64::consts::TAU);
                        if t < to {
                            cuts.push(t);
                        }
                    }
                }
            }
        }
        cuts.sort_by(f64::total_cmp);
        // A chord over parameter step h is within h^2/8 |M| of the ellipse.
        let stretch = m.iter().map(|v| v * v).sum::<f64>().sqrt();
        let step = (8.0 * ARC_TOLERANCE / stretch).sqrt();
        for pair in cuts.windows(2) {
            let (t0, t1) = (pair[0], pair[1]);
            let middle = at(t0.midpoint(t1));
            if t1 <= t0 || (0..2).any(|i| !(low[i]..=high[i]).contains(&middle[i])) {
                continue;
            }
            let count = ((t1 - t0) / step).ceil().clamp(1.0, 4096.0) as u32;
            for i in 0..count {
                let s = |i: u32| t0 + (t1 - t0) * f64::from(i) / f64::from(count);
                out.extend(
                    Self::segment(map, at(s(i)), at(s(i + 1)), colour, None)
                        .map(Primitive::into_guide),
                );
            }
        }
    }

    /// Linear and Polygon controls (RM20 p. 3193): white corner/point handles, Linear white side
    /// handles and one magenta softness handle per side; a Polygon has no softness handles.
    /// The selected window's grey guides are exactly where its mask becomes opaque inside and
    /// zero outside (`offset`), drawn only when `guides`: they are never hit.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "at most 128 points; four Linear sides"
    )]
    fn path_primitives(
        &self,
        map: ViewportMap,
        style: Style,
        selected: bool,
        guides: bool,
    ) -> Vec<Primitive> {
        let n = self.points.len();
        if !self.drawable(map) {
            return vec![];
        }
        let mut out = vec![];
        if selected && guides {
            for (side, inside) in [(0, true), (1, false)] {
                let radii: Vec<f64> = (0..n)
                    .map(|j| self.feathers.get(j).map_or(0.0, |f| f[side]))
                    .collect();
                self.guides(map, &self.points, &radii, inside, style, &mut out);
            }
        }
        let colour = if selected {
            style.selected
        } else {
            style.unselected
        };
        for j in 0..n {
            let insert =
                (selected && self.kind == ShapeKind::Polygon).then_some(Control::Insert(j as u32));
            out.extend(Self::segment(
                map,
                self.at(map, self.points[j]),
                self.at(map, self.points[(j + 1) % n]),
                colour,
                insert,
            ));
        }
        if !selected {
            return out;
        }
        for (i, &point) in self.points.iter().enumerate() {
            out.extend(Self::disk(
                map,
                self.at(map, point),
                style.selected,
                Control::Point(i as u32),
            ));
        }
        if self.kind == ShapeKind::Linear {
            for j in 0..n {
                let (a, b) = (self.points[j], self.points[(j + 1) % n]);
                let middle = [a[0].midpoint(b[0]), a[1].midpoint(b[1])];
                out.extend(Self::disk(
                    map,
                    self.at(map, middle),
                    style.selected,
                    Control::Edge(j as u8),
                ));
                let outer = self.feathers.get(j).map_or(0.0, |f| f[1]);
                let normal = self.inward(j).unwrap_or_default();
                let along = [
                    a[0] + (b[0] - a[0]) * EDGE_SOFT_SHARE - normal[0] * outer,
                    a[1] + (b[1] - a[1]) * EDGE_SOFT_SHARE - normal[1] * outer,
                ];
                out.extend(Self::disk(
                    map,
                    self.at(map, along),
                    SOFT_COLOUR,
                    Control::EdgeSoftness(j as u8),
                ));
            }
        }
        out
    }
}

impl Shape {
    /// The exact softness guide of one side of a closed outline, as display-only primitives.
    fn guides(
        &self,
        map: ViewportMap,
        outline: &[[f64; 2]],
        radii: &[f64],
        inside: bool,
        style: Style,
        out: &mut Vec<Primitive>,
    ) {
        for piece in offset::guide(outline, radii, inside) {
            match piece {
                offset::Piece::Line(a, b) => out.extend(
                    Self::segment(
                        map,
                        self.at(map, a),
                        self.at(map, b),
                        style.unselected,
                        None,
                    )
                    .map(Primitive::into_guide),
                ),
                offset::Piece::Arc {
                    centre,
                    radius,
                    start,
                    sweep,
                } => self.arc(
                    map,
                    (centre, radius),
                    [start, start + sweep],
                    style.unselected,
                    out,
                ),
            }
        }
    }
}

/// Device pixels a drawn Curve piece may depart from its cubic, unless the mask's own
/// tessellation is coarser there (extreme zoom), which is then drawn exactly as it masks.
const CURVE_DISPLAY: f64 = 0.25;
/// Cubic parameters on edge 0 of a closed Curve where its inside and outside softness handles
/// stand, apart so that both stay grabbable at zero softness.
const CURVE_SOFT_AT: [f64; 2] = [1.0 / 3.0, 2.0 / 3.0];
/// Device radius of a Bezier handle end, smaller than an anchor's.
const HANDLE_RADIUS: f32 = 3.0;
/// Device pixels from a closed Curve's drawn outline within which a down still grips its body:
/// RM20 p. 3194, "drag anywhere within or just outside a curve to move it". An anchor's radius
/// plus the stroke's half-pixel fringe. A project choice; a draft has no body, so no margin.
const CURVE_GRIP: f32 = 4.5;

/// A cubic's point and derivative direction at `t` (Bernstein form).
fn cubic_at(c: [[f64; 2]; 4], t: f64) -> ([f64; 2], [f64; 2]) {
    let u = 1.0 - t;
    let at = [0, 1].map(|i| {
        u * u * u * c[0][i]
            + 3.0 * u * u * t * c[1][i]
            + 3.0 * u * t * t * c[2][i]
            + t * t * t * c[3][i]
    });
    let along = [0, 1].map(|i| {
        u * u * (c[1][i] - c[0][i])
            + 2.0 * u * t * (c[2][i] - c[1][i])
            + t * t * (c[3][i] - c[2][i])
    });
    (at, along)
}

impl Shape {
    fn anchors(&self) -> Vec<Anchor> {
        let Some(bezier) = &self.curve else {
            return vec![];
        };
        self.points
            .iter()
            .zip(&bezier.handles)
            .map(|(&position, &[into, out])| Anchor {
                position,
                into,
                out,
            })
            .collect()
    }

    /// Whether a closed Curve's body grips a down just outside it: within `CURVE_GRIP` of its
    /// drawn outline segments (`primitives` as made for a hit; handle lines are guides). Ranked
    /// by the caller below every control and above the windows underneath.
    fn grips(&self, primitives: &[Primitive], p: [f32; 2]) -> bool {
        self.kind == ShapeKind::Curve
            && self.curve.as_ref().is_some_and(|c| c.closed)
            && primitives
                .iter()
                .any(|s| s.kind > 2.5 && !s.guide && s.along(p) <= CURVE_GRIP)
    }

    /// The largest local-to-device column scale.
    fn device_scale(&self, map: ViewportMap) -> f64 {
        let a = self.to_canvas;
        let sx = f64::from(map.dest[2]) / f64::from(map.picture.0);
        let sy = f64::from(map.dest[3]) / f64::from(map.picture.1);
        (a.xx * sx)
            .hypot(a.yx * sy)
            .max((a.xy * sx).hypot(a.yy * sy))
    }

    /// The drawn and hit pieces of a Curve in local units: the shared mask flattening
    /// (`curve::segments`), at a quarter device pixel but never finer than the mask's own
    /// tolerance. An open draft has no closing edge.
    pub(crate) fn curve_segments(&self, map: ViewportMap) -> Vec<CurveSegment> {
        let Some(bezier) = &self.curve else {
            return vec![];
        };
        let scale = self.device_scale(map);
        let tolerance = if scale > 0.0 && scale.is_finite() {
            (CURVE_DISPLAY / scale).max(bezier.tolerance)
        } else {
            bezier.tolerance
        };
        curve::segments(&self.anchors(), bezier.closed, tolerance)
    }

    /// The exact cubic parameter on Curve edge `edge` nearest a canvas point, from the mask's
    /// own flattening, kept off the two anchors. None for another shape or a missing edge.
    #[must_use]
    pub fn curve_parameter(&self, edge: usize, canvas: [f64; 2]) -> Option<f64> {
        let bezier = self.curve.as_ref()?;
        let anchors = self.anchors();
        let edges = if bezier.closed {
            anchors.len()
        } else {
            anchors.len().saturating_sub(1)
        };
        if edge >= edges || bezier.tolerance <= 0.0 || bezier.tolerance.is_nan() {
            return None;
        }
        let cubic = curve::cubic(&anchors, edge);
        let mut pieces = vec![];
        curve::flatten(cubic, bezier.tolerance, edge, &mut pieces);
        let a = self.to_canvas;
        let place = |p: [f64; 2]| {
            [
                a.xx * p[0] + a.xy * p[1] + a.tx,
                a.yx * p[0] + a.yy * p[1] + a.ty,
            ]
        };
        let away = |t: f64| {
            let p = place(cubic_at(cubic, t).0);
            (canvas[0] - p[0]).hypot(canvas[1] - p[1])
        };
        // The nearest flattened piece, then the nearest parameter of the cubic itself within
        // it: a piece is straight to the tolerance, but its parameter need not be uniform.
        let mut best: Option<(f64, [f64; 2])> = None;
        for piece in pieces {
            let (p, q) = (place(piece.a), place(piece.b));
            let d = [q[0] - p[0], q[1] - p[1]];
            let length = d[0] * d[0] + d[1] * d[1];
            let share = if length > 0.0 {
                (((canvas[0] - p[0]) * d[0] + (canvas[1] - p[1]) * d[1]) / length).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let distance = (canvas[0] - p[0] - share * d[0]).hypot(canvas[1] - p[1] - share * d[1]);
            if best.is_none_or(|(nearest, _)| distance < nearest) {
                best = Some((distance, piece.t));
            }
        }
        let [low, high] = best?.1;
        let samples = 64;
        let step = (high - low) / f64::from(samples);
        let mut t = (0..=samples)
            .map(|k| low + step * f64::from(k))
            .min_by(|x, y| away(*x).total_cmp(&away(*y)))?;
        let (mut from, mut to) = ((t - step).max(low), (t + step).min(high));
        for _ in 0..60 {
            let (left, right) = (from + (to - from) / 3.0, to - (to - from) / 3.0);
            if away(left) <= away(right) {
                to = right;
            } else {
                from = left;
            }
        }
        t = f64::midpoint(from, to);
        Some(t.clamp(1e-3, 1.0 - 1e-3)).filter(|t| t.is_finite())
    }

    /// A closed Curve's magenta softness handles (RM20 p. 3193): `Softness(-1)` on its inside
    /// guide and `Softness(1)` on its outside guide, on edge 0 at `CURVE_SOFT_AT`, each with the
    /// local unit normal along which dragging increases that softness. The model keeps one
    /// Inside and one Outside Softness per window, so there is one handle of each.
    #[must_use]
    pub fn curve_softness_handles(&self) -> Vec<(Control, [f64; 2], [f64; 2])> {
        let Some(bezier) = self.curve.as_ref().filter(|c| c.closed) else {
            return vec![];
        };
        let anchors = self.anchors();
        if anchors.len() < 2 {
            return vec![];
        }
        // The inside is the even-odd inside of the mask's flattening, read just off the edge on
        // each side: a signed area cannot tell the lobes of a self-intersecting figure apart.
        let tolerance = bezier.tolerance.max(1e-4);
        let ring: Vec<[f64; 2]> = curve::segments(&anchors, true, tolerance)
            .iter()
            .map(|s| s.a)
            .collect();
        let twice_area: f64 = (0..ring.len())
            .map(|i| {
                let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
                a[0] * b[1] - b[0] * a[1]
            })
            .sum();
        let step = 4.0 * tolerance;
        let c = curve::cubic(&anchors, 0);
        let [inside, outside] = self.feathers.first().copied().unwrap_or([0.0; 2]);
        let mut out = vec![];
        for (control, t, extent, towards) in [
            (Control::Softness(-1), CURVE_SOFT_AT[0], inside, 1.0),
            (Control::Softness(1), CURVE_SOFT_AT[1], outside, -1.0),
        ] {
            let (at, mut along) = cubic_at(c, t);
            if along[0].hypot(along[1]) <= 1e-12 {
                along = [c[3][0] - c[0][0], c[3][1] - c[0][1]];
            }
            let length = along[0].hypot(along[1]);
            if !(length > 1e-12 && length.is_finite()) {
                continue;
            }
            let left = [-along[1] / length, along[0] / length];
            let off = |side: f64| {
                offset::inside(
                    &ring,
                    [at[0] + side * step * left[0], at[1] + side * step * left[1]],
                )
            };
            // Where both sides read alike (a crossing, or a sliver thinner than the step), the
            // winding of the whole outline decides.
            let inward = match (off(1.0), off(-1.0)) {
                (true, false) => 1.0,
                (false, true) => -1.0,
                _ if twice_area < 0.0 => -1.0,
                _ => 1.0,
            };
            let normal = [towards * inward * left[0], towards * inward * left[1]];
            out.push((
                control,
                [at[0] + normal[0] * extent, at[1] + normal[1] * extent],
                normal,
            ));
        }
        out
    }

    /// Curve controls (RM20 pp. 3193–3194): the outline (its edges insert when selected), white
    /// anchors, each non-zero Bezier handle as a display-only line to a smaller white end, and
    /// on a closed Curve the exact inside/outside softness guides and their magenta handles.
    /// Unselected, only the grey outline is drawn and hit (it moves or selects the window).
    #[allow(
        clippy::cast_possible_truncation,
        reason = "anchor and edge indices of one Curve fit u32"
    )]
    fn curve_primitives(
        &self,
        map: ViewportMap,
        style: Style,
        selected: bool,
        guides: bool,
    ) -> Vec<Primitive> {
        let Some(bezier) = self.curve.as_ref().filter(|_| self.drawable(map)) else {
            return vec![];
        };
        let segments = self.curve_segments(map);
        let mut out = vec![];
        if selected && guides && bezier.closed && self.points.len() >= 2 {
            let ring: Vec<[f64; 2]> = segments.iter().map(|s| s.a).collect();
            let [inside, outside] = self.feathers.first().copied().unwrap_or([0.0; 2]);
            for (extent, side) in [(inside, true), (outside, false)] {
                self.guides(map, &ring, &vec![extent; ring.len()], side, style, &mut out);
            }
        }
        let colour = if selected {
            style.selected
        } else {
            style.unselected
        };
        for s in &segments {
            out.extend(Self::segment(
                map,
                self.at(map, s.a),
                self.at(map, s.b),
                colour,
                selected.then_some(Control::Insert(s.edge as u32)),
            ));
        }
        if !selected {
            return out;
        }
        for (i, (&point, &[into, outward])) in self.points.iter().zip(&bezier.handles).enumerate() {
            for (handle, control) in [
                (into, Control::HandleIn(i as u32)),
                (outward, Control::HandleOut(i as u32)),
            ] {
                if handle == [0.0, 0.0] {
                    continue;
                }
                let end = [point[0] + handle[0], point[1] + handle[1]];
                out.extend(
                    Self::segment(map, self.at(map, point), self.at(map, end), colour, None)
                        .map(Primitive::into_guide),
                );
                out.extend(
                    Self::disk(map, self.at(map, end), style.selected, control).map(|mut p| {
                        p.geometry.scale = HANDLE_RADIUS;
                        p
                    }),
                );
            }
        }
        let closes = !bezier.closed && self.points.len() >= 2;
        for (i, &point) in self.points.iter().enumerate() {
            let control = if closes && i == 0 {
                Control::Close
            } else {
                Control::Point(i as u32)
            };
            out.extend(Self::disk(
                map,
                self.at(map, point),
                style.selected,
                control,
            ));
        }
        for (control, at, _) in self.curve_softness_handles() {
            out.extend(Self::disk(map, self.at(map, at), SOFT_COLOUR, control));
        }
        out
    }
}

impl DeviceCircle {
    fn local(self, p: [f32; 2]) -> ([f32; 2], f32) {
        let [x, y] = [p[0] - self.centre[0], p[1] - self.centre[1]];
        // Normalize before any inverse or squared length. Legal viewport zooms can put
        // a circle far below a pixel or far beyond the surface without overflowing WGSL.
        let magnitude = self.scale.max(x.abs()).max(y.abs());
        let [x, y] = [x / magnitude, y / magnitude];
        let [xx, xy, yx, yy] = self.inverse;
        ([xx * x + xy * y, yx * x + yy * y], magnitude)
    }
    fn contains(self, p: [f32; 2]) -> bool {
        let ([x, y], magnitude) = self.local(p);
        (x * x + y * y).sqrt() <= self.scale / magnitude
    }
    /// The normal-distance estimate of the implicit ellipse, shared with the shader. One
    /// half-pixel fringe smooths the stroke; hit tolerance includes exactly that visible fringe.
    fn alpha(self, p: [f32; 2], width: f32) -> f32 {
        let ([x, y], magnitude) = self.local(p);
        let radius = (x * x + y * y).sqrt();
        if radius == 0.0 || width <= 0.0 {
            return 0.0;
        }
        let [xx, xy, yx, yy] = self.inverse;
        let [x, y] = [x / radius, y / radius];
        let [gx, gy] = [xx * x + yx * y, xy * x + yy * y];
        let distance =
            (radius - self.scale / magnitude).abs() / (gx * gx + gy * gy).sqrt() * magnitude;
        (width * 0.5 + 0.5 - distance).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Viewport;

    fn circle(id: &str, tx: f64) -> Shape {
        Shape {
            curve: None,
            id: id.into(),
            radius: 1.0,
            kind: ShapeKind::Circular,
            feather: 0.0,
            points: vec![],
            feathers: vec![],
            to_canvas: Affine {
                xx: 20.0,
                xy: 0.0,
                yx: 0.0,
                yy: 10.0,
                tx,
                ty: 40.0,
            },
        }
    }
    fn overlay(circles: Vec<Shape>) -> WindowOverlay {
        WindowOverlay {
            shapes: circles.into(),
            selected: None,
            style: Style::default(),
            coverage: None,
        }
    }

    #[test]
    fn transform_handles_win_their_body_and_keep_soft_zero_recoverable() {
        let map = Viewport::Fit.map((120, 80), (120, 80), 0).unwrap();
        let mut shape = overlay(vec![circle("w", 60.0)]);
        shape.selected = Some("w".into());
        for (point, control) in [
            ([40.0, 30.0], Control::Size(0)),
            ([80.0, 30.0], Control::Size(1)),
            ([80.0, 50.0], Control::Size(2)),
            ([40.0, 50.0], Control::Size(3)),
            ([40.0, 40.0], Control::Aspect(0)),
            ([80.0, 40.0], Control::Aspect(1)),
            ([60.0, 30.0], Control::Aspect(2)),
            ([60.0, 50.0], Control::Aspect(3)),
            ([60.0, 32.5], Control::Rotate),
            ([77.32051, 45.0], Control::Softness(1)),
        ] {
            assert_eq!(
                shape.hit_control(map, point),
                Some(Hit { id: "w", control })
            );
            assert!(shape.outline(map, point).is_some());
        }
        shape.shapes[0].kind = ShapeKind::Gradient;
        for (point, control) in [
            ([60.0, 40.0], Control::Move),
            ([60.0, 50.0], Control::Rotate),
            ([40.0, 40.0], Control::Softness(-1)),
            ([80.0, 40.0], Control::Softness(1)),
        ] {
            assert_eq!(
                shape.hit_control(map, point),
                Some(Hit { id: "w", control })
            );
        }
        assert_eq!(shape.hit(map, [5.0, 5.0]), Some("w"));
        assert_eq!(shape.hit(map, [5.0, 75.0]), None);
        shape.shapes[0].to_canvas.xx = 0.0;
        assert!(shape.hit(map, [60.0, 40.0]).is_none());
    }

    #[test]
    fn only_the_selected_window_shows_and_hits_controls_others_keep_a_grey_body() {
        let map = Viewport::Fit.map((120, 80), (120, 80), 0).unwrap();
        let mut shape = overlay(vec![circle("a", 30.0), circle("b", 90.0)]);
        shape.shapes[1].feather = 0.25;
        shape.selected = Some("a".into());
        assert_eq!(
            shape.hit_control(map, [10.0, 30.0]),
            Some(Hit {
                id: "a",
                control: Control::Size(0)
            })
        );
        // The unselected window draws no handles: its corner is empty, its axis handle,
        // rotate handle and outer soft edge are plain body/outline that select or move it.
        assert_eq!(shape.hit_control(map, [70.0, 30.0]), None);
        assert!(shape.outline(map, [70.0, 30.0]).is_none());
        for point in [[70.0, 40.0], [90.0, 32.5], [90.0, 40.0]] {
            assert_eq!(
                shape.hit_control(map, point),
                Some(Hit {
                    id: "b",
                    control: Control::Move
                }),
                "{point:?}"
            );
        }
        assert!(shape.outline(map, [115.0, 40.0]).is_none());
        assert!(shape.outline(map, [70.0, 40.0]).is_some());
        shape.selected = Some("b".into());
        assert_eq!(
            shape.hit_control(map, [70.0, 30.0]),
            Some(Hit {
                id: "b",
                control: Control::Size(0)
            })
        );
        assert_eq!(shape.hit_control(map, [10.0, 30.0]), None);
    }

    #[test]
    fn circular_and_gradient_softness_rings_are_drawn_but_never_hit() {
        let map = Viewport::Fit.map((120, 80), (120, 80), 0).unwrap();
        let mut shape = overlay(vec![circle("w", 60.0)]);
        shape.selected = Some("w".into());
        shape.shapes[0].feather = 0.4;
        // The outer Soft 1 ring, outside the body and away from its handle: display only.
        assert!(shape.outline(map, [88.0, 40.0]).is_some());
        assert_eq!(shape.hit_control(map, [88.0, 40.0]), None);
        // The explicit softness handle on that ring is still grabbable.
        assert_eq!(
            shape.hit_control(map, [84.249_04, 47.0]),
            Some(Hit {
                id: "w",
                control: Control::Softness(1)
            })
        );
        // Gradient: the line on the dark side is drawn but not hit; its handle is.
        shape.shapes[0].kind = ShapeKind::Gradient;
        assert!(shape.outline(map, [30.0, 44.0]).is_some());
        assert_eq!(shape.hit_control(map, [30.0, 44.0]), None);
        assert_eq!(
            shape.hit_control(map, [80.0, 44.0]),
            Some(Hit {
                id: "w",
                control: Control::Softness(1)
            })
        );
        // The main boundary still moves the window.
        assert_eq!(
            shape.hit_control(map, [30.0, 40.0]),
            Some(Hit {
                id: "w",
                control: Control::Move
            })
        );
    }

    #[test]
    fn transform_full_softness_omits_the_collapsed_inner_circle() {
        let map = Viewport::Fit.map((120, 80), (120, 80), 0).unwrap();
        let mut c = circle("w", 60.0);
        c.feather = c.radius;
        let primitives = c.primitives(map, Style::default(), true);
        assert_eq!(primitives.iter().filter(|p| p.control.is_none()).count(), 2);
        assert!(primitives
            .iter()
            .all(|p| p.alpha([60.0, 40.0], 2.0).is_finite()));
    }

    #[test]
    fn affine_rotation_placement_and_zoom_use_oriented_canvas_pixels() {
        let mut c = circle("rotated", 60.0);
        c.to_canvas = Affine {
            xx: 0.0,
            xy: -10.0,
            yx: 20.0,
            yy: 0.0,
            tx: 60.0,
            ty: 40.0,
        };
        let shape = overlay(vec![c]);
        let map = Viewport::Scale {
            scale: 2.0,
            center: [0.5, 0.5],
        }
        .map((160, 100), (120, 80), 0)
        .unwrap();
        assert_eq!(shape.hit(map, [80.0, 88.0]), Some("rotated"));
        assert_eq!(shape.hit(map, [108.0, 50.0]), None);
        assert!(shape.outline(map, [100.0, 50.0]).is_some());
        assert!(shape.outline(map, [80.0, 90.0]).is_some());
        assert!(shape.outline(map, [80.0, 50.0]).is_none());
    }

    #[test]
    fn cropped_picture_blackbars_zero_size_and_overlap_are_unambiguous() {
        let map = Viewport::Fit.map((120, 120), (120, 80), 0).unwrap();
        let mut shape = overlay(vec![circle("first", 60.0), circle("last", 60.0)]);
        assert_eq!(shape.hit(map, [60.0, 60.0]), Some("last"));
        shape.selected = Some("first".into());
        assert_eq!(shape.hit(map, [60.0, 60.0]), Some("first"));
        assert!(shape.hit(map, [60.0, 10.0]).is_none());
        assert!(shape.outline(map, [60.0, 10.0]).is_none());
        let cropped = Viewport::Scale {
            scale: 4.0,
            center: [0.5, 0.5],
        }
        .map((80, 60), (120, 80), 0)
        .unwrap();
        assert!(shape.hit(cropped, [-0.5, 30.0]).is_none());
        assert!(shape.hit(cropped, [79.5, 30.0]).is_some());
        for c in &mut shape.shapes {
            c.to_canvas.xx = 0.0;
            c.to_canvas.yy = 0.0;
        }
        assert!(
            shape.hit(map, [60.0, 60.0]).is_none(),
            "Size 0 has no centre handle"
        );
        assert!(shape.outline(map, [60.0, 60.0]).is_none());
    }

    #[test]
    fn coverage_clips_each_stroke_pixel_and_hit_on_the_same_soft_boundary() {
        let map = Viewport::Fit.map((120, 80), (120, 80), 0).unwrap();
        let mut shape = overlay(vec![circle("circle", 60.0)]);
        shape.coverage = Some(Wipe {
            softness: 0.25,
            ..Wipe::default()
        });
        // The upper edge crosses the mixed/pure-B boundary x=70; the circle centre is on A.
        assert_eq!(shape.hit(map, [69.5, 31.0]), Some("circle"));
        assert!(shape.outline(map, [69.5, 31.0]).is_some());
        assert!(shape.hit(map, [70.5, 31.0]).is_none());
        assert!(shape.outline(map, [70.5, 31.0]).is_none());
        for y in 0..80 {
            for x in 0..120 {
                #[allow(clippy::cast_precision_loss)]
                let p = [x as f32 + 0.5, y as f32 + 0.5];
                if shape.outline(map, p).is_some() {
                    assert_eq!(shape.hit(map, p), Some("circle"));
                }
            }
        }
    }

    #[test]
    fn legal_extreme_zooms_keep_visible_interiors_and_finite_geometry() {
        let shape = overlay(vec![circle("circle", 60.0)]);
        for scale in [2.0_f64.powi(-126), 1e-20, 1e20, 2.0_f64.powi(112)] {
            let map = Viewport::Scale {
                scale,
                center: [0.5, 0.5],
            }
            .map((100, 100), (120, 80), 0)
            .unwrap();
            let circle = shape.shapes[0].device(map).unwrap();
            assert!(circle
                .inverse
                .iter()
                .chain(&circle.centre)
                .all(|v| v.is_finite()));
            for point in [[0.5, 0.5], [50.5, 50.5], [99.5, 99.5]] {
                assert!(circle.alpha(point, 2.0).is_finite(), "{scale}, {point:?}");
                assert_eq!(
                    shape.hit(map, point),
                    if scale > 1.0 { Some("circle") } else { None },
                    "{scale}, {point:?}"
                );
                assert!(shape.outline(map, point).is_none(), "{scale}, {point:?}");
            }
        }
    }

    #[test]
    fn bilinear_current_contribution_at_a_magnified_wipe_edge_can_be_hit() {
        let map = Viewport::Scale {
            scale: 2.0,
            center: [0.5, 0.5],
        }
        .map((240, 160), (120, 80), 0)
        .unwrap();
        let mut shape = overlay(vec![circle("circle", 50.25)]);
        shape.shapes[0].to_canvas.xx = 10.0;
        shape.shapes[0].to_canvas.ty = 40.25;
        shape.coverage = Some(Wipe::default());
        // canvas (60.25,40.25): 25% of texel x=59.5 (A) and 75% x=60.5 (B).
        assert_eq!(shape.hit(map, [120.5, 80.5]), Some("circle"));
        assert!(shape.outline(map, [120.5, 80.5]).is_some());
        // The adjacent pixel's footprint consists only of B, despite being in the stroke.
        assert!(shape.hit(map, [121.5, 80.5]).is_none());
        assert!(shape.outline(map, [121.5, 80.5]).is_none());
    }

    fn path(id: &str, kind: ShapeKind, points: Vec<[f64; 2]>, feather: f64) -> Shape {
        Shape {
            curve: None,
            id: id.into(),
            radius: 0.25,
            kind,
            feather: 0.0,
            feathers: vec![[feather, feather]; points.len()],
            points,
            to_canvas: Affine {
                xx: 80.0,
                xy: 0.0,
                yx: 0.0,
                yy: 80.0,
                tx: 60.0,
                ty: 40.0,
            },
        }
    }
    fn rectangle() -> Vec<[f64; 2]> {
        vec![[-0.25, -0.25], [0.25, -0.25], [0.25, 0.25], [-0.25, 0.25]]
    }

    #[test]
    fn linear_corners_sides_and_softness_handles_win_and_only_the_body_moves() {
        let map = Viewport::Fit.map((120, 80), (120, 80), 0).unwrap();
        let mut shape = overlay(vec![path("w", ShapeKind::Linear, rectangle(), 0.05)]);
        shape.selected = Some("w".into());
        for (point, control) in [
            ([40.0, 20.0], Control::Point(0)),
            ([80.0, 20.0], Control::Point(1)),
            ([80.0, 60.0], Control::Point(2)),
            ([40.0, 60.0], Control::Point(3)),
            ([60.0, 20.0], Control::Edge(0)),
            ([80.0, 40.0], Control::Edge(1)),
            ([60.0, 60.0], Control::Edge(2)),
            ([40.0, 40.0], Control::Edge(3)),
            // 3/4 along each side, on its outer softness line 4 device pixels out.
            ([70.0, 16.0], Control::EdgeSoftness(0)),
            ([84.0, 50.0], Control::EdgeSoftness(1)),
            ([50.0, 64.0], Control::EdgeSoftness(2)),
            ([36.0, 30.0], Control::EdgeSoftness(3)),
            ([50.0, 20.0], Control::Move),
            ([55.0, 45.0], Control::Move),
        ] {
            assert_eq!(
                shape.hit_control(map, point),
                Some(Hit { id: "w", control }),
                "{point:?}"
            );
        }
        assert_eq!(shape.hit(map, [10.0, 10.0]), None);
        assert!(
            shape.outline(map, [60.0, 16.0]).is_some(),
            "outer feather line"
        );
        assert!(
            shape.outline(map, [60.0, 24.0]).is_some(),
            "inner feather line"
        );
        // Unselected: a grey outline whose stroke and body move it; no handles or feathers.
        shape.selected = None;
        assert_eq!(
            shape.hit_control(map, [40.0, 20.0]),
            Some(Hit {
                id: "w",
                control: Control::Move
            })
        );
        assert_eq!(shape.hit(map, [70.0, 16.0]), None);
        assert!(shape.outline(map, [60.0, 16.0]).is_none());
        // Size 0 has neither outline nor controls.
        shape.shapes[0].to_canvas.xx = 0.0;
        shape.shapes[0].to_canvas.yy = 0.0;
        assert_eq!(shape.hit(map, [60.0, 40.0]), None);
        assert!(shape.outline(map, [60.0, 40.0]).is_none());
    }

    #[test]
    fn polygon_points_win_edges_insert_and_even_odd_bodies_are_never_reordered() {
        let map = Viewport::Fit.map((120, 80), (120, 80), 0).unwrap();
        let mut shape = overlay(vec![path("p", ShapeKind::Polygon, rectangle(), 0.05)]);
        shape.selected = Some("p".into());
        let hit = |shape: &WindowOverlay, point| shape.hit_control(map, point).map(|h| h.control);
        assert_eq!(hit(&shape, [41.0, 21.0]), Some(Control::Point(0)));
        assert_eq!(hit(&shape, [50.0, 20.5]), Some(Control::Insert(0)));
        assert_eq!(hit(&shape, [79.5, 45.0]), Some(Control::Insert(1)));
        assert_eq!(hit(&shape, [55.0, 45.0]), Some(Control::Move));
        // No softness handles on a Polygon (p. 3193); its guides are display only.
        assert_eq!(hit(&shape, [70.0, 16.0]), None);
        assert!(shape.outline(map, [70.0, 16.0]).is_some());
        // Crossing diagonals: equal distance picks the lower edge; even-odd lobes.
        shape.shapes[0].points = vec![[-0.25, -0.25], [0.25, 0.25], [0.25, -0.25], [-0.25, 0.25]];
        assert_eq!(hit(&shape, [60.0, 40.0]), Some(Control::Insert(0)));
        assert_eq!(hit(&shape, [70.0, 40.0]), Some(Control::Move));
        assert_eq!(hit(&shape, [60.0, 30.0]), None);
        // A pentagram's centre is outside by even-odd, though its winding number is two.
        shape.shapes[0].points = (0..5)
            .map(|k| {
                let a = (f64::from(k) * 144.0 - 90.0).to_radians();
                [0.25 * a.cos(), 0.25 * a.sin()]
            })
            .collect();
        shape.shapes[0].feathers = vec![[0.0; 2]; 5];
        assert_eq!(hit(&shape, [60.0, 40.0]), None);
        assert_eq!(hit(&shape, [60.0, 30.0]), Some(Control::Move));
        // Unselected: no point handles and no insertion.
        shape.selected = None;
        assert_eq!(hit(&shape, [60.0, 20.5]), Some(Control::Move));
    }

    /// Guide coverage at a device point, from the drawn primitives only.
    fn guide_alpha(shape: &WindowOverlay, map: ViewportMap, p: [f32; 2]) -> f32 {
        shape
            .primitives(&shape.shapes[0], map)
            .iter()
            .filter(|q| q.guide)
            .map(|q| q.alpha(p, shape.style.width))
            .fold(0.0, f32::max)
    }

    #[test]
    #[allow(clippy::cast_possible_truncation)]
    fn guides_follow_the_exact_mask_contours_through_rotation_and_aspect() {
        let map = Viewport::Scale {
            scale: 1.25,
            center: [0.5, 0.5],
        }
        .map((160, 120), (120, 80), 0)
        .unwrap();
        let (sin, cos) = 0.5_f64.sin_cos();
        let mut shape = overlay(vec![path("p", ShapeKind::Polygon, rectangle(), 0.08)]);
        // Rotated, with a different scale on each local axis.
        shape.shapes[0].to_canvas = Affine {
            xx: 90.0 * cos,
            xy: -60.0 * sin,
            yx: 90.0 * sin,
            yy: 60.0 * cos,
            tx: 60.0,
            ty: 40.0,
        };
        shape.selected = Some("p".into());
        let placed = shape.shapes[0].clone();
        let device = |local: [f64; 2]| placed.at(map, local).map(|v| v as f32);
        // The inner contour is the mitred rectangle ±.17; the outer one rounds each corner.
        for local in [
            [0.17, 0.17],
            [-0.17, 0.0],
            [0.0, -0.33],
            [0.25 + 0.08 * 0.6, -0.25 - 0.08 * 0.8],
            [0.33, 0.1],
        ] {
            assert!(guide_alpha(&shape, map, device(local)) > 0.5, "{local:?}");
        }
        // The per-edge offsets overshot the inner corners; the outer corners are not mitred.
        for local in [[0.24, 0.17], [0.17, -0.245], [0.33, 0.33], [0.33, -0.33]] {
            assert_eq!(guide_alpha(&shape, map, device(local)), 0.0, "{local:?}");
        }
        // A Linear side with no softness has no guide beside it; the others keep theirs.
        shape.shapes[0].kind = ShapeKind::Linear;
        shape.shapes[0].feathers[1] = [0.0; 2];
        assert_eq!(guide_alpha(&shape, map, device([0.33, 0.0])), 0.0);
        assert!(guide_alpha(&shape, map, device([-0.33, 0.0])) > 0.5);
    }

    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn narrow_polygon_draws_no_inner_guide_and_its_guides_never_hit() {
        let map = Viewport::Fit.map((120, 80), (120, 80), 0).unwrap();
        let narrow = vec![[-0.4, -0.05], [0.4, -0.05], [0.4, 0.05], [-0.4, 0.05]];
        let mut shape = overlay(vec![path("p", ShapeKind::Polygon, narrow, 0.0)]);
        // Inside 40 and Outside 10: .2 and .05 image heights; the shape is .1 high.
        shape.shapes[0].feathers = vec![[0.2, 0.05]; 4];
        shape.selected = Some("p".into());
        // Device: the window spans x 28..92, y 36..44; the outer guide is 4 pixels out.
        for y in 0..80 {
            for x in 0..120 {
                let p = [x as f32 + 0.5, y as f32 + 0.5];
                let inside_window = (28.0..92.0).contains(&p[0]) && (36.0..44.0).contains(&p[1]);
                if guide_alpha(&shape, map, p) > 0.0 {
                    // Only the outer guide: no pixel within the window, none beyond 4 + 1.5.
                    assert!(!inside_window, "{p:?}");
                    let gap = (28.0 - p[0])
                        .max(p[0] - 92.0)
                        .max(36.0 - p[1])
                        .max(p[1] - 44.0);
                    assert!(gap > 1.0, "{p:?}");
                }
            }
        }
        // The old per-edge inner offset drew a phantom line outside, at y = 40 + 12.
        assert_eq!(guide_alpha(&shape, map, [60.0, 52.0]), 0.0);
        assert!(shape.outline(map, [60.0, 32.0]).is_some());
        assert_eq!(
            shape.hit(map, [60.0, 32.0]),
            None,
            "a guide is display only"
        );
        assert_eq!(shape.hit(map, [60.0, 52.0]), None);
        assert_eq!(
            shape.hit_control(map, [60.0, 40.0]).map(|h| h.control),
            Some(Control::Move)
        );
    }

    #[test]
    #[allow(clippy::cast_precision_loss, clippy::many_single_char_names)]
    fn every_covered_pixel_lies_in_its_primitive_quad() {
        let mut circular = circle("c", 60.0);
        circular.feather = 0.4;
        let mut gradient = circle("g", 60.0);
        gradient.kind = ShapeKind::Gradient;
        gradient.feather = 0.3;
        gradient.to_canvas.xy = 7.0;
        let star: Vec<_> = (0..5)
            .map(|k| {
                let a = (f64::from(k) * 144.0 - 90.0).to_radians();
                [0.3 * a.cos(), 0.3 * a.sin()]
            })
            .collect();
        let mut linear = path("l", ShapeKind::Linear, rectangle(), 0.05);
        linear.to_canvas.xy = -30.0;
        let shapes = [
            circular,
            gradient,
            linear,
            path("p", ShapeKind::Polygon, star, 0.03),
            bezier_shape("bez", &round(), true),
            bezier_shape("draft", &square(), false),
        ];
        for shape in shapes {
            let mut osc = overlay(vec![shape]);
            osc.selected = Some(osc.shapes[0].id.clone());
            for scale in [0.3, 1.0, 3.5, 1e20] {
                let map = Viewport::Scale {
                    scale,
                    center: [0.45, 0.55],
                }
                .map((100, 70), (120, 80), 0)
                .unwrap();
                for primitive in osc.primitives(&osc.shapes[0], map) {
                    let quad = primitive
                        .quad(map, osc.style.width)
                        .map(|q| q.map(f64::from));
                    for y in 0..70 {
                        for x in 0..100 {
                            let p = [x as f32 + 0.5, y as f32 + 0.5];
                            if primitive.alpha(p, osc.style.width) <= 0.0 || !osc.visible(map, p) {
                                continue;
                            }
                            let q = quad.unwrap_or_else(|| panic!("{p:?} has no quad"));
                            let d = [f64::from(p[0]) - q[0], f64::from(p[1]) - q[1]];
                            let det = q[2] * q[5] - q[3] * q[4];
                            let s = (d[0] * q[5] - d[1] * q[4]) / det;
                            let t = (q[2] * d[1] - q[3] * d[0]) / det;
                            assert!(
                                (0.0..=1.0).contains(&s) && (0.0..=1.0).contains(&t),
                                "{} {scale} {p:?} kind {}",
                                osc.shapes[0].id,
                                primitive.kind
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn path_controls_stay_finite_and_clipped_at_legal_extreme_zooms() {
        let mut shape = overlay(vec![path("p", ShapeKind::Polygon, rectangle(), 0.05)]);
        shape.selected = Some("p".into());
        for scale in [2.0_f64.powi(-126), 1e-20, 1e20, 2.0_f64.powi(112)] {
            let map = Viewport::Scale {
                scale,
                center: [0.5, 0.5],
            }
            .map((100, 100), (120, 80), 0)
            .unwrap();
            for p in shape.primitives(&shape.shapes[0], map) {
                assert!(p
                    .geometry
                    .inverse
                    .iter()
                    .chain(&p.geometry.centre)
                    .all(|v| v.is_finite() && v.abs() < 1e6));
            }
            for point in [[0.5, 0.5], [50.5, 50.5], [99.5, 99.5]] {
                assert_eq!(
                    shape.hit(map, point),
                    if scale > 1.0 { Some("p") } else { None },
                    "{scale}, {point:?}"
                );
            }
        }
    }

    /// A Curve on the 120x80 test map: local (x, y) is device (60 + 80x, 40 + 80y).
    fn bezier_shape(id: &str, anchors: &[([f64; 2], [f64; 2], [f64; 2])], closed: bool) -> Shape {
        Shape {
            id: id.into(),
            radius: 0.25,
            kind: ShapeKind::Curve,
            feather: 0.0,
            points: anchors.iter().map(|a| a.0).collect(),
            feathers: vec![[0.05, 0.05]],
            curve: Some(Bezier {
                handles: anchors.iter().map(|a| [a.1, a.2]).collect(),
                closed,
                tolerance: curve::tolerance(1.0),
            }),
            to_canvas: Affine {
                xx: 80.0,
                xy: 0.0,
                yx: 0.0,
                yy: 80.0,
                tx: 60.0,
                ty: 40.0,
            },
        }
    }
    /// The test rectangle as Curve anchors; anchor 2's handles lie along its two edges, so
    /// every edge stays straight.
    fn square() -> Vec<([f64; 2], [f64; 2], [f64; 2])> {
        vec![
            ([-0.25, -0.25], [0.0; 2], [0.0; 2]),
            ([0.25, -0.25], [0.0; 2], [0.0; 2]),
            ([0.25, 0.25], [0.0, -0.1], [-0.1, 0.0]),
            ([-0.25, 0.25], [0.0; 2], [0.0; 2]),
        ]
    }

    #[test]
    fn curve_anchors_handles_softness_and_edges_win_in_order_and_only_the_body_moves() {
        let map = Viewport::Fit.map((120, 80), (120, 80), 0).unwrap();
        let mut shape = overlay(vec![bezier_shape("c", &square(), true)]);
        shape.selected = Some("c".into());
        let hit = |shape: &WindowOverlay, point| shape.hit_control(map, point).map(|h| h.control);
        for (point, control) in [
            ([40.0, 20.0], Control::Point(0)),
            ([80.0, 60.0], Control::Point(2)),
            ([80.0, 52.0], Control::HandleIn(2)),
            ([72.0, 60.0], Control::HandleOut(2)),
            ([60.0, 20.5], Control::Insert(0)),
            ([80.0, 40.0], Control::Insert(1)),
            // Edge 0 at t = 1/3 and 2/3, four device pixels in and out (Inside/Outside .05).
            ([50.37, 24.0], Control::Softness(-1)),
            ([69.63, 16.0], Control::Softness(1)),
            ([60.0, 40.0], Control::Move),
        ] {
            assert_eq!(hit(&shape, point), Some(control), "{point:?}");
        }
        assert_eq!(hit(&shape, [10.0, 10.0]), None, "outside a closed Curve");
        // The handle lines are drawn but display only: under them is the edge.
        assert!(shape.outline(map, [80.0, 55.0]).is_some());
        assert_eq!(hit(&shape, [80.0, 46.0]), Some(Control::Insert(1)));
        // Unselected: a grey outline and body that move it; no handles, no insertion.
        shape.selected = None;
        for point in [[40.0, 20.0], [60.0, 20.5], [60.0, 40.0]] {
            assert_eq!(hit(&shape, point), Some(Control::Move), "{point:?}");
        }
        assert!(
            shape.outline(map, [72.0, 60.0]).is_some(),
            "the outline itself"
        );
        assert!(shape.outline(map, [80.0, 28.0]).is_some());
        // No softness handle where the selected Curve had one; four pixels out is its margin.
        assert_eq!(hit(&shape, [69.63, 16.0]), Some(Control::Move));
        assert_eq!(hit(&shape, [69.63, 15.0]), None);
    }

    #[test]
    fn a_closed_curve_grips_a_down_just_outside_it_below_its_controls_and_above_others() {
        let map = Viewport::Fit.map((120, 80), (120, 80), 0).unwrap();
        // A circle underneath spans device 40..80 x 10..70; the Curve square is 40..80 x 20..60.
        let mut under = circle("under", 60.0);
        under.to_canvas.yy = 30.0;
        let mut shape = overlay(vec![under, bezier_shape("c", &square(), true)]);
        let hit = |shape: &WindowOverlay, point: [f32; 2]| {
            shape
                .hit_control(map, point)
                .map(|h| (h.id.to_owned(), h.control))
        };
        let on = |id: &str, control| Some((id.to_owned(), control));
        for selected in [None, Some("c")] {
            shape.selected = selected.map(Into::into);
            // Within 4.5 device pixels of the outline, whether selected or not, the Curve moves.
            for point in [[60.0, 16.0], [60.0, 15.6], [84.0, 40.0], [84.4, 40.0]] {
                assert_eq!(
                    hit(&shape, point),
                    on("c", Control::Move),
                    "{selected:?} {point:?}"
                );
            }
            // Beyond it, the window underneath, or nothing.
            assert_eq!(hit(&shape, [60.0, 15.0]), on("under", Control::Move));
            assert_eq!(hit(&shape, [85.0, 40.0]), None);
        }
        // Selected, its anchors and edges still win their own pixels.
        assert_eq!(hit(&shape, [40.0, 17.0]), on("c", Control::Point(0)));
        assert_eq!(hit(&shape, [60.0, 20.5]), on("c", Control::Insert(0)));
        // A draft has no body and so no margin: selected it draws, unselected the one under moves.
        shape.shapes[1] = bezier_shape("c", &square(), false);
        assert_eq!(hit(&shape, [60.0, 16.0]), on("c", Control::Append));
        assert_eq!(hit(&shape, [84.0, 40.0]), on("c", Control::Append));
        shape.selected = None;
        assert_eq!(hit(&shape, [60.0, 16.0]), on("under", Control::Move));
        assert_eq!(hit(&shape, [84.0, 40.0]), None);
    }

    #[test]
    fn curve_softness_handles_take_their_side_from_the_lobe_they_stand_on() {
        // A figure eight whose small right lobe holds edge 0; the big left lobe decides the
        // signed area, which would put the inside handle outside the small lobe.
        let eight = vec![
            ([0.2, -0.1], [0.0; 2], [0.0; 2]),
            ([0.2, 0.1], [0.0; 2], [0.0; 2]),
            ([-0.4, -0.2], [0.0; 2], [0.0; 2]),
            ([-0.4, 0.2], [0.0; 2], [0.0; 2]),
        ];
        let shape = bezier_shape("e", &eight, true);
        let outline: Vec<[f64; 2]> = eight.iter().map(|a| a.0).collect();
        let handles = shape.curve_softness_handles();
        assert_eq!(handles.len(), 2);
        for (control, at, normal) in handles {
            let inside = control == Control::Softness(-1);
            assert_eq!(offset::inside(&outline, at), inside, "{control:?} {at:?}");
            // Its normal points further in for the inside handle, further out for the outside one.
            let expected = if inside { -1.0 } else { 1.0 };
            assert!(
                (normal[0] - expected).abs() < 1e-12 && normal[1].abs() < 1e-12,
                "{normal:?}"
            );
        }
    }

    #[test]
    fn a_selected_draft_closes_on_its_first_point_and_takes_every_other_click_to_draw() {
        let map = Viewport::Fit.map((120, 80), (120, 80), 0).unwrap();
        let mut shape = overlay(vec![
            circle("other", 100.0),
            bezier_shape("d", &square(), false),
        ]);
        shape.selected = Some("d".into());
        let control = |shape: &WindowOverlay, point: [f32; 2]| {
            shape.hit_control(map, point).map(|h| h.control)
        };
        assert_eq!(control(&shape, [40.0, 20.0]), Some(Control::Close));
        assert_eq!(control(&shape, [40.0, 60.0]), Some(Control::Point(3)));
        assert_eq!(control(&shape, [60.0, 20.5]), Some(Control::Insert(0)));
        // No closing edge, no body, no softness: everything else draws, even over another window.
        assert!(shape.outline(map, [40.0, 40.0]).is_none());
        assert!(
            shape.outline(map, [60.0, 16.0]).is_none(),
            "no guides on a draft"
        );
        for point in [[40.0, 40.0], [60.0, 40.0], [10.0, 10.0], [100.0, 40.0]] {
            assert_eq!(
                shape.hit_control(map, point),
                Some(Hit {
                    id: "d",
                    control: Control::Append
                }),
                "{point:?}"
            );
        }
        // Unselected, a draft is a grey open outline that selects (moves) it; elsewhere nothing.
        shape.selected = None;
        assert_eq!(
            shape.hit_control(map, [60.0, 20.5]),
            Some(Hit {
                id: "d",
                control: Control::Move
            })
        );
        assert_eq!(shape.hit_control(map, [60.0, 40.0]), None);
        // One point: it is that point, not a close; an empty draft still draws where clicked.
        shape.selected = Some("d".into());
        shape.shapes[1] = bezier_shape("d", &square()[..1], false);
        assert_eq!(control(&shape, [40.0, 20.0]), Some(Control::Point(0)));
        shape.shapes[1] = bezier_shape("d", &[], false);
        assert_eq!(control(&shape, [60.0, 40.0]), Some(Control::Append));
        // Size 0 has no inverse: nothing is drawn and no point can be placed.
        shape.shapes[1].to_canvas.xx = 0.0;
        shape.shapes[1].to_canvas.yy = 0.0;
        assert_eq!(control(&shape, [60.0, 40.0]), None);
    }

    /// A circle of radius .25 from four Bezier quarter arcs (kappa .5523).
    fn round() -> Vec<([f64; 2], [f64; 2], [f64; 2])> {
        let k = 0.25 * 0.552_284_749_8;
        vec![
            ([0.25, 0.0], [0.0, -k], [0.0, k]),
            ([0.0, 0.25], [k, 0.0], [-k, 0.0]),
            ([-0.25, 0.0], [0.0, k], [0.0, -k]),
            ([0.0, -0.25], [-k, 0.0], [k, 0.0]),
        ]
    }

    #[test]
    #[allow(clippy::cast_possible_truncation)]
    fn curve_outline_follows_its_cubics_and_its_guides_are_the_exact_offsets_never_hit() {
        let map = Viewport::Scale {
            scale: 1.25,
            center: [0.5, 0.5],
        }
        .map((160, 120), (120, 80), 0)
        .unwrap();
        let mut shape = overlay(vec![bezier_shape("c", &round(), true)]);
        let (sin, cos) = 0.5_f64.sin_cos();
        // Rotated, with a different scale on each local axis.
        shape.shapes[0].to_canvas = Affine {
            xx: 90.0 * cos,
            xy: -60.0 * sin,
            yx: 90.0 * sin,
            yy: 60.0 * cos,
            tx: 60.0,
            ty: 40.0,
        };
        shape.shapes[0].feathers = vec![[0.06, 0.1]];
        shape.selected = Some("c".into());
        let placed = shape.shapes[0].clone();
        let device = |local: [f64; 2]| placed.at(map, local).map(|v| v as f32);
        let anchors = placed.anchors();
        for j in 0..4 {
            for k in 1..8 {
                let t = f64::from(k) / 8.0;
                let (on, _) = cubic_at(curve::cubic(&anchors, j), t);
                assert!(shape.outline(map, device(on)).is_some(), "edge {j} t {t}");
                // The exact offsets of the curve (a circle within its .03% Bezier error).
                let r = on[0].hypot(on[1]);
                let radial = [on[0] / r, on[1] / r];
                for extent in [-0.06, 0.1] {
                    let guide = [on[0] + radial[0] * extent, on[1] + radial[1] * extent];
                    assert!(guide_alpha(&shape, map, device(guide)) > 0.5, "{guide:?}");
                }
                let between = [on[0] - radial[0] * 0.03, on[1] - radial[1] * 0.03];
                assert_eq!(guide_alpha(&shape, map, device(between)), 0.0);
            }
        }
        // A guide outside the body and its grip margin hits nothing; inside, the body moves it.
        let outer = device([-0.35 * std::f64::consts::FRAC_1_SQRT_2; 2]);
        assert!(shape.outline(map, outer).is_some());
        assert_eq!(shape.hit_control(map, outer), None);
        let inner = device([-0.19 * std::f64::consts::FRAC_1_SQRT_2; 2]);
        assert_eq!(
            shape.hit_control(map, inner).map(|h| h.control),
            Some(Control::Move)
        );
    }

    #[test]
    fn a_curve_click_names_the_exact_parameter_of_its_edge() {
        let shape = bezier_shape("c", &round(), true);
        let anchors = shape.anchors();
        for (j, t) in [(0, 0.2), (1, 0.5), (3, 0.9)] {
            let (on, _) = cubic_at(curve::cubic(&anchors, j), t);
            let canvas = [60.0 + 80.0 * on[0], 40.0 + 80.0 * on[1]];
            let got = shape.curve_parameter(j, canvas).unwrap();
            assert!((got - t).abs() < 1e-3, "{j} {t} {got}");
        }
        // A straight edge with corner handles is not uniform in its parameter: still the
        // parameter of the clicked spot itself.
        let corners = bezier_shape("s", &square(), true);
        let t = corners.curve_parameter(0, [50.0, 20.0]).unwrap();
        let (on, _) = cubic_at(curve::cubic(&corners.anchors(), 0), t);
        assert!((60.0 + 80.0 * on[0] - 50.0).abs() < 1e-6, "{t} {on:?}");
        // Next to an anchor the parameter stays off it, and an open draft has no closing edge.
        assert_eq!(shape.curve_parameter(0, [80.0, 40.0]), Some(1e-3));
        let draft = bezier_shape("d", &round(), false);
        assert_eq!(draft.curve_parameter(3, [60.0, 20.0]), None);
        assert!(draft.curve_parameter(2, [40.0, 40.0]).is_some());
    }

    #[test]
    fn curve_controls_stay_finite_and_bounded_at_legal_extreme_zooms() {
        let mut shape = overlay(vec![bezier_shape("c", &round(), true)]);
        shape.selected = Some("c".into());
        for scale in [2.0_f64.powi(-126), 1e-20, 1e20, 2.0_f64.powi(112)] {
            let map = Viewport::Scale {
                scale,
                center: [0.5, 0.5],
            }
            .map((100, 100), (120, 80), 0)
            .unwrap();
            let primitives = shape.primitives(&shape.shapes[0], map);
            // Never flattened more finely than the mask itself.
            assert!(primitives.len() < 40_000, "{scale}: {}", primitives.len());
            for p in primitives {
                assert!(p
                    .geometry
                    .inverse
                    .iter()
                    .chain(&p.geometry.centre)
                    .all(|v| v.is_finite() && v.abs() < 1e6));
            }
            for point in [[0.5, 0.5], [50.5, 50.5], [99.5, 99.5]] {
                assert_eq!(
                    shape.hit(map, point),
                    if scale > 1.0 { Some("c") } else { None },
                    "{scale}, {point:?}"
                );
            }
        }
    }
}
