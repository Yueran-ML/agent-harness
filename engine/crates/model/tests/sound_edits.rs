//! The edits of sound (T2): a setting of a track that the tracks of its groups follow, as one
//! step of undo; keyframes of the volume of a clip, its volume raised over a range or nudged;
//! attributes pasted and taken away; a fade to the playhead; effects copied; and a transition of
//! picture that brings a crossfade of its sound.

#![allow(
    clippy::needless_pass_by_value,
    reason = "settings are given as a client gives them"
)]

use columbina_model::audio::fx::GainFx;
use columbina_model::{
    apply, AudioKey, AudioParam, Change, Edit, EditError, Effect, History, Insert, InsertsOf,
    KeyInterp, Op, Project, Time, Track, TrackKind,
};
use serde_json::{json, Value};

/// The sample with V1's c1 and A1's c2 linked, cut in two at frame 50 into c3 and c5 (linked
/// too), and a second track of sound, A2, with a clip s5 of its own.
fn project() -> Project {
    let mut project = Project::sample();
    for track in &mut project.timelines[0].tracks {
        track.clips[0].link = Some("g1".into());
    }
    let mut a2 = Track::new("A2", TrackKind::Audio);
    let mut c5 = project.timelines[0].tracks[1].clips[0].clone();
    c5.id = "s5".into();
    c5.link = None;
    a2.clips = vec![c5];
    project.timelines[0].tracks.push(a2);
    project.next_id = 3;
    project.validate().unwrap();
    edit(
        &mut project,
        json!({"edit": "blade", "timeline": "t1", "at": 50, "tracks": ["V1", "A1"]}),
    );
    project
}

fn edit(project: &mut Project, edit: Value) -> Vec<Op> {
    let edit: Edit = serde_json::from_value(edit).unwrap();
    columbina_model::expand(project, edit.clone())
        .unwrap_or_else(|e| panic!("{edit:?}: {e}"))
        .done
}

fn refused(project: &mut Project, edit: Value) -> Value {
    let before = project.clone();
    let edit: Edit = serde_json::from_value(edit).unwrap();
    let error = columbina_model::expand(project, edit).unwrap_err();
    assert_eq!(*project, before);
    let mut value = serde_json::to_value(&error).unwrap();
    value.as_object_mut().unwrap().remove("message");
    value
}

fn op(project: &mut Project, op: Value) {
    apply(project, serde_json::from_value(op).unwrap()).unwrap();
}

fn track<'p>(project: &'p Project, id: &str) -> &'p Track {
    project.timelines[0]
        .tracks
        .iter()
        .find(|r| r.id == id)
        .unwrap()
}

fn clip<'p>(project: &'p Project, id: &str) -> &'p columbina_model::Clip {
    project.timelines[0]
        .tracks
        .iter()
        .flat_map(|r| &r.clips)
        .find(|c| c.id == id)
        .unwrap()
}

fn fader(project: &Project, id: &str) -> f64 {
    track(project, id).audio.strip.fader_db
}

fn seconds(at: f64) -> f64 {
    at
}

fn time(ticks: i64, scale: u32) -> Value {
    json!({"ticks": ticks, "scale": scale})
}

/// A1 and A2 in a group that links `linked`, active or not; A2's fader at -6 dB.
fn grouped(linked: Value, active: bool) -> Project {
    let mut project = project();
    op(
        &mut project,
        json!({"op": "set_track_audio", "timeline": "t1", "track": "A2", "patch": {"strip": {"fader_db": -6}}}),
    );
    op(
        &mut project,
        json!({"op": "add_track_group", "timeline": "t1", "group": {"id": "", "name": "Dialogue",
            "members": ["A1", "A2"], "linked": linked, "active": active}}),
    );
    project
}

