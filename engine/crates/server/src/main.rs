//! `columbina-engine`: the engine of a project of the studio, the only program that writes it.
//!
//! ```text
//! columbina-engine --project <DIR> [--headless] [--idle-seconds <N>]
//! ```
//!
//! The engine takes the lock of the project, opens it, and listens on a local socket (see
//! [`address`]). Its first line on stdout is `{"listening": "<address>", "pid": …,
//! "protocol": 1}`, written once it listens; nothing else is ever written to stdout. What it
//! logs goes to stderr, a line of JSON at a time, from a thread of its own (see [`log`](mod@log)), so
//! that the engine never waits for whoever reads stderr, or for no one to. The desktop and the
//! agent connect to it and speak JSON-RPC 2.0 a line at a time, as `PROTOCOL.md` at the root of
//! the workspace tells.
//!
//! An engine that is not `--headless` belongs to the program that started it, the desktop, and
//! ends when its stdin is closed, as it is when that program ends, however it ends; one whose
//! stdin is closed or empty from the start ends at once. A headless engine does not read stdin;
//! it ends once no client has been connected for `--idle-seconds` (30 unless given), counted
//! from when it starts and from when the last connection closes, or when the desktop asks it
//! to yield the project (`engine.yield`). Either ends on Ctrl+C too, on
//! SIGTERM and SIGHUP on Unix, and on Windows when its console is closed or the user logs off or
//! the system shuts down. As it ends it stops taking connections, closes those that are open,
//! saves the project, and lets the lock go.
//!
//! It ends with 0 when it was asked to; 1 when it stopped because of an error after it started
//! listening, as when the last save failed; 2 when it could not start, with `{"error": "…"}` on
//! stderr; and 3 when the project is open in another engine, with `{"held": {"pid": …,
//! "address": "…"}}` on stderr, or `{"contended": true}` if that engine has not yet said where it
//! listens.

mod address;
mod audio;
mod conn;
mod log;
mod names;
mod proxy;
mod queue;
mod rpc;
mod session;
mod timer;
mod transport;
mod view;

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use columbina_model::History;
use columbina_store::{Lock, LockError, Saver, Store};
use serde_json::json;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::OwnedMutexGuard;

use crate::address::Address;
use crate::log::log;
use crate::queue::Limits;
use crate::session::{Engine, State};

const USAGE: &str = "usage: columbina-engine --project <DIR> [--headless] [--idle-seconds <N>]";

/// How long a headless engine waits with no one connected before it ends, unless it is told.
const IDLE_SECONDS: u64 = 30;

/// How much may wait to be sent on a connection before it is closed, unless the tests say
/// otherwise.
const QUEUE_LIMITS: Limits = Limits {
    messages: 4096,
    bytes: 128 * 1024 * 1024,
};

/// How long the engine waits for its connections to close as it ends.
const CLOSING: Duration = Duration::from_secs(2);

/// How long an engine that finds its name taken, by no one who answers, tries again to listen.
const NAME_TAKEN: Duration = Duration::from_secs(2);

/// How long the log is waited for as the engine ends.
const LAST_LOG: Duration = Duration::from_millis(500);

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
    /// Not in the usage (`--queue-capacity`, `--queue-bytes`): for the tests, which make a
    /// client that does not read fill its queue.
    limits: Limits,
    /// Not in the usage (`--read-gate <FILE>`): for the tests, which hold `media.import` before
    /// it reads each file until FILE is there, to see the engine answer meanwhile.
    read_gate: Option<PathBuf>,
    /// Not in the usage, for the tests: what plays the sound (`--sound
    /// device|null|stall|fail-after=<ms>`, see `audio::Kind`; `--mute` plays silence on the
    /// device, which is the clock all the same), and `--fail-card-at <frame>`, from which frame
    /// on the first frame on the graphics card fails to be copied.
    setup: transport::Setup,
    /// Whether proxies are made (not by a headless engine, nor with `--no-proxies`, which the
    /// tests give where they would get in the way), and, not in the usage, the `ffmpeg` that
    /// makes them (`--proxy-ffmpeg <FILE>`, an absolute path) and, for the tests,
    /// `--proxy-readrate <R>`, `--proxy-encoder nvenc|prores` and `--proxy-watchdog <S>`.
    proxies: proxy::Setup,
}

