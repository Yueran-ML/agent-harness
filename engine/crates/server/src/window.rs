//! Immutable Window selection and evaluated geometry. This module never locks session State.
use std::sync::Arc;

use columbina_compose::grade::curve;
use columbina_model::grade::{
    window::{bounds, topology::TopologyError, MOST_POLYGON_POINTS},
    Bounds, Corrector, NodeKind, Param, Target, Window, WindowBezierPoint, WindowPoint,
    WindowShape,
};
use columbina_model::{Project, Timeline};
use columbina_view::window_overlay::{
    Affine, Bezier, Control, Shape, ShapeKind, Style, WindowOverlay, AXES, BOX_PAD, CORNERS,
    SOFT_NORMAL,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::presentation::FrameSnapshot;
use crate::rpc::{self, Fault};
use crate::view::Monitor;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub monitor: Monitor,
    pub revision: u64,
    pub timeline: String,
    pub target: Target,
    pub clip: Option<String>,
    pub node: String,
    pub selected: Option<String>,
    pub visible: bool,
    pub outline: bool,
    pub tool: Tool,
    #[serde(skip)]
    pub binding: u64,
    /// The selected Curve's selected anchors (RM20 p. 3195). Pure selection: neither part of the
    /// project nor of its history, and never sent by a client.
    #[serde(skip)]
    pub chosen: Option<Chosen>,
    /// A Shift-drag's band `[x0, y0, x1, y1]` in canvas pixels while it selects.
    #[serde(skip)]
    pub band: Option<[f64; 4]>,
}

/// Anchors selected on one Curve, kept while that Curve keeps `count` anchors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chosen {
    pub window: String,
    pub count: usize,
    /// Ascending anchor indices, at least one.
    pub points: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tool {
    Window,
    ImageWipe,
    None,
}

/// Clears have a sequence too, so delayed commands cannot revive an old selection.
#[derive(Clone, Default)]
pub struct Update {
    pub sequence: u64,
    pub selection: Option<Arc<Selection>>,
}

pub fn refused(reason: &str) -> Fault {
    Fault::new(
        rpc::REFUSED,
        format!("Window input is no longer valid: {reason}"),
    )
    .with_data(json!({"reason":reason}))
}

/// A refused whole-animation Polygon/Curve edit, with what the desktop needs to explain it.
/// `points` are 0-based point indices; `frames` is the key interval in `clock` frames.
pub fn topology_refused(reason: &str, error: &TopologyError, clock: &str) -> Fault {
    let detail = match error {
        TopologyError::Maximum => json!({"why":"maximum","most":MOST_POLYGON_POINTS}),
        TopologyError::Minimum { shape, least } => {
            json!({"why":"minimum","least":least,"shape":shape})
        }
        TopologyError::Unsupported => json!({"why":"unsupported"}),
        TopologyError::Shape {
            axis,
            points,
            why,
            frames,
        } => json!({"why":why,"axis":axis,"points":points,"frames":frames,"clock":clock}),
    };
    Fault::new(
        rpc::REFUSED,
        format!("Window input is no longer valid: {reason}"),
    )
    .with_data(json!({"reason":reason,"topology":detail}))
}

/// Why a down could not capture a control.
#[derive(Debug)]
pub enum Refusal {
    /// The hit no longer names a supported, current control.
    Target,
    /// A Polygon/Curve edge insertion that the model refused as a whole, with the keys' clock.
    Insert(TopologyError, &'static str),
}

/// What the down itself held, beyond its position: Alt, and a system double click.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub alt: bool,
    pub double: bool,
}

impl Selection {
    /// The selected anchors of the selected window, if it has any.
    #[must_use]
    pub fn points(&self) -> &[u32] {
        self.chosen
            .as_ref()
            .filter(|c| self.selected.as_ref() == Some(&c.window))
            .map_or(&[], |c| c.points.as_slice())
    }

    pub fn timeline<'a>(&self, project: &'a Project) -> Option<&'a Timeline> {
        project.timelines.iter().find(|t| t.id == self.timeline)
    }

    pub fn corrector<'a>(&self, project: &'a Project) -> Option<&'a Corrector> {
        let timeline = self.timeline(project)?;
        let clip = self.clip.as_ref().and_then(|id| {
            timeline
                .tracks
                .iter()
                .flat_map(|t| &t.clips)
                .find(|c| &c.id == id)
        });
        let grade = match &self.target {
            Target::Timeline { timeline: id } if id == &timeline.id => &timeline.grade,
            Target::Clip {
                timeline: id,
                clip: id2,
            } if id == &timeline.id && self.clip.as_ref() == Some(id2) => &clip?.grade,
            Target::PreClip { group } | Target::PostClip { group }
                if clip?.group.as_ref() == Some(group) =>
            {
                let group = project.groups.iter().find(|g| &g.id == group)?;
                if matches!(self.target, Target::PreClip { .. }) {
                    &group.pre
                } else {
                    &group.post
                }
            }
            _ => return None,
        };
        if !grade.is_complete() {
            return None;
        }
        let node = grade.node(&self.node)?;
        if !node.enabled || node.locked {
            return None;
        }
        match &node.kind {
            NodeKind::Corrector(c) => Some(c),
            _ => None,
        }
    }

    pub fn validate(&self, project: &Project, revision: u64) -> Result<(), Fault> {
        if self.monitor != Monitor::Timeline {
            return Err(refused("monitor"));
        }
        if self.revision != revision {
            return Err(refused("stale_revision"));
        }
        let c = self.corrector(project).ok_or_else(|| refused("target"))?;
        if self
            .selected
            .as_ref()
            .is_some_and(|id| !c.windows.iter().any(|w| &w.id == id))
        {
            return Err(refused("window"));
        }
        Ok(())
    }

    /// Coordinates and clock of the actual retained target layer, after Highlight.
    pub fn placement(&self, frame: &FrameSnapshot) -> Option<(Affine, (u32, u32), i64)> {
        if !frame.editable()
            || frame.revision != self.revision
            || frame.project.timelines[frame.timeline].id != self.timeline
        {
            return None;
        }
        if frame
            .highlight
            .as_ref()
            .is_some_and(|h| h.grade != self.target || h.node != self.node)
        {
            return None;
        }
        self.corrector(&frame.project)?;
        if matches!(self.target, Target::Timeline { .. }) {
            return Some((
                Affine {
                    xx: f64::from(frame.canvas.0),
                    xy: 0.0,
                    yx: 0.0,
                    yy: f64::from(frame.canvas.1),
                    tx: 0.0,
                    ty: 0.0,
                },
                frame.canvas,
                frame.timeline_frame,
            ));
        }
        let layer = frame
            .layers
            .iter()
            .find(|l| Some(&l.clip) == self.clip.as_ref() && l.editable)?;
        let [xx, xy, yx, yy, tx, ty] = layer.picture_to_canvas;
        Some((
            Affine {
                xx,
                xy,
                yx,
                yy,
                tx,
                ty,
            },
            layer.shown_size,
            layer.origin.key_frame,
        ))
    }

    #[allow(
        clippy::cast_precision_loss,
        reason = "the model evaluates keyframes as f64"
    )]
    pub fn overlay(
        &self,
        frame: &FrameSnapshot,
        coverage: Option<columbina_compose::wipe::Wipe>,
    ) -> Option<Arc<WindowOverlay>> {
        if !self.visible || !self.outline || self.tool != Tool::Window {
            return None;
        }
        let (placement, size, at) = self.placement(frame)?;
        let c = self.corrector(&frame.project)?;
        let shapes = c
            .windows
            .iter()
            .filter(|w| w.enabled)
            .filter_map(|w| shape_at(w, crate::tracking::placed(placement, w, at), size, at))
            .collect();
        Some(Arc::new(WindowOverlay {
            shapes,
            selected: self.selected.clone(),
            style: Style::default(),
            coverage,
            points: self.points().to_vec(),
            band: self.band,
        }))
    }
}

