//! Power Window geometry, kept with the corrector that it keys.
//!
//! The five shapes and the directions/defaults of Transform follow Resolve 20, pp. 3185–3199.
//! Numeric bounds, the local geometry, feather defaults and the transform formula below are
//! this project's choices: the manual does not publish those values or formulas.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::{check_label, Bounds, Param};
use crate::{Id, OpError};

pub mod topology;

/// The Polygon limit documented on p. 3194; it is not a limit on Curve windows.
pub const MOST_POLYGON_POINTS: usize = 128;

/// The default circle radius and rectangle half-side, in units of the image's height.
pub const DEFAULT_EXTENT: f64 = 0.25;

/// Window parameter defaults and bounds. The Transform defaults (50, and Rotate 0) are from
/// pp. 3191–3192. All ranges, Opacity 100 and Soft 1 are project choices pending comparison
/// with Resolve on a real installation; keeping them here lets that comparison refine them.
pub mod bounds {
    use super::Bounds;

    /// Uniform scale: 50 is one times the local geometry; 0 collapses it to its centre.
    pub const SIZE: Bounds = Bounds::new(50.0, 0.0, 1000.0);
    /// Shape aspect: 50 is unchanged, larger values widen and smaller values heighten it.
    pub const ASPECT: Bounds = Bounds::new(50.0, 0.0, 100.0);
    /// Horizontal centre: 50 is the middle of the picture, 100 its right edge.
    pub const PAN: Bounds = Bounds::new(50.0, -1000.0, 1000.0);
    /// Vertical centre: 50 is the middle of the picture, 100 its top edge.
    pub const TILT: Bounds = Bounds::new(50.0, -1000.0, 1000.0);
    /// Rotation in clockwise degrees, with room for several animated turns.
    pub const ROTATE: Bounds = Bounds::new(0.0, -36000.0, 36000.0);
    /// Contribution to the node's window key, as a percentage.
    pub const OPACITY: Bounds = Bounds::new(100.0, 0.0, 100.0);
    /// Linear, Circular and Gradient softness.
    pub const SOFTNESS: Bounds = Bounds::new(1.0, 0.0, 100.0);
    /// Polygon and Curve inside/outside softness, initially without feathering.
    pub const FEATHER: Bounds = Bounds::new(0.0, 0.0, 100.0);
    /// A local point or relative Bezier handle coordinate, in units of image height.
    pub const COORDINATE: Bounds = Bounds::new(0.0, -16.0, 16.0);
}

/// One window in a corrector's ordered list. Its id is stable and unique only within that
/// node, so copying a grade or applying a preset does not allocate project-wide ids.
/// Numeric parameters use the same source-frame clock as the rest of the clip's grade;
/// a timeline grade uses timeline frames. Tracking data is separate from this manual shape.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Window {
    /// Stable id within this node, for example `w1`.
    pub id: Id,
    /// Layer Name in the window list.
    pub label: String,
    /// Whether this window contributes to the key.
    pub enabled: bool,
    /// Invert the shape's mask before its opacity is applied.
    pub invert: bool,
    /// Add (Mask on) or subtract (Mask off) this window's contribution.
    pub combine: WindowCombine,
    /// Placement of the local shape in the node's input picture.
    pub transform: WindowTransform,
    /// Contribution to the node's window key, from 0 to 100.
    pub opacity: Param,
    /// The shape and its editable geometry, including an unfinished Curve draft.
    pub shape: WindowShape,
}

impl Window {
    /// A new enabled, additive window, at the centre, with no inversion and full opacity.
    #[must_use]
    pub fn new(id: impl Into<Id>, shape: WindowShape) -> Self {
        Self {
            id: id.into(),
            label: String::new(),
            enabled: true,
            invert: false,
            combine: WindowCombine::Add,
            transform: WindowTransform::default(),
            opacity: bounds::OPACITY.param(),
            shape,
        }
    }

