//! The log of the engine: lines of JSON on stderr, written by a thread of their own.
//!
//! Nothing else of the engine ever waits for stderr. Whoever reads it may have ended, so that
//! writing fails, or may not read it, so that writing would block once the pipe is full: the
//! thread of the log ignores the first and is the only one to wait in the second. A line is
//! handed to it without waiting; if [`BACKLOG`] lines are waiting already, the line is let go
//! and counted, and the count is logged with the next line that is handed over.

use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::OnceLock;
use std::time::Duration;

/// How many lines may wait to be written before more are let go.
const BACKLOG: usize = 1024;

enum Entry {
    /// Lines to write, each with its newline.
    Lines(String),
    /// Say when what was handed over before has been written.
    Flush(SyncSender<()>),
}

struct Logger {
    entries: SyncSender<Entry>,
    /// How many lines were let go since one was last handed over.
    dropped: AtomicU64,
}

static LOGGER: OnceLock<Option<Logger>> = OnceLock::new();

/// The logger, started the first time it is needed. None if its thread could not be started.
fn logger() -> Option<&'static Logger> {
    LOGGER
        .get_or_init(|| {
            let (entries, received) = mpsc::sync_channel(BACKLOG);
            std::thread::Builder::new()
                .name("columbina-log".into())
                .spawn(move || write(&received))
                .ok()
                .map(|_| Logger {
                    entries,
                    dropped: AtomicU64::new(0),
                })
        })
        .as_ref()
}

fn write(received: &Receiver<Entry>) {
    let mut stderr = std::io::stderr();
    for entry in received {
        match entry {
            Entry::Lines(lines) => {
                let _ = stderr.write_all(lines.as_bytes());
            }
            Entry::Flush(done) => {
                let _ = done.send(());
            }
        }
    }
}

/// Hand a line of JSON to the log, without waiting.
pub fn line(value: &serde_json::Value) {
    let Some(logger) = logger() else {
        // No thread for the log: write the line here, as the last resort.
        let _ = writeln!(std::io::stderr(), "{value}");
        return;
    };
    let mut lines = String::new();
    let dropped = logger.dropped.swap(0, Ordering::Relaxed);
    if dropped > 0 {
        let note = serde_json::json!({ "event": "log_dropped", "lines": dropped });
        lines.push_str(&note.to_string());
        lines.push('\n');
    }
    lines.push_str(&value.to_string());
    lines.push('\n');
    match logger.entries.try_send(Entry::Lines(lines)) {
        Err(TrySendError::Full(_)) => {
            logger.dropped.fetch_add(dropped + 1, Ordering::Relaxed);
        }
        Ok(()) | Err(TrySendError::Disconnected(_)) => {}
    }
}

/// Log what happened, and what more there is to say of it.
pub fn log(event: &str, fields: &serde_json::Value) {
    let mut value = serde_json::Map::new();
    value.insert("event".into(), event.into());
    if let Some(fields) = fields.as_object() {
        value.extend(fields.clone());
    }
    line(&serde_json::Value::Object(value));
}

/// Wait until what was handed to the log has been written, for at most `most`.
pub fn flush(most: Duration) {
    let Some(logger) = logger() else {
        return;
    };
    let (done, written) = mpsc::sync_channel(1);
    if logger.entries.try_send(Entry::Flush(done)).is_ok() {
        let _ = written.recv_timeout(most);
    }
}