/// A window's controls geometry at clock frame `at`, for the overlay and for a capture: the
/// same evaluated, clamped values the mask uses, placed on the canvas.
#[allow(
    clippy::cast_precision_loss,
    clippy::too_many_lines,
    reason = "the model evaluates keyframes as f64; one table of the five shapes"
)]
pub fn shape_at(w: &Window, placement: Affine, size: (u32, u32), at: i64) -> Option<Shape> {
    let value = |p: &Param, b: Bounds| p.value_at(at as f64).clamp(b.low, b.high);
    let point = |p: &WindowPoint| {
        [
            value(&p.x, bounds::COORDINATE),
            value(&p.y, bounds::COORDINATE),
        ]
    };
    let transform = w
        .transform
        .affine_at(at as f64, f64::from(size.0) / f64::from(size.1))?;
    // Linear Soft 1-4 span soft/200 across each side; Polygon/Curve inside/outside soft/200.
    let (kind, feather, feathers, bezier) = match &w.shape {
        WindowShape::Circular { softness } => (
            ShapeKind::Circular,
            value(softness, bounds::SOFTNESS) / 400.0,
            vec![],
            None,
        ),
        WindowShape::Gradient { softness } => (
            ShapeKind::Gradient,
            value(softness, bounds::SOFTNESS) / 400.0,
            vec![],
            None,
        ),
        WindowShape::Linear { softness, .. } => (
            ShapeKind::Linear,
            0.0,
            softness
                .iter()
                .map(|s| [value(s, bounds::SOFTNESS) / 400.0; 2])
                .collect(),
            None,
        ),
        WindowShape::Polygon {
            points,
            inside,
            outside,
        } => (
            ShapeKind::Polygon,
            0.0,
            vec![
                [
                    value(inside, bounds::FEATHER) / 200.0,
                    value(outside, bounds::FEATHER) / 200.0
                ];
                points.len()
            ],
            None,
        ),
        WindowShape::Curve {
            points,
            closed,
            inside,
            outside,
        } => {
            // The mask's own tessellation tolerance, from the transform at unit image aspect.
            let unit = w.transform.affine_at(at as f64, 1.0)?;
            let scale = unit[0].hypot(unit[1]).max(unit[2].hypot(unit[3]));
            (
                ShapeKind::Curve,
                0.0,
                vec![[
                    value(inside, bounds::FEATHER) / 200.0,
                    value(outside, bounds::FEATHER) / 200.0,
                ]],
                Some(Bezier {
                    handles: points
                        .iter()
                        .map(|p| [point(&p.into), point(&p.out)])
                        .collect(),
                    closed: *closed,
                    tolerance: if scale > 0.0 {
                        curve::tolerance(scale)
                    } else {
                        0.0
                    },
                }),
            )
        }
    };
    let [xx, yx, xy, yy, tx, ty] = transform;
    Some(Shape {
        id: w.id.clone(),
        radius: columbina_model::grade::window::DEFAULT_EXTENT,
        kind,
        feather,
        points: match &w.shape {
            WindowShape::Curve { points, .. } => {
                points.iter().map(|p| point(&p.position)).collect()
            }
            shape => outline_at(shape, at),
        },
        feathers,
        curve: bezier,
        to_canvas: compose(
            placement,
            Affine {
                xx,
                xy,
                yx,
                yy,
                tx,
                ty,
            },
        ),
    })
}

pub fn compose(a: Affine, b: Affine) -> Affine {
    Affine {
        xx: a.xx * b.xx + a.xy * b.yx,
        xy: a.xx * b.xy + a.xy * b.yy,
        yx: a.yx * b.xx + a.yy * b.yx,
        yy: a.yx * b.xy + a.yy * b.yy,
        tx: a.xx * b.tx + a.xy * b.ty + a.tx,
        ty: a.yx * b.tx + a.yy * b.ty + a.ty,
    }
}

pub fn unplace(a: Affine, point: (f64, f64)) -> Option<(f64, f64)> {
    let det = a.xx * a.yy - a.xy * a.yx;
    let (x, y) = (point.0 - a.tx, point.1 - a.ty);
    let p = ((a.yy * x - a.xy * y) / det, (-a.yx * x + a.xx * y) / det);
    (p.0.is_finite() && p.1.is_finite()).then_some(p)
}