    /// This window as it is on `frame`: every parameter fixed at the value it has there,
    /// interpolated and clamped as the mask uses it, with no keyframes. A window preset is kept
    /// so (author decision 2026-11-01), to be animated or tracked again once it is applied.
    #[must_use]
    pub fn frozen_at(&self, frame: f64) -> Self {
        let fix = |p: &Param, b: Bounds| Param::Fixed(p.value_at(frame).clamp(b.low, b.high));
        let point = |p: &WindowPoint| WindowPoint {
            x: fix(&p.x, bounds::COORDINATE),
            y: fix(&p.y, bounds::COORDINATE),
        };
        let t = &self.transform;
        let shape = match &self.shape {
            WindowShape::Linear { points, softness } => WindowShape::Linear {
                points: Box::new(std::array::from_fn(|i| point(&points[i]))),
                softness: std::array::from_fn(|i| fix(&softness[i], bounds::SOFTNESS)),
            },
            WindowShape::Circular { softness } => WindowShape::Circular {
                softness: fix(softness, bounds::SOFTNESS),
            },
            WindowShape::Gradient { softness } => WindowShape::Gradient {
                softness: fix(softness, bounds::SOFTNESS),
            },
            WindowShape::Polygon {
                points,
                inside,
                outside,
            } => WindowShape::Polygon {
                points: points.iter().map(point).collect(),
                inside: fix(inside, bounds::FEATHER),
                outside: fix(outside, bounds::FEATHER),
            },
            WindowShape::Curve {
                points,
                closed,
                inside,
                outside,
            } => WindowShape::Curve {
                points: points
                    .iter()
                    .map(|p| WindowBezierPoint {
                        position: point(&p.position),
                        into: point(&p.into),
                        out: point(&p.out),
                    })
                    .collect(),
                closed: *closed,
                inside: fix(inside, bounds::FEATHER),
                outside: fix(outside, bounds::FEATHER),
            },
        };
        Self {
            id: self.id.clone(),
            label: self.label.clone(),
            enabled: self.enabled,
            invert: self.invert,
            combine: self.combine,
            transform: WindowTransform {
                size: fix(&t.size, bounds::SIZE),
                aspect: fix(&t.aspect, bounds::ASPECT),
                pan: fix(&t.pan, bounds::PAN),
                tilt: fix(&t.tilt, bounds::TILT),
                rotate: fix(&t.rotate, bounds::ROTATE),
            },
            opacity: fix(&self.opacity, bounds::OPACITY),
            shape,
        }
    }

    fn check(&self) -> Result<(), OpError> {
        if self.id.is_empty() {
            return Err(OpError::NoId);
        }
        if self.id.starts_with('$') {
            return Err(OpError::Dollar {
                id: self.id.clone(),
            });
        }
        check_label(&self.label)?;
        self.transform.check()?;
        bounds::OPACITY.check("windows.opacity", &self.opacity)?;
        self.shape.check()
    }
}

/// Mask on is additive (the default); Mask off subtracts the window. This explicit meaning
/// resolves conflicting descriptions of that toggle on pp. 3190 and 3197 of the manual.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowCombine {
    /// Add the window's mask to the list's key.
    #[default]
    Add,
    /// Subtract the window's mask from the list's key.
    Subtract,
}

/// The Window palette's Transform controls. Local points have their origin at the shape's
/// centre, x to the right and y down, in units of image height; they are not viewer pixels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowTransform {
    /// Uniform size, 50 unchanged.
    pub size: Param,
    /// Aspect, 50 unchanged; increasing widens the shape.
    pub aspect: Param,
    /// Horizontal centre, 50 centred and increasing rightwards.
    pub pan: Param,
    /// Vertical centre, 50 centred and increasing upwards.
    pub tilt: Param,
    /// Clockwise angle in degrees, 0 unchanged.
    pub rotate: Param,
}

impl Default for WindowTransform {
    fn default() -> Self {
        Self {
            size: bounds::SIZE.param(),
            aspect: bounds::ASPECT.param(),
            pan: bounds::PAN.param(),
            tilt: bounds::TILT.param(),
            rotate: bounds::ROTATE.param(),
        }
    }
}

