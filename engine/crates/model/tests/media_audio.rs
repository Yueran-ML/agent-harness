//! Media-pool layouts affect new edits, never existing instances.
use columbina_model::{apply, Edit, History, MediaAudioTrack, Op, Project, TrackKind};
use serde_json::{json, Value};

fn mono(channel: u32) -> MediaAudioTrack {
    serde_json::from_value(
        json!({"name":format!("Mic {}", channel + 1),"format":"mono",
        "channels":[{"source":{"stream":3,"channel":channel}}]}),
    )
    .unwrap()
}

fn project() -> Project {
    let mut p = Project::sample();
    let audio = p.media[0].facts.audio.as_mut().unwrap();
    audio.channels = 4;
    audio.streams = Some(
        serde_json::from_value(json!([
            {"index":3,"channels":4,"layout":null}
        ]))
        .unwrap(),
    );
    p.media[0].audio_config = Some((0..4).map(mono).collect());
    p.validate().unwrap();
    p
}

fn placement(kind: &str) -> Edit {
    let mut value = json!({"edit":kind,"timeline":"t1",
        "source":{"kind":"media","media":"m1","from":{"ticks":0,"scale":15360},
        "to":{"ticks":15360,"scale":15360}},"at":0});
    if kind == "append" {
        value.as_object_mut().unwrap().remove("at");
    }
    serde_json::from_value(value).unwrap()
}

#[test]
fn four_mics_become_linked_mono_clips_without_reformatting_existing_tracks() {
    for kind in [
        "insert",
        "overwrite",
        "append",
        "smart_insert",
        "place_on_top",
        "ripple_overwrite",
    ] {
        let before = project();
        let old_audio = before.timelines[0].tracks[1].audio.clone();
        let mut history = History::new(before.clone());
        history.edit(vec![placement(kind)], "test").unwrap();
        let p = history.project();
        let tracks = &p.timelines[0].tracks;
        assert_eq!(
            tracks.iter().find(|t| t.id == "A1").unwrap().audio,
            old_audio,
            "{kind}"
        );
        let mono_tracks: Vec<_> = tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Audio && t.id != "A1")
            .collect();
        assert_eq!(mono_tracks.len(), 4, "{kind}");
        let link = mono_tracks[0].clips[0].link.clone().unwrap();
        for (index, track) in mono_tracks.iter().enumerate() {
            assert_eq!(track.audio.channels, 1, "{kind}");
            assert_eq!(track.name, format!("Mic {}", index + 1));
            assert_eq!(track.clips.len(), 1, "{kind}");
            let clip = &track.clips[0];
            assert_eq!(clip.length, 25);
            assert_eq!(clip.link.as_deref(), Some(link.as_str()));
            assert_eq!(
                clip.channels.as_ref().unwrap(),
                &vec![mono(u32::try_from(index).unwrap()).channels[0].clone()]
            );
        }
        assert!(tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Video)
            .flat_map(|t| &t.clips)
            .any(|c| c.link.as_deref() == Some(link.as_str())));
        p.clone().validate().unwrap();
        history.undo("test").unwrap().unwrap();
        assert_eq!(history.project().timelines, before.timelines, "{kind}");
        assert_eq!(history.project().media, before.media, "{kind}");
    }
}

#[test]
fn changing_and_resetting_pool_settings_preserves_existing_instances_and_round_trips() {
    let before = Project::sample();
    assert!(serde_json::to_value(&before).unwrap()["media"][0]
        .get("audio_config")
        .is_none());
    let mut history = History::new(before.clone());
    history
        .apply(
            vec![Op::SetMediaAudio {
                id: "m1".into(),
                config: Some(vec![mono(1)]),
            }],
            "test",
        )
        .unwrap();
    assert_eq!(history.project().timelines, before.timelines);
    let written = serde_json::to_value(history.project()).unwrap();
    assert_eq!(Project::from_value(written).unwrap(), *history.project());
    history
        .apply(
            vec![Op::SetMediaAudio {
                id: "m1".into(),
                config: None,
            }],
            "test",
        )
        .unwrap();
    assert!(serde_json::to_value(history.project()).unwrap()["media"][0]
        .get("audio_config")
        .is_none());
    history.undo("test").unwrap().unwrap();
    assert_eq!(history.project().media[0].audio_config, Some(vec![mono(1)]));
    history.undo("test").unwrap().unwrap();
    assert_eq!(history.project().media, before.media);
    assert!(serde_json::from_value::<Op>(json!({"op":"set_media_audio","id":"m1"})).is_err());
}

