//! The engine as its clients see it: the project and its history, the connections that said
//! hello, and what each method does.
//!
//! Every change to the project is made while [`State`] is locked, and so is everything that is
//! sent about it: the notification of a change is queued on every connection that said hello,
//! then the answer to whoever asked for it, all before the lock is let go. So each connection
//! is sent the changes in the order of their revisions, and a client sees its own change as a
//! notification before it sees the answer to its request. Nothing is awaited while the lock is
//! held.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use columbina_model::{Applied, History, Op};
use columbina_store::Saver;
use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_json::value::RawValue;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, watch, Mutex};

use crate::log;
use crate::names;
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
    /// How many messages may wait to be sent on a connection before it is closed.
    pub queue_capacity: usize,
    /// Who is connected.
    pub presence: watch::Sender<Presence>,
    next_connection: AtomicU64,
}

/// What is changed only while it is locked.
pub struct State {
    history: History,
    /// What saves the project after each change. It is taken when the engine ends.
    pub saver: Option<Saver>,
    /// The connections that said hello, each of which is sent every notification.
    peers: HashMap<u64, Peer>,
}

/// A connection that said hello.
struct Peer {
    kind: Kind,
    queue: mpsc::Sender<Line>,
    stop: watch::Sender<bool>,
}

/// A connection, as its reader holds it.
pub struct Connection {
    /// The number of the connection, for the log.
    pub id: u64,
    /// What the client said it is, once it said hello.
    kind: Option<Kind>,
    /// The messages to be sent on it.
    queue: mpsc::Sender<Line>,
    /// Set to stop it.
    stop: watch::Sender<bool>,
}

impl Connection {
    pub fn new(id: u64, queue: mpsc::Sender<Line>, stop: watch::Sender<bool>) -> Self {
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

    /// Queue a message to be sent. Gives false if the queue is full, or the connection is
    /// being closed: then it is to be closed.
    pub fn queue(&self, line: Line) -> bool {
        self.queue.try_send(line).is_ok()
    }
}

impl Engine {
    pub fn new(history: History, saver: Saver, project: String, queue_capacity: usize) -> Engine {
        Engine {
            state: Arc::new(Mutex::new(State {
                history,
                saver: Some(saver),
                peers: HashMap::new(),
            })),
            project,
            pid: std::process::id(),
            queue_capacity,
            presence: watch::Sender::new(Presence::default()),
            next_connection: AtomicU64::new(1),
        }
    }

    /// A number for a new connection, and count it as connected.
    pub fn arrive(&self) -> u64 {
        self.presence.send_modify(|presence| {
            presence.connected += 1;
            presence.arrivals += 1;
        });
        self.next_connection.fetch_add(1, Ordering::Relaxed)
    }

    /// Forget a connection that has ended.
    pub async fn leave(&self, id: u64) {
        self.state.lock().await.peers.remove(&id);
        self.presence
            .send_modify(|presence| presence.connected -= 1);
    }

    /// Tell every client that the project could not be saved.
    pub async fn report_save_error(&self, message: &str) {
        log("save_failed", &json!({ "message": message }));
        let line = rpc::notification_line(
            "engine.error",
            &json!({ "message": message, "kind": "save" }),
        );
        self.state.lock().await.broadcast(&line);
    }
}

impl State {
    /// Queue a line on every connection that said hello. A connection whose queue is full is
    /// closed, so that it never misses a single message and then goes on.
    fn broadcast(&mut self, line: &Line) {
        let mut full = Vec::new();
        for (id, peer) in &self.peers {
            match peer.queue.try_send(line.clone()) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => full.push((*id, true)),
                Err(TrySendError::Closed(_)) => full.push((*id, false)),
            }
        }
        for (id, was_full) in full {
            if let Some(peer) = self.peers.remove(&id) {
                let _ = peer.stop.send(true);
                if was_full {
                    log(
                        "kicked",
                        &json!({
                            "connection": id,
                            "client": peer.kind,
                            "reason": "the client does not read what it is sent, and its queue is full",
                        }),
                    );
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

    /// Whether a connection is still told of changes.
    fn has(&self, connection: &Connection) -> bool {
        self.peers.contains_key(&connection.id)
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
    Closed,
}

impl From<Fault> for Reply {
    fn from(fault: Fault) -> Self {
        Reply::Error(fault)
    }
}

/// Queue the answer to a request while the state is locked. A connection that was closed by
/// the notification that went before it is not answered.
fn answer<T: Serialize + ?Sized>(
    state: &State,
    connection: &Connection,
    id: &RawValue,
    result: &T,
) -> Reply {
    if state.has(connection) && connection.queue(rpc::result_line(id, result)) {
        Reply::Queued
    } else {
        Reply::Closed
    }
}

/// Answer one line that a client sent. Gives false if the connection is to be closed.
pub async fn handle(engine: &Engine, connection: &mut Connection, line: &[u8]) -> bool {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    if line.iter().all(u8::is_ascii_whitespace) {
        return true;
    }
    let request = match rpc::parse(line) {
        Ok(request) => request,
        Err((id, fault)) => return connection.queue(rpc::error_line(id, &fault)),
    };
    // The engine takes no notifications from clients.
    let Some(id) = request.id else {
        return true;
    };
    let reply = match dispatch(engine, connection, id, &request.method, request.params).await {
        Ok(reply) | Err(reply) => reply,
    };
    match reply {
        Reply::Error(fault) => connection.queue(rpc::error_line(Some(id), &fault)),
        Reply::Queued => true,
        Reply::Closed => false,
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
    let result = HelloResult {
        engine: ENGINE_VERSION,
        protocol: rpc::PROTOCOL,
        revision: state.history.revision(),
        project: &engine.project,
        pid: engine.pid,
    };
    Ok(answer(&state, connection, id, &result))
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
    let result = json!({
        "revision": state.history.revision(),
        "next_id": project.next_id,
        "project": project,
    });
    Ok(answer(&state, connection, id, &result))
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
    let result = TransactionResult {
        revision: applied.revision,
        done: &done,
        next_id: applied.next_id,
        names: &names,
    };
    Ok(answer(&state, connection, id, &result))
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
        return Ok(answer(&state, connection, id, &result));
    };
    let applied = step.map_err(|error| Fault::refused(&error))?;
    let done = state.publish(&applied);
    let result = StepResult {
        revision: applied.revision,
        done: &done,
        next_id: applied.next_id,
    };
    Ok(answer(&state, connection, id, &result))
}
