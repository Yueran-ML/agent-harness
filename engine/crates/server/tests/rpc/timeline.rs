//! Timeline playback through the real engine, with independent pixel and timing expectations.

use std::sync::Arc;
use std::time::Duration;

use columbina_compose::{
    Compositor, LayerColor, Scene, SceneLayer, Source, Target, Transform, YuvCoding,
};
use columbina_media::{Converter, Hardware, VideoDecoder};
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

/// A frame of a decoder as a layer of a scene, as the engine composes it: a picture of the
/// sampling of chroma of its file, decoded by the matrix and range that it says.
fn decoded_layer(decoder: &mut VideoDecoder, frame: i64, opacity: f32) -> SceneLayer {
    let picture = Converter::new()
        .native(&decoder.decoded_at(frame).unwrap())
        .unwrap();
    SceneLayer {
        color: LayerColor {
            coding: YuvCoding::of_native(&picture),
            ..LayerColor::default()
        },
        source: Source::Picture(Arc::new(picture), 0),
        transform: Transform::NONE,
        opacity,
    }
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
        scene.layers.push(decoded_layer(&mut decoder, left, 1.0));
        if let Some((right, opacity)) = right {
            scene
                .layers
                .push(decoded_layer(&mut decoder, right, opacity));
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
            layers: vec![decoded_layer(&mut decoder, source, 1.0)],
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

async fn load_audio_timeline(client: &mut Client) {
    let got = project(client).await;
    let mut color = clip("$color", "unused", 0, 100);
    color["source"] = json!({"kind":"color","color":"#ff0000ff"});
    let names = add_timeline(client, vec![color], vec![]).await;
    let mut sound = clip("$sound", "m1", 0, 100);
    sound["source"]["in"]["scale"] =
        got["project"]["media"][0]["facts"]["duration"]["scale"].clone();
    client
        .apply(
            json!([{"op":"add_track","timeline":names["$timeline"],"track":{
        "id":"$audio","kind":"audio","name":"Dialogue","enabled":true,"locked":false,
        "clips":[sound],"transitions":[]},"index":null}]),
        )
        .await;
    load(client, &names).await;
}

#[tokio::test]
async fn timeline_picture_follows_the_mixed_audio_clock_and_pauses_it() {
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
    load_audio_timeline(&mut client).await;
    action(&mut client, "transport.play", json!({"speed":1})).await;
    let (playing, _) = client
        .until_state("timeline playing", |s| {
            s["monitor"] == "timeline" && s["playing"] == true
        })
        .await;
    assert_eq!(playing["sound"], "on", "{playing}");
    let mut before = 0;
    for _ in 0..5 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let digest = action(&mut client, "transport.frame_digest", json!({})).await;
        let samples = digest["audio_samples"].as_i64().unwrap();
        assert!(samples > before, "sound clock did not advance: {digest}");
        before = samples;
        assert!(
            digest["audio_frame"]
                .as_i64()
                .unwrap()
                .abs_diff(digest["frame"].as_i64().unwrap())
                <= 1,
            "sound/picture drift: {digest}"
        );
        // Where the mixer is in the timeline, which the clock does not tell: at 25 frames a
        // second and 48000 samples, 1920 samples a frame. Sound that started a frame or more
        // away from the picture is caught here.
        let mixed = digest["audio_sample"].as_i64().unwrap();
        let shown = digest["frame"].as_i64().unwrap() * 1920;
        assert!(
            mixed.abs_diff(shown) <= 1920,
            "the sound is {} samples from the picture: {digest}",
            mixed - shown
        );
    }
    client.take_notifications();
    action(&mut client, "transport.pause", json!({})).await;
    let (paused, _) = client
        .until_state("timeline paused", |s| {
            s["monitor"] == "timeline" && s["playing"] == false
        })
        .await;
    assert_eq!(paused["sound"], "off");
    assert_eq!(
        action(&mut client, "transport.frame_digest", json!({}))
            .await
            .get("audio_samples"),
        None
    );
}

#[tokio::test]
async fn timeline_sound_plays_on_through_changes_that_it_does_not_hear() {
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
    let mut color = clip("$color", "unused", 0, 200);
    color["source"] = json!({"kind":"color","color":"#ff0000ff"});
    let names = add_timeline(&mut client, vec![color], vec![]).await;
    let mut sound = clip("$sound", "m1", 0, 100);
    sound["source"]["in"]["scale"] =
        got["project"]["media"][0]["facts"]["duration"]["scale"].clone();
    let sound = client
        .apply(
            json!([{"op":"add_track","timeline":names["$timeline"],"track":{
        "id":"$audio","kind":"audio","name":"Dialogue","enabled":true,"locked":false,
        "clips":[sound],"transitions":[]},"index":null}]),
        )
        .await["names"]["$sound"]
        .clone();
    load(&mut client, &names).await;
    action(&mut client, "transport.play", json!({"speed":1})).await;
    client
        .until_state("timeline audio started", |s| {
            s["monitor"] == "timeline" && s["sound"] == "on"
        })
        .await;
    let heard = async |client: &mut Client| {
        let digest = action(client, "transport.frame_digest", json!({})).await;
        (digest["audio_samples"].as_i64().unwrap(), digest)
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (before, _) = heard(&mut client).await;
    // A bin, and the colour of the picture: the sound plays on, and the new colour is shown.
    client
        .apply(
            json!([{"op":"add_bin","bin":{"id":"$bin","name":"Kept","parent":null}},
            {"op":"set_clip","timeline":names["$timeline"],"id":names["$color"],
            "source":{"kind":"color","color":"#00ff00ff"}}]),
        )
        .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let (after, digest) = heard(&mut client).await;
    assert!(
        after > before,
        "the sound started again: {before} then {after}"
    );
    assert_eq!(digest["pixels"][0], json!([0, 255, 0, 255]), "{digest}");
    // A change of what is heard does not start the sound again either (T5): a clip's gain is a
    // setting of the mix as it plays, and a fade another mix swapped in; the sound and the
    // picture keep together.
    client
        .apply(
            json!([{"op":"set_clip","timeline":names["$timeline"],"id":sound,
            "gain_db":-6.0}]),
        )
        .await;
    client
        .apply(
            json!([{"op":"set_clip","timeline":names["$timeline"],"id":sound,
            "fade_out":10}]),
        )
        .await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    let (again, digest) = heard(&mut client).await;
    assert!(
        again > after,
        "the sound started again: {after} then {again}"
    );
    assert_eq!(digest["audio_swaps"], 1, "{digest}");
    assert_eq!(digest["audio_short_runs"], 0, "{digest}");
    assert!(
        digest["audio_sample"]
            .as_i64()
            .unwrap()
            .abs_diff(digest["frame"].as_i64().unwrap() * 1920)
            <= 1920,
        "{digest}"
    );
}

/// The frames of a capture (`--sound capture=<path>`): left and right, interleaved.
fn captured(path: &std::path::Path) -> Vec<f32> {
    std::fs::read(path)
        .unwrap()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|word| f32::from_le_bytes(*word))
        .collect()
}

/// A timeline of a track of sound of two clips of the 4 s of media `m1` one after the other, and
/// a red picture as long, loaded; its names.
async fn eight_seconds_of_sound(client: &mut Client) -> Value {
    let got = project(client).await;
    let mut color = clip("$color", "unused", 0, 200);
    color["source"] = json!({"kind":"color","color":"#ff0000ff"});
    let names = add_timeline(client, vec![color], vec![]).await;
    let scale = got["project"]["media"][0]["facts"]["duration"]["scale"].clone();
    let mut first = clip("$first", "m1", 0, 100);
    first["source"]["in"]["scale"] = scale.clone();
    let mut second = clip("$second", "m1", 100, 100);
    second["source"]["in"]["scale"] = scale;
    let added = client
        .apply(
            json!([{"op":"add_track","timeline":names["$timeline"],"track":{
        "id":"$audio","kind":"audio","name":"Dialogue","enabled":true,"locked":false,
        "clips":[first, second],"transitions":[]},"index":null}]),
        )
        .await["names"]
        .clone();
    let mut names = names;
    for name in ["$audio", "$first", "$second"] {
        names[name] = added[name].clone();
    }
    load(client, &names).await;
    names
}

#[tokio::test]
#[allow(clippy::cast_precision_loss, reason = "milliseconds of a test")]
async fn timeline_sound_muted_while_it_plays_is_silent_within_60_ms() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let capture = dir.path().join("heard.f32");
    let sound = format!("capture={}", capture.display());
    let Some((engine, mut client)) = transport_engine_with(
        dir.path(),
        &[support::Sample::Fhd],
        &["--sound", &sound, "--test-hooks"],
    )
    .await
    else {
        return;
    };
    let names = eight_seconds_of_sound(&mut client).await;
    action(&mut client, "transport.play", json!({"speed":1})).await;
    client
        .until_state("timeline audio started", |s| {
            s["monitor"] == "timeline" && s["sound"] == "on"
        })
        .await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    let asked = std::time::Instant::now();
    let digest = action(&mut client, "transport.frame_digest", json!({})).await;
    let heard = digest["audio_samples"].as_i64().unwrap();
    let applied = std::time::Instant::now();
    client
        .apply(
            json!([{"op":"set_track","timeline":names["$timeline"],"id":names["$audio"],
            "mute":true}]),
        )
        .await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    let digest = action(&mut client, "transport.frame_digest", json!({})).await;
    assert_eq!(digest["audio_swaps"], 0, "a mute is a setting: {digest}");
    assert_eq!(digest["audio_short_runs"], 0, "{digest}");
    assert_eq!(digest["audio_underruns"], 0, "{digest}");
    action(&mut client, "transport.pause", json!({})).await;
    drop(client);
    drop(engine);
    let left: Vec<f32> = captured(&capture).into_iter().step_by(2).collect();
    // What was given to the device when the change was sent: what was heard by the digest, the
    // 20 ms that the device is heard after, and the time from the digest to the change.
    let between = applied.duration_since(asked).as_secs_f64();
    #[allow(clippy::cast_possible_truncation)]
    let sent = usize::try_from(heard + 960 + (between * 48_000.0) as i64).unwrap();
    let loud = left[sent - 4800..sent]
        .iter()
        .fold(0.0_f32, |most, x| most.max(x.abs()));
    assert!(loud > 0.05, "{loud}");
    let silent = (sent..left.len() - 480)
        .find(|&n| left[n..n + 480].iter().all(|x| x.abs() < 1e-3))
        .unwrap();
    let after = (silent - sent) as f64 / 48.0;
    println!("muted while it plays: below -60 dBFS {after:.1} ms after the change was sent");
    assert!(after <= 60.0, "{after} ms");
}

