//! Putting a source on a timeline: insert, overwrite, replace, place on top, append, smart insert
//! and ripple overwrite.

use std::collections::HashSet;

use super::misc::new_track;
use super::work::{beyond, missing, set_source_in, Work};
use super::{Created, EditError, Material};
use crate::ops::OpError;
use crate::project::{Clip, Id, Source, TrackKind};
use crate::span::{frames_for_span, source_tick};
use crate::time::{Speed, Time};

/// The tracks that the picture and the sound of a source go on, as an edit gives them: left
/// out, the first of their kind; `null`, none.
pub(super) type Targets = [Option<Option<Id>>; 2];

/// A source as it is put on a timeline: what its clips show, how many frames they last, and
/// whether it has picture and sound.
struct Made {
    source: Source,
    length: i64,
    video: bool,
    audio: bool,
}

fn made(work: &Work, material: &Material) -> Result<Made, EditError> {
    let generated = |source, length: i64| {
        if length < 1 {
            return Err(EditError::EmptyRange);
        }
        Ok(Made {
            source,
            length,
            video: true,
            audio: false,
        })
    };
    let (media_id, from, to, length) = match material {
        Material::Title {
            text,
            style,
            length,
        } => {
            let source = Source::Title {
                text: text.clone(),
                style: style.clone(),
            };
            return generated(source, *length);
        }
        Material::Color { color, length } => {
            return generated(Source::Color { color: *color }, *length);
        }
        Material::Media {
            media,
            from,
            to,
            length,
        } => (media, from, to, length),
    };
    let media = work
        .project()
        .media
        .iter()
        .find(|m| m.id == *media_id)
        .ok_or_else(|| missing("media", media_id))?;
    let duration = media.facts.duration;
    let scale = duration.scale;
    let from = from.or(media.mark_in).unwrap_or(Time { ticks: 0, scale });
    let to = to.or(media.mark_out).unwrap_or(duration);
    for time in [from, to] {
        if time.scale != scale {
            return Err(EditError::Op(OpError::Scale {
                scale: time.scale,
                media_scale: scale,
            }));
        }
    }
    let out = |why| {
        EditError::Op(OpError::OutOfSource {
            media: media_id.clone(),
            why,
        })
    };
    if from.ticks < 0 {
        return Err(out("before_start"));
    }
    if from.ticks >= duration.ticks || to.ticks > duration.ticks {
        return Err(out("past_end"));
    }
    let probe = Clip::of_media("", media_id.clone(), from, 0, 1);
    let have = longest(work, &probe);
    let length = match *length {
        Some(length) if length > have => {
            return Err(EditError::NoHandles {
                clip: media_id.clone(),
                need: length,
                have,
                longest: None,
            })
        }
        Some(length) => length,
        None => frames_for_span(to.ticks - from.ticks, Speed::ONE, work.rate(), scale).min(have),
    };
    if length < 1 {
        return Err(EditError::EmptyRange);
    }
    Ok(Made {
        source: probe.source,
        length,
        video: media.facts.video.is_some(),
        audio: media.facts.audio.is_some(),
    })
}

/// The most frames that a clip like `probe`, of media, can last from where it begins in it.
pub(super) fn longest(work: &Work, probe: &Clip) -> i64 {
    let (Some(media), Some(begins)) = (probe.media(), probe.source_in()) else {
        return 0;
    };
    let Some(media) = work.project().media.iter().find(|m| m.id == *media) else {
        return 0;
    };
    let span = media.facts.duration.ticks - begins.ticks;
    let frames = |span| frames_for_span(span, probe.speed, work.rate(), begins.scale);
    // Its last frame shows a place before the end.
    let mut length = frames(span).min(frames(span - 1) + 1);
    let with = |length| Clip {
        length,
        ..probe.clone()
    };
    if !work.fits(&with(1), 0, 0) {
        return 0;
    }
    while length > 1 && !work.fits(&with(length), 0, 0) {
        length -= 1;
    }
    length
}

