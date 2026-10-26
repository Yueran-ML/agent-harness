//! The engine as its clients see it: the binary itself, started on a directory of its own, and
//! clients that connect to it as the desktop and the agent do, by the address that it gives.

use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;

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
    /// What it writes to stdout after its first line.
    stdout: JoinHandle<String>,
}

/// What came of starting an engine.
enum Started {
    Listening(Box<Engine>),
    Ended { code: i32, stderr: String },
}

/// Start an engine on `dir`, and read its first line.
async fn start(dir: &Path, args: &[&str]) -> Started {
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
    let mut errors = BufReader::new(child.stderr.take().unwrap()).lines();
    let reading_stderr = tokio::spawn({
        let stderr = Arc::clone(&stderr);
        async move {
            while let Ok(Some(line)) = errors.next_line().await {
                let mut stderr = stderr.lock().unwrap();
                stderr.push_str(&line);
                stderr.push('\n');
            }
        }
    });
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
    if cfg!(windows) {
        let hash = engine
            .address
            .strip_prefix(r"\\.\pipe\columbina-engine-")
            .unwrap();
        assert_eq!(hash.len(), 16);
        assert!(hash
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
    } else {
        assert!(engine.address.ends_with("/cache/engine.sock"));
    }
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
