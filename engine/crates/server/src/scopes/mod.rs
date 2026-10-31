//! The video scopes of the monitors (`scopes.subscribe`, `scopes.set`, `scopes.unsubscribe`),
//! and the figures of the frame that a monitor shows (`scopes.statistics`), as `PROTOCOL.md`
//! tells them (Scopes).
//!
//! # Who counts
//!
//! A monitor tells [`Scopes::shown`] each frame it shows: the timeline monitor the target it
//! composed the frame into, once the compositor has handed the frame to the card, and the source
//! monitor the picture it hands its view. Nothing is kept while no scope is subscribed to that
//! monitor. A thread of the scopes (`columbina-scopes`) then counts each scope that is owed a
//! frame on the card, from that very texture (held until it is counted, so that the monitor draws
//! its next frame into another), reads its picture back, and queues it on the connection that
//! subscribed, as `scopes.frame`. The picture of the source monitor is first composed alone, as
//! the source monitor's test hook composes it, since its view draws it from the picture itself.
//!
//! # How often
//!
//! A scope is owed a frame when it is subscribed, when its settings change, and when its monitor
//! shows a new frame, playing or paused. It is counted at most once every [`EVERY`] (30 a
//! second): frames that its monitor shows meanwhile are passed over, and the last is counted once
//! the time has come, so a scope always ends on the frame that stays. A scope whose last frame
//! the client has not yet read is not counted again until it has, so that a client that reads
//! slowly is sent fewer frames rather than more than it reads.
//!
//! # A card that is lost
//!
//! The scopes are made on the card of the frame they count; the pipelines and every scope are
//! made again when that card is another (it was lost and made again). A frame that cannot be
//! counted or read is tried again a second later, and is passed over once a newer one is shown.

pub mod settings;

use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use base64::Engine as _;
use columbina_compose::scopes::{self as compose, Sampling, Scope, ScopeImage};
use columbina_compose::{Compositor, Gpu, Scene, Target};
use columbina_media::NativePicture;
use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::{json, Value};

use crate::gpu::Graphics;
use crate::log::log;
use crate::queue::{self, Refused};
use crate::rpc::{self, Fault, Line};
use crate::view::Monitor;

/// The least time between two frames of a scope: 30 a second.
pub const EVERY: Duration = Duration::from_nanos(1_000_000_000 / 30);

/// How many scopes a connection may have subscribed at once (a floating window of 9 scopes, and
/// the scope docked, with room to spare).
pub const MOST_A_CONNECTION: usize = 16;

/// How long after a frame could not be counted or read it is tried again.
const AGAIN: Duration = Duration::from_secs(1);

/// How often a scope whose last frame waits to be written is looked at.
const POLL: Duration = Duration::from_millis(5);

/// The histogram of `scopes.statistics` where none is asked for.
const BINS: u32 = 256;

/// What a monitor shows: a frame composed into a target of the card (the timeline monitor), or
/// a picture that its view draws itself (the source monitor).
#[derive(Clone)]
pub enum Shown {
    Target {
        gpu: Arc<Gpu>,
        target: Arc<Target>,
        frame: i64,
    },
    Picture {
        picture: Arc<NativePicture>,
        rotation: i16,
    },
}

impl Shown {
    /// The frame shown: of the timeline, or of the media.
    fn frame(&self) -> i64 {
        match self {
            Shown::Target { frame, .. } => *frame,
            Shown::Picture { picture, .. } => picture.frame,
        }
    }
}

/// The scopes of the engine, shared by the connections and the monitors.
#[derive(Clone)]
pub struct Scopes(Arc<Inner>);

struct Inner {
    /// A headless engine has no monitor, and so no scopes.
    headless: bool,
    graphics: Arc<Graphics>,
    hub: Mutex<Hub>,
    /// Told when a scope is owed a frame, or the engine ends.
    wake: Condvar,
    /// The pipelines of the scopes, on the card they were made on.
    pipelines: Mutex<Option<Arc<compose::Scopes>>>,
}

