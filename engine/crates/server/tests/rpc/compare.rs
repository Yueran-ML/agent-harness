//! What the timeline monitor compares its frame with, through the real engine: a wipe against a
//! still and against the clip before, graded as it is, measured by the scopes, never exported,
//! and the frame itself again once it is off; and a split screen.

use std::path::Path;
use std::time::{Duration, Instant};

use columbina_model::{Node, NodeKind, Param};
use serde_json::{json, Value};
use tokio::io::AsyncBufReadExt;

use super::{clip, engine, project_dir, Client};

/// A clip of `colour` from `start`, `length` frames long.
fn colour(id: &str, colour: &str, start: i64, length: i64) -> Value {
    let mut made = clip(id, "unused", start, length);
    made["source"] = json!({"kind": "color", "color": colour});
    made
}

/// A timeline of 64 × 36 at 25 frames a second, of `clips` on one track, loaded in the timeline
/// monitor; its names.
async fn timeline(client: &mut Client, clips: Vec<Value>) -> Value {
    let names = client
        .apply(json!([{"op": "add_timeline", "timeline": {
            "id": "$timeline", "name": "Compared", "width": 64, "height": 36,
            "rate": {"num": 25, "den": 1}, "sample_rate": 48000,
            "tracks": [{"id": "$v", "kind": "video", "name": "V1", "enabled": true,
                "locked": false, "clips": clips, "transitions": []}],
        }}]))
        .await["names"]
        .clone();
    client
        .result(
            "transport.load",
            Some(json!({"monitor": "timeline", "timeline": names["$timeline"]})),
        )
        .await;
    client.take_notifications();
    names
}

async fn digest(client: &mut Client) -> Value {
    client
        .result(
            "transport.frame_digest",
            Some(json!({"monitor": "timeline"})),
        )
        .await
}

/// A still of 64 × 36 of one colour, 8 bits, as a PNG.
fn still(path: &Path, rgb: [u8; 3]) {
    let file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let mut encoder = png::Encoder::new(file, 64, 36);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&rgb.repeat(64 * 36)).unwrap();
}

/// The result of a method that may be long in coming the first time it is called: the shaders
/// of the scopes are compiled then, which FXC takes most of a minute over in a build for tests.
async fn slow_result(client: &mut Client, method: &str, params: Value, within: Duration) -> Value {
    let id = client.request(method, Some(params)).await;
    let deadline = Instant::now() + within;
    loop {
        let mut line = String::new();
        tokio::time::timeout_at(deadline.into(), client.reader.read_line(&mut line))
            .await
            .expect("an answer in time")
            .unwrap();
        let message: Value = serde_json::from_str(&line).unwrap();
        if message.get("id").is_some() {
            assert_eq!(message["id"], id, "{message}");
            assert!(message.get("error").is_none(), "{message}");
            return message["result"].clone();
        }
        client.notifications.push(message);
    }
}

