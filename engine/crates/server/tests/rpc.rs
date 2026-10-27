//! The engine as its clients see it: the binary itself, started on a directory of its own, and
//! clients that connect to it as the desktop and the agent do, by the address that it gives.

use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, Command};
use tokio::task::JoinHandle;

/// The media that the tests make, shared with the tests of `columbina-media`.
#[path = "../../media/tests/support/mod.rs"]
mod support;

/// How long anything that should happen soon is waited for before the test fails.
const WAIT: Duration = Duration::from_secs(30);

/// A new directory for a project.
fn project_dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

/// A directory as the engine writes it: in full, and on Windows without `\\?\`.
fn written(dir: &Path) -> String {
    let full = std::fs::canonicalize(dir).unwrap();
    let text = full.to_str().unwrap().to_owned();
    text.strip_prefix(r"\\?\")
        .map_or(text.clone(), str::to_owned)
}

/// An engine that was started, as its owner sees it.
struct Engine {
    child: Child,
    address: String,
    pid: u32,
    stderr: Arc<Mutex<String>>,
    /// Its stderr, where it is not read.
    _unread: Option<ChildStderr>,
    /// What it writes to stdout after its first line.
    stdout: JoinHandle<String>,
}

/// What becomes of the stderr of an engine that is started.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stderr {
    /// It is read, a line at a time.
    Read,
    /// The pipe is closed at once, as when its owner has ended.
    Closed,
    /// It is never read, so that the pipe fills.
    Unread,
}

/// What came of starting an engine.
enum Started {
    Listening(Box<Engine>),
    Ended { code: i32, stderr: String },
}

/// Start an engine on `dir`, and read its first line.
async fn start(dir: &Path, args: &[&str]) -> Started {
    start_with(dir, args, Stderr::Read).await
}

/// Start an engine on `dir` whose stderr is read or not, and read its first line.
async fn start_with(dir: &Path, args: &[&str], mode: Stderr) -> Started {
    let mut child = Command::new(env!("CARGO_BIN_EXE_columbina-engine"))
        .arg("--project")
        .arg(dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let stderr = Arc::new(Mutex::new(String::new()));
    let pipe = child.stderr.take().unwrap();
    let mut unread = None;
    let reading_stderr = match mode {
        Stderr::Read => {
            let mut errors = BufReader::new(pipe).lines();
            let stderr = Arc::clone(&stderr);
            tokio::spawn(async move {
                while let Ok(Some(line)) = errors.next_line().await {
                    let mut stderr = stderr.lock().unwrap();
                    stderr.push_str(&line);
                    stderr.push('\n');
                }
            })
        }
        Stderr::Closed => {
            drop(pipe);
            tokio::spawn(async {})
        }
        Stderr::Unread => {
            unread = Some(pipe);
            tokio::spawn(async {})
        }
    };
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut first = String::new();
    tokio::time::timeout(WAIT, stdout.read_line(&mut first))
        .await
        .expect("the engine says where it listens")
        .unwrap();
    if first.is_empty() {
        let status = tokio::time::timeout(WAIT, child.wait())
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(WAIT, reading_stderr)
            .await
            .unwrap()
            .unwrap();
        let stderr = stderr.lock().unwrap().clone();
        return Started::Ended {
            code: status.code().unwrap(),
            stderr,
        };
    }
    let line: Value = serde_json::from_str(&first).unwrap();
    let keys: Vec<&String> = line.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["listening", "pid", "protocol"], "{line}");
    assert_eq!(line["protocol"], 1);
    let pid = u32::try_from(line["pid"].as_u64().unwrap()).unwrap();
    assert_eq!(Some(pid), child.id());
    let rest = tokio::spawn(async move {
        let mut rest = String::new();
        let _ = stdout.read_to_string(&mut rest).await;
        rest
    });
    Started::Listening(Box::new(Engine {
        child,
        address: line["listening"].as_str().unwrap().to_owned(),
        pid,
        stderr,
        _unread: unread,
        stdout: rest,
    }))
}

/// Start an engine on `dir` that must start.
async fn engine(dir: &Path, args: &[&str]) -> Engine {
    match start(dir, args).await {
        Started::Listening(engine) => *engine,
        Started::Ended { code, stderr } => panic!("the engine ended with {code}: {stderr}"),
    }
}

/// Start an engine on `dir` that must end at once, and give its code and its line of stderr.
async fn refused(dir: &Path, args: &[&str]) -> (i32, Value) {
    match start(dir, args).await {
        Started::Listening(engine) => panic!("the engine listens at {}", engine.address),
        Started::Ended { code, stderr } => {
            let line = stderr.lines().next().expect("a line on stderr");
            (code, serde_json::from_str(line).unwrap())
        }
    }
}

impl Engine {
    /// Wait for the engine to end, and give its code once it is checked that it wrote nothing
    /// more to stdout.
    async fn ended(mut self) -> i32 {
        let status = tokio::time::timeout(WAIT, self.child.wait())
            .await
            .expect("the engine ends")
            .unwrap();
        let rest = tokio::time::timeout(WAIT, self.stdout)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rest, "", "nothing more on stdout");
        status.code().unwrap()
    }

    fn running(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }

    fn stderr(&self) -> String {
        self.stderr.lock().unwrap().clone()
    }

    async fn client(&self, kind: &str) -> Client {
        let mut client = Client::connect(&self.address).await;
        client.hello(kind).await;
        client
    }
}

type Reader = BufReader<Box<dyn AsyncRead + Unpin + Send>>;
type Writer = Box<dyn AsyncWrite + Unpin + Send>;

/// A client of the engine, which reads and writes lines of JSON.
struct Client {
    reader: Reader,
    writer: Writer,
    next_id: u64,
    /// The notifications that came while the client waited for answers, in order.
    notifications: Vec<Value>,
}

impl Client {
    /// Open the address that the engine gave, as a client in another language would.
    async fn connect(address: &str) -> Client {
        #[cfg(windows)]
        let stream = {
            use tokio::net::windows::named_pipe::ClientOptions;
            const PIPE_BUSY: i32 = 231;
            let deadline = Instant::now() + WAIT;
            loop {
                match ClientOptions::new().open(address) {
                    Ok(pipe) => break pipe,
                    Err(error)
                        if error.raw_os_error() == Some(PIPE_BUSY) && Instant::now() < deadline =>
                    {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    Err(error) => panic!("cannot open {address}: {error}"),
                }
            }
        };
        #[cfg(unix)]
        let stream = tokio::net::UnixStream::connect(address).await.unwrap();
        let (reader, writer) = tokio::io::split(stream);
        Client {
            reader: BufReader::new(Box::new(reader)),
            writer: Box::new(writer),
            next_id: 1,
            notifications: Vec::new(),
        }
    }

    async fn send_raw(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).await.unwrap();
    }

    async fn send(&mut self, message: &Value) {
        let mut line = serde_json::to_vec(message).unwrap();
        line.push(b'\n');
        self.send_raw(&line).await;
    }

    /// The next message that the engine sends.
    async fn message(&mut self) -> Value {
        let message = read_message(&mut self.reader).await;
        message.expect("the engine closed the connection")
    }

    /// Send a request, and give its id.
    async fn request(&mut self, method: &str, params: Option<Value>) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let mut request = json!({ "jsonrpc": "2.0", "id": id, "method": method });
        if let Some(params) = params {
            request["params"] = params;
        }
        self.send(&request).await;
        id
    }

    /// Read until the answer to `id` comes, keeping the notifications that come before it.
    async fn answer(&mut self, id: u64) -> Value {
        loop {
            let message = self.message().await;
            if message.get("id").is_some() {
                assert_eq!(message["id"], id, "{message}");
                assert_eq!(message["jsonrpc"], "2.0");
                return message;
            }
            self.notifications.push(message);
        }
    }

    /// Call a method, and give the whole answer.
    async fn call(&mut self, method: &str, params: Option<Value>) -> Value {
        let id = self.request(method, params).await;
        self.answer(id).await
    }

    /// Call a method that must succeed, and give its result.
    async fn result(&mut self, method: &str, params: Option<Value>) -> Value {
        let answer = self.call(method, params).await;
        assert!(answer.get("error").is_none(), "{answer}");
        answer["result"].clone()
    }

    /// Call a method that must fail, and give its error.
    async fn error(&mut self, method: &str, params: Option<Value>) -> Value {
        let answer = self.call(method, params).await;
        assert!(answer.get("result").is_none(), "{answer}");
        answer["error"].clone()
    }

    async fn hello(&mut self, kind: &str) -> Value {
        self.result("hello", Some(json!({ "client": kind, "protocol": 1 })))
            .await
    }

    async fn apply(&mut self, ops: Value) -> Value {
        self.result("transaction.apply", Some(json!({ "ops": ops })))
            .await
    }

    /// The notifications that came, and forget them.
    fn take_notifications(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.notifications)
    }

    /// Read what is left until the engine closes the connection, and give it.
    async fn rest(mut self) -> Vec<Value> {
        let mut rest = Vec::new();
        while let Some(message) = read_message(&mut self.reader).await {
            rest.push(message);
        }
        rest
    }
}

/// The next message on a connection, or none if it was closed.
async fn read_message(reader: &mut Reader) -> Option<Value> {
    let mut line = String::new();
    let read = tokio::time::timeout(WAIT, reader.read_line(&mut line))
        .await
        .expect("a message from the engine");
    match read {
        // A line cut short is what was being written when the connection was closed.
        Ok(_) if line.ends_with('\n') => Some(serde_json::from_str(&line).unwrap()),
        _ => None,
    }
}

/// Media with picture and sound, ten seconds long.
fn media(id: &str, path: &str) -> Value {
    json!({
        "id": id,
        "path": path,
        "identity": { "size": 1000, "modified_us": 1_790_000_000_000_000_i64, "digest": "0".repeat(64) },
        "facts": {
            "duration": { "ticks": 153_600, "scale": 15360 },
            "video": {
                "width": 1920, "height": 1080, "rate": { "num": 25, "den": 1 },
                "codec": "h264", "bits": 8, "rotation": 0,
            },
            "audio": { "rate": 48000, "channels": 2, "codec": "aac" },
        },
    })
}

fn clip(id: &str, media: &str, start: i64, length: i64) -> Value {
    json!({
        "id": id, "media": media, "source_in": { "ticks": 0, "scale": 15360 },
        "start": start, "length": length,
    })
}

fn add_media(path: &str) -> Value {
    json!([{ "op": "add_media", "media": media("", path) }])
}

/// The project as `project.get` gives it.
async fn project(client: &mut Client) -> Value {
    client.result("project.get", None).await
}

