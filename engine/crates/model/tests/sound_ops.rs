//! The operations on sound (T2): a patch of a track, a bus, a VCA, a group of tracks and the
//! sound of a timeline; buses, VCAs and groups added and taken away; inserts added, put in place
//! of others, taken away and moved; and the settings of sound of a clip and a crossfade. Each is
//! taken back by its inverse, done again by the inverse of that, and refused as PROTOCOL says.

#![allow(
    clippy::needless_pass_by_value,
    clippy::too_many_lines,
    reason = "an operation is given as a client gives it; a test of a chain of steps"
)]

use columbina_model::audio::fx::{GainFx, LimiterFx};
use columbina_model::{
    apply, apply_done, Bus, ChannelMap, ClipPitch, CrossfadeLevel, Effect, Equalizer, Insert,
    InsertsOf, Op, OpError, Project, Track, TrackGroup, TrackKind, Transition, TransitionKind, Vca,
};
use serde_json::{json, Value};

fn reason(error: &OpError) -> Value {
    let mut value = serde_json::to_value(error).unwrap();
    value.as_object_mut().unwrap().remove("message");
    value
}

/// Apply `op` to `project`, check that its inverse takes it back and the inverse of that is the
/// operation as it was done, and give back the operation as it was done and its inverse.
fn applied(project: &mut Project, op: Op) -> (Op, Op) {
    let before = project.clone();
    let (done, inverse) = apply_done(project, op.clone()).unwrap_or_else(|e| panic!("{op:?}: {e}"));
    let after = project.clone();
    let mut undone = after.clone();
    let again = apply(&mut undone, inverse.clone()).unwrap();
    assert_eq!(
        undone,
        Project {
            next_id: after.next_id,
            ..before.clone()
        },
        "{op:?}"
    );
    assert_eq!(again, done, "{op:?}");
    apply(&mut undone, done.clone()).unwrap();
    assert_eq!(undone, after, "{op:?} redone");
    // The operation reads back as it is written, as a client sends it.
    let written = serde_json::to_value(&done).unwrap();
    assert_eq!(serde_json::from_value::<Op>(written).unwrap(), done);
    (done, inverse)
}

fn refused(project: &mut Project, op: Op) -> Value {
    let before = project.clone();
    let error = apply(project, op).unwrap_err();
    assert_eq!(*project, before);
    reason(&error)
}

/// The sample, with a second track of sound, A2.
fn project() -> Project {
    let mut project = Project::sample();
    project.timelines[0]
        .tracks
        .push(Track::new("A2", TrackKind::Audio));
    project.validate().unwrap();
    project
}

fn op(value: Value) -> Op {
    serde_json::from_value(value).unwrap()
}

fn track<'p>(project: &'p Project, id: &str) -> &'p Track {
    project.timelines[0]
        .tracks
        .iter()
        .find(|r| r.id == id)
        .unwrap()
}

#[test]
fn a_patch_of_a_track_sets_what_it_names_and_its_inverse_and_done_are_the_values_at_its_places() {
    let mut project = project();
    let (done, inverse) = applied(
        &mut project,
        op(
            json!({"op": "set_track_audio", "timeline": "t1", "track": "A1",
            "patch": {"strip": {"fader_db": -6, "pan": {"x": 0.5}}, "mute": true}}),
        ),
    );
    let a1 = &track(&project, "A1").audio;
    assert_eq!(a1.strip.fader_db, -6.0);
    assert_eq!(a1.strip.pan.x, 0.5);
    assert!(a1.mute);
    assert_eq!(
        serde_json::to_value(&done).unwrap()["patch"],
        json!({"strip": {"fader_db": -6.0, "pan": {"x": 0.5}}, "mute": true})
    );
    assert_eq!(
        serde_json::to_value(&inverse).unwrap()["patch"],
        json!({"strip": {"fader_db": 0.0, "pan": {"x": 0.0}}, "mute": false})
    );
    // A Ducker set where there was none is put back to none, and done whole.
    let (done, inverse) = applied(
        &mut project,
        op(
            json!({"op": "set_track_audio", "timeline": "t1", "track": "A1",
            "patch": {"strip": {"track_fx": {"sources": [{"track": "A2"}], "amount_db": 12}}}}),
        ),
    );
    assert_eq!(
        serde_json::to_value(&inverse).unwrap()["patch"],
        json!({"strip": {"track_fx": null}})
    );
    let ducker = &serde_json::to_value(&done).unwrap()["patch"]["strip"]["track_fx"];
    assert_eq!(ducker["amount_db"], json!(12.0));
    assert_eq!(ducker["recovery_ms"], json!(750.0));
}

