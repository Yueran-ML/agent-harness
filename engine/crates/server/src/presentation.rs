//! Immutable descriptions of submitted frames. Registration is only a candidate: a native
//! successful Presentation is the evidence that a tag was shown. No pictures or GPU resources
//! are retained here, and no registry guard is held while calling a view or the session.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use columbina_compose::{geometry, Scene, Source};
use columbina_model::Project;
use columbina_view::Presentation;
use serde::Serialize;
use serde_json::{json, Value};

use crate::play::sources::Provenance;
use crate::view::Monitor;

/// The origin of a scene layer before Highlight removes any layers. A reused late picture
/// keeps this record, even when the destination clip changes at a cut.
#[derive(Clone, Debug, Serialize)]
pub struct Origin {
    pub clip: String,
    pub timeline_frame: i64,
    pub key_frame: i64,
    pub media: Option<Provenance>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LayerSnapshot {
    /// The clip whose transform and grade were used, possibly different from origin.clip.
    pub clip: String,
    pub origin: Origin,
    pub shown_size: (u32, u32),
    /// Normalized, already metadata-oriented picture coordinates to canvas pixels.
    pub picture_to_canvas: [f64; 6],
    pub opacity: f32,
    pub editable: bool,
}

/// A scene together with the model version it was built from. The colour readiness and
/// comparison are frozen after composition and Viewer LUT, before this frame is submitted.
#[derive(Clone)]
pub struct FrameSnapshot {
    pub revision: u64,
    pub project: Arc<Project>,
    pub timeline: usize,
    pub timeline_frame: i64,
    pub canvas: (u32, u32),
    pub layers: Vec<LayerSnapshot>,
    pub comparison: Value,
    pub ready: bool,
    pub selection: Option<Arc<crate::window::Selection>>,
    pub overlay: Option<Arc<columbina_view::window_overlay::WindowOverlay>>,
    pub frozen: u64,
    pub highlight: Option<crate::highlight::Highlight>,
}

impl FrameSnapshot {
    pub fn editable(&self) -> bool {
        self.ready
            && self.comparison["mode"] != "split"
            && self.layers.iter().all(|layer| layer.editable)
    }

