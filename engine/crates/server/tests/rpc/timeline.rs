//! Timeline playback through the real engine, with independent pixel and timing expectations.

use std::sync::Arc;
use std::time::Duration;

use columbina_compose::{Compositor, Scene, SceneLayer, Source, Target, Transform};
use columbina_media::{Hardware, VideoDecoder};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::{
    clip, engine, project, project_dir, proxy_engine, support, transport_engine_with, Client,
};

async fn add_timeline(client: &mut Client, clips: Vec<Value>, transitions: Vec<Value>) -> Value {
    let names = client
        .apply(json!([{"op":"add_timeline", "timeline":{
            "id":"$timeline", "name":"Cut", "width":64, "height":36,
            "rate":{"num":25,"den":1}, "sample_rate":48000,
            "tracks":[{"id":"$v", "kind":"video", "name":"Picture", "enabled":true,
                "locked":false, "clips":clips, "transitions":[]}]
        }}]))
        .await["names"]
        .clone();
    for mut transition in transitions {
        for side in ["left", "right"] {
            transition[side] = names[transition[side].as_str().unwrap()].clone();
        }
        client
            .apply(
                json!([{"op":"add_transition", "timeline":names["$timeline"],
            "track":names["$v"], "transition":transition}]),
            )
            .await;
    }
    names
}

async fn load(client: &mut Client, names: &Value) -> Value {
    client
        .result(
            "transport.load",
            Some(json!({
                "monitor":"timeline", "timeline":names["$timeline"]
            })),
        )
        .await
}

async fn action(client: &mut Client, method: &str, mut params: Value) -> Value {
    params["monitor"] = json!("timeline");
    client.result(method, Some(params)).await
}

async fn sought(client: &mut Client, frame: i64) -> Value {
    client.take_notifications();
    action(client, "transport.seek", json!({"frame":frame})).await;
    client
        .until_state("exact timeline frame", |state| {
            state["monitor"] == "timeline" && state["shown"] == frame && state["exact"] == true
        })
        .await;
    action(client, "transport.frame_digest", json!({})).await
}

#[tokio::test]
async fn seeking_during_timeline_playback_keeps_the_speed_and_direction() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let engine = engine(dir.path(), &["--no-proxies", "--sound", "null"]).await;
    let mut client = engine.client("agent").await;
    let mut color = clip("$color", "unused", 0, 200);
    color["source"] = json!({"kind":"color", "color":"#ff0000ff"});
    let names = add_timeline(&mut client, vec![color], vec![]).await;
    load(&mut client, &names).await;
    action(&mut client, "transport.seek", json!({"frame":100})).await;
    action(&mut client, "transport.play", json!({"speed":-2})).await;
    client
        .until_state("reverse playback", |s| {
            s["monitor"] == "timeline" && s["playing"] == true
        })
        .await;
    client.take_notifications();
    action(&mut client, "transport.seek", json!({"frame":60})).await;
    let (state, _) = client
        .until_state("seek during playback", |s| {
            s["monitor"] == "timeline" && s["shown"] == 60
        })
        .await;
    assert_eq!(state["playing"], true, "{state}");
    assert_eq!(state["speed"], -2);
    client
        .until_state("on backward from seek", |s| {
            s["monitor"] == "timeline" && s["playing"] == true && s["frame"].as_i64().unwrap() < 58
        })
        .await;
    action(
        &mut client,
        "transport.seek",
        json!({"frame":37,"scrubbing":true}),
    )
    .await;
    let (state, _) = client
        .until_state("scrubbing pauses", |s| {
            s["monitor"] == "timeline" && s["frame"] == 37
        })
        .await;
    assert_eq!(state["playing"], false);
}

