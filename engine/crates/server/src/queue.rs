//! The queue of what is to be sent on a connection, held to a number of messages and a number
//! of bytes.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::sync::mpsc::{self, error::TrySendError};
use tokio::sync::watch;

use crate::rpc::Line;

/// How much may wait to be sent on a connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Messages.
    pub messages: usize,
    /// Bytes, unless the queue is empty: one message alone is always taken, however long.
    pub bytes: usize,
}

/// Why a message was not queued.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The queue holds as much as it may.
    Full,
    /// The connection is closing, and nothing more is written to it.
    Closed,
}

/// Where messages are queued. It is cloned for everyone who queues on the connection.
#[derive(Clone, Debug)]
pub struct Sender {
    queue: mpsc::Sender<Line>,
    bytes: Arc<AtomicUsize>,
    most_bytes: usize,
    /// How many messages were queued.
    pushed: Arc<AtomicU64>,
    /// How many messages the writer has written.
    written: watch::Receiver<u64>,
}

/// Where the writer of the connection takes them from.
#[derive(Debug)]
pub struct Receiver {
    queue: mpsc::Receiver<Line>,
    bytes: Arc<AtomicUsize>,
    written: watch::Sender<u64>,
}

/// A queue held to `limits`.
pub fn queue(limits: Limits) -> (Sender, Receiver) {
    let (sender, receiver) = mpsc::channel(limits.messages);
    let bytes = Arc::new(AtomicUsize::new(0));
    let (written, written_seen) = watch::channel(0);
    (
        Sender {
            queue: sender,
            bytes: Arc::clone(&bytes),
            most_bytes: limits.bytes,
            pushed: Arc::new(AtomicU64::new(0)),
            written: written_seen,
        },
        Receiver {
            queue: receiver,
            bytes,
            written,
        },
    )
}

impl Sender {
    /// Queue a message, without waiting.
    pub fn push(&self, line: Line) -> Result<(), Refused> {
        let length = line.len();
        let before = self.bytes.fetch_add(length, Ordering::SeqCst);
        if before > 0 && before + length > self.most_bytes {
            self.bytes.fetch_sub(length, Ordering::SeqCst);
            return Err(Refused::Full);
        }
        match self.queue.try_send(line) {
            Ok(()) => {
                self.pushed.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
            Err(error) => {
                self.bytes.fetch_sub(length, Ordering::SeqCst);
                Err(match error {
                    TrySendError::Full(_) => Refused::Full,
                    TrySendError::Closed(_) => Refused::Closed,
                })
            }
        }
    }
}

impl Sender {
    /// Wait until every message queued so far has been written, or the writer has ended.
    pub async fn written(&self) {
        let pushed = self.pushed.load(Ordering::SeqCst);
        let mut written = self.written.clone();
        let _ = written.wait_for(|&written| written >= pushed).await;
    }
}

impl Receiver {
    /// Say that `count` more messages were written.
    pub fn wrote(&self, count: u64) {
        self.written.send_modify(|written| *written += count);
    }

    /// The next message, waiting for it; none once no one can queue any more.
    pub async fn recv(&mut self) -> Option<Line> {
        let line = self.queue.recv().await?;
        self.bytes.fetch_sub(line.len(), Ordering::SeqCst);
        Some(line)
    }

    /// The next message, if one is queued.
    pub fn try_recv(&mut self) -> Option<Line> {
        let line = self.queue.try_recv().ok()?;
        self.bytes.fetch_sub(line.len(), Ordering::SeqCst);
        Some(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(length: usize) -> Line {
        vec![b'x'; length].into()
    }

    #[tokio::test]
    async fn a_queue_is_held_to_its_messages_and_its_bytes() {
        let (sender, mut receiver) = queue(Limits {
            messages: 3,
            bytes: 10,
        });
        // One message alone is taken however long it is.
        sender.push(line(50)).unwrap();
        assert_eq!(sender.push(line(1)), Err(Refused::Full));
        assert_eq!(receiver.recv().await.unwrap().len(), 50);
        sender.push(line(6)).unwrap();
        sender.push(line(4)).unwrap();
        assert_eq!(sender.push(line(1)), Err(Refused::Full));
        assert_eq!(receiver.try_recv().unwrap().len(), 6);
        sender.push(line(1)).unwrap();
        sender.push(line(1)).unwrap();
        assert_eq!(sender.push(line(1)), Err(Refused::Full), "three messages");
        drop(receiver);
        assert_eq!(sender.push(line(1)), Err(Refused::Closed));
    }

    #[tokio::test]
    async fn what_was_queued_is_waited_for_until_it_is_written() {
        let (sender, mut receiver) = queue(Limits {
            messages: 8,
            bytes: 100,
        });
        sender.push(line(1)).unwrap();
        sender.push(line(1)).unwrap();
        let waited = tokio::spawn({
            let sender = sender.clone();
            async move { sender.written().await }
        });
        receiver.recv().await.unwrap();
        receiver.wrote(1);
        tokio::task::yield_now().await;
        assert!(!waited.is_finished(), "one of two written");
        // Queued after the wait began: not waited for.
        sender.push(line(1)).unwrap();
        receiver.recv().await.unwrap();
        receiver.wrote(1);
        waited.await.unwrap();
        // A writer that ends is not waited for.
        drop(receiver);
        sender.written().await;
    }
}
