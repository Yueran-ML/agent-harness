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
            "headless": true,
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
async fn a_headless_engine_yields_its_project_to_the_desktop_and_ends() {
    let dir = project_dir();
    let engine = engine(dir.path(), &["--headless"]).await;
    let mut agent = engine.client("agent").await;
    agent.apply(add_media("C:/media/a.mp4")).await;
    let last = project(&mut agent).await["project"].clone();
    let mut desktop = Client::connect(&engine.address).await;
    let hello = desktop.hello("desktop").await;
    assert_eq!(
        (&hello["headless"], &hello["pid"]),
        (&json!(true), &json!(engine.pid))
    );
    // Answered before the engine ends, and nothing is answered after it.
    let answer = desktop.call("engine.yield", None).await;
    assert_eq!(answer["result"], json!({}), "{answer}");
    let yielding = json!({
        "jsonrpc": "2.0", "method": "engine.yielding", "params": { "to": "desktop" },
    });
    // Told to everyone, the desktop too, before the answer.
    assert_eq!(
        desktop.take_notifications(),
        std::slice::from_ref(&yielding)
    );
    let rest = desktop.rest().await;
    assert!(rest.is_empty(), "{rest:?}");
    // The agent is told, and its connection closed: it waits for the desktop's engine to name
    // itself in engine.lock.
    let rest = agent.rest().await;
    assert_eq!(rest, [yielding]);
    assert_eq!(engine.ended().await, 0);
    assert!(!dir.path().join("engine.lock").exists());
    assert_eq!(saved(dir.path()), last);
    // The lock is let go: the desktop's own engine takes the project.
    let next = self::engine(dir.path(), &["--headless", "--idle-seconds", "1"]).await;
    let mut client = next.client("desktop").await;
    assert_eq!(project(&mut client).await["project"], last);
}