#[derive(Default)]
struct Hub {
    subscribed: BTreeMap<u64, Subscribed>,
    next: u64,
    /// What each monitor shows (by [`Monitor::index`]), kept while a scope is subscribed to it.
    shown: [Option<Shown>; 2],
    /// Whether the thread of the scopes was started.
    started: bool,
    stopping: bool,
}

/// A scope subscribed by a connection.
struct Subscribed {
    connection: u64,
    queue: queue::Sender,
    view: Monitor,
    settings: compose::ScopeSettings,
    /// Raised by each change of settings: a frame counted by settings before is not sent.
    version: u64,
    /// Whether a frame is to be counted and sent.
    owed: bool,
    /// Whether a frame is being counted and read.
    reading: bool,
    /// Not counted before then.
    due: Instant,
    /// How many messages the writer of the connection has to have written for the last frame
    /// sent to have been written, until it has.
    unwritten: Option<u64>,
    /// Whether the last frame failed (so that a failure is logged once, not every second).
    failing: bool,
}

/// A frame to count for a scope.
struct Job {
    id: u64,
    version: u64,
    view: Monitor,
    settings: compose::ScopeSettings,
    shown: Shown,
}

/// The card that the thread of the scopes counts on, with the scopes made on it.
struct Card {
    gpu: Arc<Gpu>,
    pipelines: Arc<compose::Scopes>,
    /// Each scope by its id, with the version of the settings it was made by.
    made: BTreeMap<u64, (u64, Scope)>,
    /// What composes the pictures of the source monitor.
    compositor: Option<Compositor>,
    /// The last picture composed, by its address, and the target it was composed into.
    composed: Option<(usize, Arc<Target>)>,
}

/// The id of a scope as it is written.
fn written_id(id: u64) -> String {
    format!("scope-{id}")
}

/// The scope that a written id names.
fn read_id(id: &str) -> Option<u64> {
    id.strip_prefix("scope-")?.parse().ok()
}

fn missing(id: &str) -> Fault {
    Fault::params(format!("there is no scope {id} of this connection"))
        .with_data(json!({"reason": "missing", "scope_id": id}))
}

/// Read the settings of a scope, as `scopes.subscribe` and `scopes.set` take them.
fn read_settings(value: Value) -> Result<(settings::Settings, compose::ScopeSettings), Fault> {
    let refused = |message: String| {
        Fault::params(format!("the settings of the scope: {message}"))
            .with_data(json!({"reason": "settings", "message": message}))
    };
    let written: settings::Settings =
        serde_json::from_value(value).map_err(|error| refused(error.to_string()))?;
    let made = written.compose().map_err(refused)?;
    Ok((written, made))
}

/// What `scopes.subscribe` and `scopes.set` answer: the scope, the size of its picture, its
/// graticule, and its settings in full.
fn described(id: u64, written: &settings::Settings, made: &compose::ScopeSettings) -> Value {
    let (width, height) = made.kind.picture_size();
    json!({
        "scope_id": written_id(id),
        "size": {"width": width, "height": height},
        "graticule": settings::graticule(&compose::graticule::graticule(made)),
        "settings": written,
    })
}

/// How a request that a scope method answers by itself was answered: queued, or not, and the
/// connection then to be closed.
pub type Answered = Result<(), Refused>;

/// What the monitors are asked for: that one draws again what it shows.
pub type Redraw<'a> = &'a dyn Fn(Monitor);

impl Scopes {
    pub fn new(headless: bool, graphics: Arc<Graphics>) -> Scopes {
        Scopes(Arc::new(Inner {
            headless,
            graphics,
            hub: Mutex::new(Hub::default()),
            wake: Condvar::new(),
            pipelines: Mutex::new(None),
        }))
    }