#[tokio::test]
async fn timeline_cuts_dissolve_and_reverse_speed_show_the_expected_frames() {
    let Some(gpu) = columbina_compose::gpu_for_tests() else {
        return;
    };
    let dir = project_dir();
    let Some((_engine, mut client)) = transport_engine_with(
        dir.path(),
        &[support::Sample::Fhd],
        &["--sound", "null", "--test-hooks"],
    )
    .await
    else {
        return;
    };
    let got = project(&mut client).await;
    let media = &got["project"]["media"][0];
    let scale = media["facts"]["duration"]["scale"].as_i64().unwrap();
    let mut first = clip("$one", "m1", 0, 30);
    first["source"]["in"] = json!({"ticks":10*scale/25,"scale":scale});
    let mut second = clip("$two", "m1", 30, 30);
    second["source"]["in"] = json!({"ticks":60*scale/25,"scale":scale});
    let mut third = clip("$three", "m1", 60, 25);
    third["source"]["in"] = json!({"ticks":0,"scale":scale});
    let names = add_timeline(&mut client, vec![first,second,third], vec![json!({
        "id":"$dissolve", "kind":"dissolve", "left":"$one", "right":"$two", "length":10, "align":"center"
    })]).await;
    load(&mut client, &names).await;
    let path = std::path::Path::new(media["path"].as_str().unwrap());
    let mut decoder = VideoDecoder::open(path, Hardware::Auto).unwrap();
    let mut compose = Compositor::new(Arc::clone(&gpu));
    let target = Target::preview(&gpu, 64, 36).unwrap();
    // Independent source frame numbers, including both handles and the cut itself.
    for (frame, left, right) in [
        (24, 34, None),
        (25, 35, Some((55, 1.0 / 11.0))),
        (29, 39, Some((59, 5.0 / 11.0))),
        (30, 40, Some((60, 6.0 / 11.0))),
        (31, 41, Some((61, 7.0 / 11.0))),
        (34, 44, Some((64, 10.0 / 11.0))),
        (35, 65, None),
        (59, 89, None),
        (60, 0, None),
        (61, 1, None),
    ] {
        let mut scene = Scene {
            width: 64,
            height: 36,
            ..Scene::default()
        };
        scene.layers.push(SceneLayer {
            source: Source::Picture(Arc::new(decoder.frame_at(left).unwrap()), 0),
            transform: Transform::NONE,
            opacity: 1.0,
        });
        if let Some((right, opacity)) = right {
            scene.layers.push(SceneLayer {
                source: Source::Picture(Arc::new(decoder.frame_at(right).unwrap()), 0),
                transform: Transform::NONE,
                opacity,
            });
        }
        compose.render(&scene, &target).unwrap();
        let expected = format!("{:x}", Sha256::digest(target.read_rgba().unwrap()));
        let drawn = sought(&mut client, frame).await;
        assert_eq!(drawn["frame"], frame);
        assert_eq!(drawn["hash"], expected, "timeline frame {frame}: {drawn}");
    }
    client.apply(json!([{"op":"set_clip","timeline":names["$timeline"],"id":names["$three"],"speed":{"num":2,"den":1},"reverse":true}])).await;
    // Reversal starts at the final timeline-frame position: (length - 1) * speed.
    for (frame, source) in [(60, 48), (61, 46), (70, 28), (84, 0)] {
        let scene = Scene {
            width: 64,
            height: 36,
            layers: vec![SceneLayer {
                source: Source::Picture(Arc::new(decoder.frame_at(source).unwrap()), 0),
                transform: Transform::NONE,
                opacity: 1.0,
            }],
            ..Scene::default()
        };
        compose.render(&scene, &target).unwrap();
        let expected = format!("{:x}", Sha256::digest(target.read_rgba().unwrap()));
        assert_eq!(
            sought(&mut client, frame).await["hash"],
            expected,
            "reverse speed at {frame}"
        );
    }
}

#[tokio::test]
async fn timeline_plays_three_seconds_without_late_reads_and_switches_monitors() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let Some((_engine, mut client)) =
        transport_engine_with(dir.path(), &[support::Sample::Fhd], &["--sound", "null"]).await
    else {
        return;
    };
    let got = project(&mut client).await;
    let mut video = clip("$one", "m1", 0, 100);
    video["source"]["in"]["scale"] =
        got["project"]["media"][0]["facts"]["duration"]["scale"].clone();
    let names = add_timeline(&mut client, vec![video], vec![]).await;
    load(&mut client, &names).await;
    client
        .source_result("transport.load", Some(json!({"media":"m1"})))
        .await;
    action(&mut client, "transport.play", json!({"speed":1})).await;
    client
        .until_state("timeline playing", |s| {
            s["monitor"] == "timeline" && s["playing"] == true
        })
        .await;
    let states = client.states_for(Duration::from_secs(3)).await;
    let timeline: Vec<_> = states
        .iter()
        .filter(|(s, _)| s["monitor"] == "timeline")
        .collect();
    let (last, _) = timeline.last().unwrap();
    assert!(last["frame"].as_i64().unwrap() >= 70, "{timeline:?}");
    assert_eq!(last["playing"], true, "{last}");
    assert_eq!(last["late"], 0, "{timeline:?}");
    client.take_notifications();
    client
        .source_result("transport.play", Some(json!({"speed":1})))
        .await;
    // Acknowledgement of the old monitor's pause precedes the new monitor's play.
    let (paused, _) = client
        .until_state("timeline paused first", |s| {
            s["monitor"] == "timeline" && s["playing"] == false
        })
        .await;
    client
        .until_state("source playing next", |s| {
            s["monitor"] == "source" && s["playing"] == true
        })
        .await;
    assert_eq!(
        action(&mut client, "transport.state", json!({})).await["frame"],
        paused["frame"]
    );
    client.take_notifications();
    action(&mut client, "transport.play", json!({"speed":-1})).await;
    client
        .until_state("source paused first", |s| {
            s["monitor"] == "source" && s["playing"] == false
        })
        .await;
    client
        .until_state("timeline playing next", |s| {
            s["monitor"] == "timeline" && s["playing"] == true
        })
        .await;
    let (ended, _) = client
        .until_state("reverse reaches start", |s| {
            s["monitor"] == "timeline" && s["playing"] == false && s["frame"] == 0
        })
        .await;
    assert_eq!(ended["shown"], 0);
    assert_eq!(ended["exact"], true);
}