#[test]
fn a_format_of_a_number_of_channels_sets_them_and_an_adaptive_one_is_given_its_own() {
    let mut project = project();
    let (done, _) = applied(
        &mut project,
        op(
            json!({"op": "set_track_audio", "timeline": "t1", "track": "A2",
            "patch": {"format": "mono"}}),
        ),
    );
    assert_eq!(track(&project, "A2").audio.channels, 1);
    assert_eq!(
        serde_json::to_value(&done).unwrap()["patch"],
        json!({"format": "mono", "channels": 1})
    );
    applied(
        &mut project,
        op(
            json!({"op": "set_track_audio", "timeline": "t1", "track": "A2",
            "patch": {"format": "adaptive", "channels": 8}}),
        ),
    );
    assert_eq!(track(&project, "A2").audio.channels, 8);
    assert_eq!(
        refused(
            &mut project,
            op(
                json!({"op": "set_track_audio", "timeline": "t1", "track": "A2",
                "patch": {"channels": 25}})
            )
        ),
        json!({"reason": "channel_count", "track": "A2"})
    );
}

#[test]
fn a_patch_of_sound_is_refused_with_the_names_of_protocol() {
    let mut project = project();
    let set = |track: &str, patch: Value| {
        op(json!({"op": "set_track_audio", "timeline": "t1", "track": track, "patch": patch}))
    };
    for (patch, what) in [
        (json!(3), "the settings are not an object"),
        (json!({"lanes": {}}), "unknown field"),
        (json!({"strip": {"loud": true}}), "unknown field"),
        (json!({"mute": "yes"}), "invalid type"),
    ] {
        let error = refused(&mut project, set("A1", patch));
        assert_eq!(error["reason"], "audio_settings");
        assert!(error["why"].as_str().unwrap().contains(what), "{error}");
    }
    assert_eq!(
        refused(&mut project, set("V1", json!({"mute": true}))),
        json!({"reason": "wrong_track", "what": "strip", "kind": "video"})
    );
    assert_eq!(
        refused(&mut project, set("A1", json!({"strip": {"fader_db": 11}}))),
        json!({"reason": "out_of_range", "what": "fader_db"})
    );
    assert_eq!(
        refused(
            &mut project,
            set("A1", json!({"strip": {"outputs": [{"bus": "B7"}]}}))
        ),
        json!({"reason": "unknown_bus", "id": "B7"})
    );
    assert_eq!(
        refused(
            &mut project,
            set("A1", json!({"strip": {"order": ["fx", "fx", "eq"]}}))
        ),
        json!({"reason": "order"})
    );
    // A Ducker of A1 that A1 drives would go round in a loop.
    assert_eq!(
        refused(
            &mut project,
            set(
                "A1",
                json!({"strip": {"track_fx": {"sources": [{"track": "A1"}]}}})
            )
        ),
        json!({"reason": "bus_cycle", "id": "A1"})
    );
    assert_eq!(
        refused(&mut project, set("A9", json!({}))),
        json!({"reason": "missing", "what": "track", "id": "A9"})
    );
}

