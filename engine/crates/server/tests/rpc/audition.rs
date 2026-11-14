//! Audition ownership, lifecycle and monitor exclusion through the actual RPC connection.
use super::{engine, project, project_dir, support, transport_engine_with, Client, WAIT};
use serde_json::{json, Value};

async fn audition(client: &mut Client, action: Value) -> Value {
    client.result("audio.audition", Some(action)).await
}

async fn play(client: &mut Client, media: &Value) -> Value {
    audition(client, json!({"action":"play", "target":{"kind":"media","media":media}, "at":{"ticks":0,"scale":48000}, "channel":null})).await
}

async fn stopped(client: &mut Client) {
    tokio::time::timeout(WAIT, async {
        loop {
            if audition(client, json!({"action":"state"})).await["playing"] == false {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn file_audition_owns_only_its_playback_and_never_changes_the_project() {
    let dir = project_dir();
    let Some((engine, mut client)) =
        transport_engine_with(dir.path(), &[support::Sample::Wav], &["--sound", "null"]).await
    else {
        return;
    };
    let before = project(&mut client).await;
    let media = before["project"]["media"][0]["id"].clone();
    client
        .result(
            "transport.load",
            Some(json!({"monitor":"source","media":media})),
        )
        .await;
    client
        .result("transport.play", Some(json!({"monitor":"source"})))
        .await;
    client
        .until_state("source playing", |s| {
            s["monitor"] == "source" && s["playing"] == true
        })
        .await;
    let error = client.error("audio.audition", Some(json!({"action":"play", "target":{"kind":"media","media":media}, "at":{"ticks":0,"scale":0}, "channel":null}))).await;
    assert_eq!(error["code"], -32602);
    assert_eq!(
        client
            .result("transport.state", Some(json!({"monitor":"source"})))
            .await["playing"],
        true
    );
    let first = play(&mut client, &media).await;
    assert_eq!(first["playing"], true, "{first}");
    assert_eq!(
        client
            .result("transport.state", Some(json!({"monitor":"source"})))
            .await["playing"],
        false
    );
    let second = play(&mut client, &media).await;
    let error = client.error("audio.audition", Some(json!({"action":"play", "target":{"kind":"media","media":"missing"}, "at":{"ticks":0,"scale":48000}, "channel":null}))).await;
    assert_eq!(error["code"], -32602);
    let still_playing = audition(&mut client, json!({"action":"state"})).await;
    assert_eq!(still_playing["audition"], second["audition"]);
    assert_eq!(still_playing["playing"], true);
    assert_eq!(
        audition(
            &mut client,
            json!({"action":"stop","audition":first["audition"]})
        )
        .await["playing"],
        true
    );
    let mut other = engine.client("desktop").await;
    assert_eq!(
        audition(
            &mut other,
            json!({"action":"stop","audition":second["audition"]})
        )
        .await["playing"],
        true
    );
    assert_eq!(project(&mut client).await, before);
    client
        .result("transport.play", Some(json!({"monitor":"source"})))
        .await;
    client
        .until_state("source resumed", |s| {
            s["monitor"] == "source" && s["playing"] == true
        })
        .await;
    stopped(&mut client).await;
    play(&mut client, &media).await;
    client
        .result(
            "transport.load",
            Some(json!({"monitor":"source","media":media})),
        )
        .await;
    stopped(&mut client).await;
    play(&mut client, &media).await;
    drop(client);
    stopped(&mut other).await;
    play(&mut other, &media).await;
    other
        .apply(json!([{"op":"set_media_audio","id":media,"config":[]}]))
        .await;
    stopped(&mut other).await;
}

#[tokio::test]
async fn skims_leave_a_playing_monitor_alone_and_channel_edits_are_heard_at_once() {
    let dir = project_dir();
    let Some((_engine, mut client)) =
        transport_engine_with(dir.path(), &[support::Sample::Wav], &["--sound", "null"]).await
    else {
        return;
    };
    let before = project(&mut client).await;
    let media = before["project"]["media"][0]["id"].clone();
    client
        .result(
            "transport.load",
            Some(json!({"monitor":"source","media":media})),
        )
        .await;
    client
        .result("transport.play", Some(json!({"monitor":"source"})))
        .await;
    client
        .until_state("source playing", |s| {
            s["monitor"] == "source" && s["playing"] == true
        })
        .await;
    // Hovering the waveforms while a monitor plays leaves that monitor playing.
    let skim = audition(&mut client, json!({"action":"play", "target":{"kind":"media","media":media}, "at":{"ticks":0,"scale":48000}, "channel":null, "skim":true})).await;
    assert_eq!(skim["playing"], false, "{skim}");
    assert_eq!(
        client
            .result("transport.state", Some(json!({"monitor":"source"})))
            .await["playing"],
        true
    );
    // An edit of the channels is heard at once: the same audition plays on with the new layout.
    let playing = play(&mut client, &media).await;
    let stream = before["project"]["media"][0]["facts"]["audio"]["streams"][0]["index"].clone();
    client
        .apply(json!([{"op":"set_media_audio","id":media,"config":[{
            "name":"Left","format":"mono",
            "channels":[{"source":{"stream":stream,"channel":0},"level_db":-6.0}]
        }]}]))
        .await;
    let after = audition(&mut client, json!({"action":"state"})).await;
    assert_eq!(after["audition"], playing["audition"], "{after}");
    assert_eq!(after["playing"], true, "{after}");
}

#[tokio::test]
async fn headless_state_and_stale_stop_are_harmless_but_play_is_refused() {
    let dir = project_dir();
    let engine = engine(dir.path(), &["--headless"]).await;
    let mut client = engine.client("agent").await;
    let initial = audition(&mut client, json!({"action":"state"})).await;
    assert_eq!(
        initial,
        json!({"audition":null,"playing":false,"position":null,"error":null})
    );
    assert_eq!(
        audition(&mut client, json!({"action":"stop","audition":1})).await,
        initial
    );
    let error = client.error("audio.audition", Some(json!({"action":"play","target":{"kind":"media","media":"m1"},"at":{"ticks":0,"scale":1},"channel":null}))).await;
    assert_eq!(error["code"], -32601);
    let error = client
        .error(
            "audio.audition",
            Some(json!({"action":"state","surprise":true})),
        )
        .await;
    assert_eq!(error["code"], -32602);
}
