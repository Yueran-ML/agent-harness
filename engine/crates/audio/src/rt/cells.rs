//! The last cells that a tap of the meters has published (T9): one seqlock for each place of a
//! ring, and a count, so that the thread of the mix writes one cell, not the whole ring, each
//! time it ends one (the review of T9: a ring of 16 cells in one seqlock was 770 words a write).
//!
//! The writer writes the cell `n` (from 0) with its number into the place `n % kept`, then makes
//! the count `n + 1` (release). A reader takes the count (acquire) and reads the places of the
//! cells that it has not read; a place that holds a later number than the one it looks for was
//! written over before it was read, and that cell is lost.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use super::seqlock::{seqlock, SeqReader, SeqWriter, Words};

/// The writer of a ring of cells, on the thread of the mix.
#[derive(Debug)]
pub struct CellWriter<T> {
    places: Box<[SeqWriter<(u64, T)>]>,
    count: Arc<AtomicU64>,
    written: u64,
}

/// A reader of a ring of cells, off the thread of the mix.
#[derive(Debug)]
pub struct CellReader<T> {
    places: Box<[SeqReader<(u64, T)>]>,
    count: Arc<AtomicU64>,
    seen: u64,
}

impl<T> Clone for CellReader<T> {
    fn clone(&self) -> Self {
        CellReader {
            places: self.places.clone(),
            count: Arc::clone(&self.count),
            seen: self.seen,
        }
    }
}

/// A ring that keeps the last `kept` cells (at least 1), each `empty` at first, and its first
/// reader. Off the thread of the mix: it takes its memory here.
#[must_use]
pub fn cells<T: Words>(kept: usize, empty: T) -> (CellWriter<T>, CellReader<T>) {
    let (writers, readers): (Vec<_>, Vec<_>) =
        (0..kept.max(1)).map(|_| seqlock((0_u64, empty))).unzip(); // rt-ok: the ring is made off the thread of the mix
    let count = Arc::new(AtomicU64::new(0)); // rt-ok: made off the thread of the mix
    (
        CellWriter {
            places: writers.into_boxed_slice(),
            count: Arc::clone(&count),
            written: 0,
        },
        CellReader {
            places: readers.into_boxed_slice(),
            count,
            seen: 0,
        },
    )
}

impl<T: Words> CellWriter<T> {
    /// Publishes the next cell: one place and the count. Never waits, never allocates.
    pub fn push(&mut self, cell: T) {
        let n = self.written;
        let kept = self.places.len() as u64;
        #[allow(clippy::cast_possible_truncation, reason = "below the length")]
        self.places[(n % kept) as usize].write((n + 1, cell));
        self.written = n + 1;
        self.count.store(n + 1, Ordering::Release);
    }

    /// The cells published so far.
    #[must_use]
    pub fn written(&self) -> u64 {
        self.written
    }

    /// Another reader, off the thread of the mix; it reads the cells published from now on.
    #[must_use]
    pub fn reader(&self) -> CellReader<T> {
        CellReader {
            places: self.places.iter().map(SeqWriter::reader).collect(), // rt-ok: off the thread of the mix
            count: Arc::clone(&self.count),
            seen: self.written,
        }
    }
}

impl<T: Words> CellReader<T> {
    /// Gives `cell` each cell published since the last call, in order, and says how many were
    /// lost (written over before they were read). It may spin while a write is under way.
    pub fn drain(&mut self, mut cell: impl FnMut(&T)) -> u64 {
        let count = self.count.load(Ordering::Acquire);
        let kept = self.places.len() as u64;
        let mut lost = 0;
        for n in self.seen..count {
            #[allow(clippy::cast_possible_truncation, reason = "below the length")]
            let (number, value) = self.places[(n % kept) as usize].read();
            if number == n + 1 {
                cell(&value);
            } else {
                lost += 1;
            }
        }
        self.seen = self.seen.max(count);
        lost
    }
}
