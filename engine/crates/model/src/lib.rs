//! What a project of the studio is, as plain data that the engine reads and writes.
//!
//! The JSON of a project is read by JavaScript too, which holds every number as a double: so
//! every integer in it stays within 2^53.

pub mod edit;
pub mod history;
pub mod ops;
pub mod project;
pub mod resolve;
pub mod span;
pub mod time;
pub mod upgrade;

pub use edit::{expand, Change, CueItem, Edit, EditError, Expansion};
pub use history::{Applied, Changed, History, Step, TransactionError, HISTORY_MOST};
pub use ops::{apply, apply_done, Invalid, Op, OpError, LARGEST};
pub use project::{
    Alignment, AudioFacts, Bin, Clip, Cue, Facts, Id, Identity, Media, Project, Proxy, ReadError,
    Rgba, Source, TextAlign, Timeline, TitleStyle, Track, TrackKind, Transform, Transition,
    TransitionKind, VideoFacts, SCHEMA,
};
pub use time::{Rate, Speed, Time};
