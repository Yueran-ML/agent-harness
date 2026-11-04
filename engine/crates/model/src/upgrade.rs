//! A project of schema 1, as the first engine wrote it, or of schema 2, as the engine of the
//! Edit page wrote it, read into the schema of this one.
//!
//! What schema 2 adds is given the value that leaves the project as it was: a timeline is as
//! large as the picture of its first clip of picture, or 1920 by 1080 if it has none, and plays
//! its sound at 48000 samples a second; a track is named by its id, is seen or heard, is not
//! locked and has no transitions; a clip plays its media as it is; media is named by the name of
//! its file, is kept at the root and has no marks; and there are no bins.
//!
//! What schema 3 adds is given the value that leaves the picture as it was: every clip and
//! every timeline has a grade with no node, whose source is linked to its output, and no clip
//! is in a group, of which there are none; the project is not colour managed, with every other
//! setting of colour management at its default, and no media has its colour set by hand.
//! Nothing else of a project of schema 2 is changed.
//! The fields are written into the project of schema 2 before it is read, so that one that has
//! any of them already, which schema 2 does not have, is refused.
//!
//! What schema 4 adds is given the value that leaves the sound as it was, sample for sample:
//! every timeline has one stereo bus, `B1`, which it monitors and every track of sound outputs to
//! at unity, centred, through a strip that changes nothing; every track of sound is stereo, and
//! every clip has its channels mixed to it as before (`channels` none), its gain and pan as one
//! value each, and fades on a straight line over its frames; and every crossfade rises and falls
//! at +3 dB, at equal power, as every crossfade did before schema 4 (a new one is at 0 dB, Q11).
//! A track of sound that was off is on and muted: a track of sound that is off is now one that is
//! not read at all (Disable Track), and the button of its header is its mute (p. 3885), which is
//! not heard either. A project of schema 3 that has any field of schema 4 is refused, as one of
//! schema 2 with a field of schema 3 is.
//!
//! A project is upgraded a schema at a time: one of schema 2 is read into schema 3 and then into
//! schema 4, so that what each schema adds is given its value.
//!
//! The project is then checked as one that is read is.

use serde::Deserialize;

use crate::audio::CrossfadeLevel;
use crate::color_management::{ColorOverride, ColorSettings};
use crate::grade::Grade;
use crate::project::{
    schema_of, Clip, Facts, Id, Identity, Media, Project, Proxy, ReadError, Timeline, Track,
    TrackKind, TransitionKind, SCHEMA,
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
        groups: vec![],
        color: ColorSettings::default(),
    };
    keep_the_sound(&mut project);
    project.validate()?;
    Ok(project)
}

/// Give a project read from an earlier schema what keeps its sound as it was in schema 4, where
/// it was made with the defaults of schema 4: every track of sound that was off is on and
/// muted, and every crossfade is at +3 dB on both sides.
fn keep_the_sound(project: &mut Project) {
    for track in project.timelines.iter_mut().flat_map(|t| &mut t.tracks) {
        if track.kind != TrackKind::Audio {
            continue;
        }
        if !track.enabled {
            track.enabled = true;
            track.audio.mute = true;
        }
        for transition in &mut track.transitions {
            if transition.kind == TransitionKind::Crossfade {
                transition.fade_in_level_db = CrossfadeLevel::Plus3;
                transition.fade_out_level_db = CrossfadeLevel::Plus3;
            }
        }
    }
}

