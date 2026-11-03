//! Rings between one thread that writes and one that reads (`rtrb`): neither locks nor waits,
//! and neither takes memory once the ring is made. A ring of samples carries the sound (the
//! decoded sound of a clip to the mix, the mix to the card, T5); a ring of messages carries
//! messages of a fixed size (a parameter changed, a new graph, an old graph sent back to be
//! dropped).
//!
//! The capacity is rounded up to a power of two.

use rtrb::{Consumer, Producer, RingBuffer};

/// A ring of `capacity` samples (rounded up to a power of two): the end that writes and the end
/// that reads. Made off the thread of the mix: it takes its memory here.
#[must_use]
pub fn samples(capacity: usize) -> (SampleProducer, SampleConsumer) {
    let (producer, consumer) = RingBuffer::new(capacity.max(1).next_power_of_two());
    (
        SampleProducer { ring: producer },
        SampleConsumer { ring: consumer },
    )
}

/// A ring of `capacity` messages (rounded up to a power of two).
#[must_use]
pub fn messages<T: Send>(capacity: usize) -> (MessageProducer<T>, MessageConsumer<T>) {
    let (producer, consumer) = RingBuffer::new(capacity.max(1).next_power_of_two());
    (
        MessageProducer { ring: producer },
        MessageConsumer { ring: consumer },
    )
}

/// The end of a ring of samples that writes.
#[derive(Debug)]
pub struct SampleProducer {
    ring: Producer<f32>,
}

impl SampleProducer {
    /// Writes as many of `samples` as there is room for, from the first; the count written.
    pub fn push(&mut self, samples: &[f32]) -> usize {
        let (written, _) = self.ring.push_partial_slice(samples);
        written.len()
    }

    /// The room for samples now (more may come free at any time).
    #[must_use]
    pub fn free(&self) -> usize {
        self.ring.slots()
    }

    /// The samples the ring holds when full.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.ring.buffer().capacity()
    }

    /// The end that reads is gone.
    #[must_use]
    pub fn is_abandoned(&self) -> bool {
        self.ring.is_abandoned()
    }
}

/// The end of a ring of samples that reads.
#[derive(Debug)]
pub struct SampleConsumer {
    ring: Consumer<f32>,
}

impl SampleConsumer {
    /// Reads as many samples as there are into the start of `out`, up to its length; the count
    /// read.
    pub fn pop(&mut self, out: &mut [f32]) -> usize {
        let (read, _) = self.ring.pop_partial_slice(out);
        read.len()
    }

    /// Drops up to `count` samples unread; the count dropped.
    pub fn discard(&mut self, count: usize) -> usize {
        let count = count.min(self.ring.slots());
        match self.ring.read_chunk(count) {
            Ok(chunk) => {
                chunk.commit_all();
                count
            }
            Err(_) => 0,
        }
    }

    /// The samples there are to read now (more may come at any time).
    #[must_use]
    pub fn available(&self) -> usize {
        self.ring.slots()
    }

    /// The samples the ring holds when full.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.ring.buffer().capacity()
    }

    /// The end that writes is gone.
    #[must_use]
    pub fn is_abandoned(&self) -> bool {
        self.ring.is_abandoned()
    }
}

/// The end of a ring of messages that writes.
#[derive(Debug)]
pub struct MessageProducer<T> {
    ring: Producer<T>,
}

impl<T> MessageProducer<T> {
    /// Writes `message`.
    ///
    /// # Errors
    ///
    /// The message back when the ring is full.
    pub fn push(&mut self, message: T) -> Result<(), T> {
        self.ring
            .push(message)
            .map_err(|rtrb::PushError::Full(message)| message)
    }

    /// The room for messages now.
    #[must_use]
    pub fn free(&self) -> usize {
        self.ring.slots()
    }

    /// The end that reads is gone.
    #[must_use]
    pub fn is_abandoned(&self) -> bool {
        self.ring.is_abandoned()
    }
}

/// The end of a ring of messages that reads.
#[derive(Debug)]
pub struct MessageConsumer<T> {
    ring: Consumer<T>,
}

impl<T> MessageConsumer<T> {
    /// The oldest message, if there is one.
    pub fn pop(&mut self) -> Option<T> {
        self.ring.pop().ok()
    }

    /// The messages there are to read now.
    #[must_use]
    pub fn available(&self) -> usize {
        self.ring.slots()
    }

    /// The end that writes is gone.
    #[must_use]
    pub fn is_abandoned(&self) -> bool {
        self.ring.is_abandoned()
    }
}
