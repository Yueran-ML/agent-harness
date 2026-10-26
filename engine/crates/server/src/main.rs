//! `columbina-engine`: the engine of a project of the studio, the only program that writes it.
//!
//! ```text
//! columbina-engine --project <DIR> [--headless] [--idle-seconds <N>]
//! ```
//!
//! The engine takes the lock of the project, opens it, and listens on a local socket (see
//! [`address`]). Its first line on stdout is `{"listening": "<address>", "pid": …,
//! "protocol": 1}`, written once it listens; nothing else is ever written to stdout, and what it
//! logs goes to stderr, a line of JSON at a time. The desktop and the agent connect to it and
//! speak JSON-RPC 2.0 a line at a time, as `PROTOCOL.md` at the root of the workspace tells.
//!
//! An engine that is not `--headless` belongs to the program that started it, the desktop, and
//! ends when its stdin is closed, as it is when that program ends, however it ends. A headless
//! engine does not read stdin; it ends once no client has been connected for `--idle-seconds`
//! (30 unless given), counted from when it starts and from when the last connection closes.
//! Either ends on Ctrl+C too, and on SIGTERM on Unix. As it ends it stops taking connections,
//! saves the project, and lets the lock go.
//!
//! It ends with 0 when it was asked to; 1 when it stopped because of an error after it started
//! listening, as when the last save failed; 2 when it could not start, with `{"error": "…"}` on
//! stderr; and 3 when the project is open in another engine, with `{"held": {"pid": …,
//! "address": "…"}}` on stderr, or `{"contended": true}` if that engine has not yet said where it
//! listens.

mod address;
mod conn;
mod names;
mod rpc;
mod session;

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use columbina_model::History;
use columbina_store::{Lock, LockError, Saver, Store};
use serde_json::json;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::OwnedMutexGuard;

use crate::address::Address;
use crate::session::{Engine, State};

const USAGE: &str = "usage: columbina-engine --project <DIR> [--headless] [--idle-seconds <N>]";

/// How long a headless engine waits with no one connected before it ends, unless it is told.
const IDLE_SECONDS: u64 = 30;

/// How many messages may wait to be sent on a connection before it is closed, unless the tests
/// say otherwise.
const QUEUE_CAPACITY: usize = 4096;

/// The engine stopped because of an error after it started listening.
const EXIT_FAILED: i32 = 1;
/// The engine could not start.
const EXIT_CANNOT_START: i32 = 2;
/// The project is open in another engine.
const EXIT_HELD: i32 = 3;

/// What the engine was told on its command line.
#[derive(Debug, PartialEq, Eq)]
struct Options {
    project: PathBuf,
    headless: bool,
    idle: Duration,
    /// Not in the usage: for the tests, which make a client that does not read fill its queue.
    queue_capacity: usize,
}

impl Options {
    fn parse(mut args: impl Iterator<Item = OsString>) -> Result<Options, String> {
        let mut project = None;
        let mut headless = false;
        let mut idle = IDLE_SECONDS;
        let mut queue_capacity = QUEUE_CAPACITY;
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--project") => {
                    let dir = args.next().ok_or("--project needs a directory")?;
                    project = Some(PathBuf::from(dir));
                }
                Some("--headless") => headless = true,
                Some("--idle-seconds") => idle = number(args.next().as_ref(), "--idle-seconds")?,
                Some("--queue-capacity") => {
                    queue_capacity = number(args.next().as_ref(), "--queue-capacity")?;
                    if queue_capacity == 0 {
                        return Err("--queue-capacity needs a number above 0".into());
                    }
                }
                _ => {
                    let arg = arg.to_string_lossy();
                    return Err(format!("{arg} is not an argument of the engine; {USAGE}"));
                }
            }
        }
        let project = project.ok_or_else(|| format!("--project is needed; {USAGE}"))?;
        Ok(Options {
            project,
            headless,
            idle: Duration::from_secs(idle),
            queue_capacity,
        })
    }
}

/// The number that follows an argument.
fn number<T: std::str::FromStr>(value: Option<&OsString>, arg: &str) -> Result<T, String> {
    value
        .and_then(|value| value.to_str())
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| format!("{arg} needs a whole number"))
}

/// Write a line of JSON to stderr: what happened, and what more there is to say of it.
pub(crate) fn log(event: &str, fields: &serde_json::Value) {
    let mut line = serde_json::Map::new();
    line.insert("event".into(), event.into());
    if let Some(fields) = fields.as_object() {
        line.extend(fields.clone());
    }
    eprintln!("{}", serde_json::Value::Object(line));
}

/// Write a line of JSON to stderr and end the process. Nothing is dropped: whatever must be let
/// go, as the lock, must be let go before.
fn fail(code: i32, line: &serde_json::Value) -> ! {
    eprintln!("{line}");
    std::process::exit(code)
}