impl WindowTransform {
    /// The affine transform from local coordinates to normalized picture coordinates:
    /// `[a, b, c, d, tx, ty]` means `u = a*x + c*y + tx`, `v = b*x + d*y + ty`.
    /// `image_aspect` is picture width / height, before placement in the viewer. For a clip
    /// this is its source picture; for a timeline it is the canvas. Compose and native
    /// controls share this transform; the viewer applies clip/canvas placement afterwards.
    ///
    /// Our mapping is `s = size/50`, `r = 2^((aspect-50)/50)`, then scale `(s*r, s/r)`,
    /// rotate clockwise, normalize x by `image_aspect`, and move to `(pan/100, 1-tilt/100)`.
    /// This is a project choice, not a published Resolve formula. Animated values are
    /// clamped to their parameter bounds after interpolation, including Bezier overshoot.
    ///
    /// A zero Size returns a matrix collapsed to the centre. Its raw mask is empty, before
    /// Invert, Opacity and Add/Subtract; an inverted collapsed window therefore covers the
    /// picture. An unfinished Curve or a disabled window is ignored instead, even inverted.
    /// Non-finite frames, invalid aspect ratios or unrepresentable matrices return `None`.
    #[must_use]
    pub fn affine_at(&self, frame: f64, image_aspect: f64) -> Option<[f64; 6]> {
        if !frame.is_finite() || !image_aspect.is_finite() || image_aspect <= 0.0 {
            return None;
        }
        let value =
            |param: &Param, bounds: Bounds| param.value_at(frame).clamp(bounds.low, bounds.high);
        let size = value(&self.size, bounds::SIZE) / 50.0;
        let aspect = ((value(&self.aspect, bounds::ASPECT) - 50.0) / 50.0).exp2();
        let (sin, cos) = value(&self.rotate, bounds::ROTATE).to_radians().sin_cos();
        let (x, y) = (size * aspect, size / aspect);
        let matrix = [
            cos * x / image_aspect,
            sin * x,
            -sin * y / image_aspect,
            cos * y,
            value(&self.pan, bounds::PAN) / 100.0,
            1.0 - value(&self.tilt, bounds::TILT) / 100.0,
        ];
        matrix.iter().all(|v| v.is_finite()).then_some(matrix)
    }

    /// The inverse of [`Self::affine_at`], in the same six-coefficient order. A zero Size
    /// or another non-invertible/unrepresentable transform returns `None`; the raw mask is
    /// empty, and the UI can still draw its collapsed centre using the forward transform.
    #[must_use]
    pub fn inverse_at(&self, frame: f64, image_aspect: f64) -> Option<[f64; 6]> {
        let [xx, yx, xy, yy, tx, ty] = self.affine_at(frame, image_aspect)?;
        let determinant = xx * yy - yx * xy;
        if determinant == 0.0 {
            return None;
        }
        let inverse =
            [yy, -yx, -xy, xx, xy * ty - yy * tx, yx * tx - xx * ty].map(|v| v / determinant);
        inverse.iter().all(|v| v.is_finite()).then_some(inverse)
    }

    fn check(&self) -> Result<(), OpError> {
        bounds::SIZE.check("windows.transform.size", &self.size)?;
        bounds::ASPECT.check("windows.transform.aspect", &self.aspect)?;
        bounds::PAN.check("windows.transform.pan", &self.pan)?;
        bounds::TILT.check("windows.transform.tilt", &self.tilt)?;
        bounds::ROTATE.check("windows.transform.rotate", &self.rotate)
    }
}

/// An animatable point in window-local coordinates, one unit equal to the image's height.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowPoint {
    /// Rightwards coordinate.
    pub x: Param,
    /// Downwards coordinate.
    pub y: Param,
}

impl WindowPoint {
    /// A point that is fixed on every frame.
    #[must_use]
    pub fn new(x: f64, y: f64) -> Self {
        Self {
            x: x.into(),
            y: y.into(),
        }
    }

    fn check(&self) -> Result<(), OpError> {
        bounds::COORDINATE.check("windows.points.x", &self.x)?;
        bounds::COORDINATE.check("windows.points.y", &self.y)
    }
}

/// A Curve anchor and its Bezier handles. Handles are offsets from the anchor, not absolute
/// points. Both zero makes a corner; independent handles preserve an Alt-dragged handle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowBezierPoint {
    /// The anchor position.
    pub position: WindowPoint,
    /// The incoming handle, relative to the anchor.
    #[serde(rename = "in")]
    pub into: WindowPoint,
    /// The outgoing handle, relative to the anchor.
    pub out: WindowPoint,
}

