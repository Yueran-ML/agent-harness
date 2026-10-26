//! A project of the studio: the media that it is cut from, and its timelines, each a stack of
//! tracks of clips.
//!
//! This is the form in which a project is written and read, by the engine and by every client
//! of it, so the names of the fields are kept as they are. A field that is not known is refused,
//! and so is a project of another schema.

use serde::{Deserialize, Serialize};

use crate::time::{Rate, Time};

/// The schema of a project that this engine reads and writes.
pub const SCHEMA: u32 = 1;

/// What a thing of a project is known by. It never changes, and is never given to another thing.
pub type Id = String;

/// A project of the studio.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    /// The schema that the project is written in.
    pub schema: u32,
    /// The number that the next id that the engine makes ends in. Never goes back.
    pub next_id: u64,
    /// The files that the project is cut from.
    pub media: Vec<Media>,
    /// The timelines of the project.
    pub timelines: Vec<Timeline>,
}

/// A file that the project is cut from. It is listed by its path and never copied.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Media {
    /// What the clips name it by.
    pub id: Id,
    /// Where the file is.
    pub path: String,
    /// What the file was when it was taken in: its size, the time it was written, and a digest.
    pub identity: Identity,
    /// What was found in the file when it was taken in.
    pub facts: Facts,
}

/// What tells a file from another one at the same path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    /// Its size in bytes.
    pub size: u64,
    /// When it was written last, in nanoseconds since 1970.
    pub modified_ns: i64,
    /// A digest of it, in hexadecimal.
    pub digest: String,
}

/// What a file holds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Facts {
    /// How long it lasts.
    pub duration: Time,
    /// Its picture, if it has one.
    pub video: Option<VideoFacts>,
    /// Its sound, if it has any.
    pub audio: Option<AudioFacts>,
}

/// The picture of a file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VideoFacts {
    /// Its width in pixels, as it is stored.
    pub width: u32,
    /// Its height in pixels, as it is stored.
    pub height: u32,
    /// Its rate of frames.
    pub rate: Rate,
    /// The name of its codec.
    pub codec: String,
    /// The bits of each sample of a pixel.
    pub bits: u8,
    /// How far the picture is turned to be shown, in degrees.
    pub rotation: i16,
}

/// The sound of a file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioFacts {
    /// Its samples a second.
    pub rate: u32,
    /// How many channels it has.
    pub channels: u16,
    /// The name of its codec.
    pub codec: String,
}

/// A timeline: tracks of clips, laid on frames of one rate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Timeline {
    /// What it is known by.
    pub id: Id,
    /// The name that it is shown by.
    pub name: String,
    /// The rate of its frames.
    pub rate: Rate,
    /// Its tracks.
    pub tracks: Vec<Track>,
}

/// What a track holds: picture or sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    /// Picture.
    Video,
    /// Sound.
    Audio,
}

/// A track of a timeline, and the clips on it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Track {
    /// What it is known by.
    pub id: Id,
    /// Whether it holds picture or sound.
    pub kind: TrackKind,
    /// Its clips, in the order of their start.
    pub clips: Vec<Clip>,
}

/// A stretch of a source on a track: from `source_in` in the source, at frame `start` of the
/// timeline, for `length` frames. The clips of a track are kept in the order of their start.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Clip {
    /// What it is known by.
    pub id: Id,
    /// The media that it is cut from.
    pub media: Id,
    /// Where it begins in the media.
    pub source_in: Time,
    /// The frame of the timeline that it begins at.
    pub start: i64,
    /// How many frames it lasts.
    pub length: i64,
}

/// Why a project cannot be read.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    /// It is of another schema.
    #[error("the project is of schema {0}, and this engine reads schema {SCHEMA}")]
    Schema(u32),
    /// It is not a project.
    #[error("the project cannot be read: {0}")]
    Json(#[from] serde_json::Error),
}

impl Project {
    /// A project with nothing in it.
    #[must_use]
    pub fn new() -> Self {
        Self {
            schema: SCHEMA,
            next_id: 1,
            media: vec![],
            timelines: vec![],
        }
    }

    /// Read a project, refusing one of another schema before anything else is looked at.
    ///
    /// # Errors
    /// The schema is another, or the project is not one.
    pub fn from_value(value: serde_json::Value) -> Result<Self, ReadError> {
        let schema = value
            .get("schema")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        if schema != u64::from(SCHEMA) {
            return Err(ReadError::Schema(u32::try_from(schema).unwrap_or(u32::MAX)));
        }
        Ok(serde_json::from_value(value)?)
    }
}