impl Options {
    fn parse(mut args: impl Iterator<Item = OsString>) -> Result<Options, String> {
        let mut project = None;
        let mut headless = false;
        let mut idle = IDLE_SECONDS;
        let mut limits = QUEUE_LIMITS;
        let mut read_gate = None;
        let mut setup = transport::Setup {
            sound: audio::Kind::Device { mute: false },
            fail_card_at: None,
        };
        let mut proxies = proxy::Setup::default();
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--project") => {
                    let dir = args.next().ok_or("--project needs a directory")?;
                    project = Some(PathBuf::from(dir));
                }
                Some("--headless") => headless = true,
                Some("--idle-seconds") => idle = number(args.next().as_ref(), "--idle-seconds")?,
                Some("--queue-capacity") => {
                    limits.messages = above_0(args.next().as_ref(), "--queue-capacity")?;
                }
                Some("--queue-bytes") => {
                    limits.bytes = above_0(args.next().as_ref(), "--queue-bytes")?;
                }
                Some("--read-gate") => {
                    let file = args.next().ok_or("--read-gate needs a file")?;
                    read_gate = Some(PathBuf::from(file));
                }
                Some("--mute") => {
                    if let audio::Kind::Device { mute } = &mut setup.sound {
                        *mute = true;
                    }
                }
                Some("--sound") => {
                    let kind = args.next().ok_or("--sound needs an output")?;
                    let mute = setup.sound == audio::Kind::Device { mute: true };
                    setup.sound = audio::Kind::parse(&kind.to_string_lossy())?;
                    if let (audio::Kind::Device { mute: muted }, true) = (&mut setup.sound, mute) {
                        *muted = true;
                    }
                }
                Some("--fail-card-at") => {
                    setup.fail_card_at = Some(number(args.next().as_ref(), "--fail-card-at")?);
                }
                Some("--no-proxies") => proxies.make = false,
                Some("--proxy-ffmpeg") => {
                    let file = PathBuf::from(args.next().ok_or("--proxy-ffmpeg needs a file")?);
                    if !file.is_absolute() {
                        return Err(format!(
                            "--proxy-ffmpeg takes an absolute path, and {} is not one",
                            file.display()
                        ));
                    }
                    proxies.ffmpeg = Some(file);
                }
                Some("--proxy-readrate") => {
                    let rate = args.next().ok_or("--proxy-readrate needs a rate")?;
                    let rate = rate.to_string_lossy().into_owned();
                    if !rate.parse::<f64>().is_ok_and(|rate| rate > 0.0) {
                        return Err(format!("--proxy-readrate takes a rate above 0, not {rate}"));
                    }
                    proxies.readrate = Some(rate);
                }
                Some("--proxy-encoder") => {
                    let encoder = args.next().ok_or("--proxy-encoder needs an encoder")?;
                    proxies.encoder = Some(proxy::Encoder::parse(&encoder.to_string_lossy())?);
                }
                Some("--proxy-watchdog") => {
                    let seconds: u64 = number(args.next().as_ref(), "--proxy-watchdog")?;
                    proxies.watchdog = Duration::from_secs(seconds.max(1));
                }
                _ => {
                    let arg = arg.to_string_lossy();
                    return Err(format!("{arg} is not an argument of the engine; {USAGE}"));
                }
            }
        }
        let project = project.ok_or_else(|| format!("--project is needed; {USAGE}"))?;
        // A headless engine has no transport to use them.
        proxies.make &= !headless;
        Ok(Options {
            project,
            headless,
            idle: Duration::from_secs(idle),
            limits,
            read_gate,
            setup,
            proxies,
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

/// The number above 0 that follows an argument.
fn above_0(value: Option<&OsString>, arg: &str) -> Result<usize, String> {
    match number(value, arg)? {
        0 => Err(format!("{arg} needs a number above 0")),
        number => Ok(number),
    }
}

/// Write a line of JSON to stderr and end the process, waiting for stderr no more than a
/// moment. Nothing is dropped: whatever must be let go, as the lock, must be let go before.
fn fail(code: i32, line: &serde_json::Value) -> ! {
    log::line(line);
    exit(code)
}

/// End the process, once what was logged is written or a moment has passed.
fn exit(code: i32) -> ! {
    log::flush(LAST_LOG);
    std::process::exit(code)
}

fn main() {
    let options = Options::parse(std::env::args_os().skip(1))
        .unwrap_or_else(|message| fail(EXIT_CANNOT_START, &json!({ "error": message })));
    if !options.headless {
        // Before any window is made: rectangles of the desktop are in device pixels, as they
        // are for a process aware of the DPI of each monitor.
        columbina_view::prepare_process();
    }
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
            // While the lock is held: once it is let go, the socket may be the next engine's.
            address.remove();
            drop(lock);
            exit(code)
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
    // A proxy that is not where the engine keeps them is not taken: the engine reads, and
    // deletes, only its own.
    let project = proxy::owned_only(project, &proxy::proxy_dir(dir));
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
        options.limits,
        options.read_gate.clone(),
        options.headless,
        options.setup,
        &options.proxies,
    );
    let ending = runtime.block_on(serve(options, address, engine, save_errors));
    // The runtime is never shut down: the process ends with `exit`, and a shutdown would wait
    // for the thread that reads stdin, which may never end.
    std::mem::forget(runtime);
    ending
}

/// Listen at the address. Where the name is taken, ask who has it: an engine of this project
/// that is ending may still have it, and one that has closed its connections lets it go in a
/// moment, so the engine tries again for a while when no one answers.
async fn listen(
    address: &Address,
    project: &str,
) -> Result<interprocess::local_socket::tokio::Listener, Ending> {
    let deadline = Instant::now() + NAME_TAKEN;
    loop {
        match address.listen() {
            Ok(listener) => return Ok(listener),
            Err(error) if address::taken(&error) => match address::ask(&address.text).await {
                Some((other, pid)) if other == project => {
                    return Err(Ending::Held {
                        pid,
                        address: address.text.clone(),
                    });
                }
                None if Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                _ => {
                    return Err(Ending::Failed(format!(
                        "cannot listen at {}: another program listens there",
                        address.text
                    )));
                }
            },
            Err(error) => {
                return Err(Ending::Failed(format!(
                    "cannot listen at {}: {error}",
                    address.text
                )));
            }
        }
    }
}

/// Listen, say where, and serve the clients until the engine is stopped.
async fn serve(
    options: &Options,
    address: &Address,
    engine: Engine,
    mut save_errors: UnboundedReceiver<String>,
) -> Ending {
    let listener = match listen(address, &engine.project).await {
        Ok(listener) => listener,
        Err(ending) => return ending,
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
            let _ = stop.send(EXIT_FAILED);
            let reason = "connections cannot be taken";
            log(
                "stopping",
                &json!({ "reason": reason, "message": error.to_string() }),
            );
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
    tokio::spawn({
        let yielded = Arc::clone(&engine.yielded);
        let stop = stop.clone();
        async move {
            yielded.notified().await;
            let _ = stop.send(0);
            log(
                "stopping",
                &json!({ "reason": "the project was yielded to the desktop" }),
            );
        }
    });
    if options.headless {
        tokio::spawn(idle(Arc::clone(&engine), options.idle, stop));
    } else {
        tokio::spawn(orphaned(stop));
    }

    let code = stopping.recv().await.unwrap_or(0);
    // No more connections, and those that are open are closed, so that the name is free
    // before the lock is let go.
    server.abort();
    let _ = server.await;
    engine.close_all();
    let mut presence = engine.presence.subscribe();
    let closed = presence.wait_for(|presence| presence.connected == 0);
    let _ = tokio::time::timeout(CLOSING, closed).await;
    // Before the state is locked for good: the maker of proxies tells what it did with it.
    let proxies = Arc::clone(&engine);
    let _ = tokio::task::spawn_blocking(move || proxies.stop_proxies()).await;
    let state = Arc::clone(&engine.state).lock_owned().await;
    Ending::Stopped { code, state }
}

/// Stop the engine when it is interrupted or the system asks it to end.
async fn interrupted(stop: UnboundedSender<i32>) {
    let ctrl_c = async {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    };
    let why = tokio::select! {
        () = ctrl_c => "interrupted",
        why = asked_to_end() => why,
    };
    let _ = stop.send(0);
    log("stopping", &json!({ "reason": why }));
}

#[cfg(unix)]
async fn asked_to_end() -> &'static str {
    use tokio::signal::unix::{signal, SignalKind};
    let wait = |kind| async move {
        match signal(kind) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending().await,
        }
    };
    tokio::select! {
        () = wait(SignalKind::terminate()) => "terminated",
        () = wait(SignalKind::hangup()) => "hung up",
    }
}

