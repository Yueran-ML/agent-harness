//! The engine as its clients see it: the project and its history, the connections that said
//! hello, and what each method does.
//!
//! Every change to the project is made while [`State`] is locked, and so is everything that is
//! sent about it: the notification of a change is queued on every connection that said hello,
//! then the answer to whoever asked for it, all before the lock is let go. So each connection
//! is sent the changes in the order of their revisions, and a client sees its own change as a
//! notification before it sees the answer to its request. Nothing is awaited while the lock is
//! held, and nothing is logged: what is to be logged is kept and logged once it is let go.
//!
//! A transaction of a gesture (`transaction.apply` with `gesture`, a control dragged) is applied
//! and drawn at once, but told at most every [`TELL_EVERY`]: what is not told yet is held, and
//! told as one `project.changed` (with the revision it applies to in `base`), once the time has
//! come, as the gesture ends, and before anything else is told of the project. So a client that
//! sends a gesture may be answered before it is told; every client is still told every change, in
//! order, and one that asks for the project is told all that was held first.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use columbina_media::MediaError;
use columbina_model::grade::Target as GradeTarget;
use columbina_model::{Applied, Change, History, Media, Op, Project, Proxy};
use columbina_store::Saver;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_json::{json, Value};
use tokio::sync::{watch, Mutex, MutexGuard, Notify, Semaphore};

use crate::analysis::{Analysis, ThumbnailRequest, WaveformRequest};
use crate::gpu::Graphics;
use crate::grading::LutPlaces;
use crate::log::log;
use crate::luts;
use crate::names;
use crate::play::Transport;
use crate::probe::{self, NodeAt, Place, Probe};
use crate::proxy::{self, Made, Note, Proxies};
use crate::queue::{self, Limits, Refused};
use crate::render::Renders;
use crate::rpc::{self, Fault, Line};
use crate::scopes::{Scopes, Shown, StatisticsRequest};
use crate::transport;
use crate::view::{Monitor, Viewer};

#[cfg(test)]
mod tests;
mod tracker;
mod window;

/// The version of the engine that `hello` answers with.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The least time between two `project.changed` of the transactions of a gesture: 10 a second.
pub const TELL_EVERY: Duration = Duration::from_millis(100);

/// How long a gesture that is sent nothing more goes on: its next transaction after this begins
/// another, a step of its own. A client that ends its gestures never meets it.
pub const GESTURE_IDLE: Duration = Duration::from_secs(10);

/// The longest id of a gesture.
const LONGEST_GESTURE: usize = 64;

/// What kind of program a client is, as it says in `hello`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// The desktop app.
    Desktop,
    /// The agent.
    Agent,
    /// A command of the command line.
    Cli,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Desktop => "desktop",
            Kind::Agent => "agent",
            Kind::Cli => "cli",
        }
    }
}

/// Who is connected, for an engine that ends once no one has been for a while.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Presence {
    /// How many connections are open.
    pub connected: usize,
    /// How many connections were ever made: it changes with each one, however soon it ends.
    pub arrivals: u64,
}

/// The engine: what all the connections share.
pub struct Engine {
    /// The project, its history, and who is told of its changes.
    pub state: Arc<Mutex<State>>,
    /// The directory of the project, as `hello` answers it.
    pub project: String,
    /// The id of the process of the engine.
    pub pid: u32,
    /// How much may wait to be sent on a connection before it is closed.
    pub limits: Limits,
    /// Who is connected.
    pub presence: watch::Sender<Presence>,
    /// Every open connection, so that all can be closed as the engine ends.
    connections: std::sync::Mutex<Connections>,
    next_connection: AtomicU64,
    /// The files that `media.import` may read at once, of all its calls together.
    reading: Arc<Semaphore>,
    /// For the tests: a file that `media.import` waits for before it reads each file.
    read_gate: Option<PathBuf>,
    /// The view that the picture is drawn in, if one is attached.
    viewer: Viewer,
    /// The transport of the source monitor.
    transport: Transport,
    /// The maker of proxies.
    proxies: Proxies,
    /// Background thumbnails and waveforms; they never write the project history.
    analysis: Analysis,
    /// One background export, independent of playback and project edits.
    renders: Arc<Renders>,
    /// The scopes of the monitors.
    scopes: Scopes,
    /// The connection that turned the Live Preview on last (`viewer.preview`), held while one
    /// is turned on or off: it is ended as that connection ends, as no client is left to end it.
    preview_owner: Mutex<Option<u64>>,
    /// Where the LUTs that grades name are looked for, for `luts.list`.
    luts: LutPlaces,
    /// What draws frames apart from the monitors, to be measured.
    probe: Arc<Probe>,
    /// Whether the engine is headless, as `hello` answers it.
    headless: bool,
    /// Told when the engine is to end, having yielded the project to the desktop
    /// (`engine.yield`).
    pub yielded: Arc<Notify>,
}

#[derive(Default)]
struct Connections {
    /// Whether the engine is ending, and takes no more.
    closing: bool,
    /// What stops each.
    stops: HashMap<u64, watch::Sender<bool>>,
}

/// What is changed only while it is locked.
pub struct State {
    history: History,
    window: window::WindowState,
    /// The window track going on, if one is (T24).
    tracks: tracker::Tracks,
    /// What saves the project after each change. It is taken when the engine ends.
    pub saver: Option<Saver>,
    /// The connections that said hello, each of which is sent every notification.
    peers: HashMap<u64, Peer>,
    /// The connections that were closed because they did not read, to be logged once the
    /// lock is let go.
    kicked: Vec<(u64, Kind)>,
    /// The transport, told when the project changes, so that it lets go of media that was
    /// removed.
    transport: Option<Transport>,
    /// The maker of proxies, told when the project changes, so that it makes none of media that
    /// was removed.
    proxies: Proxies,
    /// When the gesture that is going on was last sent a transaction.
    gesture_last: Option<Instant>,
    /// Sequence for native gestures; their keys cannot be supplied by a wire client.
    next_owned_gesture: u64,
    /// What the transactions of a gesture changed that is not told yet.
    held: Option<Held>,
    /// When a change was last told.
    told_at: Option<Instant>,
    /// When what is held is to be told, once a task waits for it.
    tell_at: Option<Instant>,
}

/// What a gesture changed that is not told yet: the revision that it applies to, and what was
/// done since, as the last of it was done.
struct Held {
    base: u64,
    revision: u64,
    done: Vec<Op>,
    by: String,
    made_by: String,
    next_id: u64,
}

/// A connection that said hello.
struct Peer {
    kind: Kind,
    queue: queue::Sender,
    stop: watch::Sender<bool>,
}

/// A connection, as its reader holds it.
pub struct Connection {
    /// The number of the connection, for the log.
    pub id: u64,
    /// What the client said it is, once it said hello.
    kind: Option<Kind>,
    /// The messages to be sent on it.
    queue: queue::Sender,
    /// Set to stop it.
    stop: watch::Sender<bool>,
}

impl Connection {
    pub fn new(id: u64, queue: queue::Sender, stop: watch::Sender<bool>) -> Self {
        Self {
            id,
            kind: None,
            queue,
            stop,
        }
    }

    /// What the client said it is, once it said hello.
    pub fn kind(&self) -> Option<Kind> {
        self.kind
    }

    /// Queue a message to be sent, without waiting.
    pub fn queue(&self, line: Line) -> Result<(), Refused> {
        self.queue.push(line)
    }

    /// Where messages are queued on it, for what sends to it later.
    pub fn sender(&self) -> &queue::Sender {
        &self.queue
    }
}

/// Log that a connection was closed because its client does not read what it is sent.
pub fn log_kicked(id: u64, kind: Option<Kind>, limits: Limits) {
    log(
        "kicked",
        &json!({
            "connection": id,
            "client": kind,
            "reason": format!(
                "the client does not read what it is sent: {} messages or {} bytes wait for it",
                limits.messages, limits.bytes,
            ),
        }),
    );
}

