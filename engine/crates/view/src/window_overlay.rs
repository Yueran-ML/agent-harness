//! Evaluated Window controls of one composed frame. Coordinates are canvas and device pixels,
//! never CSS pixels. The producer chooses the active OSC and supplies no overlay for Split.

use crate::ViewportMap;
use columbina_compose::wipe::Wipe;

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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeKind {
    Circular,
    Gradient,
}

/// Indices are stable local corner/axis identifiers, also used to break equal-distance ties.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Move,
    Size(u8),
    Aspect(u8),
    Rotate,
    Softness(i8),
}

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
            let Some(body) = shape.device(map) else {
                continue;
            };
            let primitives = self.primitives(shape, map);
            let handle = primitives
                .iter()
                .filter(|p| p.control.is_some() && p.alpha(device, self.style.width) > 0.0)
                .min_by(|a, b| {
                    a.distance_to_handle(device)
                        .total_cmp(&b.distance_to_handle(device))
                });
            if let Some(control) = handle.and_then(|handle| handle.control) {
                return Some(Hit {
                    id: &shape.id,
                    control,
                });
            }
            let inside = match shape.kind {
                ShapeKind::Circular => body.contains(device),
                ShapeKind::Gradient => body.local(device).0[1] <= 0.0,
            };
            if inside
                || primitives
                    .iter()
                    .any(|p| p.alpha(device, self.style.width) > 0.0)
            {
                return Some(Hit {
                    id: &shape.id,
                    control: Control::Move,
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
}
impl Primitive {
    fn distance_to_handle(&self, p: [f32; 2]) -> f32 {
        (p[0] - self.anchor[0]).hypot(p[1] - self.anchor[1])
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

    fn boundaries(&self, map: ViewportMap, style: Style, selected: bool) -> Vec<Primitive> {
        let Some(body) = self.device(map) else {
            return vec![];
        };
        let mut out = vec![];
        let r = self.radius;
        let f = self.feather;
        let mut boundary = |offset: f64, colour| {
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
            });
        };
        if f > 0.0 && selected {
            let base = if self.kind == ShapeKind::Circular {
                r
            } else {
                0.0
            };
            boundary(base - f, style.unselected);
            boundary(base + f, style.unselected);
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
        );
        out
    }

    pub(crate) fn primitives(
        &self,
        map: ViewportMap,
        style: Style,
        selected: bool,
    ) -> Vec<Primitive> {
        if self.device(map).is_none() {
            return vec![];
        }
        let mut out = self.boundaries(map, style, selected);
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
                });
            }
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
            id: id.into(),
            radius: 1.0,
            kind: ShapeKind::Circular,
            feather: 0.0,
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
}