/// Chosen once at down, independent of later interpolation and pointer positions.
#[derive(Clone, Copy, Debug)]
pub struct LeftKey {
    index: Option<usize>,
    pub frame: Option<i64>,
    value: f64,
}
impl LeftKey {
    pub fn of(param: &Param, at: i64) -> Self {
        match param {
            Param::Fixed(value) => Self {
                index: None,
                frame: None,
                value: *value,
            },
            Param::Keys(keys) => {
                let index = keys
                    .partition_point(|key| key.frame <= at)
                    .saturating_sub(1);
                Self {
                    index: Some(index),
                    frame: Some(keys[index].frame),
                    value: keys[index].value,
                }
            }
        }
    }
    fn write(self, param: &mut Param, delta: f64, bounds: Bounds) {
        let next = (self.value + delta).clamp(bounds.low, bounds.high);
        match (param, self.index) {
            (Param::Fixed(value), None) => *value = next,
            (Param::Keys(keys), Some(index)) => keys[index].value = next,
            _ => unreachable!("original whole array preserves key structure"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Size,
    Aspect,
    Pan,
    Tilt,
    Rotate,
    Softness,
    /// A Linear corner or Polygon point's local coordinates.
    X(usize),
    Y(usize),
    /// Linear Soft 1–4, for side `j` from point `j` to the next.
    EdgeSoftness(usize),
    /// A Curve anchor's incoming and outgoing Bezier handle, relative to it.
    InX(usize),
    InY(usize),
    OutX(usize),
    OutY(usize),
    /// A Curve's Inside and Outside Softness.
    Inside,
    Outside,
}
impl Field {
    fn name(self) -> &'static str {
        match self {
            Self::Size => "size",
            Self::Aspect => "aspect",
            Self::Pan => "pan",
            Self::Tilt => "tilt",
            Self::Rotate => "rotate",
            Self::Softness | Self::EdgeSoftness(_) => "softness",
            Self::X(_) => "x",
            Self::Y(_) => "y",
            Self::InX(_) => "in_x",
            Self::InY(_) => "in_y",
            Self::OutX(_) => "out_x",
            Self::OutY(_) => "out_y",
            Self::Inside => "inside",
            Self::Outside => "outside",
        }
    }
    /// The Curve anchor or Polygon/Linear point whose coordinates this is.
    fn point(self) -> Option<usize> {
        match self {
            Self::X(i)
            | Self::Y(i)
            | Self::InX(i)
            | Self::InY(i)
            | Self::OutX(i)
            | Self::OutY(i) => Some(i),
            _ => None,
        }
    }
    fn bounds(self) -> Bounds {
        match self {
            Self::Size => bounds::SIZE,
            Self::Aspect => bounds::ASPECT,
            Self::Pan => bounds::PAN,
            Self::Tilt => bounds::TILT,
            Self::Rotate => bounds::ROTATE,
            Self::Softness | Self::EdgeSoftness(_) => bounds::SOFTNESS,
            Self::X(_)
            | Self::Y(_)
            | Self::InX(_)
            | Self::InY(_)
            | Self::OutX(_)
            | Self::OutY(_) => bounds::COORDINATE,
            Self::Inside | Self::Outside => bounds::FEATHER,
        }
    }
    fn param(self, window: &mut Window) -> &mut Param {
        match (self, &mut window.shape) {
            (Self::Size, _) => &mut window.transform.size,
            (Self::Aspect, _) => &mut window.transform.aspect,
            (Self::Pan, _) => &mut window.transform.pan,
            (Self::Tilt, _) => &mut window.transform.tilt,
            (Self::Rotate, _) => &mut window.transform.rotate,
            (
                Self::Softness,
                WindowShape::Circular { softness } | WindowShape::Gradient { softness },
            ) => softness,
            (Self::EdgeSoftness(j), WindowShape::Linear { softness, .. }) => &mut softness[j],
            (Self::X(i), WindowShape::Linear { points, .. }) => &mut points[i].x,
            (Self::Y(i), WindowShape::Linear { points, .. }) => &mut points[i].y,
            (Self::X(i), WindowShape::Polygon { points, .. }) => &mut points[i].x,
            (Self::Y(i), WindowShape::Polygon { points, .. }) => &mut points[i].y,
            (Self::X(i), WindowShape::Curve { points, .. }) => &mut points[i].position.x,
            (Self::Y(i), WindowShape::Curve { points, .. }) => &mut points[i].position.y,
            (Self::InX(i), WindowShape::Curve { points, .. }) => &mut points[i].into.x,
            (Self::InY(i), WindowShape::Curve { points, .. }) => &mut points[i].into.y,
            (Self::OutX(i), WindowShape::Curve { points, .. }) => &mut points[i].out.x,
            (Self::OutY(i), WindowShape::Curve { points, .. }) => &mut points[i].out.y,
            (Self::Inside, WindowShape::Curve { inside, .. }) => inside,
            (Self::Outside, WindowShape::Curve { outside, .. }) => outside,
            _ => unreachable!("captured supported shape"),
        }
    }
}

/// Local Linear/Polygon outline points at the captured frame, clamped as they are drawn.
#[allow(
    clippy::cast_precision_loss,
    reason = "the model evaluates keyframes as f64"
)]
fn outline_at(shape: &WindowShape, at: i64) -> Vec<[f64; 2]> {
    let points: &[WindowPoint] = match shape {
        WindowShape::Linear { points, .. } => points.as_ref(),
        WindowShape::Polygon { points, .. } => points,
        _ => &[],
    };
    let value = |p: &Param| {
        p.value_at(at as f64)
            .clamp(bounds::COORDINATE.low, bounds::COORDINATE.high)
    };
    points.iter().map(|p| [value(&p.x), value(&p.y)]).collect()
}

/// The outward local unit normal of `edge`, by the outline's winding; zero if degenerate.
fn outward(points: &[[f64; 2]], edge: usize) -> [f64; 2] {
    let count = points.len();
    let twice_area: f64 = (0..count)
        .map(|i| {
            let (from, to) = (points[i], points[(i + 1) % count]);
            from[0] * to[1] - to[0] * from[1]
        })
        .sum();
    let sign = if twice_area < 0.0 { -1.0 } else { 1.0 };
    let (from, to) = (points[edge], points[(edge + 1) % count]);
    let along = [to[0] - from[0], to[1] - from[1]];
    let length = along[0].hypot(along[1]);
    if length > 0.0 && length.is_finite() {
        [sign * along[1] / length, -sign * along[0] / length]
    } else {
        [0.0; 2]
    }
}