#[test]
fn buses_are_added_with_the_next_number_and_one_that_is_named_is_not_taken_away() {
    let mut project = project();
    let (done, inverse) = applied(
        &mut project,
        op(json!({"op": "add_bus", "timeline": "t1",
            "bus": {"id": "", "name": "Music", "color": "#40c080ff", "format": "5.1",
                "strip": {"inserts": [{"effect": {"kind": "gain", "gain_db": -3}}]}}})),
    );
    let Op::AddBus { bus, index, .. } = &done else {
        panic!("{done:?}")
    };
    assert_eq!((bus.id.as_str(), *index), ("B2", Some(1)));
    assert_eq!(bus.strip.inserts[0].id, "fx1");
    assert_eq!(
        inverse,
        op(json!({"op": "remove_bus", "timeline": "t1", "id": "B2"}))
    );
    // A1 sends to it, and then B2 is in use.
    applied(
        &mut project,
        op(
            json!({"op": "set_track_audio", "timeline": "t1", "track": "A1",
            "patch": {"strip": {"sends": [{"bus": "B2", "level_db": -10}]}}}),
        ),
    );
    assert_eq!(
        refused(
            &mut project,
            op(json!({"op": "remove_bus", "timeline": "t1", "id": "B2"}))
        ),
        json!({"reason": "in_use", "what": "bus", "id": "B2", "by": "A1"})
    );
    // B1 is monitored: in use by the timeline.
    applied(
        &mut project,
        op(
            json!({"op": "set_track_audio", "timeline": "t1", "track": "A1",
            "patch": {"strip": {"outputs": [], "sends": []}}}),
        ),
    );
    applied(
        &mut project,
        op(
            json!({"op": "set_track_audio", "timeline": "t1", "track": "A2",
            "patch": {"strip": {"outputs": []}}}),
        ),
    );
    assert_eq!(
        refused(
            &mut project,
            op(json!({"op": "remove_bus", "timeline": "t1", "id": "B1"}))
        ),
        json!({"reason": "in_use", "what": "bus", "id": "B1", "by": "t1"})
    );
    let (_, inverse) = applied(
        &mut project,
        op(json!({"op": "remove_bus", "timeline": "t1", "id": "B2"})),
    );
    let Op::AddBus { bus, index, .. } = inverse else {
        panic!()
    };
    assert_eq!((bus.name.as_str(), index), ("Music", Some(1)));
    assert_eq!(
        refused(
            &mut project,
            op(json!({"op": "remove_bus", "timeline": "t1", "id": "B2"}))
        ),
        json!({"reason": "unknown_bus", "id": "B2"})
    );
}

#[test]
fn a_bus_is_refused_where_its_routing_would_be() {
    let mut project = project();
    let add = |id: &str, to: &str| {
        op(
            json!({"op": "add_bus", "timeline": "t1", "bus": {"id": id, "name": id,
            "color": "#ffffffff", "format": "stereo", "strip": {"outputs": [{"bus": to}]}}}),
        )
    };
    assert_eq!(
        refused(&mut project, add("B1", "B1")),
        json!({"reason": "taken", "what": "bus", "id": "B1"})
    );
    assert_eq!(
        refused(&mut project, add("A1", "B1")),
        json!({"reason": "taken", "what": "track", "id": "A1"})
    );
    assert_eq!(
        refused(&mut project, add("$b", "B1")),
        json!({"reason": "dollar", "id": "$b"})
    );
    // B2 to B7, each into the one before, the last at the end of a path 7 buses long.
    for n in 2..=6 {
        applied(&mut project, add(&format!("B{n}"), &format!("B{}", n - 1)));
    }
    assert_eq!(
        refused(&mut project, add("B7", "B6")),
        json!({"reason": "bus_depth", "id": "B1"})
    );
    // B1 into B6 would go round in a loop.
    assert_eq!(
        refused(
            &mut project,
            op(json!({"op": "set_bus", "timeline": "t1", "id": "B1",
                "patch": {"strip": {"outputs": [{"bus": "B6"}]}}}))
        )["reason"],
        "bus_cycle"
    );
    for (patch, what) in [
        (json!({"id": "B9"}), "`id`"),
        (json!({"lanes": {}}), "`lanes`"),
    ] {
        let error = refused(
            &mut project,
            op(json!({"op": "set_bus", "timeline": "t1", "id": "B2", "patch": patch})),
        );
        assert_eq!(error["reason"], "audio_settings");
        assert!(error["why"].as_str().unwrap().contains(what));
    }
    let (done, _) = applied(
        &mut project,
        op(json!({"op": "set_bus", "timeline": "t1", "id": "B2",
            "patch": {"name": "Dialogue", "format": "mono", "mute": true}})),
    );
    assert_eq!(
        serde_json::to_value(&done).unwrap()["patch"],
        json!({"name": "Dialogue", "format": "mono", "mute": true})
    );
}

