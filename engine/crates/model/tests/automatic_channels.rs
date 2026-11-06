use columbina_model::{ChannelMap, History, Project};
use serde_json::json;

#[test]
fn omitted_remix_keeps_old_serialization_and_an_explicit_output_round_trips() {
    let old = json!({"source":{"stream":0,"channel":1},"enabled":true,"mute":false,"level_db":0.0});
    let channel: ChannelMap = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(serde_json::to_value(channel).unwrap(), old);
    let output = json!({"source":null,"remix":{"channels":6,"channel":4},"enabled":true,"mute":false,"level_db":-3.0});
    let channel: ChannelMap = serde_json::from_value(output.clone()).unwrap();
    assert_eq!(serde_json::to_value(channel).unwrap(), output);
}

#[test]
fn automatic_outputs_validate_ranges_and_cannot_also_pick_native_sources() {
    for (count, channel, valid) in [
        (1, 0, true),
        (24, 23, true),
        (0, 0, false),
        (25, 0, false),
        (2, 2, false),
    ] {
        let map: ChannelMap =
            serde_json::from_value(json!({"remix":{"channels":count,"channel":channel}})).unwrap();
        assert_eq!(
            columbina_model::audio::check_channel_maps(&[map]).is_ok(),
            valid
        );
    }
    let map: ChannelMap = serde_json::from_value(
        json!({"source":{"stream":0,"channel":0},"remix":{"channels":2,"channel":0}}),
    )
    .unwrap();
    assert!(columbina_model::audio::check_channel_maps(&[map]).is_err());
}

#[test]
fn partial_source_override_and_trim_are_one_undoable_saved_mapping() {
    let original = Project::sample();
    let clip = &original.timelines[0].tracks[1].clips[0];
    let op = serde_json::from_value(json!({"op":"set_clip","timeline":original.timelines[0].id,"id":clip.id,
        "channels":[{"remix":{"channels":2,"channel":0},"level_db":-6.0},{"source":{"stream":0,"channel":0}}]})).unwrap();
    let mut history = History::new(original.clone());
    history.apply(vec![op], "test").unwrap();
    let changed = history.project().clone();
    let mut saved: Project =
        serde_json::from_slice(&serde_json::to_vec(&changed).unwrap()).unwrap();
    saved.validate().unwrap();
    assert_eq!(saved, changed);
    history.undo("test").unwrap().unwrap();
    assert_eq!(history.project(), &original);
    history.redo("test").unwrap().unwrap();
    assert_eq!(history.project(), &changed);
}