fn main() {
    let options = Options::parse(std::env::args_os().skip(1))
        .unwrap_or_else(|message| fail(EXIT_CANNOT_START, &json!({ "error": message })));
    let dir = project_dir(&options.project)
        .unwrap_or_else(|message| fail(EXIT_CANNOT_START, &json!({ "error": message })));
    let address = Address::of(&dir).unwrap_or_else(|error| {
        let message = format!("cannot name a socket for {}: {error}", dir.display());
        fail(EXIT_CANNOT_START, &json!({ "error": message }))
    });
    let lock = match Lock::acquire(&dir, &address.text) {
        Ok(lock) => lock,
        Err(LockError::Held { pid, address }) => fail(
            EXIT_HELD,
            &json!({ "held": { "pid": pid, "address": address } }),
        ),
        Err(LockError::Contended { .. }) => fail(EXIT_HELD, &json!({ "contended": true })),
        Err(error) => fail(EXIT_CANNOT_START, &json!({ "error": error.to_string() })),
    };

    match start(&options, &dir, &address) {
        Ending::Stopped {
            mut code,
            mut state,
        } => {
            // The state stays locked, so that nothing changes the project after this save.
            if let Some(saver) = state.saver.take() {
                if let Err(error) = saver.flush() {
                    log("save_failed", &json!({ "message": error.to_string() }));
                    code = EXIT_FAILED;
                }
                drop(saver);
            }
            drop(lock);
            address.remove();
            std::process::exit(code)
        }
        Ending::Held { pid, address } => {
            drop(lock);
            fail(
                EXIT_HELD,
                &json!({ "held": { "pid": pid, "address": address } }),
            )
        }
        Ending::Failed(message) => {
            drop(lock);
            fail(EXIT_CANNOT_START, &json!({ "error": message }))
        }
    }
}

/// How an engine that took the lock ended.
enum Ending {
    /// It was stopped, with the code to end with. What is saved and let go is left to `main`;
    /// the state is held locked, so that nothing changes it any more.
    Stopped {
        code: i32,
        state: OwnedMutexGuard<State>,
    },
    /// Another engine of the project still listens at the address, as one that is ending may.
    Held { pid: u32, address: String },
    /// It could not start.
    Failed(String),
}

/// Open the project of a directory whose lock is held, and serve it until the engine is
/// stopped.
fn start(options: &Options, dir: &Path, address: &Address) -> Ending {
    let (store, project) = match Store::open(dir) {
        Ok(opened) => opened,
        Err(error) => return Ending::Failed(error.to_string()),
    };
    let (report, save_errors) = mpsc::unbounded_channel();
    let saver = Saver::new(store, move |error| {
        let _ = report.send(error.to_string());
    });
    let saver = match saver {
        Ok(saver) => saver,
        Err(error) => return Ending::Failed(format!("cannot start to save: {error}")),
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => return Ending::Failed(format!("cannot start the runtime: {error}")),
    };
    let engine = Engine::new(
        History::new(project),
        saver,
        dir.to_string_lossy().into_owned(),
        options.queue_capacity,
    );
    let ending = runtime.block_on(serve(options, address, engine, save_errors));
    // The runtime is never shut down: the process ends with `exit`, and a shutdown would wait
    // for the thread that reads stdin, which may never end.
    std::mem::forget(runtime);
    ending
}

/// Listen, say where, and serve the clients until the engine is stopped.
async fn serve(
    options: &Options,
    address: &Address,
    engine: Engine,
    mut save_errors: UnboundedReceiver<String>,
) -> Ending {
    let listener = match address.listen() {
        Ok(listener) => listener,
        Err(error) if address::taken(&error) => {
            // An engine of this project that is ending may still listen there; one of another
            // project never does, since the name is made from the directory.
            return match address::ask(&address.text).await {
                Some((project, pid)) if project == engine.project => Ending::Held {
                    pid,
                    address: address.text.clone(),
                },
                _ => Ending::Failed(format!(
                    "cannot listen at {}: another program listens there",
                    address.text
                )),
            };
        }
        Err(error) => {
            return Ending::Failed(format!("cannot listen at {}: {error}", address.text));
        }
    };
    let engine = Arc::new(engine);
    let listening = json!({
        "listening": address.text,
        "pid": engine.pid,
        "protocol": rpc::PROTOCOL,
    });
    let mut stdout = std::io::stdout().lock();
    // If no one reads stdout, the engine is found through engine.lock all the same.
    let _ = writeln!(stdout, "{listening}").and_then(|()| stdout.flush());
    drop(stdout);

    let (stop, mut stopping) = mpsc::unbounded_channel();
    let server = tokio::spawn({
        let engine = Arc::clone(&engine);
        let stop = stop.clone();
        async move {
            let error = conn::serve(listener, engine).await;
            let reason = "connections cannot be taken";
            log(
                "stopping",
                &json!({ "reason": reason, "message": error.to_string() }),
            );
            let _ = stop.send(EXIT_FAILED);
        }
    });
    tokio::spawn({
        let engine = Arc::clone(&engine);
        async move {
            while let Some(message) = save_errors.recv().await {
                engine.report_save_error(&message).await;
            }
        }
    });
    tokio::spawn(interrupted(stop.clone()));
    if options.headless {
        tokio::spawn(idle(Arc::clone(&engine), options.idle, stop));
    } else {
        tokio::spawn(orphaned(stop));
    }

    let code = stopping.recv().await.unwrap_or(0);
    server.abort();
    let _ = server.await;
    let state = Arc::clone(&engine.state).lock_owned().await;
    Ending::Stopped { code, state }
}