/// The tracks that a source goes on: its picture on a track of picture and its sound on one of
/// sound, as `targets` gives them.
fn tracks_for(work: &Work, made: &Made, targets: &Targets) -> Result<Vec<Id>, EditError> {
    let mut tracks = vec![];
    let kinds = [
        (TrackKind::Video, made.video, &targets[0]),
        (TrackKind::Audio, made.audio, &targets[1]),
    ];
    for (kind, has, given) in kinds {
        if !has {
            continue;
        }
        let id = match given {
            Some(None) => continue,
            Some(Some(id)) => id.clone(),
            None => match work.timeline().tracks.iter().find(|r| r.kind == kind) {
                Some(track) => track.id.clone(),
                None => continue,
            },
        };
        let track = work.open(&id)?;
        if track.kind != kind {
            return Err(EditError::Op(OpError::WrongKind {
                track: id,
                kind: track.kind,
            }));
        }
        tracks.push(id);
    }
    if tracks.is_empty() {
        let kind = if made.video {
            TrackKind::Video
        } else {
            TrackKind::Audio
        };
        return Err(EditError::NoTrack { kind });
    }
    Ok(tracks)
}

/// Put the clips of a source on `tracks` at `at`, linked if there are two.
fn put(work: &mut Work, tracks: &[Id], made: &Made, at: i64) -> Result<(), EditError> {
    let mut group = (tracks.len() > 1).then(String::new);
    for track in tracks {
        let mut clip = Clip::new("", made.source.clone(), at, made.length);
        clip.link = group.take();
        let inserted = work.insert(track, clip)?;
        group.clone_from(&inserted.link);
        work.created(Created::Clip {
            id: inserted.id,
            kind: work.track(track)?.kind,
            link: group.clone(),
        });
    }
    Ok(())
}

fn not_before_start(at: i64) -> Result<(), EditError> {
    if at < 0 {
        return Err(EditError::Op(OpError::OutOfTimeline { start: at }));
    }
    Ok(())
}

pub(super) fn insert(
    work: &mut Work,
    material: &Material,
    at: i64,
    targets: &Targets,
) -> Result<(), EditError> {
    let made = made(work, material)?;
    let tracks = tracks_for(work, &made, targets)?;
    work.make_room(at, made.length)?;
    put(work, &tracks, &made, at)
}

pub(super) fn overwrite(
    work: &mut Work,
    material: &Material,
    at: i64,
    targets: &Targets,
) -> Result<(), EditError> {
    let made = made(work, material)?;
    let tracks = tracks_for(work, &made, targets)?;
    not_before_start(at)?;
    work.clear_clips(&tracks, at, at + made.length)?;
    put(work, &tracks, &made, at)
}

pub(super) fn replace(
    work: &mut Work,
    material: &Material,
    at: i64,
    targets: &Targets,
) -> Result<(), EditError> {
    let made = made(work, material)?;
    let tracks = tracks_for(work, &made, targets)?;
    let rate = work.rate();
    let mut replaced = false;
    for track in &tracks {
        let found = work
            .track(track)?
            .clips
            .iter()
            .find(|c| c.start <= at && at < c.end());
        let Some(old) = found.cloned() else {
            continue;
        };
        let mut new = Clip {
            source: made.source.clone(),
            ..old.clone()
        };
        if let (
            Material::Media {
                from: Some(from), ..
            },
            Some(_),
        ) = (material, new.source_in())
        {
            // `from` is shown at `at`: the clip begins as far before it as its frames before
            // `at` show.
            let mut probe = new.clone();
            set_source_in(&mut probe, Time { ticks: 0, ..*from });
            let before = source_tick(&probe, at - old.start, rate, from.scale);
            set_source_in(
                &mut new,
                Time {
                    ticks: from.ticks.saturating_sub(before),
                    ..*from
                },
            );
        }
        let (before, after) = beyond(work.track(track)?, &old.id);
        if !work.fits(&new, before, after) {
            let have = if new.source_in().is_some_and(|t| t.ticks >= 0) {
                longest(work, &new)
            } else {
                0
            };
            return Err(EditError::NoHandles {
                clip: old.id,
                need: old.length,
                have,
                longest: None,
            });
        }
        work.remove(&old.id)?;
        work.insert(track, new)?;
        replaced = true;
    }
    if !replaced {
        return Err(EditError::NothingAt {
            track: tracks.first().cloned(),
            frame: at,
        });
    }
    Ok(())
}

