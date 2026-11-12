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
/// −8 at 5 s and 0 at 7 s.
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
        &[(1, 0.0), (3, -6.0), (5, -8.0), (7, 0.0)],
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
            (4.0, -7.0),
            (5.0, -8.0),
            (7.0, 0.0),
            (10.0, 0.0),
            (10.0, -3.0),
            (11.0, -6.0),
            (12.0, -7.0),
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
            (2.0, -7.0),
            (3.0, -8.0),
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
            (4.0, -7.0),
            (5.0, -8.0),
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
            (6.0, -8.0),
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
            (4.0, -7.0),
            (4.0, 0.0)
        ]
    );
    assert_eq!(
        points(&project, "A1", "fader_db").unwrap(),
        vec![
            (1.0, 0.0),
            (2.0, -3.0),
            (4.0, -7.0),
            (5.0, -8.0),
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

/// A track A2 with a music bed m [0, 250) in place of d.
fn with_a_bed(project: &mut Project) {
    op(
        project,
        json!({"op": "remove_clip", "timeline": "t1", "id": "d"}),
    );
    let bed = Clip::of_media(
        "m",
        "m1",
        Time {
            ticks: 0,
            scale: 15360,
        },
        0,
        250,
    );
    op(
        project,
        json!({"op": "insert_clip", "timeline": "t1", "track": "A2", "clip": bed}),
    );
}

fn spans(project: &Project, id: &str) -> Vec<(i64, i64)> {
    track(project, id)
        .clips
        .iter()
        .map(|c| (c.start, c.end()))
        .collect()
}

#[test]
fn a_grouped_move_of_a_clip_moves_what_its_other_tracks_have_over_its_frames_in_one_step() {
    let mut history = History::new({
        let mut project = project();
        grouped(&mut project, &json!(["editing"]), true);
        project
    });
    let before = history.project().clone();
    let moved: Edit = serde_json::from_value(json!({
        "edit": "move", "timeline": "t1", "clips": ["c2"], "delta": 200, "grouped": true,
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
fn a_grouped_move_takes_only_the_frames_of_the_clip_from_a_longer_clip_of_another_track() {
    let mut project = project();
    with_a_bed(&mut project);
    grouped(&mut project, &json!(["editing"]), true);
    edit(
        &mut project,
        json!({"edit": "move", "timeline": "t1", "clips": ["c2"], "delta": 200, "grouped": true}),
    );
    assert_eq!(spans(&project, "A2"), vec![(0, 50), (100, 250), (250, 300)]);
    assert_eq!(start_of(&project, "m"), Some(("A2".into(), 0)));
}

#[test]
fn a_grouped_move_to_another_track_takes_no_other_track_along() {
    let mut project = project();
    grouped(&mut project, &json!(["editing"]), true);
    edit(
        &mut project,
        json!({"edit": "move", "timeline": "t1", "clips": ["c2"], "delta": 200,
            "to_track": "A2", "grouped": true}),
    );
    assert_eq!(start_of(&project, "c2"), Some(("A2".into(), 250)));
    assert_eq!(start_of(&project, "d"), Some(("A2".into(), 50)));
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
            json!({"edit": "move", "timeline": "t1", "clips": ["c2"], "delta": 200,
                "grouped": true}),
        );
        assert_eq!(
            start_of(&project, "d"),
            Some(("A2".into(), 50)),
            "{active} {locked}"
        );
    }
}

#[test]
fn edits_not_told_grouped_take_no_other_track_of_a_group_along() {
    // What the desktop sends for an Alt+drag duplicate (a lift to clear where the copies go) and
    // for Backspace over marks (blades of the tracks given) is left as it was.
    let mut project = project();
    grouped(&mut project, &json!(["editing"]), true);
    let mut lifted = project.clone();
    edit(
        &mut lifted,
        json!({"edit": "lift", "timeline": "t1", "clips": ["c2"], "linked": false}),
    );
    assert_eq!(start_of(&lifted, "d"), Some(("A2".into(), 50)));
    let mut bladed = project.clone();
    edit(
        &mut bladed,
        json!({"edit": "blade", "timeline": "t1", "at": 75, "tracks": ["A1"], "linked": false}),
    );
    assert_eq!(spans(&bladed, "A2"), vec![(50, 100)]);
    let mut moved = project.clone();
    edit(
        &mut moved,
        json!({"edit": "move", "timeline": "t1", "clips": ["c2"], "delta": 200}),
    );
    assert_eq!(start_of(&moved, "d"), Some(("A2".into(), 50)));
}

#[test]
fn a_grouped_lift_of_two_seconds_of_dialogue_leaves_the_music_bed_outside_them() {
    let mut project = project();
    with_a_bed(&mut project);
    grouped(&mut project, &json!(["editing"]), true);
    edit(
        &mut project,
        json!({"edit": "lift", "timeline": "t1", "clips": ["c2"], "grouped": true}),
    );
    assert_eq!(start_of(&project, "c2"), None);
    assert_eq!(spans(&project, "A2"), vec![(0, 50), (100, 250)]);
    assert_eq!(start_of(&project, "m"), Some(("A2".into(), 0)));
}

#[test]
fn a_grouped_lift_a_ripple_delete_and_a_blade_take_the_other_tracks_of_a_group_along() {
    let mut project = project();
    grouped(&mut project, &json!(["editing"]), true);
    let mut lifted = project.clone();
    edit(
        &mut lifted,
        json!({"edit": "lift", "timeline": "t1", "clips": ["c2"], "grouped": true}),
    );
    assert_eq!(start_of(&lifted, "d"), None);
    let mut deleted = project.clone();
    edit(
        &mut deleted,
        json!({"edit": "ripple_delete", "timeline": "t1", "clips": ["e"], "grouped": true}),
    );
    // d is under e's neighbour, not e: only the frames of the clip named go.
    assert_eq!(start_of(&deleted, "d"), Some(("A2".into(), 50)));
    let mut bladed = project.clone();
    edit(
        &mut bladed,
        json!({"edit": "blade", "timeline": "t1", "at": 75, "tracks": ["A1"], "grouped": true}),
    );
    assert_eq!(track(&bladed, "A2").clips.len(), 2);
    let mut cut = project.clone();
    edit(
        &mut cut,
        json!({"edit": "ripple_delete_range", "timeline": "t1", "from": 60, "to": 70,
            "tracks": ["A1"], "grouped": true}),
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

/// Sequences of edits of clips: every kind of `common/edits.rs` that is not of sound, and copies
/// put now and then.
fn edits() -> impl Strategy<Value = Vec<gen::Making>> {
    prop::collection::vec(
        gen::making_of(prop_oneof![9 => 0usize..28, 1 => Just(gen::PLACE_COPIES)]),
        1..6,
    )
}

/// Whether a frame of t1 lasts a whole number of ticks of 48 kHz.
fn whole_ticks(project: &Project) -> bool {
    let rate = project.timelines[0].rate;
    (u64::from(SCALE) * u64::from(rate.den)).is_multiple_of(u64::from(rate.num))
}

/// Whether a clip is as it was but for where it is.
fn rigid(a: &Clip, b: &Clip) -> bool {
    (a.length, &a.source, a.speed, a.reverse) == (b.length, &b.source, b.speed, b.reverse)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1024))]

    /// 1. The lanes over a clip that an edit moves whole, by `delta` frames and to any track,
    /// are where it is now, tick for tick; but, at a rate of frames whose frames are not whole
    /// ticks, for the last tick of the clip, which the start of a clip after it that moved with
    /// it by a tick more or less may take.
    #[test]
    fn the_lanes_over_a_clip_move_with_it(
        recipe in gen::recipe(),
        makings in edits(),
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
                let last = i64::from(!whole_ticks(before));
                let length = (tick(before, clip.end()) - from).min(tick(after, now.end()) - to) - last;
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
        makings in edits(),
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

    /// 2b. After the place of a ripple that moves what follows it by one number of frames, the
    /// lanes of the tracks that it ripples are where what follows went (within a tick, as frames
    /// that are not whole ticks move by ticks a tick apart), and the lanes of a locked track are
    /// as they were.
    #[test]
    fn the_lanes_after_a_ripple_move_with_what_follows(
        recipe in gen::recipe(),
        makings in edits(),
    ) {
        each_applied(&recipe, &makings, true, |before, edit, after| {
            for track in sound_tracks(before).filter(|r| r.locked) {
                let now = sound_tracks(after).find(|r| r.id == track.id).unwrap();
                prop_assert_eq!(&now.audio.lanes, &track.audio.lanes, "{:?}", edit);
            }
            if !ripples(edit) || matches!(edit, Edit::InsertMove { .. }) {
                return Ok(());
            }
            let all = |p: &Project| -> Vec<(String, Clip)> {
                p.timelines[0].tracks.iter().flat_map(|r| r.clips.iter().map(move |c| (r.id.clone(), c.clone()))).collect()
            };
            let (was, is) = (all(before), all(after));
            let mut shifts = BTreeSet::new();
            let mut place = i64::MAX;
            for (r, c) in &was {
                if let Some((_, n)) = is.iter().find(|(s, n)| s == r && n.id == c.id && rigid(c, n) && n.start != c.start) {
                    shifts.insert(n.start - c.start);
                    place = place.min(c.start);
                }
            }
            let (Some(&by), 1) = (shifts.iter().next(), shifts.len()) else { return Ok(()) };
            // A ripple delete of clips apart closes each gap: what is after the last moves further.
            if matches!(edit, Edit::RippleDelete { .. }) {
                let mut gone: Vec<(i64, i64)> = was.iter().filter(|(_, c)| !is.iter().any(|(_, n)| n.id == c.id))
                    .map(|(_, c)| (c.start, c.end())).collect();
                gone.sort_unstable();
                let (mut reach, mut parts) = (i64::MIN, 0);
                for (from, to) in gone {
                    parts += usize::from(from > reach);
                    reach = reach.max(to);
                }
                if parts > 1 {
                    return Ok(());
                }
            }
            let moved = tick(before, place + by) - tick(before, place);
            // A ripple of a range of tracks named ripples those only.
            let named = match edit {
                Edit::RippleDeleteRange { tracks: Some(tracks), .. } => Some(tracks),
                _ => None,
            };
            for track in sound_tracks(before).filter(|r| !r.locked && named.is_none_or(|n| n.contains(&r.id))) {
                let now = sound_tracks(after).find(|r| r.id == track.id).unwrap();
                // From the end of what does not move across the place, past what changes: a clip
                // made shorter there ends `by` frames before where its lanes are cut.
                let end = |c: &Clip| {
                    let now = now.clips.iter().find(|n| n.id == c.id);
                    now.map_or(c.end(), |n| c.end().max(n.end() - by))
                };
                let from = track.clips.iter().filter(|c| c.start < place).map(end).fold(place, i64::max);
                let changed: Vec<(i64, i64)> = track.clips.iter()
                    .filter(|c| !now.clips.iter().any(|n| n.id == c.id && rigid(c, n) && n.start == c.start + by))
                    .map(|c| (tick(before, c.start), tick(before, c.end()))).collect();
                for (path, lane) in &track.audio.lanes {
                    let Some(went) = now.audio.lanes.get(path) else { continue };
                    let was = Curve::Lane(lane.clone());
                    for t in ticks_within(&[&was], tick(before, from), 200 * 48000) {
                        if changed.iter().any(|&(a, b)| (a..=b).contains(&t)) {
                            continue;
                        }
                        // Within a tick each way: where two pieces next to each other move by
                        // ticks a tick apart, the one tick between them is the second's.
                        let a = was.at(t, SCALE);
                        let found = (-1..=1).any(|d| (-1..=1).any(|e| {
                            near(was.at(t + e, SCALE), went.value_at(Time { ticks: t + moved + d, scale: SCALE }))
                        }));
                        prop_assert!(found, "{}.{} at {} (by {}): {} in {:?}", track.id, path, t, moved, a, edit);
                    }
                }
            }
            Ok(())
        })?;
    }

    /// 2c. A clip trimmed at its start with a ripple keeps its lanes on what it still plays:
    /// what was over its frame `f` is over its frame `f − delta`.
    #[test]
    fn a_clip_trimmed_at_its_start_with_a_ripple_keeps_its_lanes_on_what_it_plays(
        recipe in gen::recipe(),
        makings in prop::collection::vec(
            prop_oneof![3 => gen::making_of(Just(14usize)), 1 => gen::making()],
            1..6,
        ),
    ) {
        each_applied(&recipe, &makings, true, |before, edit, after| {
            if !matches!(edit, Edit::Trim { edge: columbina_model::Edge::In, ripple: true, .. }) {
                return Ok(());
            }
            for (track, clip) in sound_clips(before) {
                let Some((now_on, now)) = sound_clips(after).into_iter().find(|(r, c)| c.id == clip.id && r.id == track.id) else {
                    continue;
                };
                if now.start != clip.start || now.length == clip.length {
                    continue;
                }
                let delta = clip.length - now.length;
                let kept = clip.start + delta.max(0);
                let (from, to) = (tick(before, kept), tick(after, kept - delta));
                let last = i64::from(!whole_ticks(before));
                let length = (tick(before, clip.end()) - from).min(tick(after, now.end()) - to) - last;
                for (path, lane) in &track.audio.lanes {
                    let Some(moved) = now_on.audio.lanes.get(path) else { continue };
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

    /// 2d. A ripple adds to a lane no more than four points and two for each sloped line of it
    /// that crosses an end of a clip of its track that the edit changes (T15's review: the points
    /// that hold a lane across the starts of clips that move by ticks a tick apart).
    #[test]
    fn a_ripple_adds_few_points_to_a_lane(
        recipe in gen::recipe(),
        makings in edits(),
    ) {
        each_applied(&recipe, &makings, true, |before, edit, after| {
            if !ripples(edit) || matches!(edit, Edit::InsertMove { .. }) {
                return Ok(());
            }
            for track in sound_tracks(before) {
                let Some(now) = sound_tracks(after).find(|r| r.id == track.id) else { continue };
                // The ends of what changes on any track (a ripple cuts every track that it moves
                // where it closes or opens frames), and the frames that the edit names.
                let kept = |c: &Clip| after.timelines[0].tracks.iter().flat_map(|r| &r.clips)
                    .any(|n| n.id == c.id && n.start == c.start && n.length == c.length);
                let mut ends: Vec<i64> = before.timelines[0].tracks.iter().flat_map(|r| &r.clips)
                    .filter(|c| !kept(c))
                    .flat_map(|c| [tick(before, c.start), tick(before, c.end())]).collect();
                match edit {
                    Edit::Insert { at, .. } | Edit::RippleOverwrite { at, .. } | Edit::CloseGap { at, .. } => ends.push(tick(before, *at)),
                    Edit::RippleDeleteRange { from, to, .. } => ends.extend([tick(before, *from), tick(before, *to)]),
                    _ => {}
                }
                if let Edit::CloseGap { track: gap, at, .. } = edit {
                    let spans = before.timelines[0].tracks.iter().filter(|r| r.id == *gap).flat_map(|r| r.clips.iter().map(|c| (c.start, c.end())).chain(r.cues.iter().map(|q| (q.start, q.start + q.length))));
                    let spans: Vec<(i64, i64)> = spans.collect();
                    ends.push(tick(before, spans.iter().map(|s| s.1).filter(|e| e <= at).max().unwrap_or(0)));
                    if let Some(to) = spans.iter().map(|s| s.0).filter(|s| s > at).min() {
                        ends.push(tick(before, to));
                    }
                }
                for (path, lane) in &track.audio.lanes {
                    let Some(went) = now.audio.lanes.get(path) else { continue };
                    let crossed = ends.iter().filter(|&&b| lane.points.windows(2).any(|w| {
                        w[0].at.ticks < b && b <= w[1].at.ticks && w[0].value.to_bits() != w[1].value.to_bits()
                    })).count();
                    let added = went.points.len().saturating_sub(lane.points.len());
                    prop_assert!(added <= 4 + 2 * crossed, "{}.{}: {} points added, {} crossed, in {:?}", track.id, path, added, crossed, edit);
                }
            }
            Ok(())
        })?;
    }

    /// 3. An edit undone leaves every lane as it was, to the bit.
    #[test]
    fn an_edit_undone_leaves_the_lanes_as_they_were(
        recipe in gen::recipe(),
        makings in edits(),
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
        makings in edits(),
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
        makings in edits(),
    ) {
        each_applied(&recipe, &makings, true, |before, edit, after| {
            let track_of = |p: &Project, id: &str| sound_clips(p).into_iter().find(|(_, c)| c.id == id).map(|(r, _)| r.id.clone());
            let across = matches!(edit, Edit::PlaceCopies { .. })
                || sound_clips(after).iter().any(|(r, c)| track_of(before, &c.id).is_some_and(|t| t != r.id));
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

fn value_at(project: &Project, id: &str, path: &str, ticks: i64) -> f64 {
    track(project, id).audio.lanes[path].value_at(Time {
        ticks,
        scale: 48000,
    })
}

#[test]
fn a_ripple_trim_of_a_start_keeps_the_lane_on_what_the_clip_still_plays() {
    // c2 [50, 100) trimmed in by 10 with a ripple plays from 60 of what it played, from 50, and
    // ends at 90: its automation goes back 10 frames with it, and the 10 frames trimmed go.
    let mut project = project();
    op(
        &mut project,
        json!({"op": "remove_clip", "timeline": "t1", "id": "d"}),
    );
    let before = project.clone();
    edit(
        &mut project,
        json!({"edit": "trim", "timeline": "t1", "clip": "c2", "edge": "in", "delta": 10,
            "ripple": true}),
    );
    assert_eq!(spans(&project, "A1"), vec![(50, 90), (90, 140)]);
    let frame = 1920;
    for k in [0, 1000, 20 * frame, 39 * frame, 40 * frame, 89 * frame] {
        let (was, is) = (
            value_at(&before, "A1", "fader_db", 60 * frame + k),
            value_at(&project, "A1", "fader_db", 50 * frame + k),
        );
        assert!((was - is).abs() < 1e-9, "+{k}: {was} then {is}");
    }
    // Before the clip, as it was.
    for t in [0, 48000, 50 * frame - 1] {
        assert!(
            (value_at(&before, "A1", "fader_db", t) - value_at(&project, "A1", "fader_db", t))
                .abs()
                < 1e-9
        );
    }
    // Lengthened by 10 again, it plays from 50 once more, its lane back where it was.
    edit(
        &mut project,
        json!({"edit": "trim", "timeline": "t1", "clip": "c2", "edge": "in", "delta": -10,
            "ripple": true}),
    );
    for k in [0, 10 * frame, 49 * frame] {
        let (was, is) = (
            value_at(&before, "A1", "fader_db", 60 * frame + k),
            value_at(&project, "A1", "fader_db", 60 * frame + k),
        );
        assert!((was - is).abs() < 1e-9, "+{k}: {was} then {is}");
    }
}

#[test]
fn a_linked_clip_that_starts_after_the_place_of_a_ripple_trim_keeps_its_lane() {
    // V1 c1 [0, 50) is linked to A1's c2 [50, 100), which starts where c1 ends (an L cut). A
    // ripple trim of c1's end by −10 trims c2 too, which stays where it starts.
    let mut project = project();
    op(
        &mut project,
        json!({"op": "remove_clip", "timeline": "t1", "id": "d"}),
    );
    let mut picture = Clip::of_media(
        "c1",
        "m1",
        Time {
            ticks: 0,
            scale: 15360,
        },
        0,
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
    let before = project.clone();
    edit(
        &mut project,
        json!({"edit": "trim", "timeline": "t1", "clip": "c1", "edge": "out", "delta": -10,
            "ripple": true}),
    );
    assert_eq!(spans(&project, "A1"), vec![(50, 90), (90, 140)]);
    let frame = 1920;
    for k in [0, 1000, 20 * frame, 39 * frame] {
        let (was, is) = (
            value_at(&before, "A1", "fader_db", 50 * frame + k),
            value_at(&project, "A1", "fader_db", 50 * frame + k),
        );
        assert!((was - is).abs() < 1e-9, "c2 +{k}: {was} then {is}");
    }
    for k in [0, 25 * frame, 49 * frame] {
        let (was, is) = (
            value_at(&before, "A1", "fader_db", 100 * frame + k),
            value_at(&project, "A1", "fader_db", 90 * frame + k),
        );
        assert!((was - is).abs() < 1e-9, "e +{k}: {was} then {is}");
    }
}

#[test]
fn a_ripple_at_29_97_frames_a_second_adds_points_only_where_a_slope_crosses_a_moved_clip() {
    let mut project = project();
    let timeline = &mut project.timelines[0];
    timeline.rate = columbina_model::Rate {
        num: 30000,
        den: 1001,
    };
    timeline.tracks[1].clips = (0..40)
        .map(|n| {
            Clip::of_media(
                format!("k{n}"),
                "m1",
                Time {
                    ticks: 0,
                    scale: 15360,
                },
                n * 35,
                30,
            )
        })
        .collect();
    timeline.tracks[2].clips.clear();
    project.validate().unwrap();
    lane(
        &mut project,
        "A1",
        "fader_db",
        &[(0, 0.0), (1, -6.0), (2, 0.0), (50, 0.0)],
    );
    for n in 0..5 {
        edit(
            &mut project,
            json!({"edit": "insert", "timeline": "t1", "at": 0, "video_track": null,
                "audio_track": "A1", "source": {"kind": "media", "media": "m1", "length": 1}}),
        );
        let count = track(&project, "A1").audio.lanes["fader_db"].points.len();
        // The four points, and for each insert at most its two ends and two points about each of
        // the few starts of clips (four of them) that the slopes of the first two seconds cross.
        assert!(
            count <= 4 + (n + 1) * (2 + 2 * 4),
            "{count} after {}",
            n + 1
        );
    }
}

#[test]
fn a_copy_of_a_clip_takes_a_copy_of_its_automation_and_clears_where_it_goes() {
    let mut history = History::new({
        let mut project = project();
        op(
            &mut project,
            json!({"op": "remove_clip", "timeline": "t1", "id": "d"}),
        );
        project
    });
    let before = history.project().clone();
    let copy: Edit = serde_json::from_value(json!({"edit": "place_copies", "timeline": "t1",
        "copies": [{"clip": "c2", "track": "A1", "start": 120}]}))
    .unwrap();
    history.edit(vec![copy], "desktop").unwrap();
    let after = history.project().clone();
    assert_eq!(spans(&after, "A1"), vec![(50, 100), (100, 120), (120, 170)]);
    let made = &track(&after, "A1").clips[2];
    assert!(made.id != "c2" && !made.id.is_empty());
    let frame = 1920;
    for k in [0, 1000, 25 * frame, 49 * frame] {
        let (was, copied, kept) = (
            value_at(&before, "A1", "fader_db", 50 * frame + k),
            value_at(&after, "A1", "fader_db", 120 * frame + k),
            value_at(&after, "A1", "fader_db", 50 * frame + k),
        );
        assert!((was - copied).abs() < 1e-9, "+{k}: {was} then {copied}");
        assert!((was - kept).abs() < 1e-9, "+{k}: {was} then {kept}");
    }
    history.undo("desktop").unwrap().unwrap();
    assert_eq!(history.project().timelines, before.timelines);
}

#[test]
fn copies_of_clips_linked_together_are_linked_together_anew_and_take_no_lane_while_it_is_off() {
    let mut project = project();
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
    follows(&mut project, false);
    let lanes = track(&project, "A1").audio.lanes.clone();
    edit(
        &mut project,
        json!({"edit": "place_copies", "timeline": "t1", "copies": [
            {"clip": "c1", "track": "V1", "start": 300}, {"clip": "c2", "track": "A1", "start": 300}]}),
    );
    let at = |id: &str| {
        track(&project, id)
            .clips
            .iter()
            .find(|c| c.start == 300)
            .unwrap()
    };
    let (v, a) = (at("V1"), at("A1"));
    assert!(v.link.is_some() && v.link == a.link && v.link.as_deref() != Some("g1"));
    assert_eq!(track(&project, "A1").audio.lanes, lanes);
}

#[test]
fn a_clip_moved_where_its_frame_is_a_tick_shorter_keeps_its_lane_to_its_last_tick() {
    // At 29.97 frames a second a frame lasts 1601.6 ticks of 48 kHz: frame 1 is [1601, 3203) and
    // frame 2 [3203, 4804). A clip of frame 1 moved to frame 2 is a tick shorter there.
    let mut project = project();
    let timeline = &mut project.timelines[0];
    timeline.rate = columbina_model::Rate {
        num: 30000,
        den: 1001,
    };
    timeline.tracks[1].clips = vec![Clip::of_media(
        "c2",
        "m1",
        Time {
            ticks: 0,
            scale: 15360,
        },
        1,
        1,
    )];
    timeline.tracks[2].clips.clear();
    project.validate().unwrap();
    op(
        &mut project,
        json!({"op": "set_lane", "timeline": "t1", "of": {"track": "A1"}, "path": "fader_db",
            "lane": {"points": [{"at": {"ticks": 0, "scale": 48000}, "value": 0},
                {"at": {"ticks": 10000, "scale": 48000}, "value": -10}]}}),
    );
    let before = project.clone();
    edit(
        &mut project,
        json!({"edit": "move", "timeline": "t1", "clips": ["c2"], "delta": 1}),
    );
    for k in [0, 800, 1599] {
        let (was, is) = (
            value_at(&before, "A1", "fader_db", 1601 + k),
            value_at(&project, "A1", "fader_db", 3203 + k),
        );
        assert!((was - is).abs() < 1e-9, "+{k}: {was} then {is}");
    }
}

/// V1 c1 [0, 25) linked to A1's e [100, 150), whose sound starts long after its picture ends,
/// with c2 [50, 100) between them on A1.
fn split_far_apart() -> Project {
    let mut project = project();
    op(
        &mut project,
        json!({"op": "remove_clip", "timeline": "t1", "id": "d"}),
    );
    let mut picture = Clip::of_media(
        "c1",
        "m1",
        Time {
            ticks: 0,
            scale: 15360,
        },
        0,
        25,
    );
    picture.link = Some("g1".into());
    op(
        &mut project,
        json!({"op": "insert_clip", "timeline": "t1", "track": "V1", "clip": picture}),
    );
    op(
        &mut project,
        json!({"op": "set_clip", "timeline": "t1", "id": "e", "link": "g1"}),
    );
    project
}

#[test]
fn a_ripple_trim_of_a_start_moves_the_lanes_of_what_it_moves_before_a_later_clip_of_its_group() {
    // A ripple trim of c1's start by 5 pulls what follows its end, 25, back 5 frames: c2 goes to
    // [45, 95) with its automation, while e stays where it starts and plays from 5 frames later.
    let mut project = split_far_apart();
    let before = project.clone();
    edit(
        &mut project,
        json!({"edit": "trim", "timeline": "t1", "clip": "c1", "edge": "in", "delta": 5,
            "ripple": true}),
    );
    assert_eq!(spans(&project, "A1"), vec![(45, 95), (100, 145)]);
    let frame = 1920;
    let same = |was: i64, is: i64| {
        let (a, b) = (
            value_at(&before, "A1", "fader_db", was),
            value_at(&project, "A1", "fader_db", is),
        );
        assert!((a - b).abs() < 1e-9, "{was} then {is}: {a} then {b}");
    };
    for k in [0, 1000, 25 * frame, 49 * frame, 50 * frame - 1] {
        same(50 * frame + k, 45 * frame + k);
    }
    for k in [0, 1000, 20 * frame, 44 * frame, 45 * frame - 1] {
        same(105 * frame + k, 100 * frame + k);
    }
    for t in [0, 10 * frame, 20 * frame - 1] {
        same(t, t);
    }
}

#[test]
fn a_ripple_trim_of_an_end_keeps_the_lanes_of_a_later_clip_of_its_group_where_it_starts() {
    // A ripple trim of c1's end by −5: c2 goes back 5 frames with its automation, and e, made 5
    // frames shorter where it starts, keeps its own over what it still plays.
    let mut project = split_far_apart();
    let before = project.clone();
    edit(
        &mut project,
        json!({"edit": "trim", "timeline": "t1", "clip": "c1", "edge": "out", "delta": -5,
            "ripple": true}),
    );
    assert_eq!(spans(&project, "A1"), vec![(45, 95), (100, 145)]);
    let frame = 1920;
    let same = |was: i64, is: i64| {
        let (a, b) = (
            value_at(&before, "A1", "fader_db", was),
            value_at(&project, "A1", "fader_db", is),
        );
        assert!((a - b).abs() < 1e-9, "{was} then {is}: {a} then {b}");
    };
    for k in [0, 1000, 25 * frame, 49 * frame, 50 * frame - 1] {
        same(50 * frame + k, 45 * frame + k);
    }
    for k in [0, 1000, 20 * frame, 44 * frame, 45 * frame - 1] {
        same(100 * frame + k, 100 * frame + k);
    }
}

#[test]
fn a_lift_changes_a_lane_only_where_it_has_a_point_over_the_clip() {
    // A slope from 1 s to 6 s with no point from 2 s to 4 s, where c2 is: nothing is put there,
    // though the values at its ends would not come to the same bits again.
    let mut project = project();
    lane(&mut project, "A1", "fader_db", &[(1, 0.1), (6, -7.3)]);
    edit(
        &mut project,
        json!({"edit": "lift", "timeline": "t1", "clips": ["c2"]}),
    );
    assert_eq!(
        points(&project, "A1", "fader_db").unwrap(),
        vec![(1.0, 0.1), (6.0, -7.3)]
    );
    // A lane whose first point is within the clip: it is held at the values of the clip's ends.
    lane(&mut project, "A1", "fader_db", &[(5, -6.0), (7, -8.0)]);
    edit(
        &mut project,
        json!({"edit": "lift", "timeline": "t1", "clips": ["e"]}),
    );
    assert_eq!(
        points(&project, "A1", "fader_db").unwrap(),
        vec![(4.0, -6.0), (6.0, -7.0), (7.0, -8.0)]
    );
}

#[test]
fn a_ripple_holds_no_slope_where_the_first_clip_that_moves_goes_as_far_as_the_place() {
    // With c2 gone, e, at 4 s, is the first clip to move when 25 frames are put in at frame 60.
    // It goes as far as the lane does at the place, so the slope across its start needs no
    // point there: the lane has its two ends, the place and where it goes on.
    let mut project = project();
    op(
        &mut project,
        json!({"op": "remove_clip", "timeline": "t1", "id": "c2"}),
    );
    lane(&mut project, "A1", "fader_db", &[(1, 0.1), (9, -7.3)]);
    edit(
        &mut project,
        json!({"edit": "insert", "timeline": "t1", "at": 60, "video_track": null,
            "source": {"kind": "media", "media": "m1", "length": 25}}),
    );
    assert_eq!(start_of(&project, "e"), Some(("A1".into(), 125)));
    let times: Vec<f64> = points(&project, "A1", "fader_db")
        .unwrap()
        .iter()
        .map(|&(at, _)| at)
        .collect();
    assert_eq!(times, vec![1.0, 2.4, 3.4, 10.0]);
}

#[test]
fn a_ripple_trim_leaves_the_lanes_before_a_later_clip_of_its_group_that_nothing_moves_up_to() {
    // A1 has a clip from frame 10 to e, at 100: it starts before the place of the ripple, 25,
    // and stays, so nothing between the place and e moves and the lane before e is left as it
    // is. Its value at 4 s, -2, comes to other bits when it is taken there from 1.3 again.
    let mut project = split_far_apart();
    op(
        &mut project,
        json!({"op": "remove_clip", "timeline": "t1", "id": "c2"}),
    );
    let long = Clip::of_media(
        "c2",
        "m1",
        Time {
            ticks: 0,
            scale: 15360,
        },
        10,
        90,
    );
    op(
        &mut project,
        json!({"op": "insert_clip", "timeline": "t1", "track": "A1", "clip": long}),
    );
    lane(&mut project, "A1", "fader_db", &[(1, 1.3), (5, -3.1)]);
    edit(
        &mut project,
        json!({"edit": "trim", "timeline": "t1", "clip": "c1", "edge": "out", "delta": 5,
            "ripple": true}),
    );
    assert_eq!(spans(&project, "A1"), vec![(10, 100), (100, 155)]);
    assert_eq!(
        points(&project, "A1", "fader_db").unwrap(),
        vec![(1.0, 1.3), (5.0, -3.1)]
    );
}