    /// Only the small public description travels with a pointer. Source Identity and the
    /// Project (including its local paths) stay inside this process.
    pub fn written(&self) -> Value {
        json!({
            "revision": self.revision,
            "timeline": self.project.timelines[self.timeline].id,
            "frame": self.timeline_frame,
            "canvas": self.canvas,
            "ready": self.ready,
            "editable": self.editable(),
            "comparison": self.comparison["mode"],
            "window": self.selection.as_ref().zip(self.overlay.as_ref()).map(|(s,_)| json!({"binding":s.binding,"selected":s.selected})),
        })
    }
}

/// Build from the FINAL scene and its matching clip list, after Highlight. Provenance is
/// selected by destination clip; it never changes to match that destination's source.
pub fn layers(
    scene: &Scene,
    clips: &[String],
    origins: &[(String, Origin, bool)],
) -> Vec<LayerSnapshot> {
    let canvas = (f64::from(scene.width), f64::from(scene.height));
    scene
        .layers
        .iter()
        .zip(clips)
        .filter_map(|(layer, clip)| {
            let (_, origin, exact) = origins.iter().find(|(id, _, _)| id == clip)?;
            let shown = match &layer.source {
                Source::Picture(picture, rotation) => {
                    geometry::shown_size((picture.width, picture.height), *rotation)
                }
                Source::Color(_) | Source::Title(_) => (scene.width, scene.height),
            };
            let fitted = geometry::fit(canvas, (f64::from(shown.0), f64::from(shown.1)));
            let place = geometry::placement(canvas, fitted, &layer.transform);
            Some(LayerSnapshot {
                clip: clip.clone(),
                origin: origin.clone(),
                shown_size: shown,
                picture_to_canvas: [
                    place.a * fitted.0,
                    place.b * fitted.1,
                    place.c * fitted.0,
                    place.d * fitted.1,
                    place.tx,
                    place.ty,
                ],
                opacity: layer.opacity,
                editable: *exact
                    && origin.clip == *clip
                    && fitted.0 > 0.0
                    && fitted.1 > 0.0
                    && layer.transform.scale > 0.0,
            })
        })
        .collect()
}

const PENDING: usize = 8;

#[derive(Default)]
struct MonitorFrames {
    instance: Option<u64>,
    observed: u64,
    current: Option<(Presentation, u64, Arc<FrameSnapshot>)>,
    pending: VecDeque<(u64, Arc<FrameSnapshot>)>,
    input: Option<NativeInput>,
}

#[derive(Default)]
struct Frames {
    next: u64,
    next_event: u64,
    monitors: [MonitorFrames; 2],
    redraw: [Option<Arc<dyn Fn() + Send + Sync>>; 2],
}

#[derive(Clone)]
pub struct Input {
    pub sequence: u64,
    pub owner: u64,
    pub instance: u64,
    pub pointer: columbina_view::Pointer,
    pub frame: Option<Arc<FrameSnapshot>>,
    pub rotation: Option<RotationEvidence>,
}

/// Consecutive samples with the same actual map/frame share one proof. The sequence marks
/// the last sample in that run, so pre-arm playback is distinguished from post-arm changes.
#[derive(Clone)]
pub struct RotationSample {
    pub sequence: u64,
    pub owner: u64,
    pub instance: u64,
    pub shown: Option<Presentation>,
    pub frame: Option<Arc<FrameSnapshot>>,
}
#[derive(Clone)]
pub struct RotationEvidence {
    pub basis: crate::window::RotationBasis,
    pub degrees: f64,
    pub samples: Vec<RotationSample>,
}
struct RotationTrace {
    evidence: RotationEvidence,
    previous: Option<f64>,
}
impl RotationTrace {
    #[allow(
        clippy::cast_precision_loss,
        reason = "native pixel centres share overlay device coordinates"
    )]
    fn begin(down: &Input) -> Option<Self> {
        use columbina_view::window_overlay::Control;
        let frame = down.frame.as_ref()?;
        let map = down.pointer.presentation?.map?;
        let hit = frame.overlay.as_ref()?.hit_control(
            map,
            [
                down.pointer.window.0 as f32 + 0.5,
                down.pointer.window.1 as f32 + 0.5,
            ],
        )?;
        if hit.control != Control::Rotate {
            return None;
        }
        let basis = crate::window::RotationBasis::of(frame.selection.as_ref()?, frame, hit.id)?;
        Some(Self {
            evidence: RotationEvidence {
                basis,
                degrees: 0.0,
                samples: vec![],
            },
            previous: basis.angle_at(map, down.pointer.window),
        })
    }
    fn observe(&mut self, down: &Input, input: &Input) -> RotationEvidence {
        let map = down
            .pointer
            .presentation
            .and_then(|p| p.map)
            .expect("rotation down map");
        let angle = self.evidence.basis.angle_at(map, input.pointer.window);
        if let (Some(previous), Some(angle)) = (self.previous, angle) {
            let delta = (angle - previous + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU)
                - std::f64::consts::PI;
            self.evidence.degrees += delta.to_degrees();
        }
        self.previous = angle;
        let sample = RotationSample {
            sequence: input.sequence,
            owner: input.owner,
            instance: input.instance,
            shown: input.pointer.presentation,
            frame: input.frame.clone(),
        };
        let same = self.evidence.samples.last().is_some_and(|last| {
            last.owner == sample.owner
                && last.instance == sample.instance
                && last.shown.and_then(|p| p.map) == sample.shown.and_then(|p| p.map)
                && match (&last.frame, &sample.frame) {
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    (None, None) => true,
                    _ => false,
                }
        });
        if same {
            *self.evidence.samples.last_mut().expect("last proof") = sample;
        } else {
            self.evidence.samples.push(sample);
        }
        self.evidence.clone()
    }
}

/// Native evidence pins a down independently of the eight pending presentation candidates.
/// It contains no controller decisions and never acquires session State.
pub struct Pin {
    pub down: Input,
    pub live: AtomicBool,
    pub wake: tokio::sync::Notify,
}
struct NativeInput {
    pin: Arc<Pin>,
    claimed: bool,
    events: VecDeque<Input>,
    rotation: Option<RotationTrace>,
}
impl Drop for NativeInput {
    fn drop(&mut self) {
        self.pin.live.store(false, Ordering::Release);
        self.pin.wake.notify_one();
    }
}

#[derive(Clone, Default)]
pub struct Registry(Arc<Mutex<Frames>>);