#[test]
fn inactive_channels_are_removed_but_mute_level_and_mixed_sources_are_copied() {
    let mut p = project();
    let mut config = vec![mono(0), mono(1)];
    config[0].channels[0].enabled = false;
    config[1].channels[0].mute = true;
    config[1].channels[0].level_db = -7.0;
    config[1].channels[0].source = serde_json::from_value(json!([
        {"stream":3,"channel":1},{"stream":3,"channel":2}
    ]))
    .unwrap();
    p.media[0].audio_config = Some(config.clone());
    let mut history = History::new(p);
    history.edit(vec![placement("overwrite")], "test").unwrap();
    let placed = history.project().timelines.clone();
    let audio: Vec<_> = placed[0]
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Audio && t.id != "A1")
        .collect();
    assert_eq!(audio.len(), 1);
    assert_eq!(audio[0].clips[0].channels, Some(config[1].channels.clone()));
    history
        .apply(
            vec![Op::SetMediaAudio {
                id: "m1".into(),
                config: None,
            }],
            "test",
        )
        .unwrap();
    assert_eq!(history.project().timelines, placed);
}

#[test]
fn reducing_a_stereo_layout_routes_only_the_active_channel_to_a_mono_track() {
    let mut p = project();
    let config: MediaAudioTrack = serde_json::from_value(json!({
        "name":"Right mic", "format":"stereo", "channels":[
            {"source":{"stream":3,"channel":0},"enabled":false},
            {"source":{"stream":3,"channel":1},"level_db":-3}
        ]
    }))
    .unwrap();
    p.media[0].audio_config = Some(vec![config]);
    let mut history = History::new(p);
    history.edit(vec![placement("overwrite")], "test").unwrap();
    let track = history.project().timelines[0]
        .tracks
        .iter()
        .find(|t| t.name == "Right mic")
        .unwrap();
    assert_eq!(
        track.audio.format,
        columbina_model::audio::TrackFormat::Mono
    );
    assert_eq!(track.clips[0].channels.as_ref().unwrap().len(), 1);
    assert_eq!(
        track.clips[0].channels.as_ref().unwrap()[0].source,
        mono(1).channels[0].source
    );
    assert_eq!(track.clips[0].channels.as_ref().unwrap()[0].level_db, -3.0);
}

#[test]
fn explicitly_disabling_the_audio_target_never_creates_tracks() {
    let p = project();
    let old_audio = p.timelines[0].tracks[1].clone();
    let mut history = History::new(p);
    let mut edit = serde_json::to_value(placement("overwrite")).unwrap();
    edit["audio_track"] = Value::Null;
    history
        .edit(vec![serde_json::from_value(edit).unwrap()], "test")
        .unwrap();
    assert_eq!(history.project().timelines[0].tracks.len(), 2);
    assert_eq!(history.project().timelines[0].tracks[1], old_audio);
}

#[test]
fn matching_tracks_are_reused_and_locked_targets_fail_atomically() {
    let mut p = project();
    apply(
        &mut p,
        serde_json::from_value(json!({"op":"set_track_audio","timeline":"t1","track":"A1",
        "patch":{"format":"mono","channels":1}}))
        .unwrap(),
    )
    .unwrap();
    let mut history = History::new(p.clone());
    history.edit(vec![placement("overwrite")], "test").unwrap();
    assert_eq!(history.project().timelines[0].tracks.len(), 5);
    assert_eq!(
        history.project().timelines[0]
            .tracks
            .iter()
            .find(|t| t.id == "A1")
            .unwrap()
            .clips[0]
            .channels,
        Some(mono(0).channels)
    );
    p.timelines[0].tracks[1].locked = true;
    let mut history = History::new(p.clone());
    let mut edit = serde_json::to_value(placement("overwrite")).unwrap();
    edit["audio_track"] = "A1".into();
    assert!(history
        .edit(vec![serde_json::from_value(edit).unwrap()], "test")
        .is_err());
    assert_eq!(history.project(), &p);
}