#[test]
fn vcas_are_added_numbered_and_one_that_has_members_is_not_taken_away() {
    let mut project = project();
    let (done, _) = applied(
        &mut project,
        op(json!({"op": "add_vca", "timeline": "t1", "vca": {"id": "", "name": "Dialogue"}})),
    );
    let Op::AddVca { vca, .. } = done else {
        panic!()
    };
    assert_eq!(vca.id, "VCA1");
    applied(
        &mut project,
        op(
            json!({"op": "add_vca", "timeline": "t1", "vca": {"id": "", "name": "All", "fader_db": -3}}),
        ),
    );
    applied(
        &mut project,
        op(
            json!({"op": "set_vca", "timeline": "t1", "id": "VCA1", "patch": {"vcas": ["VCA2"], "fader_db": -6}}),
        ),
    );
    assert_eq!(
        refused(
            &mut project,
            op(
                json!({"op": "set_vca", "timeline": "t1", "id": "VCA2", "patch": {"vcas": ["VCA1"]}})
            )
        ),
        json!({"reason": "vca_cycle", "id": "VCA1"})
    );
    assert_eq!(
        refused(
            &mut project,
            op(json!({"op": "remove_vca", "timeline": "t1", "id": "VCA2"}))
        ),
        json!({"reason": "in_use", "what": "vca", "id": "VCA2", "by": "VCA1"})
    );
    applied(
        &mut project,
        op(
            json!({"op": "set_track_audio", "timeline": "t1", "track": "A1", "patch": {"strip": {"vcas": ["VCA1"]}}}),
        ),
    );
    assert_eq!(
        refused(
            &mut project,
            op(json!({"op": "remove_vca", "timeline": "t1", "id": "VCA1"}))
        ),
        json!({"reason": "in_use", "what": "vca", "id": "VCA1", "by": "A1"})
    );
    assert_eq!(
        refused(
            &mut project,
            op(json!({"op": "set_vca", "timeline": "t1", "id": "VCA9", "patch": {}}))
        ),
        json!({"reason": "unknown_vca", "id": "VCA9"})
    );
    assert_eq!(
        refused(
            &mut project,
            op(
                json!({"op": "add_vca", "timeline": "t1", "vca": {"id": "", "name": "x".repeat(300)}})
            )
        ),
        json!({"reason": "out_of_range", "what": "label"})
    );
}

#[test]
fn groups_of_tracks_are_added_numbered_set_and_taken_away() {
    let mut project = project();
    let group = |members: Value| {
        op(json!({"op": "add_track_group", "timeline": "t1",
            "group": {"id": "", "name": "Dialogue", "members": members}}))
    };
    let (done, inverse) = applied(&mut project, group(json!(["A1", "A2"])));
    let Op::AddTrackGroup { group: added, .. } = done else {
        panic!()
    };
    assert_eq!(added.id, "TG1");
    assert!(added.active);
    assert_eq!(added.linked.len(), 7);
    assert_eq!(
        inverse,
        op(json!({"op": "remove_track_group", "timeline": "t1", "id": "TG1"}))
    );
    assert_eq!(
        refused(&mut project, group(json!(["A1", "A9"]))),
        json!({"reason": "missing", "what": "track", "id": "A9"})
    );
    assert_eq!(
        refused(&mut project, group(json!(["A1", "A1"]))),
        json!({"reason": "out_of_range", "what": "members"})
    );
    applied(
        &mut project,
        op(
            json!({"op": "set_track_group", "timeline": "t1", "id": "TG1",
            "patch": {"active": false, "linked": ["fader", "mute"]}}),
        ),
    );
    assert_eq!(
        refused(
            &mut project,
            op(json!({"op": "remove_track_group", "timeline": "t1", "id": "TG9"}))
        ),
        json!({"reason": "unknown_group", "id": "TG9"})
    );
    // A member is in use; once the group is taken away it is not.
    assert_eq!(
        refused(
            &mut project,
            op(json!({"op": "remove_track", "timeline": "t1", "id": "A2"}))
        )["reason"],
        "in_use"
    );
    applied(
        &mut project,
        op(json!({"op": "remove_track_group", "timeline": "t1", "id": "TG1"})),
    );
    applied(
        &mut project,
        op(json!({"op": "remove_track", "timeline": "t1", "id": "A2"})),
    );
}

fn gain(db: f64) -> Insert {
    Insert {
        id: String::new(),
        on: true,
        effect: Effect::Gain(GainFx { gain_db: db }),
    }
}

fn inserts(project: &Project, of: &str) -> Vec<(String, Effect)> {
    let list = match of {
        "c2" => &project.timelines[0].tracks[1].clips[0].effects,
        "B1" => &project.timelines[0].buses[0].strip.inserts,
        track => &self::track(project, track).audio.strip.inserts,
    };
    list.iter()
        .map(|i| (i.id.clone(), i.effect.clone()))
        .collect()
}