impl Registry {
    pub fn input(
        &self,
        monitor: Monitor,
        owner: u64,
        instance: u64,
        pointer: columbina_view::Pointer,
        frame: Option<Arc<FrameSnapshot>>,
    ) -> u64 {
        use columbina_view::{Button, PointerKind};
        let mut frames = self.lock();
        frames.next_event += 1;
        let sequence = frames.next_event;
        let slot = &mut frames.monitors[monitor.index()];
        if slot.instance != Some(instance) {
            return sequence;
        }
        let mut input = Input {
            sequence,
            owner,
            instance,
            pointer,
            frame,
            rotation: None,
        };
        if pointer.kind == PointerKind::Down && pointer.button == Some(Button::Left) {
            let rotation = RotationTrace::begin(&input);
            slot.input = Some(NativeInput {
                pin: Arc::new(Pin {
                    down: input,
                    live: AtomicBool::new(true),
                    wake: tokio::sync::Notify::new(),
                }),
                claimed: false,
                events: VecDeque::new(),
                rotation,
            });
        } else if let Some(active) = &mut slot.input {
            if active.pin.down.owner == owner && active.pin.live.load(Ordering::Acquire) {
                let terminal = pointer.kind == PointerKind::Cancel
                    || (pointer.kind == PointerKind::Up && pointer.button == Some(Button::Left));
                if pointer.kind == PointerKind::Move || terminal {
                    if pointer.kind == PointerKind::Move {
                        input.rotation = active
                            .rotation
                            .as_mut()
                            .map(|trace| trace.observe(&active.pin.down, &input));
                    }
                    if pointer.kind == PointerKind::Move
                        && active
                            .events
                            .back()
                            .is_some_and(|e| e.pointer.kind == PointerKind::Move)
                    {
                        active.events.pop_back();
                    }
                    active.events.push_back(input);
                    if terminal {
                        active.pin.live.store(false, Ordering::Release);
                    }
                    active.pin.wake.notify_one();
                }
            }
        }
        let retired = if slot
            .input
            .as_ref()
            .is_some_and(|input| !input.claimed && !input.pin.live.load(Ordering::Acquire))
        {
            slot.input.take()
        } else {
            None
        };
        drop(frames);
        drop(retired);
        sequence
    }

    pub fn claim(&self, owner: u64, event: u64) -> Option<Arc<Pin>> {
        let mut frames = self.lock();
        let input = frames.monitors[Monitor::Timeline.index()].input.as_mut()?;
        if input.claimed
            || input.pin.down.owner != owner
            || input.pin.down.sequence != event
            || !input.pin.live.load(Ordering::Acquire)
        {
            return None;
        }
        input.claimed = true;
        Some(Arc::clone(&input.pin))
    }

    pub fn events(&self, pin: &Arc<Pin>) -> Vec<Input> {
        let mut frames = self.lock();
        frames.monitors[Monitor::Timeline.index()]
            .input
            .as_mut()
            .filter(|input| Arc::ptr_eq(&input.pin, pin))
            .map_or_else(Vec::new, |input| input.events.iter().cloned().collect())
    }

    /// Acknowledge only after State consumes a copy. An echoed up still sees a task's
    /// held move while the task is waiting to acquire State.
    pub fn acknowledged(&self, pin: &Arc<Pin>, sequence: u64) {
        let mut frames = self.lock();
        if let Some(input) = frames.monitors[Monitor::Timeline.index()]
            .input
            .as_mut()
            .filter(|input| Arc::ptr_eq(&input.pin, pin))
        {
            input.events.retain(|event| event.sequence > sequence);
            if let Some(rotation) = &mut input.rotation {
                rotation
                    .evidence
                    .samples
                    .retain(|sample| sample.sequence > sequence);
            }
        }
    }

    pub fn release(&self, pin: &Arc<Pin>) {
        let mut frames = self.lock();
        let input = &mut frames.monitors[Monitor::Timeline.index()].input;
        if input
            .as_ref()
            .is_some_and(|input| Arc::ptr_eq(&input.pin, pin))
        {
            *input = None;
        }
    }

    pub fn stop_input(&self, monitor: Monitor) {
        self.lock().monitors[monitor.index()].input = None;
    }