/// The project as it was saved.
fn saved(dir: &Path) -> Value {
    let text = std::fs::read_to_string(dir.join("project.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

#[tokio::test]
async fn the_engine_says_where_it_listens_and_answers_hello() {
    let dir = project_dir();
    let engine = engine(dir.path(), &["--headless", "--idle-seconds", "1"]).await;
    let hash = if cfg!(windows) {
        engine
            .address
            .strip_prefix(r"\\.\pipe\columbina-engine-")
            .unwrap()
            .to_owned()
    } else {
        let path = Path::new(&engine.address);
        assert!(!path.starts_with(dir.path()), "not in the project");
        let name = path.file_name().unwrap().to_str().unwrap();
        name.strip_suffix(".sock").unwrap().to_owned()
    };
    assert_eq!(hash.len(), 16);
    assert!(hash
        .bytes()
        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
    let lock = std::fs::read_to_string(dir.path().join("engine.lock")).unwrap();
    let lock: Value = serde_json::from_str(&lock).unwrap();
    assert_eq!(lock["address"], engine.address.as_str());
    assert_eq!(lock["pid"], engine.pid);

    let mut client = Client::connect(&engine.address).await;
    let hello = client.hello("agent").await;
    assert_eq!(
        hello,
        json!({
            "engine": "0.1.0",
            "protocol": 1,
            "revision": 0,
            "project": written(dir.path()),
            "pid": engine.pid,
        })
    );
    let got = project(&mut client).await;
    assert_eq!(got["revision"], 0);
    assert_eq!(got["next_id"], 1);
    assert_eq!(
        got["project"],
        json!({ "schema": 1, "next_id": 1, "media": [], "timelines": [] })
    );
    drop(client);
    assert_eq!(engine.ended().await, 0);
    assert!(!dir.path().join("engine.lock").exists());
}

#[tokio::test]
async fn requests_that_cannot_be_answered_are_refused_with_their_codes() {
    let dir = project_dir();
    let engine = engine(dir.path(), &["--headless"]).await;
    let mut client = Client::connect(&engine.address).await;

    let error = client.error("project.get", None).await;
    assert_eq!(error["code"], -32600, "{error}");
    assert!(error["message"].as_str().unwrap().contains("hello"));
    let error = client.error("no.such.method", None).await;
    assert_eq!(
        error["code"], -32600,
        "before hello, even a method that is not there"
    );

    for params in [
        json!({ "client": "robot", "protocol": 1 }),
        json!({ "client": "agent" }),
        json!({ "client": "agent", "protocol": 2 }),
        json!({ "client": "agent", "protocol": 1, "colour": "red" }),
        json!([1]),
        json!(["agent", 1]),
    ] {
        let error = client.error("hello", Some(params.clone())).await;
        assert_eq!(error["code"], -32602, "{params}: {error}");
    }
    let error = client.error("hello", None).await;
    assert_eq!(error["code"], -32602);

    client.hello("desktop").await;
    let error = client
        .error("hello", Some(json!({ "client": "agent", "protocol": 1 })))
        .await;
    assert_eq!(error["code"], -32600, "hello once");
    let error = client.error("no.such.method", None).await;
    assert_eq!(error["code"], -32601);
    let error = client.error("project.get", Some(json!({ "x": 1 }))).await;
    assert_eq!(error["code"], -32602);
    let error = client.error("history.undo", Some(json!([1]))).await;
    assert_eq!(error["code"], -32602);
    for params in [None, Some(json!({})), Some(json!({ "ops": 3 }))] {
        let error = client.error("transaction.apply", params).await;
        assert_eq!(error["code"], -32602, "{error}");
    }
    let error = client
        .error(
            "transaction.apply",
            Some(
                json!({ "ops": [{ "op": "add_media", "media": media("", "a") }, { "op": "fly" }] }),
            ),
        )
        .await;
    assert_eq!(error["code"], -32602);
    assert_eq!(error["data"]["index"], 1, "{error}");

    // A request of another version of JSON-RPC, answered to its id.
    client
        .send(&json!({ "jsonrpc": "1.0", "id": "x", "method": "project.get" }))
        .await;
    let answer = client.message().await;
    assert_eq!(answer["id"], "x");
    assert_eq!(answer["error"]["code"], -32600);
    // A notification is not answered: the next answer is to the next request.
    client
        .send(&json!({ "jsonrpc": "2.0", "method": "project.get" }))
        .await;
    assert_eq!(project(&mut client).await["revision"], 0);
}

#[tokio::test]
async fn a_line_that_is_not_json_is_refused_and_the_connection_goes_on() {
    let dir = project_dir();
    let engine = engine(dir.path(), &["--headless"]).await;
    let mut client = engine.client("cli").await;
    client.send_raw(b"{nonsense\n").await;
    let answer = client.message().await;
    assert_eq!(answer["id"], Value::Null);
    assert_eq!(answer["error"]["code"], -32700, "{answer}");
    client.send_raw(b"[1, 2]\n\n  \r\n").await;
    let answer = client.message().await;
    assert_eq!(answer["error"]["code"], -32600, "{answer}");
    // A line that ends in \r\n is read as one that ends in \n.
    client
        .send_raw(b"{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"project.get\"}\r\n")
        .await;
    let answer = client.message().await;
    assert_eq!(answer["id"], 9);
    assert_eq!(answer["result"]["revision"], 0);
}

#[tokio::test]
async fn a_line_over_16_mib_is_refused_and_the_connection_goes_on() {
    let dir = project_dir();
    let engine = engine(dir.path(), &["--headless"]).await;
    let mut client = engine.client("cli").await;
    // 17 MiB, written a MiB at a time.
    let piece = vec![b'x'; 1024 * 1024];
    for _ in 0..17 {
        client.send_raw(&piece).await;
    }
    client.send_raw(b"\n").await;
    let answer = client.message().await;
    assert_eq!(answer["id"], Value::Null);
    assert_eq!(answer["error"]["code"], -32600);
    assert_eq!(answer["error"]["message"], "request line exceeds 16 MiB");
    // The rest of the long line was let go, and the next one is read.
    assert_eq!(project(&mut client).await["revision"], 0);
    // A request just under the most is read whole.
    let path = "p".repeat(16 * 1024 * 1024 - 1024);
    let ops = json!({ "ops": add_media(&path) });
    let request =
        json!({ "jsonrpc": "2.0", "id": 5, "method": "transaction.apply", "params": ops });
    let line = serde_json::to_vec(&request).unwrap();
    assert!(line.len() < 16 * 1024 * 1024);
    client.send(&request).await;
    let notification = client.message().await;
    assert_eq!(notification["method"], "project.changed");
    let answer = client.message().await;
    assert_eq!(answer["result"]["revision"], 1);
}

#[tokio::test]
async fn every_client_is_told_of_a_change_and_the_one_that_made_it_first() {
    let dir = project_dir();
    let engine = engine(dir.path(), &["--headless"]).await;
    let mut desktop = engine.client("desktop").await;
    let mut agent = engine.client("agent").await;

    let ops = json!([
        { "op": "add_media", "media": media("$m", "C:/media/a.mp4") },
        { "op": "add_timeline", "timeline": {
            "id": "$t", "name": "Cut", "rate": { "num": 25, "den": 1 }, "tracks": [],
        } },
        { "op": "add_track", "timeline": "$t", "track": { "id": "$v", "kind": "video", "clips": [] } },
        { "op": "insert_clip", "timeline": "$t", "track": "$v", "clip": clip("$c", "$m", 0, 50) },
        { "op": "move_clip", "timeline": "$t", "id": "$c", "track": "$v", "start": 10 },
    ]);
    let applied = agent.apply(ops).await;
    assert_eq!(applied["revision"], 1);
    assert_eq!(applied["next_id"], 4);
    assert_eq!(
        applied["names"],
        json!({ "$m": "m1", "$t": "t2", "$v": "V1", "$c": "c3" })
    );
    let done = applied["done"].as_array().unwrap();
    assert_eq!(done.len(), 5);
    assert_eq!(
        done[4],
        json!({ "op": "move_clip", "timeline": "t2", "id": "c3", "track": "V1", "start": 10 })
    );

    // The agent saw its change before the answer, and the desktop sees the same.
    let seen = agent.take_notifications();
    assert_eq!(seen.len(), 1);
    let changed = &seen[0];
    assert_eq!(changed["jsonrpc"], "2.0");
    assert_eq!(changed["method"], "project.changed");
    assert!(changed.get("id").is_none());
    assert_eq!(
        changed["params"],
        json!({
            "revision": 1,
            "by": "agent",
            "made_by": "agent",
            "done": applied["done"],
            "next_id": 4,
        })
    );
    assert_eq!(&desktop.message().await, changed);

    let got = project(&mut desktop).await;
    assert_eq!(got["revision"], 1);
    assert_eq!(got["next_id"], 4);
    let timeline = &got["project"]["timelines"][0];
    assert_eq!(timeline["id"], "t2");
    assert_eq!(timeline["tracks"][0]["clips"][0]["id"], "c3");
    assert_eq!(timeline["tracks"][0]["clips"][0]["start"], 10);
    assert_eq!(desktop.take_notifications(), Vec::<Value>::new());
}

#[tokio::test]
async fn a_refused_transaction_changes_nothing() {
    let dir = project_dir();
    let engine = engine(dir.path(), &["--headless"]).await;
    let mut desktop = engine.client("desktop").await;
    let mut agent = engine.client("agent").await;

    let ops = json!([
        { "op": "add_media", "media": media("$m", "a.mp4") },
        { "op": "insert_clip", "timeline": "t9", "track": "V1", "clip": clip("", "$m", 0, 10) },
    ]);
    let error = agent
        .error("transaction.apply", Some(json!({ "ops": ops })))
        .await;
    assert_eq!(error["code"], -32001);
    assert_eq!(
        error["data"],
        json!({
            "index": 1,
            "reason": "missing",
            "what": "timeline",
            "id": "t9",
            "message": "operation 1 of the transaction was refused: there is no timeline t9",
        })
    );
    assert_eq!(error["message"], error["data"]["message"]);

    let error = agent
        .error("transaction.apply", Some(json!({ "ops": [] })))
        .await;
    assert_eq!(error["code"], -32001);
    assert_eq!(error["data"]["reason"], "empty");

    let ops = json!([{ "op": "remove_media", "id": "$nobody" }]);
    let error = agent
        .error("transaction.apply", Some(json!({ "ops": ops })))
        .await;
    assert_eq!(error["code"], -32602);
    assert_eq!(
        error["data"],
        json!({ "index": 0, "name": "$nobody", "reason": "unknown_name" })
    );
    assert!(error["message"].as_str().unwrap().contains("$nobody"));

    let got = project(&mut desktop).await;
    assert_eq!(got["revision"], 0);
    assert_eq!(got["project"]["media"], json!([]));
    assert_eq!(desktop.take_notifications(), Vec::<Value>::new());
    assert_eq!(agent.take_notifications(), Vec::<Value>::new());
}

#[tokio::test]
async fn undo_and_redo_are_told_to_everyone() {
    let dir = project_dir();
    let engine = engine(dir.path(), &["--headless"]).await;
    let mut desktop = engine.client("desktop").await;
    let mut agent = engine.client("agent").await;

    assert_eq!(
        agent.result("history.undo", None).await,
        json!({ "revision": 0, "done": null })
    );
    assert_eq!(
        agent.result("history.redo", Some(json!({}))).await,
        json!({ "revision": 0, "done": null })
    );

    desktop.apply(add_media("a.mp4")).await;
    desktop.take_notifications();
    assert_eq!(agent.message().await["params"]["revision"], 1);

    let undone = agent.result("history.undo", None).await;
    assert_eq!(
        undone,
        json!({ "revision": 2, "done": [{ "op": "remove_media", "id": "m1" }], "next_id": 2 })
    );
    let changed = agent.take_notifications();
    assert_eq!(changed.len(), 1);
    assert_eq!(
        changed[0]["params"],
        json!({
            "revision": 2, "by": "agent", "made_by": "desktop",
            "done": undone["done"], "next_id": 2,
        })
    );
    let seen = desktop.message().await;
    assert_eq!(seen["params"], changed[0]["params"]);

    let redone = desktop.result("history.redo", None).await;
    desktop.take_notifications();
    assert_eq!(redone["revision"], 3);
    assert_eq!(
        redone["done"],
        json!([{ "op": "add_media", "media": media("m1", "a.mp4"), "index": 0 }])
    );
    let seen = agent.message().await;
    assert_eq!(seen["params"]["by"], "desktop");
    assert_eq!(seen["params"]["made_by"], "desktop");
    assert_eq!(seen["params"]["revision"], 3);

    agent.result("history.undo", None).await;
    assert_eq!(
        agent.result("history.undo", None).await,
        json!({ "revision": 4, "done": null })
    );
    assert_eq!(project(&mut desktop).await["project"]["media"], json!([]));
    let seen = desktop.take_notifications();
    assert_eq!(seen.len(), 1, "nothing undone is no change: {seen:?}");
    assert_eq!(seen[0]["params"]["revision"], 4);
}

#[tokio::test]
async fn the_project_is_saved_after_a_change_and_when_the_engine_ends() {
    let dir = project_dir();
    let engine = engine(dir.path(), &["--headless", "--idle-seconds", "1"]).await;
    let mut client = engine.client("agent").await;
    client.apply(add_media("a.mp4")).await;
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(saved(dir.path()), project(&mut client).await["project"]);

    // A change soon after a save is saved within the next period, or as the engine ends.
    client.apply(add_media("b.mp4")).await;
    let last = project(&mut client).await["project"].clone();
    assert_eq!(last["media"].as_array().unwrap().len(), 2);
    drop(client);
    assert_eq!(engine.ended().await, 0);
    assert_eq!(saved(dir.path()), last);
    assert!(!dir.path().join("engine.lock").exists());

    // And the next engine opens it.
    let engine = self::engine(dir.path(), &["--headless", "--idle-seconds", "1"]).await;
    let mut client = engine.client("cli").await;
    assert_eq!(project(&mut client).await["project"], last);
}

#[tokio::test]
async fn a_save_that_fails_is_told_to_every_client() {
    let dir = project_dir();
    let engine = engine(dir.path(), &["--headless", "--idle-seconds", "1"]).await;
    let mut agent = engine.client("agent").await;
    let mut desktop = engine.client("desktop").await;
    // A directory where the project is written first keeps it from being saved.
    std::fs::create_dir(dir.path().join("project.json.tmp")).unwrap();
    agent.apply(add_media("a.mp4")).await;
    for client in [&mut agent, &mut desktop] {
        let error = loop {
            let message = client.message().await;
            if message["method"] == "engine.error" {
                break message;
            }
            assert_eq!(message["method"], "project.changed", "{message}");
        };
        assert_eq!(error["params"]["kind"], "save");
        let message = error["params"]["message"].as_str().unwrap();
        assert!(message.contains("project.json.tmp"), "{message}");
    }
    drop(agent);
    drop(desktop);
    // The last save fails too, and the engine says so as it ends.
    let stderr = Arc::clone(&engine.stderr);
    assert_eq!(engine.ended().await, 1);
    assert!(stderr.lock().unwrap().contains("save_failed"));
    assert!(!dir.path().join("engine.lock").exists());
    assert!(!dir.path().join("project.json").exists());
}

#[tokio::test]
async fn a_second_engine_of_a_project_ends_with_3_and_the_address_of_the_first() {
    let dir = project_dir();
    let first = engine(dir.path(), &["--headless"]).await;
    let (code, line) = refused(dir.path(), &["--headless"]).await;
    assert_eq!(code, 3);
    assert_eq!(
        line,
        json!({ "held": { "pid": first.pid, "address": first.address } })
    );
    // The first is not disturbed.
    let mut client = first.client("agent").await;
    assert_eq!(project(&mut client).await["revision"], 0);
}

#[tokio::test]
async fn a_project_that_cannot_be_read_ends_the_engine_with_2() {
    let dir = project_dir();
    std::fs::write(dir.path().join("project.json"), "not json").unwrap();
    let (code, line) = refused(dir.path(), &["--headless"]).await;
    assert_eq!(code, 2);
    let message = line["error"].as_str().unwrap();
    assert!(message.contains("project.json"), "{message}");
    assert!(!dir.path().join("engine.lock").exists());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("project.json")).unwrap(),
        "not json"
    );

    let (code, line) = refused(dir.path(), &["--headless", "--colour"]).await;
    assert_eq!(code, 2);
    assert!(line["error"].as_str().unwrap().contains("--colour"));
}

#[tokio::test]
async fn a_headless_engine_ends_once_no_one_has_been_connected_for_a_while() {
    let dir = project_dir();
    let engine = engine(dir.path(), &["--headless", "--idle-seconds", "1"]).await;
    let client = engine.client("agent").await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let left = Instant::now();
    drop(client);
    assert_eq!(engine.ended().await, 0);
    let after = left.elapsed();
    assert!(after >= Duration::from_millis(900), "{after:?}");
}

#[tokio::test]
async fn a_headless_engine_stays_if_someone_comes_back_in_time() {
    let dir = project_dir();
    let mut engine = engine(dir.path(), &["--headless", "--idle-seconds", "2"]).await;
    drop(engine.client("agent").await);
    tokio::time::sleep(Duration::from_secs(1)).await;
    // Comes back, goes, and comes back once more, within the two seconds.
    drop(engine.client("agent").await);
    let client = engine.client("desktop").await;
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert!(engine.running());
    drop(client);
    assert_eq!(engine.ended().await, 0);
}

#[tokio::test]
async fn an_engine_that_is_not_headless_ends_when_its_stdin_is_closed() {
    let dir = project_dir();
    let mut engine = engine(dir.path(), &[]).await;
    let mut client = engine.client("desktop").await;
    client.apply(add_media("a.mp4")).await;
    let last = project(&mut client).await["project"].clone();
    // No one connected is no reason to end.
    drop(client);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(engine.running());
    drop(engine.child.stdin.take());
    assert_eq!(engine.ended().await, 0);
    assert_eq!(saved(dir.path()), last);
    assert!(!dir.path().join("engine.lock").exists());
}

#[tokio::test]
async fn a_client_that_does_not_read_is_closed_and_the_others_get_everything() {
    let dir = project_dir();
    let engine = engine(dir.path(), &["--headless", "--queue-capacity", "8"]).await;
    let slow = engine.client("agent").await;
    let mut fast = engine.client("desktop").await;
    // Long changes, so that what the slow client does not read fills what the system holds for
    // it, and then its queue.
    let path = "p".repeat(32 * 1024);
    let count = 200;
    for revision in 1..=count {
        let applied = fast.apply(add_media(&path)).await;
        assert_eq!(applied["revision"], revision);
        let seen = fast.take_notifications();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0]["params"]["revision"], revision);
    }
    let rest = slow.rest().await;
    assert!(
        rest.len() < usize::try_from(count).unwrap(),
        "{}",
        rest.len()
    );
    for (at, message) in rest.iter().enumerate() {
        assert_eq!(message["params"]["revision"], at + 1);
    }
    let stderr = engine.stderr();
    assert!(stderr.contains("\"kicked\""), "{stderr}");
    assert!(stderr.contains("\"agent\""), "{stderr}");
    // The fast client goes on.
    assert_eq!(project(&mut fast).await["revision"], count);
}