#[test]
fn inserts_are_added_put_in_place_taken_away_and_moved_with_their_ids() {
    let mut project = project();
    let a1 = InsertsOf::Track("A1".into());
    let add = |index: Option<usize>, insert: Insert| Op::AddInsert {
        timeline: "t1".into(),
        of: a1.clone(),
        insert,
        index,
    };
    let (done, inverse) = applied(&mut project, add(None, gain(-1.0)));
    assert_eq!(
        serde_json::to_value(&done).unwrap(),
        json!({"op": "add_insert", "timeline": "t1", "of": {"track": "A1"},
            "insert": {"id": "fx1", "on": true, "effect": {"kind": "gain", "gain_db": -1.0}},
            "index": 0})
    );
    assert_eq!(
        serde_json::to_value(&inverse).unwrap(),
        json!({"op": "set_insert", "timeline": "t1", "of": {"track": "A1"}, "slot": 0, "insert": null})
    );
    applied(&mut project, add(Some(0), gain(-2.0)));
    assert_eq!(
        inserts(&project, "A1")
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
        ["fx2", "fx1"]
    );
    let (_, inverse) = applied(
        &mut project,
        Op::MoveInsert {
            timeline: "t1".into(),
            of: a1.clone(),
            from: 0,
            to: 1,
        },
    );
    assert_eq!(
        inverse,
        Op::MoveInsert {
            timeline: "t1".into(),
            of: a1.clone(),
            from: 1,
            to: 0
        }
    );
    assert_eq!(
        inserts(&project, "A1")
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
        ["fx1", "fx2"]
    );
    // A Limiter in place of fx2, given the next id; the inverse puts fx2 back.
    let limiter = Insert {
        id: String::new(),
        on: false,
        effect: Effect::Limiter(LimiterFx::default()),
    };
    let (done, inverse) = applied(
        &mut project,
        Op::SetInsert {
            timeline: "t1".into(),
            of: a1.clone(),
            slot: 1,
            insert: Some(limiter),
        },
    );
    let Op::SetInsert {
        insert: Some(put), ..
    } = done
    else {
        panic!()
    };
    assert_eq!(put.id, "fx2");
    let Op::SetInsert {
        insert: Some(back), ..
    } = inverse
    else {
        panic!()
    };
    assert_eq!(back.effect, Effect::Gain(GainFx { gain_db: -2.0 }));
    // Taken away: its inverse adds it back where it was.
    let (_, inverse) = applied(
        &mut project,
        Op::SetInsert {
            timeline: "t1".into(),
            of: a1.clone(),
            slot: 0,
            insert: None,
        },
    );
    let Op::AddInsert { insert, index, .. } = inverse else {
        panic!()
    };
    assert_eq!((insert.id.as_str(), index), ("fx1", Some(0)));
    for (op, error) in [
        (
            Op::SetInsert {
                timeline: "t1".into(),
                of: a1.clone(),
                slot: 1,
                insert: None,
            },
            json!({"reason": "out_of_list", "at": 1, "len": 1}),
        ),
        (
            Op::MoveInsert {
                timeline: "t1".into(),
                of: a1.clone(),
                from: 0,
                to: 3,
            },
            json!({"reason": "out_of_list", "at": 3, "len": 1}),
        ),
        (
            Op::AddInsert {
                timeline: "t1".into(),
                of: InsertsOf::Track("V1".into()),
                insert: gain(0.0),
                index: None,
            },
            json!({"reason": "wrong_track", "what": "strip", "kind": "video"}),
        ),
        (
            Op::AddInsert {
                timeline: "t1".into(),
                of: InsertsOf::Bus("B5".into()),
                insert: gain(0.0),
                index: None,
            },
            json!({"reason": "unknown_bus", "id": "B5"}),
        ),
    ] {
        assert_eq!(refused(&mut project, op), error);
    }
    for _ in 0..5 {
        applied(&mut project, add(None, gain(0.0)));
    }
    assert_eq!(
        refused(&mut project, add(None, gain(0.0))),
        json!({"reason": "too_many_inserts"})
    );
    // The `insert` of `set_insert` is written, as null to take one away.
    assert!(
        serde_json::from_value::<Op>(json!({"op": "set_insert", "timeline": "t1",
        "of": {"track": "A1"}, "slot": 0}))
        .is_err()
    );
}

