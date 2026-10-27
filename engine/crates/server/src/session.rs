//! The engine as its clients see it: the project and its history, the connections that said
//! hello, and what each method does.
//!
//! Every change to the project is made while [`State`] is locked, and so is everything that is
//! sent about it: the notification of a change is queued on every connection that said hello,
//! then the answer to whoever asked for it, all before the lock is let go. So each connection
//! is sent the changes in the order of their revisions, and a client sees its own change as a
//! notification before it sees the answer to its request. Nothing is awaited while the lock is
//! held, and nothing is logged: what is to be logged is kept and logged once it is let go.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use columbina_media::MediaError;
use columbina_model::{Applied, History, Media, Op, Project, Proxy};
use columbina_store::Saver;
use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_json::value::RawValue;
use tokio::sync::{watch, Mutex, MutexGuard, Notify, Semaphore};

use crate::log::log;
use crate::names;
use crate::proxy::{self, Made, Note, Proxies};
use crate::queue::{self, Limits, Refused};
use crate::rpc::{self, Fault, Line};
use crate::transport::{self, Status, Transport};
use crate::view::Viewer;

/// The version of the engine that `hello` answers with.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

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
            saver: Some(saver),
            peers: HashMap::new(),
            kicked: Vec::new(),
            transport: None,
            proxies: proxies.clone(),
        }));
        let viewer = Viewer::new(headless);
        let notify = {
            let state = Arc::clone(&state);
            Arc::new(move |status: &Status| tell(&state, status, limits))
        };
        let again = {
            let state = Arc::clone(&state);
            let dir = proxy::proxy_dir(Path::new(&project));
            Arc::new(move |media: &str, damaged: Option<&Path>| {
                if let Some(damaged) = damaged {
                    proxy::delete(&dir, damaged);
                }
                make_again(&state, media);
            })
        };
        let transport = Transport::new(headless, setup, viewer.slot(), notify, again);
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
            headless,
            yielded: Arc::new(Notify::new()),
        }
    }

    /// What is told when the engine is to end, having yielded the project.
    fn ending(&self) -> Arc<Notify> {
        Arc::clone(&self.yielded)
    }

    /// Stop the proxy being made, and delete what it wrote: the engine ends. This blocks for a
    /// few seconds at most; the state must not be locked meanwhile.
    pub fn stop_proxies(&self) {
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

    /// Forget a connection that has ended.
    pub async fn leave(&self, id: u64) {
        self.state.lock().await.peers.remove(&id);
        self.connections().stops.remove(&id);
        self.presence
            .send_modify(|presence| presence.connected -= 1);
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

/// Make the proxy of media again, as the transport asks when its file cannot be opened or
/// decoded. This is called by the transport's own thread.
fn make_again(state: &Mutex<State>, media: &str) {
    let state = state.blocking_lock();
    let project = state.history.project();
    if let Some(found) = project.media.iter().find(|found| found.id == media) {
        state.proxies.again(found);
    }
}

/// Tell every connection that said hello the state of the transport. This is called by the
/// transport's own thread, never by a task of the runtime: it waits for the state of the
/// engine, which is held only for moments.
fn tell(state: &Mutex<State>, status: &Status, limits: Limits) {
    let line = rpc::notification_line("transport.state", status);
    let mut state = state.blocking_lock();
    state.broadcast(&line);
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

    /// Save the project as it is now, and tell everyone what changed. Gives what was done, as
    /// it is written.
    fn publish(&mut self, applied: &Applied) -> Box<RawValue> {
        if let Some(saver) = &self.saver {
            saver.changed(self.history.project().clone());
        }
        let done = serde_json::value::to_raw_value(&applied.done)
            .expect("an operation is written as JSON");
        let changed = Changed {
            revision: applied.revision,
            by: &applied.by,
            made_by: &applied.made_by,
            done: &done,
            next_id: applied.next_id,
        };
        self.broadcast(&rpc::notification_line("project.changed", &changed));
        if let Some(transport) = &self.transport {
            transport.project_changed(self.history.project());
        }
        for note in self.proxies.project_changed(self.history.project()) {
            self.broadcast(&rpc::notification_line("media.proxy", &note));
        }
        done
    }
}

/// The params of `project.changed`.
#[derive(Serialize)]
struct Changed<'a> {
    revision: u64,
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
        "transaction.apply" => transaction_apply(engine, connection, kind, id, params).await,
        "history.undo" | "history.redo" => {
            history_step(engine, connection, kind, id, method, params).await
        }
        "media.import" => media_import(engine, connection, kind, id, params).await,
        "media.make_proxy" => make_proxy(engine, connection, id, params).await,
        "engine.yield" => engine_yield(engine, connection, kind, id, params).await,
        "view.attach" | "view.place" | "view.show" | "view.detach" => {
            let result = engine.viewer.call(kind, method, params).await?;
            if method == "view.attach" {
                engine.transport.redraw();
            }
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
        _ if method.starts_with("transport.") => {
            // The answer is queued before the command is sent, so that the state that it makes
            // is told after the answer.
            let (result, order) = engine.transport.prepare(method, params)?;
            let reply = match connection.queue(rpc::result_line(id, &result)) {
                Ok(()) => Reply::Queued,
                Err(refused) => Reply::Refused(refused),
            };
            order.send();
            Ok(reply)
        }
        _ => Err(Fault::new(
            rpc::METHOD_NOT_FOUND,
            format!("there is no method {method}"),
        )
        .into()),
    }
}

/// `transport.load`, of any client: the media is found in the project, and its load is sent
/// while the state is locked, so that a change that removes it, made after, unloads it after it
/// is loaded; then the load is waited for, with the state let go.
async fn transport_load(
    engine: &Engine,
    params: Option<&RawValue>,
) -> Result<serde_json::Value, Fault> {
    let method = "transport.load";
    engine.transport.check(method)?;
    let load: transport::Load = rpc::params(method, r#"{"media": "m1"}"#, params)?;
    let loading = {
        let state = engine.state.lock().await;
        let project = state.history.project();
        let Some(media) = project.media.iter().find(|media| media.id == load.media) else {
            return Err(
                Fault::params(format!("there is no media {} in the project", load.media))
                    .with_data(json!({ "reason": "missing", "media": load.media })),
            );
        };
        let grid = project.timelines.first().map(|timeline| timeline.rate);
        engine.transport.begin_load(
            &load.media,
            PathBuf::from(&media.path),
            proxy::usable(media).map(|proxy| PathBuf::from(&proxy.path)),
            media.facts.clone(),
            grid,
        )?
    };
    let status = loading.done().await?;
    Ok(serde_json::to_value(status).unwrap_or_default())
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
        r#"{"client": "desktop" | "agent" | "cli", "protocol": 1}"#,
        params,
    )?;
    if hello.protocol != rpc::PROTOCOL {
        let fault = Fault::params(format!(
            "this engine speaks protocol {}, not {}",
            rpc::PROTOCOL,
            hello.protocol
        ))
        .with_data(json!({ "protocol": rpc::PROTOCOL }));
        return Err(fault.into());
    }
    let mut state = engine.state.lock().await;
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
    let state = engine.state.lock().await;
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

/// The params of `transaction.apply`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Transaction<'a> {
    #[serde(borrow)]
    ops: Vec<&'a RawValue>,
}

/// The answer to `transaction.apply`.
#[derive(Serialize)]
struct TransactionResult<'a> {
    revision: u64,
    done: &'a RawValue,
    next_id: u64,
    names: &'a BTreeMap<String, String>,
}