#[tokio::test]
#[allow(clippy::cast_precision_loss, reason = "levels of a test")]
async fn timeline_sound_plays_through_two_hundred_changes_without_a_click() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let capture = dir.path().join("heard.f32");
    let sound = format!("capture={}", capture.display());
    let Some((engine, mut client)) = transport_engine_with(
        dir.path(),
        &[support::Sample::Fhd],
        &["--sound", &sound, "--test-hooks"],
    )
    .await
    else {
        return;
    };
    let names = eight_seconds_of_sound(&mut client).await;
    action(&mut client, "transport.play", json!({"speed":1})).await;
    client
        .until_state("timeline audio started", |s| {
            s["monitor"] == "timeline" && s["sound"] == "on"
        })
        .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let first = action(&mut client, "transport.frame_digest", json!({})).await;
    let before = first["audio_samples"].as_i64().unwrap();
    // Every 20 ms, the gain or the pan of a clip (a setting, as its volume line or pan is
    // dragged) or its fade out (another mix swapped in).
    let mut next = std::time::Instant::now();
    for change in 0..200 {
        let op = match change % 4 {
            0 => json!({"op":"set_clip","timeline":names["$timeline"],"id":names["$first"],
                "gain_db":-3.0 + f64::from(change % 7) * 0.25}),
            1 => json!({"op":"set_clip","timeline":names["$timeline"],"id":names["$second"],
                "pan": f64::from(change % 5) * 0.05}),
            2 => json!({"op":"set_clip","timeline":names["$timeline"],"id":names["$first"],
                "fade_out": change % 3}),
            _ => json!({"op":"set_clip","timeline":names["$timeline"],"id":names["$second"],
                "gain_db":-1.5}),
        };
        client.apply(json!([op])).await;
        next += Duration::from_millis(20);
        tokio::time::sleep_until(next.into()).await;
    }
    let digest = action(&mut client, "transport.frame_digest", json!({})).await;
    let after = digest["audio_samples"].as_i64().unwrap();
    assert!(
        after > before + 48_000 * 3,
        "the sound started again: {digest}"
    );
    assert_eq!(digest["audio_short_runs"], 0, "{digest}");
    assert_eq!(digest["audio_underruns"], 0, "{digest}");
    assert!(digest["audio_swaps"].as_u64().unwrap() > 0, "{digest}");
    action(&mut client, "transport.pause", json!({})).await;
    drop(client);
    drop(engine);
    let left: Vec<f32> = captured(&capture).into_iter().step_by(2).collect();
    let step = |from: usize, to: usize| {
        left[from..to]
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .fold(0.0_f32, f32::max)
    };
    // The tone of 440 Hz as it plays before the changes, and through them, but where the second
    // clip starts the media again (at 4 s), which is a step of the tone itself.
    let start = usize::try_from(before).unwrap();
    let end = usize::try_from(after).unwrap().min(left.len());
    let normal = step(4800, start);
    let changed = step(start, 192_000 - 480).max(step(192_000 + 480, end));
    println!("the largest step: {changed} through the changes, {normal} before");
    assert!(changed <= normal * 1.1, "{changed} > {normal} · 1.1");
}

