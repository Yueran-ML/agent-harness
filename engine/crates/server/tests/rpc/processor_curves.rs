//! Actual JSON-RPC read-only processor curves, without media or audio playback.
use serde_json::json;

#[tokio::test]
async fn static_processor_curves_follow_model_edits_and_never_add_history() {
    let dir = super::project_dir();
    let engine = super::engine(
        dir.path(),
        &["--headless", "--no-proxies", "--sound", "null", "--idle-seconds", "1"],
    )
    .await;
    let mut client = engine.client("agent").await;
    client.apply(json!([{"op":"add_timeline", "timeline": {
        "id":"curves", "name":"Curves", "rate":{"num":25,"den":1}, "width":64,"height":36,"sample_rate":48000,
        "tracks":[{"id":"curve-track","name":"Voice","kind":"audio","enabled":true,"locked":false,"clips":[],"transitions":[]}]
    }}])).await;
    client.apply(json!([{"edit":"set_track_audio","timeline":"curves","track":"curve-track","patch":{"strip":{
        "eq":{"on":true,"gain_db":3},
        "dynamics":{"on":true,"makeup_db":2,"compressor":{"on":true}}
    }}}])).await;
    let before = super::project(&mut client).await;
    let eq_params =
        json!({"timeline":"curves","target":{"track":"curve-track"},"frequencies":[20,1000,20000]});
    let eq = client
        .result("audio.eq_response", Some(eq_params.clone()))
        .await;
    assert_eq!(eq["revision"], before["revision"]);
    assert_eq!(eq["sample_rate"], 48000);
    assert_eq!(eq["gain_db"], json!([3.0, 3.0, 3.0]));
    let dynamics = client.result("audio.dynamics_curve", Some(json!({"timeline":"curves","target":{"track":"curve-track"},"inputs_db":[-60,-20,0]}))).await;
    assert_eq!(dynamics["revision"], before["revision"]);
    assert_eq!(dynamics["outputs_db"], json!([-58.0, -18.0, -5.5]));
    for params in [
        json!({"timeline":"curves","target":{"track":"missing"},"frequencies":[1000]}),
        json!({"timeline":"curves","target":{"track":"curve-track"},"frequencies":[0]}),
        json!({"timeline":"curves","target":{"track":"curve-track"},"frequencies":vec![1000;513]}),
    ] {
        assert_eq!(
            client.error("audio.eq_response", Some(params)).await["code"],
            -32602
        );
    }
    assert_eq!(super::project(&mut client).await, before);
    client.result("history.undo", None).await;
    assert_eq!(
        client.result("audio.eq_response", Some(eq_params)).await["gain_db"],
        json!([0.0, 0.0, 0.0])
    );
    drop(client);
    assert_eq!(engine.ended().await, 0);
}