/// The angle is measured before Window Aspect, in oriented picture-height units.
/// Native evidence keeps this immutable basis while observed Moves are coalesced.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RotationBasis {
    placement: Affine,
    centre: (f64, f64),
    aspect: f64,
}
impl RotationBasis {
    #[allow(
        clippy::cast_precision_loss,
        reason = "the model evaluates keyframes as f64"
    )]
    pub fn of(selection: &Selection, frame: &FrameSnapshot, id: &str) -> Option<Self> {
        let (placement, picture, at) = selection.placement(frame)?;
        let window = selection
            .corrector(&frame.project)?
            .windows
            .iter()
            .find(|w| w.id == id)?;
        let aspect = f64::from(picture.0) / f64::from(picture.1);
        let [_, _, _, _, x, y] = window.transform.affine_at(at as f64, aspect)?;
        Some(Self {
            placement: crate::tracking::placed(placement, window, at),
            centre: (x, y),
            aspect,
        })
    }
    pub fn angle(self, canvas: (f64, f64)) -> Option<f64> {
        let (x, y) = unplace(self.placement, canvas)?;
        let (x, y) = ((x - self.centre.0) * self.aspect, y - self.centre.1);
        (x.hypot(y) > 1e-12).then(|| y.atan2(x))
    }
    /// The angle at a native pixel centre. Within one device pixel of the centre the
    /// direction is noise (a pass near it could add an arbitrary half turn), so it is unread.
    pub fn angle_at(self, map: columbina_view::ViewportMap, point: (i32, i32)) -> Option<f64> {
        let p = self.placement;
        let (x, y) = self.centre;
        let centre = map.to_device((p.xx * x + p.xy * y + p.tx, p.yx * x + p.yy * y + p.ty));
        let device = (f64::from(point.0) + 0.5, f64::from(point.1) + 0.5);
        if (device.0 - centre.0).hypot(device.1 - centre.1) < 1.0 {
            return None;
        }
        self.angle(map.on_picture(point).0)
    }
}

/// Device pixels (an anchor's radius) a Curve pull must travel on screen before it gives a point
/// Bezier handles, so a click that wobbles still adds or closes with a corner (p. 3193) at any
/// viewer zoom. A project choice.
const PULL_THRESHOLD: f64 = 4.0;

/// How a captured Curve control turns the pointer into its fields (RM20 p. 3194).
#[derive(Clone, Copy, Debug, PartialEq)]
enum CurveDrag {
    /// Pull symmetric handles out of point `i`: its out handle follows the pointer and its in
    /// handle mirrors it. Used by drawing a new point, closing on the first point, and an
    /// Alt-drag on a corner. Nearer the down than `PULL_THRESHOLD` nothing is pulled.
    Pull(usize),
    /// Drag one handle of point `i` (`out` or in). Linked: the opposite handle turns and scales
    /// with it by the same similarity, keeping their relative angle and length ratio; with Alt
    /// it stays where it is. A zero opposite handle stays zero; a zero dragged handle at down
    /// has no direction, so the opposite handle is kept.
    Handle {
        point: usize,
        out: bool,
        linked: bool,
    },
}

/// Device pixels about a band within which an anchor's centre counts as enclosed (an anchor's
/// radius), so that a Shift click on one anchor selects it. A project choice.
const BAND_REACH: f64 = 4.0;

/// A drag of the box about two or more selected Curve anchors (RM20 p. 3195), from what was
/// displayed at down: the box centre in canvas pixels, about which it scales, stretches and
/// rotates, and per selected anchor its index, local position and `[in, out]` handles.
struct Multi {
    centre: (f64, f64),
    anchors: Vec<(usize, [f64; 2], [[f64; 2]; 2])>,
}

