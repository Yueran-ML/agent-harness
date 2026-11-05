//! Timeline playback: independent track readers, composition and the monitor transport.

mod ahead;
pub(crate) mod mix;
pub(crate) mod record;
mod sound;
pub(crate) mod sources;
mod timeline;

use std::path::PathBuf;
use std::sync::Arc;

use columbina_compose::{Compositor, Scene, Target};
use columbina_model::Project;
use serde::Deserialize;
use serde_json::{json, value::RawValue, Value};
use sha2::{Digest, Sha256};

use crate::gpu::Graphics;
use crate::grading::LutPlaces;
use crate::log::log;
use crate::rpc::{self, Fault};
use crate::scopes::{Scopes, Shown};
use crate::transport;
use crate::view::{Monitor, Viewer};

pub type Notify = Arc<dyn Fn(&Value) + Send + Sync>;

#[derive(Clone, Copy, Debug)]
pub enum Action {
    Play {
        speed: i32,
    },
    Pause,
    Seek {
        frame: i64,
        scrubbing: bool,
    },
    Step {
        frames: i64,
    },
    Proxies {
        on: bool,
    },
    /// Whether dragging the playhead plays its sound (T18; Timeline › Audio Scrubbing).
    ScrubAudio {
        on: bool,
    },
}

#[derive(Deserialize)]
#[serde(tag = "monitor", rename_all = "lowercase", deny_unknown_fields)]
enum Load {
    Source { media: String },
    Timeline { timeline: String },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Play {
    #[serde(default = "one")]
    speed: i32,
}
fn one() -> i32 {
    1
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Seek {
    frame: i64,
    #[serde(default)]
    scrubbing: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    frames: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Proxies {
    on: bool,
}

/// Removing the required monitor leaves the strict method-specific parameter parser intact.
pub fn monitor_params(
    method: &str,
    params: Option<&RawValue>,
) -> Result<(Monitor, Box<RawValue>), Fault> {
    let mut params: serde_json::Map<String, Value> =
        rpc::params(method, r#"{"monitor":"source"|"timeline", ...}"#, params)?;
    let monitor = params
        .remove("monitor")
        .ok_or_else(|| Fault::params("monitor is required"))?;
    let monitor = serde_json::from_value(monitor)
        .map_err(|_| Fault::params("monitor must be source or timeline"))?;
    Ok((
        monitor,
        serde_json::value::to_raw_value(&params).expect("JSON parameters"),
    ))
}

fn action(method: &str, params: &RawValue) -> Result<Option<Action>, Fault> {
    let action = match method {
        "transport.state" => {
            rpc::no_params(method, Some(params))?;
            return Ok(None);
        }
        "transport.pause" => {
            rpc::no_params(method, Some(params))?;
            Action::Pause
        }
        "transport.play" => {
            let Play { speed } = rpc::params(method, r#"{"speed":1}"#, Some(params))?;
            if speed == 0 || !(-transport::FASTEST..=transport::FASTEST).contains(&speed) {
                return Err(Fault::params(
                    "speed must be a whole number from -8 to 8, other than 0",
                )
                .with_data(json!({"reason":"speed"})));
            }
            Action::Play { speed }
        }
        "transport.seek" => {
            let Seek { frame, scrubbing } =
                rpc::params(method, r#"{"frame":0,"scrubbing":false}"#, Some(params))?;
            Action::Seek { frame, scrubbing }
        }
        "transport.step" => {
            let Step { frames } = rpc::params(method, r#"{"frames":1}"#, Some(params))?;
            Action::Step { frames }
        }
        "transport.proxies" => {
            let Proxies { on } = rpc::params(method, r#"{"on":true}"#, Some(params))?;
            Action::Proxies { on }
        }
        "transport.scrub_audio" => {
            let Proxies { on } = rpc::params(method, r#"{"on":true}"#, Some(params))?;
            Action::ScrubAudio { on }
        }
        _ => {
            return Err(Fault::new(
                rpc::METHOD_NOT_FOUND,
                format!("there is no method {method}"),
            ))
        }
    };
    Ok(Some(action))
}

#[derive(Clone)]
pub struct Transport {
    source: transport::Transport,
    timeline: Option<timeline::Timeline>,
    graphics: Arc<Graphics>,
    orders: Arc<tokio::sync::Mutex<()>>,
    test_hooks: bool,
}

pub enum Loading {
    Source(transport::Loading),
    Timeline(timeline::Loading),
}

impl Loading {
    pub async fn done(self) -> Result<Value, Fault> {
        match self {
            Self::Source(loading) => Ok(source_status(&loading.done().await?)),
            Self::Timeline(loading) => loading.done().await,
        }
    }
}

fn source_status(status: &transport::Status) -> Value {
    let mut value = json!(status);
    value["monitor"] = json!("source");
    value
}

impl Transport {
    pub async fn window_turn(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.orders.lock().await
    }

    pub fn window(&self, update: crate::window::Update) {
        if let Some(timeline) = &self.timeline {
            timeline.window(update);
        }
    }

    /// Enqueue synchronously while the caller holds the orders turn and has rechecked State.
    pub fn freeze_at(
        &self,
        timeline: String,
        revision: u64,
        frame: i64,
        capture: u64,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<(), Fault>>, Fault> {
        self.timeline
            .as_ref()
            .ok_or_else(|| crate::window::refused("monitor"))?
            .freeze_at(timeline, revision, frame, capture)
    }
    #[allow(
        clippy::too_many_arguments,
        reason = "what the two monitors are made of, each from the engine"
    )]
    pub fn new(
        headless: bool,
        setup: transport::Setup,
        viewer: &Viewer,
        graphics: Arc<Graphics>,
        luts: LutPlaces,
        notify: Notify,
        again: transport::Again,
        repair: transport::Again,
        scopes: &Scopes,
    ) -> Self {
        // A capture is of the timeline monitor's sound (T5, for T6): the source monitor plays
        // on an output of no device that writes nothing, so that it never makes the file anew.
        let source_setup = transport::Setup {
            sound: match &setup.sound {
                crate::audio::Kind::Capture(_) => crate::audio::Kind::Null,
                other => other.clone(),
            },
            ..setup.clone()
        };
        let test_hooks = setup.test_hooks;
        let source = transport::Transport::new(
            headless,
            source_setup,
            viewer.slot(Monitor::Source),
            {
                let notify = Arc::clone(&notify);
                Arc::new(move |status| notify(&source_status(status)))
            },
            again,
            scopes.clone(),
        );
        let timeline = (!headless).then(|| {
            timeline::Timeline::new(
                Arc::clone(&graphics),
                viewer.slot(Monitor::Timeline),
                viewer.frames.clone(),
                notify,
                repair,
                setup.sound,
                luts,
                scopes.clone(),
            )
        });
        Self {
            source,
            timeline,
            graphics,
            orders: Arc::default(),
            test_hooks,
        }
    }

    pub fn check(&self, method: &str) -> Result<(), Fault> {
        if method == "transport.frame_digest" && self.test_hooks && self.timeline.is_some() {
            Ok(())
        } else if method == "transport.scrub_audio" {
            // A method of the timeline monitor (T18), where there is one.
            self.source.check("transport.state")
        } else {
            self.source.check(method)
        }
    }

    pub fn begin_load(
        &self,
        project: &Project,
        revision: u64,
        params: Option<&RawValue>,
    ) -> Result<Loading, Fault> {
        self.check("transport.load")?;
        let load: Load = rpc::params(
            "transport.load",
            r#"{"monitor":"source","media":"m1"} or {"monitor":"timeline","timeline":"t1"}"#,
            params,
        )?;
        match load {
            Load::Source { media } => {
                let found = project
                    .media
                    .iter()
                    .find(|found| found.id == media)
                    .ok_or_else(|| {
                        Fault::params(format!("there is no media {media} in the project"))
                            .with_data(json!({"reason":"missing","media":media}))
                    })?;
                self.source
                    .begin_load(
                        &media,
                        PathBuf::from(&found.path),
                        crate::proxy::usable(found).map(|proxy| PathBuf::from(&proxy.path)),
                        found.facts.clone(),
                        project.timelines.first().map(|timeline| timeline.rate),
                    )
                    .map(Loading::Source)
            }
            Load::Timeline { timeline } => {
                if !project.timelines.iter().any(|found| found.id == timeline) {
                    return Err(Fault::params(format!(
                        "there is no timeline {timeline} in the project"
                    ))
                    .with_data(json!({"reason":"missing","timeline":timeline})));
                }
                self.timeline
                    .as_ref()
                    .expect("not headless")
                    .begin_load(revision, Arc::new(project.clone()), timeline)
                    .map(Loading::Timeline)
            }
        }
    }

    pub fn prepare(
        &self,
        method: &str,
        params: Option<&RawValue>,
    ) -> Result<(Value, Order), Fault> {
        self.check(method)?;
        let (monitor, params) = monitor_params(method, params)?;
        let action = action(method, &params)?;
        let playing = matches!(action, Some(Action::Play { .. }));
        let (result, target) = match monitor {
            Monitor::Source if matches!(action, Some(Action::ScrubAudio { .. })) => {
                return Err(Fault::params(
                    "transport.scrub_audio is of the timeline monitor: the source monitor plays no sound \
                     while its playhead is dragged",
                )
                .with_data(json!({"reason":"monitor"})));
            }
            Monitor::Source => {
                let (mut result, order) = self.source.prepare(method, Some(&params))?;
                if method == "transport.state" {
                    result["monitor"] = json!("source");
                }
                (result, TargetOrder::Source(Box::new(order)))
            }
            Monitor::Timeline => {
                let timeline = self.timeline.as_ref().expect("not headless");
                if action.as_ref().is_some_and(|action| {
                    !matches!(action, Action::Proxies { .. } | Action::ScrubAudio { .. })
                }) {
                    timeline.check_loaded()?;
                }
                let result = if action.is_none() {
                    timeline.status()
                } else {
                    json!({})
                };
                (result, TargetOrder::Timeline(action))
            }
        };
        Ok((
            result,
            Order {
                transport: self.clone(),
                target,
                playing,
            },
        ))
    }

    pub fn project_changed(&self, project: &Project, revision: u64) {
        self.source.project_changed(project);
        if let Some(timeline) = &self.timeline {
            timeline.project_changed(revision, project);
        }
    }
    pub fn proxy_ready(&self, media: &str, path: PathBuf) {
        self.source.proxy_ready(media, path);
        if let Some(timeline) = &self.timeline {
            timeline.proxy_ready(media);
        }
    }
    pub fn redraw(&self, monitor: Monitor) {
        match monitor {
            Monitor::Source => self.source.redraw(),
            Monitor::Timeline => {
                if let Some(timeline) = &self.timeline {
                    timeline.redraw();
                }
            }
        }
    }
    pub fn stop(&self) {
        self.source.stop();
        if let Some(timeline) = &self.timeline {
            timeline.stop();
        }
    }

    /// Tells the timeline monitor an order of the recording of automation (T16).
    ///
    /// # Errors
    /// The engine is headless (`-32601`), or the monitor has stopped.
    pub(crate) fn automation(&self, order: record::Order) -> Result<(), Fault> {
        let Some(timeline) = &self.timeline else {
            return Err(Fault::new(
                rpc::METHOD_NOT_FOUND,
                "there is no recording of automation in a headless engine: it plays nothing",
            ));
        };
        timeline.automation(order)
    }

    /// What writes a pass of the recording of automation (T16) from now on.
    pub(crate) fn on_record(&self, hook: record::Hook) {
        if let Some(timeline) = &self.timeline {
            let _ = timeline.automation(record::Order::Hook(hook));
        }
    }

    /// Whether the timeline monitor records automation as it plays (T16).
    pub(crate) fn recording(&self) -> bool {
        self.timeline
            .as_ref()
            .is_some_and(|timeline| timeline.status()["recording"] == true)
    }

    /// The timeline that the timeline monitor has loaded, if it has one.
    pub fn timeline_loaded(&self) -> Option<String> {
        let status = self.timeline.as_ref()?.status();
        status["timeline"].as_str().map(str::to_owned)
    }

    /// What a monitor shows now, for the scopes to measure.
    ///
    /// # Errors
    /// The engine is headless, or the monitor shows nothing.
    pub async fn shown(&self, monitor: Monitor) -> Result<Shown, Fault> {
        self.source.check("transport.state")?;
        match monitor {
            Monitor::Timeline => self.timeline.as_ref().expect("not headless").shown().await,
            Monitor::Source => {
                let snapshot = self.source.snapshot().await?;
                Ok(Shown::Picture {
                    picture: snapshot.picture,
                    rotation: snapshot.rotation,
                })
            }
        }
    }

    /// `viewer.wipe`, `viewer.split`: change what the timeline monitor compares its frame with.
    ///
    /// # Errors
    /// The engine is headless, the params are not what the method takes, or what they name
    /// cannot be found or read.
    pub async fn compare(&self, method: &str, params: Option<&RawValue>) -> Result<Value, Fault> {
        let Some(timeline) = &self.timeline else {
            return Err(Fault::new(
                rpc::METHOD_NOT_FOUND,
                format!("there is no method {method} in a headless engine: it has no monitor"),
            ));
        };
        let change = crate::compare::Change::read(method, params)?;
        timeline.compare(change).await
    }

    /// `still.grab`: the frame that the timeline monitor shows, before it is compared with
    /// anything, and the project and timeline it is of.
    ///
    /// # Errors
    /// The engine is headless, or no timeline is loaded.
    pub async fn original(&self, method: &str) -> Result<crate::stills::Frame, Fault> {
        let Some(timeline) = &self.timeline else {
            return Err(Fault::new(
                rpc::METHOD_NOT_FOUND,
                format!("there is no method {method} in a headless engine: it has no monitor"),
            ));
        };
        timeline.original().await
    }

    /// `viewer.highlight`: show a Highlight mode on the timeline monitor, or none.
    ///
    /// # Errors
    /// The engine is headless, or the params are not what the method takes.
    pub async fn highlight(&self, params: Option<&RawValue>) -> Result<Value, Fault> {
        let Some(timeline) = &self.timeline else {
            return Err(Fault::new(
                rpc::METHOD_NOT_FOUND,
                "there is no method viewer.highlight in a headless engine: it has no monitor",
            ));
        };
        let highlight = crate::highlight::Highlight::read(params)?;
        timeline.highlight(highlight).await
    }

    /// `viewer.bypass`: show the timeline monitor's picture without its grades (Bypass Color,
    /// p. 2987; Shift-D, pp. 504, 550), or with them.
    ///
    /// # Errors
    /// The engine is headless, or the params are not what the method takes (`-32602`; of the
    /// source monitor, `reason: monitor`).
    pub async fn bypass(&self, params: Option<&RawValue>) -> Result<Value, Fault> {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Asked {
            on: bool,
            #[serde(default)]
            monitor: Option<Monitor>,
        }
        let Some(timeline) = &self.timeline else {
            return Err(Fault::new(
                rpc::METHOD_NOT_FOUND,
                "there is no method viewer.bypass in a headless engine: it has no monitor",
            ));
        };
        let asked: Asked = rpc::params(
            "viewer.bypass",
            r#"{"on": true, "monitor"?: "timeline"}"#,
            params,
        )?;
        if asked.monitor == Some(Monitor::Source) {
            return Err(Fault::params(
                "the source monitor shows no grade: Bypass Color is of the timeline monitor",
            )
            .with_data(serde_json::json!({ "reason": "monitor" })));
        }
        timeline.bypass(asked.on).await
    }

    /// `viewer.preview` (a Live Preview of a LUT on a node, p. 3411) and `viewer.lut` (the Color
    /// Viewer LUT on the view or not, p. 672) of the timeline monitor (`crate::viewer_preview`).
    ///
    /// # Errors
    /// The engine is headless, or the params are not what the method takes (`-32602`).
    pub async fn viewer_preview(
        &self,
        method: &str,
        params: Option<&RawValue>,
    ) -> Result<Value, Fault> {
        let Some(timeline) = &self.timeline else {
            return Err(Fault::new(
                rpc::METHOD_NOT_FOUND,
                format!("there is no method {method} in a headless engine: it has no monitor"),
            ));
        };
        if method == "viewer.lut" {
            let on = crate::viewer_preview::viewer_lut(params)?;
            return timeline.viewer_lut(on).await;
        }
        let preview = crate::viewer_preview::Preview::read(params)?;
        timeline.preview(preview).await
    }

    pub async fn frame_digest(&self, params: Option<&RawValue>) -> Result<Value, Fault> {
        let method = "transport.frame_digest";
        self.check(method)?;
        let (monitor, params) = monitor_params(method, params)?;
        rpc::no_params(method, Some(&params))?;
        match monitor {
            Monitor::Timeline => self.timeline.as_ref().expect("not headless").digest().await,
            Monitor::Source => {
                let snapshot = self.source.snapshot().await?;
                let graphics = Arc::clone(&self.graphics);
                tokio::task::spawn_blocking(move || {
                    let frame = snapshot.picture.frame;
                    let (width, height) = columbina_compose::geometry::shown_size(
                        (snapshot.picture.width, snapshot.picture.height),
                        snapshot.rotation,
                    );
                    let gpu = graphics
                        .gpu()
                        .map_err(|error| drawing(&error.to_string()))?;
                    let mut compositor = Compositor::new(Arc::clone(&gpu));
                    let target = Target::preview(&gpu, width, height)
                        .map_err(|error| drawing(&error.to_string()))?;
                    let scene = Scene {
                        width,
                        height,
                        layers: vec![crate::render::picture_layer(
                            snapshot.picture,
                            snapshot.rotation,
                        )],
                        ..Scene::default()
                    };
                    compositor
                        .render(&scene, &target)
                        .map_err(|error| drawing(&error.to_string()))?;
                    digest(&target, Some(frame), Monitor::Source)
                })
                .await
                .map_err(|error| drawing(&error.to_string()))?
            }
        }
    }
}

enum TargetOrder {
    Source(Box<transport::Order>),
    Timeline(Option<Action>),
}
pub struct Order {
    transport: Transport,
    target: TargetOrder,
    playing: bool,
}

impl Order {
    /// Serialized across connections. A play command is sent only after the other worker
    /// confirms its pause, including stopping its audio output.
    pub async fn send(self) {
        let _one = self.transport.orders.lock().await;
        if self.playing {
            let paused = match &self.target {
                TargetOrder::Source(_) => {
                    self.transport
                        .timeline
                        .as_ref()
                        .expect("not headless")
                        .pause()
                        .await
                }
                TargetOrder::Timeline(_) => self.transport.source.pause_for_switch().await,
            };
            if let Err(error) = paused {
                log("transport_error", &json!({"message":error.message}));
                return;
            }
        }
        match self.target {
            TargetOrder::Source(order) => order.send(),
            TargetOrder::Timeline(action) => {
                if let Some(action) = action {
                    self.transport
                        .timeline
                        .as_ref()
                        .expect("not headless")
                        .act(action);
                }
            }
        }
    }
}

fn drawing(message: &str) -> Fault {
    Fault::new(rpc::TRANSPORT_FAILED, message).with_data(json!({"reason":"draw"}))
}

/// Only called by --test-hooks: a digest compares whole rendered frames in regression tests.
/// The four top-left RGBA pixels also make colour assertions independent of the digest.
fn digest(target: &Target, frame: Option<i64>, monitor: Monitor) -> Result<Value, Fault> {
    let pixels = target
        .read_rgba()
        .map_err(|error| drawing(&error.to_string()))?;
    let hash = format!("{:x}", Sha256::digest(&pixels));
    let corners: Vec<_> = pixels.as_chunks::<4>().0.iter().take(4).copied().collect();
    let (width, height) = target.size();
    Ok(
        json!({"monitor":monitor,"frame":frame,"width":width,"height":height,"hash":hash,"pixels":corners}),
    )
}