impl Engine {
    #[allow(
        clippy::too_many_arguments,
        clippy::too_many_lines,
        reason = "what the engine is made of, each from the command line or `main`"
    )]
    pub fn new(
        history: History,
        saver: Saver,
        project: String,
        limits: Limits,
        read_gate: Option<PathBuf>,
        headless: bool,
        setup: transport::Setup,
        proxy_setup: &proxy::Setup,
    ) -> Engine {
        let proxies = Proxies::new(proxy_setup, Path::new(&project));
        let state = Arc::new(Mutex::new(State {
            history,
            window: window::WindowState::default(),
            tracks: tracker::Tracks::default(),
            saver: Some(saver),
            peers: HashMap::new(),
            kicked: Vec::new(),
            transport: None,
            proxies: proxies.clone(),
            gesture_last: None,
            next_owned_gesture: 0,
            held: None,
            told_at: None,
            tell_at: None,
        }));
        let graphics = Graphics::new();
        let viewer = Viewer::new(headless, Arc::clone(&graphics));
        let analysis = Analysis::new(
            Path::new(&project),
            Box::new({
                let state = Arc::clone(&state);
                move |media: &Media, stream: Option<usize>| {
                    tell_waveform(&state, media, stream, limits);
                }
            }),
        );
        let notify = {
            let state = Arc::clone(&state);
            Arc::new(move |status: &serde_json::Value| tell(&state, status, limits))
        };
        let again = {
            let state = Arc::clone(&state);
            let dir = proxy::proxy_dir(Path::new(&project));
            Arc::new(move |media: &str, damaged: Option<&Path>| {
                if let Some(damaged) = damaged {
                    proxy::delete(&dir, damaged);
                }
                make_again(&state, media, false);
            })
        };
        let repair = {
            let state = Arc::clone(&state);
            Arc::new(move |media: &str, _: Option<&Path>| make_again(&state, media, true))
        };
        let luts = LutPlaces::new(Path::new(&project));
        let renders = Arc::new(Renders::new(
            Path::new(&project),
            Arc::clone(&graphics),
            luts.clone(),
            {
                let state = Arc::clone(&state);
                Arc::new(move |note| tell_render(&state, note, limits))
            },
        ));
        let scopes = Scopes::new(headless, Arc::clone(&graphics));
        let probe = Arc::new(Probe::new(Arc::clone(&graphics), luts.clone()));
        let transport = Transport::new(
            headless,
            setup,
            &viewer,
            graphics,
            luts.clone(),
            notify,
            again,
            repair,
            &scopes,
        );
        // A pass of the recording of automation (T16) is written as one transaction.
        transport.on_record(passes_written(&state, limits));
        let project_now = {
            let mut locked = state.try_lock().expect("no one has the state yet");
            locked.transport = Some(transport.clone());
            locked.history.project().clone()
        };
        proxies.start(
            &project_now,
            proxy::Hooks {
                notify: Box::new({
                    let state = Arc::clone(&state);
                    move |note: &Note| tell_proxy(&state, note, limits)
                }),
                made: Box::new({
                    let state = Arc::clone(&state);
                    let dir = proxy::proxy_dir(Path::new(&project));
                    move |media: &str, proxy: &Proxy| proxy_made(&state, &dir, media, proxy, limits)
                }),
                twin: Box::new({
                    let state = Arc::clone(&state);
                    move |media: &str| {
                        proxy::twin_of(state.blocking_lock().history.project(), media)
                    }
                }),
            },
        );
        Engine {
            state,
            project,
            pid: std::process::id(),
            limits,
            presence: watch::Sender::new(Presence::default()),
            connections: std::sync::Mutex::default(),
            next_connection: AtomicU64::new(1),
            reading: Arc::new(Semaphore::new(READ_AT_ONCE)),
            read_gate,
            viewer,
            transport,
            proxies,
            analysis,
            renders,
            scopes,
            preview_owner: Mutex::new(None),
            luts,
            probe,
            headless,
            yielded: Arc::new(Notify::new()),
        }
    }

    /// What is told when the engine is to end, having yielded the project.
    fn ending(&self) -> Arc<Notify> {
        Arc::clone(&self.yielded)
    }

    /// Cancel analysis and stop the proxy being made, deleting its partial output. This
    /// blocks for a few seconds at most; the state must not be locked meanwhile.
    pub fn stop_background(&self) {
        self.scopes.stop();
        self.renders.stop();
        self.transport.stop();
        self.analysis.stop();
        self.proxies.stop();
    }

    fn connections(&self) -> std::sync::MutexGuard<'_, Connections> {
        self.connections
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Count a new connection as connected, and give it a number; none if the engine is
    /// ending, and the connection is to be closed at once.
    pub fn arrive(&self, stop: watch::Sender<bool>) -> Option<u64> {
        let mut connections = self.connections();
        if connections.closing {
            return None;
        }
        let id = self.next_connection.fetch_add(1, Ordering::Relaxed);
        connections.stops.insert(id, stop);
        self.presence.send_modify(|presence| {
            presence.connected += 1;
            presence.arrivals += 1;
        });
        Some(id)
    }

    /// Forget a connection that has ended. A gesture that it was sending ends.
    pub async fn leave(&self, id: u64) {
        self.scopes.forget(id);
        // Retire the capture before native cleanup can wait behind another view operation.
        // The remaining sink cannot write through an ended capture while busy is held.
        let kicked =
            {
                let mut state = self.state.lock().await;
                if state.window.owner == Some(id) {
                    state.window_unbind("disconnect");
                }
                state.peers.remove(&id);
                let prefix = format!("{id}:");
                let native_prefix = format!("native:{id}:");
                if state.history.gesture().is_some_and(|(key, _)| {
                    key.starts_with(&prefix) || key.starts_with(&native_prefix)
                }) {
                    state.finish_gesture();
                }
                std::mem::take(&mut state.kicked)
            };
        self.log_kicked(kicked);
        self.connections().stops.remove(&id);
        self.presence
            .send_modify(|presence| presence.connected -= 1);
        self.end_preview_of(id).await;
        self.viewer.forget(id).await;
    }

    /// End the Live Preview that the connection `id` turned on last, as it ends.
    async fn end_preview_of(&self, id: u64) {
        let mut owner = self.preview_owner.lock().await;
        if *owner != Some(id) {
            return;
        }
        *owner = None;
        let off = RawValue::from_string(r#"{"on": false}"#.to_owned()).expect("it is JSON");
        let _ = self
            .transport
            .viewer_preview("viewer.preview", Some(&off))
            .await;
    }

    /// Stop every connection, and take no more.
    pub fn close_all(&self) {
        let mut connections = self.connections();
        connections.closing = true;
        for stop in connections.stops.values() {
            let _ = stop.send(true);
        }
    }

    /// Tell every client that the project could not be saved.
    pub async fn report_save_error(&self, message: &str) {
        let line = rpc::notification_line(
            "engine.error",
            &json!({ "message": message, "kind": "save" }),
        );
        let mut state = self.state.lock().await;
        state.broadcast(&line);
        let kicked = std::mem::take(&mut state.kicked);
        drop(state);
        log("save_failed", &json!({ "message": message }));
        self.log_kicked(kicked);
    }

    fn log_kicked(&self, kicked: Vec<(u64, Kind)>) {
        for (id, kind) in kicked {
            log_kicked(id, Some(kind), self.limits);
        }
    }

    /// Queue the answer to a request, let the state go, and then log what is to be logged. A
    /// connection that was closed by the notification that went before it is not answered.
    fn finish(
        &self,
        mut state: MutexGuard<'_, State>,
        connection: &Connection,
        line: Line,
    ) -> Reply {
        let reply = if state.peers.contains_key(&connection.id) {
            match connection.queue(line) {
                Ok(()) => Reply::Queued,
                Err(refused) => Reply::Refused(refused),
            }
        } else {
            Reply::Refused(Refused::Closed)
        };
        let kicked = std::mem::take(&mut state.kicked);
        drop(state);
        self.log_kicked(kicked);
        reply
    }
}

/// Tell every connection that said hello what a proxy is at. This is called by the thread of the
/// maker of proxies, never by a task of the runtime.
fn tell_proxy(state: &Mutex<State>, note: &Note, limits: Limits) {
    let line = rpc::notification_line("media.proxy", note);
    let mut state = state.blocking_lock();
    state.broadcast(&line);
    let kicked = std::mem::take(&mut state.kicked);
    drop(state);
    for (id, kind) in kicked {
        log_kicked(id, Some(kind), limits);
    }
}

/// Analysis finishes on a blocking worker. A source removed or replaced meanwhile is not
/// announced as ready for a different media snapshot.
fn tell_waveform(state: &Mutex<State>, media: &Media, stream: Option<usize>, limits: Limits) {
    let mut state = state.blocking_lock();
    if !state
        .history
        .project()
        .media
        .iter()
        .any(|found| found.id == media.id && found.identity == media.identity)
    {
        return;
    }
    let params = match stream {
        None => json!({"media":media.id}),
        Some(stream) => json!({"media":media.id, "stream":stream}),
    };
    state.broadcast(&rpc::notification_line("media.waveform", &params));
    let kicked = std::mem::take(&mut state.kicked);
    drop(state);
    for (id, kind) in kicked {
        log_kicked(id, Some(kind), limits);
    }
}

/// Set a proxy that was made on its media, by a change of the engine that is no step to undo,
/// unless the media is gone or is of another file. A proxy that the media has already (made
/// again after its file could not be opened or decoded) is not set again. Either way the
/// transport is given it, to read it again if it had given it up. The file of the proxy that
/// the media had before, if it was another one beside it that no other media has, is deleted.
/// This is called by the thread of the maker of proxies.
fn proxy_made(
    state: &Mutex<State>,
    dir: &Path,
    media: &str,
    proxy: &Proxy,
    limits: Limits,
) -> Made {
    let mut locked = state.blocking_lock();
    let Some(found) = locked
        .history
        .project()
        .media
        .iter()
        .find(|found| found.id == media)
    else {
        return Made::Gone;
    };
    if found.identity.digest != proxy.digest_of_source {
        return Made::Gone;
    }
    let before = found.proxy.clone();
    if before.as_ref() == Some(proxy) {
        if let Some(transport) = &locked.transport {
            transport.proxy_ready(media, PathBuf::from(&proxy.path));
        }
        return Made::Set;
    }
    let other_has = |path: &str| {
        locked.history.project().media.iter().any(|other| {
            other.id != media && other.proxy.as_ref().is_some_and(|proxy| proxy.path == path)
        })
    };
    let before = before.filter(|before| {
        before.path != proxy.path
            && Path::new(&before.path).parent() == Path::new(&proxy.path).parent()
            && !other_has(&before.path)
    });
    let set = Op::SetProxy {
        media: media.to_owned(),
        proxy: Some(proxy.clone()),
    };
    let made = match locked.history.apply_unrecorded(vec![set], "engine") {
        Ok(applied) => {
            locked.publish(&applied);
            if let Some(transport) = &locked.transport {
                transport.proxy_ready(media, PathBuf::from(&proxy.path));
            }
            Made::Set
        }
        Err(error) => {
            log(
                "proxy_failed",
                &json!({ "media": media, "message": error.to_string() }),
            );
            Made::Gone
        }
    };
    let kicked = std::mem::take(&mut locked.kicked);
    drop(locked);
    for (id, kind) in kicked {
        log_kicked(id, Some(kind), limits);
    }
    if let (Some(before), Made::Set) = (before, made) {
        proxy::delete(dir, Path::new(&before.path));
    }
    made
}

/// What writes the passes of the recording of automation (T16, `play::record`): one thread that
/// takes them in the order the timeline monitor gives them and writes each as one transaction
/// ([`write_pass`]). The state is taken there, not on the thread of the monitor, which a request
/// that holds the state may be waiting on.
fn passes_written(state: &Arc<Mutex<State>>, limits: Limits) -> crate::play::record::Hook {
    let (passes, written) = std::sync::mpsc::channel::<crate::play::record::Commit>();
    let state = Arc::clone(state);
    let spawned = std::thread::Builder::new()
        .name("columbina-record".into())
        .spawn(move || {
            while let Ok(commit) = written.recv() {
                write_pass(&state, commit, limits);
            }
        });
    if let Err(error) = spawned {
        log(
            "automation_record_failed",
            &json!({"message": format!("passes cannot be written: {error}")}),
        );
    }
    let passes = std::sync::Mutex::new(passes);
    Arc::new(move |commit| {
        let sent = passes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .send(commit);
        if sent.is_err() {
            log(
                "automation_record_failed",
                &json!({"message": "the thread that writes passes has ended"}),
            );
        }
    })
}

/// Write a pass (T16): of its lanes, those that the project takes as one transaction made by
/// the kind of client that recorded it; a lane whose track, bus, VCA or setting is gone, or that
/// the project refuses, is left out and told of, so that the rest is not lost; one that would
/// change nothing is left out. Everyone is told `automation.written`. What a Punch In could not
/// write of a setting still there is given back to the monitor to be punched in again.
fn write_pass(state: &Mutex<State>, commit: crate::play::record::Commit, limits: Limits) {
    let crate::play::record::Commit { edit, by, punch } = commit;
    let lanes = match edit {
        columbina_model::Edit::RecordPass { timeline, lanes } => (timeline, lanes),
        // Fill Range and Glide Range (T16b): one edit, written whole.
        columbina_model::Edit::LaneFill { .. } => {
            write_whole(state, edit, by, limits);
            return;
        }
        _ => return,
    };
    let (timeline, lanes) = lanes;
    let mut locked = state.blocking_lock();
    let mut trial = locked.history.project().clone();
    let (mut kept, mut failed, mut back) = (Vec::new(), Vec::new(), Vec::new());
    for lane in lanes {
        let one = columbina_model::Edit::RecordPass {
            timeline: timeline.clone(),
            lanes: vec![lane.clone()],
        };
        match columbina_model::expand(&mut trial, one) {
            Ok(_) => kept.push(lane),
            Err(columbina_model::EditError::EmptyRange) => {}
            Err(error) => {
                let reason = serde_json::to_value(&error).unwrap_or_default()["reason"].clone();
                let mut told = serde_json::to_value(&lane.of).unwrap_or_default();
                told["path"] = json!(lane.path);
                told["reason"] = reason;
                failed.push(told);
                if punch
                    && crate::play::record::still_there(locked.history.project(), &timeline, &lane)
                {
                    back.push(lane);
                }
            }
        }
    }
    let written = kept.len();
    let revision = if kept.is_empty() {
        None
    } else {
        let edit = columbina_model::Edit::RecordPass {
            timeline: timeline.clone(),
            lanes: kept,
        };
        match locked.apply_changes_locked(by, None, None, Instant::now(), |_| {
            Ok(vec![Change::Edit(edit)])
        }) {
            Ok(result) => Some(result.revision),
            Err(WorkedOutError::Input(fault) | WorkedOutError::Apply(fault)) => {
                log(
                    "automation_record_failed",
                    &json!({"message": fault.message, "data": fault.data}),
                );
                failed.push(json!({"reason": fault.data.as_ref()
                    .and_then(|data| data.get("reason")).cloned().unwrap_or(json!("refused"))}));
                None
            }
        }
    };
    if !failed.is_empty() {
        log(
            "automation_record_failed",
            &json!({"timeline": timeline, "failed": failed}),
        );
    }
    let told = json!({
        "timeline": timeline,
        "punch": punch,
        "lanes": if revision.is_some() { written } else { 0 },
        "revision": revision,
        "failed": failed,
    });
    locked.broadcast(&rpc::notification_line("automation.written", &told));
    if !back.is_empty() {
        if let Some(transport) = &locked.transport {
            let _ = transport.automation(crate::play::record::Order::GiveBack {
                timeline: timeline.clone(),
                lanes: back,
            });
        }
    }
    let kicked = std::mem::take(&mut locked.kicked);
    drop(locked);
    for (id, kind) in kicked {
        log_kicked(id, Some(kind), limits);
    }
}

