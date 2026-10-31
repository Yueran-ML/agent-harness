//! A small project for the tests of the edits, whose media counts a tick a frame, so that where a
//! clip begins in its media reads as a frame.
//!
//! Timeline t1 at 25/1:
//!
//! ```text
//! V1  c1 [0, 100) from 0, g1     c3 [100, 150) from 150, g2
//! V2                                  c5 [120, 170) from 50
//! A1  c2 [0, 100) from 0, g1     c4 [100, 150) from 150, g2
//! A2
//! S1    q6 [10, 40)                 q7 [110, 140)
//! ```
//!
//! m1 has picture and sound and lasts 250 frames; m2 has sound only and lasts 200. `next_id` is
//! 10.

use super::{expand, Edit, EditError, Expansion, Material};
use crate::ops::{apply, Op};
use crate::project::{
    AudioFacts, Clip, Cue, Facts, Identity, Media, Project, Timeline, Track, TrackKind, Transition,
    TransitionKind, VideoFacts,
};
use crate::time::{Rate, Time};
use crate::Alignment;

pub const RATE: Rate = Rate { num: 25, den: 1 };

pub fn at(ticks: i64) -> Time {
    Time { ticks, scale: 25 }
}

fn media(id: &str, frames: i64, video: bool) -> Media {
    Media {
        id: id.into(),
        path: format!("C:/media/{id}.mp4"),
        identity: Identity {
            size: 1000,
            modified_us: 0,
            digest: "0".repeat(64),
        },
        facts: Facts {
            duration: at(frames),
            video: video.then(|| VideoFacts {
                still: false,
                width: 1920,
                height: 1080,
                rate: RATE,
                codec: "h264".into(),
                bits: 8,
                rotation: 0,
                color: None,
            }),
            audio: Some(AudioFacts {
                rate: 48000,
                channels: 2,
                codec: "aac".into(),
            }),
        },
        proxy: None,
        name: format!("{id}.mp4"),
        bin: None,
        mark_in: None,
        mark_out: None,
    }
}

pub fn clip(id: &str, start: i64, length: i64, from: i64, link: Option<&str>) -> Clip {
    let mut clip = Clip::of_media(id, "m1", at(from), start, length);
    clip.link = link.map(Into::into);
    clip
}

fn cue(id: &str, start: i64, length: i64, text: &str) -> Cue {
    Cue {
        id: id.into(),
        start,
        length,
        text: text.into(),
    }
}

pub fn project() -> Project {
    let mut subtitles = Track::new("S1", TrackKind::Subtitle);
    subtitles.cues = vec![cue("q6", 10, 30, "hello"), cue("q7", 110, 30, "world")];
    let mut timeline = Timeline::new("t1", "Timeline 1", RATE);
    timeline.tracks = vec![
        Track::new("V1", TrackKind::Video).with_clips(vec![
            clip("c1", 0, 100, 0, Some("g1")),
            clip("c3", 100, 50, 150, Some("g2")),
        ]),
        Track::new("V2", TrackKind::Video).with_clips(vec![clip("c5", 120, 50, 50, None)]),
        Track::new("A1", TrackKind::Audio).with_clips(vec![
            clip("c2", 0, 100, 0, Some("g1")),
            clip("c4", 100, 50, 150, Some("g2")),
        ]),
        Track::new("A2", TrackKind::Audio),
        subtitles,
    ];
    let mut project = Project {
        next_id: 10,
        media: vec![media("m1", 250, true), media("m2", 200, false)],
        timelines: vec![timeline],
        ..Project::new()
    };
    project.validate().unwrap();
    project
}

/// Apply operations, each of which must be applied.
pub fn with(mut project: Project, ops: Vec<Op>) -> Project {
    for op in ops {
        apply(&mut project, op).unwrap();
    }
    project
}

/// A transition with no id between two clips of t1.
pub fn transition(track: &str, left: &str, right: &str, length: i64, align: Alignment) -> Op {
    let kind = if track.starts_with('A') {
        TransitionKind::Crossfade
    } else {
        TransitionKind::Dissolve
    };
    Op::AddTransition {
        timeline: "t1".into(),
        track: track.into(),
        transition: Transition {
            id: String::new(),
            kind,
            left: left.into(),
            right: right.into(),
            length,
            align,
        },
    }
}

/// Lock a track of t1.
pub fn lock(track: &str) -> Op {
    Op::SetTrack {
        timeline: "t1".into(),
        id: track.into(),
        name: None,
        enabled: None,
        locked: Some(true),
        language: None,
    }
}

/// Apply an edit, which must be applied, and check what it leaves.
#[allow(clippy::needless_pass_by_value, reason = "edits are written in place")]
pub fn edit(project: &mut Project, edit: Edit) -> Expansion {
    let expansion =
        expand(project, edit.clone()).unwrap_or_else(|e| panic!("{edit:?} was refused: {e}"));
    let mut copy = project.clone();
    copy.validate().unwrap();
    assert_eq!(&copy, project);
    expansion
}

/// The error an edit is refused with, once it is checked that the project is unchanged.
pub fn refused(mut project: Project, edit: Edit) -> EditError {
    let before = project.clone();
    let error = expand(&mut project, edit).unwrap_err();
    assert_eq!(project, before);
    error
}

/// The clips of a track of t1: id, start, length and where each begins in its media.
pub fn clips(project: &Project, track: &str) -> Vec<(String, i64, i64, i64)> {
    project.timelines[0]
        .tracks
        .iter()
        .find(|r| r.id == track)
        .unwrap()
        .clips
        .iter()
        .map(|c| {
            let from = c.source_in().map_or(-1, |t| t.ticks);
            (c.id.clone(), c.start, c.length, from)
        })
        .collect()
}

/// The subtitles of a track of t1: id, start and length.
pub fn cues(project: &Project, track: &str) -> Vec<(String, i64, i64)> {
    project.timelines[0]
        .tracks
        .iter()
        .find(|r| r.id == track)
        .unwrap()
        .cues
        .iter()
        .map(|q| (q.id.clone(), q.start, q.length))
        .collect()
}

/// A clip of t1.
pub fn the(project: &Project, id: &str) -> Clip {
    project.timelines[0]
        .tracks
        .iter()
        .flat_map(|r| &r.clips)
        .find(|c| c.id == id)
        .unwrap()
        .clone()
}

/// The transitions of a track of t1: left, right, length.
pub fn transitions(project: &Project, track: &str) -> Vec<(String, String, i64)> {
    project.timelines[0]
        .tracks
        .iter()
        .find(|r| r.id == track)
        .unwrap()
        .transitions
        .iter()
        .map(|x| (x.left.clone(), x.right.clone(), x.length))
        .collect()
}

/// m1 from `from` to `to`, in frames.
pub fn m1(from: Option<i64>, to: Option<i64>) -> Material {
    Material::Media {
        media: "m1".into(),
        from: from.map(at),
        to: to.map(at),
        length: None,
    }
}

/// Owned rows, to compare with `clips`.
pub fn rows(rows: &[(&str, i64, i64, i64)]) -> Vec<(String, i64, i64, i64)> {
    rows.iter()
        .map(|&(id, start, length, from)| (id.to_string(), start, length, from))
        .collect()
}