pub(super) fn place_on_top(work: &mut Work, material: &Material, at: i64) -> Result<(), EditError> {
    let made = made(work, material)?;
    not_before_start(at)?;
    let mut tracks = vec![];
    for (kind, has) in [
        (TrackKind::Video, made.video),
        (TrackKind::Audio, made.audio),
    ] {
        if has {
            tracks.push(top(work, kind, at, at + made.length)?);
        }
    }
    put(work, &tracks, &made, at)
}

/// The first track of `kind` that is not locked above the highest one that has something from
/// `from` to `to`, or a new one above them all.
fn top(work: &mut Work, kind: TrackKind, from: i64, to: i64) -> Result<Id, EditError> {
    let tracks: Vec<_> = work
        .timeline()
        .tracks
        .iter()
        .filter(|r| r.kind == kind)
        .collect();
    let highest = tracks
        .iter()
        .rposition(|r| r.clips.iter().any(|c| c.start < to && from < c.end()));
    let above = highest.map_or(0, |h| h + 1);
    if let Some(track) = tracks[above..].iter().find(|r| !r.locked) {
        return Ok(track.id.clone());
    }
    new_track(work, kind, None, None, None)
}

pub(super) fn append(
    work: &mut Work,
    material: &Material,
    targets: &Targets,
) -> Result<(), EditError> {
    let made = made(work, material)?;
    let tracks = tracks_for(work, &made, targets)?;
    let mut at = 0;
    for track in &tracks {
        let end = work.track(track)?.clips.iter().map(Clip::end).max();
        at = at.max(end.unwrap_or(0));
    }
    put(work, &tracks, &made, at)
}

pub(super) fn smart_insert(
    work: &mut Work,
    material: &Material,
    at: i64,
    targets: &Targets,
) -> Result<(), EditError> {
    let open: HashSet<Id> = work
        .unlocked(&[TrackKind::Video, TrackKind::Audio])
        .into_iter()
        .collect();
    let cuts = work
        .timeline()
        .tracks
        .iter()
        .filter(|r| open.contains(&r.id))
        .flat_map(|r| &r.clips)
        .flat_map(|c| [c.start, c.end()]);
    let nearest = cuts
        .chain([0])
        .min_by_key(|&cut| ((cut - at).abs(), cut))
        .unwrap_or(0);
    insert(work, material, nearest, targets)
}

pub(super) fn ripple_overwrite(
    work: &mut Work,
    material: &Material,
    at: i64,
    targets: &Targets,
) -> Result<(), EditError> {
    let made = made(work, material)?;
    let tracks = tracks_for(work, &made, targets)?;
    let mut found = None;
    for track in &tracks {
        let clips = &work.track(track)?.clips;
        found = found.or_else(|| {
            clips
                .iter()
                .find(|c| c.start <= at && at < c.end())
                .map(|c| (c.start, c.end()))
        });
    }
    let Some((start, end)) = found else {
        return Err(EditError::NothingAt {
            track: tracks.first().cloned(),
            frame: at,
        });
    };
    work.clear_clips(&tracks, start, end)?;
    let all = work.all_unlocked();
    work.shift(&all, end, made.length - (end - start), &HashSet::new())?;
    put(work, &tracks, &made, start)
}

#[cfg(test)]
mod tests {
    use super::super::fixture::*;
    use super::super::{Edit, EditError, Material};
    use crate::ops::Op;
    use crate::project::{Rgba, TextAlign, TitleStyle};
    use crate::Alignment;

    fn insert(source: Material, at: i64) -> Edit {
        Edit::Insert {
            timeline: "t1".into(),
            source,
            at,
            video_track: None,
            audio_track: None,
        }
    }