#[tokio::test]
async fn only_the_desktop_may_ask_and_only_a_headless_engine_yields() {
    let dir = project_dir();
    let engine = engine(dir.path(), &["--headless"]).await;
    let mut agent = engine.client("agent").await;
    let error = agent.error("engine.yield", None).await;
    assert_eq!(error["code"], -32600, "{error}");
    let mut cli = engine.client("cli").await;
    assert_eq!(cli.error("engine.yield", None).await["code"], -32600);
    let mut desktop = engine.client("desktop").await;
    let error = desktop
        .error("engine.yield", Some(json!({ "now": true })))
        .await;
    assert_eq!(error["code"], -32602, "{error}");
    // Still there.
    assert_eq!(project(&mut agent).await["revision"], 0);
    drop((agent, cli, desktop));

    // The desktop's own engine does not yield: it ends when its stdin is closed.
    let dir = project_dir();
    let mut own = self::engine(dir.path(), &[]).await;
    let mut desktop = Client::connect(&own.address).await;
    assert_eq!(desktop.hello("desktop").await["headless"], false);
    let error = desktop.error("engine.yield", None).await;
    assert_eq!(error["code"], -32601, "{error}");
    assert!(own.running());
    drop(desktop);
    drop(own.child.stdin.take());
    assert_eq!(own.ended().await, 0);
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
/// made. It makes no proxies, so that their picture is read from the media itself.
async fn transport_engine(dir: &Path, samples: &[support::Sample]) -> Option<(Engine, Client)> {
    transport_engine_with(dir, samples, &["--sound", "null"]).await
}

/// As [`transport_engine`], with the engine started with `args`.
async fn transport_engine_with(
    dir: &Path,
    samples: &[support::Sample],
    args: &[&str],
) -> Option<(Engine, Client)> {
    let mut paths = Vec::new();
    for &sample in samples {
        paths.push(support::media(sample)?.path.to_str().unwrap().to_owned());
    }
    let mut args = args.to_vec();
    args.push("--no-proxies");
    let engine = engine(dir, &args).await;
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

    // At speed 1, 25 frames a second, by the sound (of the output of no device).
    agent
        .result("transport.play", Some(json!({ "speed": 1 })))
        .await;
    let rate = frames_a_second(&mut agent, 5, Duration::from_millis(2000)).await;
    let state = agent.result("transport.state", None).await;
    assert_eq!(state["sound"], "on", "{state}");
    assert!((21.25..=28.75).contains(&rate), "{rate} frames a second");

    // Paused, it stays.
    agent.result("transport.pause", None).await;
    let (paused, _) = agent
        .until_state("paused", |state| state["playing"] == false)
        .await;
    assert_eq!(paused["exact"], true);
    assert_eq!(paused["sound"], "off");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let state = agent.result("transport.state", None).await;
    assert_eq!(state["frame"], paused["frame"]);
    assert_eq!(state["playing"], false);

    // A seek shows the frame sought, and what it did is told after the answer.
    agent.take_notifications();
    let id = agent
        .request("transport.seek", Some(json!({ "frame": 37 })))
        .await;
    agent.answer(id).await;
    let before: Vec<Value> = agent.take_notifications();
    assert!(
        before.iter().all(|told| told["params"]["frame"] != 37),
        "{before:?}"
    );
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
    // A file that is not there.
    let missing = agent
        .apply(json!([{ "op": "add_media", "media": media("", "C:/not there/x.mp4") }]))
        .await;
    let id = missing["done"][0]["media"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let error = agent
        .error("transport.load", Some(json!({ "media": id })))
        .await;
    assert_eq!(error["code"], -32002, "{error}");
    assert_eq!(error["data"]["reason"], "not_found");
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

#[tokio::test]
#[ignore = "plays on the real output device of the machine: run with --ignored"]
async fn a_source_plays_with_its_sound_on_the_output_device() {
    let dir = project_dir();
    let Some((engine, mut agent)) =
        transport_engine_with(dir.path(), &[support::Sample::Fhd], &["--mute"]).await
    else {
        return;
    };
    agent
        .result("transport.load", Some(json!({ "media": "m1" })))
        .await;
    agent
        .result("transport.play", Some(json!({ "speed": 1 })))
        .await;
    let rate = frames_a_second(&mut agent, 5, Duration::from_millis(2000)).await;
    let state = agent.result("transport.state", None).await;
    match state["sound"].as_str().unwrap() {
        "on" => eprintln!("played with sound: the device is the clock"),
        "unavailable" => eprintln!("no sound: there is no device to play it on"),
        other => panic!("the sound is {other} while it plays at speed 1"),
    }
    assert!((21.25..=28.75).contains(&rate), "{rate} frames a second");
    agent.result("transport.pause", None).await;
    agent
        .until_state("paused", |state| state["playing"] == false)
        .await;
    eprintln!("{}", engine.stderr());
}

/// Go to frame 0, a frame on, and play at speed 1 for half a second, then pause, again and again,
/// as the keys Home, → and L, then K, do: each play starts at the playhead and goes by the clock
/// (the playhead never runs ahead of it), and stops at once when it is paused, where it is.
async fn played_again_and_again_from_the_start(args: &[&str]) {
    let dir = project_dir();
    let Some((engine, mut agent)) =
        transport_engine_with(dir.path(), &[support::Sample::Fhd], args).await
    else {
        return;
    };
    agent
        .result("transport.load", Some(json!({ "media": "m1" })))
        .await;
    for round in 0..4 {
        agent
            .result("transport.seek", Some(json!({ "frame": 0 })))
            .await;
        agent
            .until_state("at 0", |state| {
                state["shown"] == 0 && state["playing"] == false
            })
            .await;
        agent
            .result("transport.step", Some(json!({ "frames": 1 })))
            .await;
        agent
            .until_state("at 1", |state| {
                state["shown"] == 1 && state["playing"] == false
            })
            .await;
        agent.take_notifications();
        let played = Instant::now();
        agent
            .result("transport.play", Some(json!({ "speed": 1 })))
            .await;
        let states = agent.states_for(Duration::from_millis(500)).await;
        let mut moved = 1;
        for (state, at) in &states {
            assert_eq!(state["playing"], true, "round {round}: {states:?}");
            let frame = state["frame"].as_i64().unwrap();
            // 25 frames a second from frame 1, and a frame or two for the reading.
            let most = 1.0 + at.duration_since(played).as_secs_f64() * 25.0 + 2.0;
            #[allow(clippy::cast_precision_loss)]
            let ahead = frame as f64 > most;
            assert!(
                !ahead,
                "round {round}: frame {frame} at most {most}: {states:?}"
            );
            moved = moved.max(frame);
        }
        assert!(moved >= 6, "round {round}: it did not play: {states:?}");
        let paused_at = Instant::now();
        agent.result("transport.pause", None).await;
        let (paused, at) = agent
            .until_state("paused", |state| state["playing"] == false)
            .await;
        assert!(
            at.duration_since(paused_at) < Duration::from_millis(250),
            "round {round}: paused after {:?}",
            at.duration_since(paused_at)
        );
        let frame = paused["frame"].as_i64().unwrap();
        assert!(
            (moved..=moved + 2).contains(&frame),
            "round {round}: {paused} after {moved}"
        );
        // Paused, it stays.
        let after = agent.states_for(Duration::from_millis(300)).await;
        assert!(
            after
                .iter()
                .all(|(state, _)| state["playing"] == false && state["frame"] == frame),
            "round {round}: {after:?}"
        );
    }
    drop(engine);
}

#[tokio::test]
async fn played_again_and_again_from_the_start_by_the_sound_it_stops_where_it_is_paused() {
    played_again_and_again_from_the_start(&["--sound", "null"]).await;
}

#[tokio::test]
#[ignore = "plays on the real output device of the machine: run with --ignored"]
async fn played_again_and_again_from_the_start_on_the_device_it_stops_where_it_is_paused() {
    played_again_and_again_from_the_start(&["--mute"]).await;
}

/// Wait until the engine has logged `event`, and give its log.
async fn until_logged(engine: &Engine, event: &str) -> String {
    let deadline = Instant::now() + WAIT;
    let wanted = format!(r#""event":"{event}""#);
    loop {
        let stderr = engine.stderr();
        if stderr.contains(&wanted) {
            return stderr;
        }
        assert!(Instant::now() < deadline, "no {event}: {stderr}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn sound_alone_plays_on_a_grid_of_frames_by_its_sound() {
    let dir = project_dir();
    let Some((_engine, mut agent)) =
        transport_engine(dir.path(), &[support::Sample::Wav, support::Sample::Noise]).await
    else {
        return;
    };
    // 2 s on a grid of 25 frames a second.
    let loaded = agent
        .result("transport.load", Some(json!({ "media": "m1" })))
        .await;
    assert_eq!(
        (
            &loaded["frame"],
            &loaded["shown"],
            &loaded["last"],
            &loaded["exact"]
        ),
        (&json!(0), &Value::Null, &json!(49), &json!(true)),
        "{loaded}"
    );
    assert_eq!(loaded["sound"], "off");
    agent
        .result("transport.play", Some(json!({ "speed": 1 })))
        .await;
    let rate = frames_a_second(&mut agent, 3, Duration::from_millis(1000)).await;
    assert!((21.25..=28.75).contains(&rate), "{rate} frames a second");
    let (stopped, _) = agent
        .until_state("stopped", |state| state["playing"] == false)
        .await;
    assert_eq!(stopped["frame"], 49, "{stopped}");
    // Sought and stepped, as a picture is.
    agent
        .result(
            "transport.seek",
            Some(json!({ "frame": 10, "scrubbing": true })),
        )
        .await;
    agent
        .until_state("sought", |state| {
            state["frame"] == 10 && state["exact"] == true
        })
        .await;
    agent
        .result("transport.step", Some(json!({ "frames": 5 })))
        .await;
    agent
        .until_state("stepped", |state| state["frame"] == 15)
        .await;
    // At another speed, and backward, it moves in silence.
    agent
        .result("transport.play", Some(json!({ "speed": -4 })))
        .await;
    let (backward, _) = agent
        .until_state("backward", |state| state["playing"] == true)
        .await;
    assert_eq!(backward["sound"], "off");
    let (stopped, _) = agent
        .until_state("at the start", |state| state["playing"] == false)
        .await;
    assert_eq!(stopped["frame"], 0, "{stopped}");

    // On the grid of the first timeline of the project: 3 s at 30 frames a second.
    agent
        .apply(json!([{ "op": "add_timeline", "timeline": {
            "id": "", "name": "Cut", "rate": { "num": 30, "den": 1 }, "tracks": [],
        } }]))
        .await;
    let loaded = agent
        .result("transport.load", Some(json!({ "media": "m2" })))
        .await;
    assert_eq!(loaded["last"], 89, "{loaded}");
    agent
        .result("transport.play", Some(json!({ "speed": 1 })))
        .await;
    let rate = frames_a_second(&mut agent, 3, Duration::from_millis(1000)).await;
    assert!((25.5..=34.5).contains(&rate), "{rate} frames a second");
    let state = agent.result("transport.state", None).await;
    assert_eq!(state["sound"], "on");
}

#[tokio::test]
async fn a_device_that_fails_is_left_for_the_clock_of_the_system_and_opened_again() {
    let dir = project_dir();
    let Some((engine, mut agent)) = transport_engine_with(
        dir.path(),
        &[support::Sample::Fhd],
        &["--sound", "fail-after=400"],
    )
    .await
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
        .until_state("playing with sound", |state| {
            state["playing"] == true && state["sound"] == "on"
        })
        .await;
    // The device fails after 400 ms: playing goes on from where it was, without a jump.
    let (lost, _) = agent
        .until_state("the sound lost", |state| state["sound"] == "unavailable")
        .await;
    assert_eq!(lost["playing"], true, "{lost}");
    let at = lost["frame"].as_i64().unwrap();
    assert!((5..=20).contains(&at), "{lost}");
    let states = agent.states_for(Duration::from_millis(600)).await;
    let frames: Vec<i64> = states
        .iter()
        .map(|(state, _)| state["frame"].as_i64().unwrap())
        .collect();
    assert!(
        frames.windows(2).all(|pair| pair[0] <= pair[1]),
        "{frames:?}"
    );
    let moved = frames.last().unwrap() - at;
    assert!((10..=20).contains(&moved), "{frames:?}");
    // Played again, the device is opened anew.
    agent.result("transport.pause", None).await;
    agent
        .result("transport.seek", Some(json!({ "frame": 0 })))
        .await;
    agent
        .result("transport.play", Some(json!({ "speed": 1 })))
        .await;
    agent
        .until_state("the sound again", |state| {
            state["playing"] == true && state["sound"] == "on"
        })
        .await;
    until_logged(&engine, "transport_sound_lost").await;
    // The log is read by a task of the test: the second device comes soon.
    let deadline = Instant::now() + WAIT;
    while engine.stderr().matches(r#""event":"sound_ready""#).count() < 2 {
        assert!(Instant::now() < deadline, "{}", engine.stderr());
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn a_device_that_never_asks_for_sound_is_given_up() {
    let dir = project_dir();
    let Some((_engine, mut agent)) =
        transport_engine_with(dir.path(), &[support::Sample::Fhd], &["--sound", "stall"]).await
    else {
        return;
    };
    agent
        .result("transport.load", Some(json!({ "media": "m1" })))
        .await;
    let started = Instant::now();
    agent
        .result("transport.play", Some(json!({ "speed": 1 })))
        .await;
    // Nothing is heard: the playhead waits for the sound, then goes on without it.
    let (lost, _) = agent
        .until_state("the sound given up", |state| {
            state["sound"] == "unavailable"
        })
        .await;
    let took = started.elapsed();
    assert!(
        took >= Duration::from_millis(700) && took < Duration::from_millis(2000),
        "{took:?}"
    );
    assert_eq!(
        (&lost["frame"], &lost["playing"]),
        (&json!(0), &json!(true))
    );
    let rate = frames_a_second(&mut agent, 3, Duration::from_millis(1500)).await;
    assert!((21.25..=28.75).contains(&rate), "{rate} frames a second");
}

#[tokio::test]
async fn a_card_that_fails_to_give_a_frame_is_left_for_the_processor() {
    // A frame sought, and frames played.
    for (frame, play) in [(40, false), (20, true)] {
        let dir = project_dir();
        let at = frame.to_string();
        let Some((engine, mut agent)) = transport_engine_with(
            dir.path(),
            &[support::Sample::Fhd],
            &["--sound", "null", "--fail-card-at", &at],
        )
        .await
        else {
            return;
        };
        agent
            .result("transport.load", Some(json!({ "media": "m1" })))
            .await;
        let loaded = until_logged(&engine, "transport_loaded").await;
        if loaded.contains(r#""decoder":"software""#) {
            eprintln!("skipped: the file is not decoded on the graphics card");
            return;
        }
        agent.take_notifications();
        if play {
            agent
                .result("transport.play", Some(json!({ "speed": 4 })))
                .await;
            agent
                .until_state("playing", |state| state["playing"] == true)
                .await;
            let (stopped, _) = agent
                .until_state("the end", |state| state["playing"] == false)
                .await;
            assert_eq!(
                (&stopped["frame"], &stopped["shown"]),
                (&json!(99), &json!(99))
            );
        } else {
            agent
                .result("transport.seek", Some(json!({ "frame": frame })))
                .await;
            let (sought, _) = agent
                .until_state("sought", |state| state["frame"] == frame)
                .await;
            assert_eq!(
                (&sought["shown"], &sought["exact"]),
                (&json!(frame), &json!(true))
            );
        }
        let stderr = until_logged(&engine, "transport_device_failed").await;
        assert!(!stderr.contains("transport_error"), "{stderr}");
        // On the processor from then on.
        agent
            .result("transport.step", Some(json!({ "frames": -3 })))
            .await;
        let wanted = if play { 96 } else { frame - 3 };
        let (stepped, _) = agent
            .until_state("stepped", |state| state["frame"] == wanted)
            .await;
        assert_eq!(stepped["exact"], true);
    }
}

#[tokio::test]
async fn media_removed_while_it_is_loaded_is_unloaded_once_it_is() {
    let dir = project_dir();
    let Some((engine, mut agent)) = transport_engine(dir.path(), &[support::Sample::Fhd]).await
    else {
        return;
    };
    let mut desktop = engine.client("desktop").await;
    // The load is sent, and the media removed by another client before it is done.
    let load = agent
        .request("transport.load", Some(json!({ "media": "m1" })))
        .await;
    desktop
        .apply(json!([{ "op": "remove_media", "id": "m1" }]))
        .await;
    let answer = agent.answer(load).await;
    if let Some(error) = answer.get("error") {
        // It was removed before the load found it.
        assert_eq!(error["data"]["reason"], "missing", "{answer}");
    }
    let (unloaded, _) = desktop
        .until_state("unloaded", |state| state["media"].is_null())
        .await;
    assert_eq!(unloaded["playing"], false);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let state = agent.result("transport.state", None).await;
    assert_eq!(state["media"], Value::Null, "{state}");
}

#[tokio::test]
async fn a_command_sent_while_another_client_loads_is_not_refused() {
    let dir = project_dir();
    let Some((engine, mut agent)) = transport_engine(dir.path(), &[support::Sample::Fhd]).await
    else {
        return;
    };
    let mut desktop = engine.client("desktop").await;
    let load = agent
        .request("transport.load", Some(json!({ "media": "m1" })))
        .await;
    // Before the load is done (opening the decoder takes some 100 ms), once the engine has
    // read it: taken, and done after it.
    tokio::time::sleep(Duration::from_millis(20)).await;
    let played = desktop
        .result("transport.play", Some(json!({ "speed": 1 })))
        .await;
    assert_eq!(played, json!({}));
    let loaded = agent.answer(load).await;
    assert_eq!(loaded["result"]["playing"], false, "{loaded}");
    desktop
        .until_state("playing", |state| state["playing"] == true)
        .await;
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
    let Some((engine, mut agent)) =
        transport_engine(dir.path(), &[support::Sample::Fhd, support::Sample::Wav]).await
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
    // Sound alone: the view is black.
    agent
        .result("transport.load", Some(json!({ "media": "m2" })))
        .await;
    let deadline = Instant::now() + WAIT;
    loop {
        let (grey, width) = parent.grey();
        let brightest = (0..540)
            .flat_map(|y| (0..960).map(move |x| (x, y)))
            .step_by(97)
            .map(|(x, y)| grey[y * width + x])
            .max()
            .unwrap();
        if brightest < 4 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the view is not black: {brightest}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
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
    let engine = engine(
        dir.path(),
        if aloud {
            &["--no-proxies"]
        } else {
            &["--mute", "--no-proxies"]
        },
    )
    .await;
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

// Proxies.

impl Client {
    /// Every `media.proxy` that comes, of those that came or come, until `done` takes those
    /// told so far; the other messages are let go. The engine tells how far the proxy being
    /// made has come every second: it fails if nothing is told of proxies for `2 * WAIT`, and
    /// after `10 * WAIT` in all. A proxy made on the processor, in `ProRes`, by a computer whose
    /// every core is busy with the other tests (as on the CI) takes a minute or more for the
    /// media of a test, and NVENC a few seconds.
    async fn proxies_until(&mut self, done: impl Fn(&[Value]) -> bool) -> Vec<Value> {
        let end = Instant::now() + 10 * WAIT;
        let mut deadline = Instant::now() + 2 * WAIT;
        let mut notes = Vec::new();
        loop {
            if done(&notes) {
                return notes;
            }
            let message = if self.notifications.is_empty() {
                let now = Instant::now();
                assert!(now < deadline && now < end, "{notes:?}");
                self.message().await
            } else {
                self.notifications.remove(0)
            };
            if message["method"] == "media.proxy" {
                notes.push(message["params"].clone());
                deadline = Instant::now() + 2 * WAIT;
            }
        }
    }

    /// Every `media.proxy` until each of `media` is done or has failed.
    async fn proxies_of(&mut self, media: &[&str]) -> Vec<Value> {
        self.proxies_until(|notes| {
            media.iter().all(|media| {
                notes.iter().any(|note| {
                    note["media"] == *media
                        && (note["state"] == "done" || note["state"] == "failed")
                })
            })
        })
        .await
    }
}

/// An engine that is not headless, makes proxies and plays its sound on no device, started with
/// `args` too, and a client of it, with `samples` taken in as media `m1`, `m2`, …
async fn proxy_engine(
    dir: &Path,
    samples: &[support::Sample],
    args: &[&str],
) -> Option<(Engine, Client)> {
    let mut paths = Vec::new();
    for &sample in samples {
        paths.push(support::media(sample)?.path.to_str().unwrap().to_owned());
    }
    let mut all = vec!["--sound", "null"];
    let ffmpeg = build_ffmpeg();
    if let Some(ffmpeg) = &ffmpeg {
        all.extend(["--proxy-ffmpeg", ffmpeg.as_str()]);
    }
    all.extend(args);
    let engine = engine(dir, &all).await;
    let mut agent = engine.client("agent").await;
    agent
        .result("media.import", Some(json!({ "paths": paths })))
        .await;
    Some((engine, agent))
}

/// The `ffmpeg` of the LGPL build that the engine was built with, which makes proxies: an engine
/// of a release build finds none but the one beside it, which the tests do not put there.
fn build_ffmpeg() -> Option<String> {
    let name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    option_env!("FFMPEG_DIR")
        .map(|dir| Path::new(dir).join("bin").join(name))
        .filter(|path| path.is_file())
        .map(|path| path.to_string_lossy().into_owned())
}

/// The files in the directory of the proxies of a project.
fn proxy_files(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir.join("cache").join("proxies"))
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// Numbers below `below` that are the same each time.
fn numbers(seed: u64, count: usize, below: i64) -> Vec<i64> {
    let mut state = seed;
    (0..count)
        .map(|_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            i64::try_from((state >> 33) % u64::try_from(below).unwrap()).unwrap()
        })
        .collect()
}

/// Check that only one proxy is made at a time, each queued, made and done.
fn one_at_a_time(notes: &[Value], media: &[&str]) {
    let mut making: Option<String> = None;
    for note in notes {
        let id = note["media"].as_str().unwrap().to_owned();
        match note["state"].as_str().unwrap() {
            "queued" => assert_eq!(note["progress"], 0.0, "{notes:?}"),
            "running" => {
                if let Some(other) = &making {
                    assert_eq!(*other, id, "two at once: {notes:?}");
                }
                making = Some(id);
                let progress = note["progress"].as_f64().unwrap();
                assert!((0.0..=1.0).contains(&progress), "{note}");
            }
            "done" => {
                assert_eq!(making.as_deref(), Some(id.as_str()), "{notes:?}");
                assert_eq!(note["progress"], 1.0);
                making = None;
            }
            other => panic!("{other}: {notes:?}"),
        }
    }
    for media in media {
        let states: Vec<&str> = notes
            .iter()
            .filter(|note| note["media"] == *media)
            .map(|note| note["state"].as_str().unwrap())
            .collect();
        assert_eq!(states.first(), Some(&"queued"), "{media}: {states:?}");
        assert_eq!(states.get(1), Some(&"running"), "{media}: {states:?}");
        assert_eq!(states.last(), Some(&"done"), "{media}: {states:?}");
    }
}

/// Check that the proxy of media of a project is of every frame of the media, each a key frame,
/// and that the frame shown at each number is the frame of the media shown at it: decoding the
/// media on, frame after frame, as the rule of the numbers of frames has it, and seeking in the
/// proxy, number after number in an order of their own, and the key frame at or before each
/// number is the frame shown there, so that scrubbing shows it at once.
fn aligned(media: &Value, seed: u64) {
    use columbina_media::{Hardware, VideoDecoder};
    let proxy = &media["proxy"];
    assert_eq!(
        proxy["digest_of_source"], media["identity"]["digest"],
        "{media}"
    );
    let path = Path::new(proxy["path"].as_str().unwrap());
    assert!(path.is_file(), "{proxy}");
    let source = Path::new(media["path"].as_str().unwrap());
    let rate = serde_json::from_value(media["facts"]["video"]["rate"].clone()).unwrap();
    let width = media["facts"]["video"]["width"].as_u64().unwrap();
    let blocks = usize::try_from(64 * proxy["width"].as_u64().unwrap() / width).unwrap();
    let carried = |picture: &columbina_media::Picture, size: usize| {
        support::number_in_blocks(&picture.luma, picture.luma_stride, picture.bits, size)
    };
    // The media, decoded on: the frame shown at each number, and how many frames it has.
    let mut original = VideoDecoder::open(source, Hardware::Auto).unwrap();
    let mut frames = 0;
    let mut last = 0;
    while let Some(picture) = original.next().unwrap() {
        frames += 1;
        last = picture.frame;
    }
    let mut original = VideoDecoder::open(source, Hardware::Auto).unwrap();
    let shown: Vec<Option<i64>> = (0..=last)
        .map(|frame| carried(&original.frame_at(frame).unwrap(), 64))
        .collect();
    // Every frame a key frame, as many as the media has.
    let gop = columbina_media::gop(path, u64::MAX).unwrap();
    assert!(gop.all_intra(), "{proxy}: {gop:?}");
    assert!(!gop.b_frames, "{proxy}: {gop:?}");
    assert_eq!(gop.packets, frames, "{proxy}");
    let mut decoder = VideoDecoder::open_at_rate(path, Hardware::Auto, rate).unwrap();
    let mut sought = numbers(seed, usize::try_from(last + 1).unwrap(), last + 1);
    sought.extend([0, last, last / 2]);
    sought.extend(0..=last);
    for frame in sought {
        let picture = decoder.frame_at(frame).unwrap();
        let at = usize::try_from(frame).unwrap();
        assert_eq!(
            carried(&picture, blocks),
            shown[at],
            "{} frame {frame}",
            source.display()
        );
        assert_eq!(
            decoder.keyframe_at_or_before(frame),
            Some(picture.frame),
            "{} frame {frame}",
            source.display()
        );
    }
}

/// The encoder that made the proxy of each of `media`, as the engine logs it: NVENC where it
/// can use a card of NVIDIA, else ProRes, which is all that a computer without one (as those of
/// the CI) has. The log is written on a thread of its own, and may come a little after the
/// proxy is told done.
async fn made_by(engine: &Engine, media: &[&str]) -> Vec<String> {
    let deadline = Instant::now() + WAIT;
    loop {
        let made = logged(engine, "proxy_made");
        let found: Option<Vec<String>> = media
            .iter()
            .map(|id| {
                made.iter()
                    .find(|line| line["media"] == *id)
                    .and_then(|line| line["encoder"].as_str())
                    .map(str::to_owned)
            })
            .collect();
        if let Some(found) = found {
            return found;
        }
        assert!(Instant::now() < deadline, "{made:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// The codec of a proxy that an encoder of the engine makes.
fn codec_of(encoder: &str) -> &'static str {
    match encoder {
        "hevc_nvenc" | "hevc_nvenc (cuda)" => "hevc",
        "prores_ks" => "prores",
        other => panic!("no proxy is made by {other}"),
    }
}

#[tokio::test]
async fn proxies_are_made_one_at_a_time_with_every_frame_a_key_frame_numbered_as_the_media() {
    use support::Sample;
    let dir = project_dir();
    let samples = [
        Sample::Uhd,
        Sample::TsH264,
        Sample::Fhd,
        Sample::Collide,
        Sample::Vfr,
        Sample::SameTimes,
        Sample::MkvLateStart,
        Sample::Portrait,
        Sample::EditList,
    ];
    let Some((engine, mut agent)) = proxy_engine(dir.path(), &samples, &[]).await else {
        return;
    };
    let ids: Vec<String> = (1..=samples.len()).map(|n| format!("m{n}")).collect();
    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
    let notes = agent.proxies_of(&ids).await;
    one_at_a_time(&notes, &ids);
    let got = project(&mut agent).await;
    let encoders = made_by(&engine, &ids).await;
    for (index, sample) in samples.into_iter().enumerate() {
        let media = &got["project"]["media"][index];
        eprintln!("{sample:?}, by {}: {}", encoders[index], media["proxy"]);
        assert_eq!(
            media["proxy"]["codec"],
            codec_of(&encoders[index]),
            "{media}"
        );
        aligned(media, 17 + index as u64);
    }
    // The 4K picture is made 1920 wide, the others kept as they are.
    let size = |index: usize| {
        let proxy = &got["project"]["media"][index]["proxy"];
        (proxy["width"].clone(), proxy["height"].clone())
    };
    assert_eq!(size(0), (json!(1920), json!(1080)));
    assert_eq!(size(1), (json!(640), json!(360)));
    assert_eq!(size(7), (json!(640), json!(360)));
}

#[tokio::test]
async fn a_proxy_made_in_prores_where_nvenc_is_not_is_numbered_as_the_media() {
    use support::Sample;
    let dir = project_dir();
    let samples = [Sample::Uhd, Sample::Collide];
    let Some((_engine, mut agent)) =
        proxy_engine(dir.path(), &samples, &["--proxy-encoder", "prores"]).await
    else {
        return;
    };
    agent.proxies_of(&["m1", "m2"]).await;
    let got = project(&mut agent).await;
    for index in 0..2 {
        let media = &got["project"]["media"][index];
        assert_eq!(media["proxy"]["codec"], "prores", "{media}");
        aligned(media, 3 + index as u64);
    }
    // And the transport reads it, every frame at once.
    let loaded = agent
        .result("transport.load", Some(json!({ "media": "m1" })))
        .await;
    assert_eq!(loaded["source"], "proxy");
    for (_, state) in scrub(&mut agent, &numbers(8, 10, 180)).await {
        assert_eq!(
            (&state["exact"], &state["shown"]),
            (&json!(true), &state["frame"])
        );
    }
}

#[tokio::test]
async fn a_proxy_of_media_that_is_removed_is_stopped_and_its_part_deleted() {
    let dir = project_dir();
    let Some((_engine, mut agent)) = proxy_engine(
        dir.path(),
        &[support::Sample::Uhd],
        &["--proxy-readrate", "1"],
    )
    .await
    else {
        return;
    };
    agent
        .proxies_until(|notes| notes.iter().any(|note| note["state"] == "running"))
        .await;
    let deadline = Instant::now() + WAIT;
    while !proxy_files(dir.path()).iter().any(|file| {
        Path::new(file)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("part"))
    }) {
        assert!(Instant::now() < deadline, "no .part");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    agent
        .apply(json!([{ "op": "remove_media", "id": "m1" }]))
        .await;
    let notes = agent
        .proxies_until(|notes| notes.iter().any(|note| note["state"] == "failed"))
        .await;
    let failed = notes.last().unwrap();
    assert_eq!(failed["media"], "m1");
    assert!(
        failed["message"].as_str().unwrap().contains("removed"),
        "{failed}"
    );
    assert!(
        proxy_files(dir.path()).is_empty(),
        "{:?}",
        proxy_files(dir.path())
    );
    // Nothing is set on media that is not there.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(proxy_files(dir.path()), Vec::<String>::new());
    assert_eq!(project(&mut agent).await["project"]["media"], json!([]));
}

/// Wait until the media at `index` of the project has a proxy made from its file, asking for the
/// project again and again, and give the media.
async fn until_proxy_set(client: &mut Client, index: usize) -> Value {
    let deadline = Instant::now() + 2 * WAIT;
    loop {
        let got = project(client).await;
        let media = got["project"]["media"][index].clone();
        if media["proxy"]["digest_of_source"] == media["identity"]["digest"] {
            return media;
        }
        assert!(Instant::now() < deadline, "{media}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn a_proxy_cut_short_by_the_end_of_the_engine_is_made_by_the_next() {
    let dir = project_dir();
    let Some((mut engine, mut agent)) = proxy_engine(
        dir.path(),
        &[support::Sample::Uhd],
        &["--proxy-readrate", "1"],
    )
    .await
    else {
        return;
    };
    agent
        .proxies_until(|notes| notes.iter().any(|note| note["state"] == "running"))
        .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    drop(agent);
    drop(engine.child.stdin.take());
    assert_eq!(engine.ended().await, 0);
    // What it wrote is deleted, and nothing is set.
    assert!(
        proxy_files(dir.path()).is_empty(),
        "{:?}",
        proxy_files(dir.path())
    );
    assert_eq!(saved(dir.path())["media"][0].get("proxy"), None);
    // The next engine finds that the media has none, and makes it.
    let engine = self::engine(dir.path(), &["--sound", "null"]).await;
    let mut agent = engine.client("agent").await;
    let media = until_proxy_set(&mut agent, 0).await;
    assert!(Path::new(media["proxy"]["path"].as_str().unwrap()).is_file());
    assert_eq!(proxy_files(dir.path()).len(), 1);
}

#[tokio::test]
async fn a_proxy_made_from_another_file_is_made_again() {
    let dir = project_dir();
    let Some(made) = support::media(support::Sample::Fhd) else {
        return;
    };
    {
        let mut engine = engine(dir.path(), &["--no-proxies", "--sound", "null"]).await;
        let mut agent = engine.client("agent").await;
        agent
            .result(
                "media.import",
                Some(json!({ "paths": [made.path.to_str().unwrap()] })),
            )
            .await;
        drop(agent);
        drop(engine.child.stdin.take());
        assert_eq!(engine.ended().await, 0);
    }
    // A proxy of another file, as when the file was put in the place of another.
    let proxies = dir.path().join("cache").join("proxies");
    std::fs::create_dir_all(&proxies).unwrap();
    let stale = proxies.join("m1-ffffffffffff.mov");
    std::fs::write(&stale, b"not a proxy").unwrap();
    let mut saved_project = saved(dir.path());
    saved_project["media"][0]["proxy"] = json!({
        "path": stale.to_str().unwrap(), "width": 1920, "height": 1080, "codec": "hevc",
        "digest_of_source": "f".repeat(64),
    });
    std::fs::write(
        dir.path().join("project.json"),
        serde_json::to_string_pretty(&saved_project).unwrap(),
    )
    .unwrap();
    let engine = engine(dir.path(), &["--sound", "null"]).await;
    let mut agent = engine.client("agent").await;
    let media = until_proxy_set(&mut agent, 0).await;
    let path = media["proxy"]["path"].as_str().unwrap();
    assert_ne!(Path::new(path), stale);
    assert!(Path::new(path).is_file());
    // The one whose place it took is deleted, once the new one is set.
    let deadline = Instant::now() + WAIT;
    while stale.exists() {
        assert!(Instant::now() < deadline, "{} is left", stale.display());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // It is no step to undo: this engine has none.
    let undone = agent.result("history.undo", None).await;
    assert_eq!(undone["done"], Value::Null, "{undone}");
}

#[tokio::test]
async fn a_proxy_is_set_and_taken_back_by_a_transaction_and_its_undo() {
    let dir = project_dir();
    let engine = engine(dir.path(), &["--headless"]).await;
    let mut agent = engine.client("agent").await;
    let proxies = dir.path().join("cache").join("proxies");
    std::fs::create_dir_all(&proxies).unwrap();
    let path = Path::new(&written(dir.path()))
        .join("cache")
        .join("proxies")
        .join("m1-000000000000.mov");
    let proxy = json!({
        "path": path.to_str().unwrap(), "width": 960, "height": 540, "codec": "hevc",
        "digest_of_source": "0".repeat(64),
    });
    let mut added = add_media("C:/media/a.mp4");
    added[0]["media"]["id"] = "$m".into();
    added
        .as_array_mut()
        .unwrap()
        .push(json!({ "op": "set_proxy", "media": "$m", "proxy": proxy }));
    let applied = agent.apply(added).await;
    assert_eq!(applied["names"]["$m"], "m1");
    assert_eq!(applied["done"][1]["media"], "m1");
    let got = project(&mut agent).await;
    assert_eq!(got["project"]["media"][0]["proxy"], proxy);
    // Taken away, and back again by the undo.
    agent
        .apply(json!([{ "op": "set_proxy", "media": "m1", "proxy": null }]))
        .await;
    assert_eq!(
        project(&mut agent).await["project"]["media"][0].get("proxy"),
        None
    );
    let undone = agent.result("history.undo", None).await;
    assert_eq!(
        undone["done"],
        json!([{ "op": "set_proxy", "media": "m1", "proxy": proxy }])
    );
    assert_eq!(
        project(&mut agent).await["project"]["media"][0]["proxy"],
        proxy
    );
    // A path that is not where the engine keeps proxies is refused.
    let refused = agent
        .error(
            "transaction.apply",
            Some(
                json!({ "ops": [{ "op": "set_proxy", "media": "m1", "proxy": {
                "path": "m1.mov", "width": 1, "height": 1, "codec": "hevc",
                "digest_of_source": "",
            } }] }),
            ),
        )
        .await;
    assert_eq!(refused["code"], -32001);
    assert_eq!(refused["data"]["reason"], "not_a_proxy_path", "{refused}");
    // A headless engine makes no proxies.
    drop(agent);
    assert_eq!(proxy_files(dir.path()), Vec::<String>::new());
}

#[tokio::test]
async fn a_proxy_elsewhere_is_refused_and_taken_away_when_read_and_never_read_or_deleted() {
    let dir = project_dir();
    let elsewhere = tempfile::tempdir().unwrap();
    let Some(made) = support::media(support::Sample::Uhd) else {
        return;
    };
    // A file elsewhere, of the name of a proxy, damaged as a proxy would be.
    let victim = elsewhere.path().join("m1-0eb91568ee7d.mov");
    std::fs::write(&victim, vec![0_u8; 4096]).unwrap();
    {
        let mut engine = engine(dir.path(), &["--no-proxies", "--sound", "null"]).await;
        let mut agent = engine.client("agent").await;
        agent
            .result(
                "media.import",
                Some(json!({ "paths": [made.path.to_str().unwrap()] })),
            )
            .await;
        std::fs::create_dir_all(dir.path().join("cache").join("proxies")).unwrap();
        let inside = Path::new(&written(dir.path()))
            .join("cache")
            .join("proxies");
        let proxy = |path: &Path| {
            json!({
                "path": path.to_str().unwrap(), "width": 1920, "height": 1080, "codec": "hevc",
                "digest_of_source": "0".repeat(64),
            })
        };
        let through = inside
            .join("..")
            .join("..")
            .join("..")
            .join(elsewhere.path().file_name().unwrap())
            .join("m1-0eb91568ee7d.mov");
        for path in [
            victim.clone(),
            through,
            inside.join("notes.txt"),
            inside.join("m1.mov"),
        ] {
            let refused = agent
                .error(
                    "transaction.apply",
                    Some(json!({ "ops": [
                        { "op": "set_proxy", "media": "m1", "proxy": proxy(&path) },
                    ] })),
                )
                .await;
            assert_eq!(refused["data"]["reason"], "not_a_proxy_path", "{refused}");
            assert_eq!(refused["data"]["path"], path.to_str().unwrap());
        }
        let mut added = add_media("C:/media/b.mp4");
        added[0]["media"]["proxy"] = proxy(&victim);
        let refused = agent
            .error("transaction.apply", Some(json!({ "ops": added })))
            .await;
        assert_eq!(refused["data"]["reason"], "not_a_proxy_path", "{refused}");
        // Taking a proxy away is always taken.
        agent
            .apply(json!([{ "op": "set_proxy", "media": "m1", "proxy": null }]))
            .await;
        drop(agent);
        drop(engine.child.stdin.take());
        assert_eq!(engine.ended().await, 0);
    }
    // A project written elsewhere names it as the proxy of its media.
    let mut saved_project = saved(dir.path());
    saved_project["media"][0]["proxy"] = json!({
        "path": victim.to_str().unwrap(), "width": 1920, "height": 1080, "codec": "hevc",
        "digest_of_source": saved_project["media"][0]["identity"]["digest"],
    });
    std::fs::write(
        dir.path().join("project.json"),
        serde_json::to_string_pretty(&saved_project).unwrap(),
    )
    .unwrap();
    let engine = engine(dir.path(), &["--no-proxies", "--sound", "null"]).await;
    let mut agent = engine.client("agent").await;
    // Read without it: the transport reads the media itself, and nothing is deleted.
    assert_eq!(
        project(&mut agent).await["project"]["media"][0].get("proxy"),
        None
    );
    let loaded = agent
        .result("transport.load", Some(json!({ "media": "m1" })))
        .await;
    assert_eq!(loaded["source"], "original", "{loaded}");
    agent
        .result("transport.seek", Some(json!({ "frame": 90 })))
        .await;
    agent.until_state("90", |state| state["shown"] == 90).await;
    let dropped = logged_at_least(&engine, "proxy_dropped", 1).await;
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert!(victim.exists(), "the file elsewhere is never deleted");
}

/// The time from each seek of a scrub to the first state that is told of its frame, and that
/// state; a seek every 1/30 s at most, as a page sends them.
async fn scrub(client: &mut Client, frames: &[i64]) -> Vec<(Duration, Value)> {
    let mut times = Vec::new();
    for &frame in frames {
        client.take_notifications();
        let sent = Instant::now();
        client
            .result(
                "transport.seek",
                Some(json!({ "frame": frame, "scrubbing": true })),
            )
            .await;
        let (state, at) = client
            .until_state("sought", |state| state["frame"] == frame)
            .await;
        times.push((at.duration_since(sent), state));
        let next = sent + Duration::from_millis(33);
        tokio::time::sleep_until(next.into()).await;
    }
    times
}

/// The median and the most of some times.
fn median_and_most(times: &[Duration]) -> (Duration, Duration) {
    let mut sorted = times.to_vec();
    sorted.sort();
    (sorted[sorted.len() / 2], *sorted.last().unwrap())
}

/// What the engine logged of `event`, once it has logged `count` of them or [`WAIT`] has
/// passed, each line read as JSON. The log is written by a thread of the engine's own, to its
/// stderr, which a task of the test reads: a line logged before a notification was sent may come
/// after it, so that a test that reads the log once, as soon as the notification comes, may not
/// find the line yet (as when the machine is busy).
async fn logged_at_least(engine: &Engine, event: &str, count: usize) -> Vec<Value> {
    let deadline = Instant::now() + WAIT;
    loop {
        let lines = logged(engine, event);
        if lines.len() >= count || Instant::now() >= deadline {
            return lines;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// What the engine logged of `event` so far, each line read as JSON.
fn logged(engine: &Engine, event: &str) -> Vec<Value> {
    let wanted = format!(r#""event":"{event}""#);
    engine
        .stderr()
        .lines()
        .filter(|line| line.contains(&wanted))
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn the_transport_reads_the_proxy_scrubs_exactly_and_falls_back_to_the_media() {
    let dir = project_dir();
    let Some((engine, mut agent)) = proxy_engine(
        dir.path(),
        &[support::Sample::Uhd, support::Sample::Fhd],
        &[],
    )
    .await
    else {
        return;
    };
    agent.proxies_of(&["m1", "m2"]).await;
    let loaded = agent
        .result("transport.load", Some(json!({ "media": "m1" })))
        .await;
    assert_eq!(
        (&loaded["source"], &loaded["proxies"], &loaded["last"]),
        (&json!("proxy"), &json!(true), &json!(179)),
        "{loaded}"
    );
    // Scrubbing shows the frame itself at once: every frame of the proxy is a key frame.
    let frames = numbers(5, 30, 180);
    let times = scrub(&mut agent, &frames).await;
    for (_, state) in &times {
        assert_eq!(state["exact"], true, "{state}");
        assert_eq!(state["shown"], state["frame"], "{state}");
    }
    let (median, most) = median_and_most(&times.iter().map(|(time, _)| *time).collect::<Vec<_>>());
    eprintln!(
        "scrubbing the proxy of 4K: the frame itself after {median:?} (median), {most:?} at most"
    );
    assert!(median < Duration::from_millis(100), "{median:?}");
    // Backward from the end, every frame (or nearly) is shown, at 60 a second.
    agent
        .result("transport.seek", Some(json!({ "frame": 179 })))
        .await;
    agent
        .until_state("179", |state| state["shown"] == 179)
        .await;
    agent
        .result("transport.play", Some(json!({ "speed": -1 })))
        .await;
    let (stopped, _) = agent
        .until_state("at frame 0", |state| {
            state["playing"] == false && state["frame"] == 0
        })
        .await;
    assert_eq!(stopped["shown"], 0);
    let deadline = Instant::now() + WAIT;
    while logged(&engine, "transport_played").is_empty() {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let played = logged(&engine, "transport_played").pop().unwrap();
    let presented = played["presented"].as_f64().unwrap();
    let seconds = played["seconds"].as_f64().unwrap();
    eprintln!(
        "backward at -1 on the proxy of 4K: {presented} frames in {seconds:.2} s, {:.1} a second",
        presented / seconds
    );
    assert!(presented >= 0.75 * 179.0, "{played}");
    // Proxies turned off: the media itself, at once while it does not play; and on again.
    agent
        .result("transport.proxies", Some(json!({ "on": false })))
        .await;
    let (off, _) = agent
        .until_state("off", |state| state["source"] == "original")
        .await;
    assert_eq!((&off["proxies"], &off["shown"]), (&json!(false), &json!(0)));
    agent
        .result(
            "transport.seek",
            Some(json!({ "frame": 61, "scrubbing": true })),
        )
        .await;
    let (sought, _) = agent.until_state("61", |state| state["frame"] == 61).await;
    // A key frame of the media stands in for it (one every 120 frames).
    assert_eq!(
        (&sought["exact"], &sought["shown"]),
        (&json!(false), &json!(0)),
        "{sought}"
    );
    agent
        .until_state("61 itself", |state| state["shown"] == 61)
        .await;
    agent
        .result("transport.proxies", Some(json!({ "on": true })))
        .await;
    agent
        .until_state("on", |state| {
            state["source"] == "proxy" && state["proxies"] == true
        })
        .await;
    // A proxy whose file is gone: the media itself is read, and the proxy is made again.
    agent
        .result("transport.load", Some(json!({ "media": "m2" })))
        .await;
    let got = project(&mut agent).await;
    let proxy = got["project"]["media"][0]["proxy"]["path"]
        .as_str()
        .unwrap()
        .to_owned();
    std::fs::remove_file(&proxy).unwrap();
    agent.take_notifications();
    let loaded = agent
        .result("transport.load", Some(json!({ "media": "m1" })))
        .await;
    assert_eq!(loaded["source"], "original", "{loaded}");
    let notes = agent.proxies_of(&["m1"]).await;
    assert_eq!(notes.last().unwrap()["state"], "done", "{notes:?}");
    // Made again while the media is loaded, it is read at once.
    agent
        .until_state("the proxy again", |state| state["source"] == "proxy")
        .await;
    assert!(Path::new(&proxy).is_file());
    agent
        .result(
            "transport.seek",
            Some(json!({ "frame": 97, "scrubbing": true })),
        )
        .await;
    let (sought, _) = agent.until_state("97", |state| state["frame"] == 97).await;
    assert_eq!(
        (&sought["exact"], &sought["shown"]),
        (&json!(true), &json!(97))
    );
    assert_ne!(
        logged_at_least(&engine, "transport_proxy_failed", 1).await,
        Vec::<Value>::new()
    );
}

#[tokio::test]
async fn a_proxy_damaged_inside_is_let_go_for_the_media_and_made_again() {
    let dir = project_dir();
    let Some((engine, mut agent)) = proxy_engine(
        dir.path(),
        &[support::Sample::Uhd],
        &["--proxy-readrate", "1"],
    )
    .await
    else {
        return;
    };
    agent.proxies_of(&["m1"]).await;
    let got = project(&mut agent).await;
    let path = got["project"]["media"][0]["proxy"]["path"]
        .as_str()
        .unwrap()
        .to_owned();
    // Its middle third zeroed: it opens, and the frames there cannot be decoded.
    let mut bytes = std::fs::read(&path).unwrap();
    let third = bytes.len() / 3;
    bytes[third..2 * third].fill(0);
    std::fs::write(&path, &bytes).unwrap();
    let loaded = agent
        .result("transport.load", Some(json!({ "media": "m1" })))
        .await;
    assert_eq!(loaded["source"], "proxy", "{loaded}");
    agent.take_notifications();
    agent
        .result("transport.seek", Some(json!({ "frame": 90 })))
        .await;
    // The media itself shows the frame, and the proxy is made again (slowly: as the media plays).
    let (sought, _) = agent
        .until_state("90 of the media", |state| {
            state["shown"] == 90 && state["source"] == "original"
        })
        .await;
    assert_eq!(sought["exact"], true, "{sought}");
    // Changes of the project meanwhile do not have it read again.
    for name in ["a", "b", "c", "d", "e"] {
        agent
            .apply(json!([{ "op": "add_timeline", "timeline": {
                "id": "", "name": name, "rate": { "num": 25, "den": 1 }, "tracks": [],
            } }]))
            .await;
    }
    let notes = agent.proxies_of(&["m1"]).await;
    assert_eq!(notes.last().unwrap()["state"], "done", "{notes:?}");
    agent
        .until_state("the proxy again", |state| state["source"] == "proxy")
        .await;
    let failures = logged_at_least(&engine, "transport_proxy_failed", 1).await;
    assert_eq!(failures[0]["reason"], "decode");
    let opened = logged_at_least(&engine, "transport_source", 2).await;
    // Once what came before has been written: no more of either.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let failures = logged(&engine, "transport_proxy_failed");
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(
        logged(&engine, "transport_source"),
        opened,
        "to the media, and back"
    );
    assert_eq!(opened.len(), 2, "to the media, and back: {opened:?}");
    // The new one is whole.
    agent
        .result(
            "transport.seek",
            Some(json!({ "frame": 100, "scrubbing": true })),
        )
        .await;
    let (sought, _) = agent
        .until_state("100", |state| state["frame"] == 100)
        .await;
    assert_eq!(
        (&sought["exact"], &sought["shown"]),
        (&json!(true), &json!(100))
    );
}

#[tokio::test]
async fn an_ffmpeg_that_writes_no_frame_is_stopped() {
    let dir = project_dir();
    let Some((_engine, mut agent)) = proxy_engine(
        dir.path(),
        &[support::Sample::Uhd],
        &[
            "--proxy-readrate",
            "0.001",
            "--proxy-watchdog",
            "2",
            "--proxy-encoder",
            "prores",
        ],
    )
    .await
    else {
        return;
    };
    let started = Instant::now();
    let notes = agent.proxies_of(&["m1"]).await;
    let failed = notes.last().unwrap();
    assert_eq!(failed["state"], "failed", "{notes:?}");
    assert!(
        failed["message"]
            .as_str()
            .unwrap()
            .contains("wrote no frame for 2 s"),
        "{failed}"
    );
    assert!(started.elapsed() < Duration::from_secs(20));
    // Not taken for a failure of the file: nothing is remembered, and no .part is left.
    assert_eq!(proxy_files(dir.path()), Vec::<String>::new());
}

#[tokio::test]
async fn a_proxy_that_could_not_be_made_is_not_tried_again_until_asked() {
    let dir = project_dir();
    let Some(made) = support::media(support::Sample::Fhd) else {
        return;
    };
    let path = made.path.to_str().unwrap();
    let digest = {
        let mut engine = engine(dir.path(), &["--no-proxies", "--sound", "null"]).await;
        let mut agent = engine.client("agent").await;
        agent
            .result("media.import", Some(json!({ "paths": [path, path] })))
            .await;
        let got = project(&mut agent).await;
        drop(agent);
        drop(engine.child.stdin.take());
        assert_eq!(engine.ended().await, 0);
        got["project"]["media"][0]["identity"]["digest"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    // Remembered as a failure, beside a file that no media names.
    let proxies = dir.path().join("cache").join("proxies");
    std::fs::create_dir_all(&proxies).unwrap();
    std::fs::write(
        proxies.join("failed.json"),
        serde_json::to_string(&json!({ digest.clone(): "no encoder made it" })).unwrap(),
    )
    .unwrap();
    std::fs::write(proxies.join("m7-0123456789ab.mov"), b"of media long gone").unwrap();
    let mut args = vec!["--sound", "null"];
    let ffmpeg = build_ffmpeg();
    if let Some(ffmpeg) = &ffmpeg {
        args.extend(["--proxy-ffmpeg", ffmpeg.as_str()]);
    }
    let engine = engine(dir.path(), &args).await;
    let deadline = Instant::now() + WAIT;
    while logged(&engine, "proxy_skipped").len() < 2 {
        assert!(Instant::now() < deadline, "{}", engine.stderr());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // The file that no media names is deleted; what failed is kept.
    assert_eq!(proxy_files(dir.path()), ["failed.json"]);
    let mut agent = engine.client("agent").await;
    let got = project(&mut agent).await;
    assert_eq!(got["project"]["media"][0].get("proxy"), None);
    // Asked for, it is made; the other media of the same file is given the same proxy.
    agent
        .result("media.make_proxy", Some(json!({ "media": "m1" })))
        .await;
    agent.proxies_of(&["m1"]).await;
    agent
        .result("media.make_proxy", Some(json!({ "media": "m2" })))
        .await;
    let notes = agent.proxies_of(&["m2"]).await;
    assert_eq!(notes.last().unwrap()["state"], "done", "{notes:?}");
    let got = project(&mut agent).await;
    let media = &got["project"]["media"];
    assert_eq!(media[0]["proxy"], media[1]["proxy"], "{media}");
    let failed = std::fs::read_to_string(proxies.join("failed.json")).unwrap();
    assert!(!failed.contains(&digest), "{failed}");
    let error = agent
        .error("media.make_proxy", Some(json!({ "media": "m9" })))
        .await;
    assert_eq!(error["data"]["reason"], "missing");
    let mut files = proxy_files(dir.path());
    files.sort();
    assert_eq!(files.len(), 2, "one proxy for the two: {files:?}");
}

#[cfg(windows)]
#[tokio::test]
async fn the_ffmpeg_of_an_engine_that_is_killed_ends_with_it() {
    let dir = project_dir();
    let Some((mut engine, mut agent)) = proxy_engine(
        dir.path(),
        &[support::Sample::Uhd],
        &["--proxy-readrate", "0.5"],
    )
    .await
    else {
        return;
    };
    let running = |pid: u64| {
        let output = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).contains(&format!("\"{pid}\""))
    };
    // The ffmpeg that makes the proxy, once it has made a frame. Others may have been started
    // before it and have failed at once: those of NVENC, where there is no card of NVIDIA (as on
    // the CI), whose pids are logged first.
    agent
        .proxies_until(|notes| {
            notes
                .iter()
                .any(|note| note["state"] == "running" && note["progress"].as_f64() > Some(0.0))
        })
        .await;
    let deadline = Instant::now() + WAIT;
    let pid = loop {
        if let Some(started) = logged(&engine, "proxy_encoding").pop() {
            let pid = started["pid"].as_u64().unwrap();
            if running(pid) {
                break pid;
            }
        }
        assert!(
            Instant::now() < deadline,
            "no ffmpeg runs: {}",
            engine.stderr()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    // Killed, as by the task manager: it has no time to stop its ffmpeg.
    engine.child.start_kill().unwrap();
    let _ = engine.child.wait().await;
    let deadline = Instant::now() + Duration::from_secs(5);
    while running(pid) {
        assert!(
            Instant::now() < deadline,
            "ffmpeg {pid} goes on: {:?}",
            logged(&engine, "proxy_job_object")
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn proxies_turned_off_while_playing_take_effect_at_the_next_seek() {
    let dir = project_dir();
    let Some((_engine, mut agent)) = proxy_engine(dir.path(), &[support::Sample::Uhd], &[]).await
    else {
        return;
    };
    agent.proxies_of(&["m1"]).await;
    agent
        .result("transport.load", Some(json!({ "media": "m1" })))
        .await;
    agent
        .result("transport.play", Some(json!({ "speed": 1 })))
        .await;
    agent
        .until_state("playing", |state| state["playing"] == true)
        .await;
    agent
        .result("transport.proxies", Some(json!({ "on": false })))
        .await;
    let (told, _) = agent
        .until_state("told", |state| state["proxies"] == false)
        .await;
    assert_eq!(
        (&told["playing"], &told["source"]),
        (&json!(true), &json!("proxy"))
    );
    agent
        .result("transport.seek", Some(json!({ "frame": 30 })))
        .await;
    // Told once it plays on from there, from the media itself: never paused meanwhile.
    let (sought, _) = agent
        .until_state("the media", |state| state["source"] == "original")
        .await;
    assert_eq!(sought["playing"], true, "{sought}");
    agent
        .until_state("on from 30", |state| {
            state["playing"] == true && state["frame"].as_i64().unwrap() > 35
        })
        .await;
    agent.result("transport.pause", None).await;
    agent
        .result("transport.proxies", Some(json!({ "on": true })))
        .await;
    let (on, _) = agent
        .until_state("the proxy", |state| state["source"] == "proxy")
        .await;
    assert_eq!(on["playing"], false);
}

/// The memory that a process holds (its working set on Windows, its resident set elsewhere), in
/// MiB, as the system tells it; none where it cannot be read.
fn memory_of(pid: u32) -> Option<f64> {
    if cfg!(windows) {
        let output = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&output.stdout);
        // "columbina-engine.exe","1234","Console","1","123,456 K"
        let field = text.trim().rsplit("\",\"").next()?;
        let kib: f64 = field
            .chars()
            .filter(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .ok()?;
        Some(kib / 1024.0)
    } else {
        let output = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        let kib: f64 = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .ok()?;
        Some(kib / 1024.0)
    }
}

/// How long the proxy of the 4K file takes to make, how fast a frame sought while scrubbing it is
/// shown, how fast it plays backward, and how much memory the engine holds meanwhile: run with
/// `--ignored --nocapture`. `COLUMBINA_MEASURE_FILE` names another file to measure in its place
/// (one of 4K HEVC from a camera, for the time a proxy takes).
#[tokio::test]
#[ignore = "a measurement, not a test"]
#[allow(clippy::too_many_lines)]
async fn measure_proxies() {
    let file = match std::env::var("COLUMBINA_MEASURE_FILE") {
        Ok(file) => std::path::PathBuf::from(file),
        Err(_) => match support::media(support::Sample::Uhd) {
            Some(made) => made.path,
            None => return,
        },
    };
    let dir = project_dir();
    let ffmpeg = build_ffmpeg().expect("the ffmpeg of the build");
    let engine = engine(dir.path(), &["--mute", "--proxy-ffmpeg", &ffmpeg]).await;
    let mut agent = engine.client("agent").await;
    let started = Instant::now();
    agent
        .result(
            "media.import",
            Some(json!({ "paths": [file.to_str().unwrap()] })),
        )
        .await;
    let mut most_memory: f64 = 0.0;
    let notes = agent.proxies_of(&["m1"]).await;
    eprintln!(
        "{}: the proxy, {:?} after the import; {:?}",
        file.display(),
        started.elapsed(),
        notes.last()
    );
    for made in logged(&engine, "proxy_made") {
        eprintln!("{made}");
    }
    let loaded = agent
        .result("transport.load", Some(json!({ "media": "m1" })))
        .await;
    eprintln!("{loaded}");
    let last = loaded["last"].as_i64().unwrap();
    most_memory = most_memory.max(memory_of(engine.pid).unwrap_or(0.0));
    for (source, on) in [("proxy", true), ("original", false)] {
        agent
            .result("transport.proxies", Some(json!({ "on": on })))
            .await;
        agent
            .until_state(source, |state| state["source"] == source)
            .await;
        // Scrubbing: a seek every 1/30 s.
        let frames = numbers(9, 60, last + 1);
        let times = scrub(&mut agent, &frames).await;
        let exact = times
            .iter()
            .filter(|(_, state)| state["exact"] == true)
            .count();
        let (median, most) =
            median_and_most(&times.iter().map(|(time, _)| *time).collect::<Vec<_>>());
        most_memory = most_memory.max(memory_of(engine.pid).unwrap_or(0.0));
        eprintln!(
            "{source}: scrubbing, the first state of each seek after {median:?} (median), \
             {most:?} at most; {exact} of {} exact at once",
            times.len()
        );
        // Backward at -1 for 3 s, from the end.
        agent
            .result("transport.seek", Some(json!({ "frame": last })))
            .await;
        agent
            .until_state("the end", |state| state["shown"] == last)
            .await;
        agent
            .result("transport.play", Some(json!({ "speed": -1 })))
            .await;
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(250)).await;
            most_memory = most_memory.max(memory_of(engine.pid).unwrap_or(0.0));
        }
        agent.result("transport.pause", None).await;
        agent
            .until_state("paused", |state| state["playing"] == false)
            .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        if let Some(played) = logged(&engine, "transport_played").pop() {
            let presented = played["presented"].as_f64().unwrap_or(0.0);
            let seconds = played["seconds"].as_f64().unwrap_or(1.0);
            eprintln!(
                "{source}: backward at -1, {presented} frames in {seconds:.2} s: {:.1} a second",
                presented / seconds
            );
        }
        // Forward at 1, for 3 s, from the start.
        agent
            .result("transport.seek", Some(json!({ "frame": 0 })))
            .await;
        agent
            .result("transport.play", Some(json!({ "speed": 1 })))
            .await;
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(250)).await;
            most_memory = most_memory.max(memory_of(engine.pid).unwrap_or(0.0));
        }
        agent.result("transport.pause", None).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        if let Some(played) = logged(&engine, "transport_played").pop() {
            eprintln!("{source}: forward at 1: {played}");
        }
        eprintln!(
            "{source}: the engine held at most {most_memory:.0} MiB so far (its working set)"
        );
    }
    eprintln!(
        "the engine held at most {most_memory:.0} MiB (its working set), and {:.0} MiB at the end",
        memory_of(engine.pid).unwrap_or(0.0)
    );
}
