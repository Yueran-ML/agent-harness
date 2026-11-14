//! What the threads of the sound share, and the tools of the thread of the mix: nothing here
//! locks, waits, or takes memory once made (convention 3 of the plan; checked by
//! `tests/rt_audit.rs` and `tests/no_alloc.rs`).
//!
//! - [`ring`](mod@ring): rings between one thread that writes and one that reads, of frames of
//!   samples and of messages (`rtrb`).
//! - [`seqlock`](mod@seqlock): a value written by one thread and read by any, the reader never
//!   holding the writer back.
//! - [`cells`](mod@cells): the last cells of a tap of the meters, one place written at a time.
//! - [`smooth`]: the 10 ms ramp of a parameter changed while the sound plays.
//! - [`flush`], [`flush32`]: the state of a feedback put to 0 before it turns denormal.

pub mod cells;
pub mod ring;
pub mod seqlock;
pub mod smooth;

pub use cells::{cells, CellReader, CellWriter};
pub use ring::{
    messages, samples, MessageConsumer, MessageProducer, SampleConsumer, SampleProducer,
};
pub use seqlock::{seqlock, SeqReader, SeqWriter, Words};
pub use smooth::{Domain, Smoothed};

/// The size below which the state of a feedback is put to 0.
pub const FLUSH_BELOW: f64 = 1e-30;

/// `x`, or 0 when it is smaller in size than 1e-30: for the state of a filter or an envelope
/// after each sample, so that a tail into silence ends in zeros and never in denormals (the
/// floating-point unit is not set to flush them; see `node`'s "Denormals").
#[must_use]
#[inline]
pub fn flush(x: f64) -> f64 {
    if x.abs() < FLUSH_BELOW {
        0.0
    } else {
        x
    }
}

/// [`flush`] for `f32`.
#[must_use]
#[inline]
#[allow(clippy::cast_possible_truncation, reason = "1e-30 is an f32")]
pub fn flush32(x: f32) -> f32 {
    if x.abs() < FLUSH_BELOW as f32 {
        0.0
    } else {
        x
    }
}
