//! The speed points of a clip, set whole: how it plays its source where its speed changes on the
//! way (see [`crate::retime`]).
//!
//! The clip itself holds them ([`crate::project::Clip::retime`]), and it is checked as any
//! change of a clip is. A mapping of one speed is not kept as speed points: it is the speed and
//! the direction of the clip, so that a clip plays one way in one form only.

use super::{
    check_clip, check_room, check_transitions, clip_at, lasts, range, timeline_at, unattached, Op,
    OpError,
};
use crate::project::Project;
use crate::retime::Retime;
use crate::time::Speed;

pub(super) fn set_retime(
    project: &mut Project,
    timeline_id: &str,
    id: &str,
    retime: Option<Retime>,
    length: Option<i64>,
) -> Result<Op, OpError> {
    let t = timeline_at(project, timeline_id)?;
    let timeline = &project.timelines[t];
    let (r, at) = clip_at(timeline, id)?;
    let track = &timeline.tracks[r];
    let old = &track.clips[at];
    // A title, a colour and a still show one picture however they are played.
    if !lasts(project, old) {
        return Err(range("retime_source"));
    }
    let mut new = old.clone();
    if let Some(length) = length {
        new.length = length;
    }
    new.set_mapping(retime);
    check_clip(project, timeline.rate, track.kind, &new)?;
    check_room(&track.clips, &new)?;
    if unattached(track, id).is_err() {
        let mut beside = track.clone();
        beside.clips[at] = new.clone();
        check_transitions(project, timeline.rate, &beside)?;
    }
    // A clip that played as its source is, forward, is put back so by no mapping at all.
    let plain = old.retime.is_none() && old.speed == Speed::ONE && !old.reverse;
    let inverse = Op::SetRetime {
        timeline: timeline_id.into(),
        id: id.into(),
        retime: (!plain).then(|| old.mapping()),
        length: length.map(|_| old.length),
    };
    project.timelines[t].tracks[r].clips[at] = new;
    Ok(inverse)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::super::{apply, apply_done, range_message, Invalid};
    use super::*;
    use crate::project::{Clip, Source, TextAlign, TitleStyle, TrackKind};
    use crate::retime::{SignedSpeed, SpeedPoint};
    use crate::time::Time;

    // The sample: timeline t1 at 25/1, with c1 on V1 and c2 on A1, each on frames [0, 100) and
    // cut from the start of m1, which lasts 10 seconds, 250 frames.

    /// A mapping of speed points, each a frame and the `num` and `den` of its speed.
    fn mapping(points: &[(i64, i64, u32)]) -> Retime {
        Retime {
            points: points
                .iter()
                .map(|&(frame, num, den)| SpeedPoint {
                    frame,
                    speed: SignedSpeed { num, den },
                })
                .collect(),
        }
    }

    /// 40 frames forward, a picture held for 20, and 40 frames back at half the speed.
    fn held() -> Retime {
        mapping(&[(0, 1, 1), (40, 0, 1), (60, -1, 2)])
    }

    fn set(id: &str, retime: Option<Retime>, length: Option<i64>) -> Op {
        Op::SetRetime {
            timeline: "t1".into(),
            id: id.into(),
            retime,
            length,
        }
    }

    fn c1(project: &Project) -> &Clip {
        &project.timelines[0].tracks[0].clips[0]
    }

    /// Apply `op` to `project`, check that its inverse takes it back to what it was and that
    /// the inverse of that does it again; and give back the project after it and its inverse.
    fn round_trip(project: &Project, op: Op) -> (Project, Op) {
        let shown = format!("{op:?}");
        let mut done = project.clone();
        let inverse = apply(&mut done, op).unwrap_or_else(|e| panic!("{shown}: {e}"));
        let mut undone = done.clone();
        let again = apply(&mut undone, inverse.clone()).unwrap();
        assert_eq!(&undone, project, "{shown} then {inverse:?}");
        apply(&mut undone, again).unwrap();
        assert_eq!(undone, done, "{shown} done again");
        (done, inverse)
    }

    /// The error that `op` is refused with, once it is checked that the project is unchanged.
    fn refused_in(mut project: Project, op: Op) -> OpError {
        let before = project.clone();
        let error = apply(&mut project, op).unwrap_err();
        assert_eq!(project, before);
        error
    }

    fn refused(op: Op) -> OpError {
        refused_in(Project::sample(), op)
    }

    #[test]
    fn the_speed_points_of_a_clip_are_set_whole_and_set_back_by_the_inverse() {
        let sample = Project::sample();
        let (done, inverse) = round_trip(&sample, set("c1", Some(held()), None));
        let clip = c1(&done);
        assert_eq!(clip.retime, Some(held()));
        assert_eq!(
            (clip.speed, clip.reverse, clip.length),
            (Speed::ONE, false, 100)
        );
        // The clip played as its source is: no mapping puts it back so.
        assert_eq!(inverse, set("c1", None, None));
        // It is the same whatever went before it: sent again, it leaves the clip as it is,
        // and is taken back to the same.
        let (again, inverse) = round_trip(&done, set("c1", Some(held()), None));
        assert_eq!(again, done);
        assert_eq!(inverse, set("c1", Some(held()), None));
        // With a length, the clip lasts so long: 50 frames, and 100 more at half the speed.
        let slower = mapping(&[(0, 1, 1), (50, 1, 2)]);
        let (longer, inverse) = round_trip(&done, set("c1", Some(slower.clone()), Some(150)));
        assert_eq!(
            (c1(&longer).retime.as_ref(), c1(&longer).length),
            (Some(&slower), 150)
        );
        assert_eq!(inverse, set("c1", Some(held()), Some(100)));
        // None puts it back to the speed of its source, forward, and keeps its length unless
        // one is given.
        let (plain, inverse) = round_trip(&longer, set("c1", None, None));
        let clip = c1(&plain);
        assert_eq!(
            (clip.retime.as_ref(), clip.speed, clip.reverse),
            (None, Speed::ONE, false)
        );
        assert_eq!(clip.length, 150);
        assert_eq!(inverse, set("c1", Some(slower), None));
        let (plain, _) = round_trip(&longer, set("c1", None, Some(100)));
        assert_eq!(plain, sample);
        // The operation is given back as it was done, as it was given.
        let mut project = Project::sample();
        let op = set("c1", Some(held()), Some(90));
        let (done, inverse) = apply_done(&mut project, op.clone()).unwrap();
        assert_eq!((done, inverse), (op, set("c1", None, Some(100))));
    }

    #[test]
    fn a_mapping_of_one_speed_is_set_as_the_speed_of_the_clip() {
        let sample = Project::sample();
        let fast = Speed { num: 2, den: 1 };
        let backward = Retime::constant(fast, true);
        let (done, inverse) = round_trip(&sample, set("c1", Some(backward.clone()), None));
        let clip = c1(&done);
        assert_eq!(
            (clip.speed, clip.reverse, clip.retime.as_ref()),
            (fast, true, None)
        );
        assert_eq!(inverse, set("c1", None, None));
        // Speed points in its place put its speed back to that of its source beside them, and
        // their inverse is the mapping of the speed that it had.
        let (retimed, inverse) = round_trip(&done, set("c1", Some(held()), None));
        let clip = c1(&retimed);
        assert_eq!((clip.speed, clip.reverse), (Speed::ONE, false));
        assert_eq!(clip.retime, Some(held()));
        assert_eq!(inverse, set("c1", Some(backward), None));
        // A freeze frame from end to end is no speed: it is kept as a speed point.
        let (still, _) = round_trip(&sample, set("c1", Some(Retime::still()), None));
        assert_eq!(c1(&still).retime, Some(Retime::still()));
        // One speed that no clip plays at is refused as the speed of a clip is.
        let fastest = Retime::constant(Speed { num: 101, den: 1 }, false);
        assert_eq!(refused(set("c1", Some(fastest), None)), range("speed"));
    }

    #[test]
    fn speed_points_that_a_clip_cannot_have_are_refused_and_told_why() {
        // Not those of a clip of its length.
        let late = mapping(&[(0, 1, 1), (100, 2, 1)]);
        assert_eq!(refused(set("c1", Some(late), None)), range("retime"));
        // They are with the length that is set with them.
        let slow = mapping(&[(0, 1, 1), (100, 1, 2)]);
        round_trip(&Project::sample(), set("c1", Some(slow), Some(101)));
        assert_eq!(
            refused(set("c1", Some(mapping(&[])), None)),
            range("retime")
        );
        assert_eq!(
            refused(set("c1", Some(mapping(&[(1, 1, 2)])), None)),
            range("retime")
        );
        let too_fast = mapping(&[(0, 1, 1), (50, 101, 1)]);
        assert_eq!(refused(set("c1", Some(too_fast), None)), range("speed"));
        let apart = mapping(&[(0, 65537, 65537), (4, 65539, 65539)]);
        assert_eq!(refused(set("c1", Some(apart), None)), range("retime_unit"));
        // More than its media has: 50 frames, and 50 at five times the speed, are 300.
        let past_end = OpError::OutOfSource {
            media: "m1".into(),
            why: "past_end",
        };
        let far = mapping(&[(0, 1, 1), (50, 5, 1)]);
        assert_eq!(refused(set("c1", Some(far), None)), past_end);
        // At four times the speed they are 250, all that it has.
        let all = mapping(&[(0, 1, 1), (50, 4, 1)]);
        round_trip(&Project::sample(), set("c1", Some(all.clone()), None));
        assert_eq!(refused(set("c1", Some(all), Some(101))), past_end);
        // No frame, and a clip that is not there.
        let no_frame = OpError::OutOfSource {
            media: "m1".into(),
            why: "length",
        };
        assert_eq!(refused(set("c1", Some(held()), Some(0))), no_frame);
        assert_eq!(
            refused(set("c9", Some(held()), None)),
            OpError::Missing {
                what: "clip",
                id: "c9".into()
            }
        );
        // A clip that would cover frames of the one after it: nothing is moved out of its way.
        let mut beside = Project::sample();
        let source_in = Time {
            ticks: 0,
            scale: 15360,
        };
        let next = Clip::of_media("", "m1", source_in, 120, 50);
        apply(
            &mut beside,
            Op::InsertClip {
                timeline: "t1".into(),
                track: "V1".into(),
                clip: next,
            },
        )
        .unwrap();
        round_trip(&beside, set("c1", Some(held()), Some(120)));
        assert_eq!(
            refused_in(beside, set("c1", Some(held()), Some(121))),
            OpError::Overlap { with: "c3".into() }
        );
    }

    #[test]
    fn speed_points_are_set_on_a_clip_of_media_that_lasts_whether_or_not_its_track_is_locked() {
        // A track that is locked is changed as by any other operation: the edits are what
        // keep a locked track as it is.
        let mut locked = Project::sample();
        locked.timelines[0].tracks[0].locked = true;
        round_trip(&locked, set("c1", Some(held()), None));
        // A title, whatever it is to be set to.
        let mut titled = Project::sample();
        let title = Source::Title {
            text: "Title".into(),
            style: TitleStyle {
                font: String::new(),
                size: 96.0.into(),
                color: crate::project::Rgba([255, 255, 255, 255]),
                x: 0.5.into(),
                y: 0.5.into(),
                align: TextAlign::Center,
                stroke: None,
                shadow: None,
                background: None,
                spans: Vec::new(),
            },
        };
        titled.timelines[0].tracks[0].clips[0] = Clip::new("c1", title, 0, 100);
        for retime in [Some(held()), Some(Retime::default()), None] {
            assert_eq!(
                refused_in(titled.clone(), set("c1", retime, None)),
                range("retime_source")
            );
        }
        // A still.
        let mut still = Project::sample();
        still.media[0].facts.audio = None;
        still.media[0].facts.video.as_mut().unwrap().still = true;
        still.timelines[0].tracks.truncate(1);
        assert_eq!(
            refused_in(still, set("c1", Some(held()), None)),
            range("retime_source")
        );
        // The sound of media lasts too.
        let (done, _) = round_trip(&Project::sample(), set("c2", Some(held()), None));
        assert_eq!(done.timelines[0].tracks[1].clips[0].retime, Some(held()));
        assert_eq!(done.timelines[0].tracks[1].kind, TrackKind::Audio);
    }

    #[test]
    fn the_one_speed_of_a_clip_is_not_set_beside_its_speed_points() {
        let (retimed, _) = round_trip(&Project::sample(), set("c1", Some(held()), None));
        let set_clip =
            |speed: Option<Speed>, reverse: Option<bool>, enabled: Option<bool>| Op::SetClip {
                timeline: "t1".into(),
                id: "c1".into(),
                enabled,
                speed,
                reverse,
                link: None,
                transform: None,
                crop: None,
                composite: None,
                retime_process: None,
                gain_db: None,
                pan: None,
                fade_in: None,
                fade_out: None,
                audio_fade_in: None,
                audio_fade_out: None,
                eq: None,
                effects: None,
                pitch_correction: None,
                pitch: None,
                channels: None,
                source: None,
            };
        for op in [
            set_clip(Some(Speed { num: 2, den: 1 }), None, None),
            set_clip(Some(Speed::ONE), None, None),
            set_clip(None, Some(true), None),
            set_clip(None, Some(false), None),
        ] {
            assert_eq!(refused_in(retimed.clone(), op), range("retimed"));
        }
        // What else is set of it is set as of any clip.
        let mut off = retimed.clone();
        apply(&mut off, set_clip(None, None, Some(false))).unwrap();
        assert!(!c1(&off).enabled);
        assert_eq!(c1(&off).retime, Some(held()));
        // Once it is back at one speed, its speed is set.
        let mut plain = retimed;
        apply(&mut plain, set("c1", None, None)).unwrap();
        apply(&mut plain, set_clip(None, Some(true), None)).unwrap();
        assert!(c1(&plain).reverse);
    }

    #[test]
    fn a_project_is_read_with_speed_points_as_the_operation_leaves_them() {
        let wrong = |change: &dyn Fn(&mut Clip)| {
            let mut project = Project::sample();
            change(&mut project.timelines[0].tracks[0].clips[0]);
            project.validate().unwrap_err()
        };
        let of_c1 = |what: &'static str| Invalid {
            what: "clip",
            id: "c1".into(),
            error: range(what),
        };
        // One speed is kept as the speed of the clip.
        assert_eq!(
            wrong(&|clip| clip.retime = Some(Retime::default())),
            of_c1("retime_constant")
        );
        let fast = Retime::constant(Speed { num: 2, den: 1 }, true);
        assert_eq!(
            wrong(&|clip| clip.retime = Some(fast.clone())),
            of_c1("retime_constant")
        );
        // Beside speed points the speed is that of the source, forward.
        assert_eq!(
            wrong(&|clip| {
                clip.retime = Some(held());
                clip.speed = Speed { num: 2, den: 1 };
            }),
            of_c1("retime_speed")
        );
        assert_eq!(
            wrong(&|clip| {
                clip.retime = Some(held());
                clip.reverse = true;
            }),
            of_c1("retime_speed")
        );
        // They are those of a clip of its length, in its media.
        assert_eq!(
            wrong(&|clip| {
                clip.retime = Some(held());
                clip.length = 60;
            }),
            of_c1("retime")
        );
        assert_eq!(
            wrong(&|clip| clip.retime = Some(mapping(&[(0, 1, 1), (50, 5, 1)]))),
            Invalid {
                what: "clip",
                id: "c1".into(),
                error: OpError::OutOfSource {
                    media: "m1".into(),
                    why: "past_end",
                },
            }
        );
        // Only media that lasts has them.
        assert_eq!(
            wrong(&|clip| {
                clip.retime = Some(held());
                clip.source = Source::Color {
                    color: crate::project::Rgba([0, 0, 0, 255]),
                };
            }),
            of_c1("retime_source")
        );
        // As the operation leaves them, they are read, and written, as they are.
        let mut project = Project::sample();
        apply(&mut project, set("c1", Some(held()), None)).unwrap();
        let mut read = project.clone();
        read.validate().unwrap();
        assert_eq!(read, project);
        let value = serde_json::to_value(&project).unwrap();
        assert_eq!(Project::from_value(value).unwrap(), project);
    }

    #[test]
    fn the_operation_is_written_with_the_names_that_other_languages_read() {
        let op = set("c1", Some(mapping(&[(0, 1, 1), (40, -1, 2)])), Some(120));
        let written = serde_json::json!({
            "op": "set_retime", "timeline": "t1", "id": "c1",
            "retime": { "points": [
                { "frame": 0, "speed": { "num": 1, "den": 1 } },
                { "frame": 40, "speed": { "num": -1, "den": 2 } },
            ] },
            "length": 120,
        });
        assert_eq!(serde_json::to_value(&op).unwrap(), written);
        assert_eq!(serde_json::from_value::<Op>(written).unwrap(), op);
        // No speed points are written as null, which may not be left out; a length may.
        let none = serde_json::json!({
            "op": "set_retime", "timeline": "t1", "id": "c1", "retime": null,
        });
        assert_eq!(serde_json::to_value(set("c1", None, None)).unwrap(), none);
        assert_eq!(
            serde_json::from_value::<Op>(none).unwrap(),
            set("c1", None, None)
        );
        let left_out = serde_json::json!({ "op": "set_retime", "timeline": "t1", "id": "c1" });
        let error = serde_json::from_value::<Op>(left_out).unwrap_err();
        assert!(
            error.to_string().contains("missing field `retime`"),
            "{error}"
        );
        let unknown = serde_json::json!({
            "op": "set_retime", "timeline": "t1", "id": "c1", "retime": null, "ripple": true,
        });
        assert!(serde_json::from_value::<Op>(unknown).is_err());
    }

    #[test]
    fn every_refusal_of_speed_points_has_a_message_of_its_own() {
        let whats = [
            "retime",
            "retime_unit",
            "frame",
            "hold",
            "speed_point",
            "retime_constant",
            "retime_speed",
            "retime_source",
            "retimed",
        ];
        let messages: HashSet<String> = whats.iter().map(|what| range_message(what)).collect();
        assert_eq!(messages.len(), whats.len());
        for what in whats {
            assert_ne!(
                range_message(what),
                format!("the {what} is out of its range")
            );
        }
    }
}