#[tokio::test]
async fn timeline_sound_survives_seek_and_is_handed_over_before_source_playback() {
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
    load_audio_timeline(&mut client).await;
    action(&mut client, "transport.play", json!({"speed":1})).await;
    client
        .until_state("timeline audio started", |s| {
            s["monitor"] == "timeline" && s["sound"] == "on"
        })
        .await;
    client.take_notifications();
    action(&mut client, "transport.seek", json!({"frame":30})).await;
    let (sought, _) = client
        .until_state("timeline sought with sound", |s| {
            s["monitor"] == "timeline" && s["frame"] == 30
        })
        .await;
    assert_eq!(sought["sound"], "on");
    assert_eq!(sought["playing"], true);
    client
        .until_state("sound continued after seek", |s| {
            s["monitor"] == "timeline" && s["frame"].as_i64() > Some(33)
        })
        .await;
    client
        .source_result("transport.load", Some(json!({"media":"m1"})))
        .await;
    client.take_notifications();
    client.source_result("transport.play", None).await;
    let (paused, _) = client
        .until_state("timeline stopped before source", |s| {
            s["monitor"] == "timeline" && s["playing"] == false
        })
        .await;
    assert_eq!(paused["sound"], "off");
    client
        .until_state("source owns sound", |s| {
            s["monitor"] == "source" && s["sound"] == "on"
        })
        .await;
    action(&mut client, "transport.play", json!({"speed":2})).await;
    let (source, _) = client
        .until_state("source stopped before timeline", |s| {
            s["monitor"] == "source" && s["playing"] == false
        })
        .await;
    assert_eq!(source["sound"], "off");
    let (fast, _) = client
        .until_state("timeline shuttles silently", |s| {
            s["monitor"] == "timeline" && s["playing"] == true && s["speed"] == 2
        })
        .await;
    assert_eq!(fast["sound"], "off");
}