    fn overwrite(source: Material, at: i64) -> Edit {
        Edit::Overwrite {
            timeline: "t1".into(),
            source,
            at,
            video_track: None,
            audio_track: None,
        }
    }

    fn colour(length: i64) -> Material {
        Material::Color {
            color: Rgba([0, 0, 0, 255]),
            length,
        }
    }

    #[test]
    fn an_insert_cuts_every_open_track_and_moves_what_follows_on() {
        let mut project = project();
        // 20 frames of m1 from frame 10, at frame 50, through c1 and c2.
        edit(&mut project, insert(m1(Some(10), Some(30)), 50));
        // c1 and c2 are cut at 50: their second halves, c10 and c12 (in a new group, g11),
        // start 20 frames later, and the new clips c13 and c15 (in g14) fill the room.
        assert_eq!(
            clips(&project, "V1"),
            rows(&[
                ("c1", 0, 50, 0),
                ("c13", 50, 20, 10),
                ("c10", 70, 50, 50),
                ("c3", 120, 50, 150),
            ])
        );
        assert_eq!(
            clips(&project, "A1"),
            rows(&[
                ("c2", 0, 50, 0),
                ("c15", 50, 20, 10),
                ("c12", 70, 50, 50),
                ("c4", 120, 50, 150),
            ])
        );
        assert_eq!(clips(&project, "V2"), rows(&[("c5", 140, 50, 50)]));
        assert_eq!(the(&project, "c13").link, the(&project, "c15").link);
        assert_eq!(the(&project, "c10").link, the(&project, "c12").link);
        assert_ne!(the(&project, "c10").link, the(&project, "c1").link);
        // Subtitles after the place move on; the one before stays.
        assert_eq!(
            cues(&project, "S1"),
            [("q6".into(), 10, 30), ("q7".into(), 130, 30)]
        );
    }

    #[test]
    fn an_insert_leaves_a_locked_track_as_it_was() {
        let mut project = with(project(), vec![lock("V2"), lock("S1")]);
        edit(&mut project, insert(m1(Some(0), Some(10)), 100));
        assert_eq!(clips(&project, "V2"), rows(&[("c5", 120, 50, 50)]));
        assert_eq!(cues(&project, "S1")[1], ("q7".into(), 110, 30));
        assert_eq!(clips(&project, "V1")[2], ("c3".into(), 110, 50, 150));
    }

    #[test]
    fn an_insert_splits_a_subtitle_and_undo_brings_back_the_whole_subtitle() {
        let before = project();
        let mut history = crate::History::new(before.clone());
        history
            .edit(vec![insert(m1(Some(0), Some(10)), 25)], "desktop")
            .unwrap();
        let after = history.project().clone();
        let subtitles = &after.timelines[0].tracks[4].cues;
        assert_eq!(
            subtitles
                .iter()
                .map(|q| (q.start, q.length, q.text.as_str()))
                .collect::<Vec<_>>(),
            [(10, 15, "hello"), (35, 15, "hello"), (120, 30, "world")]
        );
        assert_eq!(subtitles[0].id, "q6");
        assert_ne!(subtitles[1].id, subtitles[0].id);
        assert_eq!(subtitles[2].id, "q7");
        history.undo("agent").unwrap().unwrap();
        assert_eq!(
            history.project(),
            &crate::Project {
                next_id: after.next_id,
                ..before
            }
        );
        history.redo("desktop").unwrap().unwrap();
        assert_eq!(history.project(), &after);
    }

    #[test]
    fn an_insert_at_a_subtitle_edge_makes_no_empty_subtitle() {
        for (at, start) in [(10, 20), (40, 10)] {
            let mut project = project();
            edit(&mut project, insert(m1(Some(0), Some(10)), at));
            assert_eq!(
                cues(&project, "S1"),
                [("q6".into(), start, 30), ("q7".into(), 120, 30)]
            );
        }
    }

    #[test]
    fn an_insert_inside_a_locked_subtitle_leaves_it_whole() {
        let mut project = with(project(), vec![lock("S1")]);
        let before = project.timelines[0].tracks[4].clone();
        edit(&mut project, insert(m1(Some(0), Some(10)), 25));
        assert_eq!(project.timelines[0].tracks[4], before);
    }

