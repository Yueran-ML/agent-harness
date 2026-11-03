//! Rings between one thread that writes and one that reads (`rtrb`): neither locks nor waits,
//! and neither takes memory once the ring is made. A ring of samples carries the sound (the
//! decoded sound of a clip to the mix, the mix to the card, T5); a ring of messages carries
//! messages of a fixed size (a parameter changed, a new graph, an old graph sent back to be
//! dropped).
//!
//! A ring of samples carries whole frames of interleaved sound: each frame is a sample of every
//! channel, and every count of it (pushed, popped, dropped, free, held) is in frames, so that a
//! frame is never cut in two. Capacities are rounded up to a power of two (in frames).

use rtrb::{Consumer, Producer, RingBuffer};

/// A ring of `frames` frames of `channels` interleaved samples (the frames rounded up to a
/// power of two): the end that writes and the end that reads. Made off the thread of the mix:
/// it takes its memory here.
///
/// # Panics
///
/// When `channels` is 0.
#[must_use]
pub fn samples(frames: usize, channels: usize) -> (SampleProducer, SampleConsumer) {
    assert!(channels > 0, "a ring of no channels");
    let frames = frames.max(1).next_power_of_two();
    let (producer, consumer) = RingBuffer::new(frames * channels);
    (
        SampleProducer {
            ring: producer,
            channels,
        },
        SampleConsumer {
            ring: consumer,
            channels,
        },
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
    channels: usize,
}

impl SampleProducer {
    /// Writes as many whole frames of `samples` (interleaved) as there is room for, from the
    /// first; the frames written. Samples after the last whole frame are not written.
    pub fn push(&mut self, samples: &[f32]) -> usize {
        let frames = samples.len().min(self.ring.slots()) / self.channels;
        let (written, _) = self
            .ring
            .push_partial_slice(&samples[..frames * self.channels]);
        written.len() / self.channels
    }

    /// The room for frames now (more may come free at any time).
    #[must_use]
    pub fn free(&self) -> usize {
        self.ring.slots() / self.channels
    }

    /// The frames the ring holds when full.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.ring.buffer().capacity() / self.channels
    }

    /// The channels of a frame.
    #[must_use]
    pub fn channels(&self) -> usize {
        self.channels
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
    channels: usize,
}

impl SampleConsumer {
    /// Reads as many whole frames as there are into the start of `out`, as many as fit in it;
    /// the frames read.
    pub fn pop(&mut self, out: &mut [f32]) -> usize {
        let frames = out.len().min(self.ring.slots()) / self.channels;
        let (read, _) = self
            .ring
            .pop_partial_slice(&mut out[..frames * self.channels]);
        read.len() / self.channels
    }

    /// Drops up to `frames` frames unread; the frames dropped.
    pub fn discard(&mut self, frames: usize) -> usize {
        let frames = frames.min(self.ring.slots() / self.channels);
        match self.ring.read_chunk(frames * self.channels) {
            Ok(chunk) => {
                chunk.commit_all();
                frames
            }
            Err(_) => 0,
        }
    }

    /// The frames there are to read now (more may come at any time).
    #[must_use]
    pub fn available(&self) -> usize {
        self.ring.slots() / self.channels
    }

    /// The frames the ring holds when full.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.ring.buffer().capacity() / self.channels
    }

    /// The channels of a frame.
    #[must_use]
    pub fn channels(&self) -> usize {
        self.channels
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