#[test]
fn a_fader_of_a_group_moves_the_others_by_as_much_and_is_undone_in_one_step() {
    let project = grouped(json!(["fader"]), true);
    let mut history = History::new(project.clone());
    let move_a1 = serde_json::from_value::<Edit>(json!({"edit": "set_track_audio",
        "timeline": "t1", "track": "A1", "patch": {"strip": {"fader_db": -3}}}))
    .unwrap();
    history
        .change(vec![Change::Edit(move_a1)], "desktop", |change, _| change)
        .unwrap();
    assert_eq!(fader(history.project(), "A1"), -3.0);
    assert_eq!(fader(history.project(), "A2"), -9.0);
    history.undo("desktop").unwrap().unwrap();
    assert_eq!(fader(history.project(), "A1"), 0.0);
    assert_eq!(fader(history.project(), "A2"), -6.0);
    // Not active: A1 alone. Clutched (follow_groups false): A1 alone too.
    for (active, follow) in [(false, true), (true, false)] {
        let mut project = grouped(json!(["fader"]), active);
        edit(
            &mut project,
            json!({"edit": "set_track_audio", "timeline": "t1", "track": "A1",
                "patch": {"strip": {"fader_db": -3}}, "follow_groups": follow}),
        );
        assert_eq!((fader(&project, "A1"), fader(&project, "A2")), (-3.0, -6.0));
    }
    // A group that does not link the fader leaves A2's alone.
    let mut project = grouped(json!(["mute"]), true);
    edit(
        &mut project,
        json!({"edit": "set_track_audio", "timeline": "t1", "track": "A1",
            "patch": {"strip": {"fader_db": -3}}}),
    );
    assert_eq!(fader(&project, "A2"), -6.0);
}

