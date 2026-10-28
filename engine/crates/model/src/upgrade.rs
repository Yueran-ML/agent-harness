//! A project of schema 1, as the first engine wrote it, read into the schema of this one.
//!
//! What schema 2 adds is given the value that leaves the project as it was: a timeline is as
//! large as the picture of its first clip of picture, or 1920 by 1080 if it has none, and plays
//! its sound at 48000 samples a second; a track is named by its id, is seen or heard, is not
//! locked and has no transitions; a clip plays its media as it is; media is named by the name of
//! its file, is kept at the root and has no marks; and there are no bins. The project is then
//! checked as one that is read is.

use serde::Deserialize;

use crate::project::{
    schema_of, Clip, Facts, Id, Identity, Media, Project, Proxy, ReadError, Timeline, Track,
    TrackKind, SCHEMA,
};
use crate::time::{Rate, Time};

/// A project of schema 1.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Project1 {
    #[allow(dead_code)]
    schema: u32,
    next_id: u64,
    media: Vec<Media1>,
    timelines: Vec<Timeline1>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Media1 {
    id: Id,
    path: String,
    identity: Identity,
    facts: Facts,
    #[serde(default)]
    proxy: Option<Proxy>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Timeline1 {
    id: Id,
    name: String,
    rate: Rate,
    tracks: Vec<Track1>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind1 {
    Video,
    Audio,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Track1 {
    id: Id,
    kind: Kind1,
    clips: Vec<Clip1>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Clip1 {
    id: Id,
    media: Id,
    source_in: Time,
    start: i64,
    length: i64,
}

/// The size of a timeline of schema 1 whose clips are of none of the media.
const DEFAULT_SIZE: (u32, u32) = (1920, 1080);

/// Read a project of schema 1 into this schema, and check it as a project that is read is
/// checked.
///
/// # Errors
/// It is not an object, it names no schema or another one than 1, it is not a project of
/// schema 1, or what it becomes is not a valid project.
pub fn from_schema_1(value: serde_json::Value) -> Result<Project, ReadError> {
    let schema = schema_of(&value)?;
    if schema != 1 {
        return Err(ReadError::Schema(schema));
    }
    let old: Project1 = serde_json::from_value(value)?;
    let media: Vec<Media> = old.media.into_iter().map(media_of).collect();
    let timelines = old
        .timelines
        .into_iter()
        .map(|timeline| timeline_of(timeline, &media))
        .collect();
    let mut project = Project {
        schema: SCHEMA,
        next_id: old.next_id,
        bins: vec![],
        media,
        timelines,
    };
    project.validate()?;
    Ok(project)
}

fn media_of(old: Media1) -> Media {
    Media {
        name: file_name(&old.path).into(),
        id: old.id,
        path: old.path,
        identity: old.identity,
        facts: old.facts,
        proxy: old.proxy,
        bin: None,
        mark_in: None,
        mark_out: None,
    }
}

/// The last part of a path, on Windows or on Unix; or the path itself, if it has none.
fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or(path)
}

fn timeline_of(old: Timeline1, media: &[Media]) -> Timeline {
    let (width, height) = size_of(&old, media);
    let tracks = old
        .tracks
        .into_iter()
        .map(|track| {
            let kind = match track.kind {
                Kind1::Video => TrackKind::Video,
                Kind1::Audio => TrackKind::Audio,
            };
            let clips = track
                .clips
                .into_iter()
                .map(|c| Clip::of_media(c.id, c.media, c.source_in, c.start, c.length))
                .collect();
            Track::new(track.id, kind).with_clips(clips)
        })
        .collect();
    Timeline {
        width,
        height,
        tracks,
        ..Timeline::new(old.id, old.name, old.rate)
    }
}

/// The size of the picture of the first clip of picture of a timeline, the one that starts
/// first and, of those that start at one frame, the one on the lowest track; as it is shown,
/// turned upright, and made even and no smaller than 16 nor larger than 16384 pixels. A
/// timeline with no such clip is 1920 by 1080.
fn size_of(timeline: &Timeline1, media: &[Media]) -> (u32, u32) {
    let first = timeline
        .tracks
        .iter()
        .filter(|track| matches!(track.kind, Kind1::Video))
        .flat_map(|track| &track.clips)
        .enumerate()
        .min_by_key(|(at, clip)| (clip.start, *at))
        .map(|(_, clip)| clip);
    let picture = first
        .and_then(|clip| media.iter().find(|m| m.id == clip.media))
        .and_then(|m| m.facts.video.as_ref());
    let Some(picture) = picture else {
        return DEFAULT_SIZE;
    };
    let (width, height) = if matches!(picture.rotation.rem_euclid(180), 90) {
        (picture.height, picture.width)
    } else {
        (picture.width, picture.height)
    };
    let fit = |pixels: u32| (pixels & !1).clamp(16, 16384);
    (fit(width), fit(height))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{Source, Transform};
    use crate::time::Speed;

    const WRITTEN: &str = include_str!("../tests/data/schema1.json");

    fn written() -> serde_json::Value {
        serde_json::from_str(WRITTEN).unwrap()
    }

    #[test]
    fn a_project_of_schema_1_is_read_with_what_schema_2_adds_set_as_it_was() {
        let project = from_schema_1(written()).unwrap();
        assert_eq!(project.schema, 2);
        assert_eq!(project.next_id, 7);
        assert_eq!(project.bins, []);

        let [talk, phone] = &project.media[..] else {
            panic!("{:?}", project.media)
        };
        assert_eq!(talk.name, "talk.mp4");
        assert_eq!(phone.name, "VID_0001.mov");
        assert!(talk.proxy.is_some());
        for media in &project.media {
            assert_eq!(media.bin, None);
            assert_eq!((media.mark_in, media.mark_out), (None, None));
        }

        let sizes: Vec<_> = project
            .timelines
            .iter()
            .map(|t| (t.width, t.height, t.sample_rate))
            .collect();
        // The first, of a clip of the talk; the second, of the phone held upright; the third
        // has no picture.
        assert_eq!(
            sizes,
            [
                (1920, 1080, 48000),
                (1080, 1920, 48000),
                (1920, 1080, 48000)
            ]
        );

        let first = &project.timelines[0];
        let track = &first.tracks[0];
        assert_eq!(
            (&*track.name, track.enabled, track.locked),
            ("V1", true, false)
        );
        assert_eq!(track.transitions, []);
        let clip = &track.clips[0];
        assert_eq!(
            clip.source,
            Source::Media {
                media: "m1".into(),
                source_in: Time {
                    ticks: 0,
                    scale: 15360
                }
            }
        );
        assert_eq!((clip.start, clip.length), (0, 100));
        assert!(clip.enabled && !clip.reverse);
        assert_eq!(clip.speed, Speed::ONE);
        assert_eq!(clip.link, None);
        assert_eq!(clip.transform, Transform::NONE);
        assert_eq!((clip.gain_db, clip.pan), (0.0, 0.0));
        assert_eq!((clip.fade_in, clip.fade_out), (0, 0));
        assert_eq!(project.timelines[2].tracks[0].clips[0].id, "c3");
    }

    #[test]
    fn a_project_that_was_upgraded_is_written_and_read_back_as_it_was() {
        let project = from_schema_1(written()).unwrap();
        let text = serde_json::to_string_pretty(&project).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["schema"], 2);
        let read = Project::from_value(value).unwrap();
        assert_eq!(read, project);
        assert_eq!(serde_json::to_string_pretty(&read).unwrap(), text);
    }

    #[test]
    fn a_project_is_upgraded_only_from_schema_1() {
        let (project, upgraded) = Project::from_value_upgrading(written()).unwrap();
        assert!(upgraded);
        let again = serde_json::to_value(&project).unwrap();
        let (read, upgraded) = Project::from_value_upgrading(again).unwrap();
        assert!(!upgraded);
        assert_eq!(read, project);

        let mut later = written();
        later["schema"] = 3.into();
        let error = Project::from_value_upgrading(later.clone()).unwrap_err();
        assert!(matches!(error, ReadError::Schema(3)), "{error}");
        let error = from_schema_1(later).unwrap_err();
        assert!(matches!(error, ReadError::Schema(3)), "{error}");
        let error = Project::from_value_upgrading(serde_json::json!({})).unwrap_err();
        assert!(matches!(error, ReadError::NoSchema), "{error}");
    }

    #[test]
    fn a_field_of_schema_1_that_is_not_known_is_refused() {
        let mut value = written();
        value["timelines"][0]["tracks"][0]["colour"] = "red".into();
        let error = from_schema_1(value).unwrap_err();
        assert!(matches!(error, ReadError::Json(_)), "{error}");
        assert!(error.to_string().contains("colour"), "{error}");
    }

    #[test]
    fn a_clip_of_schema_1_with_a_source_too_is_refused() {
        let mut value = written();
        value["timelines"][0]["tracks"][0]["clips"][0]["source"] = serde_json::json!({ "kind": "media", "media": "m1", "in": { "ticks": 0, "scale": 15360 } });
        let error = from_schema_1(value).unwrap_err();
        assert!(
            error.to_string().contains("unknown field `source`"),
            "{error}"
        );
    }

    #[test]
    fn a_project_of_schema_1_that_is_not_valid_is_refused() {
        let mut value = written();
        value["timelines"][0]["tracks"][0]["clips"][0]["media"] = "m9".into();
        let error = from_schema_1(value).unwrap_err();
        assert!(matches!(error, ReadError::Invalid(_)), "{error}");
        assert!(
            error.to_string().contains("there is no media m9"),
            "{error}"
        );
    }

    #[test]
    fn a_timeline_is_made_as_large_as_its_first_picture_made_even_and_within_bounds() {
        let mut value = written();
        value["media"][1]["facts"]["video"]["width"] = 1367.into();
        value["media"][1]["facts"]["video"]["height"] = 9.into();
        value["media"][1]["facts"]["video"]["rotation"] = 180.into();
        let project = from_schema_1(value).unwrap();
        let phone = &project.timelines[1];
        assert_eq!((phone.width, phone.height), (1366, 16));

        // The clip that starts first gives the size, whatever track it is on.
        let mut value = written();
        let clip = value["timelines"][1]["tracks"][0]["clips"][0].clone();
        let mut earlier = value["timelines"][0]["tracks"][0]["clips"][0].clone();
        earlier["id"] = "c6".into();
        value["timelines"][1]["tracks"] = serde_json::json!([
            { "id": "V1", "kind": "video", "clips": [clip] },
            { "id": "V2", "kind": "video", "clips": [] },
        ]);
        let mut talk = earlier;
        talk["start"] = 0.into();
        talk["source_in"] = serde_json::json!({ "ticks": 0, "scale": 15360 });
        value["timelines"][1]["tracks"][1]["clips"] = serde_json::json!([talk]);
        let project = from_schema_1(value).unwrap();
        let phone = &project.timelines[1];
        assert_eq!((phone.width, phone.height), (1920, 1080));
    }

    #[test]
    fn media_is_named_by_the_last_part_of_its_path() {
        assert_eq!(file_name(r"C:\media\talk.mp4"), "talk.mp4");
        assert_eq!(file_name("/home/me/a b.mov"), "a b.mov");
        assert_eq!(file_name(r"\\server\share\clip.mxf"), "clip.mxf");
        assert_eq!(file_name("C:/media/dir/"), "dir");
        assert_eq!(file_name("plain"), "plain");
    }
}