/// On Windows, the console is closed, the user logs off or the system shuts down: the handler
/// of tokio does not return, so that the process is not ended before the engine has saved.
#[cfg(windows)]
async fn asked_to_end() -> &'static str {
    use tokio::signal::windows::{ctrl_break, ctrl_close, ctrl_logoff, ctrl_shutdown};
    macro_rules! wait {
        ($signal:expr) => {
            async {
                match $signal {
                    Ok(mut signal) => {
                        signal.recv().await;
                    }
                    Err(_) => std::future::pending::<()>().await,
                }
            }
        };
    }
    tokio::select! {
        () = wait!(ctrl_break()) => "interrupted",
        () = wait!(ctrl_close()) => "the console was closed",
        () = wait!(ctrl_logoff()) => "the user logs off",
        () = wait!(ctrl_shutdown()) => "the system shuts down",
    }
}

/// Stop an engine that is not headless when its stdin is closed: the program that started it
/// has ended, or wants it to end.
async fn orphaned(stop: UnboundedSender<i32>) {
    use tokio::io::AsyncReadExt;
    let mut stdin = tokio::io::stdin();
    let mut buffer = [0; 256];
    while let Ok(1..) = stdin.read(&mut buffer).await {}
    let _ = stop.send(0);
    log("stopping", &json!({ "reason": "stdin was closed" }));
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
    let _ = stop.send(0);
    let seconds = idle.as_secs();
    log(
        "stopping",
        &json!({ "reason": "no one was connected", "seconds": seconds }),
    );
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

    /// An absolute path, on this system.
    const FFMPEG: &str = if cfg!(windows) {
        r"C:\ff\ffmpeg.exe"
    } else {
        "/ff/ffmpeg"
    };

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
                limits: Limits {
                    messages: 4096,
                    bytes: 128 * 1024 * 1024,
                },
                read_gate: None,
                setup: transport::Setup {
                    sound: audio::Kind::Device { mute: false },
                    fail_card_at: None,
                },
                proxies: proxy::Setup::default(),
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
                "8",
                "--queue-bytes",
                "1000",
                "--read-gate",
                "g",
                "--mute",
                "--fail-card-at",
                "30",
                "--proxy-ffmpeg",
                FFMPEG,
                "--proxy-readrate",
                "0.5",
                "--proxy-encoder",
                "prores",
                "--proxy-watchdog",
                "2",
            ])
            .unwrap(),
            Options {
                project: "a b".into(),
                headless: true,
                idle: Duration::from_secs(2),
                limits: Limits {
                    messages: 8,
                    bytes: 1000,
                },
                read_gate: Some("g".into()),
                setup: transport::Setup {
                    sound: audio::Kind::Device { mute: true },
                    fail_card_at: Some(30),
                },
                // A headless engine makes none.
                proxies: proxy::Setup {
                    make: false,
                    ffmpeg: Some(FFMPEG.into()),
                    readrate: Some("0.5".into()),
                    encoder: Some(proxy::Encoder::Prores),
                    watchdog: Duration::from_secs(2),
                },
            }
        );
        assert!(
            !parse(&["--project", "p", "--no-proxies"])
                .unwrap()
                .proxies
                .make
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
            &["--project", "p", "--queue-bytes", "0"],
            &["--project", "p", "--colour"],
            &["--project", "p", "--sound"],
            &["--project", "p", "--sound", "loud"],
            &["--project", "p", "--fail-card-at", "x"],
            &["--project", "p", "--proxy-ffmpeg"],
            &["--project", "p", "--proxy-ffmpeg", "ffmpeg"],
            &["--project", "p", "--proxy-readrate", "0"],
            &["--project", "p", "--proxy-encoder", "x264"],
            &["--project", "p", "--proxy-watchdog", "x"],
        ] {
            assert!(parse(args).is_err(), "{args:?}");
        }
        let sound = |args: &[&str]| parse(args).unwrap().setup.sound;
        assert_eq!(
            sound(&["--project", "p", "--sound", "null"]),
            audio::Kind::Null
        );
        assert_eq!(
            sound(&["--project", "p", "--mute", "--sound", "device"]),
            audio::Kind::Device { mute: true }
        );
        assert_eq!(
            sound(&["--project", "p", "--sound", "fail-after=250"]),
            audio::Kind::FailAfter(Duration::from_millis(250))
        );
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