    fn hub(&self) -> MutexGuard<'_, Hub> {
        self.0.lock()
    }

    /// Refuse a method of the scopes in a headless engine, which has no monitors.
    ///
    /// # Errors
    /// The engine is headless.
    pub fn check(&self, method: &str) -> Result<(), Fault> {
        if self.0.headless {
            Err(Fault::new(
                rpc::METHOD_NOT_FOUND,
                format!(
                    "there is no method {method} in a headless engine: it has no monitor to \
                     measure"
                ),
            ))
        } else {
            Ok(())
        }
    }

    /// `scopes.subscribe`: a scope of `view` for the connection `connection`, whose answer is
    /// queued on `sender` before any frame of it.
    ///
    /// # Errors
    /// The engine is headless, the params are not those of the method, the settings are out of
    /// range, or the connection has as many scopes as it may.
    pub fn subscribe(
        &self,
        connection: u64,
        sender: &queue::Sender,
        request: &RawValue,
        params: Option<&RawValue>,
        redraw: Redraw<'_>,
    ) -> Result<Answered, Fault> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Subscribe {
            view: Monitor,
            settings: Value,
        }
        let method = "scopes.subscribe";
        self.check(method)?;
        let asked: Subscribe = rpc::params(
            method,
            r#"{"view": "timeline" | "source", "settings": {"kind": "waveform", ...}}"#,
            params,
        )?;
        let (written, made) = read_settings(asked.settings)?;
        let mut hub = self.hub();
        let had = hub
            .subscribed
            .values()
            .filter(|scope| scope.connection == connection)
            .count();
        if had >= MOST_A_CONNECTION {
            return Err(Fault::params(format!(
                "a connection may have at most {MOST_A_CONNECTION} scopes"
            ))
            .with_data(json!({"reason": "too_many", "most": MOST_A_CONNECTION})));
        }
        hub.next += 1;
        let id = hub.next;
        let line = rpc::result_line(request, &described(id, &written, &made));
        let answered = sender.push(line);
        if answered.is_ok() {
            hub.subscribed.insert(
                id,
                Subscribed {
                    connection,
                    queue: sender.clone(),
                    view: asked.view,
                    settings: made,
                    version: 0,
                    owed: true,
                    reading: false,
                    due: Instant::now(),
                    unwritten: None,
                    failing: false,
                },
            );
            self.start(&mut hub);
        }
        let shown = hub.shown[asked.view.index()].is_some();
        drop(hub);
        self.0.wake.notify_all();
        if answered.is_ok() && !shown {
            redraw(asked.view);
        }
        Ok(answered)
    }

    /// `scopes.set`: other settings for a scope of the connection, and a frame by them.
    ///
    /// # Errors
    /// The engine is headless, the params are not those of the method, the settings are out of
    /// range, or the connection has no such scope.
    pub fn set(
        &self,
        connection: u64,
        sender: &queue::Sender,
        request: &RawValue,
        params: Option<&RawValue>,
        redraw: Redraw<'_>,
    ) -> Result<Answered, Fault> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Set {
            scope_id: String,
            settings: Value,
        }
        let method = "scopes.set";
        self.check(method)?;
        let asked: Set = rpc::params(
            method,
            r#"{"scope_id": "scope-1", "settings": {"kind": "waveform", ...}}"#,
            params,
        )?;
        let (written, made) = read_settings(asked.settings)?;
        let mut hub = self.hub();
        let id = read_id(&asked.scope_id)
            .filter(|id| {
                hub.subscribed
                    .get(id)
                    .is_some_and(|scope| scope.connection == connection)
            })
            .ok_or_else(|| missing(&asked.scope_id))?;
        let line = rpc::result_line(request, &described(id, &written, &made));
        let answered = sender.push(line);
        let scope = hub.subscribed.get_mut(&id).expect("found above");
        scope.settings = made;
        scope.version += 1;
        scope.owed = true;
        scope.due = Instant::now();
        let view = scope.view;
        let shown = hub.shown[view.index()].is_some();
        drop(hub);
        self.0.wake.notify_all();
        if !shown {
            redraw(view);
        }
        Ok(answered)
    }

    /// `scopes.unsubscribe`: no frame of the scope is sent after the answer.
    ///
    /// # Errors
    /// The engine is headless, the params are not those of the method, or the connection has no
    /// such scope.
    pub fn unsubscribe(
        &self,
        connection: u64,
        sender: &queue::Sender,
        request: &RawValue,
        params: Option<&RawValue>,
    ) -> Result<Answered, Fault> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Unsubscribe {
            scope_id: String,
        }
        let method = "scopes.unsubscribe";
        self.check(method)?;
        let asked: Unsubscribe = rpc::params(method, r#"{"scope_id": "scope-1"}"#, params)?;
        let mut hub = self.hub();
        let id = read_id(&asked.scope_id)
            .filter(|id| {
                hub.subscribed
                    .get(id)
                    .is_some_and(|scope| scope.connection == connection)
            })
            .ok_or_else(|| missing(&asked.scope_id))?;
        hub.subscribed.remove(&id);
        tidy(&mut hub);
        Ok(sender.push(rpc::result_line(request, &json!({}))))
    }

    /// Forget the scopes of a connection that has ended.
    pub fn forget(&self, connection: u64) {
        let mut hub = self.hub();
        hub.subscribed
            .retain(|_, scope| scope.connection != connection);
        tidy(&mut hub);
    }

    /// What a monitor shows now: each scope of it is owed a frame. Kept only while a scope is
    /// subscribed to the monitor, so it costs nothing otherwise.
    pub fn shown(&self, view: Monitor, shown: Shown) {
        let mut hub = self.hub();
        let mut any = false;
        for scope in hub.subscribed.values_mut() {
            if scope.view == view {
                scope.owed = true;
                any = true;
            }
        }
        if any {
            hub.shown[view.index()] = Some(shown);
            drop(hub);
            self.0.wake.notify_all();
        }
    }

    /// A monitor shows nothing (it was unloaded).
    pub fn cleared(&self, view: Monitor) {
        self.hub().shown[view.index()] = None;
    }

    /// End the thread of the scopes, as the engine ends.
    pub fn stop(&self) {
        let mut hub = self.hub();
        hub.stopping = true;
        hub.subscribed.clear();
        hub.shown = [None, None];
        drop(hub);
        self.0.wake.notify_all();
    }

    /// `scopes.statistics` of what a monitor shows, as it was given: worked out on the card.
    /// This blocks: call it where blocking is allowed.
    ///
    /// # Errors
    /// The bins are out of range, or the frame cannot be measured (the card was lost).
    pub fn statistics(&self, view: Monitor, shown: &Shown, bins: u32) -> Result<Value, Fault> {
        let drawing = |message: String| {
            Fault::new(rpc::TRANSPORT_FAILED, message).with_data(json!({"reason": "draw"}))
        };
        let (gpu, texture) = match shown {
            Shown::Target { gpu, target, .. } => (Arc::clone(gpu), Arc::clone(target.texture())),
            Shown::Picture { picture, rotation } => {
                let gpu = self
                    .0
                    .graphics
                    .gpu()
                    .map_err(|error| drawing(error.to_string()))?;
                let mut compositor = Compositor::new(Arc::clone(&gpu));
                let target = self
                    .0
                    .compose(&gpu, &mut compositor, picture, *rotation, None)
                    .map_err(drawing)?;
                (gpu, Arc::clone(target.texture()))
            }
        };
        let pipelines = self.0.pipelines(&gpu);
        let figures = pipelines
            .statistics(&texture, &compose::Signal::default(), bins)
            .map_err(|error| drawing(error.to_string()))?;
        let mut value = settings::statistics(&figures);
        value["view"] = json!(view);
        value["frame"] = json!(shown.frame());
        value["width"] = json!(texture.width());
        value["height"] = json!(texture.height());
        Ok(value)
    }

    /// Start the thread of the scopes, once.
    fn start(&self, hub: &mut Hub) {
        if hub.started {
            return;
        }
        hub.started = true;
        let inner = Arc::clone(&self.0);
        let started = std::thread::Builder::new()
            .name("columbina-scopes".into())
            .spawn(move || inner.run());
        if let Err(error) = started {
            log(
                "scopes_failed",
                &json!({"message": format!("the thread of the scopes cannot be started: {error}")}),
            );
        }
    }
}