/// Send `count` transactions without waiting for their answers, and read until all are
/// answered. Gives the notifications and the revisions of the answers, in the order in which
/// they came.
async fn burst(client: &mut Client, count: usize) -> (Vec<Value>, Vec<(usize, u64)>) {
    let mut requests = Vec::new();
    for id in 1..=count {
        let request = json!({
            "jsonrpc": "2.0", "id": id, "method": "transaction.apply",
            "params": { "ops": add_media(&format!("{id}.mp4")) },
        });
        requests.extend(serde_json::to_vec(&request).unwrap());
        requests.push(b'\n');
    }
    let Client { reader, writer, .. } = client;
    let writing = async {
        // A few lines at a time, so that the two clients' requests come in between each other.
        for piece in requests.chunks(4096) {
            writer.write_all(piece).await.unwrap();
            tokio::task::yield_now().await;
        }
    };
    let reading = async {
        let mut notifications = Vec::new();
        let mut answers = Vec::new();
        while answers.len() < count {
            let message = read_message(reader).await.unwrap();
            if let Some(id) = message.get("id") {
                let revision = message["result"]["revision"].as_u64().unwrap();
                answers.push((usize::try_from(id.as_u64().unwrap()).unwrap(), revision));
                // Its own change came before it.
                assert!(
                    notifications
                        .iter()
                        .any(|n: &Value| n["params"]["revision"] == revision),
                    "{message}"
                );
            } else {
                notifications.push(message);
            }
        }
        (notifications, answers)
    };
    let ((), read) = tokio::join!(writing, reading);
    read
}