#[test]
fn the_effects_of_a_clip_and_of_a_bus_are_chains_of_inserts_too() {
    let mut project = project();
    let c2 = InsertsOf::Clip("c2".into());
    applied(
        &mut project,
        Op::AddInsert {
            timeline: "t1".into(),
            of: c2.clone(),
            insert: gain(6.0),
            index: None,
        },
    );
    // An effect of a clip keeps the id it is given, which may be none.
    assert_eq!(inserts(&project, "c2")[0].0, "");
    let mut sidechained = LimiterFx::default();
    sidechained.sidechain.on = true;
    assert_eq!(
        refused(
            &mut project,
            Op::AddInsert {
                timeline: "t1".into(),
                of: c2.clone(),
                insert: Insert {
                    id: String::new(),
                    on: true,
                    effect: Effect::Limiter(sidechained),
                },
                index: None,
            }
        ),
        json!({"reason": "out_of_range", "what": "sidechain"})
    );
    applied(
        &mut project,
        Op::AddInsert {
            timeline: "t1".into(),
            of: InsertsOf::Bus("B1".into()),
            insert: gain(-3.0),
            index: None,
        },
    );
    assert_eq!(inserts(&project, "B1")[0].0, "fx1");
}

#[test]
fn set_clip_sets_the_sound_of_schema_4_of_a_clip() {
    let mut project = project();
    let mut eq = Equalizer::clip();
    eq.on = true;
    eq.bands[1].gain_db = 4.0;
    let channel = ChannelMap {
        source: columbina_model::ChannelSources::default(),
        enabled: true,
        mute: false,
        level_db: -3.0,
    };
    let (done, inverse) = applied(
        &mut project,
        Op::SetClip {
            timeline: "t1".into(),
            id: "c2".into(),
            enabled: None,
            speed: None,
            reverse: None,
            link: None,
            transform: None,
            gain_db: None,
            pan: None,
            fade_in: None,
            fade_out: None,
            audio_fade_in: None,
            audio_fade_out: None,
            eq: Some(eq.clone()),
            effects: Some(vec![gain(2.0)]),
            pitch_correction: Some(true),
            pitch: Some(ClipPitch {
                semitones: -2,
                cents: 50.0,
            }),
            channels: Some(Some(vec![channel.clone(), channel.clone()])),
            source: None,
        },
    );
    let clip = &project.timelines[0].tracks[1].clips[0];
    assert_eq!(clip.eq, eq);
    assert!(clip.pitch_correction);
    assert_eq!(clip.channels.as_ref().unwrap().len(), 2);
    let written = serde_json::to_value(&inverse).unwrap();
    assert_eq!(written["channels"], Value::Null);
    assert_eq!(written["effects"], json!([]));
    assert_eq!(written["pitch_correction"], json!(false));
    assert!(serde_json::to_value(&done).unwrap()["channels"].is_array());
    // A clip that names one channel of a stereo track, a pitch too high.
    let mut wrong = serde_json::to_value(&done).unwrap();
    wrong["channels"] = json!([{"source": null}]);
    assert_eq!(
        refused(&mut project, serde_json::from_value(wrong.clone()).unwrap()),
        json!({"reason": "channel_count", "track": "A1"})
    );
    wrong["channels"] = Value::Null;
    wrong["pitch"] = json!({"semitones": 13});
    assert_eq!(
        refused(&mut project, serde_json::from_value(wrong).unwrap()),
        json!({"reason": "out_of_range", "what": "pitch"})
    );
}