/// Write an edit of the recording of automation that is not a pass (Fill Range, Glide Range;
/// T16b) as one transaction, and tell everyone `automation.written`, as a pass is told.
fn write_whole(state: &Mutex<State>, edit: columbina_model::Edit, by: Kind, limits: Limits) {
    let columbina_model::Edit::LaneFill {
        timeline, lanes, ..
    } = &edit
    else {
        return;
    };
    let (timeline, count) = (timeline.clone(), lanes.len());
    let mut locked = state.blocking_lock();
    let mut failed = Vec::new();
    let revision = match locked.apply_changes_locked(by, None, None, Instant::now(), |_| {
        Ok(vec![Change::Edit(edit)])
    }) {
        Ok(result) => Some(result.revision),
        Err(WorkedOutError::Input(fault) | WorkedOutError::Apply(fault)) => {
            let reason = fault
                .data
                .as_ref()
                .and_then(|data| data.get("reason"))
                .cloned()
                .unwrap_or(json!("refused"));
            if reason != json!("empty_range") {
                log(
                    "automation_record_failed",
                    &json!({"message": fault.message, "data": fault.data}),
                );
                failed.push(json!({ "reason": reason }));
            }
            None
        }
    };
    let told = json!({
        "timeline": timeline,
        "punch": false,
        "lanes": if revision.is_some() { count } else { 0 },
        "revision": revision,
        "failed": failed,
    });
    locked.broadcast(&rpc::notification_line("automation.written", &told));
    let kicked = std::mem::take(&mut locked.kicked);
    drop(locked);
    for (id, kind) in kicked {
        log_kicked(id, Some(kind), limits);
    }
}

/// `automation.record`, `automation.touch`, `automation.punch`: the recording of automation of
/// the timeline monitor (T16, `play::record`). A touch is answered at once, once it is checked
/// against the project and recording is on.
async fn automation_call(
    engine: &Engine,
    kind: Kind,
    method: &str,
    params: Option<&RawValue>,
) -> Result<Value, Fault> {
    let (order, answer) = crate::play::record::read(method, params, kind)?;
    if answer.is_none() {
        read_project(engine, |project| {
            crate::play::record::check(project, &order)
        })
        .await?;
        if !engine.transport.recording() {
            return Err(Fault::new(
                rpc::TRANSPORT_FAILED,
                "automation is not being recorded: automation.record turns it on",
            )
            .with_data(json!({"reason": "not_recording"})));
        }
    }
    engine.transport.automation(order)?;
    match answer {
        Some(answer) => answer.await.map_err(|_| {
            Fault::new(rpc::TRANSPORT_FAILED, "the timeline monitor has stopped")
                .with_data(json!({"reason": "stopped"}))
        })?,
        None => Ok(json!({})),
    }
}

/// The params of `automation.copy`.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CopyParams {
    timeline: String,
    of: Vec<columbina_model::LanesOf>,
    #[serde(default)]
    from: Option<columbina_model::Time>,
    #[serde(default)]
    to: Option<columbina_model::Time>,
    #[serde(default)]
    at: Option<columbina_model::Time>,
}

/// `automation.copy` (T16b, p. 3913): of the tracks, buses and VCAs named, the lanes of the
/// categories of recording that are on, over a range (`from`, `to`) or at a time (`at`, a
/// snapshot), for a client to keep and paste with the edit `automation_paste`.
async fn automation_copy(engine: &Engine, params: Option<&RawValue>) -> Result<Value, Fault> {
    let asked: CopyParams = rpc::params(
        "automation.copy",
        r#"{"timeline": "t1", "of": [{"track": "A1"}], "from"?: {"ticks": 0, "scale": 48000}, "to"?: {"ticks": 48000, "scale": 48000}, "at"?: {"ticks": 0, "scale": 48000}}"#,
        params,
    )?;
    let range = match (asked.from, asked.to) {
        (Some(from), Some(to)) => Some((from, to)),
        (None, None) => None,
        _ => {
            return Err(Fault::params("give both from and to, or neither")
                .with_data(json!({"reason": "range"})))
        }
    };
    read_project(engine, |project| {
        let timeline = project
            .timelines
            .iter()
            .find(|t| t.id == asked.timeline)
            .ok_or_else(|| {
                Fault::params(format!("there is no timeline {}", asked.timeline)).with_data(
                    json!({"reason": "missing", "what": "timeline", "id": asked.timeline}),
                )
            })?;
        let lanes = columbina_model::edit::copy_lanes(timeline, &asked.of, range, asked.at)
            .map_err(|error| {
                let data = serde_json::to_value(&error).unwrap_or_default();
                Fault::params(error.to_string()).with_data(data)
            })?;
        Ok(json!({ "lanes": lanes }))
    })
    .await
}

/// Make the proxy of media again, as the transport asks when its file cannot be opened or
/// decoded. This is called by the transport's own thread.
fn make_again(state: &Mutex<State>, media: &str, repair: bool) {
    let state = state.blocking_lock();
    let project = state.history.project();
    if let Some(found) = project.media.iter().find(|found| found.id == media) {
        if repair {
            state.proxies.repair(found);
        } else {
            state.proxies.again(found);
        }
    }
}

/// Tell every connection that said hello the state of the transport. This is called by the
/// transport's own thread, never by a task of the runtime: it waits for the state of the
/// engine, which is held only for moments.
fn tell(state: &Mutex<State>, status: &serde_json::Value, limits: Limits) {
    let line = rpc::notification_line("transport.state", status);
    let mut state = state.blocking_lock();
    state.broadcast(&line);
    let kicked = std::mem::take(&mut state.kicked);
    drop(state);
    for (id, kind) in kicked {
        log_kicked(id, Some(kind), limits);
    }
}

fn tell_render(state: &Mutex<State>, note: &serde_json::Value, limits: Limits) {
    let mut state = state.blocking_lock();
    state.broadcast(&rpc::notification_line("render.progress", note));
    let kicked = std::mem::take(&mut state.kicked);
    drop(state);
    for (id, kind) in kicked {
        log_kicked(id, Some(kind), limits);
    }
}

impl State {
    /// Queue a line on every connection that said hello. A connection whose queue is full is
    /// closed, so that it never misses a single message and then goes on.
    fn broadcast(&mut self, line: &Line) {
        let mut gone = Vec::new();
        for (id, peer) in &self.peers {
            if let Err(refused) = peer.queue.push(line.clone()) {
                gone.push((*id, refused));
            }
        }
        for (id, refused) in gone {
            if let Some(peer) = self.peers.remove(&id) {
                let _ = peer.stop.send(true);
                if refused == Refused::Full {
                    self.kicked.push((id, peer.kind));
                }
            }
        }
    }

    /// Save the project as it is now, and tell everyone what changed, after what a gesture
    /// changed that was held. Gives what was done, as it is written.
    fn publish(&mut self, applied: &Applied) -> Box<RawValue> {
        self.flush();
        self.save_and_draw();
        let done = serde_json::value::to_raw_value(&applied.done)
            .expect("an operation is written as JSON");
        self.tell(
            None,
            applied.revision,
            &applied.by,
            &applied.made_by,
            &done,
            applied.next_id,
        );
        self.took_media(&applied.done);
        done
    }

    /// Save the project as it is now and draw it, as a transaction of a gesture changed it, and
    /// tell everyone what changed if [`TELL_EVERY`] has gone by since a change was last told,
    /// or the gesture ended (`ends`); else hold it, to be told with what follows of the gesture,
    /// which is the one it `begins` or the one whose changes are held. Gives what was
    /// done, as it is written, and when what is held is to be told, where a task is to wait for
    /// it.
    fn publish_in_gesture(
        &mut self,
        applied: &Applied,
        begins: bool,
        ends: bool,
        now: Instant,
    ) -> (Box<RawValue>, Option<Instant>) {
        // What is held is of one gesture: that of another is told before this one begins.
        if begins {
            self.flush();
        }
        self.save_and_draw();
        let done = serde_json::value::to_raw_value(&applied.done)
            .expect("an operation is written as JSON");
        let held = self.held.get_or_insert_with(|| Held {
            base: applied.revision - 1,
            revision: applied.revision,
            done: Vec::new(),
            by: applied.by.clone(),
            made_by: applied.made_by.clone(),
            next_id: applied.next_id,
        });
        held.revision = applied.revision;
        held.next_id = applied.next_id;
        held.done.extend(applied.done.iter().cloned());
        let due = self.told_at.map(|told| told + TELL_EVERY);
        if ends || due.is_none_or(|due| due <= now) {
            self.flush();
            return (done, None);
        }
        if self.tell_at.is_some() {
            return (done, None);
        }
        self.tell_at = due;
        (done, due)
    }

    /// Tell everyone what a gesture changed that was held, if anything was.
    fn flush(&mut self) {
        self.tell_at = None;
        let Some(held) = self.held.take() else {
            return;
        };
        let done =
            serde_json::value::to_raw_value(&held.done).expect("an operation is written as JSON");
        let base = (held.base + 1 != held.revision).then_some(held.base);
        self.tell(
            base,
            held.revision,
            &held.by,
            &held.made_by,
            &done,
            held.next_id,
        );
        self.took_media(&held.done);
    }

    /// Save the project as it is now, and have the transport draw it.
    fn save_and_draw(&mut self) {
        self.window_project_changed();
        if let Some(saver) = &self.saver {
            saver.changed(self.history.project().clone());
        }
        if let Some(transport) = &self.transport {
            transport.project_changed(self.history.project(), self.history.revision());
        }
    }

    /// Tell everyone of a change, and the maker of proxies.
    fn tell(
        &mut self,
        base: Option<u64>,
        revision: u64,
        by: &str,
        made_by: &str,
        done: &RawValue,
        next_id: u64,
    ) {
        let changed = Changed {
            revision,
            base,
            by,
            made_by,
            done,
            next_id,
        };
        self.broadcast(&rpc::notification_line("project.changed", &changed));
        self.told_at = Some(Instant::now());
        for note in self.proxies.project_changed(self.history.project()) {
            self.broadcast(&rpc::notification_line("media.proxy", &note));
        }
    }