pub struct Movement {
    /// The complete list at down, restored by Esc.
    pub original: Vec<Window>,
    /// What every update starts from: `original`, or it with the down's own edit (a point
    /// inserted or drawn, a Curve closed, or a Curve point made a corner).
    base: Vec<Window>,
    /// Whether `base` differs from `original`, written by the first armed step.
    edited: bool,
    inserted: Option<usize>,
    /// The captured Linear side's outward local normal at down, or the local direction in
    /// which a Curve softness handle increases its softness.
    normal: [f64; 2],
    index: usize,
    keys: Vec<(Field, LeftKey)>,
    bezier: Option<CurveDrag>,
    multi: Option<Multi>,
    /// A Shift-drag's band: every anchor's canvas position as displayed at down.
    band: Option<Vec<(u32, (f64, f64))>>,
    /// Where a band reaches, in canvas pixels: the last position it was given.
    band_to: (f64, f64),
    pub control: Control,
    down: (f64, f64),
    canvas_down: (f64, f64),
    /// Device pixels per canvas pixel on each axis of the map shown at down (a captured drag
    /// keeps that map), so that the pull threshold is measured on screen.
    device: [f64; 2],
    local_down: (f64, f64),
    transform: Affine,
    /// The shown picture size at down, for the Curve geometry of `shape_at`.
    picture: (u32, u32),
    size: f64,
    gradient: bool,
    pub rotation: RotationBasis,
    pub placement: Affine,
    pub clock: &'static str,
    pub frame: i64,
    /// The size of the frames the window's clip is tracked on, for Frame mode to key a window
    /// never tracked (T24).
    analysis: Option<[usize; 2]>,
}
impl Movement {
    pub fn selected(&self) -> &str {
        &self.original[self.index].id
    }
    /// The list with the down's own edit (an inserted or drawn point, a closed Curve, a corner),
    /// written by the first armed step, if this capture made one.
    pub fn edited(&self) -> Option<&[Window]> {
        self.edited.then_some(self.base.as_slice())
    }
    pub fn keys(&self) -> Value {
        let mut keys = serde_json::Map::new();
        let mut points: Vec<(usize, serde_json::Map<String, Value>)> = Vec::new();
        for (field, key) in &self.keys {
            if let Some(i) = field.point() {
                let at = points.iter().position(|(p, _)| *p == i).unwrap_or_else(|| {
                    let mut entry = serde_json::Map::new();
                    entry.insert("point".into(), json!(i));
                    points.push((i, entry));
                    points.len() - 1
                });
                points[at].1.insert(field.name().into(), json!(key.frame));
                continue;
            }
            match *field {
                Field::EdgeSoftness(j) => {
                    keys.insert("softness".into(), json!(key.frame));
                    keys.insert("edge".into(), json!(j));
                }
                _ => {
                    keys.insert(field.name().into(), json!(key.frame));
                }
            }
        }
        if !points.is_empty() {
            keys.insert(
                "points".into(),
                Value::Array(points.into_iter().map(|(_, e)| Value::Object(e)).collect()),
            );
        }
        if let Some(i) = self.inserted {
            keys.insert("inserted".into(), json!(i));
        }
        keys.insert("clock".into(), json!(self.clock));
        keys.insert("frame".into(), json!(self.frame));
        Value::Object(keys)
    }
    pub fn new(
        selection: &Selection,
        frame: &FrameSnapshot,
        id: &str,
        canvas: (f64, f64),
    ) -> Option<Self> {
        Self::capture(
            selection,
            frame,
            id,
            Control::Move,
            canvas,
            [1.0; 2],
            Modifiers::default(),
        )
        .ok()
    }
    /// Device pixels per canvas pixel on each axis of `map`, the map a capture's down was shown
    /// with.
    #[must_use]
    pub fn device_per_canvas(map: columbina_view::ViewportMap) -> [f64; 2] {
        [
            f64::from(map.dest[2]) / f64::from(map.picture.0),
            f64::from(map.dest[3]) / f64::from(map.picture.1),
        ]
    }
    /// Capture one control. A Polygon/Curve `Insert` also inserts its point into `base` for the
    /// whole animation at the clicked spot of the displayed edge, or refuses as a whole. On a
    /// Curve, `Append` draws a point, `Close` closes the draft, and an Alt double click on a
    /// Bezier point makes it a corner, each in `base` too. `device` is
    /// [`Movement::device_per_canvas`] of the map the down was shown with.
    ///
    /// # Errors
    ///
    /// [`Refusal::Insert`] says why the model cannot insert this point exactly.
    pub fn capture(
        selection: &Selection,
        frame: &FrameSnapshot,
        id: &str,
        control: Control,
        canvas: (f64, f64),
        device: [f64; 2],
        modifiers: Modifiers,
    ) -> Result<Self, Refusal> {
        let mut movement = Self::locate(selection, frame, id, control, canvas, modifiers)
            .ok_or(Refusal::Target)?;
        movement.device = device;
        if let Control::Insert(edge) = control {
            movement.insert(edge as usize, canvas)?;
        }
        Ok(movement)
    }
    fn insert(&mut self, edge: usize, canvas: (f64, f64)) -> Result<(), Refusal> {
        let window = &self.original[self.index];
        let inserted = if matches!(window.shape, WindowShape::Curve { .. }) {
            // The exact cubic parameter under the click, on the displayed (down-frame) curve.
            let shape = shape_at(window, self.placement, self.picture, self.frame)
                .ok_or(Refusal::Target)?;
            let t = shape
                .curve_parameter(edge, [canvas.0, canvas.1])
                .ok_or(Refusal::Target)?;
            self.base[self.index].shape.insert_curve_point(edge, t)
        } else {
            let points = outline_at(&window.shape, self.frame);
            let (a, b) = (points[edge], points[(edge + 1) % points.len()]);
            let d = [b[0] - a[0], b[1] - a[1]];
            let q = [self.local_down.0 - a[0], self.local_down.1 - a[1]];
            let length = d[0] * d[0] + d[1] * d[1];
            // The clicked spot on the displayed edge; its ends are point handles.
            let share = if length > 0.0 && length.is_finite() {
                ((q[0] * d[0] + q[1] * d[1]) / length).clamp(1e-3, 1.0 - 1e-3)
            } else {
                0.5
            };
            self.base[self.index]
                .shape
                .insert_polygon_point(edge, share)
        }
        .map_err(|error| Refusal::Insert(error, self.clock))?;
        self.inserted = Some(inserted);
        self.edited = true;
        self.capture_fields(&[Field::X(inserted), Field::Y(inserted)]);
        Ok(())
    }
    /// The left keys of `fields` in `base`, which the drag then writes.
    fn capture_fields(&mut self, fields: &[Field]) {
        self.keys = fields
            .iter()
            .map(|&field| {
                (
                    field,
                    LeftKey::of(field.param(&mut self.base[self.index]), self.frame),
                )
            })
            .collect();
    }
    #[allow(
        clippy::cast_precision_loss,
        clippy::too_many_lines,
        reason = "the model evaluates keyframes as f64; one table of controls to fields"
    )]
    fn locate(
        selection: &Selection,
        frame: &FrameSnapshot,
        id: &str,
        control: Control,
        canvas: (f64, f64),
        modifiers: Modifiers,
    ) -> Option<Self> {
        let (placement, picture, at) = selection.placement(frame)?;
        let original = selection.corrector(&frame.project)?.windows.clone();
        let index = original.iter().position(|w| w.id == id && w.enabled)?;
        // A tracked window is hit and dragged where its track shows it (T24).
        let placement = crate::tracking::placed(placement, &original[index], at);
        let shape = &original[index].shape;
        let gradient = matches!(shape, WindowShape::Gradient { .. });
        let circular = matches!(shape, WindowShape::Circular { .. });
        let linear = matches!(shape, WindowShape::Linear { .. });
        let curve = match shape {
            WindowShape::Curve { points, closed, .. } => Some((points.len(), *closed)),
            _ => None,
        };
        let points = outline_at(shape, at);
        let n = curve.map_or(points.len(), |(count, _)| count);
        let view = shape_at(&original[index], placement, picture, at)?;
        // A Curve anchor's handles as displayed (E): a corner has both at zero.
        let corner = |i: usize| {
            view.curve
                .as_ref()
                .is_some_and(|c| c.handles[i] == [[0.0; 2]; 2])
        };
        let pull = |i: usize| vec![Field::OutX(i), Field::OutY(i), Field::InX(i), Field::InY(i)];
        let mut base = original.clone();
        let mut edited = false;
        let mut inserted = None;
        let mut bezier = None;
        let mut multi = None;
        let mut band = None;
        let mut normal = [0.0; 2];
        let fields: Vec<Field> = match control {
            Control::Move => vec![Field::Pan, Field::Tilt],
            Control::Size(i) if circular && i < 4 => vec![Field::Size],
            Control::Aspect(i) if circular && i < 4 => vec![Field::Aspect],
            Control::Rotate if circular || gradient => vec![Field::Rotate],
            Control::Softness(side) if circular && side == 1 || gradient && side.abs() == 1 => {
                vec![Field::Softness]
            }
            Control::Softness(side) if curve.is_some_and(|(_, closed)| closed) => {
                normal = view
                    .curve_softness_handles()
                    .into_iter()
                    .find(|(c, _, _)| *c == control)?
                    .2;
                vec![if side < 0 {
                    Field::Inside
                } else {
                    Field::Outside
                }]
            }
            // An Alt double click makes a Curve point a corner (p. 3194); a drag then moves it.
            Control::Point(i) if curve.is_some() && modifiers.alt && modifiers.double => {
                let i = i as usize;
                if i >= n {
                    return None;
                }
                for field in pull(i) {
                    let key = LeftKey::of(field.param(&mut base[index]), at);
                    key.write(field.param(&mut base[index]), -key.value, field.bounds());
                }
                edited = base != original;
                vec![Field::X(i), Field::Y(i)]
            }
            // Alt-dragging a corner pulls out Bezier handles (p. 3194).
            Control::Point(i)
                if curve.is_some() && modifiers.alt && (i as usize) < n && corner(i as usize) =>
            {
                bezier = Some(CurveDrag::Pull(i as usize));
                pull(i as usize)
            }
            Control::Point(i) if (i as usize) < n => {
                vec![Field::X(i as usize), Field::Y(i as usize)]
            }
            Control::HandleIn(i) | Control::HandleOut(i) if curve.is_some() && (i as usize) < n => {
                let (i, out) = (i as usize, matches!(control, Control::HandleOut(_)));
                bezier = Some(CurveDrag::Handle {
                    point: i,
                    out,
                    linked: !modifiers.alt,
                });
                // The dragged handle first, then the opposite one.
                if out {
                    vec![Field::OutX(i), Field::OutY(i), Field::InX(i), Field::InY(i)]
                } else {
                    vec![Field::InX(i), Field::InY(i), Field::OutX(i), Field::OutY(i)]
                }
            }
            // Clicking an open Curve's first point closes it with a corner there; dragging on it
            // closes it with a Bezier curve (p. 3193). Its handles' left keys go to 0 at once,
            // and a drag past the pull threshold pulls them out again.
            Control::Close if curve.is_some_and(|(count, closed)| !closed && count >= 2) => {
                if let WindowShape::Curve { closed, .. } = &mut base[index].shape {
                    *closed = true;
                }
                for field in pull(0) {
                    let key = LeftKey::of(field.param(&mut base[index]), at);
                    key.write(field.param(&mut base[index]), -key.value, field.bounds());
                }
                edited = true;
                bezier = Some(CurveDrag::Pull(0));
                pull(0)
            }
            Control::Edge(j) if linear && usize::from(j) < n => {
                let (j, k) = (usize::from(j), (usize::from(j) + 1) % n);
                vec![Field::X(j), Field::Y(j), Field::X(k), Field::Y(k)]
            }
            Control::EdgeSoftness(j) if linear && usize::from(j) < n => {
                normal = outward(&points, usize::from(j));
                vec![Field::EdgeSoftness(usize::from(j))]
            }
            Control::Insert(j)
                if (j as usize) < n
                    && (matches!(shape, WindowShape::Polygon { .. }) || curve.is_some()) =>
            {
                vec![]
            }
            Control::Append if curve.is_some_and(|(_, closed)| !closed) => vec![],
            // A Shift-drag selects the anchors its band encloses; it writes nothing.
            Control::Band if curve.is_some() => {
                let a = view.to_canvas;
                band = Some(
                    view.points
                        .iter()
                        .zip(0..)
                        .map(|(p, i)| {
                            (
                                i,
                                (
                                    a.xx * p[0] + a.xy * p[1] + a.tx,
                                    a.yx * p[0] + a.yy * p[1] + a.ty,
                                ),
                            )
                        })
                        .collect(),
                );
                vec![]
            }
            // The box about the selected anchors: moving writes their positions; scaling,
            // stretching and rotating also turn their handles by the linear part.
            Control::BoxMove
            | Control::BoxCorner(_)
            | Control::BoxEdge(_)
            | Control::BoxRotate(_)
                if curve.is_some() && selection.selected.as_deref() == Some(id) =>
            {
                let chosen: Vec<u32> = selection
                    .points()
                    .iter()
                    .copied()
                    .filter(|&i| (i as usize) < n)
                    .collect();
                let bounds = view.anchor_bounds(&chosen).filter(|_| chosen.len() >= 2)?;
                let handles = &view.curve.as_ref()?.handles;
                multi = Some(Multi {
                    centre: (bounds[0].midpoint(bounds[2]), bounds[1].midpoint(bounds[3])),
                    anchors: chosen
                        .iter()
                        .map(|&i| (i as usize, view.points[i as usize], handles[i as usize]))
                        .collect(),
                });
                chosen
                    .iter()
                    .flat_map(|&i| {
                        let i = i as usize;
                        if control == Control::BoxMove {
                            vec![Field::X(i), Field::Y(i)]
                        } else {
                            vec![
                                Field::X(i),
                                Field::Y(i),
                                Field::InX(i),
                                Field::InY(i),
                                Field::OutX(i),
                                Field::OutY(i),
                            ]
                        }
                    })
                    .collect()
            }
            _ => return None,
        };
        let aspect = f64::from(picture.0) / f64::from(picture.1);
        let [xx, yx, xy, yy, tx, ty] = original[index].transform.affine_at(at as f64, aspect)?;
        let transform = Affine {
            xx,
            yx,
            xy,
            yy,
            tx,
            ty,
        };
        let down = unplace(placement, canvas)?;
        let local_down = unplace(transform, down)?;
        if control == Control::Append {
            // A drawn point lies where it was clicked, static, a corner until pulled.
            let local = [local_down.0, local_down.1];
            let range = bounds::COORDINATE.low..=bounds::COORDINATE.high;
            if local.iter().any(|v| !range.contains(v)) {
                return None;
            }
            let WindowShape::Curve { points, .. } = &mut base[index].shape else {
                return None;
            };
            points.push(WindowBezierPoint {
                position: WindowPoint::new(local[0], local[1]),
                into: WindowPoint::new(0.0, 0.0),
                out: WindowPoint::new(0.0, 0.0),
            });
            edited = true;
            inserted = Some(n);
            bezier = Some(CurveDrag::Pull(n));
        }
        let keys_of = |base: &mut Vec<Window>, fields: &[Field]| -> Vec<(Field, LeftKey)> {
            fields
                .iter()
                .map(|&field| (field, LeftKey::of(field.param(&mut base[index]), at)))
                .collect()
        };
        let keys = if control == Control::Append {
            keys_of(&mut base, &pull(n))
        } else {
            keys_of(&mut base, &fields)
        };
        let size = original[index]
            .transform
            .size
            .value_at(at as f64)
            .clamp(bounds::SIZE.low, bounds::SIZE.high);
        let clock = if matches!(selection.target, Target::Timeline { .. }) {
            "timeline"
        } else {
            let timeline = selection.timeline(&frame.project)?;
            let clip = timeline
                .tracks
                .iter()
                .flat_map(|t| &t.clips)
                .find(|c| Some(&c.id) == selection.clip.as_ref())?;
            if columbina_model::keyframes::KeyClock::of(clip, &frame.project, timeline.rate)
                .counts_source()
            {
                "source"
            } else {
                "clip"
            }
        };
        Some(Self {
            base,
            original,
            edited,
            inserted,
            normal,
            index,
            keys,
            bezier,
            multi,
            band,
            band_to: canvas,
            control,
            down,
            canvas_down: canvas,
            device: [1.0; 2],
            local_down,
            transform,
            picture,
            size,
            gradient,
            rotation: RotationBasis {
                placement,
                centre: (tx, ty),
                aspect,
            },
            placement,
            clock,
            frame: at,
            analysis: crate::tracking::analysis_of(&frame.project, &selection.target),
        })
    }
    pub fn at(&self, canvas: (f64, f64)) -> Option<Vec<Window>> {
        self.at_rotation(canvas, None)
    }
    /// The values a Curve drag gives its fields, as absolute targets, in `keys` order.
    fn bezier_targets(&self, canvas: (f64, f64), delta: [f64; 2]) -> Vec<Option<f64>> {
        let value = |k: usize| self.keys[k].1.value;
        match self.bezier {
            None => vec![None; self.keys.len()],
            Some(CurveDrag::Pull(_)) => {
                // In device pixels: the same threshold on screen at any zoom.
                let travelled = ((canvas.0 - self.canvas_down.0) * self.device[0])
                    .hypot((canvas.1 - self.canvas_down.1) * self.device[1]);
                if travelled < PULL_THRESHOLD {
                    // Not pulled yet: every field keeps its value in `base`.
                    return (0..self.keys.len()).map(|k| Some(value(k))).collect();
                }
                // [out x, out y, in x, in y]
                vec![
                    Some(delta[0]),
                    Some(delta[1]),
                    Some(-delta[0]),
                    Some(-delta[1]),
                ]
            }
            Some(CurveDrag::Handle { linked, .. }) => {
                // [dragged x, dragged y, opposite x, opposite y]
                let old = [value(0), value(1)];
                let new = [old[0] + delta[0], old[1] + delta[1]];
                let other = [value(2), value(3)];
                let length = old[0] * old[0] + old[1] * old[1];
                let other = if linked && length > 0.0 && length.is_finite() {
                    // other · (new / old), as complex numbers: one rotation and scale.
                    let ratio = [
                        (new[0] * old[0] + new[1] * old[1]) / length,
                        (new[1] * old[0] - new[0] * old[1]) / length,
                    ];
                    [
                        other[0] * ratio[0] - other[1] * ratio[1],
                        other[0] * ratio[1] + other[1] * ratio[0],
                    ]
                } else {
                    other
                };
                vec![Some(new[0]), Some(new[1]), Some(other[0]), Some(other[1])]
            }
        }
    }
    /// How a box corner, side or rotation drag changes each captured field, in `keys` order: the
    /// canvas map about the box centre, conjugated to local units, applied in full to the
    /// displayed anchors and by its linear part to their relative handles.
    #[allow(
        clippy::many_single_char_names,
        reason = "the canvas map and the vectors from the centre in plane notation"
    )]
    fn multi_deltas(&self, canvas: (f64, f64)) -> Option<Vec<f64>> {
        let multi = self.multi.as_ref()?;
        let c = multi.centre;
        let u = (self.canvas_down.0 - c.0, self.canvas_down.1 - c.1);
        let v = (canvas.0 - c.0, canvas.1 - c.1);
        // In device pixels, where the box stands `BOX_PAD` off the anchors: that part of the
        // down offset does not scale, so the handle stays under the pointer. A down offset
        // reaching less than one device pixel past the pad scales as if it reached one.
        let (ud, vd) = (
            (u.0 * self.device[0], u.1 * self.device[1]),
            (v.0 * self.device[0], v.1 * self.device[1]),
        );
        let share =
            |from: f64, to: f64, inset: f64| (1.0 + (to - from) / (from - inset).max(1.0)).max(0.0);
        // [a, b, c, d]: x' = a x + b y, y' = c x + d y about the centre, in canvas pixels.
        let a = match self.control {
            // Uniform: the pointer's progress along its down offset.
            Control::BoxCorner(corner) => {
                let from = ud.0.hypot(ud.1);
                let k = if from > 1e-9 {
                    let sign = CORNERS[usize::from(corner) % 4];
                    share(
                        from,
                        (vd.0 * ud.0 + vd.1 * ud.1) / from,
                        BOX_PAD * (sign[0] * ud.0 + sign[1] * ud.1) / from,
                    )
                } else {
                    1.0
                };
                [k, 0.0, 0.0, k]
            }
            Control::BoxEdge(side) => {
                let axis = AXES[usize::from(side) % 4];
                let k = share(
                    ud.0 * axis[0] + ud.1 * axis[1],
                    vd.0 * axis[0] + vd.1 * axis[1],
                    BOX_PAD,
                );
                if side < 2 {
                    [k, 0.0, 0.0, 1.0]
                } else {
                    [1.0, 0.0, 0.0, k]
                }
            }
            Control::BoxRotate(_) => {
                let turn = if u.0.hypot(u.1) > 1e-9 && v.0.hypot(v.1) > 1e-9 {
                    v.1.atan2(v.0) - u.1.atan2(u.0)
                } else {
                    0.0
                };
                let (sin, cos) = turn.sin_cos();
                [cos, -sin, sin, cos]
            }
            _ => return None,
        };
        let place = compose(self.placement, self.transform);
        let linear = Affine {
            tx: 0.0,
            ty: 0.0,
            ..place
        };
        let map = |p: (f64, f64)| (a[0] * p.0 + a[1] * p.1, a[2] * p.0 + a[3] * p.1);
        let mut out = Vec::with_capacity(self.keys.len());
        for &(_, p, handles) in &multi.anchors {
            let at = (
                place.xx * p[0] + place.xy * p[1] + place.tx,
                place.yx * p[0] + place.yy * p[1] + place.ty,
            );
            let moved = map((at.0 - c.0, at.1 - c.1));
            let q = unplace(place, (c.0 + moved.0, c.1 + moved.1))?;
            out.extend([q.0 - p[0], q.1 - p[1]]);
            for h in handles {
                let vector = (
                    linear.xx * h[0] + linear.xy * h[1],
                    linear.yx * h[0] + linear.yy * h[1],
                );
                let turned = unplace(linear, map(vector))?;
                out.extend([turned.0 - h[0], turned.1 - h[1]]);
            }
        }
        Some(out)
    }

    /// Whether this capture is a Shift-drag selecting anchors.
    #[must_use]
    pub fn bands(&self) -> bool {
        self.band.is_some()
    }

    /// Extend the band to a canvas position.
    pub fn extend_band(&mut self, canvas: (f64, f64)) {
        self.band_to = canvas;
    }

    /// The band from the down to where it was last extended, `[x0, y0, x1, y1]` in canvas
    /// pixels.
    #[must_use]
    pub fn band_rect(&self) -> [f64; 4] {
        let (a, b) = (self.canvas_down, self.band_to);
        [a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1)]
    }

    /// The anchors the band encloses, within an anchor's radius on screen, ascending.
    #[must_use]
    pub fn band_points(&self) -> Vec<u32> {
        let r = self.band_rect();
        let grow = [BAND_REACH / self.device[0], BAND_REACH / self.device[1]];
        self.band.as_ref().map_or_else(Vec::new, |anchors| {
            anchors
                .iter()
                .filter(|(_, q)| {
                    (r[0] - grow[0]..=r[2] + grow[0]).contains(&q.0)
                        && (r[1] - grow[1]..=r[3] + grow[1]).contains(&q.1)
                })
                .map(|&(i, _)| i)
                .collect()
        })
    }

    /// The selected Curve's anchor count at down.
    #[must_use]
    pub fn anchors(&self) -> usize {
        match &self.original[self.index].shape {
            WindowShape::Curve { points, .. } => points.len(),
            _ => 0,
        }
    }

    pub fn at_rotation(&self, canvas: (f64, f64), rotation: Option<f64>) -> Option<Vec<Window>> {
        let picture = unplace(self.placement, canvas)?;
        let local = unplace(self.transform, picture)?;
        let delta = [local.0 - self.local_down.0, local.1 - self.local_down.1];
        let extent = columbina_model::grade::window::DEFAULT_EXTENT;
        let amount = match self.control {
            Control::Move
            | Control::Point(_)
            | Control::Edge(_)
            | Control::Insert(_)
            | Control::HandleIn(_)
            | Control::HandleOut(_)
            | Control::Close
            | Control::Append
            | Control::Band
            | Control::BoxMove
            | Control::BoxCorner(_)
            | Control::BoxEdge(_)
            | Control::BoxRotate(_) => 0.0,
            Control::EdgeSoftness(_) => {
                400.0 * (delta[0] * self.normal[0] + delta[1] * self.normal[1])
            }
            Control::Size(i) => {
                let corner = CORNERS[usize::from(i)].map(|x| x * extent);
                let stretch = (1.0
                    + (delta[0] * corner[0] + delta[1] * corner[1])
                        / (corner[0] * corner[0] + corner[1] * corner[1]))
                    .max(0.0);
                self.size * (stretch - 1.0)
            }
            Control::Aspect(i) => {
                let axis = AXES[usize::from(i)];
                let sign = if axis[0] == 0.0 { -1.0 } else { 1.0 };
                let left = self.keys[0].1.value;
                let (lower, upper) = (
                    (-left / (50.0 * sign)).exp2(),
                    ((100.0 - left) / (50.0 * sign)).exp2(),
                );
                let stretch = (1.0 + (delta[0] * axis[0] + delta[1] * axis[1]) / extent)
                    .clamp(lower.min(upper), lower.max(upper));
                sign * 50.0 * stretch.log2()
            }
            Control::Rotate => rotation?,
            // A Curve's Inside/Outside Softness spans value/200 local units.
            Control::Softness(_)
                if matches!(self.keys.first(), Some((Field::Inside | Field::Outside, _))) =>
            {
                200.0 * (delta[0] * self.normal[0] + delta[1] * self.normal[1])
            }
            Control::Softness(side) => {
                400.0
                    * if self.gradient {
                        f64::from(side) * delta[1]
                    } else {
                        delta[0] * SOFT_NORMAL[0] + delta[1] * SOFT_NORMAL[1]
                    }
            }
        };
        let targets = self.bezier_targets(canvas, delta);
        let deltas = match self.control {
            Control::BoxCorner(_) | Control::BoxEdge(_) | Control::BoxRotate(_) => {
                Some(self.multi_deltas(canvas)?)
            }
            _ => None,
        };
        let mut windows = self.base.clone();
        for (k, (&(field, key), target)) in self.keys.iter().zip(targets).enumerate() {
            let amount = match (target, field) {
                (Some(target), _) => target - key.value,
                _ if deltas.is_some() => deltas.as_ref().map_or(0.0, |d| d[k]),
                (None, Field::Pan) => 100.0 * (picture.0 - self.down.0),
                (None, Field::Tilt) => -100.0 * (picture.1 - self.down.1),
                (None, Field::X(_)) => delta[0],
                (None, Field::Y(_)) => delta[1],
                (None, _) => amount,
            };
            key.write(
                field.param(&mut windows[self.index]),
                amount,
                field.bounds(),
            );
        }
        let aspect = f64::from(self.picture.0) / f64::from(self.picture.1);
        Some(crate::tracking::frame_mode(
            &self.base,
            windows,
            self.index,
            self.frame,
            aspect,
            self.analysis,
        ))
    }
    pub fn op(selection: &Selection, windows: &[Window]) -> Value {
        json!({"op":"set_node","grade":selection.target,"id":selection.node,"params":{"windows":windows}})
    }
}
