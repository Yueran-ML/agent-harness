//! Automation Follows Edit (T15; Reference Manual 20, pp. 3910–3911): the lanes of a track of
//! sound go with the clips that an edit moves, ripples or takes away, and stay where they are
//! when the switch is off; and the edits of clips that the groups of tracks linking Editing take
//! to their other tracks (p. 3917).

#[path = "common/edits.rs"]
#[allow(
    dead_code,
    reason = "the generators of the edits, of which these tests use some"
)]
mod gen;

use std::collections::BTreeSet;

use columbina_model::audio::automation::{fixed_value, paths, view, Owner};
use columbina_model::{
    apply, Clip, Edit, History, Lane, LanePoint, Project, Time, Track, TrackKind,
};
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;
use serde_json::{json, Value};

const SECOND: i64 = 48000;

/// Timeline t1 at 25 frames a second and 48 kHz: V1 empty; A1 with c2 [50, 100) (2–4 s) and e
/// [100, 150) (4–6 s); A2 with d [50, 100). A1 has a lane of its fader: 0 dB at 1 s, −6 at 3 s,
/// −12 at 5 s and 0 at 7 s.
fn project() -> Project {
    let mut project = Project::sample();
    let of = |id: &str, start: i64, from: i64| {
        Clip::of_media(
            id,
            "m1",
            Time {
                ticks: from * 15360 / 25,
                scale: 15360,
            },
            start,
            50,
        )
    };
    let timeline = &mut project.timelines[0];
    timeline.tracks[0].clips.clear();
    timeline.tracks[1].clips = vec![of("c2", 50, 0), of("e", 100, 100)];
    timeline
        .tracks
        .push(Track::new("A2", TrackKind::Audio).with_clips(vec![of("d", 50, 0)]));
    project.next_id = 10;
    project.validate().unwrap();
    lane(
        &mut project,
        "A1",
        "fader_db",
        &[(1, 0.0), (3, -6.0), (5, -12.0), (7, 0.0)],
    );
    project
}

fn lane(project: &mut Project, track: &str, path: &str, points: &[(i64, f64)]) {
    let points: Vec<Value> = points
        .iter()
        .map(|&(s, v)| json!({"at": {"ticks": s * SECOND, "scale": SECOND}, "value": v}))
        .collect();
    op(
        project,
        json!({"op": "set_lane", "timeline": "t1", "of": {"track": track}, "path": path,
            "lane": {"points": points}}),
    );
}

fn op(project: &mut Project, op: Value) {
    apply(project, serde_json::from_value(op).unwrap()).unwrap();
}

fn edit(project: &mut Project, edit: Value) {
    let edit: Edit = serde_json::from_value(edit).unwrap();
    columbina_model::expand(project, edit.clone()).unwrap_or_else(|e| panic!("{edit:?}: {e}"));
}

fn track<'p>(project: &'p Project, id: &str) -> &'p Track {
    project.timelines[0]
        .tracks
        .iter()
        .find(|r| r.id == id)
        .unwrap()
}

fn start_of(project: &Project, clip: &str) -> Option<(String, i64)> {
    project.timelines[0].tracks.iter().find_map(|r| {
        r.clips
            .iter()
            .find(|c| c.id == clip)
            .map(|c| (r.id.clone(), c.start))
    })
}

/// The points of a lane, as seconds (in 48ths of a millisecond where they are not whole) and
/// values.
fn points(project: &Project, id: &str, path: &str) -> Option<Vec<(f64, f64)>> {
    let lane = track(project, id).audio.lanes.get(path)?;
    #[allow(clippy::cast_precision_loss, reason = "small times")]
    let seconds = |p: &LanePoint| p.at.ticks as f64 / f64::from(p.at.scale);
    Some(lane.points.iter().map(|p| (seconds(p), p.value)).collect())
}

fn follows(project: &mut Project, on: bool) {
    op(
        project,
        json!({"op": "set_timeline_audio", "timeline": "t1", "patch": {"follows_edit": on}}),
    );
}

