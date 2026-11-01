//! Immutable Window selection and evaluated geometry. This module never locks session State.
use std::sync::Arc;

use columbina_model::grade::{
    window::{bounds, topology::TopologyError, MOST_POLYGON_POINTS},
    Bounds, Corrector, NodeKind, Param, Target, Window, WindowPoint, WindowShape,
};
use columbina_model::{Project, Timeline};
use columbina_view::window_overlay::{
    Affine, Control, Shape, ShapeKind, Style, WindowOverlay, AXES, CORNERS, SOFT_NORMAL,
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

/// A refused whole-animation Polygon edit, with what the desktop needs to explain it. `points`
/// are 0-based Polygon point indices; `frames` is the key interval in `clock` frames.
pub fn topology_refused(reason: &str, error: &TopologyError, clock: &str) -> Fault {
    let detail = match error {
        TopologyError::Maximum => json!({"why":"maximum","most":MOST_POLYGON_POINTS}),
        TopologyError::Minimum => json!({"why":"minimum","least":3}),
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
    /// A Polygon edge insertion that the model refused as a whole, with the keys' clock.
    Insert(TopologyError, &'static str),
}

impl Selection {
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
        let value = |p: &Param, b: Bounds| p.value_at(at as f64).clamp(b.low, b.high);
        let shapes = c
            .windows
            .iter()
            .filter(|w| w.enabled)
            .filter_map(|w| {
                // Linear Soft 1–4 span soft/200 across each side; Polygon inside/outside soft/200.
                let (kind, feather, feathers) = match &w.shape {
                    WindowShape::Circular { softness } => (
                        ShapeKind::Circular,
                        value(softness, bounds::SOFTNESS) / 400.0,
                        vec![],
                    ),
                    WindowShape::Gradient { softness } => (
                        ShapeKind::Gradient,
                        value(softness, bounds::SOFTNESS) / 400.0,
                        vec![],
                    ),
                    WindowShape::Linear { softness, .. } => (
                        ShapeKind::Linear,
                        0.0,
                        softness
                            .iter()
                            .map(|s| [value(s, bounds::SOFTNESS) / 400.0; 2])
                            .collect(),
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
                    ),
                    WindowShape::Curve { .. } => return None,
                };
                let [xx, yx, xy, yy, tx, ty] = w
                    .transform
                    .affine_at(at as f64, f64::from(size.0) / f64::from(size.1))?;
                Some(Shape {
                    id: w.id.clone(),
                    radius: columbina_model::grade::window::DEFAULT_EXTENT,
                    kind,
                    feather,
                    points: outline_at(&w.shape, at),
                    feathers,
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
            })
            .collect();
        Some(Arc::new(WindowOverlay {
            shapes,
            selected: self.selected.clone(),
            style: Style::default(),
            coverage,
        }))
    }
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
            Self::X(_) | Self::Y(_) => bounds::COORDINATE,
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
            placement,
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

pub struct Movement {
    /// The complete list at down, restored by Esc.
    pub original: Vec<Window>,
    /// What every update starts from: `original`, or it with an inserted Polygon point.
    base: Vec<Window>,
    inserted: Option<usize>,
    /// The captured Linear side's outward local normal at down.
    normal: [f64; 2],
    index: usize,
    keys: Vec<(Field, LeftKey)>,
    pub control: Control,
    down: (f64, f64),
    local_down: (f64, f64),
    transform: Affine,
    size: f64,
    gradient: bool,
    pub rotation: RotationBasis,
    pub placement: Affine,
    pub clock: &'static str,
    pub frame: i64,
}
impl Movement {
    pub fn selected(&self) -> &str {
        &self.original[self.index].id
    }
    /// The list with an inserted point, written by the first armed step, if this inserts.
    pub fn inserted(&self) -> Option<&[Window]> {
        self.inserted.map(|_| self.base.as_slice())
    }
    pub fn keys(&self) -> Value {
        let mut keys = serde_json::Map::new();
        let mut points: Vec<(usize, serde_json::Map<String, Value>)> = Vec::new();
        for (field, key) in &self.keys {
            match *field {
                Field::X(i) | Field::Y(i) => {
                    let at = points.iter().position(|(p, _)| *p == i).unwrap_or_else(|| {
                        let mut entry = serde_json::Map::new();
                        entry.insert("point".into(), json!(i));
                        points.push((i, entry));
                        points.len() - 1
                    });
                    points[at].1.insert(field.name().into(), json!(key.frame));
                }
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
        Self::capture(selection, frame, id, Control::Move, canvas).ok()
    }
    /// Capture one control. A Polygon `Insert` also inserts its point into `base` for the whole
    /// animation at the clicked share of the displayed edge, or refuses as a whole.
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
    ) -> Result<Self, Refusal> {
        let mut movement =
            Self::locate(selection, frame, id, control, canvas).ok_or(Refusal::Target)?;
        if let Control::Insert(edge) = control {
            movement.insert(usize::from(edge))?;
        }
        Ok(movement)
    }
    fn insert(&mut self, edge: usize) -> Result<(), Refusal> {
        let points = outline_at(&self.original[self.index].shape, self.frame);
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
        let inserted = self.base[self.index]
            .shape
            .insert_polygon_point(edge, share)
            .map_err(|error| Refusal::Insert(error, self.clock))?;
        self.inserted = Some(inserted);
        self.keys = [Field::X(inserted), Field::Y(inserted)]
            .into_iter()
            .map(|field| {
                (
                    field,
                    LeftKey::of(field.param(&mut self.base[self.index]), self.frame),
                )
            })
            .collect();
        Ok(())
    }
    #[allow(
        clippy::cast_precision_loss,
        reason = "the model evaluates keyframes as f64"
    )]
    fn locate(
        selection: &Selection,
        frame: &FrameSnapshot,
        id: &str,
        control: Control,
        canvas: (f64, f64),
    ) -> Option<Self> {
        let (placement, picture, at) = selection.placement(frame)?;
        let mut original = selection.corrector(&frame.project)?.windows.clone();
        let index = original.iter().position(|w| {
            w.id == id && w.enabled && !matches!(w.shape, WindowShape::Curve { .. })
        })?;
        let shape = &original[index].shape;
        let gradient = matches!(shape, WindowShape::Gradient { .. });
        let circular = matches!(shape, WindowShape::Circular { .. });
        let linear = matches!(shape, WindowShape::Linear { .. });
        let points = outline_at(shape, at);
        let n = points.len();
        let fields: Vec<Field> = match control {
            Control::Move => vec![Field::Pan, Field::Tilt],
            Control::Size(i) if circular && i < 4 => vec![Field::Size],
            Control::Aspect(i) if circular && i < 4 => vec![Field::Aspect],
            Control::Rotate if circular || gradient => vec![Field::Rotate],
            Control::Softness(side) if circular && side == 1 || gradient && side.abs() == 1 => {
                vec![Field::Softness]
            }
            Control::Point(i) if usize::from(i) < n => {
                vec![Field::X(usize::from(i)), Field::Y(usize::from(i))]
            }
            Control::Edge(j) if linear && usize::from(j) < n => {
                let (j, k) = (usize::from(j), (usize::from(j) + 1) % n);
                vec![Field::X(j), Field::Y(j), Field::X(k), Field::Y(k)]
            }
            Control::EdgeSoftness(j) if linear && usize::from(j) < n => {
                vec![Field::EdgeSoftness(usize::from(j))]
            }
            Control::Insert(j)
                if matches!(shape, WindowShape::Polygon { .. }) && usize::from(j) < n =>
            {
                vec![]
            }
            _ => return None,
        };
        let normal = match control {
            Control::EdgeSoftness(j) => outward(&points, usize::from(j)),
            _ => [0.0; 2],
        };
        let keys = fields
            .iter()
            .map(|&field| (field, LeftKey::of(field.param(&mut original[index]), at)))
            .collect();
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
            base: original.clone(),
            original,
            inserted: None,
            normal,
            index,
            keys,
            control,
            down,
            local_down,
            transform,
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
        })
    }
    pub fn at(&self, canvas: (f64, f64)) -> Option<Vec<Window>> {
        self.at_rotation(canvas, None)
    }
    pub fn at_rotation(&self, canvas: (f64, f64), rotation: Option<f64>) -> Option<Vec<Window>> {
        let picture = unplace(self.placement, canvas)?;
        let local = unplace(self.transform, picture)?;
        let delta = [local.0 - self.local_down.0, local.1 - self.local_down.1];
        let extent = columbina_model::grade::window::DEFAULT_EXTENT;
        let amount = match self.control {
            Control::Move | Control::Point(_) | Control::Edge(_) | Control::Insert(_) => 0.0,
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
            Control::Softness(side) => {
                400.0
                    * if self.gradient {
                        f64::from(side) * delta[1]
                    } else {
                        delta[0] * SOFT_NORMAL[0] + delta[1] * SOFT_NORMAL[1]
                    }
            }
        };
        let mut windows = self.base.clone();
        for &(field, key) in &self.keys {
            let amount = match field {
                Field::Pan => 100.0 * (picture.0 - self.down.0),
                Field::Tilt => -100.0 * (picture.1 - self.down.1),
                Field::X(_) => delta[0],
                Field::Y(_) => delta[1],
                _ => amount,
            };
            key.write(
                field.param(&mut windows[self.index]),
                amount,
                field.bounds(),
            );
        }
        Some(windows)
    }
    pub fn op(selection: &Selection, windows: &[Window]) -> Value {
        json!({"op":"set_node","grade":selection.target,"id":selection.node,"params":{"windows":windows}})
    }
}
