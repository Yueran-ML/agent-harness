//! Clipboard operations through the actual revision-checked engine connection.

use columbina_model::Project;
use serde_json::json;

use super::{engine, project, project_dir, Client};

#[tokio::test]
async fn copy_cut_paste_keeps_its_snapshot_and_each_edit_undoes_once() {
    let dir = project_dir();
    let mut sample = Project::sample();
    let mut voice = sample.timelines[0].tracks[1].clips[0].clone();
    voice.start = 50;
    voice.length = 50;
    voice.link = None;
    let clip = voice.id.clone();
    for track in &mut sample.timelines[0].tracks {
        track.clips.clear();
        track.transitions.clear();
    }
    sample.timelines[0].tracks[1].clips.push(voice);
    sample.validate().unwrap();
    std::fs::write(
        dir.path().join("project.json"),
        serde_json::to_vec_pretty(&sample).unwrap(),
    )
    .unwrap();
    let engine = engine(dir.path(), &["--headless", "--idle-seconds", "1"]).await;
    let mut client = Client::connect(&engine.address).await;
    client.hello("desktop").await;
    let before = project(&mut client).await;
    let request = json!({"timeline":"t1","revision":before["revision"],"clips":[clip],"at":75,"from":60,"to":90});
    let copied = client.result("clips.copy", Some(request.clone())).await;
    assert_eq!(project(&mut client).await, before);
    assert_eq!(copied["clipboard"]["clips"][0]["clip"]["start"], 60);
    let mut stale = request.clone();
    stale["revision"] = json!(before["revision"].as_u64().unwrap() + 1);
    assert_eq!(
        client.error("clips.copy", Some(stale)).await["data"]["reason"],
        "stale_revision"
    );
    let mut partial = request;
    partial.as_object_mut().unwrap().remove("to");
    assert_eq!(
        client.error("clips.copy", Some(partial)).await["data"]["reason"],
        "range"
    );
    let cut = client
        .result(
            "transaction.apply",
            Some(json!({"revision":copied["revision"],"ops":[{
                "edit":"cut_clips","timeline":"t1","clips":[clip],"from":60,"to":90,"linked":false
            }]})),
        )
        .await;
    let after_cut = project(&mut client).await;
    assert_eq!(
        after_cut["project"]["timelines"][0]["tracks"][1]["clips"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    client
        .result(
            "transaction.apply",
            Some(json!({"revision":cut["revision"],"ops":[{
                "edit":"paste_clips","timeline":"t1","clipboard":copied["clipboard"],"at":225
            }]})),
        )
        .await;
    let pasted = project(&mut client).await;
    let clips = pasted["project"]["timelines"][0]["tracks"][1]["clips"]
        .as_array()
        .unwrap();
    let placed = clips.iter().find(|c| c["start"] == 210).unwrap();
    assert_eq!(placed["length"], 30);
    assert_eq!(
        placed["source"],
        copied["clipboard"]["clips"][0]["clip"]["source"]
    );
    client.result("history.undo", None).await;
    assert_eq!(
        project(&mut client).await["project"]["timelines"],
        after_cut["project"]["timelines"]
    );
    client.result("history.undo", None).await;
    assert_eq!(
        project(&mut client).await["project"]["timelines"],
        before["project"]["timelines"]
    );
    drop(client);
    assert_eq!(engine.ended().await, 0);
}
