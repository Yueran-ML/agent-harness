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

/// A circle about the local origin, evaluated by the producer at the frame being displayed.
#[derive(Clone, Debug, PartialEq)]
pub struct Circular {
    pub id: String,
    pub radius: f64,
    pub to_canvas: Affine,
}

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
    pub circles: Box<[Circular]>,
    pub selected: Option<String>,
    pub style: Style,
    pub coverage: Option<Wipe>,
}

impl WindowOverlay {
    /// Hit at a device pixel centre. Selected wins overlaps, then the last circle in the list.
    /// This is a pointer-down query; a captured drag continues across a comparison boundary.
    #[must_use]
    pub fn hit(&self, map: ViewportMap, device: [f32; 2]) -> Option<&str> {
        self.at(map, device, |circle| {
            circle.contains(device) || circle.alpha(device, self.style.width) > 0.0
        })
        .map(|(id, _)| id)
    }

    /// Stroke coverage at a device pixel centre, including its antialiased half-pixel fringe.
    #[must_use]
    pub fn outline(&self, map: ViewportMap, device: [f32; 2]) -> Option<(&str, f32)> {
        self.at(map, device, |circle| {
            circle.alpha(device, self.style.width) > 0.0
        })
        .map(|(id, circle)| (id, circle.alpha(device, self.style.width)))
    }

    fn at(
        &self,
        map: ViewportMap,
        device: [f32; 2],
        accepts: impl Fn(DeviceCircle) -> bool,
    ) -> Option<(&str, DeviceCircle)> {
        if !self.visible(map, device) {
            return None;
        }
        let find = |selected| {
            self.circles
                .iter()
                .rev()
                .filter(|circle| (self.selected.as_deref() == Some(circle.id.as_str())) == selected)
                .find_map(|circle| {
                    circle
                        .device(map)
                        .filter(|shape| accepts(*shape))
                        .map(|shape| (circle.id.as_str(), shape))
                })
        };
        find(true).or_else(|| find(false))
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

impl Circular {
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

    fn circle(id: &str, tx: f64) -> Circular {
        Circular {
            id: id.into(),
            radius: 1.0,
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
    fn overlay(circles: Vec<Circular>) -> WindowOverlay {
        WindowOverlay {
            circles: circles.into(),
            selected: None,
            style: Style::default(),
            coverage: None,
        }
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
        for c in &mut shape.circles {
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
            let circle = shape.circles[0].device(map).unwrap();
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
        shape.circles[0].to_canvas.xx = 10.0;
        shape.circles[0].to_canvas.ty = 40.25;
        shape.coverage = Some(Wipe::default());
        // canvas (60.25,40.25): 25% of texel x=59.5 (A) and 75% x=60.5 (B).
        assert_eq!(shape.hit(map, [120.5, 80.5]), Some("circle"));
        assert!(shape.outline(map, [120.5, 80.5]).is_some());
        // The adjacent pixel's footprint consists only of B, despite being in the stroke.
        assert!(shape.hit(map, [121.5, 80.5]).is_none());
        assert!(shape.outline(map, [121.5, 80.5]).is_none());
    }
}