/// The params of `scopes.statistics`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatisticsRequest {
    pub view: Monitor,
    #[serde(default = "bins")]
    pub bins: u32,
}

fn bins() -> u32 {
    BINS
}

impl StatisticsRequest {
    /// Read the params of `scopes.statistics`.
    ///
    /// # Errors
    /// They are not those of the method, or the bins are not from 16 to 1024.
    pub fn read(params: Option<&RawValue>) -> Result<StatisticsRequest, Fault> {
        let asked: StatisticsRequest = rpc::params(
            "scopes.statistics",
            r#"{"view": "timeline" | "source", "bins": 256}"#,
            params,
        )?;
        if !(16..=1024).contains(&asked.bins) {
            return Err(Fault::params(format!(
                "the bins of the histogram are {}, not from 16 to 1024",
                asked.bins
            ))
            .with_data(json!({"reason": "bins"})));
        }
        Ok(asked)
    }
}

/// Let go of what a monitor shows once no scope is subscribed to it.
fn tidy(hub: &mut Hub) {
    for view in [Monitor::Source, Monitor::Timeline] {
        if !hub.subscribed.values().any(|scope| scope.view == view) {
            hub.shown[view.index()] = None;
        }
    }
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, Hub> {
        self.hub.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The pipelines of the scopes on `gpu`, made if they are not there (some 10 ms).
    fn pipelines(&self, gpu: &Arc<Gpu>) -> Arc<compose::Scopes> {
        let mut pipelines = self
            .pipelines
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match &*pipelines {
            Some(made) if Arc::ptr_eq(made.gpu(), gpu) => Arc::clone(made),
            _ => {
                let made = Arc::new(compose::Scopes::new(Arc::clone(gpu)));
                *pipelines = Some(Arc::clone(&made));
                made
            }
        }
    }

    /// Compose a picture alone, as the source monitor shows it, into `into` if it is of its size
    /// and no one else holds it, or else into a new target.
    fn compose(
        &self,
        gpu: &Arc<Gpu>,
        compositor: &mut Compositor,
        picture: &Arc<NativePicture>,
        rotation: i16,
        into: Option<Arc<Target>>,
    ) -> Result<Arc<Target>, String> {
        let (width, height) =
            columbina_compose::geometry::shown_size((picture.width, picture.height), rotation);
        let target = match into {
            Some(target)
                if target.size() == (width, height)
                    && Arc::strong_count(&target) == 1
                    && Arc::ptr_eq(compositor.gpu(), gpu) =>
            {
                target
            }
            _ => Arc::new(Target::preview(gpu, width, height).map_err(|error| error.to_string())?),
        };
        let scene = Scene {
            width,
            height,
            layers: vec![crate::render::picture_layer(Arc::clone(picture), rotation)],
            ..Scene::default()
        };
        let _turn = self.graphics.draw_turn(true);
        compositor
            .render(&scene, &target)
            .map_err(|error| error.to_string())?;
        Ok(target)
    }

    /// The thread of the scopes: count each scope that is owed a frame, once it is due, read it
    /// back and send it, until the engine ends.
    fn run(self: Arc<Self>) {
        let mut card: Option<Card> = None;
        let mut hub = self.lock();
        loop {
            if hub.stopping {
                return;
            }
            let now = Instant::now();
            let mut wait: Option<Duration> = None;
            let mut jobs = Vec::new();
            let Hub {
                subscribed, shown, ..
            } = &mut *hub;
            for (&id, scope) in subscribed.iter_mut() {
                if !scope.owed || scope.reading {
                    continue;
                }
                let Some(shown) = &shown[scope.view.index()] else {
                    continue;
                };
                if let Some(index) = scope.unwritten {
                    if scope.queue.written_count() < index {
                        wait = Some(wait.map_or(POLL, |wait| wait.min(POLL)));
                        continue;
                    }
                    scope.unwritten = None;
                }
                if now < scope.due {
                    let until = scope.due - now;
                    wait = Some(wait.map_or(until, |wait| wait.min(until)));
                    continue;
                }
                scope.owed = false;
                scope.reading = true;
                scope.due = now + EVERY;
                jobs.push(Job {
                    id,
                    version: scope.version,
                    view: scope.view,
                    settings: scope.settings.clone(),
                    shown: shown.clone(),
                });
            }
            if let Some(card) = &mut card {
                card.made.retain(|id, _| subscribed.contains_key(id));
            }
            if jobs.is_empty() {
                hub = match wait {
                    Some(wait) => {
                        self.wake
                            .wait_timeout(hub, wait)
                            .unwrap_or_else(PoisonError::into_inner)
                            .0
                    }
                    None => self.wake.wait(hub).unwrap_or_else(PoisonError::into_inner),
                };
                continue;
            }
            drop(hub);
            let done: Vec<(Job, Result<Line, String>)> = jobs
                .into_iter()
                .map(|job| {
                    let result = self
                        .count(&mut card, &job)
                        .map(|image| frame_line(&job, &image));
                    (job, result)
                })
                .collect();
            hub = self.lock();
            for (job, result) in done {
                deliver(&mut hub, &job, result);
            }
        }
    }

    /// Count the frame of a job into its scope on the card, and read its picture back.
    fn count(&self, card: &mut Option<Card>, job: &Job) -> Result<ScopeImage, String> {
        let gpu = match &job.shown {
            Shown::Target { gpu, .. } => Arc::clone(gpu),
            Shown::Picture { .. } => self.graphics.gpu().map_err(|error| error.to_string())?,
        };
        if let Some(why) = gpu.lost_reason() {
            return Err(format!("the graphics device was lost: {why}"));
        }
        if card
            .as_ref()
            .is_none_or(|card| !Arc::ptr_eq(&card.gpu, &gpu))
        {
            *card = Some(Card {
                pipelines: self.pipelines(&gpu),
                gpu: Arc::clone(&gpu),
                made: BTreeMap::new(),
                compositor: None,
                composed: None,
            });
        }
        let card = card.as_mut().expect("made above");
        let texture = match &job.shown {
            Shown::Target { target, .. } => Arc::clone(target.texture()),
            Shown::Picture { picture, rotation } => {
                let address = Arc::as_ptr(picture) as usize;
                match &card.composed {
                    Some((composed, target)) if *composed == address => {
                        Arc::clone(target.texture())
                    }
                    _ => {
                        let into = card.composed.take().map(|(_, target)| target);
                        let compositor = card
                            .compositor
                            .get_or_insert_with(|| Compositor::new(Arc::clone(&gpu)));
                        let target = self.compose(&gpu, compositor, picture, *rotation, into)?;
                        let texture = Arc::clone(target.texture());
                        card.composed = Some((address, target));
                        texture
                    }
                }
            }
        };
        let fresh = card
            .made
            .get(&job.id)
            .is_none_or(|(version, _)| *version != job.version);
        if fresh {
            let scope = card
                .pipelines
                .scope(job.settings.clone())
                .map_err(|error| error.to_string())?;
            card.made.insert(job.id, (job.version, scope));
        }
        let (_, scope) = &card.made[&job.id];
        card.pipelines
            .draw(
                &texture,
                scope,
                Sampling::auto(texture.width(), texture.height()),
            )
            .map_err(|error| error.to_string())?;
        card.pipelines
            .read_image(scope)
            .map_err(|error| error.to_string())
    }
}

/// The line of `scopes.frame` of a picture of a scope.
fn frame_line(job: &Job, image: &ScopeImage) -> Line {
    rpc::notification_line(
        "scopes.frame",
        &json!({
            "scope_id": written_id(job.id),
            "view": job.view,
            "frame": job.shown.frame(),
            "width": image.width,
            "height": image.height,
            "rgba": base64::engine::general_purpose::STANDARD.encode(&image.rgba),
        }),
    )
}

/// Send what came of a job, if its scope is still subscribed with the settings it was counted
/// by: a frame is queued on its connection; a failure is tried again later.
fn deliver(hub: &mut Hub, job: &Job, result: Result<Line, String>) {
    let Some(scope) = hub.subscribed.get_mut(&job.id) else {
        return;
    };
    scope.reading = false;
    if scope.version != job.version {
        return;
    }
    let again = |scope: &mut Subscribed| {
        scope.owed = true;
        scope.due = Instant::now() + AGAIN;
    };
    match result {
        Ok(line) => match scope.queue.push_counted(line) {
            Ok(index) => {
                scope.unwritten = Some(index);
                scope.failing = false;
            }
            Err(_) => again(scope),
        },
        Err(message) => {
            if !scope.failing {
                log(
                    "scope_failed",
                    &json!({"scope_id": written_id(job.id), "message": message}),
                );
            }
            scope.failing = true;
            again(scope);
        }
    }
}