#[test]
fn the_levels_of_a_crossfade_are_set_and_a_dissolve_has_none() {
    let mut project = project();
    // c2 cut in two, a crossfade between the halves.
    let timeline = &mut project.timelines[0];
    let mut half = timeline.tracks[1].clips[0].clone();
    timeline.tracks[1].clips[0].length = 50;
    half.id = "c3".into();
    half.start = 50;
    half.length = 50;
    half.source = columbina_model::Source::Media {
        media: "m1".into(),
        source_in: columbina_model::Time {
            ticks: 2 * 15360,
            scale: 15360,
        },
    };
    timeline.tracks[1].clips.push(half);
    timeline.tracks[1].transitions.push(Transition {
        id: "x1".into(),
        kind: TransitionKind::Crossfade,
        left: "c2".into(),
        right: "c3".into(),
        length: 10,
        align: columbina_model::Alignment::Center,
        fade_in_level_db: CrossfadeLevel::Zero,
        fade_out_level_db: CrossfadeLevel::Zero,
    });
    project.next_id = 4;
    project.validate().unwrap();
    let (_, inverse) = applied(
        &mut project,
        op(json!({"op": "set_transition", "timeline": "t1", "id": "x1",
            "fade_in_level_db": 3, "fade_out_level_db": -3})),
    );
    assert_eq!(
        serde_json::to_value(&inverse).unwrap(),
        json!({"op": "set_transition", "timeline": "t1", "id": "x1",
            "fade_in_level_db": 0, "fade_out_level_db": 0})
    );
    let x1 = &project.timelines[0].tracks[1].transitions[0];
    assert_eq!(
        (x1.fade_in_level_db, x1.fade_out_level_db),
        (CrossfadeLevel::Plus3, CrossfadeLevel::Minus3)
    );
    assert!(
        serde_json::from_value::<Op>(json!({"op": "set_transition", "timeline": "t1",
        "id": "x1", "fade_in_level_db": 2}))
        .is_err()
    );
}

#[test]
fn the_sound_of_a_timeline_is_set_by_a_patch() {
    let mut project = project();
    let (done, inverse) = applied(
        &mut project,
        op(json!({"op": "set_timeline_audio", "timeline": "t1",
            "patch": {"exclusive_solo": true, "loudness": {"target": -16, "standard": "youtube"}}})),
    );
    assert!(project.timelines[0].audio.exclusive_solo);
    assert_eq!(
        serde_json::to_value(&inverse).unwrap()["patch"],
        json!({"exclusive_solo": false, "loudness": {"target": -23.0, "standard": "ebu_r128"}})
    );
    assert_eq!(
        serde_json::to_value(&done).unwrap()["patch"]["loudness"]["target"],
        json!(-16.0)
    );
    assert_eq!(
        refused(
            &mut project,
            op(json!({"op": "set_timeline_audio", "timeline": "t1", "patch": {"monitor": "B4"}}))
        ),
        json!({"reason": "unknown_bus", "id": "B4"})
    );
    assert_eq!(
        refused(
            &mut project,
            op(json!({"op": "set_timeline_audio", "timeline": "t1",
                "patch": {"loudness": {"target": -5}}}))
        ),
        json!({"reason": "out_of_range", "what": "audio"})
    );
}

#[test]
fn the_ids_of_the_engine_skip_none_and_a_bus_vca_or_group_given_one_keeps_it() {
    let mut project = project();
    let bus = |id: &str| Op::AddBus {
        timeline: "t1".into(),
        bus: Bus {
            id: id.into(),
            name: id.into(),
            ..Bus::first()
        },
        index: None,
    };
    applied(&mut project, bus("B7"));
    let (done, _) = applied(&mut project, bus(""));
    let Op::AddBus { bus: made, .. } = done else {
        panic!()
    };
    assert_eq!(made.id, "B8");
    let (done, _) = applied(
        &mut project,
        Op::AddVca {
            timeline: "t1".into(),
            vca: Vca {
                id: "Strings".into(),
                name: "Strings".into(),
                fader_db: 0.0,
                mute: false,
                solo: false,
                vcas: vec![],
                lanes: columbina_model::Lanes::default(),
            },
            index: Some(0),
        },
    );
    assert_eq!(
        serde_json::to_value(&done).unwrap()["vca"]["id"],
        json!("Strings")
    );
    let group = TrackGroup {
        id: "TG3".into(),
        name: "g".into(),
        color: columbina_model::Rgba([0, 0, 0, 255]),
        members: vec!["A1".into()],
        linked: vec![],
        active: true,
    };
    applied(
        &mut project,
        Op::AddTrackGroup {
            timeline: "t1".into(),
            group: group.clone(),
            index: None,
        },
    );
    let (done, _) = applied(
        &mut project,
        Op::AddTrackGroup {
            timeline: "t1".into(),
            group: TrackGroup {
                id: String::new(),
                ..group
            },
            index: None,
        },
    );
    assert_eq!(
        serde_json::to_value(&done).unwrap()["group"]["id"],
        json!("TG4")
    );
}
