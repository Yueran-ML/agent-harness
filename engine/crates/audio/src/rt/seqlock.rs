//! A value written by one thread and read by any: the levels of a meter's tap, the clock of what
//! the card has played, the reduction of a compressor shown by its ball. The writer never waits;
//! a reader that comes upon a write under way tries again, or, on the thread of the mix, gives up
//! ([`SeqReader::try_read`]).
//!
//! A sequence counter is odd while a write is under way and moves on by two with each write; a
//! read is good when the counter was even and the same before and after it. The value is kept in
//! atomic words, so that a read that overlaps a write is not a race of the memory, only a read to
//! throw away (H.-J. Boehm, "Can seqlocks get along with programming language memory models?",
//! 2012: the fences of the writer and of the reader).

use std::marker::PhantomData;
use std::sync::atomic::{fence, AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;

/// A value that a seqlock can hold: a fixed count of 32-bit words.
pub trait Words: Copy {
    /// The words of a value.
    const WORDS: usize;

    /// The word `index` of the value, `index` below [`Words::WORDS`].
    fn word(&self, index: usize) -> u32;

    /// The value of the words that `word` gives by index.
    fn from_words(word: impl Fn(usize) -> u32) -> Self;
}

impl Words for u32 {
    const WORDS: usize = 1;
    fn word(&self, _: usize) -> u32 {
        *self
    }
    fn from_words(word: impl Fn(usize) -> u32) -> Self {
        word(0)
    }
}

impl Words for i32 {
    const WORDS: usize = 1;
    fn word(&self, _: usize) -> u32 {
        u32::from_ne_bytes(self.to_ne_bytes())
    }
    fn from_words(word: impl Fn(usize) -> u32) -> Self {
        i32::from_ne_bytes(word(0).to_ne_bytes())
    }
}

impl Words for f32 {
    const WORDS: usize = 1;
    fn word(&self, _: usize) -> u32 {
        self.to_bits()
    }
    fn from_words(word: impl Fn(usize) -> u32) -> Self {
        f32::from_bits(word(0))
    }
}

impl Words for bool {
    const WORDS: usize = 1;
    fn word(&self, _: usize) -> u32 {
        u32::from(*self)
    }
    fn from_words(word: impl Fn(usize) -> u32) -> Self {
        word(0) != 0
    }
}

impl Words for u64 {
    const WORDS: usize = 2;
    #[allow(clippy::cast_possible_truncation, reason = "the halves of the word")]
    fn word(&self, index: usize) -> u32 {
        (*self >> (32 * index)) as u32
    }
    fn from_words(word: impl Fn(usize) -> u32) -> Self {
        u64::from(word(0)) | (u64::from(word(1)) << 32)
    }
}

impl Words for i64 {
    const WORDS: usize = 2;
    fn word(&self, index: usize) -> u32 {
        u64::from_ne_bytes(self.to_ne_bytes()).word(index)
    }
    fn from_words(word: impl Fn(usize) -> u32) -> Self {
        i64::from_ne_bytes(u64::from_words(word).to_ne_bytes())
    }
}

impl Words for f64 {
    const WORDS: usize = 2;
    fn word(&self, index: usize) -> u32 {
        self.to_bits().word(index)
    }
    fn from_words(word: impl Fn(usize) -> u32) -> Self {
        f64::from_bits(u64::from_words(word))
    }
}

impl<T: Words, const N: usize> Words for [T; N] {
    const WORDS: usize = T::WORDS * N;
    fn word(&self, index: usize) -> u32 {
        self[index / T::WORDS].word(index % T::WORDS)
    }
    fn from_words(word: impl Fn(usize) -> u32) -> Self {
        std::array::from_fn(|item| T::from_words(|index| word(item * T::WORDS + index)))
    }
}

impl<A: Words, B: Words> Words for (A, B) {
    const WORDS: usize = A::WORDS + B::WORDS;
    fn word(&self, index: usize) -> u32 {
        if index < A::WORDS {
            self.0.word(index)
        } else {
            self.1.word(index - A::WORDS)
        }
    }
    fn from_words(word: impl Fn(usize) -> u32) -> Self {
        (
            A::from_words(&word),
            B::from_words(|index| word(A::WORDS + index)),
        )
    }
}

impl<A: Words, B: Words, C: Words> Words for (A, B, C) {
    const WORDS: usize = A::WORDS + B::WORDS + C::WORDS;
    fn word(&self, index: usize) -> u32 {
        if index < A::WORDS {
            self.0.word(index)
        } else if index < A::WORDS + B::WORDS {
            self.1.word(index - A::WORDS)
        } else {
            self.2.word(index - A::WORDS - B::WORDS)
        }
    }
    fn from_words(word: impl Fn(usize) -> u32) -> Self {
        (
            A::from_words(&word),
            B::from_words(|index| word(A::WORDS + index)),
            C::from_words(|index| word(A::WORDS + B::WORDS + index)),
        )
    }
}

/// What the writer and the readers share.
#[derive(Debug)]
struct Shared {
    sequence: AtomicUsize,
    words: Box<[AtomicU32]>,
}

/// A seqlock holding `initial`: the one writer and a reader (cloned for more). Made off the
/// thread of the mix: it takes its memory here.
#[must_use]
pub fn seqlock<T: Words>(initial: T) -> (SeqWriter<T>, SeqReader<T>) {
    let words: Box<[AtomicU32]> = (0..T::WORDS)
        .map(|index| AtomicU32::new(initial.word(index)))
        .collect(); // rt-ok: the seqlock is made off the thread of the mix
    let shared = Shared {
        sequence: AtomicUsize::new(0),
        words,
    };
    let shared = Arc::new(shared); // rt-ok: the seqlock is made off the thread of the mix
    (
        SeqWriter {
            shared: Arc::clone(&shared),
            value: PhantomData,
        },
        SeqReader {
            shared,
            value: PhantomData,
        },
    )
}

/// The one writer of a seqlock.
#[derive(Debug)]
pub struct SeqWriter<T> {
    shared: Arc<Shared>,
    value: PhantomData<fn(T) -> T>,
}

impl<T: Words> SeqWriter<T> {
    /// Writes `value`, never waiting.
    pub fn write(&mut self, value: T) {
        let shared = &*self.shared;
        let sequence = shared.sequence.load(Ordering::Relaxed);
        shared
            .sequence
            .store(sequence.wrapping_add(1), Ordering::Relaxed);
        fence(Ordering::Release);
        for (index, word) in shared.words.iter().enumerate() {
            word.store(value.word(index), Ordering::Relaxed);
        }
        shared
            .sequence
            .store(sequence.wrapping_add(2), Ordering::Release);
    }

    /// Another reader. Not on the thread of the mix: it counts a reference.
    #[must_use]
    pub fn reader(&self) -> SeqReader<T> {
        SeqReader {
            shared: Arc::clone(&self.shared),
            value: PhantomData,
        }
    }
}

/// A reader of a seqlock.
#[derive(Debug)]
pub struct SeqReader<T> {
    shared: Arc<Shared>,
    value: PhantomData<fn(T) -> T>,
}

impl<T> Clone for SeqReader<T> {
    fn clone(&self) -> Self {
        SeqReader {
            shared: Arc::clone(&self.shared),
            value: PhantomData,
        }
    }
}

impl<T: Words> SeqReader<T> {
    /// The value last written, or `None` when a write was under way: one try, never waiting.
    #[must_use]
    pub fn try_read(&self) -> Option<T> {
        let shared = &*self.shared;
        let before = shared.sequence.load(Ordering::Acquire);
        if before % 2 == 1 {
            return None;
        }
        let value = T::from_words(|index| shared.words[index].load(Ordering::Relaxed));
        fence(Ordering::Acquire);
        let after = shared.sequence.load(Ordering::Relaxed);
        (before == after).then_some(value)
    }

    /// The value last written, trying again while writes come in the way. Not on the thread of
    /// the mix.
    #[must_use]
    pub fn read(&self) -> T {
        loop {
            if let Some(value) = self.try_read() {
                return value;
            }
            std::hint::spin_loop();
        }
    }
}
