//! Immutable Window selection and evaluated geometry. This module never locks session State.
use std::sync::Arc;

use columbina_model::grade::{Corrector, NodeKind, Param, Target, Window, WindowShape};
use columbina_model::{Project, Timeline};
use columbina_view::window_overlay::{Affine, Circular, Style, WindowOverlay};
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
        let circles = c
            .windows
            .iter()
            .filter(|w| w.enabled && matches!(w.shape, WindowShape::Circular { .. }))
            .filter_map(|w| {
                let [xx, yx, xy, yy, tx, ty] = w
                    .transform
                    .affine_at(at as f64, f64::from(size.0) / f64::from(size.1))?;
                Some(Circular {
                    id: w.id.clone(),
                    radius: columbina_model::grade::window::DEFAULT_EXTENT,
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
            circles,
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
    fn write(self, param: &mut Param, delta: f64) {
        match (param, self.index) {
            (Param::Fixed(value), None) => *value = self.value + delta,
            (Param::Keys(keys), Some(index)) => keys[index].value = self.value + delta,
            _ => unreachable!("original whole array preserves key structure"),
        }
    }
}

pub struct Movement {
    pub original: Vec<Window>,
    index: usize,
    pub pan: LeftKey,
    pub tilt: LeftKey,
    down: (f64, f64),
    pub placement: Affine,
    pub clock: &'static str,
    pub frame: i64,
}
impl Movement {
    pub fn selected(&self) -> &str {
        &self.original[self.index].id
    }
    pub fn new(
        selection: &Selection,
        frame: &FrameSnapshot,
        id: &str,
        canvas: (f64, f64),
    ) -> Option<Self> {
        let (placement, _, at) = selection.placement(frame)?;
        let original = selection.corrector(&frame.project)?.windows.clone();
        let index = original.iter().position(|w| {
            w.id == id && w.enabled && matches!(w.shape, WindowShape::Circular { .. })
        })?;
        let pan = LeftKey::of(&original[index].transform.pan, at);
        let tilt = LeftKey::of(&original[index].transform.tilt, at);
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
            pan,
            tilt,
            down: unplace(placement, canvas)?,
            placement,
            clock,
            frame: at,
        })
    }
    pub fn at(&self, canvas: (f64, f64)) -> Option<Vec<Window>> {
        let (u, v) = unplace(self.placement, canvas)?;
        let mut windows = self.original.clone();
        self.pan.write(
            &mut windows[self.index].transform.pan,
            100.0 * (u - self.down.0),
        );
        self.tilt.write(
            &mut windows[self.index].transform.tilt,
            -100.0 * (v - self.down.1),
        );
        Some(windows)
    }
    pub fn op(selection: &Selection, windows: &[Window]) -> Value {
        json!({"op":"set_node","grade":selection.target,"id":selection.node,"params":{"windows":windows}})
    }
}