    #[test]
    fn an_insert_inside_a_transition_keeps_it_with_the_remaining_end() {
        let mut project = with(
            project(),
            vec![transition("V1", "c1", "c3", 20, crate::Alignment::Center)],
        );
        let done = edit(&mut project, insert(colour(10), 95));
        assert!(done.clamped);
        let transition = &project.timelines[0].tracks[0].transitions[0];
        assert_eq!(transition.length, 11);
        assert_eq!(transition.right, "c3");
        assert_eq!(the(&project, &transition.left).length, 5);
    }

    #[test]
    fn an_insert_onto_a_locked_track_is_refused() {
        let project = with(project(), vec![lock("A1")]);
        let error = refused(project, insert(m1(None, Some(10)), 0));
        assert_eq!(error, EditError::Locked { track: "A1".into() });
    }

    #[test]
    fn an_insert_takes_the_marks_of_its_media_and_a_track_that_is_null_is_not_used() {
        let mut project = with(
            project(),
            vec![Op::SetMedia {
                id: "m1".into(),
                name: None,
                bin: None,
                mark_in: Some(Some(at(200))),
                mark_out: Some(Some(at(240))),
            }],
        );
        let edit_ = Edit::Insert {
            timeline: "t1".into(),
            source: m1(None, None),
            at: 150,
            video_track: Some(Some("V2".into())),
            audio_track: Some(None),
        };
        // c5 on V2 is cut at 150, and its second half, c10, moves on 40 frames.
        edit(&mut project, edit_);
        assert_eq!(the(&project, "c10").start, 190);
        assert_eq!(the(&project, "c11").start, 150);
        assert_eq!(the(&project, "c11").length, 40);
        assert_eq!(the(&project, "c11").source_in(), Some(at(200)));
        assert_eq!(the(&project, "c11").link, None);
        assert_eq!(clips(&project, "A1").len(), 2);
    }

    #[test]
    fn an_insert_of_more_than_the_media_has_is_refused() {
        let source = Material::Media {
            media: "m1".into(),
            from: Some(at(200)),
            to: None,
            length: Some(60),
        };
        let error = refused(project(), insert(source, 0));
        assert_eq!(
            error,
            EditError::NoHandles {
                clip: "m1".into(),
                need: 60,
                have: 50,
                longest: None
            }
        );
    }

    #[test]
    fn an_overwrite_cuts_away_what_it_covers_on_its_tracks_only() {
        let mut project = project();
        // m2 has sound only: frames [90, 120) of A1 are overwritten.
        let source = Material::Media {
            media: "m2".into(),
            from: None,
            to: None,
            length: Some(30),
        };
        edit(&mut project, overwrite(source, 90));
        assert_eq!(
            clips(&project, "A1"),
            rows(&[("c2", 0, 90, 0), ("c14", 90, 30, 0), ("c12", 120, 30, 170)])
        );
        assert_eq!(the(&project, "c14").link, None);
        assert_eq!(clips(&project, "V1").len(), 2);
        // The rest of c4 is in a group of its own, apart from c3.
        assert_ne!(the(&project, "c12").link, the(&project, "c3").link);
    }

    #[test]
    fn an_overwrite_inside_a_clip_leaves_its_two_ends() {
        let mut project = project();
        edit(
            &mut project,
            Edit::Overwrite {
                timeline: "t1".into(),
                source: colour(10),
                at: 20,
                video_track: None,
                audio_track: None,
            },
        );
        assert_eq!(
            clips(&project, "V1"),
            rows(&[
                ("c1", 0, 20, 0),
                ("c14", 20, 10, -1),
                ("c12", 30, 70, 30),
                ("c3", 100, 50, 150)
            ])
        );
        assert_eq!(clips(&project, "A1").len(), 2);
    }