    /// Have the maker of proxies make those of media that `done` added.
    fn took_media(&self, done: &[Op]) {
        let added = done
            .iter()
            .filter_map(|op| match op {
                Op::AddMedia { media, .. } => Some(media.id.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        if !added.is_empty() {
            self.proxies.imported(self.history.project(), &added);
        }
    }

    /// End the gesture that is going on where it was sent nothing for [`GESTURE_IDLE`].
    fn end_idle_gesture(&mut self, now: Instant) {
        if self.window_keeps_gesture() {
            return;
        }
        if self.history.gesture().is_some()
            && self
                .gesture_last
                .is_none_or(|last| last + GESTURE_IDLE <= now)
        {
            self.finish_gesture();
        }
    }

    /// The shared tail of disconnect, idle, and an explicitly finished native gesture.
    fn finish_gesture(&mut self) {
        self.history.end_gesture();
        self.gesture_last = None;
        self.flush();
    }

    /// Reserve an owned token without writing the project or ending any current gesture.
    fn begin_owned_gesture(&mut self, owner: u64, base: u64) -> Result<OwnedGesture, Fault> {
        if base != self.history.revision() {
            return Err(stale_revision(base, self.history.revision()));
        }
        self.next_owned_gesture += 1;
        Ok(OwnedGesture {
            owner,
            key: format!("native:{owner}:{}", self.next_owned_gesture),
            base,
            last_own: None,
        })
    }

    /// Finish without inventing a final operation. An unwritten token changes no history.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "finishing consumes the token so an unwritten token cannot subsequently write"
    )]
    fn finish_owned_gesture(&mut self, owner: u64, gesture: OwnedGesture) -> Result<(), Fault> {
        self.check_owned_gesture(owner, &gesture)?;
        if gesture.last_own.is_some() {
            self.finish_gesture();
        }
        Ok(())
    }

    fn check_owned_gesture(&self, owner: u64, gesture: &OwnedGesture) -> Result<(), Fault> {
        if owner != gesture.owner {
            return Err(
                Fault::new(rpc::REFUSED, "the gesture belongs to another connection")
                    .with_data(json!({"reason":"gesture_owner"})),
            );
        }
        let expected = gesture.last_own.unwrap_or(gesture.base);
        let actual = self.history.revision();
        if actual != expected {
            return Err(stale_revision(expected, actual));
        }
        if gesture.last_own.is_some()
            && self.history.gesture() != Some((gesture.key.as_str(), gesture.base))
        {
            return Err(Fault::new(
                rpc::REFUSED,
                "the gesture has ended or another gesture replaced it",
            )
            .with_data(json!({"reason":"stale_gesture"})));
        }
        Ok(())
    }

    /// Commit prepared changes, finish their names, and publish through the same path for
    /// both wire and worked-out transactions. The caller holds State throughout.
    fn apply_prepared(
        &mut self,
        requested: Vec<Change>,
        mut names: names::Names,
        kind: Kind,
        gesture: Option<(&str, bool)>,
        now: Instant,
    ) -> Result<PreparedResult, Fault> {
        let changed = match gesture {
            Some((key, end)) => {
                self.history
                    .change_in_gesture(requested, kind.as_str(), &mut names, key, end)
            }
            None => self
                .history
                .change_with(requested, kind.as_str(), &mut names),
        }
        .map_err(|error| {
            // Wire end=true also ends a matching gesture when applying it fails.
            if gesture.is_some() && self.history.gesture().is_none() {
                self.gesture_last = None;
                self.flush();
            }
            Fault::refused(&error)
        })?;
        let applied = &changed.applied;
        let names = names.finish();
        let (done, due) = if gesture.is_some() {
            let going_on = self.history.gesture().is_some();
            self.gesture_last = going_on.then_some(now);
            self.publish_in_gesture(applied, !changed.merged, !going_on, now)
        } else {
            (self.publish(applied), None)
        };
        Ok(PreparedResult {
            revision: applied.revision,
            done,
            next_id: applied.next_id,
            names,
            clamped: changed.clamped,
            due,
        })
    }

    /// Calculate against the locked project, then parse, prepare, and commit under that lock.
    fn apply_worked_out_locked(
        &mut self,
        kind: Kind,
        revision: Option<u64>,
        owned: Option<(u64, &mut OwnedGesture)>,
        now: Instant,
        work: impl FnOnce(&Project) -> Result<Vec<Value>, Fault>,
    ) -> Result<PreparedResult, WorkedOutError> {
        self.apply_changes_locked(kind, revision, owned, now, work)
    }

    /// [`State::apply_worked_out_locked`] of operations and edits written as JSON, or of changes
    /// that the engine made (see [`WorkedOut`]).
    fn apply_changes_locked<W: WorkedOut>(
        &mut self,
        kind: Kind,
        revision: Option<u64>,
        owned: Option<(u64, &mut OwnedGesture)>,
        now: Instant,
        work: impl FnOnce(&Project) -> Result<Vec<W>, Fault>,
    ) -> Result<PreparedResult, WorkedOutError> {
        if let Some((owner, gesture)) = &owned {
            self.check_owned_gesture(*owner, gesture)
                .map_err(WorkedOutError::Apply)?;
        }
        self.end_idle_gesture(now);
        if let Some((owner, gesture)) = &owned {
            // Idle may have ended an otherwise current token. Native input must not reopen it.
            self.check_owned_gesture(*owner, gesture)
                .map_err(WorkedOutError::Apply)?;
        }
        let key = owned.as_ref().map(|(_, gesture)| gesture.key.as_str());
        if let Some(revision) =
            revision.filter(|revision| !self.history.takes_revision(*revision, key))
        {
            return Err(WorkedOutError::Apply(stale_revision(
                revision,
                self.history.revision(),
            )));
        }
        let mut ops = Vec::new();
        for (index, op) in work(self.history.project())
            .map_err(WorkedOutError::Input)?
            .into_iter()
            .enumerate()
        {
            ops.push(op.input(index).map_err(WorkedOutError::Input)?);
        }
        let names = names::prepare(&mut ops).map_err(|error| {
            WorkedOutError::Input(
                Fault::params(error.to_string()).with_data(json!({"reason":error.reason})),
            )
        })?;
        // Native endings are explicit and only follow a successful commit. Never use the
        // wire end=true semantics, which end even when applying the changes fails.
        let result = self
            .apply_prepared(
                ops.into_iter().map(|input| input.change).collect(),
                names,
                kind,
                key.map(|key| (key, false)),
                now,
            )
            .map_err(WorkedOutError::Apply)?;
        if let Some((_, gesture)) = owned {
            gesture.last_own = Some(result.revision);
        }
        Ok(result)
    }
}

/// Tell what a gesture changed that is held once it is due, as [`State::publish_in_gesture`]
/// asks: unless it was told before, with something else.
fn tell_when_due(engine: &Engine, due: Instant) {
    let state = Arc::clone(&engine.state);
    let limits = engine.limits;
    tokio::spawn(async move {
        let mut due = due;
        loop {
            tokio::time::sleep_until(due.into()).await;
            let mut locked = state.lock().await;
            match locked.tell_at {
                Some(at) if at > Instant::now() => due = at,
                Some(_) => {
                    locked.flush();
                    let kicked = std::mem::take(&mut locked.kicked);
                    drop(locked);
                    for (id, kind) in kicked {
                        log_kicked(id, Some(kind), limits);
                    }
                    return;
                }
                None => return,
            }
        }
    });
}

/// The params of `project.changed`.
#[derive(Serialize)]
struct Changed<'a> {
    revision: u64,
    /// The revision that `done` applies to, where it is not the one before `revision`: what a
    /// gesture changed, held and told at once.
    #[serde(skip_serializing_if = "Option::is_none")]
    base: Option<u64>,
    by: &'a str,
    made_by: &'a str,
    done: &'a RawValue,
    next_id: u64,
}

/// How a request was answered.
pub enum Reply {
    /// With an error, which is queued once the request is done with.
    Error(Fault),
    /// Its answer was queued already, in its order among the notifications.
    Queued,
    /// Its answer could not be queued, and the connection is to be closed.
    Refused(Refused),
}

impl From<Fault> for Reply {
    fn from(fault: Fault) -> Self {
        Reply::Error(fault)
    }
}

/// Answer one line that a client sent. Gives why the connection is to be closed, if it is.
pub async fn handle(
    engine: &Engine,
    connection: &mut Connection,
    line: &[u8],
) -> Result<(), Refused> {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    if line.iter().all(u8::is_ascii_whitespace) {
        return Ok(());
    }
    let request = match rpc::parse(line) {
        Ok(request) => request,
        Err((id, fault)) => return connection.queue(rpc::error_line(id, &fault)),
    };
    // The engine takes no notifications from clients.
    let Some(id) = request.id else {
        return Ok(());
    };
    let reply = match dispatch(engine, connection, id, &request.method, request.params).await {
        Ok(reply) | Err(reply) => reply,
    };
    match reply {
        Reply::Error(fault) => connection.queue(rpc::error_line(Some(id), &fault)),
        Reply::Queued => Ok(()),
        Reply::Refused(refused) => Err(refused),
    }
}

