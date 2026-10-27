//! The connections of the engine: taking them, reading requests from each, and writing to each
//! what is queued for it.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use interprocess::local_socket::tokio::{Listener, Stream};
use interprocess::local_socket::traits::tokio::Listener as _;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::watch;

use crate::log::log;
use crate::queue::{self, Refused};
use crate::rpc::{self, Fault, Frame, Lines, MAX_LINE};
use crate::session::{self, Connection, Engine};

/// How many failures to take a connection in a row make the engine give up.
const MOST_ACCEPT_FAILURES: u32 = 20;

/// How much is read from a connection at a time.
const READ_SIZE: usize = 64 * 1024;

/// How much of what is queued for a connection is written at once, at most, unless a single
/// message is longer.
const WRITE_BATCH: usize = 256 * 1024;

/// How long what was queued for a client that has stopped sending is still written, before its
/// connection is closed.
const DRAIN: Duration = Duration::from_secs(5);

/// Take connections, each served by tasks of its own, until taking them fails
/// [`MOST_ACCEPT_FAILURES`] times in a row. Gives the last failure.
pub async fn serve(listener: Listener, engine: Arc<Engine>) -> io::Error {
    let mut failures = 0;
    loop {
        match listener.accept().await {
            Ok(stream) => {
                failures = 0;
                tokio::spawn(connection(Arc::clone(&engine), stream));
            }
            Err(error) => {
                failures += 1;
                log(
                    "accept_failed",
                    &json!({
                        "message": error.to_string(),
                        "os_error": error.raw_os_error(),
                        "in_a_row": failures,
                    }),
                );
                if failures >= MOST_ACCEPT_FAILURES {
                    return error;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    }
}

/// How the reading of a connection ended.
enum End {
    /// The client closed it, or it failed.
    Closed,
    /// It was stopped by the engine: its queue was full when a notification was queued, or
    /// the engine is ending.
    Stopped,
    /// Its queue was full when an answer was to be queued.
    Full,
}

/// Serve a connection until it ends.
async fn connection(engine: Arc<Engine>, stream: Stream) {
    let stream = Arc::new(stream);
    let (stop, stopped) = watch::channel(false);
    let Some(id) = engine.arrive(stop.clone()) else {
        close(stream);
        return;
    };
    let (queue, queued) = queue::queue(engine.limits);
    let writer = tokio::spawn(write(Arc::clone(&stream), queued, stopped.clone()));
    let mut connection = Connection::new(id, queue, stop.clone());
    let end = read(&engine, &mut connection, &stream, stopped).await;
    if let End::Full = end {
        session::log_kicked(id, connection.kind(), engine.limits);
    }
    // No more is queued for it: once what is queued is written, the writer ends.
    engine.leave(id).await;
    drop(connection);
    let mut writer = writer;
    if !matches!(end, End::Closed) || tokio::time::timeout(DRAIN, &mut writer).await.is_err() {
        let _ = stop.send(true);
        let _ = writer.await;
    }
    close(stream);
}

/// Close a stream at once, whether or not the client has read all that was written to it.
fn close(stream: Arc<Stream>) {
    // Dropped with writes that the client has not read, a pipe of Windows is handed to a
    // thread that waits until the client reads them, which a client that has stopped reading
    // never does, so that the pipe would never close and its name never be free.
    #[cfg(windows)]
    #[allow(irrefutable_let_patterns)]
    if let Stream::NamedPipe(pipe) = &*stream {
        pipe.inner().assume_flushed();
    }
    drop(stream);
}

/// Read requests from a connection and answer them, in the order in which they come, until it
/// ends.
async fn read(
    engine: &Engine,
    connection: &mut Connection,
    stream: &Stream,
    mut stopped: watch::Receiver<bool>,
) -> End {
    let mut reader = stream;
    let mut chunk = vec![0; READ_SIZE];
    let mut lines = Lines::new(MAX_LINE);
    let mut frames = Vec::new();
    loop {
        let read = tokio::select! {
            read = reader.read(&mut chunk) => read,
            _ = stopped.wait_for(|stop| *stop) => return End::Stopped,
        };
        let count = match read {
            Ok(0) | Err(_) => return End::Closed,
            Ok(count) => count,
        };
        lines.feed(&chunk[..count], &mut frames);
        for frame in frames.drain(..) {
            if *stopped.borrow() {
                return End::Stopped;
            }
            let handled = match frame {
                Frame::Line(line) => session::handle(engine, connection, &line).await,
                Frame::TooLong => connection.queue(rpc::error_line(
                    None,
                    &Fault::new(rpc::INVALID_REQUEST, "request line exceeds 16 MiB"),
                )),
            };
            match handled {
                Ok(()) => {}
                Err(_) if *stopped.borrow() => return End::Stopped,
                Err(Refused::Full) => return End::Full,
                Err(Refused::Closed) => return End::Closed,
            }
        }
    }
}

/// Write what is queued for a connection, as much of it at once as there is, until the queue
/// ends or the connection is stopped.
async fn write(
    stream: Arc<Stream>,
    mut queued: queue::Receiver,
    mut stopped: watch::Receiver<bool>,
) {
    let mut writer = &*stream;
    let mut batch = Vec::new();
    loop {
        let first = tokio::select! {
            biased;
            _ = stopped.wait_for(|stop| *stop) => return,
            line = queued.recv() => line,
        };
        let Some(first) = first else {
            return;
        };
        batch.clear();
        batch.extend_from_slice(&first);
        let mut lines = 1;
        while batch.len() < WRITE_BATCH {
            match queued.try_recv() {
                Some(line) => {
                    batch.extend_from_slice(&line);
                    lines += 1;
                }
                None => break,
            }
        }
        let written = tokio::select! {
            biased;
            _ = stopped.wait_for(|stop| *stop) => return,
            written = writer.write_all(&batch) => written,
        };
        if written.is_err() {
            return;
        }
        queued.wrote(lines);
        if batch.capacity() > 4 * WRITE_BATCH {
            // Let go of what a long message took.
            batch = Vec::new();
        }
    }
}
