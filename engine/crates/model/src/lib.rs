//! What a project of the studio is, as plain data that the engine reads and writes.
//!
//! The JSON of a project is read by JavaScript too, which holds every number as a double: so
//! every integer in it stays within 2^53.

pub mod history;
pub mod ops;
pub mod project;
pub mod time;

pub use history::{Applied, History, Step, TransactionError, HISTORY_MOST};
pub use ops::{apply, apply_done, Invalid, Op, OpError, LARGEST};
pub use project::{
    AudioFacts, Clip, Facts, Id, Identity, Media, Project, Proxy, ReadError, Timeline, Track, TrackKind,
    VideoFacts, SCHEMA,
};
pub use time::{Rate, Time};