    pub fn current(
        &self,
        monitor: Monitor,
        instance: u64,
    ) -> Option<(Presentation, Arc<FrameSnapshot>)> {
        let frames = self.lock();
        let slot = &frames.monitors[monitor.index()];
        (slot.instance == Some(instance)).then_some(())?;
        slot.current.as_ref().map(|(p, _, f)| (*p, Arc::clone(f)))
    }
    fn lock(&self) -> MutexGuard<'_, Frames> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn redraw_on_invalidate(&self, monitor: Monitor, redraw: Arc<dyn Fn() + Send + Sync>) {
        self.lock().redraw[monitor.index()] = Some(redraw);
    }

    /// Called only when a successfully constructed view becomes the attached view.
    pub fn activate(&self, monitor: Monitor, instance: Option<u64>) {
        let retired = std::mem::replace(
            &mut self.lock().monitors[monitor.index()],
            MonitorFrames {
                instance,
                ..MonitorFrames::default()
            },
        );
        // A last Project Arc can destroy a large model. Do that outside the registry lock.
        drop(retired);
    }

    pub fn gone(&self, monitor: Monitor, instance: u64) {
        let mut frames = self.lock();
        if frames.monitors[monitor.index()].instance == Some(instance) {
            let retired = std::mem::take(&mut frames.monitors[monitor.index()]);
            drop(frames);
            drop(retired);
        }
    }

    /// Unloading a timeline retires its descriptions even while the native clear is pending.
    /// That old image is then unknown, never attributed to a newly loaded project.
    pub fn reset(&self, monitor: Monitor, instance: u64) {
        let mut frames = self.lock();
        let slot = &mut frames.monitors[monitor.index()];
        if slot.instance == Some(instance) {
            slot.input = None;
            let retired = (slot.current.take(), std::mem::take(&mut slot.pending));
            drop(frames);
            drop(retired);
        }
    }

    /// A surface/device loss invalidates old evidence and candidates. A later unrecognized
    /// tag is rejected until the producer submits another registered frame.
    pub fn invalidate(&self, monitor: Monitor, instance: u64) {
        let mut frames = self.lock();
        let slot = &mut frames.monitors[monitor.index()];
        if slot.instance == Some(instance) {
            slot.input = None;
            let retired = (slot.current.take(), std::mem::take(&mut slot.pending));
            let redraw = frames.redraw[monitor.index()].clone();
            drop(frames);
            drop(retired);
            if let Some(redraw) = redraw {
                redraw();
            }
        }
    }

    pub fn register(
        &self,
        monitor: Monitor,
        instance: u64,
        snapshot: Arc<FrameSnapshot>,
    ) -> Option<u64> {
        let mut frames = self.lock();
        if frames.monitors[monitor.index()].instance != Some(instance) {
            return None;
        }
        frames.next += 1;
        let tag = frames.next;
        let slot = &mut frames.monitors[monitor.index()];
        slot.pending.push_back((tag, snapshot));
        let retired = (slot.pending.len() > PENDING).then(|| slot.pending.pop_front());
        drop(frames);
        drop(retired);
        Some(tag)
    }

    pub fn discard(&self, monitor: Monitor, instance: u64, tag: u64) {
        let mut frames = self.lock();
        let slot = &mut frames.monitors[monitor.index()];
        if slot.instance == Some(instance) {
            let retired = slot
                .pending
                .iter()
                .position(|(id, _)| *id == tag)
                .and_then(|index| slot.pending.remove(index));
            drop(frames);
            drop(retired);
        }
    }

    /// Accept only a Presentation copied from the native view, never a client-supplied tag.
    /// A late pointer can resolve its retained frame but cannot move current backwards.
    pub fn observe(
        &self,
        monitor: Monitor,
        instance: u64,
        shown: Option<Presentation>,
    ) -> Option<Arc<FrameSnapshot>> {
        let mut frames = self.lock();
        let slot = &mut frames.monitors[monitor.index()];
        if slot.instance != Some(instance) {
            return None;
        }
        let shown = shown?;
        let snapshot = shown.tag.filter(|_| shown.map.is_some()).and_then(|tag| {
            slot.current
                .as_ref()
                .filter(|(_, id, _)| *id == tag)
                .map(|(_, _, frame)| Arc::clone(frame))
                .or_else(|| {
                    slot.pending
                        .iter()
                        .find(|(id, _)| *id == tag)
                        .map(|(_, frame)| Arc::clone(frame))
                })
        });
        let advanced = shown.serial > slot.observed;
        let retired = if shown.serial >= slot.observed {
            slot.observed = shown.serial;
            let retired = slot.current.take();
            if let (Some(tag), Some(snapshot)) = (shown.tag, snapshot.as_ref()) {
                slot.pending.retain(|(id, _)| *id != tag);
                slot.current = Some((shown, tag, Arc::clone(snapshot)));
            }
            retired
        } else {
            None
        };
        if advanced {
            if let Some(input) = &slot.input {
                input.pin.wake.notify_one();
            }
        }
        drop(frames);
        drop(retired);
        snapshot
    }
}