/// `transaction.apply`: apply operations, all of them or none.
async fn transaction_apply(
    engine: &Engine,
    connection: &Connection,
    kind: Kind,
    id: &RawValue,
    params: Option<&RawValue>,
) -> Result<Reply, Reply> {
    let transaction: Transaction = rpc::params("transaction.apply", r#"{"ops": [...]}"#, params)?;
    let mut ops = Vec::with_capacity(transaction.ops.len());
    for (index, op) in transaction.ops.iter().enumerate() {
        let op: Op = serde_json::from_str(op.get()).map_err(|error| {
            Fault::params(format!("operation {index} is not an operation: {error}"))
                .with_data(json!({ "index": index, "reason": "invalid_op" }))
        })?;
        ops.push(op);
    }
    check_proxies(&ops, &proxy::proxy_dir(Path::new(&engine.project)))?;
    let mut names = names::prepare(&mut ops).map_err(|error| {
        Fault::params(error.to_string()).with_data(json!({
            "index": error.index,
            "name": error.name,
            "reason": error.reason,
        }))
    })?;

    let mut state = engine.state.lock().await;
    let applied = state
        .history
        .apply_mapped(ops, kind.as_str(), |op, done| names.map(op, done))
        .map_err(|error| Fault::refused(&error))?;
    let names = names.finish(&applied.done);
    let done = state.publish(&applied);
    let line = rpc::result_line(
        id,
        &TransactionResult {
            revision: applied.revision,
            done: &done,
            next_id: applied.next_id,
            names: &names,
        },
    );
    Ok(engine.finish(state, connection, line))
}

/// Refuse a transaction that sets a proxy (by `set_proxy`, or on media that it adds) that is not
/// one where the engine keeps proxies (`dir`): the engine reads the proxies of media, and deletes
/// those that are damaged or that it makes anew, so that a proxy elsewhere would have it read,
/// and delete, any file. Taking a proxy away is always taken.
fn check_proxies(ops: &[Op], dir: &Path) -> Result<(), Fault> {
    for (index, op) in ops.iter().enumerate() {
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
    if import.paths.is_empty() {
        return Err(Fault::params("media.import takes at least one path").into());
    }
    if let Some((index, path)) = import
        .paths
        .iter()
        .enumerate()
        .find(|(_, path)| !Path::new(path).is_absolute())
    {
        let fault = Fault::params(format!(
            "media.import takes absolute paths, and {path} is not one"
        ))
        .with_data(json!({ "index": index, "path": path, "reason": "relative" }));
        return Err(fault.into());
    }
    let media = read_all(&engine.reading, engine.read_gate.as_ref(), import.paths).await?;
    let ops = media
        .into_iter()
        .map(|media| Op::AddMedia { media, index: None })
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
    state.proxies.imported(state.history.project(), &added);
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
    Ok(Media {
        id: String::new(),
        path: full.to_str().ok_or(MediaError::NotUtf8)?.to_owned(),
        identity,
        facts,
        proxy: None,
    })
}