#[tokio::test]
async fn timeline_falls_back_to_system_time_after_audio_device_failure() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let Some((_engine, mut client)) = transport_engine_with(
        dir.path(),
        &[support::Sample::Fhd],
        &["--sound", "fail-after=250"],
    )
    .await
    else {
        return;
    };
    load_audio_timeline(&mut client).await;
    action(&mut client, "transport.play", json!({"speed":1})).await;
    client
        .until_state("timeline sound", |s| {
            s["monitor"] == "timeline" && s["sound"] == "on"
        })
        .await;
    let (lost, _) = client
        .until_state("failed timeline device", |s| {
            s["monitor"] == "timeline" && s["sound"] == "unavailable"
        })
        .await;
    assert_eq!(lost["playing"], true);
    let at = lost["frame"].as_i64().unwrap();
    assert!((0..=12).contains(&at), "{lost}");
    let (continued, _) = client
        .until_state("system clock advances", |s| {
            s["monitor"] == "timeline" && s["frame"].as_i64() > Some(at + 4)
        })
        .await;
    assert_eq!(continued["playing"], true);
    assert_eq!(continued["sound"], "unavailable");
    action(&mut client, "transport.pause", json!({})).await;
    action(&mut client, "transport.play", json!({"speed":1})).await;
    client
        .until_state("next play retries the device", |s| {
            s["monitor"] == "timeline" && s["sound"] == "on"
        })
        .await;
}