/// Call a method. The methods of later tasks are added here.
#[allow(
    clippy::too_many_lines,
    reason = "one explicit branch for each RPC route"
)]
async fn dispatch(
    engine: &Engine,
    connection: &mut Connection,
    id: &RawValue,
    method: &str,
    params: Option<&RawValue>,
) -> Result<Reply, Reply> {
    if method == "hello" {
        return hello(engine, connection, id, params).await;
    }
    let Some(kind) = connection.kind else {
        let fault = Fault::new(rpc::INVALID_REQUEST, format!("say hello before {method}"));
        return Err(fault.into());
    };
    match method {
        "project.get" => project_get(engine, connection, id, params).await,
        "timeline.duration" => timeline_duration(engine, connection, id, params).await,
        "transaction.apply" => transaction_apply(engine, connection, kind, id, params).await,
        "history.undo" | "history.redo" => {
            history_step(engine, connection, kind, id, method, params).await
        }
        "media.import" => media_import(engine, connection, kind, id, params).await,
        "media.inspect" => media_inspect(engine, connection, id, params).await,
        "render.start" => render_start(engine, connection, id, method, params).await,
        "render.cancel" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Cancel {
                job: String,
            }
            let request: Cancel = rpc::params(method, r#"{"job":"render-1"}"#, params)?;
            engine.renders.cancel(&request.job)?;
            Ok(match connection.queue(rpc::result_line(id, &json!({}))) {
                Ok(()) => Reply::Queued,
                Err(refused) => Reply::Refused(refused),
            })
        }
        "media.make_proxy" => make_proxy(engine, connection, id, params).await,
        "media.thumbnail" | "media.waveform" => {
            media_analysis(engine, connection, id, method, params).await
        }
        "engine.yield" => engine_yield(engine, connection, kind, id, params).await,
        "scopes.statistics" if measured_apart(params) => {
            statistics_apart(engine, connection, id, params).await
        }
        "scopes.subscribe" | "scopes.set" | "scopes.unsubscribe" | "scopes.statistics" => {
            scopes_call(engine, connection, id, method, params).await
        }
        "view.sample" => view_sample(engine, connection, id, params).await,
        "timeline.thumbnail" => timeline_thumbnail(engine, connection, id, params).await,
        "view.pointer" => view_pointer(engine, connection, kind, id, params).await,
        "window.bind" | "window.input" | "window.unbind" => {
            window::call(engine, connection, kind, id, method, params).await
        }
        "track.start" | "track.stop" | "track.edit" => {
            tracker::call(engine, connection, kind, id, method, params).await
        }
        "luts.list" | "grade.export_lut" | "luts.preview" => {
            luts_call(engine, connection, id, method, params).await
        }
        _ if crate::window_presets::is_method(method) => {
            crate::window_presets::call(engine, connection, kind, id, method, params).await
        }
        "viewer.wipe" | "viewer.split" | "viewer.highlight" | "viewer.bypass"
        | "viewer.preview" | "viewer.lut" => {
            let result = if method == "viewer.preview" {
                let mut owner = engine.preview_owner.lock().await;
                let result = viewer(engine, method, params).await?;
                *owner = (!result["preview"].is_null()).then_some(connection.id);
                result
            } else {
                viewer(engine, method, params).await?
            };
            if method == "viewer.highlight" || result["mode"] == "split" {
                engine.state.lock().await.window_end("viewer_mode");
            }
            Ok(match connection.queue(rpc::result_line(id, &result)) {
                Ok(()) => Reply::Queued,
                Err(refused) => Reply::Refused(refused),
            })
        }
        "view.attach" | "view.place" | "view.show" | "view.detach" | "view.display"
        | "view.viewport" => {
            let result = engine.viewer.call(kind, method, params).await?;
            let written: Value = params
                .and_then(|p| serde_json::from_str(p.get()).ok())
                .unwrap_or(Value::Null);
            if written["monitor"] == "timeline"
                && (matches!(method, "view.attach" | "view.place" | "view.detach")
                    || method == "view.viewport"
                        && (written.get("mode").is_some()
                            || matches!(
                                written["operation"].as_str(),
                                Some("zoom" | "pan" | "scale" | "fit")
                            ))
                    || method == "view.show" && written["shown"] == false)
            {
                engine.viewer.frames.stop_input(Monitor::Timeline);
                engine.state.lock().await.window_end("view_changed");
            }
            if method == "view.attach" {
                let (monitor, _) = crate::play::monitor_params(method, params)?;
                engine.transport.redraw(monitor);
            }
            Ok(match connection.queue(rpc::result_line(id, &result)) {
                Ok(()) => Reply::Queued,
                Err(refused) => Reply::Refused(refused),
            })
        }
        "automation.record"
        | "automation.touch"
        | "automation.punch"
        | "automation.all_read"
        | "automation.join"
        | "automation.fill" => {
            let result = automation_call(engine, kind, method, params).await?;
            Ok(match connection.queue(rpc::result_line(id, &result)) {
                Ok(()) => Reply::Queued,
                Err(refused) => Reply::Refused(refused),
            })
        }
        "automation.copy" => {
            let result = automation_copy(engine, params).await?;
            Ok(match connection.queue(rpc::result_line(id, &result)) {
                Ok(()) => Reply::Queued,
                Err(refused) => Reply::Refused(refused),
            })
        }
        "transport.load" => {
            let result = transport_load(engine, params).await?;
            Ok(match connection.queue(rpc::result_line(id, &result)) {
                Ok(()) => Reply::Queued,
                Err(refused) => Reply::Refused(refused),
            })
        }
        "transport.frame_digest" => {
            let result = engine.transport.frame_digest(params).await?;
            Ok(match connection.queue(rpc::result_line(id, &result)) {
                Ok(()) => Reply::Queued,
                Err(refused) => Reply::Refused(refused),
            })
        }
        _ if method.starts_with("transport.") => {
            // The answer is queued before the command is sent, so that the state that it makes
            // is told after the answer.
            let (result, order) = engine.transport.prepare(method, params)?;
            if method != "transport.state" {
                engine.viewer.frames.stop_input(Monitor::Timeline);
                engine.state.lock().await.window_end("transport");
            }
            let reply = match connection.queue(rpc::result_line(id, &result)) {
                Ok(()) => Reply::Queued,
                Err(refused) => Reply::Refused(refused),
            };
            order.send().await;
            Ok(reply)
        }
        // `still.*`, `gallery.*`, `grade.append_node`, or no method of that name.
        _ => {
            crate::stills::call(
                engine,
                &engine.transport,
                connection,
                kind,
                id,
                method,
                params,
            )
            .await
        }
    }
}

/// `scopes.*`: the scopes of the monitors, and the statistics of what a monitor shows.
async fn scopes_call(
    engine: &Engine,
    connection: &Connection,
    id: &RawValue,
    method: &str,
    params: Option<&RawValue>,
) -> Result<Reply, Reply> {
    let redraw = |view: Monitor| engine.transport.redraw(view);
    let (number, sender) = (connection.id, connection.sender());
    let answered = match method {
        "scopes.subscribe" => engine
            .scopes
            .subscribe(number, sender, id, params, &redraw)?,
        "scopes.set" => engine.scopes.set(number, sender, id, params, &redraw)?,
        "scopes.unsubscribe" => engine.scopes.unsubscribe(number, sender, id, params)?,
        _ => {
            engine.scopes.check(method)?;
            let asked = StatisticsRequest::read(params)?;
            let shown = engine.transport.shown(asked.view).await?;
            let scopes = engine.scopes.clone();
            let result = tokio::task::spawn_blocking(move || {
                scopes.statistics(asked.view, &shown, asked.bins)
            })
            .await
            .map_err(|error| Fault::new(rpc::TRANSPORT_FAILED, error.to_string()))??;
            connection.queue(rpc::result_line(id, &result))
        }
    };
    Ok(match answered {
        Ok(()) => Reply::Queued,
        Err(refused) => Reply::Refused(refused),
    })
}

/// `view.pointer`: have the view of a monitor take the pointer for this connection, or let it go.
async fn view_pointer(
    engine: &Engine,
    connection: &Connection,
    kind: Kind,
    id: &RawValue,
    params: Option<&RawValue>,
) -> Result<Reply, Reply> {
    let change = engine
        .viewer
        .prepare_pointer(kind, connection.id, connection.sender(), params)
        .await?;
    change.apply().await?;
    let mut state = engine.state.lock().await;
    let result = change.commit();
    if result["monitor"] == "timeline" {
        state.window_pointer_changed();
    }
    Ok(match connection.queue(rpc::result_line(id, &result)) {
        Ok(()) => Reply::Queued,
        Err(refused) => Reply::Refused(refused),
    })
}

/// Whether `scopes.statistics` asks for a frame drawn apart from what a monitor shows: of a
/// timeline at a frame, or at a node of a grade.
fn measured_apart(params: Option<&RawValue>) -> bool {
    params
        .and_then(|params| {
            serde_json::from_str::<serde_json::Map<String, Value>>(params.get()).ok()
        })
        .is_some_and(|params| {
            ["timeline", "frame", "grade", "node", "at", "clip"]
                .iter()
                .any(|name| params.contains_key(*name))
        })
}

/// The params of `scopes.statistics` measured apart from what a monitor shows.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StatisticsApart {
    #[serde(default)]
    view: Option<Monitor>,
    #[serde(default)]
    timeline: Option<String>,
    #[serde(default)]
    frame: Option<i64>,
    #[serde(default = "bins")]
    bins: u32,
    #[serde(default)]
    grade: Option<GradeTarget>,
    #[serde(default)]
    node: Option<String>,
    #[serde(default)]
    at: Option<Place>,
    #[serde(default)]
    clip: Option<String>,
}

fn bins() -> u32 {
    256
}

/// `viewer.wipe` and `viewer.split` (what the timeline monitor compares with),
/// `viewer.highlight` (what it shows of the key of a node), and `viewer.bypass` (its picture
/// without its grades).
async fn viewer(engine: &Engine, method: &str, params: Option<&RawValue>) -> Result<Value, Fault> {
    if method == "viewer.highlight" {
        return engine.transport.highlight(params).await;
    }
    if method == "viewer.bypass" {
        return engine.transport.bypass(params).await;
    }
    if method == "viewer.preview" || method == "viewer.lut" {
        return engine.transport.viewer_preview(method, params).await;
    }
    crate::stills::compare(engine, &engine.transport, method, params).await
}

/// The params of `view.sample`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Sample {
    #[serde(default)]
    monitor: Option<Monitor>,
    #[serde(default)]
    timeline: Option<String>,
    #[serde(default)]
    frame: Option<i64>,
    x: f64,
    y: f64,
    #[serde(default = "one")]
    size: u32,
    #[serde(default)]
    grade: Option<GradeTarget>,
    #[serde(default)]
    node: Option<String>,
    #[serde(default)]
    clip: Option<String>,
}

fn one() -> u32 {
    1
}

/// The node that params name, at `place`: none where they name none.
fn node_at(
    grade: Option<GradeTarget>,
    node: Option<String>,
    place: Place,
    clip: Option<String>,
) -> Result<Option<NodeAt>, Fault> {
    match (grade, node) {
        (Some(grade), Some(node)) => Ok(Some(NodeAt {
            grade,
            node,
            place,
            clip,
        })),
        (None, None) if clip.is_none() => Ok(None),
        _ => Err(Fault::params(
            "a node is named by both its grade and its node, and a clip only with them",
        )
        .with_data(json!({ "reason": "node" }))),
    }
}

/// Where a frame to measure is: of the timeline that the timeline monitor shows, at the frame
/// it shows (a monitor's), or of a timeline at a frame (`timeline` and `frame`). Gives the
/// timeline, the frame and the monitor.
async fn frame_named(
    engine: &Engine,
    monitor: Option<Monitor>,
    timeline: Option<String>,
    frame: Option<i64>,
    names: (&str, &str),
) -> Result<(Option<Shown>, Option<(String, i64)>), Fault> {
    match (monitor, timeline, frame) {
        (Some(monitor), None, None) => {
            let shown = engine.transport.shown(monitor).await?;
            let at = match (&shown, engine.transport.timeline_loaded()) {
                (Shown::Target { frame, .. }, Some(timeline)) => Some((timeline, *frame)),
                _ => None,
            };
            Ok((Some(shown), at))
        }
        (None, Some(timeline), Some(frame)) => Ok((None, Some((timeline, frame)))),
        _ => Err(Fault::params(format!(
            "a frame is named by {} alone, or by timeline and frame",
            names.0
        ))
        .with_data(json!({ "reason": names.1 }))),
    }
}

/// The refusal of a node of a grade on the source monitor, which shows media ungraded.
fn no_grades_there() -> Fault {
    Fault::params("the source monitor shows media, which is graded on a timeline only")
        .with_data(json!({ "reason": "node" }))
}

/// `scopes.statistics` of a frame drawn apart from what a monitor shows: of a timeline at a frame
/// (in a headless engine too), or at a node of a grade, of the frame that the timeline monitor
/// shows or of a timeline at a frame.
async fn statistics_apart(
    engine: &Engine,
    connection: &Connection,
    id: &RawValue,
    params: Option<&RawValue>,
) -> Result<Reply, Reply> {
    let asked: StatisticsApart = rpc::params(
        "scopes.statistics",
        r#"{"view": "timeline" | "source"} or {"timeline": "t1", "frame": 0}, "bins"?: 256, "grade"?: {"level": "clip", ...}, "node"?: "n1", "at"?: "input" | "output", "clip"?: "c1""#,
        params,
    )?;
    if !(16..=1024).contains(&asked.bins) {
        return Err(Fault::params(format!(
            "the bins of the histogram are {}, not from 16 to 1024",
            asked.bins
        ))
        .with_data(json!({"reason": "bins"}))
        .into());
    }
    let node = node_at(
        asked.grade,
        asked.node,
        asked.at.unwrap_or_default(),
        asked.clip,
    )?;
    if node.is_none() && asked.at.is_some() {
        return Err(Fault::params("at is given only with a node")
            .with_data(json!({ "reason": "node" }))
            .into());
    }
    if asked.view == Some(Monitor::Source) {
        return Err(no_grades_there().into());
    }
    let monitor = asked.view;
    let (_, at) = frame_named(
        engine,
        monitor,
        asked.timeline,
        asked.frame,
        ("view", "view"),
    )
    .await?;
    let (timeline, frame) = at.ok_or_else(|| {
        Fault::new(
            rpc::TRANSPORT_FAILED,
            "the timeline monitor shows no timeline",
        )
        .with_data(json!({ "reason": "not_loaded" }))
    })?;
    let project = engine.state.lock().await.history.project().clone();
    let (probe, scopes, bins) = (Arc::clone(&engine.probe), engine.scopes.clone(), asked.bins);
    let result = tokio::task::spawn_blocking(move || {
        let drawn = probe.draw(&project, &timeline, frame, node.as_ref())?;
        let signal = drawn.signal;
        let shown = Shown::Target {
            gpu: drawn.gpu,
            target: drawn.target,
            frame,
        };
        let mut value = scopes.statistics_by(Monitor::Timeline, &shown, bins, Some(signal))?;
        if let Some(object) = value.as_object_mut() {
            object.remove("view");
            if let Some(monitor) = monitor {
                object.insert("view".into(), json!(monitor));
            }
            object.insert("timeline".into(), json!(timeline));
            if let Some(node) = &node {
                object.insert("node".into(), node.written());
            }
        }
        Ok::<_, Fault>(value)
    })
    .await
    .map_err(|error| probe::drawing(&error.to_string()))??;
    Ok(match connection.queue(rpc::result_line(id, &result)) {
        Ok(()) => Reply::Queued,
        Err(refused) => Reply::Refused(refused),
    })
}