impl Default for Project {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, feature = "sample"))]
impl Project {
    /// A small project to test with: ten seconds of a talk, on one track of picture and one
    /// of sound.
    ///
    /// The engine makes an id of a letter for the kind of the thing and `next_id`, and then adds
    /// one to `next_id`; so an id that it makes is new, whatever its letter, as long as `next_id`
    /// is above the number that every id of the project ends in. The ids here were written by
    /// hand, as those of a project made elsewhere may be, and the last of them ends in 2: so
    /// `next_id` is 3.
    #[must_use]
    pub fn sample() -> Self {
        const SCALE: u32 = 15360;
        let clip = |id: &str| Clip {
            id: id.into(),
            media: "m1".into(),
            source_in: Time {
                ticks: 0,
                scale: SCALE,
            },
            start: 0,
            length: 100,
        };
        Self {
            schema: SCHEMA,
            next_id: 3,
            media: vec![Media {
                id: "m1".into(),
                path: "C:/media/talk.mp4".into(),
                identity: Identity {
                    size: 12_500_000,
                    modified_ns: 1_790_000_000_000_000_000,
                    digest: "0".repeat(64),
                },
                facts: Facts {
                    duration: Time {
                        ticks: 10 * i64::from(SCALE),
                        scale: SCALE,
                    },
                    video: Some(VideoFacts {
                        width: 1920,
                        height: 1080,
                        rate: Rate { num: 25, den: 1 },
                        codec: "h264".into(),
                        bits: 8,
                        rotation: 0,
                    }),
                    audio: Some(AudioFacts {
                        rate: 48000,
                        channels: 2,
                        codec: "aac".into(),
                    }),
                },
            }],
            timelines: vec![Timeline {
                id: "t1".into(),
                name: "Timeline 1".into(),
                rate: Rate { num: 25, den: 1 },
                tracks: vec![
                    Track {
                        id: "V1".into(),
                        kind: TrackKind::Video,
                        clips: vec![clip("c1")],
                    },
                    Track {
                        id: "A1".into(),
                        kind: TrackKind::Audio,
                        clips: vec![clip("c2")],
                    },
                ],
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_is_written_with_the_names_that_other_languages_read() {
        let text = serde_json::to_string_pretty(&Project::sample()).unwrap();
        let written: serde_json::Value = serde_json::from_str(&text).unwrap();
        let expected = serde_json::json!({
            "schema": 1,
            "next_id": 3,
            "media": [{
                "id": "m1",
                "path": "C:/media/talk.mp4",
                "identity": {
                    "size": 12_500_000,
                    "modified_ns": 1_790_000_000_000_000_000_i64,
                    "digest": "0".repeat(64),
                },
                "facts": {
                    "duration": { "ticks": 153_600, "scale": 15360 },
                    "video": {
                        "width": 1920,
                        "height": 1080,
                        "rate": { "num": 25, "den": 1 },
                        "codec": "h264",
                        "bits": 8,
                        "rotation": 0,
                    },
                    "audio": { "rate": 48000, "channels": 2, "codec": "aac" },
                },
            }],
            "timelines": [{
                "id": "t1",
                "name": "Timeline 1",
                "rate": { "num": 25, "den": 1 },
                "tracks": [
                    {
                        "id": "V1",
                        "kind": "video",
                        "clips": [{
                            "id": "c1",
                            "media": "m1",
                            "source_in": { "ticks": 0, "scale": 15360 },
                            "start": 0,
                            "length": 100,
                        }],
                    },
                    {
                        "id": "A1",
                        "kind": "audio",
                        "clips": [{
                            "id": "c2",
                            "media": "m1",
                            "source_in": { "ticks": 0, "scale": 15360 },
                            "start": 0,
                            "length": 100,
                        }],
                    },
                ],
            }],
        });
        assert_eq!(written, expected);
    }

    #[test]
    fn a_project_is_written_and_read_back_as_it_was() {
        let project = Project::sample();
        let text = serde_json::to_string_pretty(&project).unwrap();
        assert_eq!(serde_json::from_str::<Project>(&text).unwrap(), project);
    }

    #[test]
    fn a_project_of_another_schema_is_refused() {
        let mut value = serde_json::to_value(Project::sample()).unwrap();
        value["schema"] = 99.into();
        let error = Project::from_value(value).unwrap_err();
        assert!(error.to_string().contains("schema 99"), "{error}");
    }

    #[test]
    fn a_field_that_is_not_known_is_refused() {
        let mut value = serde_json::to_value(Project::sample()).unwrap();
        value["colour"] = "red".into();
        assert!(Project::from_value(value).is_err());
    }
}