#[test]
fn a_clip_moved_takes_the_automation_over_it_and_leaves_the_values_at_the_ends_behind() {
    // Plan T15: a clip of [2 s, 4 s) moved to [10 s, 12 s) takes (3 s, −6) to (11 s, −6); 2–4 s is
    // left going straight between the values that were at its ends.
    let mut history = History::new(project());
    let before = history.project().clone();
    let moved: Edit = serde_json::from_value(json!({
        "edit": "move", "timeline": "t1", "clips": ["c2"], "delta": 200,
    }))
    .unwrap();
    history.edit(vec![moved], "agent").unwrap();
    let after = history.project().clone();
    assert_eq!(start_of(&after, "c2"), Some(("A1".into(), 250)));
    assert_eq!(
        points(&after, "A1", "fader_db").unwrap(),
        vec![
            (1.0, 0.0),
            (2.0, -3.0),
            (4.0, -9.0),
            (5.0, -12.0),
            (7.0, 0.0),
            (10.0, 0.0),
            (10.0, -3.0),
            (11.0, -6.0),
            (12.0, -9.0),
            (12.0, 0.0),
        ]
    );
    let lane = &track(&after, "A1").audio.lanes["fader_db"];
    let at = |s: i64| {
        lane.value_at(Time {
            ticks: s,
            scale: 48000,
        })
    };
    assert_eq!(at(11 * SECOND), -6.0);
    assert_eq!(at(10 * SECOND + 24000), -4.5);
    // One step of undo takes the lane back as it was, to the bit.
    history.undo("desktop").unwrap().unwrap();
    assert_eq!(
        serde_json::to_value(history.project().timelines[0].tracks[1].audio.lanes.clone()).unwrap(),
        serde_json::to_value(before.timelines[0].tracks[1].audio.lanes.clone()).unwrap()
    );
}

#[test]
fn with_follows_edit_off_no_edit_changes_a_lane() {
    let mut project = project();
    follows(&mut project, false);
    let lanes = track(&project, "A1").audio.lanes.clone();
    edit(
        &mut project,
        json!({"edit": "move", "timeline": "t1", "clips": ["c2"], "delta": 200}),
    );
    edit(
        &mut project,
        json!({"edit": "ripple_delete", "timeline": "t1", "clips": ["e"]}),
    );
    assert_eq!(track(&project, "A1").audio.lanes, lanes);
}

#[test]
fn a_ripple_delete_takes_the_points_inside_away_and_the_rest_back() {
    let mut project = project();
    // d would be in the way of the ripple on A2.
    op(
        &mut project,
        json!({"op": "remove_clip", "timeline": "t1", "id": "d"}),
    );
    edit(
        &mut project,
        json!({"edit": "ripple_delete", "timeline": "t1", "clips": ["c2"]}),
    );
    assert_eq!(start_of(&project, "e"), Some(("A1".into(), 50)));
    assert_eq!(
        points(&project, "A1", "fader_db").unwrap(),
        vec![
            (1.0, 0.0),
            (2.0, -3.0),
            (2.0, -9.0),
            (3.0, -12.0),
            (5.0, 0.0)
        ]
    );
}

#[test]
fn a_lift_leaves_the_values_at_the_ends_of_the_gap_and_moves_nothing() {
    let mut project = project();
    let lift: Edit =
        serde_json::from_value(json!({"edit": "lift", "timeline": "t1", "clips": ["c2"]})).unwrap();
    let done = columbina_model::expand(&mut project, lift).unwrap().done;
    // The lane changes by one splice of the points that change, so that its undo holds them
    // only.
    let lanes: Vec<Value> = done
        .iter()
        .map(|op| serde_json::to_value(op).unwrap())
        .filter(|op| op["op"] == "splice_lane" || op["op"] == "set_lane")
        .collect();
    assert_eq!(lanes.len(), 1);
    assert_eq!(
        (&lanes[0]["op"], &lanes[0]["at"], &lanes[0]["remove"]),
        (&json!("splice_lane"), &json!(1), &json!(1))
    );
    assert_eq!(lanes[0]["points"].as_array().unwrap().len(), 2);
    assert_eq!(
        points(&project, "A1", "fader_db").unwrap(),
        vec![
            (1.0, 0.0),
            (2.0, -3.0),
            (4.0, -9.0),
            (5.0, -12.0),
            (7.0, 0.0)
        ]
    );
}

