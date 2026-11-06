use std::collections::BTreeMap;

use columbina_model::edit::{copy_clips, ClipClipboard};
use columbina_model::{apply, Clip, Edit, History, Project, Rate, Time, Track, TrackKind};
use serde_json::json;

fn project() -> Project {
    let mut p = Project::sample();
    let mut voice = p.timelines[0].tracks[1].clips[0].clone();
    voice.start = 50;
    voice.length = 50;
    voice.fade_in = 10;
    voice.link = None;
    for track in &mut p.timelines[0].tracks {
        track.clips.clear();
        track.transitions.clear();
    }
    p.timelines[0].tracks[1].clips.push(voice.clone());
    voice.id = "bed".into();
    voice.start = 0;
    voice.length = 150;
    p.timelines[0]
        .tracks
        .push(Track::new("A2", TrackKind::Audio).with_clips(vec![voice]));
    for track in ["A1", "A2"] {
        apply(&mut p, serde_json::from_value(json!({"op":"set_lane","timeline":"t1","of":{"track":track},"path":"fader_db",
            "lane":{"points":[{"at":{"ticks":0,"scale":25},"value":0},{"at":{"ticks":150,"scale":25},"value":-12}]}})).unwrap()).unwrap();
    }
    p.validate().unwrap();
    p
}

fn copy(p: &Project, range: Option<(i64, i64)>, grouped: bool) -> ClipClipboard {
    let id = p.timelines[0].tracks[1].clips[0].id.clone();
    copy_clips(&p.timelines[0], &[id], 75, range, false, grouped).unwrap()
}

#[test]
fn cutting_a_range_keeps_both_tails_of_group_peers_and_undo_restores_all_lanes() {
    let mut p = project();
    group(&mut p);
    let id = p.timelines[0].tracks[1].clips[0].id.clone();
    let clipboard = copy(&p, Some((60, 90)), true);
    let before = p.clone();
    let mut history = History::new(p);
    history
        .edit(
            vec![Edit::CutClips {
                timeline: "t1".into(),
                clips: vec![id],
                from: Some(60),
                to: Some(90),
                linked: false,
                grouped: true,
            }],
            "desktop",
        )
        .unwrap();
    let tracks = &history.project().timelines[0].tracks;
    assert_eq!(
        tracks[1]
            .clips
            .iter()
            .map(|c| (c.start, c.length))
            .collect::<Vec<_>>(),
        vec![(50, 10), (90, 10)]
    );
    assert_eq!(
        tracks[2]
            .clips
            .iter()
            .map(|c| (c.start, c.length))
            .collect::<Vec<_>>(),
        vec![(0, 60), (90, 60)]
    );
    let cut = history.project().clone();
    history
        .edit(vec![paste(clipboard, 225, BTreeMap::new())], "desktop")
        .unwrap();
    history.undo("desktop").unwrap().unwrap();
    assert_eq!(history.project().timelines, cut.timelines);
    history.undo("desktop").unwrap().unwrap();
    assert_eq!(history.project().timelines, before.timelines);
}

fn paste(clipboard: ClipClipboard, at: i64, tracks: BTreeMap<String, String>) -> Edit {
    Edit::PasteClips {
        timeline: "t1".into(),
        clipboard,
        at,
        tracks,
    }
}