#[tokio::test]
async fn timeline_undo_while_playing_changes_the_picture_without_stopping() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let engine = engine(
        dir.path(),
        &["--no-proxies", "--sound", "null", "--test-hooks"],
    )
    .await;
    let mut client = engine.client("agent").await;
    let mut color = clip("$color", "unused", 0, 200);
    color["source"] = json!({"kind":"color","color":"#ff0000ff"});
    let names = add_timeline(&mut client, vec![color], vec![]).await;
    client
        .apply(
            json!([{"op":"set_clip","timeline":names["$timeline"],"id":names["$color"],
        "source":{"kind":"color","color":"#0000ffff"}}]),
        )
        .await;
    load(&mut client, &names).await;
    let blue = action(&mut client, "transport.frame_digest", json!({})).await;
    assert_eq!(blue["pixels"][0], json!([0, 0, 255, 255]));
    action(&mut client, "transport.play", json!({"speed":1})).await;
    client
        .until_state("playing before undo", |s| {
            s["monitor"] == "timeline" && s["playing"] == true
        })
        .await;
    client.take_notifications();
    client.result("history.undo", None).await;
    let current = action(&mut client, "transport.frame_digest", json!({})).await;
    let frame = current["frame"].as_i64().unwrap();
    let (after, _) = client
        .until_state("next picture after undo", |s| {
            s["monitor"] == "timeline" && s["shown"].as_i64() > Some(frame)
        })
        .await;
    assert_eq!(after["playing"], true);
    let red = action(&mut client, "transport.frame_digest", json!({})).await;
    assert_eq!(red["pixels"][0], json!([255, 0, 0, 255]), "{red}");
    assert_ne!(red["hash"], blue["hash"]);
}

#[tokio::test]
async fn timeline_transport_requires_a_monitor_and_hides_test_hooks_by_default() {
    let dir = project_dir();
    let engine = engine(dir.path(), &["--no-proxies", "--sound", "null"]).await;
    let mut client = engine.client("agent").await;
    for method in [
        "transport.load",
        "transport.play",
        "transport.pause",
        "transport.seek",
        "transport.step",
        "transport.state",
        "transport.proxies",
    ] {
        for params in [
            None,
            Some(json!({})),
            Some(json!({"monitor":null})),
            Some(json!({"monitor":"unknown"})),
        ] {
            assert_eq!(
                client.error(method, params).await["code"],
                -32602,
                "{method}"
            );
        }
    }
    assert_eq!(
        client
            .error(
                "transport.frame_digest",
                Some(json!({"monitor":"timeline"}))
            )
            .await["code"],
        -32601
    );
    for params in [
        json!({"monitor":"timeline","media":"m1"}),
        json!({"monitor":"source","timeline":"t1"}),
        json!({"monitor":"timeline","timeline":"t1","extra":true}),
    ] {
        assert_eq!(
            client.error("transport.load", Some(params)).await["code"],
            -32602
        );
    }
    for speed in [0, 9, -9, i32::MIN] {
        let error = client
            .error(
                "transport.play",
                Some(json!({"monitor":"timeline","speed":speed})),
            )
            .await;
        assert_eq!(error["code"], -32602);
        assert_eq!(error["data"]["reason"], "speed");
    }
    for method in [
        "transport.play",
        "transport.pause",
        "transport.seek",
        "transport.step",
    ] {
        let params = match method {
            "transport.seek" => json!({"monitor":"timeline","frame":0}),
            "transport.step" => json!({"monitor":"timeline","frames":1}),
            _ => json!({"monitor":"timeline"}),
        };
        assert_eq!(
            client.error(method, Some(params)).await["data"]["reason"],
            "not_loaded"
        );
    }
}