#[test]
fn an_insert_moves_the_points_after_it_on_and_holds_the_value_over_what_it_puts_in() {
    let mut project = project();
    edit(
        &mut project,
        json!({"edit": "insert", "timeline": "t1", "at": 75, "video_track": null,
            "source": {"kind": "media", "media": "m1", "length": 25}}),
    );
    assert_eq!(
        points(&project, "A1", "fader_db").unwrap(),
        vec![
            (1.0, 0.0),
            (3.0, -6.0),
            (4.0, -6.0),
            (6.0, -12.0),
            (8.0, 0.0)
        ]
    );
}

#[test]
fn a_trim_a_roll_a_slip_and_a_blade_leave_the_lanes_alone() {
    let mut project = project();
    let lanes = track(&project, "A1").audio.lanes.clone();
    for change in [
        json!({"edit": "trim", "timeline": "t1", "clip": "e", "edge": "out", "delta": -10}),
        json!({"edit": "roll", "timeline": "t1", "left": "c2", "right": "e", "delta": 5}),
        json!({"edit": "slip", "timeline": "t1", "clip": "e", "delta": 5}),
        json!({"edit": "blade", "timeline": "t1", "at": 60, "tracks": ["A1"]}),
    ] {
        edit(&mut project, change);
        assert_eq!(track(&project, "A1").audio.lanes, lanes);
    }
}

#[test]
fn a_clip_moved_to_another_track_takes_its_automation_into_a_lane_made_there() {
    let mut project = project();
    op(
        &mut project,
        json!({"op": "remove_clip", "timeline": "t1", "id": "d"}),
    );
    edit(
        &mut project,
        json!({"edit": "move", "timeline": "t1", "clips": ["c2"], "delta": 0, "to_track": "A2"}),
    );
    assert_eq!(start_of(&project, "c2"), Some(("A2".into(), 50)));
    assert_eq!(
        points(&project, "A2", "fader_db").unwrap(),
        vec![
            (2.0, 0.0),
            (2.0, -3.0),
            (3.0, -6.0),
            (4.0, -9.0),
            (4.0, 0.0)
        ]
    );
    assert_eq!(
        points(&project, "A1", "fader_db").unwrap(),
        vec![
            (1.0, 0.0),
            (2.0, -3.0),
            (4.0, -9.0),
            (5.0, -12.0),
            (7.0, 0.0)
        ]
    );
}

fn grouped(project: &mut Project, linked: &Value, active: bool) {
    op(
        project,
        json!({"op": "add_track_group", "timeline": "t1", "group": {"id": "", "name": "Takes",
            "members": ["A1", "A2"], "linked": linked, "active": active}}),
    );
}

#[test]
fn a_move_of_a_clip_of_a_group_that_links_editing_moves_the_clips_of_its_other_tracks_in_one_step()
{
    let mut history = History::new({
        let mut project = project();
        grouped(&mut project, &json!(["editing"]), true);
        project
    });
    let before = history.project().clone();
    let moved: Edit = serde_json::from_value(json!({
        "edit": "move", "timeline": "t1", "clips": ["c2"], "delta": 200,
    }))
    .unwrap();
    history.edit(vec![moved], "agent").unwrap();
    assert_eq!(start_of(history.project(), "d"), Some(("A2".into(), 250)));
    assert_eq!(start_of(history.project(), "c2"), Some(("A1".into(), 250)));
    history.undo("desktop").unwrap().unwrap();
    assert_eq!(start_of(history.project(), "d"), start_of(&before, "d"));
    assert_eq!(start_of(history.project(), "c2"), start_of(&before, "c2"));
}

