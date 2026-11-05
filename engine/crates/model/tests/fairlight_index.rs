//! T20: changing track presentation preserves its identity and all its sound references.
use columbina_model::{apply, apply_done, Op, Project, Track, TrackKind};
use serde_json::json;

fn op(value: serde_json::Value) -> Op {
    serde_json::from_value(value).unwrap()
}

#[test]
fn move_track_preserves_the_object_and_undo_restores_the_full_order() {
    let mut project = Project::sample();
    project.timelines[0]
        .tracks
        .push(Track::new("A2", TrackKind::Audio));
    let original = project.clone();
    let moved = project.timelines[0].tracks[2].clone();
    let (done, undo) = apply_done(
        &mut project,
        op(json!({"op":"move_track","timeline":"t1","id":"A2","index":0})),
    )
    .unwrap();
    assert_eq!(project.timelines[0].tracks[0], moved);
    let after = project.clone();
    apply(&mut project, undo).unwrap();
    assert_eq!(project, original);
    apply(&mut project, done).unwrap();
    assert_eq!(project, after);
}

#[test]
fn move_track_rejects_missing_out_of_range_and_unchanged_without_mutating() {
    let mut project = Project::sample();
    let before = project.clone();
    for value in [
        json!({"id":"missing","index":0}),
        json!({"id":"V1","index":2}),
        json!({"id":"V1","index":0}),
    ] {
        let request =
            json!({"op":"move_track","timeline":"t1","id":value["id"],"index":value["index"]});
        assert!(apply(&mut project, op(request)).is_err());
        assert_eq!(project, before);
    }
}

#[test]
fn optional_track_color_roundtrips_and_undo_restores_absence() {
    let mut project = Project::sample();
    let before = project.clone();
    let (_, undo) = apply_done(
        &mut project,
        op(json!({"op":"set_track","timeline":"t1","id":"A1","color":"#5aa0e0ff"})),
    )
    .unwrap();
    let encoded = serde_json::to_value(&project).unwrap();
    assert_eq!(encoded["timelines"][0]["tracks"][1]["color"], "#5aa0e0ff");
    assert_eq!(serde_json::from_value::<Project>(encoded).unwrap(), project);
    apply(&mut project, undo).unwrap();
    assert_eq!(project, before);
    assert!(
        serde_json::to_value(&project).unwrap()["timelines"][0]["tracks"][1]
            .get("color")
            .is_none()
    );
}
