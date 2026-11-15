//! The Retime Controls (Reference Manual of Resolve 20, pp. 1070–1082): the edits of the speed
//! points of a clip.
//!
//! Each is worked out on the mapping of the clip ([`crate::retime`]), which a clip of one speed
//! has too ([`Clip::mapping`]), and done by setting its mapping whole (`set_retime`), by moving
//! where it begins in its media where the lowest place of its path is then another
//! (`trim_clip`), and by the operations of a ripple where what follows moves with its end. The
//! rules of locks, links, ripples and transitions are those of `set_speed`.
//!
//! # The frame
//!
//! An edit is made at a frame of the clip that it names, counted from its first. The clips
//! linked to it take the change at the same frame of the timeline, each on its own mapping and
//! counted from its own start, so that picture and sound that lie together stay as long as
//! each other. A linked clip that the frame is not in, as where a split edit lets the sound
//! start later or end sooner than the picture, is left as it is, though a ripple still counts
//! it as one of the group whose end moves; and one that does not last, a title, a colour or a
//! still, is left as it is too.
//!
//! # Where the clip begins in its media
//!
//! A change that leaves the path of the clip as it was before the segment that it changes
//! leaves the clip showing, in the frames before that segment, the pictures that it showed:
//! where the lowest place of the path is then another, the clip begins that much later in its
//! media, or earlier ([`crate::retime::Course::later_as`]). The first segment turned round
//! shows what it showed, the last first, as a clip of one speed does that is played backward,
//! and what follows goes on from where it then stops. A clip put back to its own speed begins
//! where it did, and shows from there, forward and once, all that it showed.

use std::collections::HashSet;

use super::work::{fit_fades, retime_as, Work};
use super::{Edge, EditError};
use crate::ops::{lasts, OpError};
use crate::project::{Clip, Id};
use crate::retime::{Retime, Retimed, SignedSpeed, Trimmed};
use crate::time::{Rate, Speed};

/// What an edit of the Retime Controls does to a clip, at a frame of it.
#[derive(Clone, Copy, Debug)]
pub(super) enum Retiming {
    /// A speed point is put at the frame (Add Speed Point, p. 1076).
    Point,
    /// The segment that the frame is in plays at a speed, the way that it went, and shows what
    /// it showed, so it lasts more frames or fewer (Change Speed, pp. 1074 and 1076).
    Speed(Speed),
    /// The picture of the frame is held for so many frames more (Freeze Frame, p. 1075).
    Freeze(i64),
    /// The segment that the frame is in plays the other way (Reverse Segment, p. 1076).
    Reverse,
    /// The speed point that begins the segment that the frame is in is cleared (Clear Speed
    /// Point, p. 1076).
    Clear,
    /// The clip plays at the speed of its source, forward (Reset to 100%, p. 1074): it is made
    /// at no frame.
    Reset,
}

/// How an edit of the Retime Controls leaves a clip to play.
struct Played {
    /// Its mapping.
    retime: Retime,
    /// How many frames it lasts.
    length: i64,
    /// The frame of the clip, as it played before, at whose beginning the path was at the
    /// place where it now begins: the clip begins in its media where that keeps the path
    /// there. None where the clip begins where it did.
    from: Option<i64>,
}

fn range(what: &'static str) -> EditError {
    EditError::Op(OpError::OutOfRange { what })
}

/// How `clip` plays once `retiming` is made at its frame `k`.
fn played(clip: &Clip, k: i64, retiming: Retiming) -> Result<Played, OpError> {
    let mapping = clip.mapping();
    let length = clip.length;
    let same = |retime: Retime| Retimed { retime, length };
    let (retimed, from) = match retiming {
        Retiming::Point => (same(mapping.with_point(k, length)?), Some(0)),
        Retiming::Speed(speed) => {
            // The segment goes on the way that it went: a freeze, forward.
            let backward = mapping
                .points
                .iter()
                .rev()
                .find(|point| point.frame <= k)
                .is_some_and(|point| point.speed.is_backward());
            let speed = SignedSpeed::of(speed, backward);
            (mapping.with_speed_rippled(k, speed, length)?, Some(0))
        }
        Retiming::Freeze(hold) => (mapping.with_freeze(k, hold, length)?, Some(0)),
        Retiming::Reverse => {
            let reversed = same(mapping.with_segment_reversed(k, length)?);
            // The first segment shows what it showed, the last first: its path begins where
            // it stopped. Any other goes back from where the one before it stopped.
            let first = mapping.points.get(1).map_or(length, |next| next.frame);
            (reversed, Some(if k < first { first } else { 0 }))
        }
        Retiming::Clear => (same(mapping.without_point(k, length)?), Some(0)),
        Retiming::Reset => (mapping.reset(length)?, None),
    };
    Ok(Played {
        retime: retimed.retime,
        length: retimed.length,
        from,
    })
}

