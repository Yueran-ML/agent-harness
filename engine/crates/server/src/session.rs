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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use columbina_model::{Applied, History, Op, Project};
use columbina_store::Saver;
use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_json::value::RawValue;
use tokio::sync::{watch, Mutex, MutexGuard};

use crate::log::log;
use crate::names;
use crate::queue::{self, Limits, Refused};
use crate::rpc::{self, Fault, Line};

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
    pub fn new(history: History, saver: Saver, project: String, limits: Limits) -> Engine {
        Engine {
            state: Arc::new(Mutex::new(State {
                history,
                saver: Some(saver),
                peers: HashMap::new(),
                kicked: Vec::new(),
            })),
            project,
            pid: std::process::id(),
            limits,
            presence: watch::Sender::new(Presence::default()),
            connections: std::sync::Mutex::default(),
            next_connection: AtomicU64::new(1),
        }
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
        _ => Err(Fault::new(
            rpc::METHOD_NOT_FOUND,
            format!("there is no method {method}"),
        )
        .into()),
    }
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