/// `timeline.thumbnail`: a small picture of a frame of a timeline as it is graded, drawn off
/// screen (`crate::thumbnail`).
async fn timeline_thumbnail(
    engine: &Engine,
    connection: &Connection,
    id: &RawValue,
    params: Option<&RawValue>,
) -> Result<Reply, Reply> {
    let request = crate::thumbnail::Request::read(params)?;
    let project = engine.state.lock().await.history.project().clone();
    let probe = Arc::clone(&engine.probe);
    let result = tokio::task::spawn_blocking(move || {
        crate::thumbnail::thumbnail(&probe, &project, &request)
    })
    .await
    .map_err(|error| Fault::new(rpc::CANNOT_READ, error.to_string()))??;
    Ok(match connection.queue(rpc::result_line(id, &result)) {
        Ok(()) => Reply::Queued,
        Err(refused) => Reply::Refused(refused),
    })
}

/// `view.sample`: the mean of a few pixels of what a monitor shows, or of a frame of a timeline
/// drawn off screen, about a point of the picture; and, for a node of a grade, of the picture
/// that goes into it and comes out of it there.
async fn view_sample(
    engine: &Engine,
    connection: &Connection,
    id: &RawValue,
    params: Option<&RawValue>,
) -> Result<Reply, Reply> {
    let asked: Sample = rpc::params(
        "view.sample",
        r#"{"monitor": "timeline" | "source"} or {"timeline": "t1", "frame": 0}, "x": 960.5, "y": 540.5, "size"?: 1, "grade"?: {"level": "clip", ...}, "node"?: "n1", "clip"?: "c1""#,
        params,
    )?;
    let node = node_at(asked.grade, asked.node, Place::Output, asked.clip)?;
    if node.is_some() && asked.monitor == Some(Monitor::Source) {
        return Err(no_grades_there().into());
    }
    let monitor = asked.monitor;
    let (shown, at) = frame_named(
        engine,
        monitor,
        asked.timeline,
        asked.frame,
        ("monitor", "monitor"),
    )
    .await?;
    if node.is_some() && at.is_none() {
        return Err(Fault::new(
            rpc::TRANSPORT_FAILED,
            "the timeline monitor shows no timeline",
        )
        .with_data(json!({ "reason": "not_loaded" }))
        .into());
    }
    let shown_frame = shown.as_ref().map(|shown| match shown {
        Shown::Target { frame, .. } => *frame,
        Shown::Picture { picture, .. } => picture.frame,
    });
    let project = engine.state.lock().await.history.project().clone();
    let probe = Arc::clone(&engine.probe);
    let (x, y, size) = (asked.x, asked.y, asked.size);
    let result = tokio::task::spawn_blocking(move || {
        let mean = |drawn: &probe::Drawn| {
            let (width, height) = drawn.target.size();
            let region = probe::square(x, y, size, width, height)?;
            probe::mean_of(drawn, region).map(|rgb| (rgb, region, (width, height)))
        };
        let drawn = match shown {
            Some(Shown::Target { gpu, target, .. }) => probe::Drawn {
                gpu,
                target,
                signal: columbina_compose::scopes::Signal::default(),
            },
            Some(Shown::Picture { picture, rotation }) => probe.draw_picture(picture, rotation)?,
            None => {
                let (timeline, frame) = at.as_ref().expect("named with the frame");
                probe.draw(&project, timeline, *frame, None)?
            }
        };
        let (rgb, region, (width, height)) = mean(&drawn)?;
        let mut value = json!({
            "frame": at.as_ref().map(|(_, frame)| *frame).or(shown_frame),
            "x": x,
            "y": y,
            "size": size,
            "width": width,
            "height": height,
            "region": { "x": region.x, "y": region.y, "width": region.width, "height": region.height },
            "rgb": rgb,
        });
        match monitor {
            Some(monitor) => value["monitor"] = json!(monitor),
            None => value["timeline"] = json!(at.as_ref().map(|(timeline, _)| timeline)),
        }
        if let (Some(node), Some((timeline, frame))) = (node, &at) {
            let mut values = Vec::new();
            for place in [Place::Input, Place::Output, Place::Key] {
                let at_node = NodeAt {
                    place,
                    ..node.clone()
                };
                let drawn = probe.draw(&project, timeline, *frame, Some(&at_node))?;
                values.push(mean(&drawn)?.0);
            }
            let mut written = node.written();
            written.as_object_mut().map(|object| object.remove("at"));
            written["input"] = json!(values[0]);
            written["output"] = json!(values[1]);
            written["key"] = json!(values[2][0]);
            value["node"] = written;
        }
        Ok::<_, Fault>(value)
    })
    .await
    .map_err(|error| probe::drawing(&error.to_string()))??;
    Ok(match connection.queue(rpc::result_line(id, &result)) {
        Ok(()) => Reply::Queued,
        Err(refused) => Reply::Refused(refused),
    })
}

/// `luts.list`, or `grade.export_lut`.
async fn luts_call(
    engine: &Engine,
    connection: &Connection,
    id: &RawValue,
    method: &str,
    params: Option<&RawValue>,
) -> Result<Reply, Reply> {
    if method == "grade.export_lut" {
        export_lut(engine, connection, id, params).await
    } else if method == "luts.preview" {
        luts_preview(engine, connection, id, params).await
    } else {
        luts_list(engine, connection, id, params).await
    }
}

/// `grade.export_lut`: a grade baked into a cube and written (`crate::lut_export`).
async fn export_lut(
    engine: &Engine,
    connection: &Connection,
    id: &RawValue,
    params: Option<&RawValue>,
) -> Result<Reply, Reply> {
    let request = crate::lut_export::Request::read(params)?;
    let project = engine.state.lock().await.history.project().clone();
    let places = engine.luts.clone();
    let dir = PathBuf::from(&engine.project);
    let result = tokio::task::spawn_blocking(move || {
        crate::lut_export::export(&project, &dir, &places, &request)
    })
    .await
    .map_err(|error| Fault::new(rpc::CANNOT_READ, error.to_string()))??;
    Ok(match connection.queue(rpc::result_line(id, &result)) {
        Ok(()) => Reply::Queued,
        Err(refused) => Reply::Refused(refused),
    })
}

/// `luts.preview`: what LUTs of the library do, on the test image or on a frame of a timeline
/// (`crate::lut_preview`).
async fn luts_preview(
    engine: &Engine,
    connection: &Connection,
    id: &RawValue,
    params: Option<&RawValue>,
) -> Result<Reply, Reply> {
    let request = crate::lut_preview::Request::read(params)?;
    let project = engine.state.lock().await.history.project().clone();
    let probe = Arc::clone(&engine.probe);
    let places = engine.luts.clone();
    let result = tokio::task::spawn_blocking(move || {
        crate::lut_preview::preview(&probe, &places, &project, &request)
    })
    .await
    .map_err(|error| Fault::new(rpc::CANNOT_READ, error.to_string()))??;
    Ok(match connection.queue(rpc::result_line(id, &result)) {
        Ok(()) => Reply::Queued,
        Err(refused) => Reply::Refused(refused),
    })
}

/// `luts.list`: the folders that LUTs are looked for in, and the LUTs in them.
async fn luts_list(
    engine: &Engine,
    connection: &Connection,
    id: &RawValue,
    params: Option<&RawValue>,
) -> Result<Reply, Reply> {
    let asked = luts::Request::read(params)?;
    let places = engine.luts.clone();
    let result = tokio::task::spawn_blocking(move || luts::list(&places, &asked))
        .await
        .map_err(|error| Fault::new(rpc::CANNOT_READ, error.to_string()))?;
    Ok(match connection.queue(rpc::result_line(id, &result)) {
        Ok(()) => Reply::Queued,
        Err(refused) => Reply::Refused(refused),
    })
}

async fn media_analysis(
    engine: &Engine,
    connection: &Connection,
    id: &RawValue,
    method: &str,
    params: Option<&RawValue>,
) -> Result<Reply, Reply> {
    let (media_id, thumbnail, stream) = if method == "media.thumbnail" {
        let request: ThumbnailRequest =
            rpc::params(method, r#"{"media":"m1","height":90}"#, params)?;
        (request.media.clone(), Some(request), None)
    } else {
        let request: WaveformRequest = rpc::params(method, r#"{"media":"m1"}"#, params)?;
        (request.media, None, request.stream)
    };
    let media = {
        let state = engine.state.lock().await;
        state
            .history
            .project()
            .media
            .iter()
            .find(|media| media.id == media_id)
            .cloned()
            .ok_or_else(|| {
                Fault::params(format!("there is no media {media_id} in the project"))
                    .with_data(json!({"reason":"missing", "media":media_id}))
            })?
    };
    let (result, job) = match thumbnail {
        Some(request) => (engine.analysis.thumbnail(media, request).await?, None),
        None => engine.analysis.waveform(media, stream).await?,
    };
    let reply = match connection.queue(rpc::result_line(id, &result)) {
        Ok(()) => Reply::Queued,
        Err(refused) => Reply::Refused(refused),
    };
    if let Some(job) = job {
        job.start();
    }
    Ok(reply)
}

/// `transport.load`, of any client: the media is found in the project, and its load is sent
/// while the state is locked, so that a change that removes it, made after, unloads it after it
/// is loaded; then the load is waited for, with the state let go.
async fn transport_load(
    engine: &Engine,
    params: Option<&RawValue>,
) -> Result<serde_json::Value, Fault> {
    let loading = {
        let mut state = engine.state.lock().await;
        let loading = engine.transport.begin_load(
            state.history.project(),
            state.history.revision(),
            params,
        )?;
        state.window_end("load");
        engine.viewer.frames.stop_input(Monitor::Timeline);
        loading
    };
    loading.done().await
}

/// The params of `hello`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Hello {
    client: Kind,
    protocol: u32,
}

/// The answer to `hello`.
#[derive(Serialize)]
struct HelloResult<'a> {
    engine: &'a str,
    protocol: u32,
    revision: u64,
    project: &'a str,
    pid: u32,
    headless: bool,
}