/// Shape-specific geometry. The point order is kept exactly as edited. A self-intersecting
/// outline uses the even-odd fill rule; topology changes are whole-list edits, not keyframes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WindowShape {
    /// Four corners, initially top-left, top-right, bottom-right, bottom-left.
    Linear {
        /// Corner pinning keeps these four independent positions.
        points: Box<[WindowPoint; 4]>,
        /// Soft 1–4 for edges 0→1, 1→2, 2→3, 3→0, respectively.
        softness: [Param; 4],
    },
    /// A circle of radius [`DEFAULT_EXTENT`], made elliptical by Transform Aspect.
    Circular {
        /// Soft 1.
        softness: Param,
    },
    /// A closed polygon, with 3 to 128 points and independent inside/outside feathering.
    Polygon {
        /// Vertices in boundary order.
        points: Vec<WindowPoint>,
        /// Softness inside the boundary.
        inside: Param,
        /// Softness outside the boundary.
        outside: Param,
    },
    /// A cubic Bezier path. An open path is a saved drawing draft, ignored by key generation.
    Curve {
        /// Anchors in boundary order. There is no Polygon-style 128-point limit.
        points: Vec<WindowBezierPoint>,
        /// Only a closed path with at least two anchors contributes a mask.
        closed: bool,
        /// Softness inside the boundary.
        inside: Param,
        /// Softness outside the boundary.
        outside: Param,
    },
    /// A half-plane gradient through the local origin, white on negative local y, black on
    /// positive local y. Transform controls its centre and clockwise direction.
    Gradient {
        /// Soft 1, the transition around the boundary.
        softness: Param,
    },
}

fn rectangle() -> [WindowPoint; 4] {
    let r = DEFAULT_EXTENT;
    [
        WindowPoint::new(-r, -r),
        WindowPoint::new(r, -r),
        WindowPoint::new(r, r),
        WindowPoint::new(-r, r),
    ]
}

impl WindowShape {
    /// The initial four-corner Linear window.
    #[must_use]
    pub fn linear() -> Self {
        Self::Linear {
            points: Box::new(rectangle()),
            softness: std::array::from_fn(|_| bounds::SOFTNESS.param()),
        }
    }

    /// The initial Circular window.
    #[must_use]
    pub fn circular() -> Self {
        Self::Circular {
            softness: bounds::SOFTNESS.param(),
        }
    }

    /// The initial Polygon window, a rectangle whose edges may receive more points.
    #[must_use]
    pub fn polygon() -> Self {
        Self::Polygon {
            points: rectangle().into(),
            inside: bounds::FEATHER.param(),
            outside: bounds::FEATHER.param(),
        }
    }

    /// An empty Curve drawing draft. Closing it requires at least two anchors.
    #[must_use]
    pub fn curve() -> Self {
        Self::Curve {
            points: vec![],
            closed: false,
            inside: bounds::FEATHER.param(),
            outside: bounds::FEATHER.param(),
        }
    }

    /// The initial Gradient window.
    #[must_use]
    pub fn gradient() -> Self {
        Self::Gradient {
            softness: bounds::SOFTNESS.param(),
        }
    }

    /// Whether this is an unfinished Curve drawing, which must not affect the node's key,
    /// even when its Invert toggle is on.
    #[must_use]
    pub fn is_draft(&self) -> bool {
        matches!(self, Self::Curve { closed: false, .. })
    }

    fn check(&self) -> Result<(), OpError> {
        match self {
            Self::Linear { points, softness } => {
                for point in points {
                    point.check()?;
                }
                for soft in softness {
                    bounds::SOFTNESS.check("windows.softness", soft)?;
                }
            }
            Self::Circular { softness } | Self::Gradient { softness } => {
                bounds::SOFTNESS.check("windows.softness", softness)?;
            }
            Self::Polygon {
                points,
                inside,
                outside,
            } => {
                if !(3..=MOST_POLYGON_POINTS).contains(&points.len()) {
                    return Err(OpError::OutOfRange {
                        what: "windows.polygon.points",
                    });
                }
                for point in points {
                    point.check()?;
                }
                check_feather(inside, outside)?;
            }
            Self::Curve {
                points,
                closed,
                inside,
                outside,
            } => {
                if *closed && points.len() < 2 {
                    return Err(OpError::OutOfRange {
                        what: "windows.curve.points",
                    });
                }
                for point in points {
                    point.position.check()?;
                    point.into.check()?;
                    point.out.check()?;
                }
                check_feather(inside, outside)?;
            }
        }
        Ok(())
    }
}

fn check_feather(inside: &Param, outside: &Param) -> Result<(), OpError> {
    bounds::FEATHER.check("windows.inside", inside)?;
    bounds::FEATHER.check("windows.outside", outside)
}

pub(super) fn check_windows(windows: &[Window]) -> Result<(), OpError> {
    let mut ids = HashSet::new();
    for window in windows {
        window.check()?;
        if !ids.insert(&window.id) {
            return Err(OpError::Taken {
                what: "window",
                id: window.id.clone(),
            });
        }
    }
    Ok(())
}
