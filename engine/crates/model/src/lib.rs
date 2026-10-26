//! What a project of the studio is, as plain data that the engine reads and writes.

pub mod project;
pub mod time;

pub use project::{
    AudioFacts, Clip, Facts, Id, Identity, Media, Project, ReadError, Timeline, Track, TrackKind,
    VideoFacts, SCHEMA,
};
pub use time::{Rate, Time};