#[tokio::test]
async fn two_clients_that_change_the_project_at_once_see_the_same_changes_in_order() {
    let dir = project_dir();
    let engine = engine(dir.path(), &["--headless"]).await;
    let mut desktop = engine.client("desktop").await;
    let mut agent = engine.client("agent").await;
    let count = 200;
    let ((mut desktop_seen, desktop_answers), (mut agent_seen, agent_answers)) =
        tokio::join!(burst(&mut desktop, count), burst(&mut agent, count));
    // Each read until its own answers came: the rest of the other's changes are still coming.
    while desktop_seen.len() < 2 * count {
        desktop_seen.push(desktop.message().await);
    }
    while agent_seen.len() < 2 * count {
        agent_seen.push(agent.message().await);
    }
    assert_eq!(desktop_seen, agent_seen);
    let total = u64::try_from(2 * count).unwrap();
    let revisions: Vec<u64> = desktop_seen
        .iter()
        .map(|n| n["params"]["revision"].as_u64().unwrap())
        .collect();
    assert_eq!(revisions, (1..=total).collect::<Vec<_>>());
    // Each client's requests were answered in the order in which it sent them.
    for answers in [&desktop_answers, &agent_answers] {
        let ids: Vec<usize> = answers.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, (1..=count).collect::<Vec<_>>());
        assert!(answers.windows(2).all(|pair| pair[0].1 < pair[1].1));
    }
    let by: Vec<&str> = desktop_seen
        .iter()
        .map(|n| n["params"]["by"].as_str().unwrap())
        .collect();
    assert_eq!(by.iter().filter(|by| **by == "agent").count(), count);
    assert_eq!(project(&mut agent).await["revision"], total);
}

/// A program that is not an engine of the project, at the name that the engine would listen at.
#[cfg(windows)]
fn squat(address: &str, project: String) -> JoinHandle<()> {
    use tokio::net::windows::named_pipe::ServerOptions;
    let server = ServerOptions::new()
        .first_pipe_instance(true)
        .create(address)
        .unwrap();
    tokio::spawn(async move {
        server.connect().await.unwrap();
        let (read, mut write) = tokio::io::split(server);
        let mut line = String::new();
        BufReader::new(read).read_line(&mut line).await.unwrap();
        let hello: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(hello["method"], "hello");
        let answer = json!({
            "jsonrpc": "2.0", "id": hello["id"],
            "result": { "engine": "0.1.0", "protocol": 1, "revision": 0, "project": project, "pid": 4242 },
        });
        let mut answer = serde_json::to_vec(&answer).unwrap();
        answer.push(b'\n');
        write.write_all(&answer).await.unwrap();
        tokio::time::sleep(Duration::from_secs(5)).await;
    })
}

#[cfg(windows)]
#[tokio::test]
async fn an_engine_whose_pipe_is_taken_asks_who_has_it() {
    let dir = project_dir();
    // Where the engine of this project listens.
    let first = engine(dir.path(), &["--headless", "--idle-seconds", "0"]).await;
    let address = first.address.clone();
    assert_eq!(first.ended().await, 0);

    // An engine of the same project that is still ending.
    let squatter = squat(&address, written(dir.path()));
    let (code, line) = refused(dir.path(), &["--headless"]).await;
    assert_eq!(code, 3);
    assert_eq!(line, json!({ "held": { "pid": 4242, "address": address } }));
    assert!(!dir.path().join("engine.lock").exists());
    squatter.abort();
    let _ = squatter.await;

    // Something else.
    let squatter = squat(&address, "elsewhere".into());
    let (code, line) = refused(dir.path(), &["--headless"]).await;
    assert_eq!(code, 2);
    assert!(line["error"].as_str().unwrap().contains(&address), "{line}");
    squatter.abort();
}

/// Start an engine on `dir` whose stderr is read or not, which must start.
async fn engine_with(dir: &Path, args: &[&str], stderr: Stderr) -> Engine {
    match start_with(dir, args, stderr).await {
        Started::Listening(engine) => *engine,
        Started::Ended { code, stderr } => panic!("the engine ended with {code}: {stderr}"),
    }
}

#[tokio::test]
async fn an_engine_whose_stderr_is_closed_still_ends_when_its_stdin_is_closed() {
    let dir = project_dir();
    let mut engine = engine_with(dir.path(), &[], Stderr::Closed).await;
    let mut client = engine.client("desktop").await;
    client.apply(add_media("a.mp4")).await;
    let last = project(&mut client).await["project"].clone();
    drop(client);
    drop(engine.child.stdin.take());
    assert_eq!(engine.ended().await, 0);
    assert_eq!(saved(dir.path()), last);
    assert!(!dir.path().join("engine.lock").exists());
}

#[tokio::test]
async fn a_headless_engine_whose_stderr_is_closed_still_ends_when_no_one_is_connected() {
    let dir = project_dir();
    let args = [
        "--headless",
        "--idle-seconds",
        "1",
        "--queue-capacity",
        "4",
        "--queue-bytes",
        "262144",
    ];
    let engine = engine_with(dir.path(), &args, Stderr::Closed).await;
    // Clients that do not read, each of which is closed and logged to no one.
    let mut slow = Vec::new();
    for _ in 0..5 {
        slow.push(engine.client("agent").await);
    }
    let mut client = engine.client("desktop").await;
    let path = "p".repeat(64 * 1024);
    for _ in 0..10 {
        client.apply(add_media(&path)).await;
        client.take_notifications();
    }
    for slow in slow {
        slow.rest().await;
    }
    drop(client);
    assert_eq!(engine.ended().await, 0);
    assert!(!dir.path().join("engine.lock").exists());
}

#[tokio::test]
async fn an_engine_whose_stderr_is_never_read_goes_on_however_much_it_logs() {
    let dir = project_dir();
    let args = [
        "--headless",
        "--idle-seconds",
        "1",
        "--queue-capacity",
        "4",
        "--queue-bytes",
        "262144",
    ];
    let engine = engine_with(dir.path(), &args, Stderr::Unread).await;
    // Clients that do not read, each of which is closed and logged: more than the pipe of
    // stderr holds.
    let mut slow = Vec::new();
    for _ in 0..600 {
        slow.push(engine.client("agent").await);
    }
    let mut fast = engine.client("desktop").await;
    let path = "p".repeat(64 * 1024);
    for _ in 0..20 {
        fast.apply(add_media(&path)).await;
        fast.take_notifications();
    }
    for client in slow {
        client.rest().await;
    }
    assert_eq!(project(&mut fast).await["revision"], 20);
    drop(fast);
    assert_eq!(engine.ended().await, 0);
}

#[tokio::test]
async fn an_engine_that_ends_lets_the_next_one_listen_at_once() {
    let dir = project_dir();
    let mut current = engine(dir.path(), &[]).await;
    for cycle in 0..30 {
        // A client is still connected as the engine ends.
        let client = current.client("desktop").await;
        drop(current.child.stdin.take());
        let next = loop {
            match start(dir.path(), &[]).await {
                Started::Listening(next) => break *next,
                // The last one still has the project open: try again.
                Started::Ended { code: 3, .. } => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Started::Ended { code, stderr } => {
                    panic!("cycle {cycle}: the engine ended with {code}: {stderr}")
                }
            }
        };
        assert_eq!(current.ended().await, 0);
        drop(client);
        current = next;
    }
}

#[tokio::test]
async fn a_name_of_a_track_is_named_only_with_its_timeline() {
    let dir = project_dir();
    let engine = engine(dir.path(), &["--headless"]).await;
    let mut agent = engine.client("agent").await;
    let timeline = |id: &str, track: &str| {
        json!({
            "id": id, "name": id, "rate": { "num": 25, "den": 1 },
            "tracks": [{ "id": track, "kind": "video", "clips": [] }],
        })
    };
    let ops = json!([
        { "op": "add_media", "media": media("$m", "a.mp4") },
        { "op": "add_timeline", "timeline": timeline("$one", "$v") },
        { "op": "add_timeline", "timeline": timeline("$two", "$w") },
        // $v is the V1 of $one, and $two has a V1 too.
        { "op": "insert_clip", "timeline": "$two", "track": "$v", "clip": clip("", "$m", 0, 10) },
    ]);
    let error = agent
        .error("transaction.apply", Some(json!({ "ops": ops })))
        .await;
    assert_eq!(error["code"], -32602);
    assert_eq!(
        error["data"],
        json!({ "index": 3, "name": "$v", "reason": "name_elsewhere" })
    );
    assert_eq!(project(&mut agent).await["revision"], 0);
}

#[tokio::test]
async fn clips_named_on_two_tracks_at_one_frame_are_told_apart() {
    let dir = project_dir();
    let engine = engine(dir.path(), &["--headless"]).await;
    let mut agent = engine.client("agent").await;
    let ops = json!([
        { "op": "add_media", "media": media("$m", "a.mp4") },
        { "op": "add_timeline", "timeline": {
            "id": "$t", "name": "Cut", "rate": { "num": 25, "den": 1 },
            "tracks": [
                { "id": "$pic", "kind": "video", "clips": [clip("$p2", "$m", 40, 10), clip("$p1", "$m", 0, 10)] },
                { "id": "$snd", "kind": "audio", "clips": [clip("$s1", "$m", 0, 20), clip("$s2", "$m", 40, 20)] },
            ],
        } },
        { "op": "move_clip", "timeline": "$t", "id": "$s1", "track": "$snd", "start": 100 },
        { "op": "remove_clip", "timeline": "$t", "id": "$p1" },
    ]);
    let applied = agent.apply(ops).await;
    // m1; t2, then V1 and its clips in the order of the list, then A1 and its clips.
    assert_eq!(
        applied["names"],
        json!({
            "$m": "m1", "$t": "t2", "$pic": "V1", "$p2": "c3", "$p1": "c4",
            "$snd": "A1", "$s1": "c5", "$s2": "c6",
        })
    );
    let got = project(&mut agent).await;
    let tracks = &got["project"]["timelines"][0]["tracks"];
    assert_eq!(tracks[0]["clips"], json!([clip("c3", "m1", 40, 10)]));
    assert_eq!(
        tracks[1]["clips"],
        json!([clip("c6", "m1", 40, 20), clip("c5", "m1", 100, 20)])
    );
}

#[tokio::test]
async fn an_id_that_is_too_long_or_not_a_string_or_a_whole_number_is_refused() {
    let dir = project_dir();
    let engine = engine(dir.path(), &["--headless"]).await;
    let mut client = engine.client("cli").await;
    for id in [json!("i".repeat(300)), json!(1.5), Value::Null, json!([1])] {
        client
            .send(&json!({ "jsonrpc": "2.0", "id": id, "method": "project.get" }))
            .await;
        let answer = client.message().await;
        assert_eq!(answer["id"], Value::Null, "{id}");
        assert_eq!(answer["error"]["code"], -32600, "{id}");
    }
    let id = "i".repeat(200);
    client
        .send(&json!({ "jsonrpc": "2.0", "id": id, "method": "project.get" }))
        .await;
    assert_eq!(client.message().await["id"], id.as_str());
}