/// The first frame of a sample, in the source monitor and alone on a timeline of its size: the
/// same picture, decoded alike (its own range and matrix, its chroma of its own sampling), so
/// the same pixels, as each draws them for a digest.
async fn both_monitors_show(sample: support::Sample) {
    let dir = project_dir();
    let Some((_engine, mut client)) =
        transport_engine_with(dir.path(), &[sample], &["--sound", "null", "--test-hooks"]).await
    else {
        return;
    };
    let got = project(&mut client).await;
    let media = &got["project"]["media"][0];
    let video = &media["facts"]["video"];
    let mut alone = clip("$one", "m1", 0, 5);
    alone["source"]["in"] = json!({"ticks":0, "scale":media["facts"]["duration"]["scale"]});
    let names = client
        .apply(json!([{"op":"add_timeline", "timeline":{
            "id":"$timeline", "name":"Alone", "width":video["width"], "height":video["height"],
            "rate":{"num":25,"den":1}, "sample_rate":48000,
            "tracks":[{"id":"$v", "kind":"video", "name":"Picture", "enabled":true,
                "locked":false, "clips":[alone], "transitions":[]}]
        }}]))
        .await["names"]
        .clone();
    load(&mut client, &names).await;
    let timeline = sought(&mut client, 0).await;
    let loaded = client
        .source_result("transport.load", Some(json!({ "media": "m1" })))
        .await;
    assert_eq!(
        (&loaded["shown"], &loaded["exact"]),
        (&json!(0), &json!(true))
    );
    let source = client.source_result("transport.frame_digest", None).await;
    assert_eq!(
        (&source["width"], &source["height"], &source["frame"]),
        (&video["width"], &video["height"], &json!(0)),
        "{sample:?}: {source}"
    );
    assert_eq!(
        source["hash"], timeline["hash"],
        "{sample:?}: the source monitor {source}, the timeline {timeline}"
    );
    if sample == support::Sample::RedPng {
        // Red exactly, as the still is: drawn as R'G'B' (not of BT.601, R' 231, nor made
        // Y'CbCr, 253).
        for pixel in source["pixels"].as_array().unwrap() {
            assert_eq!(pixel.as_array().unwrap()[..3], [255, 0, 0], "{source}");
        }
    }
}

#[tokio::test]
async fn the_source_and_timeline_monitors_show_a_picture_alike() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    for sample in [
        support::Sample::Bt709Full,
        support::Sample::H264High422,
        support::Sample::Hevc444,
        support::Sample::RedPng,
    ] {
        both_monitors_show(sample).await;
    }
}