#[test]
fn a_group_that_is_off_or_links_no_editing_or_a_locked_member_takes_no_clip_along() {
    for (linked, active, locked) in [
        (json!(["fader", "mute"]), true, false),
        (json!(["editing"]), false, false),
        (json!(["editing"]), true, true),
    ] {
        let mut project = project();
        grouped(&mut project, &linked, active);
        if locked {
            op(
                &mut project,
                json!({"op": "set_track", "timeline": "t1", "id": "A2", "locked": true}),
            );
        }
        edit(
            &mut project,
            json!({"edit": "move", "timeline": "t1", "clips": ["c2"], "delta": 200}),
        );
        assert_eq!(
            start_of(&project, "d"),
            Some(("A2".into(), 50)),
            "{active} {locked}"
        );
    }
}

#[test]
fn a_lift_a_ripple_delete_and_a_blade_of_a_track_of_a_group_take_its_other_tracks_along() {
    let mut project = project();
    grouped(&mut project, &json!(["editing"]), true);
    let mut lifted = project.clone();
    edit(
        &mut lifted,
        json!({"edit": "lift", "timeline": "t1", "clips": ["c2"]}),
    );
    assert_eq!(start_of(&lifted, "d"), None);
    let mut deleted = project.clone();
    edit(
        &mut deleted,
        json!({"edit": "ripple_delete", "timeline": "t1", "clips": ["e"]}),
    );
    // d is under e's neighbour, not e: only what overlaps the clip named goes with it.
    assert_eq!(start_of(&deleted, "d"), Some(("A2".into(), 50)));
    let mut bladed = project.clone();
    edit(
        &mut bladed,
        json!({"edit": "blade", "timeline": "t1", "at": 75, "tracks": ["A1"]}),
    );
    assert_eq!(track(&bladed, "A2").clips.len(), 2);
    let mut cut = project.clone();
    edit(
        &mut cut,
        json!({"edit": "ripple_delete_range", "timeline": "t1", "from": 60, "to": 70,
            "tracks": ["A1"]}),
    );
    let lengths: Vec<i64> = track(&cut, "A2").clips.iter().map(|c| c.length).collect();
    assert_eq!(lengths, vec![10, 30]);
}

#[test]
fn a_trim_of_a_track_of_a_group_does_not_take_its_other_tracks_along() {
    let mut project = project();
    grouped(&mut project, &json!(["editing"]), true);
    edit(
        &mut project,
        json!({"edit": "trim", "timeline": "t1", "clip": "c2", "edge": "in", "delta": 10}),
    );
    assert_eq!(track(&project, "A2").clips[0].start, 50);
}

// The five properties of the plan, over random timelines with lanes on A1 and A2 (a fader, a
// mute, a pan and a frequency of EQ, with jumps) and random sequences of every edit that
// `common/edits.rs` makes.

/// A lane, or the setting as it is set where there is none.
#[derive(Clone, Debug)]
enum Curve {
    Lane(Lane),
    Fixed(f64),
}

impl Curve {
    fn at(&self, ticks: i64, scale: u32) -> f64 {
        match self {
            Curve::Lane(lane) => lane.value_at(Time { ticks, scale }),
            Curve::Fixed(value) => *value,
        }
    }

    fn values(&self) -> Vec<f64> {
        match self {
            Curve::Lane(lane) => lane.points.iter().map(|p| p.value).collect(),
            Curve::Fixed(value) => vec![*value],
        }
    }
}

fn owner(track: &Track) -> Owner<'_> {
    Owner::Strip {
        strip: &track.audio.strip,
        mute: track.audio.mute,
        bus: false,
    }
}

fn sound_tracks(project: &Project) -> impl Iterator<Item = &Track> {
    project.timelines[0]
        .tracks
        .iter()
        .filter(|r| r.kind == TrackKind::Audio)
}