/// Stop the engine on Ctrl+C, or on SIGTERM on Unix.
async fn interrupted(stop: UnboundedSender<i32>) {
    let ctrl_c = async {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    };
    tokio::select! {
        () = ctrl_c => {}
        () = terminated() => {}
    }
    log("stopping", &json!({ "reason": "interrupted" }));
    let _ = stop.send(0);
}

#[cfg(unix)]
async fn terminated() {
    use tokio::signal::unix::{signal, SignalKind};
    match signal(SignalKind::terminate()) {
        Ok(mut terminate) => {
            terminate.recv().await;
        }
        Err(_) => std::future::pending().await,
    }
}

#[cfg(not(unix))]
async fn terminated() {
    std::future::pending::<()>().await;
}

/// Stop an engine that is not headless when its stdin is closed: the program that started it
/// has ended, or wants it to end.
async fn orphaned(stop: UnboundedSender<i32>) {
    use tokio::io::AsyncReadExt;
    let mut stdin = tokio::io::stdin();
    let mut buffer = [0; 256];
    while let Ok(1..) = stdin.read(&mut buffer).await {}
    log("stopping", &json!({ "reason": "stdin was closed" }));
    let _ = stop.send(0);
}

/// Stop a headless engine once no one has been connected for `idle`.
async fn idle(engine: Arc<Engine>, idle: Duration, stop: UnboundedSender<i32>) {
    let mut presence = engine.presence.subscribe();
    loop {
        let Ok(arrivals) = presence
            .wait_for(|presence| presence.connected == 0)
            .await
            .map(|presence| presence.arrivals)
        else {
            return;
        };
        let came = presence.wait_for(|presence| presence.arrivals != arrivals);
        tokio::select! {
            () = tokio::time::sleep(idle) => break,
            _ = came => {}
        }
    }
    let seconds = idle.as_secs();
    log(
        "stopping",
        &json!({ "reason": "no one was connected", "seconds": seconds }),
    );
    let _ = stop.send(0);
}

/// The directory of the project, made if it is not there, written in full.
fn project_dir(given: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(given)
        .map_err(|error| format!("cannot make the directory {}: {error}", given.display()))?;
    let full = std::fs::canonicalize(given)
        .map_err(|error| format!("cannot find the directory {}: {error}", given.display()))?;
    Ok(plain(full))
}

/// A path that Windows writes as `\\?\C:\…` or `\\?\UNC\server\…`, written as `C:\…` or
/// `\\server\…`, as other programs write it.
fn plain(path: PathBuf) -> PathBuf {
    if cfg!(windows) {
        if let Some(text) = path.to_str() {
            if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
                return PathBuf::from(format!(r"\\{rest}"));
            }
            if let Some(rest) = text.strip_prefix(r"\\?\") {
                if rest.as_bytes().get(1) == Some(&b':') {
                    return PathBuf::from(rest);
                }
            }
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Options, String> {
        Options::parse(args.iter().map(OsString::from))
    }

    #[test]
    fn the_command_line_is_read() {
        assert_eq!(
            parse(&["--project", "p"]).unwrap(),
            Options {
                project: "p".into(),
                headless: false,
                idle: Duration::from_secs(30),
                queue_capacity: 4096,
            }
        );
        assert_eq!(
            parse(&[
                "--headless",
                "--idle-seconds",
                "2",
                "--project",
                "a b",
                "--queue-capacity",
                "8"
            ])
            .unwrap(),
            Options {
                project: "a b".into(),
                headless: true,
                idle: Duration::from_secs(2),
                queue_capacity: 8,
            }
        );
    }

    #[test]
    fn a_wrong_command_line_is_refused() {
        for args in [
            &[][..],
            &["--headless"],
            &["--project"],
            &["--project", "p", "--idle-seconds", "x"],
            &["--project", "p", "--idle-seconds", "-1"],
            &["--project", "p", "--queue-capacity", "0"],
            &["--project", "p", "--colour"],
        ] {
            assert!(parse(args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn a_path_is_written_as_other_programs_write_it() {
        if cfg!(windows) {
            assert_eq!(plain(r"\\?\F:\a b\c".into()), PathBuf::from(r"F:\a b\c"));
            assert_eq!(
                plain(r"\\?\UNC\server\share\p".into()),
                PathBuf::from(r"\\server\share\p")
            );
            assert_eq!(
                plain(r"\\?\Volume{1}\p".into()),
                PathBuf::from(r"\\?\Volume{1}\p")
            );
        }
        assert_eq!(plain("/a/b".into()), PathBuf::from("/a/b"));
    }
}
