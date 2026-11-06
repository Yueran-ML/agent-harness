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
