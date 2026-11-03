//! What the threads of the sound share, and the tools of the thread of the mix: nothing here
//! locks, waits, or takes memory once made (convention 3 of the plan; checked by
//! `tests/rt_audit.rs` and `tests/no_alloc.rs`).
//!
//! - [`ring`](mod@ring): rings between one thread that writes and one that reads, of samples
//!   and of messages (`rtrb`).
//! - [`seqlock`](mod@seqlock): a value written by one thread and read by any, the reader never
//!   holding the writer back.
//! - [`smooth`]: the 10 ms ramp of a parameter changed while the sound plays.

pub mod ring;
pub mod seqlock;
pub mod smooth;

pub use ring::{
    messages, samples, MessageConsumer, MessageProducer, SampleConsumer, SampleProducer,
};
pub use seqlock::{seqlock, SeqReader, SeqWriter, Words};
pub use smooth::{Domain, Smoothed};