fn grey(pixel: &Value) -> bool {
    let [r, g, b] = [0, 1, 2].map(|i| pixel[i].as_i64().unwrap());
    (r - g).abs() <= 1 && (g - b).abs() <= 1 && r > 20
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one wipe against a still, from on to off, and a split screen"
)]
async fn a_wipe_against_a_still_is_shown_and_measured_and_off_gives_the_frame_back() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let output = project_dir();
    let engine = engine(
        dir.path(),
        &["--no-proxies", "--sound", "null", "--test-hooks"],
    )
    .await;
    let mut client = engine.client("desktop").await;
    let names = timeline(&mut client, vec![colour("$red", "#ff0000ff", 0, 10)]).await;
    let before = digest(&mut client).await;
    assert_eq!(before["pixels"][0], json!([255, 0, 0, 255]), "{before}");
    let blue = output.path().join("1.1.1.png");
    still(&blue, [0, 0, 255]);

    // Half and half: the frame on the left, the still on the right.
    let path = blue.to_str().unwrap();
    let wiped = client
        .result(
            "viewer.wipe",
            Some(json!({"style": "horizontal", "reference": {"kind": "still", "path": path}})),
        )
        .await;
    assert_eq!(wiped["mode"], "wipe", "{wiped}");
    assert_eq!(wiped["wipe"]["position"], 0.5);
    assert_eq!(
        wiped["wipe"]["reference"],
        json!({"kind": "still", "path": path})
    );
    let half = digest(&mut client).await;
    assert_eq!(half["pixels"][0], json!([255, 0, 0, 255]), "{half}");
    assert_ne!(half["hash"], before["hash"]);
    // The scopes measure what the viewer shows: half red, half blue.
    let figures = slow_result(
        &mut client,
        "scopes.statistics",
        json!({"view": "timeline"}),
        Duration::from_secs(300),
    )
    .await;
    for (channel, mean) in [("red", 0.5), ("green", 0.0), ("blue", 0.5)] {
        let measured = figures[channel]["mean"].as_f64().unwrap();
        assert!((measured - mean).abs() < 0.02, "{channel}: {figures}");
    }
    // Inverted, the still is on the left; at 0, it is all there is.
    let inverted = client
        .result(
            "viewer.wipe",
            Some(json!({"invert": true, "reference": {"kind": "still", "path": path}})),
        )
        .await;
    assert_eq!(inverted["wipe"]["invert"], true);
    assert_eq!(
        digest(&mut client).await["pixels"][0],
        json!([0, 0, 255, 255])
    );
    client
        .result(
            "viewer.wipe",
            Some(json!({"style": "box", "position": 0, "reference": {"kind": "still", "path": path}})),
        )
        .await;
    let all_still = digest(&mut client).await;
    for pixel in all_still["pixels"].as_array().unwrap() {
        assert_eq!(*pixel, json!([0, 0, 255, 255]), "{all_still}");
    }

    // An export renders the timeline, never the comparison.
    client.take_notifications();
    let film = output.path().join("exported.mov");
    let job = client
        .result(
            "render.start",
            Some(
                json!({"timeline": names["$timeline"], "path": film, "codec": "prores",
                "quality": "high"}),
            ),
        )
        .await["job"]
        .clone();
    loop {
        let message = if client.notifications.is_empty() {
            client.message().await
        } else {
            client.notifications.remove(0)
        };
        if message["method"] == "render.progress"
            && message["params"]["job"] == job
            && message["params"]["state"] != "running"
        {
            assert_eq!(message["params"]["state"], "done", "{message}");
            break;
        }
    }
    let mut decoder =
        columbina_media::VideoDecoder::open(&film, columbina_media::Hardware::Off).unwrap();
    let rgba = columbina_media::Converter::new()
        .rgba_scaled(&decoder.decoded_at(0).unwrap(), 64, 36)
        .unwrap();
    let middle = rgba.data.as_chunks::<4>().0[64 * 18 + 32];
    assert!(middle[0] > 240 && middle[2] < 15, "{middle:?}");

    // Off: the frame itself, bit for bit.
    let off = client
        .result("viewer.wipe", Some(json!({"on": false})))
        .await;
    assert_eq!(off, json!({"mode": "off"}));
    assert_eq!(digest(&mut client).await["hash"], before["hash"]);

    // A split screen of the frame and the still: two side by side, letterboxed, so the top row
    // is black; the frame outlined.
    let split = client
        .result(
            "viewer.split",
            Some(
                json!({"items": [{"kind": "current"}, {"kind": "still", "path": path}],
                "selected": 0}),
            ),
        )
        .await;
    assert_eq!(split["mode"], "split", "{split}");
    assert_eq!(split["split"]["outline"], true);
    let tiled = digest(&mut client).await;
    assert_eq!(tiled["pixels"][0], json!([0, 0, 0, 255]), "{tiled}");
    // A wipe set while splitting takes its place; turning the split off then changes nothing.
    client
        .result(
            "viewer.wipe",
            Some(json!({"reference": {"kind": "still", "path": path}})),
        )
        .await;
    assert_eq!(digest(&mut client).await["hash"], half["hash"]);
    let still_wiping = client
        .result("viewer.split", Some(json!({"on": false})))
        .await;
    assert_eq!(still_wiping["mode"], "wipe");
    client
        .result("viewer.wipe", Some(json!({"on": false})))
        .await;
    assert_eq!(digest(&mut client).await["hash"], before["hash"]);

    // What cannot be shown is refused, and changes nothing.
    let missing = output.path().join("missing.png");
    let error = client
        .error(
            "viewer.wipe",
            Some(json!({"reference": {"kind": "still", "path": missing}})),
        )
        .await;
    assert_eq!(error["code"], -32602, "{error}");
    assert_eq!(error["data"]["reason"], "still");
    let error = client
        .error("viewer.wipe", Some(json!({"style": "mix"})))
        .await;
    assert_eq!(error["data"]["reason"], "reference", "{error}");
    let error = client
        .error(
            "viewer.wipe",
            Some(json!({"softness": 2, "reference": {"kind": "still", "path": path}})),
        )
        .await;
    assert_eq!(error["data"]["reason"], "range", "{error}");
    assert_eq!(digest(&mut client).await["hash"], before["hash"]);
}