/// The curve of `path` on `track`: its lane, or its setting; none where it has no such setting.
fn curve(track: &Track, path: &str) -> Option<Curve> {
    if let Some(lane) = track.audio.lanes.get(path) {
        return Some(Curve::Lane(lane.clone()));
    }
    fixed_value(&view(owner(track)), path).map(Curve::Fixed)
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * (1.0 + a.abs().max(b.abs()))
}

/// The tick of frame `frame` of t1 in ticks of 48 kHz, as a lane counts it.
fn tick(project: &Project, frame: i64) -> i64 {
    project.timelines[0].rate.ticks_of(frame, SCALE)
}

const SCALE: u32 = 48000;

/// The ticks to look at within `from..to`: its ends, the ticks of points near it and some between.
fn ticks_within(curves: &[&Curve], from: i64, to: i64) -> Vec<i64> {
    let mut ticks: Vec<i64> = (0..=16).map(|k| from + (to - from) * k / 16).collect();
    for curve in curves {
        if let Curve::Lane(lane) = curve {
            for p in &lane.points {
                ticks.extend([p.at.ticks - 1, p.at.ticks, p.at.ticks + 1]);
            }
        }
    }
    ticks.retain(|t| (from..to).contains(t));
    ticks.sort_unstable();
    ticks.dedup();
    ticks
}

/// Every audio clip of t1 with its track.
fn sound_clips(project: &Project) -> Vec<(&Track, &Clip)> {
    sound_tracks(project)
        .flat_map(|r| r.clips.iter().map(move |c| (r, c)))
        .collect()
}

/// The edits that ripple what is after their place.
fn ripples(edit: &Edit) -> bool {
    matches!(
        edit,
        Edit::Insert { .. }
            | Edit::SmartInsert { .. }
            | Edit::RippleOverwrite { .. }
            | Edit::RippleDelete { .. }
            | Edit::RippleDeleteRange { .. }
            | Edit::CloseGap { .. }
            | Edit::InsertMove { .. }
            | Edit::Trim { ripple: true, .. }
            | Edit::SetSpeed { ripple: true, .. }
    )
}

/// Apply the edits that `makings` make one after another, with Follows Edit on or off, and look
/// at each that is applied: the project before and after it.
fn each_applied(
    recipe: &gen::Recipe,
    makings: &[gen::Making],
    follows_edit: bool,
    mut look: impl FnMut(&Project, &Edit, &Project) -> Result<(), TestCaseError>,
) -> Result<(), TestCaseError> {
    let mut project = gen::project_of(recipe);
    follows(&mut project, follows_edit);
    for making in makings {
        let edit = gen::edit_of(&project, making);
        let before = project.clone();
        if columbina_model::expand(&mut project, edit.clone()).is_ok() {
            look(&before, &edit, &project)?;
        }
    }
    Ok(())
}

