use columbina_model::{apply, Change, Edit, History, LanesOf, Op, Project, Track, TrackKind};
use serde_json::json;

#[test]
fn separated_group_copies_keep_each_slices_automation_and_undo_together() {
    let mut p = editing_project();
    let source = &mut p.timelines[0]
        .tracks
        .iter_mut()
        .find(|t| t.id == "A1")
        .unwrap()
        .clips;
    source[0].length = 25;
    let mut second = source[0].clone();
    second.id = "second".into();
    second.start = 100;
    source.push(second);
    let copies: Vec<_> = source
        .iter()
        .map(|c| json!({"clip": c.id, "track": "A1", "start": c.start + 150}))
        .collect();
    let lane = serde_json::from_value(json!({"op":"set_lane","timeline":"t1","of":{"track":"A2"},"path":"fader_db",
        "lane":{"points":[{"at":{"ticks":0,"scale":25},"value":0}, {"at":{"ticks":150,"scale":25},"value":-12}]}})).unwrap();
    apply(&mut p, lane).unwrap();
    let before = p.clone();
    let bed = |p: &Project| {
        p.timelines[0]
            .tracks
            .iter()
            .find(|t| t.id == "A2")
            .unwrap()
            .audio
            .lanes["fader_db"]
            .clone()
    };
    let original = bed(&before);
    let mut history = History::new(p);
    let edit = serde_json::from_value(
        json!({"edit":"place_copies","timeline":"t1","grouped":true,"copies":copies}),
    )
    .unwrap();
    history.edit(vec![edit], "desktop").unwrap();
    let result = bed(history.project());
    for start in [50, 100] {
        for offset in 0..25 {
            let source_time = columbina_model::Time {
                ticks: start + offset,
                scale: 25,
            };
            let destination_time = columbina_model::Time {
                ticks: start + offset + 150,
                scale: 25,
            };
            assert!(
                (original.value_at(source_time) - result.value_at(destination_time)).abs() < 1e-12
            );
        }
    }
    for ticks in 0..150 {
        let time = columbina_model::Time { ticks, scale: 25 };
        assert!((original.value_at(time) - result.value_at(time)).abs() < 1e-12);
    }
    history.undo("desktop").unwrap().unwrap();
    assert_eq!(history.project().timelines, before.timelines);
}

fn project() -> Project {
    let mut project = Project::sample();
    let group: Op = serde_json::from_value(json!({"op":"add_track_group","timeline":"t1",
        "group":{"id":"TG1","name":"All Group","members":[],"all_tracks":true}}))
    .unwrap();
    apply(&mut project, group).unwrap();
    // Added after All Group: no stored member list needs repairing.
    project.timelines[0]
        .tracks
        .push(Track::new("A2", TrackKind::Audio));
    project.validate().unwrap();
    project
}

#[test]
fn all_group_includes_new_tracks_and_undo_restores_one_fader_edit() {
    let before = project();
    let mut history = History::new(before.clone());
    let edit: Edit = serde_json::from_value(
        json!({"edit":"set_track_audio","timeline":"t1","track":"A1",
        "patch":{"strip":{"fader_db":-3}},"follow_groups":true}),
    )
    .unwrap();
    history
        .change(vec![Change::Edit(edit)], "desktop", |change, _| change)
        .unwrap();
    assert_eq!(
        history.project().timelines[0]
            .tracks
            .iter()
            .find(|t| t.id == "A2")
            .unwrap()
            .audio
            .strip
            .fader_db,
        -3.0
    );
    history.undo("desktop").unwrap().unwrap();
    assert_eq!(history.project().timelines, before.timelines);
}

#[test]
fn bypass_preserves_group_settings_and_stops_mix_and_automation_followers() {
    let mut project = project();
    let of = LanesOf::Track("A1".into());
    assert_eq!(
        columbina_model::edit::grouped_with(&project.timelines[0], &of, "fader_db"),
        vec!["A2"]
    );
    let bypass = serde_json::from_value(
        json!({"op":"set_timeline_audio","timeline":"t1","patch":{"groups_bypassed":true}}),
    )
    .unwrap();
    let inverse = apply(&mut project, bypass).unwrap();
    assert!(project.timelines[0].track_groups[0].active);
    assert_eq!(
        columbina_model::edit::grouped_with(&project.timelines[0], &of, "fader_db"),
        Vec::<String>::new()
    );
    let edit = serde_json::from_value(json!({"edit":"set_track_audio","timeline":"t1","track":"A1","patch":{"strip":{"fader_db":-3}}})).unwrap();
    columbina_model::expand(&mut project, edit).unwrap();
    assert_eq!(
        project.timelines[0]
            .tracks
            .iter()
            .find(|t| t.id == "A2")
            .unwrap()
            .audio
            .strip
            .fader_db,
        0.0
    );
    apply(&mut project, inverse).unwrap();
    assert_eq!(
        columbina_model::edit::grouped_with(&project.timelines[0], &of, "fader_db"),
        vec!["A2"]
    );
}