#[tokio::test]
async fn a_timeline_wipe_against_the_clip_before_shows_it_graded_at_the_same_offset() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let engine = engine(
        dir.path(),
        &["--no-proxies", "--sound", "null", "--test-hooks"],
    )
    .await;
    let mut client = engine.client("desktop").await;
    let names = timeline(
        &mut client,
        vec![
            colour("$green", "#20c040ff", 0, 24),
            colour("$red", "#ff0000ff", 24, 24),
        ],
    )
    .await;
    // The clip before is graded grey: saturation 0.
    let mut node = Node::corrector("$grey");
    let NodeKind::Corrector(settings) = &mut node.kind else {
        unreachable!("a corrector")
    };
    settings.primaries.saturation = Param::Fixed(0.0);
    let target = json!({"level": "clip", "timeline": names["$timeline"], "clip": names["$green"]});
    client
        .apply(
            json!([{"op": "add_node", "grade": target, "node": node, "links": [
            {"from": "source", "to": "$grey", "kind": "rgb", "input": 0},
            {"from": "$grey", "to": "output", "kind": "rgb", "input": 0}]}]),
        )
        .await;
    client
        .result(
            "transport.seek",
            Some(json!({"monitor": "timeline", "frame": 30})),
        )
        .await;
    let before = digest(&mut client).await;
    assert_eq!(before["frame"], 30);
    assert_eq!(before["pixels"][0], json!([255, 0, 0, 255]), "{before}");

    // All of the reference: the clip before, six frames into it as the playhead is into the
    // red one, graded grey.
    let wiped = client
        .result(
            "viewer.wipe",
            Some(json!({"position": 0, "reference": {"kind": "neighbour", "side": "previous"}})),
        )
        .await;
    assert_eq!(
        wiped["wipe"]["reference"],
        json!({"kind": "neighbour", "side": "previous", "at": 6}),
        "{wiped}"
    );
    let shown = digest(&mut client).await;
    for pixel in shown["pixels"].as_array().unwrap() {
        assert!(grey(pixel), "{shown}");
    }
    // A mix of half: between red and grey.
    client
        .result(
            "viewer.wipe",
            Some(json!({"style": "mix", "reference": {"kind": "neighbour", "side": "previous"}})),
        )
        .await;
    let mixed = digest(&mut client).await["pixels"][0].clone();
    let red = mixed[0].as_i64().unwrap();
    assert!(
        (150..240).contains(&red) && mixed[1].as_i64().unwrap() > 20,
        "{mixed}"
    );
    // A clip at a frame: the red one at its first frame, so the mix is red again.
    client
        .result(
            "viewer.wipe",
            Some(json!({"style": "mix", "reference": {"kind": "clip", "clip": names["$red"]}})),
        )
        .await;
    assert_eq!(
        digest(&mut client).await["pixels"][0],
        json!([255, 0, 0, 255])
    );
    // The red clip is the last: there is none after it, which is refused.
    let error = client
        .error(
            "viewer.wipe",
            Some(json!({"reference": {"kind": "neighbour", "side": "next"}})),
        )
        .await;
    assert_eq!(error["data"]["reason"], "reference", "{error}");

    // Off: the frame itself again.
    client
        .result("viewer.wipe", Some(json!({"on": false})))
        .await;
    assert_eq!(digest(&mut client).await["hash"], before["hash"]);
}