    #[test]
    fn a_replace_keeps_the_place_and_length_of_the_clip() {
        let mut project = project();
        let edit_ = Edit::Replace {
            timeline: "t1".into(),
            source: m1(Some(60), None),
            at: 130,
            video_track: None,
            audio_track: Some(None),
        };
        edit(&mut project, edit_);
        // Frame 60 of m1 is shown at 130, so c3 begins 30 frames before it.
        assert_eq!(clips(&project, "V1")[1], ("c3".into(), 100, 50, 30));
        assert_eq!(the(&project, "c3").link.as_deref(), Some("g2"));
        assert_eq!(clips(&project, "A1")[1], ("c4".into(), 100, 50, 150));
    }

    #[test]
    fn a_replace_with_too_little_source_is_refused() {
        let edit_ = Edit::Replace {
            timeline: "t1".into(),
            source: m1(Some(220), None),
            at: 0,
            video_track: None,
            audio_track: None,
        };
        let error = refused(project(), edit_);
        assert_eq!(
            error,
            EditError::NoHandles {
                clip: "c1".into(),
                need: 100,
                have: 30,
                longest: None
            }
        );
        let edit_ = Edit::Replace {
            timeline: "t1".into(),
            source: m1(None, None),
            at: 400,
            video_track: None,
            audio_track: None,
        };
        let error = refused(project(), edit_);
        assert_eq!(
            error,
            EditError::NothingAt {
                track: Some("V1".into()),
                frame: 400
            }
        );
    }

    #[test]
    fn a_replace_keeps_the_transitions_of_the_clip() {
        let mut project = with(
            project(),
            vec![transition("V1", "c1", "c3", 10, Alignment::Center)],
        );
        let edit_ = Edit::Replace {
            timeline: "t1".into(),
            source: m1(Some(60), None),
            at: 100,
            video_track: None,
            audio_track: Some(None),
        };
        // From frame 60 of m1, which has the 5 frames before it that the transition shows.
        edit(&mut project, edit_);
        assert_eq!(clips(&project, "V1")[1], ("c3".into(), 100, 50, 60));
        assert_eq!(
            transitions(&project, "V1"),
            [("c1".into(), "c3".into(), 10)]
        );
    }

    #[test]
    fn a_source_is_placed_on_top_above_what_is_there_or_on_a_new_track() {
        let mut project = project();
        let place = |at| Edit::PlaceOnTop {
            timeline: "t1".into(),
            source: colour(10),
            at,
        };
        // V2 is clear at 0.
        edit(&mut project, place(0));
        assert_eq!(clips(&project, "V2")[0], ("c10".into(), 0, 10, -1));
        // V2 has c5 at 130: a new track, V3, is made above it.
        edit(&mut project, place(130));
        assert_eq!(clips(&project, "V3"), rows(&[("c11", 130, 10, -1)]));
        let ids: Vec<&str> = project.timelines[0]
            .tracks
            .iter()
            .map(|r| r.id.as_str())
            .collect();
        assert_eq!(ids, ["V1", "V2", "V3", "A1", "A2", "S1"]);
    }

    #[test]
    fn media_placed_on_top_puts_its_sound_on_top_too() {
        let mut project = project();
        let place = Edit::PlaceOnTop {
            timeline: "t1".into(),
            source: m1(Some(0), Some(10)),
            at: 50,
        };
        edit(&mut project, place);
        assert_eq!(clips(&project, "V2")[0], ("c10".into(), 50, 10, 0));
        assert_eq!(clips(&project, "A2"), rows(&[("c12", 50, 10, 0)]));
        assert_eq!(the(&project, "c10").link, the(&project, "c12").link);
    }

    #[test]
    fn an_append_goes_after_the_last_clip_of_its_tracks() {
        let mut project = project();
        let append = Edit::Append {
            timeline: "t1".into(),
            source: m1(Some(0), Some(10)),
            video_track: Some(Some("V2".into())),
            audio_track: None,
        };
        edit(&mut project, append);
        // V2 ends at 170 and A1 at 150: both go at 170, linked.
        assert_eq!(clips(&project, "V2")[1], ("c10".into(), 170, 10, 0));
        assert_eq!(clips(&project, "A1")[2], ("c12".into(), 170, 10, 0));
        let title = Material::Title {
            text: "End".into(),
            style: TitleStyle {
                font: String::new(),
                size: 96,
                color: Rgba([255, 255, 255, 255]),
                x: 0.5,
                y: 0.5,
                align: TextAlign::Center,
            },
            length: 25,
        };
        let append = Edit::Append {
            timeline: "t1".into(),
            source: title,
            video_track: None,
            audio_track: None,
        };
        edit(&mut project, append);
        assert_eq!(clips(&project, "V1")[2], ("c13".into(), 150, 25, -1));
    }

