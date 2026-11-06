//! One real engine: user presets, input-point Learn with undo, and playing/stopped FX readers.
use serde_json::json;
use std::time::{Duration, Instant};

#[tokio::test]
async fn presets_learn_and_live_fx_readings_use_real_engine() {
    let dir = super::project_dir();
    let library = tempfile::tempdir().unwrap();
    let mut engine = match super::start_in(
        dir.path(),
        &["--no-proxies", "--sound", "null"],
        super::Stderr::Read,
        &[("COLUMBINA_AUDIO_PRESETS", library.path().as_os_str())],
    )
    .await
    {
        super::Started::Listening(engine) => *engine,
        super::Started::Ended { code, stderr } => panic!("{code}: {stderr}"),
    };
    let mut client = engine.client("desktop").await;
    let wav = dir.path().join("noise.wav");
    super::sound::wav(&wav, 2, 48000 * 8, |n, c| {
        0.06 * (std::f64::consts::TAU * (997.0 + f64::from(c) * 211.0) * f64::from(n) / 48000.0)
            .sin()
    });
    let media = client
        .result("media.import", Some(json!({"paths":[wav]})))
        .await["added"][0]
        .clone();
    let mut clip = super::clip("c", media.as_str().unwrap(), 0, 200);
    clip["source"]["in"] = json!({"ticks":0,"scale":48000});
    client.apply(json!([{"op":"add_timeline","timeline":{"id":"fx","name":"FX","rate":{"num":25,"den":1},"width":64,"height":36,"sample_rate":48000,"tracks":[{"id":"a","name":"Audio","kind":"audio","enabled":true,"locked":false,"clips":[clip],"transitions":[]}]}}])).await;
    client.apply(json!([{"edit":"set_track_audio","timeline":"fx","track":"a","patch":{"strip":{"inserts":[
        {"id":"gain","on":true,"effect":{"kind":"gain","gain_db":6}},
        {"id":"nr","on":true,"effect":{"kind":"noise_reduction"}},
        {"id":"width","on":true,"effect":{"kind":"stereo_width"}},
        {"id":"limiter","on":true,"effect":{"kind":"limiter"}},
        {"id":"soft","on":true,"effect":{"kind":"soft_clipper"}}
    ],"dynamics":{"on":true}}}}])).await;
    let before = super::project(&mut client).await;
    let payload = json!({"kind":"effect","settings":{"kind":"gain","gain_db":-3}});
    let preset = client
        .result(
            "audio_presets.save",
            Some(json!({"name":"Test gain","payload":payload})),
        )
        .await;
    assert_eq!(
        client.result("audio_presets.list", Some(json!({}))).await["presets"][0],
        preset
    );
    client.result("audio_presets.update",Some(json!({"preset":preset["id"],"payload":{"kind":"effect","settings":{"kind":"gain","gain_db":-6}}}))).await;
    assert_eq!(
        client.result("audio_presets.list", None).await["presets"][0]["payload"]["settings"]["gain_db"],
        -6.0
    );
    client
        .result("audio_presets.delete", Some(json!({"preset":preset["id"]})))
        .await;
    assert_eq!(
        client.result("audio_presets.list", None).await["presets"],
        json!([])
    );
    assert_eq!(super::project(&mut client).await, before);
    let curve = client
        .result(
            "audio.soft_clip_curve",
            Some(json!({"timeline":"fx","target":{"track":"a"},"slot":4,"inputs":[-1,0,1]})),
        )
        .await;
    assert_eq!(curve["outputs"][1], 0.0);
    assert!(curve["outputs"][2].as_f64().unwrap() < 1.0);
    assert_eq!(curve["revision"], before["revision"]);
    let learn = json!({"timeline":"fx","target":{"track":"a"},"slot":1,"range":{"from":0,"to":25},"revision":before["revision"]});
    let job = client
        .result("audio.noise_learn", Some(learn.clone()))
        .await;
    let (done, _) = super::sound::ended(&mut client, "audio.analysis", &job["job"]).await;
    assert_eq!(done["state"], "done", "{done}");
    assert!(done["result"]["applied"].is_object(), "{done}");
    assert_eq!(done["profile"].as_array().unwrap().len(), 1025);
    let learned = super::project(&mut client).await;
    let effect = &learned["project"]["timelines"][0]["tracks"][0]["strip"]["inserts"][1]["effect"];
    assert_eq!(effect["mode"], "manual", "{learned}");
    assert_eq!(effect["profile"], done["profile"]);
    assert_eq!(
        client.error("audio.noise_learn", Some(learn)).await["data"]["reason"],
        "stale_revision"
    );
    client.result("history.undo", None).await;
    assert_eq!(
        super::project(&mut client).await["project"],
        before["project"]
    );
    assert_eq!(
        client
            .result("audio.fx_meters", Some(json!({"timeline":"fx"})))
            .await["playing"],
        false
    );
    client
        .result(
            "transport.load",
            Some(json!({"monitor":"timeline","timeline":"fx"})),
        )
        .await;
    client
        .result(
            "transport.play",
            Some(json!({"monitor":"timeline","speed":1})),
        )
        .await;
    let start = Instant::now();
    let meters = loop {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let r = client
            .result("audio.fx_meters", Some(json!({"timeline":"fx"})))
            .await;
        if r["playing"] == true
            && r["readings"].as_array().is_some_and(|a| {
                a.iter().any(|r| {
                    r["insert"] == "nr"
                        && r["reading"]["input_db"].as_f64().unwrap_or(-200.0) > -80.0
                })
            })
        {
            break r;
        }
        assert!(start.elapsed() < Duration::from_secs(30), "{r}");
    };
    let readings = meters["readings"].as_array().unwrap();
    for id in ["nr", "width", "limiter"] {
        assert!(readings.iter().any(|r| r["insert"] == id));
    }
    println!(
        "live graph {}: {} typed FX readers",
        meters["generation"],
        readings.len()
    );
    client
        .result("transport.pause", Some(json!({"monitor":"timeline"})))
        .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        client
            .result("audio.fx_meters", Some(json!({"timeline":"fx"})))
            .await["readings"],
        json!([])
    );
    println!(
        "preset CRUD did not alter history; Learn wrote 1025-bin manual profile in one undo step; stop cleared readers"
    );
    drop(client);
    drop(engine.child.stdin.take());
    assert_eq!(engine.ended().await, 0);
}