/// Whether a clip is as it was but for where it is.
fn rigid(a: &Clip, b: &Clip) -> bool {
    (a.length, &a.source, a.speed, a.reverse) == (b.length, &b.source, b.speed, b.reverse)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1024))]

    /// 1. The lanes over a clip that an edit moves whole, by `delta` frames and to any track,
    /// are where it is now, tick for tick (but for the last tick of the clip, which the start of
    /// a clip after it that moved with it by a tick more or less may take at a rate of frames
    /// whose frames are not whole ticks).
    #[test]
    fn the_lanes_over_a_clip_move_with_it(
        recipe in gen::recipe(),
        makings in prop::collection::vec(gen::making_of(0usize..28), 1..6),
    ) {
        each_applied(&recipe, &makings, true, |before, edit, after| {
            for (track, clip) in sound_clips(before) {
                let Some((now_on, now)) = sound_clips(after).into_iter().find(|(_, c)| c.id == clip.id) else {
                    continue;
                };
                if !rigid(clip, now) {
                    continue;
                }
                let (from, to) = (tick(before, clip.start), tick(after, now.start));
                let length = (tick(before, clip.end()) - from).min(tick(after, now.end()) - to) - 1;
                for (path, lane) in &track.audio.lanes {
                    let Some(moved) = now_on.audio.lanes.get(path) else {
                        // Only a track that cannot have the setting is left without its lane.
                        prop_assert!(
                            now_on.id != track.id && !paths(owner(now_on)).contains(path),
                            "{} lost {} over {} in {:?}", now_on.id, path, clip.id, edit
                        );
                        continue;
                    };
                    let was = Curve::Lane(lane.clone());
                    for k in ticks_within(&[&was], from, from + length) {
                        let k = k - from;
                        let (a, b) = (was.at(from + k, SCALE), moved.value_at(Time { ticks: to + k, scale: SCALE }));
                        prop_assert!(near(a, b), "{}.{} over {} at +{}: {} then {} in {:?}", track.id, path, clip.id, k, a, b, edit);
                    }
                }
            }
            Ok(())
        })?;
    }

    /// 2. Outside the clips that an edit changes, before and after it, and, of an edit that
    /// ripples, before the first of them (or the gap that it closes), every lane is as it was.
    #[test]
    fn the_lanes_outside_what_an_edit_changes_stay_as_they_were(
        recipe in gen::recipe(),
        makings in prop::collection::vec(gen::making_of(0usize..28), 1..6),
    ) {
        each_applied(&recipe, &makings, true, |before, edit, after| {
            let all_before = sound_clips(before);
            let all_after = sound_clips(after);
            let place = |clips: &[(&Track, &Clip)], id: &str| {
                clips.iter().find(|(_, c)| c.id == id).map(|(r, c)| (r.id.clone(), c.start, c.length))
            };
            let mut spans: Vec<(String, i64, i64)> = vec![];
            for (clips, other, project) in [(&all_before, &all_after, before), (&all_after, &all_before, after)] {
                for (r, c) in clips {
                    if place(clips, &c.id) != place(other, &c.id) {
                        spans.push((r.id.clone(), tick(project, c.start), tick(project, c.end())));
                    }
                }
            }
            // What ripples moves from the first clip of any kind that the edit changes.
            let every = |p: &Project| -> Vec<(String, i64, i64)> {
                p.timelines[0].tracks.iter().flat_map(|r| r.clips.iter()).map(|c| (c.id.clone(), c.start, c.length)).collect()
            };
            let (was, is) = (every(before), every(after));
            let mut first = was.iter().filter(|c| !is.contains(c)).chain(is.iter().filter(|c| !was.contains(c)))
                .map(|c| tick(before, c.1)).min().unwrap_or(i64::MAX);
            if let Edit::CloseGap { track, at, .. } = edit {
                let ends = before.timelines[0].tracks.iter().filter(|r| r.id == *track)
                    .flat_map(|r| r.clips.iter().map(Clip::end).chain(r.cues.iter().map(|q| q.start + q.length)));
                first = first.min(tick(before, ends.filter(|e| e <= at).max().unwrap_or(0)));
            }
            // An insert move takes its clips out and puts them back in from `at`, which may be
            // where they were.
            if let Edit::InsertMove { clips, at, linked, .. } = edit {
                let moved = gen::worked_on(before, clips, *linked);
                let starts = before.timelines[0].tracks.iter().flat_map(|r| &r.clips)
                    .filter(|c| moved.contains(&c.id)).map(|c| c.start);
                first = first.min(tick(before, starts.min().unwrap_or(*at).min(*at)));
            }
            if let Edit::RippleDeleteRange { from, .. } = edit {
                first = first.min(tick(before, *from));
            }
            for track in sound_tracks(before) {
                let Some(now) = sound_tracks(after).find(|r| r.id == track.id) else { continue };
                let touched = |t: i64| {
                    (ripples(edit) && t >= first)
                        || spans.iter().any(|(r, from, to)| *r == track.id && (*from..=*to).contains(&t))
                };
                let paths: BTreeSet<&String> = track.audio.lanes.keys().chain(now.audio.lanes.keys()).collect();
                for path in paths {
                    let (Some(was), Some(is)) = (curve(track, path), curve(now, path)) else { continue };
                    for t in ticks_within(&[&was, &is], 0, 200 * 48000) {
                        if touched(t) {
                            continue;
                        }
                        let (a, b) = (was.at(t, SCALE), is.at(t, SCALE));
                        prop_assert!(near(a, b), "{}.{} at {}: {} then {} in {:?}", track.id, path, t, a, b, edit);
                    }
                }
            }
            Ok(())
        })?;
    }

    /// 3. An edit undone leaves every lane as it was, to the bit.
    #[test]
    fn an_edit_undone_leaves_the_lanes_as_they_were(
        recipe in gen::recipe(),
        makings in prop::collection::vec(gen::making_of(0usize..28), 1..6),
    ) {
        let mut history = History::new(gen::project_of(&recipe));
        for making in &makings {
            let before = history.project().clone();
            let edit = gen::edit_of(&before, making);
            if history.edit(vec![edit.clone()], "agent").is_err() {
                continue;
            }
            history.undo("desktop").unwrap().unwrap();
            let lanes = |p: &Project| serde_json::to_string(
                &sound_tracks(p).map(|r| (&r.id, &r.audio.lanes)).collect::<Vec<_>>()).unwrap();
            prop_assert_eq!(lanes(history.project()), lanes(&before), "{:?} undone", edit);
            history.redo("desktop").unwrap().unwrap();
        }
    }

    /// 4. With Follows Edit off, no edit changes a lane of a track that it leaves.
    #[test]
    fn with_follows_edit_off_the_lanes_stay_as_they_are(
        recipe in gen::recipe(),
        makings in prop::collection::vec(gen::making_of(0usize..28), 1..6),
    ) {
        each_applied(&recipe, &makings, false, |before, edit, after| {
            for track in sound_tracks(before) {
                if let Some(now) = sound_tracks(after).find(|r| r.id == track.id) {
                    prop_assert_eq!(&now.audio.lanes, &track.audio.lanes, "{:?}", edit);
                }
            }
            Ok(())
        })?;
    }

    /// 5. No edit gives a lane a value beyond the least and the greatest that it had (or its
    /// setting had, where it had none), or that the lanes of the setting on the other tracks of
    /// sound had where a clip went from one track to another.
    #[test]
    fn no_edit_takes_a_lane_beyond_its_values(
        recipe in gen::recipe(),
        makings in prop::collection::vec(gen::making_of(0usize..28), 1..6),
    ) {
        each_applied(&recipe, &makings, true, |before, edit, after| {
            let track_of = |p: &Project, id: &str| sound_clips(p).into_iter().find(|(_, c)| c.id == id).map(|(r, _)| r.id.clone());
            let across = sound_clips(after).iter().any(|(r, c)| track_of(before, &c.id).is_some_and(|t| t != r.id));
            for now in sound_tracks(after) {
                for (path, lane) in &now.audio.lanes {
                    let mut allowed: Vec<f64> = sound_tracks(before)
                        .filter(|r| r.id == now.id || across)
                        .filter_map(|r| curve(r, path))
                        .flat_map(|c| c.values())
                        .collect();
                    if allowed.is_empty() {
                        allowed = curve(now, path).map(|c| c.values()).unwrap_or_default();
                    }
                    let least = allowed.iter().copied().fold(f64::INFINITY, f64::min);
                    let most = allowed.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    for p in &lane.points {
                        prop_assert!(least <= p.value && p.value <= most,
                            "{}.{}: {} beyond {}..{} in {:?}", now.id, path, p.value, least, most, edit);
                    }
                }
            }
            Ok(())
        })?;
    }
}