    #[test]
    fn a_smart_insert_goes_in_at_the_nearest_cut() {
        let mut project = project();
        let smart = Edit::SmartInsert {
            timeline: "t1".into(),
            source: colour(5),
            at: 108,
            video_track: None,
            audio_track: Some(None),
        };
        edit(&mut project, smart);
        // The cuts near 108 are 100 (c1 | c3) and 120 (c5's start): 100 is nearer.
        assert_eq!(
            clips(&project, "V1"),
            rows(&[("c1", 0, 100, 0), ("c10", 100, 5, -1), ("c3", 105, 50, 150)])
        );
        assert_eq!(clips(&project, "A1")[1], ("c4".into(), 105, 50, 150));
        assert_eq!(clips(&project, "V2"), rows(&[("c5", 125, 50, 50)]));
    }

    #[test]
    fn a_ripple_overwrite_takes_the_length_of_the_source_and_ripples_the_rest() {
        let mut project = with(project(), vec![lock("S1")]);
        let edit_ = Edit::RippleOverwrite {
            timeline: "t1".into(),
            source: m1(Some(0), Some(20)),
            at: 10,
            video_track: None,
            audio_track: None,
        };
        // c1 and c2 last 100 frames, the source 20: what follows moves 80 frames back. V2 has
        // c5 from 120, which follows; S1 is locked.
        edit(&mut project, edit_);
        assert_eq!(
            clips(&project, "V1"),
            rows(&[("c10", 0, 20, 0), ("c3", 20, 50, 150)])
        );
        assert_eq!(
            clips(&project, "A1"),
            rows(&[("c12", 0, 20, 0), ("c4", 20, 50, 150)])
        );
        assert_eq!(clips(&project, "V2"), rows(&[("c5", 40, 50, 50)]));
    }

    #[test]
    fn a_ripple_overwrite_is_blocked_by_what_another_track_has_in_the_frames_it_takes_away() {
        let project = with(
            project(),
            vec![crate::Op::MoveClip {
                timeline: "t1".into(),
                id: "c5".into(),
                track: "V2".into(),
                start: 30,
            }],
        );
        let edit_ = Edit::RippleOverwrite {
            timeline: "t1".into(),
            source: m1(Some(0), Some(20)),
            at: 10,
            video_track: None,
            audio_track: None,
        };
        let error = refused(project, edit_);
        assert_eq!(
            error,
            EditError::RippleBlocked {
                track: "V2".into(),
                clip: "c5".into()
            }
        );
    }

    #[test]
    fn an_overwrite_onto_a_locked_track_or_before_the_start_is_refused() {
        let locked = with(project(), vec![lock("V1")]);
        let error = refused(locked, overwrite(colour(10), 20));
        assert_eq!(error, EditError::Locked { track: "V1".into() });
        let error = refused(project(), overwrite(colour(10), -5));
        assert_eq!(
            error,
            EditError::Op(crate::OpError::OutOfTimeline { start: -5 })
        );
    }

    #[test]
    fn a_locked_track_above_is_passed_over_for_a_new_one() {
        let mut project = with(project(), vec![lock("V2")]);
        edit(
            &mut project,
            Edit::PlaceOnTop {
                timeline: "t1".into(),
                source: colour(10),
                at: 0,
            },
        );
        assert_eq!(clips(&project, "V2"), rows(&[("c5", 120, 50, 50)]));
        assert_eq!(clips(&project, "V3"), rows(&[("c10", 0, 10, -1)]));
    }

