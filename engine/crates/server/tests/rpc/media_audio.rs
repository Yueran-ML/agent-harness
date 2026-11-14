//! Pool attributes and multi-track edits through the real engine connection.
use columbina_model::Project;
use serde_json::json;

use super::{engine, project, project_dir, Client};

#[tokio::test]
async fn pool_layout_splits_linked_instances_and_undo_restores_the_tracks() {
    let dir = project_dir();
    let mut sample = Project::sample();
    let audio = sample.media[0].facts.audio.as_mut().unwrap();
    audio.channels = 4;
    audio.streams = Some(
        serde_json::from_value(json!([
            {"index":3,"channels":4,"layout":null}
        ]))
        .unwrap(),
    );
    std::fs::write(
        dir.path().join("project.json"),
        serde_json::to_vec_pretty(&sample).unwrap(),
    )
    .unwrap();
    let engine = engine(dir.path(), &["--headless", "--idle-seconds", "1"]).await;
    let mut client = Client::connect(&engine.address).await;
    client.hello("desktop").await;
    let before = project(&mut client).await;
    let config: Vec<_> = (0..4)
        .map(|channel| {
            json!({
                "name":format!("Mic {}", channel + 1),"format":"mono",
                "channels":[{"source":{"stream":3,"channel":channel}}]
            })
        })
        .collect();
    client
        .result(
            "transaction.apply",
            Some(json!({"revision":before["revision"],"ops":[{
                "op":"set_media_audio","id":"m1","config":config
            }]})),
        )
        .await;
    let configured = project(&mut client).await;
    assert_eq!(
        configured["project"]["timelines"],
        before["project"]["timelines"]
    );
    assert_eq!(
        configured["project"]["media"][0]["audio_config"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    client.result("transaction.apply", Some(json!({"revision":configured["revision"],"ops":[{
        "edit":"overwrite","timeline":"t1","source":{"kind":"media","media":"m1","length":25},"at":150
    }]}))).await;
    let placed = project(&mut client).await;
    let tracks = placed["project"]["timelines"][0]["tracks"]
        .as_array()
        .unwrap();
    let mono: Vec<_> = tracks
        .iter()
        .filter(|track| track["kind"] == "audio" && track["id"] != "A1")
        .collect();
    assert_eq!(mono.len(), 4);
    let group = &mono[0]["clips"][0]["link"];
    assert!(group.is_string());
    for (index, track) in mono.iter().enumerate() {
        assert_eq!(track["format"], "mono");
        assert_eq!(track["clips"][0]["link"], *group);
        assert_eq!(
            track["clips"][0]["channels"][0]["source"],
            json!({"stream":3,"channel":index})
        );
    }
    client.result("history.undo", None).await;
    assert_eq!(
        project(&mut client).await["project"]["timelines"],
        before["project"]["timelines"]
    );
    client.result("history.undo", None).await;
    assert_eq!(
        project(&mut client).await["project"]["media"],
        before["project"]["media"]
    );
    drop(client);
    assert_eq!(engine.ended().await, 0);
}

#[tokio::test]
async fn a_partially_changed_automatic_layout_roundtrips_through_history_and_disk() {
    let dir = project_dir();
    let sample = Project::sample();
    let timeline = &sample.timelines[0];
    let track = timeline
        .tracks
        .iter()
        .find(|track| track.kind == columbina_model::TrackKind::Audio)
        .unwrap();
    let id = track.clips[0].id.clone();
    let timeline_id = timeline.id.clone();
    std::fs::write(
        dir.path().join("project.json"),
        serde_json::to_vec_pretty(&sample).unwrap(),
    )
    .unwrap();
    let engine = engine(dir.path(), &["--headless", "--idle-seconds", "1"]).await;
    let mut client = Client::connect(&engine.address).await;
    client.hello("desktop").await;
    let before = project(&mut client).await;
    client
        .result(
            "transaction.apply",
            Some(json!({"revision":before["revision"],"ops":[{
                "op":"set_clip", "timeline":timeline_id, "id":id,
                "channels":[
                    {"source":null,"remix":{"channels":2,"channel":0},"level_db":-6},
                    {"source":{"stream":0,"channel":0}}
                ]
            }]})),
        )
        .await;
    let changed = project(&mut client).await;
    assert_ne!(
        changed["project"]["timelines"],
        before["project"]["timelines"]
    );
    let loaded: Project = serde_json::from_value(changed["project"].clone()).unwrap();
    let maps = loaded.timelines[0]
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
        .find(|clip| clip.id == id)
        .unwrap()
        .channels
        .as_ref()
        .unwrap();
    assert_eq!(
        maps[0].remix,
        Some(columbina_model::RemixSource {
            channels: 2,
            channel: 0
        })
    );
    assert!(maps[1].remix.is_none());
    client.result("history.undo", None).await;
    assert_eq!(project(&mut client).await["project"], before["project"]);
    client.result("history.redo", None).await;
    assert_eq!(project(&mut client).await["project"], changed["project"]);
    drop(client);
    assert_eq!(engine.ended().await, 0);
    let saved: Project =
        serde_json::from_slice(&std::fs::read(dir.path().join("project.json")).unwrap()).unwrap();
    assert_eq!(saved, loaded);
}