/// The timeline monitor's state once it shows `frame`.
async fn shown_at(client: &mut Client, frame: i64) -> Value {
    client
        .result(
            "transport.seek",
            Some(json!({"monitor": "timeline", "frame": frame})),
        )
        .await;
    client
        .until_state("the frame sought", |state| {
            state["monitor"] == "timeline" && state["shown"] == frame && state["exact"] == true
        })
        .await;
    client
        .result("transport.state", Some(json!({"monitor": "timeline"})))
        .await
}

#[tokio::test]
async fn neighbours_two_away_and_black_cells_are_compared_and_the_state_tells_where_they_are() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let engine = engine(
        dir.path(),
        &["--no-proxies", "--sound", "null", "--test-hooks"],
    )
    .await;
    let mut client = engine.client("desktop").await;
    timeline(
        &mut client,
        vec![
            colour("$green", "#20c040ff", 0, 24),
            colour("$red", "#ff0000ff", 24, 24),
            colour("$blue", "#0000ffff", 48, 24),
        ],
    )
    .await;
    let state = shown_at(&mut client, 54).await;
    assert!(state.get("compared").is_none(), "{state}");

    // Neighbor Clips: the two before the blue clip, six frames into each, the frame shown, and
    // a black cell where there is nothing.
    let split = client
        .result(
            "viewer.split",
            Some(json!({"items": [
                {"kind": "neighbour", "side": "previous", "steps": 2},
                {"kind": "neighbour", "side": "previous"},
                {"kind": "current"},
                {"kind": "black"},
            ]})),
        )
        .await;
    let items = &split["split"]["items"];
    assert_eq!(
        items[0],
        json!({"kind": "neighbour", "side": "previous", "steps": 2, "at": 6}),
        "{split}"
    );
    assert_eq!(items[1]["at"], 30, "{split}");
    assert_eq!(items[3], json!({"kind": "black"}), "{split}");
    let (told, _) = client
        .until_state("the references told", |state| {
            state.get("compared").is_some()
        })
        .await;
    assert_eq!(told["compared"], json!([6, 30, null, null]), "{told}");
    // They follow the playhead, and the state with them.
    let state = shown_at(&mut client, 60).await;
    assert_eq!(state["compared"], json!([12, 36, null, null]), "{state}");
    // None two after the blue one, nor black, in a wipe.
    for reference in [
        json!({"kind": "neighbour", "side": "next", "steps": 2}),
        json!({"kind": "black"}),
    ] {
        let error = client
            .error("viewer.wipe", Some(json!({"reference": reference})))
            .await;
        assert_eq!(error["data"]["reason"], "reference", "{error}");
    }
    // A wipe against the clip two before; off, nothing is compared.
    let wiped = client
        .result(
            "viewer.wipe",
            Some(json!({"reference": {"kind": "neighbour", "side": "previous", "steps": 2}})),
        )
        .await;
    assert_eq!(wiped["wipe"]["reference"]["at"], 12, "{wiped}");
    let state = shown_at(&mut client, 61).await;
    assert_eq!(state["compared"], json!([13]), "{state}");
    client
        .result("viewer.wipe", Some(json!({"on": false})))
        .await;
    let state = shown_at(&mut client, 62).await;
    assert!(state.get("compared").is_none(), "{state}");
}

#[tokio::test]
async fn a_headless_engine_has_no_viewer_to_compare_in() {
    let dir = project_dir();
    let engine = engine(dir.path(), &["--headless"]).await;
    let mut client = engine.client("agent").await;
    for method in ["viewer.wipe", "viewer.split"] {
        let error = client.error(method, Some(json!({"on": false}))).await;
        assert_eq!(error["code"], -32601, "{error}");
    }
}
