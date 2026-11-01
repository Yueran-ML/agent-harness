//! Immutable Window selection and evaluated geometry. This module never locks session State.
use std::sync::Arc;

use columbina_model::grade::{
    window::bounds, Bounds, Corrector, NodeKind, Param, Target, Window, WindowShape,
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
        let shapes = c
            .windows
            .iter()
            .filter(|w| w.enabled)
            .filter_map(|w| {
                let (kind, softness) = match &w.shape {
                    WindowShape::Circular { softness } => (ShapeKind::Circular, softness),
                    WindowShape::Gradient { softness } => (ShapeKind::Gradient, softness),
                    _ => return None,
                };
                let [xx, yx, xy, yy, tx, ty] = w
                    .transform
                    .affine_at(at as f64, f64::from(size.0) / f64::from(size.1))?;
                Some(Shape {
                    id: w.id.clone(),
                    radius: columbina_model::grade::window::DEFAULT_EXTENT,
                    kind,
                    feather: softness
                        .value_at(at as f64)
                        .clamp(bounds::SOFTNESS.low, bounds::SOFTNESS.high)
                        / 400.0,
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

#[derive(Clone, Copy)]
enum Field {
    Size,
    Aspect,
    Pan,
    Tilt,
    Rotate,
    Softness,
}
impl Field {
    fn name(self) -> &'static str {
        match self {
            Self::Size => "size",
            Self::Aspect => "aspect",
            Self::Pan => "pan",
            Self::Tilt => "tilt",
            Self::Rotate => "rotate",
            Self::Softness => "softness",
        }
    }
    fn bounds(self) -> Bounds {
        match self {
            Self::Size => bounds::SIZE,
            Self::Aspect => bounds::ASPECT,
            Self::Pan => bounds::PAN,
            Self::Tilt => bounds::TILT,
            Self::Rotate => bounds::ROTATE,
            Self::Softness => bounds::SOFTNESS,
        }
    }
    fn param(self, window: &mut Window) -> &mut Param {
        match self {
            Self::Size => &mut window.transform.size,
            Self::Aspect => &mut window.transform.aspect,
            Self::Pan => &mut window.transform.pan,
            Self::Tilt => &mut window.transform.tilt,
            Self::Rotate => &mut window.transform.rotate,
            Self::Softness => match &mut window.shape {
                WindowShape::Circular { softness } | WindowShape::Gradient { softness } => softness,
                _ => unreachable!("captured supported shape"),
            },
        }
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
    pub original: Vec<Window>,
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
    pub fn keys(&self) -> Value {
        let mut keys = serde_json::Map::new();
        for (field, key) in &self.keys {
            keys.insert(field.name().into(), json!(key.frame));
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
        Self::capture(selection, frame, id, Control::Move, canvas)
    }
    #[allow(
        clippy::cast_precision_loss,
        reason = "the model evaluates keyframes as f64"
    )]
    pub fn capture(
        selection: &Selection,
        frame: &FrameSnapshot,
        id: &str,
        control: Control,
        canvas: (f64, f64),
    ) -> Option<Self> {
        let (placement, picture, at) = selection.placement(frame)?;
        let mut original = selection.corrector(&frame.project)?.windows.clone();
        let index = original.iter().position(|w| {
            w.id == id
                && w.enabled
                && matches!(
                    w.shape,
                    WindowShape::Circular { .. } | WindowShape::Gradient { .. }
                )
        })?;
        let gradient = matches!(original[index].shape, WindowShape::Gradient { .. });
        let fields: &[Field] = match control {
            Control::Move => &[Field::Pan, Field::Tilt],
            Control::Size(i) if !gradient && i < 4 => &[Field::Size],
            Control::Aspect(i) if !gradient && i < 4 => &[Field::Aspect],
            Control::Rotate => &[Field::Rotate],
            Control::Softness(side) if side == 1 || gradient && side == -1 => &[Field::Softness],
            _ => return None,
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
            original,
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
            Control::Move => 0.0,
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
        let mut windows = self.original.clone();
        for &(field, key) in &self.keys {
            let amount = match field {
                Field::Pan => 100.0 * (picture.0 - self.down.0),
                Field::Tilt => -100.0 * (picture.1 - self.down.1),
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
