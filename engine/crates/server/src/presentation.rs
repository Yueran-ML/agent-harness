//! Immutable descriptions of submitted frames. Registration is only a candidate: a native
//! successful Presentation is the evidence that a tag was shown. No pictures or GPU resources
//! are retained here, and no registry guard is held while calling a view or the session.

use std::collections::VecDeque;
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
pub struct FrameSnapshot {
    pub revision: u64,
    pub project: Arc<Project>,
    pub timeline: usize,
    pub timeline_frame: i64,
    pub canvas: (u32, u32),
    pub layers: Vec<LayerSnapshot>,
    pub comparison: Value,
    pub ready: bool,
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
}

#[derive(Default)]
struct Frames {
    next: u64,
    monitors: [MonitorFrames; 2],
    redraw: [Option<Arc<dyn Fn() + Send + Sync>>; 2],
}

#[derive(Clone, Default)]
pub struct Registry(Arc<Mutex<Frames>>);

impl Registry {
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