#[tokio::test]
async fn a_client_whose_answers_wait_for_too_many_bytes_is_closed() {
    let dir = project_dir();
    let args = ["--headless", "--queue-bytes", "1048576"];
    let engine = engine(dir.path(), &args).await;
    let mut slow = engine.client("agent").await;
    slow.apply(add_media(&"p".repeat(256 * 1024))).await;
    slow.take_notifications();
    // A hundred requests for the whole project, sent at once and never read: far fewer messages
    // than a queue holds, and far more bytes.
    let mut requests = Vec::new();
    for id in 0..100 {
        let request = json!({ "jsonrpc": "2.0", "id": id, "method": "project.get" });
        requests.extend(serde_json::to_vec(&request).unwrap());
        requests.push(b'\n');
    }
    slow.send_raw(&requests).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let rest = slow.rest().await;
    assert!(rest.len() < 100, "{}", rest.len());
    let stderr = engine.stderr();
    assert!(stderr.contains("\"kicked\""), "{stderr}");
    let mut other = engine.client("desktop").await;
    assert_eq!(project(&mut other).await["revision"], 1);
}

#[tokio::test]
async fn files_are_taken_in_as_media_in_one_transaction() {
    let Some(made) = support::media(support::Sample::Fhd) else {
        return;
    };
    let dir = project_dir();
    let engine = engine(dir.path(), &["--headless"]).await;
    let mut agent = engine.client("agent").await;
    let mut desktop = engine.client("desktop").await;
    let path = made.path.to_str().unwrap();
    // The same file twice is two media, as in an editor.
    let result = agent
        .result("media.import", Some(json!({ "paths": [path, path] })))
        .await;
    assert_eq!(
        result,
        json!({ "revision": 1, "added": ["m1", "m2"], "next_id": 3 })
    );
    let changed = agent.take_notifications();
    assert_eq!(changed.len(), 1, "{changed:?}");
    assert_eq!(changed[0]["method"], "project.changed");
    assert_eq!(changed[0]["params"]["by"], "agent");
    let done = changed[0]["params"]["done"].as_array().unwrap();
    assert_eq!(done.len(), 2);
    assert_eq!(done[0]["op"], "add_media");
    assert_eq!(done[0]["media"]["id"], "m1");
    assert_eq!(desktop.message().await, changed[0]);

    let got = project(&mut desktop).await;
    let media = &got["project"]["media"][0];
    let full = std::path::absolute(&made.path).unwrap();
    assert_eq!(media["path"], full.to_str().unwrap());
    let size = std::fs::metadata(&made.path).unwrap().len();
    assert_eq!(media["identity"]["size"], size);
    assert_eq!(media["identity"]["digest"].as_str().unwrap().len(), 64);
    let facts = &media["facts"];
    assert_eq!(facts["video"]["width"], 1920);
    assert_eq!(facts["video"]["codec"], "h264");
    assert_eq!(facts["video"]["rate"], json!({ "num": 25, "den": 1 }));
    assert_eq!(
        facts["audio"],
        json!({ "rate": 48000, "channels": 2, "codec": "aac" })
    );
    let duration = &facts["duration"];
    assert_eq!(
        duration["ticks"].as_i64().unwrap(),
        4 * duration["scale"].as_i64().unwrap()
    );
    assert_eq!(got["project"]["media"][1]["identity"], media["identity"]);

    // A clip is cut from it in the scale of its duration.
    let clip = json!([{ "op": "add_timeline", "timeline": {
        "id": "", "name": "Cut", "rate": { "num": 25, "den": 1 },
        "tracks": [{ "id": "", "kind": "video", "clips": [{
            "id": "", "media": "m1", "source_in": { "ticks": 0, "scale": duration["scale"] },
            "start": 0, "length": 100,
        }] }],
    } }]);
    agent.apply(clip).await;
}

#[tokio::test]
async fn files_that_cannot_be_read_are_none_of_them_taken_in() {
    let Some(made) = support::media(support::Sample::Wav) else {
        return;
    };
    let dir = project_dir();
    let engine = engine(dir.path(), &["--headless"]).await;
    let mut agent = engine.client("agent").await;
    let good = made.path.to_str().unwrap();
    let gone = dir.path().join("gone.mp4");
    let gone = gone.to_str().unwrap();
    let text = dir.path().join("notes.txt");
    std::fs::write(
        &text,
        "These are notes, and no picture or sound.
",
    )
    .unwrap();
    let text = text.to_str().unwrap();
    for (paths, path, reason) in [
        (json!([good, gone, text]), gone, "not_found"),
        (json!([good, text, gone]), text, "not_media"),
        (
            json!([dir.path().to_str().unwrap()]),
            dir.path().to_str().unwrap(),
            "unreadable",
        ),
    ] {
        let error = agent
            .error("media.import", Some(json!({ "paths": paths })))
            .await;
        assert_eq!(error["code"], -32002, "{error}");
        assert_eq!(error["data"]["path"], path, "{error}");
        assert_eq!(error["data"]["reason"], reason, "{error}");
        assert!(error["message"].as_str().unwrap().contains(path), "{error}");
    }
    let got = project(&mut agent).await;
    assert_eq!(got["revision"], 0);
    assert_eq!(got["project"]["media"], json!([]));
    assert_eq!(agent.take_notifications(), Vec::<Value>::new());

    for params in [json!({ "paths": [] }), json!({ "paths": "a" }), json!({})] {
        let error = agent.error("media.import", Some(params)).await;
        assert_eq!(error["code"], -32602, "{error}");
    }
    // A path that is not absolute means nothing to the engine.
    for relative in ["a.mp4", "media/a.mp4"] {
        let error = agent
            .error("media.import", Some(json!({ "paths": [good, relative] })))
            .await;
        assert_eq!(error["code"], -32602, "{error}");
        assert_eq!(
            error["data"],
            json!({ "index": 1, "path": relative, "reason": "relative" })
        );
    }
    // The sound alone is taken in.
    let result = agent
        .result("media.import", Some(json!({ "paths": [good] })))
        .await;
    assert_eq!(result["added"], json!(["m1"]));
}

#[tokio::test]
async fn files_are_read_while_the_engine_goes_on_answering() {
    let Some(made) = support::media(support::Sample::Fhd) else {
        return;
    };
    let dir = project_dir();
    // The import waits before it reads each file until the gate is there.
    let gate = dir.path().join("gate");
    let engine = engine(
        dir.path(),
        &["--headless", "--read-gate", gate.to_str().unwrap()],
    )
    .await;
    let mut agent = engine.client("agent").await;
    let mut desktop = engine.client("desktop").await;
    let path = made.path.to_str().unwrap();
    let importing = agent
        .request("media.import", Some(json!({ "paths": [path, path] })))
        .await;
    // While the import is held, another client's transaction is applied and answered.
    let timeline = json!([{ "op": "add_timeline", "timeline": {
        "id": "", "name": "Cut", "rate": { "num": 25, "den": 1 }, "tracks": [],
    } }]);
    let applied = desktop.apply(timeline).await;
    assert_eq!(applied["revision"], 1);
    assert_eq!(project(&mut desktop).await["project"]["media"], json!([]));
    std::fs::write(&gate, b"").unwrap();
    let imported = agent.answer(importing).await;
    assert_eq!(
        imported["result"],
        json!({ "revision": 2, "added": ["m2", "m3"], "next_id": 4 }),
        "{imported}"
    );
}

#[tokio::test]
async fn a_headless_engine_has_no_view() {
    let dir = project_dir();
    let engine = engine(dir.path(), &["--headless"]).await;
    let mut desktop = engine.client("desktop").await;
    for method in ["view.attach", "view.place", "view.show", "view.detach"] {
        let error = desktop.error(method, None).await;
        assert_eq!(error["code"], -32601, "{method}: {error}");
        assert!(
            error["message"].as_str().unwrap().contains("headless"),
            "{error}"
        );
    }
}

#[tokio::test]
async fn the_view_is_the_desktops_and_its_params_are_checked() {
    let dir = project_dir();
    let engine = engine(dir.path(), &[]).await;
    let mut agent = engine.client("agent").await;
    let rect = json!({ "parent": "1", "x": 0, "y": 0, "width": 64, "height": 36 });
    let error = agent.error("view.attach", Some(rect)).await;
    assert_eq!(error["code"], -32600, "{error}");

    let mut desktop = engine.client("desktop").await;
    let error = desktop
        .error(
            "view.place",
            Some(json!({ "x": 0, "y": 0, "width": 64, "height": 36 })),
        )
        .await;
    assert_eq!(error["code"], -32003, "{error}");
    assert_eq!(error["data"]["reason"], "no_view");
    let error = desktop
        .error("view.show", Some(json!({ "shown": true })))
        .await;
    assert_eq!(error["data"]["reason"], "no_view", "{error}");
    let detached = desktop.result("view.detach", None).await;
    assert_eq!(detached, json!({ "detached": false }));
    for parent in ["", "abc", "0", "1.5"] {
        let params = json!({ "parent": parent, "x": 0, "y": 0, "width": 64, "height": 36 });
        let error = desktop.error("view.attach", Some(params)).await;
        assert_eq!(error["code"], -32602, "{parent}: {error}");
        assert_eq!(error["data"]["reason"], "parent", "{parent}: {error}");
    }
    for params in [
        json!({ "parent": 1, "x": 0, "y": 0, "width": 64, "height": 36 }),
        json!({ "parent": "1", "x": 0, "y": 0, "width": -1, "height": 36 }),
        json!({ "parent": "1", "x": 0, "y": 0, "width": 64 }),
        json!({ "parent": "1", "x": 0, "y": 0, "width": 64, "height": 36, "z": 1 }),
    ] {
        let error = desktop.error("view.attach", Some(params.clone())).await;
        assert_eq!(error["code"], -32602, "{params}: {error}");
    }
    let error = desktop
        .error("view.show", Some(json!({ "shown": "yes" })))
        .await;
    assert_eq!(error["code"], -32602, "{error}");
    let error = desktop.error("view.detach", Some(json!({ "x": 1 }))).await;
    assert_eq!(error["code"], -32602, "{error}");
    if cfg!(windows) {
        // A handle that is no window.
        let params = json!({ "parent": "2147418113", "x": 0, "y": 0, "width": 64, "height": 36 });
        let error = desktop.error("view.attach", Some(params)).await;
        assert_eq!(error["code"], -32602, "{error}");
        assert_eq!(error["data"]["reason"], "parent", "{error}");
        let params = json!({ "parent": "1", "x": 0, "y": 0, "width": 20000, "height": 36 });
        let error = desktop.error("view.attach", Some(params)).await;
        assert_eq!(error["data"]["reason"], "rect", "{error}");
    }
}

/// A window of the test's own, hidden, that the engine's view is made in, as it is made in the
/// desktop's: in another process.
#[cfg(windows)]
#[allow(unsafe_code)]
mod parent {
    use std::sync::mpsc;
    use std::thread::JoinHandle;