#[test]
fn invalid_layouts_leave_the_entire_transaction_unchanged() {
    let before = project();
    for patch in [
        json!({"format":"stereo"}),
        json!({"channels":[]}),
        json!({"order":"film"}),
        json!({"channels":[{"level_db":25}]}),
    ] {
        let mut row = serde_json::to_value(mono(0)).unwrap();
        row.as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        let mut history = History::new(before.clone());
        let op: Op =
            serde_json::from_value(json!({"op":"set_media_audio","id":"m1","config":[row]}))
                .unwrap();
        assert!(history.apply(vec![op], "test").is_err());
        assert_eq!(history.project(), &before);
    }
    let mut silent = before;
    silent.media[0].facts.audio = None;
    silent.media[0].audio_config = None;
    assert!(apply(
        &mut silent,
        Op::SetMediaAudio {
            id: "m1".into(),
            config: Some(vec![mono(0)])
        }
    )
    .is_err());
}

#[test]
fn replacing_an_instance_keeps_its_mapping_and_an_empty_layout_inserts_only_video() {
    let mut p = project();
    let mapping = vec![mono(1).channels[0].clone(), mono(0).channels[0].clone()];
    p.timelines[0].tracks[1].clips[0].channels = Some(mapping.clone());
    let mut history = History::new(p);
    history.edit(vec![placement("replace")], "test").unwrap();
    assert_eq!(history.project().timelines[0].tracks.len(), 2);
    assert_eq!(
        history.project().timelines[0].tracks[1].clips[0].channels,
        Some(mapping)
    );
    history
        .apply(
            vec![Op::SetMediaAudio {
                id: "m1".into(),
                config: Some(vec![]),
            }],
            "test",
        )
        .unwrap();
    let old_audio = history.project().timelines[0].tracks[1].clone();
    let mut edit: Value = serde_json::to_value(placement("overwrite")).unwrap();
    edit["at"] = 150.into();
    history
        .edit(vec![serde_json::from_value(edit).unwrap()], "test")
        .unwrap();
    assert_eq!(history.project().timelines[0].tracks[1], old_audio);
}

#[test]
fn a_configured_ripple_overwrite_removes_the_replaced_range_from_the_default_track() {
    // The clip found at `at` on the default tracks is the one replaced. Its sound on A1 goes
    // with it even though the new sound is routed to the configured tracks: keeping it would
    // leave the old sound under the new picture, and ripple timing must hold on every track.
    let mut p = project();
    for (track, id) in [(0, "c3"), (1, "c4")] {
        let mut later = p.timelines[0].tracks[track].clips[0].clone();
        later.id = id.into();
        later.start = 100;
        later.length = 50;
        p.timelines[0].tracks[track].clips.push(later);
    }
    p.validate().unwrap();
    let edit: Edit = serde_json::from_value(json!({"edit":"ripple_overwrite","timeline":"t1",
        "source":{"kind":"media","media":"m1","from":{"ticks":0,"scale":15360},
        "to":{"ticks":15360,"scale":15360}},"at":10}))
    .unwrap();
    let mut history = History::new(p);
    history.edit(vec![edit], "test").unwrap();
    let p = history.project();
    let tracks = &p.timelines[0].tracks;
    let spans = |id: &str| -> Vec<(String, i64, i64)> {
        tracks
            .iter()
            .find(|t| t.id == id)
            .unwrap()
            .clips
            .iter()
            .map(|c| (c.id.clone(), c.start, c.start + c.length))
            .collect()
    };
    let v1 = spans("V1");
    assert_eq!(v1.len(), 2);
    assert_eq!((v1[0].1, v1[0].2), (0, 25));
    assert_eq!(v1[1], ("c3".to_owned(), 25, 75));
    assert_eq!(spans("A1"), [("c4".to_owned(), 25, 75)]);
    let mono: Vec<_> = tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Audio && t.id != "A1")
        .collect();
    assert_eq!(mono.len(), 4);
    assert!(mono
        .iter()
        .all(|t| t.clips.len() == 1 && (t.clips[0].start, t.clips[0].length) == (0, 25)));
    p.clone().validate().unwrap();
}