/// Read a project of schema 2 into this schema, and check it as a project that is read is
/// checked: the project gets no groups and the default colour settings, each media no colour
/// set by hand, and each timeline and each clip of a track of picture or of sound a grade that
/// changes nothing and no group; and then what [`from_schema_3`] gives a project of schema 3.
///
/// # Errors
/// It is not an object, it names no schema or another one than 2, it has a field of schema 3
/// or 4 already, or what it becomes is not a valid project.
///
/// # Panics
/// Never: a grade is always written as JSON.
pub fn from_schema_2(mut value: serde_json::Value) -> Result<Project, ReadError> {
    let schema = schema_of(&value)?;
    if schema != 2 {
        return Err(ReadError::Schema(schema));
    }
    let grade = serde_json::to_value(Grade::default()).expect("a grade is written as JSON");
    let color = serde_json::to_value(ColorSettings::default()).expect("settings are JSON");
    let color_override =
        serde_json::to_value(ColorOverride::default()).expect("a colour of media is JSON");
    let project = value.as_object_mut().ok_or(ReadError::NotAnObject)?;
    project.insert("schema".into(), 3.into());
    add(2, project, "groups", serde_json::json!([]))?;
    add(2, project, "color", color)?;
    let media = project
        .get_mut("media")
        .and_then(serde_json::Value::as_array_mut);
    for media in media.into_iter().flatten() {
        if let Some(media) = media.as_object_mut() {
            add(2, media, "color_override", color_override.clone())?;
        }
    }
    let timelines = project
        .get_mut("timelines")
        .and_then(serde_json::Value::as_array_mut);
    for timeline in timelines.into_iter().flatten() {
        let Some(timeline) = timeline.as_object_mut() else {
            continue;
        };
        add(2, timeline, "grade", grade.clone())?;
        let tracks = timeline
            .get_mut("tracks")
            .and_then(serde_json::Value::as_array_mut);
        for track in tracks.into_iter().flatten() {
            let clips = track
                .get_mut("clips")
                .and_then(serde_json::Value::as_array_mut);
            for clip in clips.into_iter().flatten() {
                if let Some(clip) = clip.as_object_mut() {
                    add(2, clip, "grade", grade.clone())?;
                    add(2, clip, "group", serde_json::Value::Null)?;
                }
            }
        }
    }
    from_schema_3(value)
}

/// The fields that schema 4 adds to a timeline.
pub(crate) const TIMELINE_4: [&str; 4] = ["buses", "vcas", "track_groups", "audio"];
/// The fields that schema 4 adds to a track of sound.
pub(crate) const TRACK_4: [&str; 10] = [
    "format",
    "channels",
    "order",
    "mute",
    "solo",
    "solo_safe",
    "adc",
    "automation_arm",
    "strip",
    "lanes",
];
/// The fields that schema 4 adds to a clip.
pub(crate) const CLIP_4: [&str; 7] = [
    "audio_fade_in",
    "audio_fade_out",
    "eq",
    "effects",
    "pitch_correction",
    "pitch",
    "channels",
];
/// The fields that schema 4 adds to a transition.
pub(crate) const TRANSITION_4: [&str; 2] = ["fade_in_level_db", "fade_out_level_db"];

/// Read a project of schema 3 into this schema, so that it sounds as it did (see the module),
/// and check it as a project that is read is checked: a track of sound that was off is on and
/// muted, and a crossfade is at +3 dB on both sides; everything else of schema 4 is left out, and
/// so read as its default.
///
/// # Errors
/// It is not an object, it names no schema or another one than 3, it has a field of schema 4
/// already, or what it becomes is not a valid project.
pub fn from_schema_3(mut value: serde_json::Value) -> Result<Project, ReadError> {
    let schema = schema_of(&value)?;
    if schema != 3 {
        return Err(ReadError::Schema(schema));
    }
    let project = value.as_object_mut().ok_or(ReadError::NotAnObject)?;
    project.insert("schema".into(), SCHEMA.into());
    let timelines = project
        .get_mut("timelines")
        .and_then(serde_json::Value::as_array_mut);
    for timeline in timelines.into_iter().flatten() {
        let Some(timeline) = timeline.as_object_mut() else {
            continue;
        };
        fresh(timeline, &TIMELINE_4)?;
        let tracks = timeline
            .get_mut("tracks")
            .and_then(serde_json::Value::as_array_mut);
        for track in tracks
            .into_iter()
            .flatten()
            .filter_map(|r| r.as_object_mut())
        {
            let sound = track.get("kind").and_then(serde_json::Value::as_str) == Some("audio");
            if sound {
                fresh(track, &TRACK_4)?;
                if track.get("enabled") == Some(&serde_json::Value::Bool(false)) {
                    track.insert("enabled".into(), true.into());
                    track.insert("mute".into(), true.into());
                }
            }
            let clips = track
                .get_mut("clips")
                .and_then(serde_json::Value::as_array_mut);
            for clip in clips
                .into_iter()
                .flatten()
                .filter_map(|c| c.as_object_mut())
            {
                fresh(clip, &CLIP_4)?;
            }
            let transitions = track
                .get_mut("transitions")
                .and_then(serde_json::Value::as_array_mut);
            for transition in transitions
                .into_iter()
                .flatten()
                .filter_map(|x| x.as_object_mut())
            {
                fresh(transition, &TRANSITION_4)?;
                let crossfade =
                    transition.get("kind").and_then(serde_json::Value::as_str) == Some("crossfade");
                if crossfade {
                    transition.insert("fade_in_level_db".into(), 3.into());
                    transition.insert("fade_out_level_db".into(), 3.into());
                }
            }
        }
    }
    Project::from_value(value)
}