/// `hello`: say what the client is. From then on the connection is sent every notification.
async fn hello(
    engine: &Engine,
    connection: &mut Connection,
    id: &RawValue,
    params: Option<&RawValue>,
) -> Result<Reply, Reply> {
    if connection.kind.is_some() {
        let fault = Fault::new(
            rpc::INVALID_REQUEST,
            "hello was said on this connection already",
        );
        return Err(fault.into());
    }
    let hello: Hello = rpc::params(
        "hello",
        r#"{"client": "desktop" | "agent" | "cli", "protocol": 2}"#,
        params,
    )?;
    if hello.protocol != rpc::PROTOCOL {
        let fault = Fault::new(
            rpc::INVALID_REQUEST,
            format!(
                "this engine speaks protocol {}, not {}",
                rpc::PROTOCOL,
                hello.protocol
            ),
        )
        .with_data(json!({ "reason": "protocol", "supported": [rpc::PROTOCOL] }));
        return Err(fault.into());
    }
    let mut state = engine.state.lock().await;
    // What was held is told before the revision of the answer, to those who said hello before.
    state.flush();
    state.peers.insert(
        connection.id,
        Peer {
            kind: hello.client,
            queue: connection.queue.clone(),
            stop: connection.stop.clone(),
        },
    );
    connection.kind = Some(hello.client);
    let line = rpc::result_line(
        id,
        &HelloResult {
            engine: ENGINE_VERSION,
            protocol: rpc::PROTOCOL,
            revision: state.history.revision(),
            project: &engine.project,
            pid: engine.pid,
            headless: engine.headless,
        },
    );
    Ok(engine.finish(state, connection, line))
}

/// The answer to `project.get`.
#[derive(Serialize)]
struct Got<'a> {
    revision: u64,
    next_id: u64,
    project: &'a Project,
}

/// `project.get`: the whole project, at the revision that it is at.
async fn project_get(
    engine: &Engine,
    connection: &Connection,
    id: &RawValue,
    params: Option<&RawValue>,
) -> Result<Reply, Reply> {
    rpc::no_params("project.get", params)?;
    let mut state = engine.state.lock().await;
    // What was held is told first: the project answered is never behind what is told after it.
    state.flush();
    let project = state.history.project();
    let line = rpc::result_line(
        id,
        &Got {
            revision: state.history.revision(),
            next_id: project.next_id,
            project,
        },
    );
    Ok(engine.finish(state, connection, line))
}

/// `timeline.duration`: the end of the last clip or subtitle, in timeline frames.
async fn timeline_duration(
    engine: &Engine,
    connection: &Connection,
    id: &RawValue,
    params: Option<&RawValue>,
) -> Result<Reply, Reply> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Duration {
        timeline: String,
    }
    let request: Duration = rpc::params("timeline.duration", r#"{"timeline": "t1"}"#, params)?;
    let state = engine.state.lock().await;
    let timeline = state
        .history
        .project()
        .timelines
        .iter()
        .find(|t| t.id == request.timeline)
        .ok_or_else(|| {
            Fault::params(format!(
                "there is no timeline {} in the project",
                request.timeline
            ))
            .with_data(json!({"reason": "missing", "timeline": request.timeline}))
        })?;
    let line = rpc::result_line(
        id,
        &json!({"frames": columbina_model::resolve::duration(timeline)}),
    );
    Ok(engine.finish(state, connection, line))
}

/// The params of `transaction.apply`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Transaction<'a> {
    #[serde(borrow)]
    ops: Vec<&'a RawValue>,
    revision: Option<u64>,
    #[serde(default)]
    gesture: Option<GestureParams>,
}

/// The gesture that a transaction is part of: its id, which the client chooses, and whether it
/// ends with this transaction.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GestureParams {
    id: String,
    #[serde(default)]
    end: bool,
}

/// A committed transaction and the time its held notification needs to be flushed.
/// Its serialized fields are the unchanged answer to `transaction.apply`.
#[derive(Debug, Serialize)]
struct PreparedResult {
    revision: u64,
    done: Box<RawValue>,
    next_id: u64,
    names: BTreeMap<String, String>,
    clamped: Vec<usize>,
    #[serde(skip)]
    due: Option<Instant>,
}

/// Kept by the native gesture's caller. The base never advances, even across retries.
#[derive(Debug)]
struct OwnedGesture {
    owner: u64,
    key: String,
    base: u64,
    last_own: Option<u64>,
}

/// Preserve the existing wrapper's early-input error and locked-commit reply paths.
#[derive(Debug)]
enum WorkedOutError {
    Input(Fault),
    Apply(Fault),
}

/// `render.start`: check an export against a snapshot of the project, at the revision that the
/// client gives if it gives one, and start it.
async fn render_start(
    engine: &Engine,
    connection: &Connection,
    id: &RawValue,
    method: &str,
    params: Option<&RawValue>,
) -> Result<Reply, Reply> {
    let request: crate::render::Request = rpc::params(
        method,
        r#"{"timeline":"t1","path":"...","codec":"h264","quality":"high"}"#,
        params,
    )?;
    let project = {
        let state = engine.state.lock().await;
        let actual = state.history.revision();
        if let Some(expected) = request.revision.filter(|expected| *expected != actual) {
            return Err(stale_revision(expected, actual).into());
        }
        state.history.project().clone()
    };
    let renders = Arc::clone(&engine.renders);
    let launch = tokio::task::spawn_blocking(move || renders.prepare(project, request))
        .await
        .map_err(|error| Fault::new(rpc::REFUSED, error.to_string()))??;
    let reply = match connection.queue(rpc::result_line(id, &json!({"job":launch.job}))) {
        Ok(()) => Reply::Queued,
        Err(refused) => Reply::Refused(refused),
    };
    launch.start();
    Ok(reply)
}

/// The refusal of a request prepared by the project at revision `expected`, which is now at
/// `actual`: `transaction.apply` and `render.start` given a `revision`.
fn stale_revision(expected: u64, actual: u64) -> Fault {
    Fault::new(
        rpc::REFUSED,
        "the project changed since this request was prepared",
    )
    .with_data(json!({"reason":"stale_revision", "expected":expected, "actual":actual}))
}

/// `transaction.apply`: apply operations, all of them or none.
async fn transaction_apply(
    engine: &Engine,
    connection: &Connection,
    kind: Kind,
    id: &RawValue,
    params: Option<&RawValue>,
) -> Result<Reply, Reply> {
    let transaction: Transaction = rpc::params(
        "transaction.apply",
        r#"{"ops": [...], "revision"?: 12, "gesture"?: {"id": "g1", "end"?: false}}"#,
        params,
    )?;
    if let Some(gesture) = &transaction.gesture {
        if gesture.id.is_empty() || gesture.id.chars().count() > LONGEST_GESTURE {
            return Err(Fault::params(format!(
                "the id of a gesture is from 1 to {LONGEST_GESTURE} characters"
            ))
            .with_data(json!({ "reason": "gesture" }))
            .into());
        }
    }
    // The gestures of a connection are its own: another's of the same id is another gesture.
    let key = transaction
        .gesture
        .as_ref()
        .map(|gesture| (format!("{}:{}", connection.id, gesture.id), gesture.end));
    let mut ops = Vec::with_capacity(transaction.ops.len());
    for (index, op) in transaction.ops.iter().enumerate() {
        let op: names::Input = serde_json::from_str(op.get()).map_err(|error| {
            Fault::params(format!("operation {index} is not an operation: {error}"))
                .with_data(json!({ "index": index, "reason": "invalid_op" }))
        })?;
        ops.push(op);
    }
    check_proxies(&ops, &proxy::proxy_dir(Path::new(&engine.project)))?;
    let names = names::prepare(&mut ops).map_err(|error| {
        Fault::params(error.to_string()).with_data(json!({
            "index": error.index,
            "name": error.name,
            "reason": error.reason,
        }))
    })?;

    let mut state = engine.state.lock().await;
    let now = Instant::now();
    state.end_idle_gesture(now);
    let gesture = key.as_ref().map(|(key, _)| key.as_str());
    if let Some(revision) = transaction
        .revision
        .filter(|revision| !state.history.takes_revision(*revision, gesture))
    {
        let fault = stale_revision(revision, state.history.revision());
        return Ok(engine.finish(state, connection, rpc::error_line(Some(id), &fault)));
    }
    let result = match state.apply_prepared(
        ops.into_iter().map(|input| input.change).collect(),
        names,
        kind,
        key.as_ref().map(|(key, end)| (key.as_str(), *end)),
        now,
    ) {
        Ok(result) => result,
        Err(fault) => {
            return Ok(engine.finish(state, connection, rpc::error_line(Some(id), &fault)));
        }
    };
    if let Some(due) = result.due {
        tell_when_due(engine, due);
    }
    let line = rpc::result_line(id, &result);
    Ok(engine.finish(state, connection, line))
}

/// Tell every connection that said hello `method` with `params` (`gallery.changed`).
pub(crate) async fn tell_everyone(engine: &Engine, method: &str, params: &serde_json::Value) {
    let mut state = engine.state.lock().await;
    state.broadcast(&rpc::notification_line(method, params));
    let kicked = std::mem::take(&mut state.kicked);
    drop(state);
    engine.log_kicked(kicked);
}

/// Read the project as it is now, while the state is locked.
pub(crate) async fn read_project<T>(engine: &Engine, read: impl FnOnce(&Project) -> T) -> T {
    read(engine.state.lock().await.history.project())
}

/// What a transaction worked out by the engine is made of: operations and edits written as the
/// wire writes them, or changes made by the engine itself (the edits of a grade given whole, which
/// the wire does not name).
pub(crate) trait WorkedOut {
    /// The change, the `index`th of its transaction.
    fn input(self, index: usize) -> Result<names::Input, Fault>;
}

impl WorkedOut for Value {
    fn input(self, index: usize) -> Result<names::Input, Fault> {
        serde_json::from_value(self).map_err(|error| {
            Fault::params(format!("operation {index} is not an operation: {error}"))
                .with_data(json!({"index":index, "reason":"invalid_op"}))
        })
    }
}

impl WorkedOut for Change {
    fn input(self, _index: usize) -> Result<names::Input, Fault> {
        Ok(self.into())
    }
}

/// Apply, as one transaction made by `kind`, the operations and edits that `work` makes of the
/// project as it is (`still.apply`, `grade.append_node`), worked out while the project is locked
/// so that it does not change in between; and answer as `transaction.apply` does. `revision`,
/// where given, binds the request as it binds `transaction.apply`.
pub(crate) async fn apply_worked_out<W: WorkedOut>(
    engine: &Engine,
    connection: &Connection,
    kind: Kind,
    id: &RawValue,
    revision: Option<u64>,
    work: impl FnOnce(&Project) -> Result<Vec<W>, Fault>,
) -> Result<Reply, Reply> {
    let mut state = engine.state.lock().await;
    let result = match state.apply_changes_locked(kind, revision, None, Instant::now(), work) {
        Ok(result) => result,
        Err(WorkedOutError::Input(fault)) => return Err(fault.into()),
        Err(WorkedOutError::Apply(fault)) => {
            return Ok(engine.finish(state, connection, rpc::error_line(Some(id), &fault)));
        }
    };
    if let Some(due) = result.due {
        tell_when_due(engine, due);
    }
    let line = rpc::result_line(id, &result);
    Ok(engine.finish(state, connection, line))
}