#[tokio::test]
#[ignore = "plays on the real output device of the machine (muted): run with --ignored"]
async fn timeline_sound_plays_on_the_output_device_through_a_change() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let Some((engine, mut client)) = transport_engine_with(
        dir.path(),
        &[support::Sample::Fhd],
        &["--mute", "--test-hooks"],
    )
    .await
    else {
        return;
    };
    let names = eight_seconds_of_sound(&mut client).await;
    action(&mut client, "transport.play", json!({"speed":1})).await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    let state = action(&mut client, "transport.state", json!({})).await;
    if state["sound"] != "on" {
        eprintln!("no sound: there is no device to play it on ({state})");
        return;
    }
    // The device may take a while to let its first sound be heard (opening it, waking it).
    let started = std::time::Instant::now();
    let mut before = action(&mut client, "transport.frame_digest", json!({})).await;
    while before["audio_samples"].as_i64().is_none() {
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "no sound heard: {before}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
        before = action(&mut client, "transport.frame_digest", json!({})).await;
    }
    client
        .apply(
            json!([{"op":"set_track","timeline":names["$timeline"],"id":names["$audio"],
            "mute":true}]),
        )
        .await;
    tokio::time::sleep(Duration::from_millis(700)).await;
    let after = action(&mut client, "transport.frame_digest", json!({})).await;
    eprintln!("on the device: {before} then {after}");
    assert!(
        after["audio_samples"].as_i64().unwrap() > before["audio_samples"].as_i64().unwrap(),
        "the sound started again or stopped"
    );
    assert_eq!(after["audio_short_runs"], 0, "{after}");
    assert_eq!(after["audio_underruns"], 0, "{after}");
    action(&mut client, "transport.pause", json!({})).await;
    eprintln!("{}", engine.stderr());
}

/// The lane of `path` of track `track` of timeline `timeline` in the project as `project.get`
/// gives it, if it has one.
fn lane_of(project: &Value, timeline: &Value, track: &Value, path: &str) -> Option<Value> {
    let t = project["project"]["timelines"]
        .as_array()?
        .iter()
        .find(|t| t["id"] == *timeline)?;
    let track = t["tracks"].as_array()?.iter().find(|r| r["id"] == *track)?;
    track["lanes"].get(path).cloned()
}

/// The value of a lane (`{points, kind}`, points at times of 48 kHz) at sample `at`.
#[allow(clippy::cast_precision_loss, reason = "samples of a test")]
fn lane_value(lane: &Value, at: i64) -> f64 {
    let points: Vec<(i64, f64)> = lane["points"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            assert_eq!(p["at"]["scale"], 48_000, "{p}");
            (
                p["at"]["ticks"].as_i64().unwrap(),
                p["value"].as_f64().unwrap(),
            )
        })
        .collect();
    let i = points.partition_point(|&(t, _)| t <= at);
    match (i.checked_sub(1).map(|i| points[i]), points.get(i)) {
        (Some((t, u)), Some(&(next, v))) => u + (v - u) * (at - t) as f64 / (next - t) as f64,
        (Some((_, u)), None) => u,
        (None, Some(&(_, v))) => v,
        (None, None) => f64::NAN,
    }
}