#[test]
fn ordinary_projects_default_to_enabled_groups_and_only_one_all_group_is_allowed() {
    let before = Project::sample();
    let written = serde_json::to_value(&before).unwrap();
    assert_eq!(written["timelines"][0]["audio"]["groups_bypassed"], false);
    let mut project = project();
    let duplicate = serde_json::from_value(json!({"op":"add_track_group","timeline":"t1",
        "group":{"id":"TG2","name":"Another All","members":[],"all_tracks":true}}))
    .unwrap();
    let saved = project.clone();
    assert!(apply(&mut project, duplicate).is_err());
    assert_eq!(project, saved);
}

#[test]
fn old_projects_without_group_flags_deserialize_and_roundtrip() {
    let mut value = serde_json::to_value(project()).unwrap();
    value["timelines"][0]["audio"]
        .as_object_mut()
        .unwrap()
        .remove("groups_bypassed");
    value["timelines"][0]["track_groups"][0]
        .as_object_mut()
        .unwrap()
        .remove("all_tracks");
    let loaded: Project = serde_json::from_value(value).unwrap();
    assert!(!loaded.timelines[0].audio.groups_bypassed);
    assert!(!loaded.timelines[0].track_groups[0].all_tracks);
    assert_eq!(
        serde_json::from_value::<Project>(serde_json::to_value(&loaded).unwrap()).unwrap(),
        loaded
    );
}

fn editing_project() -> Project {
    let mut p = project();
    let source = p.timelines[0]
        .tracks
        .iter_mut()
        .find(|t| t.id == "A1")
        .unwrap();
    let mut clip = source.clips[0].clone();
    clip.start = 50;
    clip.length = 50;
    clip.link = None;
    source.clips = vec![clip.clone()];
    clip.id = "bed".into();
    clip.start = 0;
    clip.length = 150;
    p.timelines[0]
        .tracks
        .iter_mut()
        .find(|t| t.id == "A2")
        .unwrap()
        .clips = vec![clip];
    p.validate().unwrap();
    p
}
#[test]
fn editing_bypass_keeps_the_bed_and_all_group_lift_takes_only_its_range() {
    for bypass in [false, true] {
        let mut p = editing_project();
        p.timelines[0].audio.groups_bypassed = bypass;
        let id = p.timelines[0]
            .tracks
            .iter()
            .find(|t| t.id == "A1")
            .unwrap()
            .clips[0]
            .id
            .clone();
        let before = p.clone();
        let mut history = History::new(p);
        let edit = serde_json::from_value(
            json!({"edit":"lift","timeline":"t1","clips":[id],"linked":false,"grouped":true}),
        )
        .unwrap();
        history
            .change(vec![Change::Edit(edit)], "desktop", |change, _| change)
            .unwrap();
        let bed = &history.project().timelines[0]
            .tracks
            .iter()
            .find(|t| t.id == "A2")
            .unwrap()
            .clips;
        assert_eq!(
            bed.iter().map(|c| (c.start, c.length)).collect::<Vec<_>>(),
            if bypass {
                vec![(0, 150)]
            } else {
                vec![(0, 50), (100, 50)]
            }
        );
        history.undo("desktop").unwrap().unwrap();
        assert_eq!(history.project().timelines, before.timelines);
    }
}
#[test]
fn grouped_copy_preserves_long_source_and_copies_only_the_span_with_one_undo() {
    for bypass in [false, true] {
        let mut p = editing_project();
        p.timelines[0].audio.groups_bypassed = bypass;
        let id = p.timelines[0]
            .tracks
            .iter()
            .find(|t| t.id == "A1")
            .unwrap()
            .clips[0]
            .id
            .clone();
        let before = p.clone();
        let mut history = History::new(p);
        let edit = serde_json::from_value(json!({"edit":"place_copies","timeline":"t1","copies":[{"clip":id,"track":"A1","start":200}],"grouped":true})).unwrap();
        history
            .change(vec![Change::Edit(edit)], "desktop", |change, _| change)
            .unwrap();
        let bed = &history.project().timelines[0]
            .tracks
            .iter()
            .find(|t| t.id == "A2")
            .unwrap()
            .clips;
        assert_eq!(
            bed[0],
            before.timelines[0]
                .tracks
                .iter()
                .find(|t| t.id == "A2")
                .unwrap()
                .clips[0]
        );
        assert_eq!(
            bed.iter().map(|c| (c.start, c.length)).collect::<Vec<_>>(),
            if bypass {
                vec![(0, 150)]
            } else {
                vec![(0, 150), (200, 50)]
            }
        );
        if !bypass {
            let copied = bed[1].source_in().unwrap();
            let original = bed[0].source_in().unwrap();
            assert_eq!(copied.ticks - original.ticks, i64::from(original.scale) * 2);
        }
        history.undo("desktop").unwrap().unwrap();
        assert_eq!(history.project().timelines, before.timelines);
    }
}