/// Refuse a transaction that sets a proxy (by `set_proxy`, or on media that it adds) that is not
/// one where the engine keeps proxies (`dir`): the engine reads the proxies of media, and deletes
/// those that are damaged or that it makes anew, so that a proxy elsewhere would have it read,
/// and delete, any file. Taking a proxy away is always taken.
fn check_proxies(ops: &[names::Input], dir: &Path) -> Result<(), Fault> {
    for (index, op) in ops.iter().enumerate() {
        let Change::Op(op) = &op.change else { continue };
        let proxy = match op {
            Op::SetProxy { proxy, .. } => proxy.as_ref(),
            Op::AddMedia { media, .. } => media.proxy.as_ref(),
            _ => None,
        };
        if let Some(proxy) = proxy.filter(|proxy| !proxy::owned(dir, Path::new(&proxy.path))) {
            let error = columbina_model::OpError::NotAProxyPath {
                path: proxy.path.clone(),
            };
            return Err(Fault::refused(
                &columbina_model::TransactionError::Refused { index, error },
            ));
        }
    }
    Ok(())
}

/// The answer to `history.undo` and `history.redo` that did something.
#[derive(Serialize)]
struct StepResult<'a> {
    revision: u64,
    done: &'a RawValue,
    next_id: u64,
}

/// `history.undo` and `history.redo`: take back the latest step, or do again the step that
/// was taken back last, whoever made it.
async fn history_step(
    engine: &Engine,
    connection: &Connection,
    kind: Kind,
    id: &RawValue,
    method: &str,
    params: Option<&RawValue>,
) -> Result<Reply, Reply> {
    rpc::no_params(method, params)?;
    let mut state = engine.state.lock().await;
    let step = if method == "history.undo" {
        state.history.undo(kind.as_str())
    } else {
        state.history.redo(kind.as_str())
    };
    // An undo or a redo ends the gesture that was going on, and what it held is told first.
    state.gesture_last = None;
    state.flush();
    let Some(step) = step else {
        let result = json!({ "revision": state.history.revision(), "done": null });
        let line = rpc::result_line(id, &result);
        return Ok(engine.finish(state, connection, line));
    };
    let applied = step.map_err(|error| Fault::refused(&error))?;
    let done = state.publish(&applied);
    let line = rpc::result_line(
        id,
        &StepResult {
            revision: applied.revision,
            done: &done,
            next_id: applied.next_id,
        },
    );
    Ok(engine.finish(state, connection, line))
}

/// `engine.yield`, of the desktop, to a headless engine (an agent's) that holds the project it
/// opens: the engine tells every client that it yields (`engine.yielding`), answers, and once
/// these are written ends as it ends when it is stopped: it takes no more connections, closes
/// those it has, saves the project and lets the lock go, so that the desktop starts its own
/// engine on the project.
async fn engine_yield(
    engine: &Engine,
    connection: &Connection,
    kind: Kind,
    id: &RawValue,
    params: Option<&RawValue>,
) -> Result<Reply, Reply> {
    let method = "engine.yield";
    if kind != Kind::Desktop {
        let fault = Fault::new(
            rpc::INVALID_REQUEST,
            "only the desktop may ask an engine to yield its project",
        );
        return Err(fault.into());
    }
    if !engine.headless {
        let fault = Fault::new(
            rpc::METHOD_NOT_FOUND,
            format!(
                "there is no method {method} in an engine that is not headless: it is the \
                 desktop's own, and ends when its stdin is closed"
            ),
        );
        return Err(fault.into());
    }
    rpc::no_params(method, params)?;
    let mut state = engine.state.lock().await;
    state.broadcast(&rpc::notification_line(
        "engine.yielding",
        &json!({ "to": "desktop" }),
    ));
    let senders: Vec<queue::Sender> = state
        .peers
        .values()
        .map(|peer| peer.queue.clone())
        .collect();
    let line = rpc::result_line(id, &json!({}));
    let reply = engine.finish(state, connection, line);
    let ending = engine.ending();
    tokio::spawn(async move {
        // What was told and answered is written before the connections are closed; a client
        // that does not read it is not waited for long.
        let written = futures_all(senders.iter().map(queue::Sender::written));
        let _ = tokio::time::timeout(YIELD_WRITTEN, written).await;
        ending.notify_one();
    });
    Ok(reply)
}

/// Wait for every one of some futures, one after another.
async fn futures_all<F: std::future::Future<Output = ()>>(futures: impl Iterator<Item = F>) {
    for future in futures {
        future.await;
    }
}

/// How long the answer to `engine.yield` is waited for to be written before the engine ends.
const YIELD_WRITTEN: std::time::Duration = std::time::Duration::from_secs(2);

/// The params of `media.make_proxy`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MakeProxy {
    media: String,
}

/// `media.make_proxy`: make the proxy of media again, if it needs one, though it could not be
/// made before.
async fn make_proxy(
    engine: &Engine,
    connection: &Connection,
    id: &RawValue,
    params: Option<&RawValue>,
) -> Result<Reply, Reply> {
    let method = "media.make_proxy";
    let asked: MakeProxy = rpc::params(method, r#"{"media": "m1"}"#, params)?;
    let state = engine.state.lock().await;
    let project = state.history.project();
    let Some(media) = project.media.iter().find(|media| media.id == asked.media) else {
        let fault = Fault::params(format!("there is no media {} in the project", asked.media))
            .with_data(json!({ "reason": "missing", "media": asked.media }));
        return Err(fault.into());
    };
    if !state.proxies.retry(media) {
        let fault = Fault::new(
            rpc::METHOD_NOT_FOUND,
            format!(
                "there is no method {method} in an engine that makes no proxies (a headless \
                 one, or one started with --no-proxies)"
            ),
        );
        return Err(fault.into());
    }
    let line = rpc::result_line(id, &json!({}));
    Ok(engine.finish(state, connection, line))
}

/// How many files `media.import` reads at once, of all its calls together.
const READ_AT_ONCE: usize = 8;

/// The params of `media.import`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Import {
    paths: Vec<String>,
    bin: Option<String>,
}

/// Media metadata prepared for a larger transaction, without importing it yet.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Inspect {
    paths: Vec<String>,
}

async fn media_inspect(
    engine: &Engine,
    connection: &Connection,
    id: &RawValue,
    params: Option<&RawValue>,
) -> Result<Reply, Reply> {
    let inspect: Inspect = rpc::params("media.inspect", r#"{"paths": ["<file>", ...]}"#, params)?;
    check_paths("media.inspect", &inspect.paths)?;
    let media = read_all(&engine.reading, engine.read_gate.as_ref(), inspect.paths).await?;
    let line = rpc::result_line(id, &json!({"media":media}));
    Ok(engine.finish(engine.state.lock().await, connection, line))
}

fn check_paths(method: &str, paths: &[String]) -> Result<(), Fault> {
    if paths.is_empty() {
        return Err(Fault::params(format!("{method} takes at least one path")));
    }
    if let Some((index, path)) = paths
        .iter()
        .enumerate()
        .find(|(_, path)| !Path::new(path).is_absolute())
    {
        return Err(Fault::params(format!(
            "{method} takes absolute paths, and {path} is not one"
        ))
        .with_data(json!({"index":index,"path":path,"reason":"relative"})));
    }
    Ok(())
}

/// The answer to `media.import`.
#[derive(Serialize)]
struct Imported<'a> {
    revision: u64,
    added: &'a [String],
    next_id: u64,
}

/// `media.import`: take files in as media, each as it is found to be, in one transaction; or,
/// if one of them cannot be read, none of them.
///
/// The files are read on threads of their own, a few at once, before the state is locked: so
/// that the engine goes on answering while they are read. A path must be absolute: the engine's
/// own directory means nothing to a client.
async fn media_import(
    engine: &Engine,
    connection: &Connection,
    kind: Kind,
    id: &RawValue,
    params: Option<&RawValue>,
) -> Result<Reply, Reply> {
    let import: Import = rpc::params("media.import", r#"{"paths": ["<file>", ...]}"#, params)?;
    check_paths("media.import", &import.paths)?;
    let media = read_all(&engine.reading, engine.read_gate.as_ref(), import.paths).await?;
    let ops = media
        .into_iter()
        .map(|mut media| {
            media.bin.clone_from(&import.bin);
            Op::AddMedia { media, index: None }
        })
        .collect();
    let mut state = engine.state.lock().await;
    let applied = state
        .history
        .apply(ops, kind.as_str())
        .map_err(|error| Fault::refused(&error))?;
    let added: Vec<String> = applied
        .done
        .iter()
        .filter_map(|op| match op {
            Op::AddMedia { media, .. } => Some(media.id.clone()),
            _ => None,
        })
        .collect();
    state.publish(&applied);
    let line = rpc::result_line(
        id,
        &Imported {
            revision: applied.revision,
            added: &added,
            next_id: applied.next_id,
        },
    );
    Ok(engine.finish(state, connection, line))
}

/// Read each file as media, in the order given, at most [`READ_AT_ONCE`] at once of all the
/// calls; the error of the first in the order given that cannot be read. Once one cannot be
/// read, no other is begun.
async fn read_all(
    reading: &Arc<Semaphore>,
    gate: Option<&PathBuf>,
    paths: Vec<String>,
) -> Result<Vec<Media>, Fault> {
    let stop = Arc::new(AtomicBool::new(false));
    let mut tasks = Vec::with_capacity(paths.len());
    for path in paths {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let permit = Arc::clone(reading)
            .acquire_owned()
            .await
            .expect("the semaphore is never closed");
        let stopped = Arc::clone(&stop);
        let read_path = path.clone();
        let gate = gate.cloned();
        let task = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            if let Some(gate) = gate {
                wait_for(&gate);
            }
            if stopped.load(Ordering::Relaxed) {
                return None;
            }
            let read = read(&read_path);
            if read.is_err() {
                stopped.store(true, Ordering::Relaxed);
            }
            Some(read)
        });
        tasks.push((path, task));
    }
    let mut media = Vec::with_capacity(tasks.len());
    let mut failed = None;
    for (path, task) in tasks {
        match task.await {
            Ok(Some(Ok(read))) => media.push(read),
            // Not begun, since another could not be read.
            Ok(None) => {}
            Ok(Some(Err(error))) => {
                failed
                    .get_or_insert_with(|| cannot_read(&path, error.reason(), &error.to_string()));
            }
            Err(error) => {
                failed.get_or_insert_with(|| {
                    cannot_read(
                        &path,
                        "failed",
                        &format!("the engine failed to read it: {error}"),
                    )
                });
            }
        }
    }
    match failed {
        Some(fault) => Err(fault),
        None => Ok(media),
    }
}

/// The error of a file that cannot be read.
pub fn cannot_read(path: &str, reason: &str, message: &str) -> Fault {
    Fault::new(
        rpc::CANNOT_READ,
        format!("a file cannot be read: {path}: {message}"),
    )
    .with_data(json!({ "path": path, "reason": reason, "message": message }))
}

/// Wait until a file is there, or a minute has passed: the gate of `--read-gate`, which only
/// the tests give.
fn wait_for(gate: &Path) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !gate.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// A file as media, with no id yet: its path, which is absolute, written as the system writes
/// it, its identity and what it holds. This blocks.
fn read(path: &str) -> Result<Media, MediaError> {
    let full = std::path::absolute(Path::new(path))?;
    let identity = columbina_media::identity(&full)?;
    let facts = columbina_media::probe(&full)?;
    let name = full
        .file_name()
        .map(|name| name.to_str().ok_or(MediaError::NotUtf8))
        .transpose()?
        .unwrap_or_default()
        .to_owned();
    Ok(Media {
        id: String::new(),
        path: full.to_str().ok_or(MediaError::NotUtf8)?.to_owned(),
        identity,
        facts,
        proxy: None,
        name,
        bin: None,
        mark_in: None,
        mark_out: None,
        color_override: columbina_model::ColorOverride::default(),
    })
}