pub fn written(shown: Option<Presentation>) -> Value {
    shown.map_or(Value::Null, |shown| json!({
        "serial": shown.serial,
        "tag": shown.tag,
        "map": shown.map.map(|map| json!({ "surface": map.surface, "picture": map.picture, "dest": map.dest })),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> Arc<Project> {
        Arc::new(
            Project::from_value(
                json!({"schema":3,"next_id":1,"bins":[],"media":[],"timelines":[]}),
            )
            .unwrap(),
        )
    }
    fn snapshot(project: Arc<Project>, revision: u64) -> Arc<FrameSnapshot> {
        Arc::new(FrameSnapshot {
            revision,
            project,
            timeline: 0,
            timeline_frame: 10,
            canvas: (64, 32),
            layers: vec![],
            comparison: json!({"mode":"off"}),
            ready: true,
            selection: None,
            overlay: None,
            frozen: 0,
            highlight: None,
        })
    }
    fn shown(serial: u64, tag: u64) -> Presentation {
        Presentation {
            serial,
            tag: Some(tag),
            map: columbina_view::Viewport::Fit.map((128, 64), (64, 32), 0),
        }
    }

    #[test]
    fn candidates_do_not_replace_current_and_are_bounded_without_holding_old_projects() {
        let registry = Registry::default();
        registry.activate(Monitor::Timeline, Some(1));
        let old_project = project();
        let old_weak = Arc::downgrade(&old_project);
        let old = registry
            .register(Monitor::Timeline, 1, snapshot(old_project, 4))
            .unwrap();
        assert!(registry.lock().monitors[1].current.is_none());
        assert_eq!(
            registry
                .observe(Monitor::Timeline, 1, Some(shown(10, old)))
                .unwrap()
                .revision,
            4
        );
        let evicted_project = project();
        let evicted_weak = Arc::downgrade(&evicted_project);
        let evicted = registry
            .register(Monitor::Timeline, 1, snapshot(evicted_project, 5))
            .unwrap();
        let mut last = 0;
        for revision in 6..40 {
            last = registry
                .register(Monitor::Timeline, 1, snapshot(project(), revision))
                .unwrap();
        }
        assert_eq!(registry.lock().monitors[1].pending.len(), PENDING);
        assert!(evicted_weak.upgrade().is_none());
        assert!(registry
            .observe(Monitor::Timeline, 1, Some(shown(9, evicted)))
            .is_none());
        assert_eq!(
            registry
                .observe(Monitor::Timeline, 1, Some(shown(10, old)))
                .unwrap()
                .revision,
            4
        );
        assert!(old_weak.upgrade().is_some());
        assert_eq!(
            registry
                .observe(Monitor::Timeline, 1, Some(shown(11, last)))
                .unwrap()
                .revision,
            39
        );
        assert!(old_weak.upgrade().is_none());
    }

    #[test]
    fn unknown_other_monitor_replaced_instance_and_invalidated_tags_never_resolve() {
        let registry = Registry::default();
        registry.activate(Monitor::Timeline, Some(1));
        registry.activate(Monitor::Source, Some(2));
        let p = project();
        let weak = Arc::downgrade(&p);
        let tag = registry
            .register(Monitor::Timeline, 1, snapshot(p, 7))
            .unwrap();
        assert!(registry
            .observe(Monitor::Source, 2, Some(shown(1, tag)))
            .is_none());
        assert!(registry
            .observe(Monitor::Timeline, 1, Some(shown(1, tag + 100)))
            .is_none());
        registry.activate(Monitor::Timeline, Some(3));
        assert!(weak.upgrade().is_none());
        assert!(registry
            .observe(Monitor::Timeline, 1, Some(shown(1, tag)))
            .is_none());
        let tag = registry
            .register(Monitor::Timeline, 3, snapshot(project(), 8))
            .unwrap();
        registry.invalidate(Monitor::Timeline, 3);
        assert!(registry
            .observe(Monitor::Timeline, 3, Some(shown(2, tag)))
            .is_none());
        registry.activate(Monitor::Timeline, None);
        assert!(registry
            .register(Monitor::Timeline, 3, snapshot(project(), 9))
            .is_none());
    }

    #[test]
    fn presentation_rotation_and_placement_use_normalized_oriented_picture_coordinates() {
        let picture = columbina_media::NativePicture {
            width: 640,
            height: 360,
            ..Default::default()
        };
        let scene = Scene {
            width: 1280,
            height: 720,
            layers: vec![columbina_compose::SceneLayer {
                source: Source::Picture(Arc::new(picture), 90),
                transform: geometry::Transform {
                    x: 0.1,
                    y: 0.0,
                    scale: 0.5,
                    rotation: 90.0,
                },
                opacity: 1.0,
                color: columbina_compose::LayerColor::default(),
            }],
            ..Default::default()
        };
        let origin = Origin {
            clip: "c1".into(),
            timeline_frame: 0,
            key_frame: 0,
            media: None,
        };
        let layers = layers(&scene, &["c1".into()], &[("c1".into(), origin, true)]);
        assert_eq!(layers[0].shown_size, (360, 640));
        for (actual, expected) in layers[0]
            .picture_to_canvas
            .into_iter()
            .zip([0.0, -360.0, 202.5, 0.0, 948.0, 258.75])
        {
            assert!((actual - expected).abs() < 1e-8, "{actual} != {expected}");
        }
    }

    #[test]
    fn gone_releases_frames_without_redraw_and_old_instance_cannot_clear_a_new_one() {
        let registry = Registry::default();
        let redraws = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        registry.redraw_on_invalidate(Monitor::Timeline, {
            let redraws = Arc::clone(&redraws);
            Arc::new(move || {
                redraws.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            })
        });
        registry.activate(Monitor::Timeline, Some(2));
        let p = project();
        let weak = Arc::downgrade(&p);
        let tag = registry
            .register(Monitor::Timeline, 2, snapshot(p, 1))
            .unwrap();
        registry.gone(Monitor::Timeline, 1);
        assert!(registry
            .observe(Monitor::Timeline, 2, Some(shown(1, tag)))
            .is_some());
        registry.gone(Monitor::Timeline, 2);
        assert!(weak.upgrade().is_none());
        assert_eq!(redraws.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert!(registry
            .observe(Monitor::Timeline, 2, Some(shown(1, tag)))
            .is_none());
    }

    #[test]
    fn late_pointers_cannot_restore_current_after_a_newer_clear_or_unknown_presentation() {
        let registry = Registry::default();
        registry.activate(Monitor::Timeline, Some(1));
        let old = registry
            .register(Monitor::Timeline, 1, snapshot(project(), 1))
            .unwrap();
        let pending = registry
            .register(Monitor::Timeline, 1, snapshot(project(), 2))
            .unwrap();
        registry
            .observe(Monitor::Timeline, 1, Some(shown(1, old)))
            .unwrap();
        registry.observe(
            Monitor::Timeline,
            1,
            Some(Presentation {
                serial: 3,
                tag: None,
                map: None,
            }),
        );
        assert!(registry
            .observe(Monitor::Timeline, 1, Some(shown(2, pending)))
            .is_some());
        assert!(registry.lock().monitors[1].current.is_none());
        let current = registry
            .register(Monitor::Timeline, 1, snapshot(project(), 3))
            .unwrap();
        registry
            .observe(Monitor::Timeline, 1, Some(shown(4, current)))
            .unwrap();
        registry.observe(Monitor::Timeline, 1, Some(shown(5, current + 100)));
        assert!(registry.lock().monitors[1].current.is_none());
        registry
            .observe(Monitor::Timeline, 1, Some(shown(2, pending)))
            .unwrap();
        assert!(registry.lock().monitors[1].current.is_none());
        assert_eq!(registry.lock().monitors[1].observed, 5);
    }
}