#[test]
fn pan_and_send_relative_followers_return_from_clamps_when_restored_per_preview() {
    let mut p = project();
    for (id, x, gain) in [("A1", 0.0, 0.0), ("A2", 0.9, 9.0)] {
        let op = serde_json::from_value(json!({"op":"set_track_audio","timeline":"t1","track":id,
            "patch":{"strip":{"pan":{"x":x},"sends":[{"bus":"B1","level_db":gain}]}}}))
        .unwrap();
        apply(&mut p, op).unwrap();
    }
    let original = p.clone();
    for (x, gain, wanted_x, wanted_gain) in [(0.5, 5.0, 1.0, 10.0), (0.0, 0.0, 0.9, 9.0)] {
        // The frontend sends these original snapshots with follow_groups false before each preview.
        for id in ["A1", "A2"] {
            let strip = &original.timelines[0]
                .tracks
                .iter()
                .find(|t| t.id == id)
                .unwrap()
                .audio
                .strip;
            let restore =
                serde_json::from_value(json!({"edit":"set_track_audio","timeline":"t1","track":id,
                "patch":{"strip":{"pan":strip.pan,"sends":strip.sends}},"follow_groups":false}))
                .unwrap();
            columbina_model::expand(&mut p, restore).unwrap();
        }
        let edit = serde_json::from_value(json!({"edit":"set_track_audio","timeline":"t1","track":"A1",
            "patch":{"strip":{"pan":{"x":x},"sends":[{"bus":"B1","level_db":gain}]}},"follow_groups":true})).unwrap();
        columbina_model::expand(&mut p, edit).unwrap();
        let strip = &p.timelines[0]
            .tracks
            .iter()
            .find(|t| t.id == "A2")
            .unwrap()
            .audio
            .strip;
        assert_eq!(strip.pan.x, wanted_x);
        assert_eq!(strip.sends[0].level_db, wanted_gain);
    }
    assert_eq!(
        columbina_model::edit::grouped_with(&p.timelines[0], &LanesOf::Track("A1".into()), "pan.x"),
        vec!["A2"]
    );
    assert_eq!(
        columbina_model::edit::grouped_with(
            &p.timelines[0],
            &LanesOf::Track("A1".into()),
            "send.B1.level_db"
        ),
        vec!["A2"]
    );
    assert_eq!(
        columbina_model::edit::grouped_with(
            &p.timelines[0],
            &LanesOf::Track("A1".into()),
            "insert.I1.gain_db"
        ),
        Vec::<String>::new()
    );
}
#[test]
fn grouped_copy_merges_overlapping_spans_and_retains_reverse_source_mapping() {
    let mut p = editing_project();
    let source = p.timelines[0]
        .tracks
        .iter()
        .find(|t| t.id == "A1")
        .unwrap()
        .clips[0]
        .clone();
    let mut second = source.clone();
    second.id = "second".into();
    second.start = 100;
    p.timelines[0]
        .tracks
        .iter_mut()
        .find(|t| t.id == "A1")
        .unwrap()
        .clips
        .push(second.clone());
    p.timelines[0]
        .tracks
        .iter_mut()
        .find(|t| t.id == "A2")
        .unwrap()
        .clips[0]
        .reverse = true;
    let before = p.clone();
    let edit = serde_json::from_value(json!({"edit":"place_copies","timeline":"t1","grouped":true,
        "copies":[{"clip":source.id,"track":"A1","start":200},{"clip":second.id,"track":"A1","start":250}]})).unwrap();
    columbina_model::expand(&mut p, edit).unwrap();
    let clips = &p.timelines[0]
        .tracks
        .iter()
        .find(|t| t.id == "A2")
        .unwrap()
        .clips;
    assert_eq!(
        clips
            .iter()
            .map(|c| (c.start, c.length))
            .collect::<Vec<_>>(),
        vec![(0, 150), (200, 100)]
    );
    assert!(clips[1].reverse);
    assert_eq!(
        clips[1].source_in(),
        before.timelines[0]
            .tracks
            .iter()
            .find(|t| t.id == "A2")
            .unwrap()
            .clips[0]
            .source_in()
    );
}