    #[test]
    fn an_append_to_an_empty_track_goes_at_the_start_and_one_with_no_track_is_refused() {
        let mut project = project();
        let sound = Material::Media {
            media: "m2".into(),
            from: None,
            to: None,
            length: Some(10),
        };
        let append = |audio_track| Edit::Append {
            timeline: "t1".into(),
            source: sound.clone(),
            video_track: None,
            audio_track,
        };
        edit(&mut project, append(Some(Some("A2".into()))));
        assert_eq!(clips(&project, "A2"), rows(&[("c10", 0, 10, 0)]));
        let error = refused(project, append(Some(None)));
        assert_eq!(
            error,
            EditError::NoTrack {
                kind: crate::TrackKind::Audio
            }
        );
    }

    #[test]
    fn a_smart_insert_halfway_between_two_cuts_takes_the_earlier() {
        let mut project = project();
        let smart = Edit::SmartInsert {
            timeline: "t1".into(),
            source: colour(5),
            at: 110,
            video_track: None,
            audio_track: None,
        };
        edit(&mut project, smart);
        assert_eq!(clips(&project, "V1")[1], ("c10".into(), 100, 5, -1));
    }

    #[test]
    fn a_smart_insert_looks_at_the_cuts_of_open_tracks_only() {
        let mut project = with(project(), vec![lock("V1"), lock("A1")]);
        let smart = Edit::SmartInsert {
            timeline: "t1".into(),
            source: colour(5),
            at: 100,
            video_track: Some(Some("V2".into())),
            audio_track: None,
        };
        // The cut at 100 is on locked tracks: the nearest open one is the start of c5.
        edit(&mut project, smart);
        assert_eq!(
            clips(&project, "V2"),
            rows(&[("c11", 120, 5, -1), ("c5", 125, 50, 50)])
        );
        assert_eq!(
            cues(&project, "S1"),
            [
                ("q6".into(), 10, 30),
                ("q7".into(), 110, 10),
                ("q10".into(), 125, 20)
            ]
        );
    }

    #[test]
    fn a_ripple_overwrite_with_a_longer_source_pushes_what_follows_on() {
        let mut project = with(
            project(),
            vec![Op::InsertClip {
                timeline: "t1".into(),
                track: "V1".into(),
                clip: clip("c8", 200, 10, 0, None),
            }],
        );
        let edit_ = Edit::RippleOverwrite {
            timeline: "t1".into(),
            source: m1(Some(0), Some(120)),
            at: 120,
            video_track: None,
            audio_track: None,
        };
        edit(&mut project, edit_);
        assert_eq!(
            clips(&project, "V1"),
            rows(&[("c1", 0, 100, 0), ("c10", 100, 120, 0), ("c8", 270, 10, 0)])
        );
        assert_eq!(clips(&project, "A1")[1], ("c12".into(), 100, 120, 0));
        let edit_ = Edit::RippleOverwrite {
            timeline: "t1".into(),
            source: colour(5),
            at: 400,
            video_track: None,
            audio_track: None,
        };
        let error = refused(project, edit_);
        assert_eq!(
            error,
            EditError::NothingAt {
                track: Some("V1".into()),
                frame: 400
            }
        );
    }

    #[test]
    fn a_track_left_out_and_a_track_that_is_null_are_told_apart() {
        let read = |value| serde_json::from_value::<Edit>(value).unwrap();
        let source = serde_json::json!({ "kind": "media", "media": "m1" });
        let left_out = read(serde_json::json!({
            "edit": "insert", "timeline": "t1", "source": source, "at": 10,
        }));
        let null = read(serde_json::json!({
            "edit": "insert", "timeline": "t1", "source": source, "at": 10, "audio_track": null,
        }));
        let Edit::Insert { audio_track, .. } = left_out else {
            panic!()
        };
        assert_eq!(audio_track, None);
        let Edit::Insert { audio_track, .. } = null.clone() else {
            panic!()
        };
        assert_eq!(audio_track, Some(None));
        // And it is written back as it was read.
        let written = serde_json::to_value(&null).unwrap();
        assert_eq!(read(written), null);
    }
}