#[tokio::test]
async fn timeline_scrubbing_settles_on_the_exact_frame() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let Some((_engine, mut client)) = transport_engine_with(
        dir.path(),
        &[support::Sample::Fhd],
        &["--sound", "null", "--test-hooks"],
    )
    .await
    else {
        return;
    };
    let got = project(&mut client).await;
    let mut video = clip("$one", "m1", 0, 100);
    video["source"]["in"]["scale"] =
        got["project"]["media"][0]["facts"]["duration"]["scale"].clone();
    let names = add_timeline(&mut client, vec![video], vec![]).await;
    load(&mut client, &names).await;
    let exact = sought(&mut client, 37).await;
    sought(&mut client, 0).await;
    client.take_notifications();
    action(
        &mut client,
        "transport.seek",
        json!({"frame":37,"scrubbing":true}),
    )
    .await;
    let (coarse, at) = client
        .until_state("coarse scrub", |s| {
            s["monitor"] == "timeline" && s["frame"] == 37
        })
        .await;
    assert_eq!(coarse["playing"], false);
    assert_eq!(coarse["exact"], false);
    let (fine, then) = client
        .until_state("settled scrub", |s| {
            s["monitor"] == "timeline" && s["frame"] == 37 && s["exact"] == true
        })
        .await;
    assert_eq!(fine["shown"], 37);
    assert!(then.duration_since(at) < Duration::from_secs(2));
    assert_eq!(
        action(&mut client, "transport.frame_digest", json!({})).await["hash"],
        exact["hash"]
    );
}

#[tokio::test]
async fn timeline_rebuilds_a_broken_shared_proxy_instead_of_reusing_its_twin() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let Some((_engine, mut client)) = proxy_engine(
        dir.path(),
        &[support::Sample::Fhd, support::Sample::Fhd],
        &["--proxy-encoder", "prores", "--test-hooks"],
    )
    .await
    else {
        return;
    };
    client.proxies_of(&["m1", "m2"]).await;
    let got = project(&mut client).await;
    let media = &got["project"]["media"];
    assert_eq!(media[0]["proxy"], media[1]["proxy"]);
    let path = std::path::Path::new(media[0]["proxy"]["path"].as_str().unwrap());
    std::fs::write(path, b"broken shared proxy").unwrap();
    let mut video = clip("$one", "m1", 0, 100);
    video["source"]["in"]["scale"] = media[0]["facts"]["duration"]["scale"].clone();
    let names = add_timeline(&mut client, vec![video], vec![]).await;
    client.take_notifications();
    let loaded = load(&mut client, &names).await;
    assert_eq!(loaded["source"], "original");
    assert_eq!(loaded["exact"], true);
    let repaired = client.proxies_of(&["m1"]).await;
    assert!(
        repaired.iter().any(|note| note["state"] == "running"),
        "a twin must not be reused: {repaired:?}"
    );
    assert_eq!(repaired.last().unwrap()["state"], "done", "{repaired:?}");
    let got = project(&mut client).await;
    let fixed = std::path::Path::new(
        got["project"]["media"][0]["proxy"]["path"]
            .as_str()
            .unwrap(),
    );
    assert!(VideoDecoder::open(fixed, Hardware::Off)
        .unwrap()
        .frame_at(99)
        .is_ok());
    let (ready, _) = client
        .until_state("repaired proxy shown", |s| {
            s["monitor"] == "timeline" && s["source"] == "proxy" && s["exact"] == true
        })
        .await;
    assert_eq!(ready["frame"], 0);
    // A second enabled track without a proxy is reported as mixed.
    let mut second = clip("$two", "m2", 0, 100);
    second["source"]["in"]["scale"] = media[1]["facts"]["duration"]["scale"].clone();
    client.take_notifications();
    client.apply(json!([
        {"op":"set_proxy","media":"m2","proxy":null},
        {"op":"add_track","timeline":names["$timeline"],"track":{
            "id":"$second","kind":"video","name":"Upper","enabled":true,"locked":false,"clips":[second],"transitions":[]},"index":null}
    ])).await;
    client
        .until_state("mixed proxy and original", |s| {
            s["monitor"] == "timeline" && s["source"] == "mixed"
        })
        .await;
    action(&mut client, "transport.proxies", json!({"on":false})).await;
    client
        .until_state("originals after switch", |s| {
            s["monitor"] == "timeline" && s["source"] == "original" && s["proxies"] == false
        })
        .await;
}