#[tokio::test]
#[allow(
    clippy::cast_precision_loss,
    clippy::too_many_lines,
    reason = "levels of a test, which follows a pass from the touch to the undo"
)]
async fn automation_recorded_as_it_plays_is_heard_at_once_and_written_as_one_step_of_undo() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let capture = dir.path().join("heard.f32");
    let sound = format!("capture={}", capture.display());
    let Some((engine, mut client)) = transport_engine_with(
        dir.path(),
        &[support::Sample::Fhd],
        &["--sound", &sound, "--test-hooks"],
    )
    .await
    else {
        return;
    };
    let names = eight_seconds_of_sound(&mut client).await;
    let (timeline, track) = (names["$timeline"].clone(), names["$audio"].clone());
    client
        .apply(json!([{"op":"set_timeline_audio","timeline":timeline,
            "patch":{"record":{"touch":"latch","on_stop":"return"}}}]))
        .await;
    let touch = |value: f64, touching: bool| {
        json!({"timeline": timeline, "track": track, "path": "fader_db",
            "value": value, "touching": touching})
    };
    // A touch while recording is off, and one of no setting, are refused.
    let off = client
        .error("automation.touch", Some(touch(-40.0, true)))
        .await;
    assert_eq!(off["data"]["reason"], "not_recording", "{off}");
    let on = client
        .result(
            "automation.record",
            Some(json!({"timeline": timeline, "on": true})),
        )
        .await;
    assert_eq!(on["on"], true, "{on}");
    assert_eq!(on["glide_ms"], 250, "{on}");
    let mut wrong = touch(0.0, true);
    wrong["path"] = json!("solo");
    let refused = client.error("automation.touch", Some(wrong)).await;
    assert_eq!(refused["data"]["reason"], "path", "{refused}");

    action(&mut client, "transport.play", json!({"speed":1})).await;
    client
        .until_state("timeline audio started", |s| {
            s["monitor"] == "timeline" && s["sound"] == "on" && s["recording"] == true
        })
        .await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    let before = action(&mut client, "transport.frame_digest", json!({})).await;
    let heard_before = before["audio_sample"].as_i64().unwrap();
    // The fader touched and held at −40 dB for a second, every 20 ms, then let go (Latch: it
    // holds to the stop).
    let mut next = std::time::Instant::now();
    for _ in 0..50 {
        client
            .result("automation.touch", Some(touch(-40.0, true)))
            .await;
        next += Duration::from_millis(20);
        tokio::time::sleep_until(next.into()).await;
    }
    client
        .result("automation.touch", Some(touch(-40.0, false)))
        .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let digest = action(&mut client, "transport.frame_digest", json!({})).await;
    let stopped_near = digest["audio_sample"].as_i64().unwrap();
    assert_eq!(digest["audio_short_runs"], 0, "{digest}");
    assert_eq!(digest["audio_underruns"], 0, "{digest}");
    assert_eq!(
        digest["audio_swaps"], 0,
        "held live, no graph swapped: {digest}"
    );
    let revision = project(&mut client).await["revision"].as_u64().unwrap();
    action(&mut client, "transport.pause", json!({})).await;

    // Written at the stop, as one transaction.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let (lane, written) = loop {
        let got = project(&mut client).await;
        if let Some(lane) = lane_of(&got, &timeline, &track, "fader_db") {
            break (lane, got["revision"].as_u64().unwrap());
        }
        assert!(std::time::Instant::now() < deadline, "no lane written");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(written, revision + 1, "one transaction");
    let first = lane["points"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["value"] == -40.0)
        .unwrap()["at"]["ticks"]
        .as_i64()
        .unwrap();
    assert!(first >= heard_before, "{first} before {heard_before}");
    assert!(
        first < heard_before + 48_000 / 5,
        "{first} after {heard_before}"
    );
    assert!((lane_value(&lane, first - 4_800) - 0.0).abs() < 1e-9);
    for at in [first, first + 48_000, stopped_near - 4_800] {
        assert!((lane_value(&lane, at) + 40.0).abs() < 0.11, "{at}: {lane}");
    }
    // Back to 0 dB over 250 ms after the stop.
    let end = lane["points"].as_array().unwrap().last().unwrap()["at"]["ticks"]
        .as_i64()
        .unwrap();
    assert!((lane_value(&lane, end) - 0.0).abs() < 1e-9, "{lane}");
    // One step of undo takes the pass back.
    client.result("history.undo", None).await;
    let undone = project(&mut client).await;
    assert!(lane_of(&undone, &timeline, &track, "fader_db").is_none());

    // Heard at once: the level 100 ms to 300 ms after the touch is 40 dB down.
    drop(client);
    drop(engine);
    let left: Vec<f32> = captured(&capture).into_iter().step_by(2).collect();
    let rms = |from: i64, to: i64| {
        let part = &left[usize::try_from(from).unwrap()..usize::try_from(to).unwrap()];
        (part.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>() / part.len() as f64).sqrt()
    };
    let loud = rms(first - 14_400, first - 4_800);
    let held = rms(first + 4_800, first + 14_400);
    let db = 20.0 * (held / loud).log10();
    println!("held live at -40 dB: heard {db:.1} dB from 100 ms after the touch");
    assert!(loud > 0.01, "{loud}");
    assert!((db + 40.0).abs() < 1.0, "{db} dB");
}