#[test]
fn a_clip_replaced_in_place_leaves_its_automation_as_it_is() {
    let mut project = project();
    let lanes = track(&project, "A1").audio.lanes.clone();
    edit(
        &mut project,
        json!({"edit": "replace", "timeline": "t1", "at": 60, "video_track": null,
            "source": {"kind": "media", "media": "m1"}}),
    );
    assert_eq!(track(&project, "A1").audio.lanes, lanes);
}

#[test]
fn a_ripple_trim_of_a_clip_whose_linked_sound_runs_on_cuts_its_lane_at_the_end_of_the_sound() {
    // V1 c1 [50, 100) is linked to A1's c2, made to run to 110; e after it at 110 on A1. A
    // ripple trim of c1's end by −10 ends the group's clips 10 frames sooner: on A1 the frames
    // taken away are the last of c2, [100, 110), not those before c1's end.
    let mut project = project();
    op(
        &mut project,
        json!({"op": "remove_clip", "timeline": "t1", "id": "d"}),
    );
    op(
        &mut project,
        json!({"op": "move_clip", "timeline": "t1", "id": "e", "track": "A1", "start": 110}),
    );
    op(
        &mut project,
        json!({"op": "trim_clip", "timeline": "t1", "id": "c2",
            "source_in": {"ticks": 0, "scale": 15360}, "start": 50, "length": 60}),
    );
    let mut picture = Clip::of_media(
        "c1",
        "m1",
        Time {
            ticks: 0,
            scale: 15360,
        },
        50,
        50,
    );
    picture.link = Some("g1".into());
    op(
        &mut project,
        json!({"op": "insert_clip", "timeline": "t1", "track": "V1", "clip": picture}),
    );
    op(
        &mut project,
        json!({"op": "set_clip", "timeline": "t1", "id": "c2", "link": "g1"}),
    );
    let lane = track(&project, "A1").audio.lanes["fader_db"].clone();
    edit(
        &mut project,
        json!({"edit": "trim", "timeline": "t1", "clip": "c1", "edge": "out", "delta": -10,
            "ripple": true}),
    );
    assert_eq!(start_of(&project, "e"), Some(("A1".into(), 100)));
    let now = &track(&project, "A1").audio.lanes["fader_db"];
    let value = |lane: &Lane, s: f64| {
        #[allow(clippy::cast_possible_truncation, reason = "whole ticks")]
        let ticks = (s * 48000.0) as i64;
        lane.value_at(Time {
            ticks,
            scale: 48000,
        })
    };
    // What c2 keeps, to 4 s, is as it was; e's automation came back 0.4 s with it.
    for s in [2.0, 3.0, 3.9] {
        assert!((value(now, s) - value(&lane, s)).abs() < 1e-9, "{s}");
    }
    for s in [4.0, 4.5, 5.0, 6.0] {
        assert!((value(now, s) - value(&lane, s + 0.4)).abs() < 1e-9, "{s}");
    }
}

#[test]
fn a_lane_that_the_track_moved_to_cannot_have_stays_behind_and_the_move_is_made() {
    let mut project = project();
    op(
        &mut project,
        json!({"op": "remove_clip", "timeline": "t1", "id": "d"}),
    );
    op(
        &mut project,
        json!({"op": "add_insert", "timeline": "t1", "of": {"track": "A1"},
            "insert": {"effect": {"kind": "gain", "gain_db": 0}}}),
    );
    lane(
        &mut project,
        "A1",
        "insert.fx1.gain_db",
        &[(1, 0.0), (3, 6.0)],
    );
    edit(
        &mut project,
        json!({"edit": "move", "timeline": "t1", "clips": ["c2"], "delta": 0, "to_track": "A2"}),
    );
    assert_eq!(start_of(&project, "c2"), Some(("A2".into(), 50)));
    assert!(points(&project, "A2", "insert.fx1.gain_db").is_none());
    assert!(points(&project, "A2", "fader_db").is_some());
    assert_eq!(
        points(&project, "A1", "insert.fx1.gain_db").unwrap(),
        vec![(1.0, 0.0), (2.0, 3.0), (4.0, 6.0)]
    );
}