    use windows::core::w;
    use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
        ScreenToClient, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, FindWindowExW, GetClientRect,
        GetMessageW, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId, PostMessageW,
        RegisterClassExW, TranslateMessage, GWL_STYLE, MSG, WINDOW_EX_STYLE, WM_CLOSE, WNDCLASSEXW,
        WS_CLIPCHILDREN, WS_EX_TOOLWINDOW, WS_POPUP, WS_VISIBLE,
    };

    pub struct Parent {
        pub hwnd: isize,
        thread: Option<JoinHandle<()>>,
    }

    fn hwnd(value: isize) -> HWND {
        HWND(value as *mut core::ffi::c_void)
    }

    unsafe extern "system" fn procedure(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> windows::Win32::Foundation::LRESULT {
        if message == windows::Win32::UI::WindowsAndMessaging::WM_DESTROY {
            unsafe { windows::Win32::UI::WindowsAndMessaging::PostQuitMessage(0) };
        }
        unsafe { DefWindowProcW(window, message, wparam, lparam) }
    }

    impl Parent {
        /// A top-level window that is not shown, served by a thread of its own.
        pub fn open() -> Parent {
            Parent::made(false)
        }

        /// A top-level window that is shown, of 960 × 540 at the top left of the screen, as a
        /// tool window, so that what the view draws in it can be read back.
        pub fn shown() -> Parent {
            Parent::made(true)
        }

        fn made(shown: bool) -> Parent {
            let (made, making) = mpsc::channel();
            let thread = std::thread::spawn(move || unsafe {
                let class = WNDCLASSEXW {
                    cbSize: u32::try_from(std::mem::size_of::<WNDCLASSEXW>()).unwrap(),
                    lpfnWndProc: Some(procedure),
                    lpszClassName: w!("ColumbinaTestDesktop"),
                    ..WNDCLASSEXW::default()
                };
                RegisterClassExW(&raw const class);
                let (extended, style, width, height) = if shown {
                    (
                        WS_EX_TOOLWINDOW,
                        WS_POPUP | WS_VISIBLE | WS_CLIPCHILDREN,
                        960,
                        540,
                    )
                } else {
                    (WINDOW_EX_STYLE(0), WS_POPUP, 800, 600)
                };
                let window = CreateWindowExW(
                    extended,
                    w!("ColumbinaTestDesktop"),
                    w!("columbina-engine test"),
                    style,
                    0,
                    0,
                    width,
                    height,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap();
                made.send(window.0 as isize).unwrap();
                let mut message = MSG::default();
                while GetMessageW(&raw mut message, None, 0, 0).0 > 0 {
                    let _ = TranslateMessage(&raw const message);
                    DispatchMessageW(&raw const message);
                }
            });
            Parent {
                hwnd: making.recv().unwrap(),
                thread: Some(thread),
            }
        }

        /// The views in it: their handles, the processes they belong to, their rectangles in
        /// its client area, and whether they are shown.
        pub fn views(&self) -> Vec<(isize, u32, [i32; 4], bool)> {
            let mut views = Vec::new();
            let mut after = None;
            loop {
                let found = unsafe {
                    FindWindowExW(Some(hwnd(self.hwnd)), after, w!("ColumbinaView"), None)
                };
                let Ok(found) = found else { break };
                let mut process = 0;
                unsafe { GetWindowThreadProcessId(found, Some(&raw mut process)) };
                let mut rect = RECT::default();
                unsafe { GetWindowRect(found, &raw mut rect) }.unwrap();
                let mut corners = [
                    POINT {
                        x: rect.left,
                        y: rect.top,
                    },
                    POINT {
                        x: rect.right,
                        y: rect.bottom,
                    },
                ];
                for corner in &mut corners {
                    let _ = unsafe { ScreenToClient(hwnd(self.hwnd), corner) };
                }
                let style = unsafe { GetWindowLongW(found, GWL_STYLE) }.cast_unsigned();
                views.push((
                    found.0 as isize,
                    process,
                    [
                        corners[0].x,
                        corners[0].y,
                        corners[1].x - corners[0].x,
                        corners[1].y - corners[0].y,
                    ],
                    style & WS_VISIBLE.0 != 0,
                ));
                after = Some(found);
            }
            views
        }
    }

    impl Parent {
        /// Its client area as it is shown, the drawing of the engine's view in it included,
        /// wherever it is (`PrintWindow` with `PW_CLIENTONLY | PW_RENDERFULLCONTENT`, which works
        /// on a locked desktop too): the luma of each pixel, of its grey, a row after another,
        /// and its width.
        pub fn grey(&self) -> (Vec<u8>, usize) {
            let mut client = RECT::default();
            unsafe { GetClientRect(hwnd(self.hwnd), &raw mut client) }.unwrap();
            let (width, height) = (client.right, client.bottom);
            unsafe {
                let screen = GetDC(None);
                let memory = CreateCompatibleDC(Some(screen));
                let info = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: u32::try_from(std::mem::size_of::<BITMAPINFOHEADER>()).unwrap(),
                        biWidth: width,
                        biHeight: -height,
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB.0,
                        ..BITMAPINFOHEADER::default()
                    },
                    ..BITMAPINFO::default()
                };
                let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
                let bitmap = CreateDIBSection(
                    Some(memory),
                    &raw const info,
                    DIB_RGB_COLORS,
                    &raw mut bits,
                    None,
                    0,
                )
                .unwrap();
                let old = SelectObject(memory, bitmap.into());
                let printed = PrintWindow(hwnd(self.hwnd), memory, PRINT_WINDOW_FLAGS(1 | 2));
                let length = usize::try_from(width * height * 4).unwrap();
                let pixels = std::slice::from_raw_parts(bits.cast::<u8>(), length).to_vec();
                SelectObject(memory, old);
                let _ = DeleteObject(bitmap.into());
                let _ = DeleteDC(memory);
                ReleaseDC(None, screen);
                assert!(printed.as_bool(), "PrintWindow failed");
                // BGRA: the green of a grey is its grey.
                let grey = pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|pixel| pixel[1])
                    .collect();
                (grey, usize::try_from(width).unwrap())
            }
        }
    }

    impl Drop for Parent {
        fn drop(&mut self) {
            let _ = unsafe { PostMessageW(Some(hwnd(self.hwnd)), WM_CLOSE, WPARAM(0), LPARAM(0)) };
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
}

/// Wait until the views in a parent are as `wanted` says.
#[cfg(windows)]
async fn until_views(
    parent: &parent::Parent,
    what: &str,
    wanted: impl Fn(&[(isize, u32, [i32; 4], bool)]) -> bool,
) -> Vec<(isize, u32, [i32; 4], bool)> {
    let deadline = Instant::now() + WAIT;
    loop {
        let views = parent.views();
        if wanted(&views) {
            return views;
        }
        assert!(Instant::now() < deadline, "{what}: {views:?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[cfg(windows)]
#[tokio::test]
async fn the_desktop_attaches_one_view_to_its_window_and_moves_shows_and_detaches_it() {
    let dir = project_dir();
    let engine = engine(dir.path(), &[]).await;
    let parent = parent::Parent::open();
    let mut desktop = engine.client("desktop").await;
    let rect = |x: i32, y: i32, width: u32, height: u32| json!({ "x": x, "y": y, "width": width, "height": height });
    let mut attach = rect(10, 20, 320, 180);
    attach["parent"] = json!(parent.hwnd.to_string());
    let attached = desktop.result("view.attach", Some(attach.clone())).await;
    assert!(attached["adapter"].as_str().is_some(), "{attached}");
    assert!(attached["backend"].as_str().is_some(), "{attached}");
    // A window of the engine's, in the test's window.
    let views = until_views(&parent, "attached", |views| views.len() == 1).await;
    let (first, process, place, shown) = views[0];
    assert_eq!(process, engine.pid);
    assert_eq!(place, [10, 20, 320, 180]);
    assert!(shown);

    assert_eq!(
        desktop
            .result("view.place", Some(rect(-5, 40, 200, 100)))
            .await,
        json!({})
    );
    until_views(&parent, "moved", |views| {
        views.len() == 1 && views[0].2 == [-5, 40, 200, 100]
    })
    .await;
    desktop
        .result("view.show", Some(json!({ "shown": false })))
        .await;
    until_views(&parent, "hidden", |views| views.len() == 1 && !views[0].3).await;
    desktop
        .result("view.show", Some(json!({ "shown": true })))
        .await;
    until_views(&parent, "shown", |views| views.len() == 1 && views[0].3).await;

    // A second attach takes the place of the first.
    let mut again = rect(0, 0, 64, 36);
    again["parent"] = json!(parent.hwnd.to_string());
    desktop.result("view.attach", Some(again)).await;
    until_views(&parent, "attached again", |views| {
        views.len() == 1 && views[0].0 != first && views[0].2 == [0, 0, 64, 36]
    })
    .await;

    // The view is the engine's: it stays when the desktop's connection closes.
    drop(desktop);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(parent.views().len(), 1);
    let mut desktop = engine.client("desktop").await;
    let detached = desktop.result("view.detach", None).await;
    assert_eq!(detached, json!({ "detached": true }));
    assert_eq!(parent.views(), []);
    // The log is written by a thread of its own, and read here by a task: it comes soon.
    let deadline = Instant::now() + WAIT;
    while !engine.stderr().contains(r#""event":"view_detached""#) {
        assert!(Instant::now() < deadline, "{}", engine.stderr());
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let stderr = engine.stderr();
    assert!(stderr.contains(r#""event":"view_attached""#), "{stderr}");
    assert!(stderr.contains(r#""event":"view_ready""#), "{stderr}");
}

#[cfg(windows)]
#[tokio::test]
async fn a_view_whose_window_is_closed_is_gone_and_then_there_is_none() {
    let dir = project_dir();
    let engine = engine(dir.path(), &[]).await;
    let parent = parent::Parent::open();
    let mut desktop = engine.client("desktop").await;
    let place = json!({ "x": 0, "y": 0, "width": 64, "height": 36 });
    let mut attach = place.clone();
    attach["parent"] = json!(parent.hwnd.to_string());
    desktop.result("view.attach", Some(attach)).await;
    // The desktop's window is closed, and the view's window with it.
    drop(parent);
    let deadline = Instant::now() + WAIT;
    let error = loop {
        let answer = desktop.call("view.place", Some(place.clone())).await;
        if let Some(error) = answer.get("error") {
            break error.clone();
        }
        assert!(Instant::now() < deadline, "the view still has its window");
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(error["code"], -32003, "{error}");
    assert_eq!(error["data"]["reason"], "gone", "{error}");
    // The view was let go.
    let error = desktop.error("view.place", Some(place)).await;
    assert_eq!(error["data"]["reason"], "no_view", "{error}");
    let error = desktop
        .error("view.show", Some(json!({ "shown": true })))
        .await;
    assert_eq!(error["data"]["reason"], "no_view", "{error}");
    let detached = desktop.result("view.detach", None).await;
    assert_eq!(detached, json!({ "detached": false }));
}

// The transport.

impl Client {
    /// The next `transport.state` that `wanted` takes, of those that came or come, and when it
    /// was read; the others before it are let go.
    async fn until_state(
        &mut self,
        what: &str,
        wanted: impl Fn(&Value) -> bool,
    ) -> (Value, Instant) {
        let deadline = Instant::now() + WAIT;
        let mut seen = Vec::new();
        loop {
            let message = if self.notifications.is_empty() {
                assert!(Instant::now() < deadline, "{what}: {seen:?}");
                self.message().await
            } else {
                self.notifications.remove(0)
            };
            if message["method"] == "transport.state" {
                let state = message["params"].clone();
                if wanted(&state) {
                    return (state, Instant::now());
                }
                seen.push(state);
            }
        }
    }

    /// Each `transport.state` that comes within `time`, with when it was read.
    async fn states_for(&mut self, time: Duration) -> Vec<(Value, Instant)> {
        let end = Instant::now() + time;
        let mut states = Vec::new();
        for message in std::mem::take(&mut self.notifications) {
            if message["method"] == "transport.state" {
                states.push((message["params"].clone(), Instant::now()));
            }
        }
        while let Ok(message) =
            tokio::time::timeout_at(end.into(), read_message(&mut self.reader)).await
        {
            let message = message.expect("the engine closed the connection");
            if message["method"] == "transport.state" {
                states.push((message["params"].clone(), Instant::now()));
            }
        }
        states
    }
}

/// An engine that is not headless (so that it has a transport) and plays its sound muted, and
/// a client of it, with `samples` taken in as media `m1`, `m2`, … None where they cannot be
/// made.
async fn transport_engine(dir: &Path, samples: &[support::Sample]) -> Option<(Engine, Client)> {
    let mut paths = Vec::new();
    for &sample in samples {
        paths.push(support::media(sample)?.path.to_str().unwrap().to_owned());
    }
    let engine = engine(dir, &["--mute"]).await;
    let mut agent = engine.client("agent").await;
    agent
        .result("media.import", Some(json!({ "paths": paths })))
        .await;
    agent.take_notifications();
    Some((engine, agent))
}

/// How many frames a second the playhead moves at, read from what is told over `time`, once
/// it is past `from`.
async fn frames_a_second(client: &mut Client, from: i64, time: Duration) -> f64 {
    let (first, at) = client
        .until_state("playing", |state| state["frame"].as_i64().unwrap() >= from)
        .await;
    let states = client.states_for(time).await;
    let (last, then) = states.last().expect("states while playing");
    assert_eq!(last["playing"], true, "{states:?}");
    let frames = last["frame"].as_i64().unwrap() - first["frame"].as_i64().unwrap();
    #[allow(clippy::cast_precision_loss)]
    let rate = frames as f64 / then.duration_since(at).as_secs_f64();
    rate
}

#[tokio::test]
async fn a_source_plays_pauses_seeks_and_steps() {
    let dir = project_dir();
    let Some((engine, mut agent)) = transport_engine(dir.path(), &[support::Sample::Fhd]).await
    else {
        return;
    };
    let loaded = agent
        .result("transport.load", Some(json!({ "media": "m1" })))
        .await;
    assert_eq!(loaded["media"], "m1", "{loaded}");
    assert_eq!(loaded["frame"], 0);
    assert_eq!(loaded["shown"], 0);
    assert_eq!(loaded["playing"], false);
    assert_eq!(loaded["exact"], true);
    assert_eq!(loaded["last"], 99);
    // The state was told before the answer, as a change is.
    let told = agent.take_notifications();
    assert_eq!(
        told.last().unwrap()["method"],
        "transport.state",
        "{told:?}"
    );
    assert_eq!(told.last().unwrap()["params"], loaded);

    // At speed 1, 25 frames a second, by the sound where there is a device to play it.
    agent
        .result("transport.play", Some(json!({ "speed": 1 })))
        .await;
    let rate = frames_a_second(&mut agent, 5, Duration::from_millis(1200)).await;
    let state = agent.result("transport.state", None).await;
    match state["sound"].as_str().unwrap() {
        "on" => eprintln!("played with sound: the device is the clock"),
        "unavailable" => eprintln!("skipped the sound: there is no device to play it on"),
        other => panic!("the sound is {other} while it plays at speed 1"),
    }
    assert!((22.5..=27.5).contains(&rate), "{rate} frames a second");

    // Paused, it stays.
    agent.result("transport.pause", None).await;
    let (paused, _) = agent
        .until_state("paused", |state| state["playing"] == false)
        .await;
    assert_eq!(paused["exact"], true);
    assert_eq!(
        paused["sound"],
        if state["sound"] == "on" {
            "off"
        } else {
            "unavailable"
        }
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    let state = agent.result("transport.state", None).await;
    assert_eq!(state["frame"], paused["frame"]);
    assert_eq!(state["playing"], false);

    // A seek shows the frame sought.
    agent
        .result("transport.seek", Some(json!({ "frame": 37 })))
        .await;
    let (sought, _) = agent
        .until_state("sought", |state| state["frame"] == 37)
        .await;
    assert_eq!(
        (&sought["exact"], &sought["shown"]),
        (&json!(true), &json!(37))
    );
    // Beyond the media, the nearest end.
    agent
        .result("transport.seek", Some(json!({ "frame": 500 })))
        .await;
    agent
        .until_state("the end", |state| {
            state["frame"] == 99 && state["shown"] == 99
        })
        .await;
    agent
        .result("transport.seek", Some(json!({ "frame": -5 })))
        .await;
    agent
        .until_state("the start", |state| {
            state["frame"] == 0 && state["shown"] == 0
        })
        .await;

    // Steps of one and of ten, within the media.
    agent
        .result("transport.seek", Some(json!({ "frame": 40 })))
        .await;
    agent.until_state("40", |state| state["shown"] == 40).await;
    for (frames, wanted) in [
        (1, 41),
        (-1, 40),
        (10, 50),
        (-10, 40),
        (-1, 39),
        (100, 99),
        (-1000, 0),
    ] {
        agent
            .result("transport.step", Some(json!({ "frames": frames })))
            .await;
        let (stepped, _) = agent
            .until_state("stepped", |state| state["frame"] == wanted)
            .await;
        assert_eq!(stepped["shown"], wanted, "{stepped}");
        assert_eq!(stepped["exact"], true);
        assert_eq!(stepped["playing"], false);
    }
    drop(engine);
}

#[tokio::test]
async fn a_scrub_shows_the_key_frame_first_and_the_frame_once_it_settles() {
    let dir = project_dir();
    let Some((_engine, mut agent)) = transport_engine(dir.path(), &[support::Sample::Fhd]).await
    else {
        return;
    };
    agent
        .result("transport.load", Some(json!({ "media": "m1" })))
        .await;
    agent.take_notifications();
    // 20 seeks within half a second, as a page sends them while the playhead is dragged. Key
    // frames are at 0 and 50.
    let started = Instant::now();
    let mut standing_in = Vec::new();
    for index in 0..20 {
        let frame = 5 * index + 3;
        agent
            .result(
                "transport.seek",
                Some(json!({ "frame": frame, "scrubbing": true })),
            )
            .await;
        for message in agent.take_notifications() {
            if message["params"]["exact"] == false {
                standing_in.push(message["params"].clone());
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    // Only the last frame sought is decoded, once no newer seek came.
    let states = agent.states_for(Duration::from_secs(2)).await;
    let (settled, _) = states.last().expect("the frame settles");
    assert_eq!(
        (&settled["frame"], &settled["shown"], &settled["exact"]),
        (&json!(98), &json!(98), &json!(true)),
        "{states:?}"
    );
    standing_in.extend(
        states
            .iter()
            .map(|(state, _)| state.clone())
            .filter(|state| state["exact"] == false),
    );
    assert!(
        standing_in.iter().any(|state| state["frame"] == 98),
        "the key frame stood in for the last frame sought: {standing_in:?}"
    );
    assert!(!standing_in.is_empty(), "no key frame stood in");
    for state in &standing_in {
        let shown = state["shown"].as_i64().unwrap();
        let frame = state["frame"].as_i64().unwrap();
        assert!([0, 50].contains(&shown), "{state}");
        assert!(shown <= frame && frame - shown < 50, "{state}");
    }
}

#[tokio::test]
async fn backward_from_the_middle_it_plays_to_frame_0_and_stops() {
    let dir = project_dir();
    let Some((_engine, mut agent)) = transport_engine(dir.path(), &[support::Sample::Fhd]).await
    else {
        return;
    };
    agent
        .result("transport.load", Some(json!({ "media": "m1" })))
        .await;
    agent
        .result("transport.seek", Some(json!({ "frame": 50 })))
        .await;
    agent.until_state("50", |state| state["shown"] == 50).await;
    let started = Instant::now();
    agent
        .result("transport.play", Some(json!({ "speed": -1 })))
        .await;
    let (playing, _) = agent
        .until_state("playing backward", |state| state["playing"] == true)
        .await;
    assert_eq!(playing["speed"], -1);
    assert_ne!(playing["sound"], "on", "backward is silent");
    let (stopped, _) = agent
        .until_state("stopped", |state| state["playing"] == false)
        .await;
    let took = started.elapsed();
    assert_eq!(stopped["frame"], 0, "{stopped}");
    assert_eq!(stopped["shown"], 0);
    assert_eq!(stopped["exact"], true);
    // 50 frames at 25 a second.
    assert!(took >= Duration::from_millis(1800), "{took:?}");
    assert!(took < Duration::from_secs(6), "{took:?}");
}

#[tokio::test]
async fn at_speed_4_it_moves_four_times_as_fast_and_stops_at_the_end() {
    let dir = project_dir();
    let Some((_engine, mut agent)) = transport_engine(dir.path(), &[support::Sample::Fhd]).await
    else {
        return;
    };
    agent
        .result("transport.load", Some(json!({ "media": "m1" })))
        .await;
    agent
        .result("transport.play", Some(json!({ "speed": 4 })))
        .await;
    let rate = frames_a_second(&mut agent, 8, Duration::from_millis(500)).await;
    assert!((85.0..=115.0).contains(&rate), "{rate} frames a second");
    let (stopped, _) = agent
        .until_state("stopped", |state| state["playing"] == false)
        .await;
    assert_eq!(stopped["frame"], 99, "{stopped}");
    assert_eq!(stopped["shown"], 99);
    assert_eq!(stopped["speed"], 4);
    assert_ne!(stopped["sound"], "on");
    // Played from the end, it stays there.
    agent
        .result("transport.play", Some(json!({ "speed": 2 })))
        .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let state = agent.result("transport.state", None).await;
    assert_eq!(
        (&state["frame"], &state["playing"]),
        (&json!(99), &json!(false))
    );
}

#[tokio::test]
async fn another_media_is_loaded_and_media_that_is_removed_is_unloaded() {
    let dir = project_dir();
    let Some((engine, mut agent)) =
        transport_engine(dir.path(), &[support::Sample::Fhd, support::Sample::Fhd10]).await
    else {
        return;
    };
    agent
        .result("transport.load", Some(json!({ "media": "m1" })))
        .await;
    agent
        .result("transport.play", Some(json!({ "speed": 1 })))
        .await;
    agent
        .until_state("playing", |state| state["frame"].as_i64() > Some(3))
        .await;
    let loaded = agent
        .result("transport.load", Some(json!({ "media": "m2" })))
        .await;
    assert_eq!(loaded["media"], "m2");
    assert_eq!(loaded["frame"], 0);
    assert_eq!(loaded["playing"], false);
    assert_eq!(loaded["last"], 24);
    assert_eq!(loaded["sound"], "none");

    // Another client sees the media go.
    let mut desktop = engine.client("desktop").await;
    agent
        .apply(json!([{ "op": "remove_media", "id": "m2" }]))
        .await;
    for client in [&mut agent, &mut desktop] {
        let (unloaded, _) = client
            .until_state("unloaded", |state| state["media"].is_null())
            .await;
        assert_eq!(unloaded["frame"], Value::Null);
        assert_eq!(unloaded["playing"], false);
    }
    let error = agent
        .error("transport.play", Some(json!({ "speed": 1 })))
        .await;
    assert_eq!(error["code"], -32004, "{error}");
    assert_eq!(error["data"]["reason"], "not_loaded");
    // Removing media that is not loaded unloads nothing.
    agent
        .result("transport.load", Some(json!({ "media": "m1" })))
        .await;
    agent
        .apply(json!([{ "op": "add_media", "media": media("", "C:/x.mp4") }]))
        .await;
    agent
        .apply(json!([{ "op": "remove_media", "id": "m3" }]))
        .await;
    let state = agent.result("transport.state", None).await;
    assert_eq!(state["media"], "m1");
}

#[tokio::test]
async fn what_the_transport_cannot_do_is_refused() {
    let dir = project_dir();
    let Some((_engine, mut agent)) =
        transport_engine(dir.path(), &[support::Sample::Fhd, support::Sample::Wav]).await
    else {
        return;
    };
    for method in ["transport.pause", "transport.step"] {
        let params = (method == "transport.step").then(|| json!({ "frames": 1 }));
        let error = agent.error(method, params).await;
        assert_eq!(error["code"], -32004, "{method}: {error}");
        assert_eq!(error["data"]["reason"], "not_loaded");
    }
    let state = agent.result("transport.state", None).await;
    assert_eq!(state["media"], Value::Null);
    let error = agent
        .error("transport.load", Some(json!({ "media": "m9" })))
        .await;
    assert_eq!(error["code"], -32602, "{error}");
    assert_eq!(error["data"], json!({ "reason": "missing", "media": "m9" }));
    // Sound only: no picture to show (phase 0).
    let error = agent
        .error("transport.load", Some(json!({ "media": "m2" })))
        .await;
    assert_eq!(error["code"], -32002, "{error}");
    assert_eq!(error["data"]["reason"], "no_video");
    agent
        .result("transport.load", Some(json!({ "media": "m1" })))
        .await;
    for speed in [json!(0), json!(9), json!(-9), json!(1.5), json!("1")] {
        let error = agent
            .error("transport.play", Some(json!({ "speed": speed })))
            .await;
        assert_eq!(error["code"], -32602, "{speed}: {error}");
    }
    for (method, params) in [
        ("transport.seek", json!({})),
        ("transport.seek", json!({ "frame": "1" })),
        ("transport.step", json!({ "frames": 1, "x": 1 })),
        ("transport.state", json!({ "x": 1 })),
        ("transport.load", json!({})),
    ] {
        let error = agent.error(method, Some(params.clone())).await;
        assert_eq!(error["code"], -32602, "{method} {params}: {error}");
    }
    let error = agent.error("transport.rewind", None).await;
    assert_eq!(error["code"], -32601, "{error}");
}

#[tokio::test]
async fn a_headless_engine_has_no_transport() {
    let dir = project_dir();
    let engine = engine(dir.path(), &["--headless"]).await;
    let mut agent = engine.client("agent").await;
    for method in [
        "transport.load",
        "transport.play",
        "transport.pause",
        "transport.seek",
        "transport.step",
        "transport.state",
    ] {
        let error = agent.error(method, None).await;
        assert_eq!(error["code"], -32601, "{method}: {error}");
        assert!(
            error["message"].as_str().unwrap().contains("headless"),
            "{error}"
        );
    }
}

/// The number that a frame of the test media carries, as the view shows it: the picture of
/// 1920 × 1080 fitted into a view of 960 × 540 at the top left of the window, so that each
/// block of a digit is 32 pixels wide, and grey of `(16 + 14·d − 16) × 255 / 219`.
#[cfg(windows)]
fn number_shown(grey: &[u8], width: usize) -> Option<i64> {
    let mut number = 0;
    for block in 0..3 {
        let mut sum = 0_u32;
        for y in 8..24 {
            for x in (block * 32 + 8)..(block * 32 + 24) {
                sum += u32::from(grey[y * width + x]);
            }
        }
        let mean = f64::from(sum) / 256.0;
        let digit = mean / (14.0 * 255.0 / 219.0);
        if (digit - digit.round()).abs() > 0.3 || !(-0.3..=15.3).contains(&digit) {
            return None;
        }
        #[allow(clippy::cast_possible_truncation)]
        let digit = digit.round() as i64;
        number += digit << (4 * block);
    }
    Some(number)
}

#[cfg(windows)]
#[tokio::test]
async fn the_view_shows_the_frame_sought() {
    let dir = project_dir();
    let Some((engine, mut agent)) = transport_engine(dir.path(), &[support::Sample::Fhd]).await
    else {
        return;
    };
    let parent = parent::Parent::shown();
    let mut desktop = engine.client("desktop").await;
    let attach =
        json!({ "parent": parent.hwnd.to_string(), "x": 0, "y": 0, "width": 960, "height": 540 });
    desktop.result("view.attach", Some(attach)).await;
    agent
        .result("transport.load", Some(json!({ "media": "m1" })))
        .await;
    for (frame, scrubbing) in [(37, false), (77, true), (5, false)] {
        agent
            .result(
                "transport.seek",
                Some(json!({ "frame": frame, "scrubbing": scrubbing })),
            )
            .await;
        agent
            .until_state("sought", |state| {
                state["shown"] == frame && state["exact"] == true
            })
            .await;
        let deadline = Instant::now() + WAIT;
        loop {
            let (grey, width) = parent.grey();
            let shown = number_shown(&grey, width);
            if shown == Some(frame) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "frame {frame}: the view shows {shown:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

/// How the transport plays the 4K file and the 1080p file, and how fast it shows a frame
/// sought while scrubbing: run with `--ignored --nocapture`. `COLUMBINA_MEASURE_FILE` names
/// another file to play (one longer than 10 s, for the drift of the sound), and
/// `COLUMBINA_MEASURE_SOUND=1` plays the sound aloud.
#[tokio::test]
#[ignore = "a measurement, not a test"]
#[allow(clippy::too_many_lines)]
async fn measure_playing_and_scrubbing() {
    let (Some(fhd), Some(uhd)) = (
        support::media(support::Sample::Fhd),
        support::media(support::Sample::Uhd),
    ) else {
        return;
    };
    let mut files = vec![uhd.path.clone(), fhd.path.clone()];
    if let Ok(file) = std::env::var("COLUMBINA_MEASURE_FILE") {
        files.insert(0, std::path::PathBuf::from(file));
    }
    let dir = project_dir();
    let aloud = std::env::var("COLUMBINA_MEASURE_SOUND").as_deref() == Ok("1");
    let engine = engine(dir.path(), if aloud { &[] } else { &["--mute"] }).await;
    let mut agent = engine.client("agent").await;
    let paths: Vec<&str> = files.iter().map(|path| path.to_str().unwrap()).collect();
    let added = agent
        .result("media.import", Some(json!({ "paths": paths })))
        .await;
    for (media, path) in added["added"].as_array().unwrap().iter().zip(&files) {
        let loaded = agent
            .result("transport.load", Some(json!({ "media": media })))
            .await;
        let last = loaded["last"].as_i64().unwrap();
        eprintln!("{}: {loaded}", path.display());
        // Played to its end, or for 12 s.
        agent.take_notifications();
        agent
            .result("transport.play", Some(json!({ "speed": 1 })))
            .await;
        agent
            .until_state("playing", |state| state["playing"] == true)
            .await;
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            let states = agent.states_for(Duration::from_millis(100)).await;
            if states.iter().any(|(state, _)| state["playing"] == false)
                || Instant::now() > deadline
            {
                break;
            }
        }
        agent.result("transport.pause", None).await;
        // Scrubbing: a seek every 1/30 s, as a page sends them, then the time to the key frame
        // and to the frame itself, after a last seek.
        let mut keys = Vec::new();
        let mut exacts = Vec::new();
        for round in 0..6_i64 {
            for index in 0..10_i64 {
                let frame = (round * 37 + index * 7 + 11) % (last + 1);
                agent
                    .result(
                        "transport.seek",
                        Some(json!({ "frame": frame, "scrubbing": true })),
                    )
                    .await;
                tokio::time::sleep(Duration::from_millis(33)).await;
            }
            agent
                .until_state("settled", |state| state["exact"] == true)
                .await;
            agent.take_notifications();
            let target = (round * 53 + 97) % (last + 1);
            let sent = Instant::now();
            agent
                .result(
                    "transport.seek",
                    Some(json!({ "frame": target, "scrubbing": true })),
                )
                .await;
            let (first, at) = agent
                .until_state("first", |state| state["frame"] == target)
                .await;
            let at = if first["exact"] == false {
                keys.push(at.duration_since(sent));
                agent
                    .until_state("exact", |state| {
                        state["frame"] == target && state["exact"] == true
                    })
                    .await
                    .1
            } else {
                at
            };
            exacts.push(at.duration_since(sent));
        }
        // Fast forward and backward, for a while each, from the start and the middle.
        for (from, speed) in [(0, 8), (last / 2, -1), (last, -8)] {
            agent
                .result("transport.seek", Some(json!({ "frame": from })))
                .await;
            agent
                .result("transport.play", Some(json!({ "speed": speed })))
                .await;
            tokio::time::sleep(Duration::from_millis(1500)).await;
            agent.result("transport.pause", None).await;
        }
        eprintln!(
            "{}: scrubbing, the key frame shown after {keys:?}; the frame itself after {exacts:?} \
             (which waits {} ms for no newer seek)",
            path.display(),
            150
        );
    }
    drop(agent);
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    for line in engine.stderr().lines() {
        if line.contains("transport_played")
            || line.contains("transport_loaded")
            || line.contains("sound_")
        {
            eprintln!("{line}");
        }
    }
}