/// Refuse an object of a project of schema 3 that has a field of schema 4.
fn fresh(
    object: &serde_json::Map<String, serde_json::Value>,
    names: &[&str],
) -> Result<(), ReadError> {
    match names.iter().find(|name| object.contains_key(**name)) {
        Some(name) => {
            let error = format!("unknown field `{name}` in a project of schema 3");
            Err(ReadError::Json(serde::de::Error::custom(error)))
        }
        None => Ok(()),
    }
}

/// Add a field of the next schema to an object of a project of `schema`, which may not have it.
fn add(
    schema: u32,
    object: &mut serde_json::Map<String, serde_json::Value>,
    name: &str,
    value: serde_json::Value,
) -> Result<(), ReadError> {
    if object.contains_key(name) {
        let error = format!("unknown field `{name}` in a project of schema {schema}");
        return Err(ReadError::Json(serde::de::Error::custom(error)));
    }
    object.insert(name.into(), value);
    Ok(())
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
        color_override: ColorOverride::default(),
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
        assert_eq!(project.schema, SCHEMA);
        assert_eq!(project.groups, []);
        assert_eq!(project.color, ColorSettings::default());
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
            assert_eq!(media.color_override, ColorOverride::default());
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
        assert_eq!(
            (clip.gain_db.fixed(), clip.pan.fixed()),
            (Some(0.0), Some(0.0))
        );
        assert_eq!((clip.fade_in, clip.fade_out), (0, 0));
        assert_eq!((&clip.grade, &clip.group), (&Grade::default(), &None));
        assert_eq!(first.grade, Grade::default());
        assert_eq!(project.timelines[2].tracks[0].clips[0].id, "c3");
    }

    #[test]
    fn a_project_that_was_upgraded_is_written_and_read_back_as_it_was() {
        let project = from_schema_1(written()).unwrap();
        let text = serde_json::to_string_pretty(&project).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["schema"], SCHEMA);
        let read = Project::from_value(value).unwrap();
        assert_eq!(read, project);
        assert_eq!(serde_json::to_string_pretty(&read).unwrap(), text);
    }

    #[test]
    fn a_project_is_upgraded_only_from_schema_1_2_and_3() {
        let (project, upgraded) = Project::from_value_upgrading(written()).unwrap();
        assert_eq!(upgraded, Some(1));
        let again = serde_json::to_value(&project).unwrap();
        let (read, upgraded) = Project::from_value_upgrading(again).unwrap();
        assert_eq!(upgraded, None);
        assert_eq!(read, project);
        let (read, upgraded) = Project::from_value_upgrading(written_2()).unwrap();
        assert_eq!(upgraded, Some(2));
        assert_eq!(read, from_schema_2(written_2()).unwrap());
        let (read, upgraded) = Project::from_value_upgrading(written_3()).unwrap();
        assert_eq!(upgraded, Some(3));
        assert_eq!(read, from_schema_3(written_3()).unwrap());

        let mut later = written();
        later["schema"] = 5.into();
        let error = Project::from_value_upgrading(later.clone()).unwrap_err();
        assert!(matches!(error, ReadError::Schema(5)), "{error}");
        let error = from_schema_1(later.clone()).unwrap_err();
        assert!(matches!(error, ReadError::Schema(5)), "{error}");
        let error = from_schema_2(later.clone()).unwrap_err();
        assert!(matches!(error, ReadError::Schema(5)), "{error}");
        let error = from_schema_3(later).unwrap_err();
        assert!(matches!(error, ReadError::Schema(5)), "{error}");
        let error = from_schema_2(written()).unwrap_err();
        assert!(matches!(error, ReadError::Schema(1)), "{error}");
        let error = from_schema_3(written_2()).unwrap_err();
        assert!(matches!(error, ReadError::Schema(2)), "{error}");
        let error = Project::from_value_upgrading(serde_json::json!({})).unwrap_err();
        assert!(matches!(error, ReadError::NoSchema), "{error}");
    }

    /// What schema 4 writes of a project of schema 3 that was upgraded: its fields at their
    /// defaults, the tracks of sound that were off on and muted, and the crossfades at +3 dB.
    fn as_schema_4(project: &mut serde_json::Value) {
        use crate::audio::written;
        project["schema"] = 4.into();
        for timeline in project["timelines"].as_array_mut().unwrap() {
            written::with(timeline, written::timeline());
            for track in timeline["tracks"].as_array_mut().unwrap() {
                if track["kind"] == "audio" {
                    written::with(track, written::track());
                    if track["enabled"] == false {
                        track["enabled"] = true.into();
                        track["mute"] = true.into();
                    }
                }
                for clip in track
                    .get_mut("clips")
                    .into_iter()
                    .flat_map(|c| c.as_array_mut().unwrap())
                {
                    written::with(clip, written::clip());
                }
                for transition in track
                    .get_mut("transitions")
                    .into_iter()
                    .flat_map(|c| c.as_array_mut().unwrap())
                {
                    let level = if transition["kind"] == "crossfade" {
                        3
                    } else {
                        0
                    };
                    transition["fade_in_level_db"] = level.into();
                    transition["fade_out_level_db"] = level.into();
                }
            }
        }
    }

    const WRITTEN_3: &str = include_str!("../tests/data/schema3.json");

    fn written_3() -> serde_json::Value {
        serde_json::from_str(WRITTEN_3).unwrap()
    }

    #[test]
    fn a_project_of_schema_3_is_read_so_that_it_sounds_as_it_did() {
        let old = written_3();
        let project = from_schema_3(old.clone()).unwrap();
        assert_eq!(project.schema, SCHEMA);
        let timeline = &project.timelines[0];
        // Bus 1 is there, monitored, and every track of sound outputs to it at unity.
        assert_eq!(timeline.buses, vec![crate::audio::Bus::first()]);
        assert_eq!(timeline.audio.monitor.as_deref(), Some("B1"));
        let sound: Vec<_> = timeline
            .tracks
            .iter()
            .filter(|r| r.kind == TrackKind::Audio)
            .collect();
        assert_eq!(sound.len(), 2);
        for track in &sound {
            assert_eq!(track.audio.strip, crate::audio::Strip::to_first_bus());
            assert_eq!(track.audio.format, crate::audio::TrackFormat::Stereo);
            assert!(track.enabled);
        }
        // The track that was off is muted; the other is not.
        assert_eq!(
            sound.iter().map(|r| r.audio.mute).collect::<Vec<_>>(),
            [false, true]
        );
        // The crossfade is at +3 dB on both sides, and the dissolve at 0.
        let crossfade = &sound[0].transitions[0];
        assert_eq!(crossfade.kind, TransitionKind::Crossfade);
        assert_eq!(
            (crossfade.fade_in_level_db, crossfade.fade_out_level_db),
            (CrossfadeLevel::Plus3, CrossfadeLevel::Plus3)
        );
        let dissolve = &timeline.tracks[0].transitions[0];
        assert_eq!(dissolve.fade_in_level_db, CrossfadeLevel::Zero);
        // The gain and pan are fixed, the fades are of frames on a straight line, and the
        // channels are mixed as libav mixes them.
        for clip in sound.iter().flat_map(|r| &r.clips) {
            assert!(clip.gain_db.fixed().is_some() && clip.pan.fixed().is_some());
            assert_eq!((&clip.audio_fade_in, &clip.audio_fade_out), (&None, &None));
            assert_eq!(clip.channels, None);
            assert!(!clip.pitch_correction);
        }
        // What it is written as is what it was, with the fields of schema 4 added so.
        let mut expected = old;
        as_schema_4(&mut expected);
        let text = serde_json::to_string_pretty(&project).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&text).unwrap(),
            expected
        );
        // And it is read back as it is, and written again byte for byte.
        let read = Project::from_value(expected).unwrap();
        assert_eq!(read, project);
        assert_eq!(serde_json::to_string_pretty(&read).unwrap(), text);
    }

    #[test]
    fn a_project_of_schema_3_with_a_field_of_schema_4_is_refused() {
        for path in [
            &["timelines", "0", "buses"][..],
            &["timelines", "0", "audio"],
            &["timelines", "0", "tracks", "1", "mute"],
            &["timelines", "0", "tracks", "1", "strip"],
            &["timelines", "0", "tracks", "1", "clips", "0", "eq"],
            &["timelines", "0", "tracks", "1", "clips", "0", "channels"],
            &[
                "timelines",
                "0",
                "tracks",
                "1",
                "transitions",
                "0",
                "fade_in_level_db",
            ],
        ] {
            let mut value = written_3();
            let (last, within) = path.split_last().unwrap();
            let mut at = &mut value;
            for part in within {
                at = match part.parse::<usize>() {
                    Ok(index) => &mut at[index],
                    Err(_) => &mut at[*part],
                };
            }
            at[*last] = serde_json::json!(null);
            let error = from_schema_3(value).unwrap_err();
            assert!(matches!(error, ReadError::Json(_)), "{error}");
            assert!(error.to_string().contains(last), "{error}");
        }
        // A track of picture that is off stays off: only one of sound is muted.
        let mut value = written_3();
        value["timelines"][0]["tracks"][0]["enabled"] = false.into();
        let project = from_schema_3(value).unwrap();
        assert!(!project.timelines[0].tracks[0].enabled);
    }

    const WRITTEN_2: &str = include_str!("../tests/data/schema2.json");

    fn written_2() -> serde_json::Value {
        serde_json::from_str(WRITTEN_2).unwrap()
    }

    #[test]
    fn a_project_of_schema_2_is_read_with_no_grade_and_no_group_and_nothing_else_changed() {
        let old = written_2();
        let project = from_schema_2(old.clone()).unwrap();
        assert_eq!(project.schema, SCHEMA);
        assert_eq!(project.color, ColorSettings::default());
        assert_eq!(
            project.color.science,
            crate::color_management::Science::Yrgb
        );
        // What it is written as is what it was, with only the fields of schema 3 added.
        let mut expected = old;
        expected["schema"] = SCHEMA.into();
        expected["groups"] = serde_json::json!([]);
        expected["color"] = serde_json::to_value(ColorSettings::default()).unwrap();
        let media = expected["media"].as_array_mut().unwrap();
        assert!(!media.is_empty(), "the project has media to upgrade");
        for media in media {
            media["color_override"] = serde_json::json!({
                "input": null, "levels": "auto", "lut": { "1d": null, "3d": null }, "inverse_drt": true,
            });
        }
        let grade = serde_json::to_value(Grade::default()).unwrap();
        let mut clips = 0;
        for timeline in expected["timelines"].as_array_mut().unwrap() {
            timeline["grade"] = grade.clone();
            for track in timeline["tracks"].as_array_mut().unwrap() {
                for clip in track
                    .get_mut("clips")
                    .into_iter()
                    .flat_map(|c| c.as_array_mut().unwrap())
                {
                    clip["grade"] = grade.clone();
                    clip["group"] = serde_json::Value::Null;
                    clips += 1;
                }
            }
        }
        assert!(clips >= 4, "the project has clips of each kind to upgrade");
        as_schema_4(&mut expected);
        assert_eq!(serde_json::to_value(&project).unwrap(), expected);
        // And it reads back as it is.
        assert_eq!(Project::from_value(expected).unwrap(), project);
    }

    #[test]
    fn a_project_of_schema_2_with_a_field_of_schema_3_is_refused() {
        for path in [
            &["groups"][..],
            &["color"],
            &["media", "0", "color_override"],
            &["timelines", "0", "grade"],
            &["timelines", "0", "tracks", "0", "clips", "0", "group"],
        ] {
            let mut value = written_2();
            let (last, within) = path.split_last().unwrap();
            let mut at = &mut value;
            for part in within {
                at = match part.parse::<usize>() {
                    Ok(index) => &mut at[index],
                    Err(_) => &mut at[*part],
                };
            }
            at[*last] = serde_json::json!(null);
            let error = from_schema_2(value).unwrap_err();
            assert!(matches!(error, ReadError::Json(_)), "{error}");
            assert!(error.to_string().contains(last), "{error}");
        }
        let mut invalid = written_2();
        invalid["timelines"][0]["tracks"][0]["clips"][0]["source"]["media"] = "m99".into();
        let error = from_schema_2(invalid).unwrap_err();
        assert!(matches!(error, ReadError::Invalid(_)), "{error}");
        let error = from_schema_2(serde_json::json!([2])).unwrap_err();
        assert!(matches!(error, ReadError::NotAnObject), "{error}");
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