/// `clip` made to play as `play` says, on a timeline of `rate`: with its mapping, kept as one
/// speed where it is of one, its length, where it then begins in its media, and its fades no
/// longer than it lasts.
fn as_played(clip: &Clip, play: Played, rate: Rate) -> Result<Clip, OpError> {
    let mut new = clip.clone();
    new.length = play.length;
    let Some(begins) = clip.source_in() else {
        return Ok(new);
    };
    let later = match play.from {
        Some(from) => {
            let before = clip.mapping().course(clip.length)?;
            let after = play.retime.course(play.length)?;
            before.later_as(from, &after, rate, begins.scale)?
        }
        None => 0,
    };
    let trimmed = Trimmed {
        retime: play.retime,
        length: play.length,
        later,
    };
    retime_as(&mut new, trimmed, begins);
    fit_fades(&mut new, Edge::Out, rate);
    Ok(new)
}

/// A clip that an edit of the Retime Controls works on, with its track, and how it then plays.
type Change = ((Id, Clip), Played);

/// The clips that an edit of the Retime Controls of `clip` works on, each with its track and
/// how it then plays: the clip, which must last and have the frame `at`, and with `linked` the
/// clips linked to it that last. One that the same frame of the timeline is not in plays as
/// it did: it is still one of the group, whose end a ripple moves what follows by.
fn changes(
    work: &Work,
    clip: &str,
    at: Option<i64>,
    retiming: Retiming,
    linked: bool,
) -> Result<Vec<Change>, EditError> {
    let (_, given) = work.owned(clip)?;
    if !lasts(work.project(), &given) {
        return Err(range("retime_source"));
    }
    // The frame of the timeline that the change is made at.
    let frame = match at {
        Some(at) if (0..given.length).contains(&at) => Some(given.start + at),
        Some(_) => return Err(range("frame")),
        None => None,
    };
    let mut all = vec![];
    for id in work.group(&[clip.into()], linked)? {
        let (track, c) = work.owned(&id)?;
        if !lasts(work.project(), &c) {
            continue;
        }
        let play = match frame {
            Some(frame) if !(c.start..c.end()).contains(&frame) => Played {
                retime: c.mapping(),
                length: c.length,
                from: None,
            },
            Some(frame) => played(&c, frame - c.start, retiming)?,
            None => played(&c, 0, retiming)?,
        };
        all.push(((track, c), play));
    }
    Ok(all)
}