#[test]
fn a_group_moves_pans_and_sends_by_as_much_and_sets_mutes_and_solos_alike_within_their_ranges() {
    let mut project = grouped(json!(["fader", "panning", "mute", "solo", "sends"]), true);
    op(
        &mut project,
        json!({"op": "add_bus", "timeline": "t1", "bus": {"id": "", "name": "Verb", "color": "#ffffffff", "format": "stereo"}}),
    );
    for (track, level, x) in [("A1", -10, 0.2), ("A2", -20, 0.8)] {
        op(
            &mut project,
            json!({"op": "set_track_audio", "timeline": "t1", "track": track, "patch": {"strip": {"sends": [{"bus": "B2", "level_db": level}], "pan": {"x": x}}}}),
        );
    }
    let done = edit(
        &mut project,
        json!({"edit": "set_track_audio", "timeline": "t1", "track": "A1",
            "patch": {"mute": true, "solo": true, "strip": {"fader_db": 8, "pan": {"x": 0.6},
                "sends": [{"bus": "B2", "level_db": -4}]}}}),
    );
    assert_eq!(done.len(), 2);
    let a2 = &track(&project, "A2").audio;
    assert!(a2.mute && a2.solo);
    // -6 + 8, at most +10 dB.
    assert_eq!(a2.strip.fader_db, 2.0);
    assert!((a2.strip.pan.x - 1.0).abs() < 1e-12, "0.8 + 0.4, at most 1");
    assert_eq!(a2.strip.sends[0].level_db, -14.0);
    // A2's own patch names only what changes of it.
    assert_eq!(
        serde_json::to_value(&done[1]).unwrap()["patch"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        ["mute", "solo", "strip"]
    );
    // A fader at -∞ stays there as the group goes down.
    let mut project = grouped(json!(["fader"]), true);
    op(
        &mut project,
        json!({"op": "set_track_audio", "timeline": "t1", "track": "A2", "patch": {"strip": {"fader_db": -120}}}),
    );
    edit(
        &mut project,
        json!({"edit": "set_track_audio", "timeline": "t1", "track": "A1", "patch": {"strip": {"fader_db": -5}}}),
    );
    assert_eq!(fader(&project, "A2"), -120.0);
}

#[test]
fn a_locked_track_is_not_set_and_one_in_its_group_is_left_alone() {
    let mut project = grouped(json!(["fader"]), true);
    op(
        &mut project,
        json!({"op": "set_track", "timeline": "t1", "id": "A2", "locked": true}),
    );
    edit(
        &mut project,
        json!({"edit": "set_track_audio", "timeline": "t1", "track": "A1", "patch": {"strip": {"fader_db": -3}}}),
    );
    assert_eq!(fader(&project, "A2"), -6.0);
    assert_eq!(
        refused(
            &mut project,
            json!({"edit": "set_track_audio", "timeline": "t1", "track": "A2", "patch": {"mute": true}})
        ),
        json!({"reason": "locked", "track": "A2"})
    );
}

#[test]
fn an_exclusive_solo_takes_the_solo_off_the_others() {
    let mut project = project();
    op(
        &mut project,
        json!({"op": "set_timeline_audio", "timeline": "t1", "patch": {"exclusive_solo": true}}),
    );
    op(
        &mut project,
        json!({"op": "set_track_audio", "timeline": "t1", "track": "A2", "patch": {"solo": true, "solo_safe": true}}),
    );
    op(
        &mut project,
        json!({"op": "set_bus", "timeline": "t1", "id": "B1", "patch": {"solo": true}}),
    );
    edit(
        &mut project,
        json!({"edit": "set_track_audio", "timeline": "t1", "track": "A1", "patch": {"solo": true}}),
    );
    assert!(track(&project, "A1").audio.solo);
    let a2 = &track(&project, "A2").audio;
    assert!(!a2.solo && a2.solo_safe, "its Solo Safe stays");
    assert!(!project.timelines[0].buses[0].solo);
}

#[test]
fn a_range_of_the_volume_is_raised_with_a_pair_of_keys_at_each_end() {
    let mut project = project();
    // s5 is fixed at 0 dB; [1 s, 2 s) of its media 6 dB down.
    edit(
        &mut project,
        json!({"edit": "clip_gain_range", "timeline": "t1", "clip": "s5",
            "from": time(15360, 15360), "to": time(2 * 15360, 15360), "delta_db": -6}),
    );
    let gain = &clip(&project, "s5").gain_db;
    let AudioParam::Keys(keys) = gain else {
        panic!("{gain:?}")
    };
    let at: Vec<(i64, u32, f64)> = keys
        .iter()
        .map(|k| (k.at.ticks, k.at.scale, k.value))
        .collect();
    assert_eq!(
        at,
        [
            (47999, 48000, 0.0),
            (48000, 48000, -6.0),
            (95999, 48000, -6.0),
            (96000, 48000, 0.0)
        ]
    );
    assert!((gain.value_at(seconds(1.5)) + 6.0).abs() < 1e-12);
    assert_eq!(gain.value_at(seconds(0.5)), 0.0);
    assert_eq!(gain.value_at(seconds(2.0)), 0.0);
    // Again over [1.5 s, 3 s): the keys inside move, those outside stay.
    edit(
        &mut project,
        json!({"edit": "clip_gain_range", "timeline": "t1", "clip": "s5",
            "from": time(72000, 48000), "to": time(144_000, 48000), "delta_db": 2}),
    );
    let gain = &clip(&project, "s5").gain_db;
    assert!((gain.value_at(1.25) + 6.0).abs() < 1e-12);
    assert!((gain.value_at(1.75) + 4.0).abs() < 1e-12);
    assert!((gain.value_at(2.5) - 2.0).abs() < 1e-12);
    assert_eq!(gain.value_at(3.5), 0.0);
    // Down past -96 dB, kept there and said to be clamped.
    let edit_of = |delta: f64| {
        serde_json::from_value::<Edit>(json!({"edit": "clip_gain_range", "timeline": "t1",
            "clip": "s5", "from": time(0, 1), "to": time(1, 1), "delta_db": delta}))
        .unwrap()
    };
    let expansion = columbina_model::expand(&mut project, edit_of(-200.0)).unwrap();
    assert!(expansion.clamped);
    assert_eq!(clip(&project, "s5").gain_db.value_at(0.5), -96.0);
    assert_eq!(
        columbina_model::expand(&mut project.clone(), edit_of(0.0)).unwrap_err(),
        EditError::EmptyRange
    );
}

#[test]
fn keys_of_a_clip_are_added_moved_eased_and_taken_away_by_their_time() {
    let mut project = project();
    let keys = |project: &Project| match &clip(project, "s5").gain_db {
        AudioParam::Keys(keys) => keys.clone(),
        AudioParam::Fixed(db) => panic!("fixed at {db}"),
    };
    let one = |action: &str, at: Value, more: Value| {
        let mut edit = json!({"edit": "clip_gain_keys", "timeline": "t1", "clip": "s5",
            "action": action, "at": at});
        edit.as_object_mut()
            .unwrap()
            .extend(more.as_object().unwrap().clone());
        edit
    };
    edit(&mut project, one("add", time(1, 1), json!({"value": -6})));
    // Added with no value: the value there.
    edit(&mut project, one("add", time(3, 1), json!({})));
    assert_eq!(
        keys(&project).iter().map(|k| k.value).collect::<Vec<_>>(),
        [-6.0, -6.0]
    );
    edit(
        &mut project,
        one(
            "move",
            time(3, 1),
            json!({"to": time(96000, 48000), "value": 0}),
        ),
    );
    edit(
        &mut project,
        one("set_interp", time(2, 1), json!({"interp": "ease_in"})),
    );
    assert_eq!(
        keys(&project),
        [
            AudioKey {
                at: Time { ticks: 1, scale: 1 },
                value: -6.0,
                interp: KeyInterp::Linear
            },
            AudioKey {
                at: Time {
                    ticks: 96000,
                    scale: 48000
                },
                value: 0.0,
                interp: KeyInterp::EaseIn
            }
        ]
    );
    // The pan has keys of its own.
    edit(
        &mut project,
        one("add", time(1, 2), json!({"param": "pan", "value": 0.5})),
    );
    assert!(matches!(clip(&project, "s5").pan, AudioParam::Keys(_)));
    assert_eq!(
        refused(&mut project, one("remove", time(5, 1), json!({}))),
        json!({"reason": "missing", "what": "key", "id": "5/1"})
    );
    assert_eq!(
        refused(
            &mut project,
            one("move", time(1, 1), json!({"to": time(2, 1)}))
        ),
        json!({"reason": "out_of_range", "what": "to"})
    );
    edit(&mut project, one("remove", time(1, 1), json!({})));
    // The last key taken away leaves its value, fixed.
    edit(&mut project, one("remove", time(2, 1), json!({})));
    assert_eq!(clip(&project, "s5").gain_db, AudioParam::Fixed(0.0));
}

#[test]
fn the_volume_with_three_keys_is_put_back_to_0_db_by_remove_attributes() {
    let mut project = project();
    for at in 1..=3 {
        edit(
            &mut project,
            json!({"edit": "clip_gain_keys", "timeline": "t1", "clip": "s5", "action": "add",
                "at": time(at, 1), "value": -3 * at}),
        );
    }
    op(
        &mut project,
        json!({"op": "add_insert", "timeline": "t1", "of": {"clip": "s5"},
            "insert": {"effect": {"kind": "gain", "gain_db": 3}}}),
    );
    let done = edit(
        &mut project,
        json!({"edit": "remove_attributes", "timeline": "t1", "clips": ["s5", "c2"],
            "items": ["volume"]}),
    );
    // c2 has its volume already: only s5 is set.
    assert_eq!(done.len(), 1);
    let c5 = clip(&project, "s5");
    assert_eq!(c5.gain_db, AudioParam::Fixed(0.0));
    assert_eq!(c5.effects.len(), 1, "only the volume is put back");
    edit(
        &mut project,
        json!({"edit": "remove_attributes", "timeline": "t1", "clips": ["s5"], "items": ["plugins", "equalizer"]}),
    );
    assert_eq!(clip(&project, "s5").effects.len(), 0);
}

#[test]
fn attributes_are_pasted_with_the_keys_where_they_were_in_each_clip() {
    let mut project = project();
    // A key of c2, which begins at 0 s of m1, at 1 s; c5 begins at 2 s of m1.
    edit(
        &mut project,
        json!({"edit": "clip_gain_keys", "timeline": "t1", "clip": "c2", "action": "add",
            "at": time(15360, 15360), "value": -9}),
    );
    op(
        &mut project,
        json!({"op": "add_insert", "timeline": "t1", "of": {"clip": "c2"},
            "insert": {"effect": {"kind": "gain", "gain_db": 3}}}),
    );
    edit(
        &mut project,
        json!({"edit": "paste_attributes", "timeline": "t1", "from": "c2", "to": ["c5", "s5"],
            "items": ["volume", "plugins"]}),
    );
    let AudioParam::Keys(keys) = &clip(&project, "c5").gain_db else {
        panic!()
    };
    assert_eq!(
        (keys[0].at, keys[0].value),
        (Time { ticks: 3, scale: 1 }, -9.0)
    );
    assert_eq!(clip(&project, "c5").effects, clip(&project, "c2").effects);
    // s5 begins where c2 does: its key is where c2's is.
    let AudioParam::Keys(keys) = &clip(&project, "s5").gain_db else {
        panic!()
    };
    assert_eq!(
        keys[0].at,
        Time {
            ticks: 15360,
            scale: 15360
        }
    );
    assert_eq!(
        refused(
            &mut project,
            json!({"edit": "paste_attributes", "timeline": "t1", "from": "c2", "to": ["s5"], "items": []})
        ),
        json!({"reason": "out_of_range", "what": "items"})
    );
}

#[test]
fn clips_of_sound_are_nudged_keys_and_all_and_their_pictures_are_passed_over() {
    let mut project = project();
    edit(
        &mut project,
        json!({"edit": "clip_gain_keys", "timeline": "t1", "clip": "c2", "action": "add",
            "at": time(1, 1), "value": -6}),
    );
    let done = edit(
        &mut project,
        json!({"edit": "nudge_clip_gain", "timeline": "t1", "clips": ["c1"], "delta_db": 3}),
    );
    // c1 is picture; c2, linked to it, is sound.
    assert_eq!(done.len(), 1);
    assert_eq!(clip(&project, "c2").gain_db.value_at(0.0), -3.0);
    assert_eq!(clip(&project, "c1").gain_db, AudioParam::Fixed(0.0));
    let expansion = columbina_model::expand(
        &mut project,
        serde_json::from_value(json!({"edit": "nudge_clip_gain", "timeline": "t1",
            "clips": ["s5"], "delta_db": 30}))
        .unwrap(),
    )
    .unwrap();
    assert!(expansion.clamped);
    assert_eq!(clip(&project, "s5").gain_db, AudioParam::Fixed(24.0));
    assert_eq!(
        refused(
            &mut project,
            json!({"edit": "nudge_clip_gain", "timeline": "t1", "clips": ["c1"], "delta_db": 1, "linked": false})
        ),
        json!({"reason": "out_of_range", "what": "clips"})
    );
    assert_eq!(
        refused(
            &mut project,
            json!({"edit": "nudge_clip_gain", "timeline": "t1", "clips": ["s5"], "delta_db": 0})
        ),
        json!({"reason": "empty_range"})
    );
}

#[test]
fn a_clip_fades_to_the_playhead_over_time_for_sound_and_frames_for_picture() {
    let mut project = project();
    edit(
        &mut project,
        json!({"edit": "fade_to_playhead", "timeline": "t1", "clips": ["s5"], "side": "in", "at": 25}),
    );
    let fade = clip(&project, "s5").audio_fade_in.unwrap();
    assert_eq!(
        fade.length,
        Time {
            ticks: 25,
            scale: 25
        }
    );
    edit(
        &mut project,
        json!({"edit": "fade_to_playhead", "timeline": "t1", "clips": ["c1"], "side": "out", "at": 40}),
    );
    assert_eq!(clip(&project, "c1").fade_out, 10);
    assert_eq!(
        refused(
            &mut project,
            json!({"edit": "fade_to_playhead", "timeline": "t1", "clips": ["s5"], "side": "out", "at": 100})
        ),
        json!({"reason": "nothing_at", "track": "A2", "frame": 100})
    );
    // A fade out of 70 frames, and then one in of 99 frames is longer than the clip with it.
    edit(
        &mut project,
        json!({"edit": "fade_to_playhead", "timeline": "t1", "clips": ["s5"], "side": "out", "at": 30}),
    );
    assert_eq!(
        refused(
            &mut project,
            json!({"edit": "fade_to_playhead", "timeline": "t1", "clips": ["s5"], "side": "in", "at": 99})
        ),
        json!({"reason": "out_of_range", "what": "fade"})
    );
}

#[test]
fn effects_are_copied_from_one_chain_onto_others_in_place_of_theirs() {
    let mut project = project();
    for db in [1.0, 2.0] {
        apply(
            &mut project,
            Op::AddInsert {
                timeline: "t1".into(),
                of: InsertsOf::Track("A1".into()),
                insert: Insert {
                    id: String::new(),
                    on: true,
                    effect: Effect::Gain(GainFx { gain_db: db }),
                },
                index: None,
            },
        )
        .unwrap();
    }
    op(
        &mut project,
        json!({"op": "add_insert", "timeline": "t1", "of": {"track": "A2"},
            "insert": {"effect": {"kind": "gain", "gain_db": 9}}}),
    );
    edit(
        &mut project,
        json!({"edit": "copy_effects", "timeline": "t1", "from": {"track": "A1"},
            "to": [{"track": "A2"}, {"bus": "B1"}, {"clip": "s5"}, {"track": "A1"}]}),
    );
    let a1 = &track(&project, "A1").audio.strip.inserts;
    assert_eq!(&track(&project, "A2").audio.strip.inserts, a1);
    assert_eq!(&project.timelines[0].buses[0].strip.inserts, a1);
    let effects = |list: &[Insert]| list.iter().map(|i| i.effect.clone()).collect::<Vec<_>>();
    assert_eq!(effects(&clip(&project, "s5").effects), effects(a1));
}

#[test]
fn a_transition_of_picture_brings_a_crossfade_at_the_cut_of_its_linked_sound() {
    let mut project = project();
    let done = edit(
        &mut project,
        json!({"edit": "add_transition", "timeline": "t1", "left": "c1", "right": "c3",
            "length": 10, "linked": true}),
    );
    assert_eq!(done.len(), 2);
    let a1 = &track(&project, "A1").transitions;
    assert_eq!(a1.len(), 1);
    assert_eq!(
        (a1[0].left.as_str(), a1[0].right.as_str(), a1[0].length),
        ("c2", "c5", 10)
    );
    assert_eq!(a1[0].kind, columbina_model::TransitionKind::Crossfade);
    // Without `linked`, as before: the picture alone.
    let mut project = self::project();
    let done = edit(
        &mut project,
        json!({"edit": "add_transition", "timeline": "t1", "left": "c1", "right": "c3", "length": 10}),
    );
    assert_eq!(done.len(), 1);
    assert_eq!(track(&project, "A1").transitions.len(), 0);
    // A1 locked: the picture alone.
    let mut project = self::project();
    op(
        &mut project,
        json!({"op": "set_track", "timeline": "t1", "id": "A1", "locked": true}),
    );
    let done = edit(
        &mut project,
        json!({"edit": "add_transition", "timeline": "t1", "left": "c1", "right": "c3",
            "length": 10, "linked": true}),
    );
    assert_eq!(done.len(), 1);
}

// The review of T2.

#[test]
fn a_linked_crossfade_is_left_out_where_a_clip_of_sound_fades_at_the_cut() {
    let mut project = project();
    // c2 fades out at its cut with c5.
    op(
        &mut project,
        json!({"op": "set_clip", "timeline": "t1", "id": "c2",
            "audio_fade_out": {"length": {"ticks": 1, "scale": 5}}}),
    );
    let expansion = columbina_model::expand(
        &mut project,
        serde_json::from_value(json!({"edit": "add_transition", "timeline": "t1",
            "left": "c1", "right": "c3", "length": 10, "linked": true}))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(expansion.done.len(), 1, "the picture alone");
    assert!(expansion.clamped);
    assert_eq!(track(&project, "V1").transitions.len(), 1);
    assert_eq!(track(&project, "A1").transitions.len(), 0);
}

#[test]
fn a_fader_brought_up_from_minus_infinity_moves_the_group_from_minus_100() {
    let mut project = grouped(json!(["fader"]), true);
    op(
        &mut project,
        json!({"op": "set_track_audio", "timeline": "t1", "track": "A1", "patch": {"strip": {"fader_db": -120}}}),
    );
    op(
        &mut project,
        json!({"op": "set_track_audio", "timeline": "t1", "track": "A2", "patch": {"strip": {"fader_db": -20}}}),
    );
    edit(
        &mut project,
        json!({"edit": "set_track_audio", "timeline": "t1", "track": "A1", "patch": {"strip": {"fader_db": -90}}}),
    );
    // -100 to -90 is 10 dB: A2 goes from -20 to -10, not to +10.
    assert_eq!(fader(&project, "A2"), -10.0);
}

#[test]
fn an_exclusive_solo_of_a_track_or_a_bus_takes_the_solo_off_vcas_too() {
    let mut project = project();
    for one in [
        json!({"op": "set_timeline_audio", "timeline": "t1", "patch": {"exclusive_solo": true}}),
        json!({"op": "add_vca", "timeline": "t1", "vca": {"id": "", "name": "All", "solo": true}}),
        json!({"op": "add_bus", "timeline": "t1", "bus": {"id": "", "name": "Verb", "color": "#ffffffff", "format": "stereo"}}),
    ] {
        op(&mut project, one);
    }
    edit(
        &mut project,
        json!({"edit": "set_track_audio", "timeline": "t1", "track": "A1", "patch": {"solo": true}}),
    );
    assert!(!project.timelines[0].vcas[0].solo);
    // A bus soloed by the edit of a bus takes the solo off A1 and the others.
    op(
        &mut project,
        json!({"op": "set_vca", "timeline": "t1", "id": "VCA1", "patch": {"solo": true}}),
    );
    edit(
        &mut project,
        json!({"edit": "set_bus", "timeline": "t1", "id": "B2", "patch": {"solo": true}}),
    );
    assert!(project.timelines[0].buses[1].solo);
    assert!(!track(&project, "A1").audio.solo);
    assert!(!project.timelines[0].vcas[0].solo);
}

#[test]
fn copied_effects_are_new_inserts_with_the_next_ids_of_their_strip() {
    let mut project = project();
    for id in ["fx3", "fx5"] {
        op(
            &mut project,
            json!({"op": "add_insert", "timeline": "t1", "of": {"track": "A1"},
                "insert": {"id": id, "effect": {"kind": "gain"}}}),
        );
    }
    edit(
        &mut project,
        json!({"edit": "copy_effects", "timeline": "t1", "from": {"track": "A1"},
            "to": [{"track": "A2"}, {"clip": "s5"}]}),
    );
    let ids = |list: &[Insert]| list.iter().map(|i| i.id.clone()).collect::<Vec<_>>();
    assert_eq!(
        ids(&track(&project, "A2").audio.strip.inserts),
        ["fx1", "fx2"]
    );
    assert_eq!(ids(&clip(&project, "s5").effects), ["", ""]);
}

#[test]
fn a_range_keeps_the_ease_of_the_segment_that_it_splits_and_of_a_key_that_it_replaces() {
    let mut project = project();
    for (at, interp) in [(0, "ease_out"), (4, "ease_in")] {
        edit(
            &mut project,
            json!({"edit": "clip_gain_keys", "timeline": "t1", "clip": "s5", "action": "add",
                "at": time(at, 1), "value": -10, "interp": interp}),
        );
    }
    // A key at 1 s, which the inner key of the range takes the place of.
    edit(
        &mut project,
        json!({"edit": "clip_gain_keys", "timeline": "t1", "clip": "s5", "action": "add",
            "at": time(48000, 48000), "value": -10, "interp": "ease_out"}),
    );
    edit(
        &mut project,
        json!({"edit": "clip_gain_range", "timeline": "t1", "clip": "s5",
            "from": time(1, 1), "to": time(2, 1), "delta_db": -6}),
    );
    let AudioParam::Keys(keys) = &clip(&project, "s5").gain_db else {
        panic!()
    };
    let interp_at = |ticks: i64| {
        keys.iter()
            .find(|k| {
                k.at == Time {
                    ticks,
                    scale: 48000,
                }
            })
            .unwrap()
            .interp
    };
    // The segment from 1 s to 4 s was linear out of 1 s: the outer key before 1 s is linear.
    assert_eq!(interp_at(47999), KeyInterp::Linear);
    assert_eq!(interp_at(48000), KeyInterp::EaseOut, "kept");
    assert_eq!(interp_at(95999), KeyInterp::Linear);
    // The segment from 1 s to 4 s eased out of 1 s: the outer key at 2 s carries that ease.
    assert_eq!(interp_at(96000), KeyInterp::EaseOut);
}

#[test]
fn the_volume_pasted_onto_a_clip_that_plays_backward_is_heard_where_it_was() {
    let mut project = project();
    edit(
        &mut project,
        json!({"edit": "clip_gain_keys", "timeline": "t1", "clip": "c2", "action": "add",
            "at": time(1, 1), "value": -9, "interp": "ease_in"}),
    );
    op(
        &mut project,
        json!({"op": "set_clip", "timeline": "t1", "id": "s5", "reverse": true}),
    );
    edit(
        &mut project,
        json!({"edit": "paste_attributes", "timeline": "t1", "from": "c2", "to": ["s5"], "items": ["volume"]}),
    );
    // c2 is 1 s into its media from its start at 0 s; s5 plays 4 s of m1 backward from 4 s.
    let AudioParam::Keys(keys) = &clip(&project, "s5").gain_db else {
        panic!()
    };
    assert_eq!(
        (keys[0].at, keys[0].interp),
        (Time { ticks: 3, scale: 1 }, KeyInterp::EaseOut)
    );
}

#[test]
fn edits_of_the_sound_of_a_clip_refuse_a_clip_of_picture_and_keys_outside_its_media() {
    let mut project = project();
    let video = json!({"reason": "wrong_track", "what": "sound", "kind": "video"});
    for (one, error) in [
        (
            json!({"edit": "clip_gain_keys", "timeline": "t1", "clip": "c1", "action": "add", "at": time(1, 1)}),
            video.clone(),
        ),
        (
            json!({"edit": "clip_gain_range", "timeline": "t1", "clip": "c1", "from": time(1, 1), "to": time(2, 1), "delta_db": 1}),
            video.clone(),
        ),
        (
            json!({"edit": "remove_attributes", "timeline": "t1", "clips": ["c1"], "items": ["volume"]}),
            video.clone(),
        ),
        (
            json!({"edit": "paste_attributes", "timeline": "t1", "from": "c2", "to": ["c1"], "items": ["volume"]}),
            video,
        ),
        // s5 plays 0 s to 4 s of m1.
        (
            json!({"edit": "clip_gain_keys", "timeline": "t1", "clip": "s5", "action": "add", "at": time(9, 1)}),
            json!({"reason": "out_of_range", "what": "at"}),
        ),
        (
            json!({"edit": "clip_gain_range", "timeline": "t1", "clip": "s5", "from": time(3, 1), "to": time(5, 1), "delta_db": 1}),
            json!({"reason": "out_of_range", "what": "to"}),
        ),
    ] {
        assert_eq!(refused(&mut project, one), error);
    }
}

#[test]
fn several_clips_fade_to_the_playhead_in_one_edit() {
    let mut project = project();
    let done = edit(
        &mut project,
        json!({"edit": "fade_to_playhead", "timeline": "t1", "clips": ["s5", "c1", "s5"], "side": "in", "at": 20}),
    );
    assert_eq!(done.len(), 2);
    assert_eq!(clip(&project, "c1").fade_in, 20);
    assert!(clip(&project, "s5").audio_fade_in.is_some());
}

#[test]
fn a_track_that_is_taken_away_leaves_its_groups_in_the_same_step() {
    let mut project = grouped(json!(["fader"]), true);
    let mut history = History::new(project.clone());
    history
        .change(
            vec![Change::Edit(
                serde_json::from_value(
                    json!({"edit": "remove_track", "timeline": "t1", "track": "A2"}),
                )
                .unwrap(),
            )],
            "desktop",
            |change, _| change,
        )
        .unwrap();
    assert_eq!(
        history.project().timelines[0].track_groups[0].members,
        ["A1"]
    );
    history.undo("desktop").unwrap().unwrap();
    assert_eq!(history.project().timelines[0], project.timelines[0]);
    // A Ducker that A2 drives still stands in the way.
    op(
        &mut project,
        json!({"op": "set_track_audio", "timeline": "t1", "track": "A1",
            "patch": {"strip": {"track_fx": {"sources": [{"track": "A2"}]}}}}),
    );
    assert_eq!(
        refused(
            &mut project,
            json!({"edit": "remove_track", "timeline": "t1", "track": "A2"})
        ),
        json!({"reason": "in_use", "what": "track", "id": "A2", "by": "A1"})
    );
}

#[test]
fn the_edit_of_a_transition_sets_the_levels_of_a_crossfade() {
    let mut project = project();
    edit(
        &mut project,
        json!({"edit": "add_transition", "timeline": "t1", "left": "c2", "right": "c5", "length": 10}),
    );
    let id = track(&project, "A1").transitions[0].id.clone();
    edit(
        &mut project,
        json!({"edit": "set_transition", "timeline": "t1", "id": id, "fade_in_level_db": 3, "fade_out_level_db": -3}),
    );
    let x = &track(&project, "A1").transitions[0];
    assert_eq!(
        (x.fade_in_level_db, x.fade_out_level_db),
        (
            columbina_model::CrossfadeLevel::Plus3,
            columbina_model::CrossfadeLevel::Minus3
        )
    );
}