fn group(p: &mut Project) {
    apply(
        p,
        serde_json::from_value(json!({"op":"add_track_group","timeline":"t1","group":{
        "id":"TG1","name":"Voices and bed","members":["A1","A2"],"linked":["editing"]}}))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn a_snapshot_survives_cut_and_retains_the_playhead_anchor_settings_and_automation() {
    let mut p = project();
    group(&mut p);
    let unchanged = p.clone();
    let clipboard = copy(&p, Some((60, 90)), true);
    assert_eq!(p, unchanged);
    assert_eq!(clipboard.clips.len(), 2);
    assert!(clipboard
        .clips
        .iter()
        .all(|c| c.clip.start == 60 && c.clip.length == 30));
    let id = p.timelines[0].tracks[1].clips[0].id.clone();
    columbina_model::expand(
        &mut p,
        serde_json::from_value(
            json!({"edit":"lift","timeline":"t1","clips":[id],"linked":false,"grouped":true}),
        )
        .unwrap(),
    )
    .unwrap();
    let before_paste = p.clone();
    let mut history = History::new(p);
    history
        .edit(
            vec![paste(clipboard.clone(), 225, BTreeMap::new())],
            "desktop",
        )
        .unwrap();
    for item in clipboard.clips {
        let track = history.project().timelines[0]
            .tracks
            .iter()
            .find(|t| t.id == item.track)
            .unwrap();
        let original = unchanged.timelines[0]
            .tracks
            .iter()
            .find(|t| t.id == item.track)
            .unwrap();
        let inserted = track.clips.iter().find(|c| c.start == 210).unwrap();
        assert_eq!(inserted.length, 30);
        assert_eq!(inserted.source, item.clip.source);
        assert_eq!(inserted.gain_db, item.clip.gain_db);
        for offset in 0..30 {
            let a = original.audio.lanes["fader_db"].value_at(Time {
                ticks: 60 + offset,
                scale: 25,
            });
            let b = track.audio.lanes["fader_db"].value_at(Time {
                ticks: 210 + offset,
                scale: 25,
            });
            assert!((a - b).abs() < 1e-12);
        }
    }
    history.undo("desktop").unwrap().unwrap();
    assert_eq!(history.project().timelines, before_paste.timelines);
}

#[test]
fn captured_values_do_not_follow_later_source_changes_and_paste_can_be_repeated() {
    let mut p = project();
    let clipboard = copy(&p, None, false);
    let captured = clipboard.clips[0].clip.clone();
    p.timelines[0].tracks[1].clips[0].gain_db = serde_json::from_value(json!(-20)).unwrap();
    columbina_model::expand(&mut p, paste(clipboard.clone(), 225, BTreeMap::new())).unwrap();
    columbina_model::expand(&mut p, paste(clipboard, 325, BTreeMap::new())).unwrap();
    let clips = &p.timelines[0].tracks[1].clips;
    for start in [200, 300] {
        let inserted = clips.iter().find(|c| c.start == start).unwrap();
        assert_eq!(inserted.gain_db, captured.gain_db);
        assert_eq!(inserted.fade_in, captured.fade_in);
        assert_ne!(inserted.id, captured.id);
    }
    assert_ne!(clips[1].id, clips[2].id);
}

#[test]
fn different_rates_keep_duration_and_anchor_and_track_mapping_is_explicit() {
    let mut p = project();
    let clipboard = copy(&p, None, false);
    p.timelines[0].rate = Rate { num: 50, den: 1 };
    let mut history = History::new(p);
    history
        .edit(
            vec![paste(
                clipboard,
                450,
                BTreeMap::from([("A1".into(), "A2".into())]),
            )],
            "desktop",
        )
        .unwrap();
    let clip = history.project().timelines[0].tracks[2]
        .clips
        .iter()
        .find(|c| c.start == 400)
        .unwrap();
    assert_eq!((clip.start, clip.length, clip.fade_in), (400, 100, 20));
    assert_eq!(history.project().timelines[0].tracks[1].clips.len(), 1);
}

#[test]
fn invalid_later_content_rolls_back_earlier_overwrites_and_insertions() {
    let mut p = project();
    let mut clipboard = copy(&p, None, false);
    let mut bad = clipboard.clips[0].clone();
    bad.track = "A2".into();
    bad.clip = Clip::of_media(
        "bad",
        "missing",
        Time {
            ticks: 0,
            scale: 25,
        },
        50,
        50,
    );
    clipboard.clips.push(bad);
    let before = p.clone();
    assert!(columbina_model::expand(&mut p, paste(clipboard, 75, BTreeMap::new())).is_err());
    assert_eq!(p, before);
}

#[test]
fn locked_wrong_kind_negative_position_and_malformed_lanes_are_refused_atomically() {
    let original = project();
    let clipboard = copy(&original, None, false);
    for scenario in 0..4 {
        let mut p = original.clone();
        let mut captured = serde_json::to_value(&clipboard).unwrap();
        let mut tracks = BTreeMap::new();
        let at = if scenario == 2 { 0 } else { 225 };
        if scenario == 0 {
            p.timelines[0].tracks[1].locked = true;
        }
        if scenario == 1 {
            tracks.insert("A1".into(), "V1".into());
        }
        if scenario == 3 {
            captured["clips"][0]["lanes"][0]["scale"] = json!(0);
        }
        let before = p.clone();
        let saved: ClipClipboard = serde_json::from_value(captured).unwrap();
        assert!(
            columbina_model::expand(&mut p, paste(saved, at, tracks)).is_err(),
            "scenario {scenario}"
        );
        assert_eq!(p, before);
    }
}

#[test]
fn disabled_groups_and_follows_edit_keep_their_independent_meanings() {
    let mut p = project();
    group(&mut p);
    p.timelines[0].audio.groups_bypassed = true;
    let clipboard = copy(&p, None, true);
    assert_eq!(clipboard.clips.len(), 1);
    p.timelines[0].audio.follows_edit = false;
    let lanes = p.timelines[0].tracks[1].audio.lanes.clone();
    columbina_model::expand(&mut p, paste(clipboard, 225, BTreeMap::new())).unwrap();
    assert_eq!(p.timelines[0].tracks[1].audio.lanes, lanes);
    assert_eq!(
        copy(&p, None, false).clips[0].lanes,
        Vec::<columbina_model::edit::ClipLane>::new()
    );
}

#[test]
fn linked_selection_pastes_a_fresh_link_group_and_roundtrips_as_json() {
    let mut p = project();
    let mut picture = p.timelines[0].tracks[1].clips[0].clone();
    picture.id = "picture".into();
    picture.link = Some("pair".into());
    p.timelines[0].tracks[1].clips[0].link = picture.link.clone();
    p.timelines[0].tracks[0].clips.push(picture);
    p.validate().unwrap();
    let id = p.timelines[0].tracks[1].clips[0].id.clone();
    let clipboard = copy_clips(&p.timelines[0], &[id], 75, None, true, false).unwrap();
    assert_eq!(clipboard.clips.len(), 2);
    let encoded = serde_json::to_vec(&clipboard).unwrap();
    let decoded: ClipClipboard = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded, clipboard);
    columbina_model::expand(&mut p, paste(decoded, 225, BTreeMap::new())).unwrap();
    let video = p.timelines[0].tracks[0]
        .clips
        .iter()
        .find(|c| c.start == 200)
        .unwrap();
    let audio = p.timelines[0].tracks[1]
        .clips
        .iter()
        .find(|c| c.start == 200)
        .unwrap();
    assert!(video.link.is_some());
    assert_ne!(video.link.as_deref(), Some("pair"));
    assert_eq!(video.link, audio.link);
}

fn with_crossfade() -> Project {
    let mut p = project();
    let left = &mut p.timelines[0].tracks[1].clips[0];
    left.fade_in = 0;
    let mut right = left.clone();
    right.id = "right".into();
    right.start = left.end();
    *right.source_in_mut().unwrap() = Time {
        ticks: 2 * 15360,
        scale: 15360,
    };
    p.timelines[0].tracks[1].clips.push(right);
    apply(
        &mut p,
        serde_json::from_value(json!({
            "op":"add_transition", "timeline":"t1", "track":"A1",
            "transition":{"id":"", "kind":"crossfade", "left":"c2", "right":"right",
                "length":10, "align":"center", "fade_in_level_db":-3, "fade_out_level_db":3}
        }))
        .unwrap(),
    )
    .unwrap();
    p.validate().unwrap();
    p
}

#[test]
fn a_crossfade_survives_cut_and_paste_with_fresh_ids_and_one_undo() {
    let p = with_crossfade();
    let clipboard = copy_clips(
        &p.timelines[0],
        &["c2".into(), "right".into()],
        100,
        None,
        false,
        false,
    )
    .unwrap();
    assert_eq!(clipboard.transitions.len(), 1);
    let original = p.timelines[0].tracks[1].transitions[0].clone();
    let mut history = History::new(p);
    history
        .edit(
            vec![Edit::CutClips {
                timeline: "t1".into(),
                clips: vec!["c2".into(), "right".into()],
                from: None,
                to: None,
                linked: false,
                grouped: false,
            }],
            "desktop",
        )
        .unwrap();
    let cut = history.project().clone();
    history
        .edit(vec![paste(clipboard, 250, BTreeMap::new())], "desktop")
        .unwrap();
    let track = &history.project().timelines[0].tracks[1];
    let copied = &track.transitions[0];
    assert_ne!(copied.id, original.id);
    assert_eq!(copied.left, track.clips[0].id);
    assert_eq!(copied.right, track.clips[1].id);
    assert_eq!(copied.length, original.length);
    assert_eq!(copied.kind, original.kind);
    assert_eq!(copied.align, original.align);
    assert_eq!(copied.fade_in_level_db, original.fade_in_level_db);
    assert_eq!(copied.fade_out_level_db, original.fade_out_level_db);
    history.undo("desktop").unwrap().unwrap();
    assert_eq!(history.project().timelines, cut.timelines);
}

#[test]
fn transition_capture_requires_both_sides_and_ranges_use_existing_trim_fitting() {
    let mut p = with_crossfade();
    assert_eq!(copy(&p, None, false).transitions.len(), 0);
    let clipboard = copy_clips(
        &p.timelines[0],
        &["c2".into(), "right".into()],
        100,
        Some((98, 102)),
        false,
        false,
    )
    .unwrap();
    columbina_model::expand(&mut p, paste(clipboard, 250, BTreeMap::new())).unwrap();
    let transitions = &p.timelines[0].tracks[1].transitions;
    assert_eq!(transitions.len(), 2);
    assert_eq!(transitions[1].length, 4);
    assert_eq!(
        transitions[1].fade_in_level_db,
        transitions[0].fade_in_level_db
    );
}

#[test]
fn transition_duration_converts_to_destination_rate_and_invalid_indices_are_atomic() {
    let mut p = with_crossfade();
    let clipboard = copy_clips(
        &p.timelines[0],
        &["c2".into(), "right".into()],
        100,
        None,
        false,
        false,
    )
    .unwrap();
    let mut target = p.timelines[0].clone();
    target.id = "destination".into();
    target.rate = Rate { num: 50, den: 1 };
    for track in &mut target.tracks {
        track.clips.clear();
        track.transitions.clear();
    }
    p.timelines.push(target);
    let edit = Edit::PasteClips {
        timeline: "destination".into(),
        clipboard: clipboard.clone(),
        at: 300,
        tracks: BTreeMap::new(),
    };
    columbina_model::expand(&mut p, edit).unwrap();
    assert_eq!(p.timelines[1].tracks[1].transitions[0].length, 20);
    let before = p.clone();
    let mut invalid = clipboard;
    invalid.transitions[0].right = invalid.clips.len();
    assert!(columbina_model::expand(&mut p, paste(invalid, 100, BTreeMap::new())).is_err());
    assert_eq!(p, before);
}