/// Make an edit of the Retime Controls: `retiming` at frame `at` of `clip`, and of the clips
/// linked to it with `linked`. Where the clips then last longer or shorter, what follows them
/// moves with their end with `ripple`, as it does for `set_speed`; without, a clip that would
/// cover the next one on its track is cut at its end where that one starts, and the edit says
/// that it was clamped.
pub(super) fn retime(
    work: &mut Work,
    clip: &str,
    at: Option<i64>,
    retiming: Retiming,
    ripple: bool,
    linked: bool,
) -> Result<(), EditError> {
    if let Retiming::Speed(speed) = retiming {
        let (num, den) = (u64::from(speed.num), u64::from(speed.den));
        if den == 0 || num * 100 < den || num > den * 100 {
            return Err(range("speed"));
        }
    }
    let rate = work.rate();
    let all = changes(work, clip, at, retiming, linked)?;
    let (group, plays): (Vec<(Id, Clip)>, Vec<Played>) = all.into_iter().unzip();
    // The picture and the sound of a split edit can end at different frames. Everything after
    // the group moves by the change in its overall end, whichever member was named.
    let place = group
        .iter()
        .map(|(_, c)| c.end())
        .max()
        .expect("the clip that is named is one of them");
    let ends = group
        .iter()
        .zip(&plays)
        .map(|((_, c), play)| c.start + play.length)
        .max()
        .expect("the clip that is named is one of them");
    if ripple && ends != place {
        work.split_ripple(&group, place)?;
    }
    let mut changed = vec![];
    let mut clamped = false;
    for ((track, c), mut play) in group.into_iter().zip(plays) {
        let after = work
            .track(&track)?
            .clips
            .iter()
            .filter(|o| o.id != c.id && o.start >= c.end())
            .map(|o| o.start)
            .min();
        // What stays where it is bounds the clip, which is cut at its end there.
        if let Some(after) = after.filter(|&after| !ripple || after < place) {
            if c.start + play.length > after {
                play.length = after - c.start;
                play.retime = play.retime.cut_to(play.length);
                clamped = true;
            }
        }
        let new = as_played(&c, play, rate)?;
        changed.push((c, new));
    }
    let shift = changed
        .iter()
        .map(|(_, c)| c.end())
        .max()
        .expect("the clip that is named is one of them")
        - place;
    changed.retain(|(old, new)| old != new);
    if clamped {
        work.clamp();
    }
    let skip: HashSet<Id> = changed.iter().map(|(c, _)| c.id.clone()).collect();
    let tracks = work.all_unlocked();
    for (old, _) in &changed {
        work.detach(&old.id)?;
    }
    // Room is made before a clip gets longer, and the gap is closed once it is shorter.
    if ripple && shift > 0 {
        work.shift(&tracks, place, shift, &skip)?;
    }
    for (_, new) in &changed {
        work.replay(new)?;
    }
    if ripple && shift < 0 {
        work.shift(&tracks, place, shift, &skip)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::fixture::*;
    use super::super::{Edit, EditError};
    use crate::ops::{Op, OpError};
    use crate::project::{Alignment, Clip, Rgba, Source};
    use crate::time::Speed;

    fn point(clip: &str, at: i64) -> Edit {
        Edit::AddSpeedPoint {
            timeline: "t1".into(),
            clip: clip.into(),
            at,
            linked: true,
        }
    }

    fn segment_speed(clip: &str, at: i64, (num, den): (u32, u32), ripple: bool) -> Edit {
        Edit::SetSegmentSpeed {
            timeline: "t1".into(),
            clip: clip.into(),
            at,
            speed: Speed { num, den },
            ripple,
            linked: true,
        }
    }

    fn freeze(clip: &str, at: i64, hold: i64, ripple: bool) -> Edit {
        Edit::FreezeFrame {
            timeline: "t1".into(),
            clip: clip.into(),
            at,
            hold,
            ripple,
            linked: true,
        }
    }

    fn reverse(clip: &str, at: i64) -> Edit {
        Edit::ReverseSegment {
            timeline: "t1".into(),
            clip: clip.into(),
            at,
            linked: true,
        }
    }

    fn clear(clip: &str, at: i64) -> Edit {
        Edit::ClearSpeedPoint {
            timeline: "t1".into(),
            clip: clip.into(),
            at,
            linked: true,
        }
    }

    fn reset(clip: &str, ripple: bool) -> Edit {
        Edit::ResetSpeed {
            timeline: "t1".into(),
            clip: clip.into(),
            ripple,
            linked: true,
        }
    }

    fn out_of_range(what: &'static str) -> EditError {
        EditError::Op(OpError::OutOfRange { what })
    }

    /// The operation that sets the speed points of a clip of t1, and its length if one is
    /// given.
    fn set_retime(id: &str, points: &[(i64, i64, u32)], length: Option<i64>) -> Op {
        Op::SetRetime {
            timeline: "t1".into(),
            id: id.into(),
            retime: Some(speed_points(points)),
            length,
        }
    }

    /// The fixture with a speed point halfway through c1 and c2, which are linked.
    fn halved() -> crate::Project {
        let mut project = project();
        edit(&mut project, point("c1", 50));
        project
    }

    /// V1 c1 [0, 100) g1, c3 [100, 150) g2; A1 c2 [0, 130) g1, c4 [130, 180) g2: the sound of
    /// g1 runs on under the picture of g2.
    fn l_cut() -> crate::Project {
        let mut project = project();
        let tracks = &mut project.timelines[0].tracks;
        tracks[0].clips = vec![
            clip("c1", 0, 100, 0, Some("g1")),
            clip("c3", 100, 50, 150, Some("g2")),
        ];
        tracks[1].clips.clear();
        tracks[2].clips = vec![
            clip("c2", 0, 130, 0, Some("g1")),
            clip("c4", 130, 50, 180, Some("g2")),
        ];
        tracks[4].cues.clear();
        project.validate().unwrap();
        project
    }

    #[test]
    fn a_speed_point_is_put_on_a_clip_of_one_speed_and_moves_no_picture() {
        let before = project();
        let shown = pictures(&before, "c5");
        let mut project = before.clone();
        let done = edit(&mut project, point("c5", 20));
        assert!(!done.clamped);
        assert_eq!(
            done.done,
            [set_retime("c5", &[(0, 1, 1), (20, 1, 1)], None)]
        );
        assert_eq!(clips(&project, "V2"), rows(&[("c5", 120, 50, 50)]));
        assert_eq!(
            the(&project, "c5").retime,
            Some(speed_points(&[(0, 1, 1), (20, 1, 1)]))
        );
        assert_eq!(pictures(&project, "c5"), shown);
        // Where a speed point is already, nothing is done.
        assert_eq!(edit(&mut project, point("c5", 20)).done, [] as [Op; 0]);
        assert_eq!(edit(&mut project, point("c5", 0)).done, [] as [Op; 0]);
        undo(&mut project, &done);
        assert_eq!(project, before);
        // A clip that plays backward at twice the speed keeps that speed in both segments.
        let mut backward = before.clone();
        retime(&mut backward, "c5", &[(0, -2, 1)], 25);
        let shown = pictures(&backward, "c5");
        assert_eq!(shown[..3], [98, 96, 94]);
        edit(&mut backward, point("c5", 10));
        let c5 = the(&backward, "c5");
        assert_eq!(pictures(&backward, "c5"), shown);
        assert_eq!(
            (c5.speed, c5.reverse, c5.retime),
            (
                Speed::ONE,
                false,
                Some(speed_points(&[(0, -2, 1), (10, -2, 1)]))
            )
        );
        // The clip has the frame, and is there.
        for at in [-1, 50] {
            assert_eq!(
                refused(before.clone(), point("c5", at)),
                out_of_range("frame")
            );
        }
        assert_eq!(
            refused(before.clone(), point("c9", 0)),
            EditError::Op(OpError::Missing {
                what: "clip",
                id: "c9".into()
            })
        );
        assert_eq!(
            refused(before, point("c5", i64::MAX)),
            EditError::Op(OpError::TooLarge { what: "at" })
        );
    }

    #[test]
    fn the_clips_linked_to_a_clip_take_the_change_at_the_same_frame_of_the_timeline() {
        let before = project();
        let points = Some(speed_points(&[(0, 1, 1), (40, 1, 1)]));
        // The picture named, and the sound named.
        for named in ["c1", "c2"] {
            let mut project = before.clone();
            edit(&mut project, point(named, 40));
            assert_eq!(the(&project, "c1").retime, points);
            assert_eq!(the(&project, "c2").retime, points);
        }
        // Told not to take the linked clips, the edit leaves them.
        let mut project = before.clone();
        let alone = Edit::AddSpeedPoint {
            timeline: "t1".into(),
            clip: "c1".into(),
            at: 40,
            linked: false,
        };
        edit(&mut project, alone);
        assert_eq!(the(&project, "c1").retime, points);
        assert_eq!(the(&project, "c2"), the(&before, "c2"));
        // A linked clip on a locked track is left, and a clip on one is not changed by name.
        let mut project = with(before.clone(), vec![lock("A1")]);
        edit(&mut project, point("c1", 40));
        assert_eq!(the(&project, "c1").retime, points);
        assert_eq!(the(&project, "c2"), the(&before, "c2"));
        assert_eq!(
            refused(with(before.clone(), vec![lock("V1")]), point("c1", 40)),
            EditError::Locked { track: "V1".into() }
        );
        // The sound of a split edit starts 30 frames after its picture: frame 40 of the
        // picture is its frame 10, and frame 20 of the picture is before it.
        let split = with(
            before,
            vec![Op::TrimClip {
                timeline: "t1".into(),
                id: "c2".into(),
                source_in: Some(at(30)),
                start: 30,
                length: 70,
            }],
        );
        let mut project = split.clone();
        edit(&mut project, point("c1", 40));
        assert_eq!(the(&project, "c1").retime, points);
        assert_eq!(
            the(&project, "c2").retime,
            Some(speed_points(&[(0, 1, 1), (10, 1, 1)]))
        );
        let mut project = split.clone();
        edit(&mut project, point("c1", 20));
        assert_eq!(
            the(&project, "c1").retime,
            Some(speed_points(&[(0, 1, 1), (20, 1, 1)]))
        );
        assert_eq!(the(&project, "c2"), the(&split, "c2"));
    }

    #[test]
    fn a_segment_twice_as_fast_lasts_half_as_long_and_a_ripple_pulls_what_follows() {
        let before = halved();
        let shown = pictures(&before, "c1");
        let mut project = before.clone();
        let done = edit(&mut project, segment_speed("c1", 60, (2, 1), true));
        assert!(!done.clamped);
        let points = Some(speed_points(&[(0, 1, 1), (50, 2, 1)]));
        assert_eq!(the(&project, "c1").retime, points);
        assert_eq!(the(&project, "c2").retime, points);
        assert_eq!(
            clips(&project, "V1"),
            rows(&[("c1", 0, 75, 0), ("c3", 75, 50, 150)])
        );
        assert_eq!(
            clips(&project, "A1"),
            rows(&[("c2", 0, 75, 0), ("c4", 75, 50, 150)])
        );
        assert_eq!(clips(&project, "V2"), rows(&[("c5", 95, 50, 50)]));
        assert_eq!(cues(&project, "S1")[1], ("q7".into(), 85, 30));
        // The frames before the segment show what they showed, and the segment the same
        // stretch of m1, every other picture of it.
        let now = pictures(&project, "c1");
        let fast: Vec<i64> = (0..25).map(|k| 50 + 2 * k).collect();
        assert_eq!(now[..50], shown[..50]);
        assert_eq!(now[50..], fast);
        undo(&mut project, &done);
        assert_eq!(project, before);
    }

    #[test]
    fn a_segment_half_as_fast_lasts_twice_as_long_and_a_ripple_pushes_what_follows() {
        let before = halved();
        let shown = pictures(&before, "c1");
        let mut project = before.clone();
        let done = edit(&mut project, segment_speed("c1", 10, (1, 2), true));
        assert!(!done.clamped);
        let points = Some(speed_points(&[(0, 1, 2), (100, 1, 1)]));
        assert_eq!(the(&project, "c1").retime, points);
        assert_eq!(the(&project, "c2").retime, points);
        assert_eq!(
            clips(&project, "V1"),
            rows(&[("c1", 0, 150, 0), ("c3", 150, 50, 150)])
        );
        assert_eq!(
            clips(&project, "A1"),
            rows(&[("c2", 0, 150, 0), ("c4", 150, 50, 150)])
        );
        assert_eq!(clips(&project, "V2"), rows(&[("c5", 170, 50, 50)]));
        assert_eq!(cues(&project, "S1")[1], ("q7".into(), 160, 30));
        // Each picture of the segment is shown twice, and what follows it is as it was.
        let now = pictures(&project, "c1");
        let slow: Vec<i64> = (0..100).map(|k| k / 2).collect();
        assert_eq!(now[..100], slow);
        assert_eq!(now[100..], shown[50..]);
        undo(&mut project, &done);
        assert_eq!(project, before);
    }

    #[test]
    fn a_segment_slowed_without_a_ripple_is_cut_where_the_next_clip_starts() {
        let before = halved();
        let mut project = before.clone();
        let done = edit(&mut project, segment_speed("c1", 10, (1, 2), false));
        assert!(done.clamped);
        // The 100 frames that the first segment then lasts fill the clip, which is one of half
        // the speed, as its linked sound is.
        for id in ["c1", "c2"] {
            let clip = the(&project, id);
            assert_eq!(
                (clip.speed, clip.reverse, clip.retime),
                (Speed { num: 1, den: 2 }, false, None)
            );
        }
        assert_eq!(
            clips(&project, "V1"),
            rows(&[("c1", 0, 100, 0), ("c3", 100, 50, 150)])
        );
        let slow: Vec<i64> = (0..100).map(|k| k / 2).collect();
        assert_eq!(pictures(&project, "c1"), slow);
        undo(&mut project, &done);
        assert_eq!(project, before);
        // Faster, it leaves the frames after it empty, and nothing moves.
        let done = edit(&mut project, segment_speed("c1", 60, (2, 1), false));
        assert!(!done.clamped);
        assert_eq!(
            clips(&project, "V1"),
            rows(&[("c1", 0, 75, 0), ("c3", 100, 50, 150)])
        );
        assert_eq!(clips(&project, "V2"), rows(&[("c5", 120, 50, 50)]));
    }

    #[test]
    fn a_ripple_that_would_close_over_a_clip_of_another_track_is_refused() {
        let mut before = project();
        before.timelines[0].tracks[1].clips[0].start = 80;
        before.validate().unwrap();
        edit(&mut before, point("c1", 50));
        let faster = segment_speed("c1", 60, (2, 1), true);
        assert_eq!(
            refused(before.clone(), faster.clone()),
            EditError::RippleBlocked {
                track: "V2".into(),
                clip: "c5".into()
            }
        );
        // With that track locked, it stays, and what is not locked moves.
        let mut project = with(before, vec![lock("V2")]);
        edit(&mut project, faster);
        assert_eq!(clips(&project, "V2"), rows(&[("c5", 80, 50, 50)]));
        assert_eq!(clips(&project, "V1")[1], ("c3".into(), 75, 50, 150));
    }

    #[test]
    fn a_ripple_of_a_split_edit_is_refused_rather_than_parting_the_next_group() {
        // Frame 10 is in the picture and in its sound; frame 110 of the sound is after the
        // picture, which is one of the group all the same.
        for edit in [
            freeze("c1", 10, 5, true),
            freeze("c2", 10, 5, true),
            freeze("c2", 110, 5, true),
        ] {
            assert_eq!(
                refused(l_cut(), edit.clone()),
                EditError::RippleBlocked {
                    track: "V1".into(),
                    clip: "c3".into()
                },
                "{edit:?}"
            );
        }
        // Without a ripple, the sound is held at its frame 110 and cut where the next sound
        // starts, and the picture, which has no such frame, is left as it is.
        let before = l_cut();
        let mut project = before.clone();
        let done = edit(&mut project, freeze("c2", 110, 5, false));
        assert!(done.clamped);
        assert_eq!(the(&project, "c1"), the(&before, "c1"));
        assert_eq!(
            the(&project, "c2").retime,
            Some(speed_points(&[(0, 1, 1), (110, 0, 1), (115, 1, 1)]))
        );
        assert_eq!(
            clips(&project, "A1"),
            rows(&[("c2", 0, 130, 0), ("c4", 130, 50, 180)])
        );
    }

    #[test]
    fn the_one_segment_of_a_clip_of_one_speed_is_given_a_speed_as_set_speed_gives_it() {
        let before = project();
        let mut project = before.clone();
        let done = edit(&mut project, segment_speed("c5", 49, (2, 1), false));
        assert_eq!(done.done, [set_retime("c5", &[(0, 2, 1)], Some(25))]);
        assert_eq!(clips(&project, "V2"), rows(&[("c5", 120, 25, 50)]));
        let mut other = before.clone();
        let set_speed = Edit::SetSpeed {
            timeline: "t1".into(),
            clip: "c5".into(),
            speed: Speed { num: 2, den: 1 },
            reverse: None,
            ripple: false,
            linked: true,
        };
        edit(&mut other, set_speed);
        assert_eq!(the(&project, "c5"), the(&other, "c5"));
        // Backward, it stays backward.
        let mut project = before.clone();
        retime(&mut project, "c5", &[(0, -1, 1)], 50);
        edit(&mut project, segment_speed("c5", 0, (1, 2), false));
        let c5 = the(&project, "c5");
        assert_eq!(
            (c5.speed, c5.reverse, c5.retime, c5.length),
            (Speed { num: 1, den: 2 }, true, None, 100)
        );
        // A speed that no segment plays at is refused before the clip is looked for.
        for speed in [(1, 101), (101, 1), (0, 1), (1, 0)] {
            assert_eq!(
                refused(before.clone(), segment_speed("c99", 0, speed, false)),
                out_of_range("speed")
            );
        }
        for speed in [(1, 100), (100, 1)] {
            edit(&mut before.clone(), segment_speed("c5", 0, speed, false));
        }
    }

    #[test]
    fn a_frame_is_held_and_the_clip_lasts_as_much_longer() {
        let before = project();
        let shown = pictures(&before, "c5");
        let mut project = before.clone();
        let done = edit(&mut project, freeze("c5", 10, 5, false));
        assert!(!done.clamped);
        let points = [(0, 1, 1), (10, 0, 1), (15, 1, 1)];
        assert_eq!(done.done, [set_retime("c5", &points, Some(55))]);
        assert_eq!(clips(&project, "V2"), rows(&[("c5", 120, 55, 50)]));
        // The picture of frame 10 for 5 frames more, and then all that the clip showed.
        let now = pictures(&project, "c5");
        assert_eq!(now[..10], shown[..10]);
        assert_eq!(now[10..15], [60; 5]);
        assert_eq!(now[15..], shown[10..]);
        undo(&mut project, &done);
        assert_eq!(project, before);
        // A freeze lasts a frame at least, and no more than a count may be.
        assert_eq!(
            refused(before.clone(), freeze("c5", 10, 0, false)),
            out_of_range("hold")
        );
        assert_eq!(
            refused(before, freeze("c5", 10, i64::MAX, false)),
            EditError::Op(OpError::TooLarge { what: "hold" })
        );
    }

    #[test]
    fn a_freeze_ripples_what_follows_or_is_cut_where_the_next_clip_starts() {
        let before = project();
        let points = Some(speed_points(&[(0, 1, 1), (40, 0, 1), (60, 1, 1)]));
        // With a ripple, the linked clips last 20 frames more, and what follows moves on.
        let mut project = before.clone();
        let done = edit(&mut project, freeze("c1", 40, 20, true));
        assert!(!done.clamped);
        assert_eq!(the(&project, "c1").retime, points);
        assert_eq!(the(&project, "c2").retime, points);
        assert_eq!(
            clips(&project, "V1"),
            rows(&[("c1", 0, 120, 0), ("c3", 120, 50, 150)])
        );
        assert_eq!(
            clips(&project, "A1"),
            rows(&[("c2", 0, 120, 0), ("c4", 120, 50, 150)])
        );
        assert_eq!(clips(&project, "V2"), rows(&[("c5", 140, 50, 50)]));
        assert_eq!(cues(&project, "S1")[1], ("q7".into(), 130, 30));
        let held: Vec<i64> = (0..40).chain([40; 20]).chain(40..100).collect();
        assert_eq!(pictures(&project, "c1"), held);
        undo(&mut project, &done);
        assert_eq!(project, before);
        // Without, each is cut at its end where the next clip of its track starts: its last
        // 20 frames are not shown.
        let done = edit(&mut project, freeze("c1", 40, 20, false));
        assert!(done.clamped);
        assert_eq!(the(&project, "c1").retime, points);
        assert_eq!(
            clips(&project, "V1"),
            rows(&[("c1", 0, 100, 0), ("c3", 100, 50, 150)])
        );
        assert_eq!(
            clips(&project, "A1"),
            rows(&[("c2", 0, 100, 0), ("c4", 100, 50, 150)])
        );
        let held: Vec<i64> = (0..40).chain([40; 20]).chain(40..80).collect();
        assert_eq!(pictures(&project, "c1"), held);
        // A linked clip on a locked track stays as it is, and so does all its track.
        let mut project = with(before.clone(), vec![lock("A1")]);
        let sound = project.timelines[0].tracks[2].clone();
        edit(&mut project, freeze("c1", 40, 20, true));
        assert_eq!(project.timelines[0].tracks[2], sound);
        assert_eq!(clips(&project, "V1")[1], ("c3".into(), 120, 50, 150));
        // A transition at the end of the clip goes with its end.
        let mut project = with(
            before,
            vec![transition("V1", "c1", "c3", 10, Alignment::Center)],
        );
        let done = edit(&mut project, freeze("c1", 40, 20, true));
        assert!(!done.clamped);
        assert_eq!(
            transitions(&project, "V1"),
            [("c1".into(), "c3".into(), 10)]
        );
        assert_eq!(clips(&project, "V1")[1], ("c3".into(), 120, 50, 150));
    }

    #[test]
    fn a_clip_of_one_speed_turned_round_shows_what_it_showed_the_last_first() {
        let before = project();
        let mut shown = pictures(&before, "c5");
        let mut project = before.clone();
        let done = edit(&mut project, reverse("c5", 30));
        assert!(!done.clamped);
        assert_eq!(done.done, [set_retime("c5", &[(0, -1, 1)], None)]);
        let c5 = the(&project, "c5");
        assert_eq!((c5.speed, c5.reverse, c5.retime), (Speed::ONE, true, None));
        assert_eq!(clips(&project, "V2"), rows(&[("c5", 120, 50, 50)]));
        shown.reverse();
        assert_eq!(pictures(&project, "c5"), shown);
        // Turned round again, it is as it was.
        edit(&mut project, reverse("c5", 0));
        assert_eq!(project, before);
    }

    #[test]
    fn a_segment_turned_round_goes_back_from_where_the_one_before_it_stopped() {
        // c5 in three segments of its own speed: frames 0 to 10, 10 to 30 and 30 to 50.
        let mut before = project();
        retime(&mut before, "c5", &[(0, 1, 1), (10, 1, 1), (30, 1, 1)], 50);
        let shown = pictures(&before, "c5");
        let mut project = before.clone();
        let done = edit(&mut project, reverse("c5", 15));
        assert!(!done.clamped);
        assert_eq!(
            the(&project, "c5").retime,
            Some(speed_points(&[(0, 1, 1), (10, -1, 1), (30, 1, 1)]))
        );
        // 10 frames up, 20 down and 20 up again: the path is lowest 10 frames below where it
        // begins, and the clip begins there in m1, 10 frames earlier than it did. It is moved
        // there before it is turned.
        assert_eq!(clips(&project, "V2"), rows(&[("c5", 120, 50, 40)]));
        assert!(
            matches!(
                done.done.as_slice(),
                [Op::TrimClip { .. }, Op::SetRetime { .. }]
            ),
            "{:?}",
            done.done
        );
        let path: Vec<i64> = (50..60).chain((40..60).rev()).chain(40..60).collect();
        assert_eq!(pictures(&project, "c5"), path);
        assert_eq!(path[..10], shown[..10]);
        undo(&mut project, &done);
        assert_eq!(project, before);
        // The first segment of several shows what it showed, the last first, and what follows
        // goes on from where it then stops: the clip begins where it did.
        let mut project = before.clone();
        edit(&mut project, reverse("c5", 9));
        assert_eq!(
            the(&project, "c5").retime,
            Some(speed_points(&[(0, -1, 1), (10, 1, 1), (30, 1, 1)]))
        );
        assert_eq!(clips(&project, "V2"), rows(&[("c5", 120, 50, 50)]));
        let path: Vec<i64> = (50..60).rev().chain(50..90).collect();
        assert_eq!(pictures(&project, "c5"), path);
        // c1 begins at the start of m1: a segment has nothing below it to go back over.
        let mut project = super::super::fixture::project();
        edit(&mut project, point("c1", 10));
        assert_eq!(
            refused(project, reverse("c1", 20)),
            EditError::Op(OpError::OutOfSource {
                media: "m1".into(),
                why: "before_start"
            })
        );
    }

    #[test]
    fn a_clip_put_back_to_its_own_speed_shows_once_what_it_showed() {
        // c5 goes 10 frames forward, holds a picture for 10, goes back over the 10, and on at
        // twice the speed: 40 frames of m1 from its frame 50.
        let mut before = project();
        let points = [(0, 1, 1), (10, 0, 1), (20, -1, 1), (30, 2, 1)];
        retime(&mut before, "c5", &points, 50);
        let mut project = before.clone();
        let done = edit(&mut project, reset("c5", false));
        assert!(!done.clamped);
        assert_eq!(done.done, [set_retime("c5", &[(0, 1, 1)], Some(40))]);
        let c5 = the(&project, "c5");
        assert_eq!((c5.speed, c5.reverse, c5.retime), (Speed::ONE, false, None));
        assert_eq!(clips(&project, "V2"), rows(&[("c5", 120, 40, 50)]));
        let once: Vec<i64> = (50..90).collect();
        assert_eq!(pictures(&project, "c5"), once);
        // At its own speed already, nothing is done.
        assert_eq!(edit(&mut project, reset("c5", false)).done, [] as [Op; 0]);
        undo(&mut project, &done);
        assert_eq!(project, before);
        // A clip of one speed, backward and twice as fast, is put back as any other.
        let plain = super::super::fixture::project();
        let mut project = plain.clone();
        retime(&mut project, "c5", &[(0, -2, 1)], 25);
        edit(&mut project, reset("c5", false));
        assert_eq!(project, plain);
        // With a ripple, what follows moves back with the end of the linked clips; without,
        // it stays, and the clip is cut where the next one starts.
        let mut project = plain.clone();
        edit(&mut project, freeze("c1", 40, 20, true));
        edit(&mut project, reset("c1", true));
        assert_eq!(project, plain);
        // c1 twice as fast shows its 100 frames of m1 in 50, and c3 is pulled up to it: at its
        // own speed again it would last 100, and is cut at the 50 that it has.
        edit(&mut project, segment_speed("c1", 0, (2, 1), true));
        let pulled = rows(&[("c1", 0, 50, 0), ("c3", 50, 50, 150)]);
        assert_eq!(clips(&project, "V1"), pulled);
        let done = edit(&mut project, reset("c1", false));
        assert!(done.clamped);
        assert_eq!(clips(&project, "V1"), pulled);
        let c1 = the(&project, "c1");
        assert_eq!((c1.speed, c1.reverse, c1.retime), (Speed::ONE, false, None));
        let first: Vec<i64> = (0..50).collect();
        assert_eq!(pictures(&project, "c1"), first);
    }

    #[test]
    fn a_speed_point_that_is_cleared_makes_one_segment_of_two() {
        // c5 with its frame 10 held for 10 frames more: it lasts 60.
        let mut before = project();
        edit(&mut before, freeze("c5", 10, 10, false));
        let shown = pictures(&before, "c5");
        // Without the speed point where the freeze begins, the first 20 frames show at half
        // the speed what the first 10 showed, and the rest is as it was.
        let mut project = before.clone();
        let done = edit(&mut project, clear("c5", 12));
        assert!(!done.clamped);
        assert_eq!(
            done.done,
            [set_retime("c5", &[(0, 1, 2), (20, 1, 1)], None)]
        );
        assert_eq!(clips(&project, "V2"), rows(&[("c5", 120, 60, 50)]));
        let now = pictures(&project, "c5");
        let slow: Vec<i64> = (0..20).map(|k| 50 + k / 2).collect();
        assert_eq!(now[..20], slow);
        assert_eq!(now[20..], shown[20..]);
        undo(&mut project, &done);
        assert_eq!(project, before);
        // Without the one where it plays on, the last 50 frames go through the 40 frames of
        // m1 that the last 40 showed, and the first 10 are as they were.
        edit(&mut project, clear("c5", 59));
        assert_eq!(
            the(&project, "c5").retime,
            Some(speed_points(&[(0, 1, 1), (10, 4, 5)]))
        );
        assert_eq!(clips(&project, "V2"), rows(&[("c5", 120, 60, 50)]));
        assert_eq!(pictures(&project, "c5")[..10], shown[..10]);
        // The first segment has no speed point to clear.
        assert_eq!(refused(before, clear("c5", 9)), out_of_range("speed_point"));
    }

    #[test]
    fn a_clip_begins_later_where_clearing_a_speed_point_takes_the_lowest_of_its_path_away() {
        // c5 goes 10 frames up, 20 down and 20 up, from frame 60 of m1: it begins at frame 50,
        // the lowest that it shows.
        let mut before = project();
        retime(&mut before, "c5", &[(0, 1, 1), (10, -1, 1), (30, 1, 1)], 50);
        let shown = pictures(&before, "c5");
        assert_eq!((shown[0], shown[29], shown[49]), (60, 50, 69));
        // Without the speed point at frame 30, the way down and the way up are one segment
        // that goes nowhere: the path is lowest where it begins, 10 frames later in m1. The
        // clip is given its speed points before it is moved there.
        let mut project = before.clone();
        let done = edit(&mut project, clear("c5", 35));
        assert_eq!(
            the(&project, "c5").retime,
            Some(speed_points(&[(0, 1, 1), (10, 0, 1)]))
        );
        assert_eq!(clips(&project, "V2"), rows(&[("c5", 120, 50, 60)]));
        assert!(
            matches!(
                done.done.as_slice(),
                [Op::SetRetime { .. }, Op::TrimClip { .. }]
            ),
            "{:?}",
            done.done
        );
        let now = pictures(&project, "c5");
        assert_eq!(now[..10], shown[..10]);
        assert_eq!(now[10..], [70; 40]);
        undo(&mut project, &done);
        assert_eq!(project, before);
    }

    #[test]
    fn a_clip_that_does_not_last_takes_none_of_them() {
        let mut project = project();
        let black = Source::Color {
            color: Rgba([0, 0, 0, 255]),
        };
        project.timelines[0].tracks[1].clips[0] = Clip::new("c5", black, 120, 50);
        project.validate().unwrap();
        for edit in [
            point("c5", 10),
            segment_speed("c5", 10, (2, 1), false),
            freeze("c5", 10, 5, false),
            reverse("c5", 10),
            clear("c5", 10),
            reset("c5", false),
        ] {
            assert_eq!(
                refused(project.clone(), edit.clone()),
                out_of_range("retime_source"),
                "{edit:?}"
            );
        }
    }

    #[test]
    fn the_edits_of_the_retime_controls_are_read_by_their_names() {
        let read = |json: serde_json::Value| serde_json::from_value::<Edit>(json).unwrap();
        // `ripple` is off and `linked` on where they are left out.
        assert_eq!(
            read(serde_json::json!({
                "edit": "add_speed_point", "timeline": "t1", "clip": "c5", "at": 20,
            })),
            point("c5", 20)
        );
        assert_eq!(
            read(serde_json::json!({
                "edit": "set_segment_speed", "timeline": "t1", "clip": "c5", "at": 20,
                "speed": { "num": 1, "den": 2 },
            })),
            segment_speed("c5", 20, (1, 2), false)
        );
        assert_eq!(
            read(serde_json::json!({
                "edit": "freeze_frame", "timeline": "t1", "clip": "c5", "at": 20, "hold": 5,
                "ripple": true,
            })),
            freeze("c5", 20, 5, true)
        );
        assert_eq!(
            read(serde_json::json!({
                "edit": "reverse_segment", "timeline": "t1", "clip": "c5", "at": 20,
            })),
            reverse("c5", 20)
        );
        assert_eq!(
            read(serde_json::json!({
                "edit": "clear_speed_point", "timeline": "t1", "clip": "c5", "at": 20,
            })),
            clear("c5", 20)
        );
        assert_eq!(
            read(serde_json::json!({
                "edit": "reset_speed", "timeline": "t1", "clip": "c5", "ripple": true,
            })),
            reset("c5", true)
        );
        // A speed is taken as it is written, and a field that an edit has not is refused.
        let odd = serde_json::json!({
            "edit": "set_segment_speed", "timeline": "t1", "clip": "c5", "at": 0,
            "speed": { "num": 24000, "den": 1001 },
        });
        assert_eq!(read(odd), segment_speed("c5", 0, (24000, 1001), false));
        let unknown = serde_json::json!({
            "edit": "reset_speed", "timeline": "t1", "clip": "c5", "at": 3,
        });
        assert!(serde_json::from_value::<Edit>(unknown).is_err());
    }
}
