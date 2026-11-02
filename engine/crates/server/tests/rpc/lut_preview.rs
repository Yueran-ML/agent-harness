//! The LUT browser's previews through the real engine: a LUT on the test image and on a frame of
//! the current clip (`luts.preview`, pp. 3410–3411, 3259), its Live Preview in the timeline
//! monitor (`viewer.preview`, p. 3411), the LUTs of a source clip (`set_media_color`, pp.
//! 3409–3410), and the state of colour management being made (`color_pending`).

use std::io::Cursor;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{json, Value};

use super::{clip, engine, project_dir, Client};

/// How long a first frame of a new shader may take in a build for tests.
const SLOW: Duration = Duration::from_secs(300);

/// The answer to a method that may be long in coming, keeping the notifications before it.
async fn slow_answer(client: &mut Client, method: &str, params: Value) -> Value {
    use tokio::io::AsyncBufReadExt;
    let id = client.request(method, Some(params)).await;
    let deadline = Instant::now() + SLOW;
    loop {
        let mut line = String::new();
        tokio::time::timeout_at(deadline.into(), client.reader.read_line(&mut line))
            .await
            .expect("an answer in time")
            .unwrap();
        let message: Value = serde_json::from_str(&line).unwrap();
        if message.get("id").is_some() {
            assert_eq!(message["id"], id, "{message}");
            return message;
        }
        client.notifications.push(message);
    }
}

async fn slow(client: &mut Client, method: &str, params: Value) -> Value {
    let answer = slow_answer(client, method, params).await;
    assert!(answer.get("error").is_none(), "{answer}");
    answer["result"].clone()
}

async fn slow_error(client: &mut Client, method: &str, params: Value) -> Value {
    let answer = slow_answer(client, method, params).await;
    assert!(answer.get("result").is_none(), "{answer}");
    answer["error"].clone()
}

/// An orange, as a colour clip of 8 bits has it, and the same with red and blue swapped.
const ORANGE: [f64; 3] = [200.0 / 255.0, 110.0 / 255.0, 40.0 / 255.0];
const SWAPPED: [f64; 3] = [40.0 / 255.0, 110.0 / 255.0, 200.0 / 255.0];

/// A cube of 2³ that swaps red and blue: its point (r, g, b) holds (b, g, r).
fn swap() -> String {
    let mut cube = String::from("LUT_3D_SIZE 2\n");
    for b in 0..2 {
        for g in 0..2 {
            for r in 0..2 {
                cube.push_str(&[b, g, r].map(|v: i32| v.to_string()).join(" "));
                cube.push('\n');
            }
        }
    }
    cube
}

/// A timeline of 64 × 36 of an orange colour clip of 20 frames over all of it, whose grade has
/// one corrector that does nothing; its names (`$timeline`, `$c`, `$node`) and its grade.
async fn orange(client: &mut Client) -> (Value, Value) {
    let mut orange = clip("$c", "unused", 0, 20);
    orange["source"] = json!({"kind": "color", "color": "#c86e28ff"});
    let mut names = client
        .apply(json!([{"op": "add_timeline", "timeline": {
            "id": "$timeline", "name": "Orange", "width": 64, "height": 36,
            "rate": {"num": 25, "den": 1}, "sample_rate": 48000, "tracks": [
                {"id": "$v", "kind": "video", "name": "V1", "enabled": true, "locked": false,
                 "transitions": [], "clips": [orange]},
            ],
        }}]))
        .await["names"]
        .clone();
    let grade = json!({"level": "clip", "timeline": names["$timeline"], "clip": names["$c"]});
    let node = serde_json::to_value(columbina_model::Node::corrector("$node")).unwrap();
    let added = client
        .apply(
            json!([{"op": "add_node", "grade": grade, "node": node, "links": [
            {"from": "source", "to": "$node", "kind": "rgb", "input": 0},
            {"from": "$node", "to": "output", "kind": "rgb", "input": 0}]}]),
        )
        .await;
    names["$node"] = added["names"]["$node"].clone();
    (names, grade)
}

/// A PNG of 64 × 36 of `rgb`.
fn still(path: &std::path::Path, rgb: [u8; 3]) {
    let file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let mut encoder = png::Encoder::new(file, 64, 36);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&rgb.repeat(64 * 36)).unwrap();
}

/// The pixels of a preview: its width, height and R'G'B'A.
fn pixels(preview: &Value) -> (u32, u32, Vec<u8>) {
    let png = base64::engine::general_purpose::STANDARD
        .decode(preview["png"].as_str().unwrap())
        .unwrap();
    let mut reader = png::Decoder::new(Cursor::new(png)).read_info().unwrap();
    let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buffer).unwrap();
    buffer.truncate(info.buffer_size());
    (info.width, info.height, buffer)
}

fn rgb_at(pixels: &(u32, u32, Vec<u8>), x: u32, y: u32) -> [f64; 3] {
    let at = ((y * pixels.0 + x) * 4) as usize;
    [0, 1, 2].map(|i| f64::from(pixels.2[at + i]) / 255.0)
}

fn close(seen: [f64; 3], wanted: [f64; 3], within: f64) -> bool {
    (0..3).all(|i| (seen[i] - wanted[i]).abs() <= within)
}

/// The colour at the middle of the frame that the timeline monitor shows, once it is `wanted`.
async fn shows(client: &mut Client, wanted: [f64; 3]) {
    let deadline = Instant::now() + SLOW;
    loop {
        let sampled = slow(
            client,
            "view.sample",
            json!({"monitor": "timeline", "x": 32.5, "y": 18.5, "size": 3}),
        )
        .await;
        let seen = [0, 1, 2].map(|i| sampled["rgb"][i].as_f64().unwrap());
        if close(seen, wanted, 4e-3) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{seen:?} is shown, and {wanted:?} was wanted"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn a_lut_is_previewed_on_the_test_image_and_on_the_clip_through_its_node() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    std::fs::create_dir(dir.path().join("looks")).unwrap();
    std::fs::write(dir.path().join("looks").join("swap.cube"), swap()).unwrap();
    let engine = engine(
        dir.path(),
        &["--no-proxies", "--sound", "null", "--headless"],
    )
    .await;
    let mut client = engine.client("agent").await;
    let (names, grade) = orange(&mut client).await;

    // The test image: 16:9 at the height asked for; one that cannot be read says why.
    let answer = slow(
        &mut client,
        "luts.preview",
        json!({"luts": ["looks/swap.cube", "looks/gone.cube"], "height": 18}),
    )
    .await;
    let previews = answer["previews"].as_array().unwrap();
    assert_eq!(previews.len(), 2, "{answer}");
    assert_eq!(previews[0]["lut"], "looks/swap.cube");
    let image = pixels(&previews[0]);
    assert_eq!((image.0, image.1), (32, 18));
    // Its ramp of grey is grey through the swap; its red at the top left is blue.
    let grey = rgb_at(&image, 16, 17);
    assert!(close(grey, [grey[0]; 3], 1.0 / 255.0), "{grey:?}");
    let top = rgb_at(&image, 0, 0);
    assert!(top[2] > 0.8 && top[0] < 0.2, "{top:?}");
    assert_eq!(previews[1]["lut"], "looks/gone.cube");
    assert!(previews[1]["error"].is_string(), "{answer}");

    // On the clip at a frame, through its node: red and blue swapped. The project is as it was.
    let before = client.result("project.get", None).await;
    let on = json!({"timeline": names["$timeline"], "frame": 12, "clip": names["$c"],
        "grade": grade, "node": names["$node"]});
    let answer = slow(
        &mut client,
        "luts.preview",
        json!({"luts": ["looks/swap.cube"], "height": 18, "frame": on}),
    )
    .await;
    let preview = &answer["previews"][0];
    assert_eq!(preview["lut"], "looks/swap.cube", "{answer}");
    assert!(preview.get("frame").is_none(), "{answer}");
    let seen = pixels(preview);
    assert_eq!((seen.0, seen.1), (32, 18));
    let middle = rgb_at(&seen, 16, 9);
    assert!(close(middle, SWAPPED, 2.5 / 255.0), "{middle:?}");
    let after = client.result("project.get", None).await;
    assert_eq!(after["revision"], before["revision"]);
    assert_eq!(after["project"], before["project"]);

    // Refused: a node that is not there, and params that are not the method's.
    let mut gone = on.clone();
    gone["node"] = json!("n999");
    let error = slow_error(
        &mut client,
        "luts.preview",
        json!({"luts": ["looks/swap.cube"], "frame": gone}),
    )
    .await;
    assert_eq!(error["code"], -32602, "{error}");
    assert_eq!(error["data"]["reason"], "node", "{error}");
    for (params, reason) in [
        (json!({"luts": []}), Some("luts")),
        (json!({"luts": ["../swap.cube"]}), Some("name")),
        (
            json!({"luts": ["looks/swap.cube"], "height": 300}),
            Some("height"),
        ),
        (json!({"lut": "looks/swap.cube"}), None),
    ] {
        let error = slow_error(&mut client, "luts.preview", params.clone()).await;
        assert_eq!(error["code"], -32602, "{params}: {error}");
        assert_eq!(
            error["data"].get("reason").cloned(),
            reason.map(|reason| json!(reason)),
            "{params}: {error}"
        );
    }
    // A headless engine has no monitor to preview on.
    let error = client
        .error("viewer.preview", Some(json!({"on": false})))
        .await;
    assert_eq!(error["code"], -32601, "{error}");
    let error = client.error("viewer.lut", Some(json!({"on": false}))).await;
    assert_eq!(error["code"], -32601, "{error}");
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "on, at a frame of the clip, through another change, its still, and off"
)]
async fn a_live_preview_shows_a_lut_on_the_node_in_the_monitor_alone() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    std::fs::create_dir(dir.path().join("looks")).unwrap();
    std::fs::write(dir.path().join("looks").join("swap.cube"), swap()).unwrap();
    let engine = engine(dir.path(), &["--no-proxies", "--sound", "null"]).await;
    let mut client = engine.client("desktop").await;
    let (names, grade) = orange(&mut client).await;
    client
        .result(
            "transport.load",
            Some(json!({"monitor": "timeline", "timeline": names["$timeline"]})),
        )
        .await;
    shows(&mut client, ORANGE).await;
    let before = client.result("project.get", None).await;

    // On: the monitor shows the LUT on the node; the state says so; the project is as it was.
    let asked = json!({"grade": grade, "node": names["$node"], "lut": "looks/swap.cube"});
    let answer = slow(&mut client, "viewer.preview", asked.clone()).await;
    assert_eq!(
        answer,
        json!({"monitor": "timeline", "preview": asked}),
        "{answer}"
    );
    shows(&mut client, SWAPPED).await;
    let state = client
        .result("transport.state", Some(json!({"monitor": "timeline"})))
        .await;
    assert_eq!(state["preview"], asked, "{state}");
    let after = client.result("project.get", None).await;
    assert_eq!(after["revision"], before["revision"]);
    assert_eq!(after["project"], before["project"]);

    // Hover Scrub: a frame of the clip is shown, and the playhead stays where it is.
    let mut scrubbed = asked.clone();
    scrubbed["frame"] = json!(15);
    slow(&mut client, "viewer.preview", scrubbed.clone()).await;
    let state = client
        .result("transport.state", Some(json!({"monitor": "timeline"})))
        .await;
    assert_eq!(
        (&state["frame"], &state["shown"]),
        (&json!(0), &json!(15)),
        "{state}"
    );
    shows(&mut client, SWAPPED).await;

    // A still is grabbed of the frame as the project grades it.
    let still = slow(&mut client, "still.grab", json!({})).await;
    let path = dir
        .path()
        .join("gallery")
        .join("stills")
        .join(format!("{}.png", still["id"].as_str().unwrap()));
    let mut reader = png::Decoder::new(Cursor::new(std::fs::read(path).unwrap()))
        .read_info()
        .unwrap();
    let width = reader.info().width;
    let mut data = vec![0; reader.output_buffer_size().unwrap()];
    reader.next_frame(&mut data).unwrap();
    let at = ((18 * width + 32) * 3 * 2) as usize;
    let rgb = [0, 1, 2]
        .map(|c| f64::from(u16::from_be_bytes([data[at + c * 2], data[at + c * 2 + 1]])) / 65535.0);
    assert!(close(rgb, ORANGE, 2.5 / 255.0), "{rgb:?}");
    shows(&mut client, SWAPPED).await;

    // A node that cannot take it shows the picture as it is, the preview still on.
    let mut gone = asked.clone();
    gone["node"] = json!("n999");
    slow(&mut client, "viewer.preview", gone.clone()).await;
    shows(&mut client, ORANGE).await;
    slow(&mut client, "viewer.preview", asked.clone()).await;
    shows(&mut client, SWAPPED).await;

    // Off: the project's grade again, at the playhead.
    assert_eq!(
        slow(&mut client, "viewer.preview", json!({"on": false})).await,
        json!({"monitor": "timeline", "preview": null})
    );
    shows(&mut client, ORANGE).await;
    let state = client
        .result("transport.state", Some(json!({"monitor": "timeline"})))
        .await;
    assert!(state.get("preview").is_none(), "{state}");
    assert_eq!(state["shown"], 0, "{state}");

    // The Color Viewer LUT is turned off and on; the state says where it is off.
    assert_eq!(
        slow(&mut client, "viewer.lut", json!({"on": false})).await,
        json!({"monitor": "timeline", "on": false})
    );
    let state = client
        .result("transport.state", Some(json!({"monitor": "timeline"})))
        .await;
    assert_eq!(state["viewer_lut"], false, "{state}");
    assert_eq!(
        slow(&mut client, "viewer.lut", json!({"on": true})).await,
        json!({"monitor": "timeline", "on": true})
    );
    let state = client
        .result("transport.state", Some(json!({"monitor": "timeline"})))
        .await;
    assert!(state.get("viewer_lut").is_none(), "{state}");

    // Refused.
    for (method, params, reason) in [
        (
            "viewer.preview",
            json!({"on": false, "monitor": "source"}),
            Some("monitor"),
        ),
        (
            "viewer.preview",
            json!({"grade": grade, "node": "n1"}),
            Some("lut"),
        ),
        (
            "viewer.preview",
            json!({"grade": grade, "node": "n1", "lut": "C:/a.cube"}),
            Some("name"),
        ),
        (
            "viewer.lut",
            json!({"on": true, "monitor": "source"}),
            Some("monitor"),
        ),
        ("viewer.lut", json!({}), None),
    ] {
        let error = slow_error(&mut client, method, params.clone()).await;
        assert_eq!(error["code"], -32602, "{method} {params}: {error}");
        assert_eq!(
            error["data"].get("reason").cloned(),
            reason.map(|reason| json!(reason)),
            "{method} {params}: {error}"
        );
    }
}

/// The Live Preview that the timeline monitor shows, as `client` reads it from the state.
async fn previewed(client: &mut Client) -> Value {
    let state = client
        .result("transport.state", Some(json!({"monitor": "timeline"})))
        .await;
    state.get("preview").cloned().unwrap_or(Value::Null)
}

/// Wait until no Live Preview is on, as `client` reads the state.
async fn ended(client: &mut Client) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while previewed(client).await != Value::Null {
        assert!(Instant::now() < deadline, "the preview is still on");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn a_live_preview_ends_with_the_connection_that_turned_it_on_last() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    std::fs::create_dir(dir.path().join("looks")).unwrap();
    std::fs::write(dir.path().join("looks").join("swap.cube"), swap()).unwrap();
    let engine = engine(dir.path(), &["--no-proxies", "--sound", "null"]).await;
    let mut watcher = engine.client("desktop").await;
    let (names, grade) = orange(&mut watcher).await;
    watcher
        .result(
            "transport.load",
            Some(json!({"monitor": "timeline", "timeline": names["$timeline"]})),
        )
        .await;
    let asked = json!({"grade": grade, "node": names["$node"], "lut": "looks/swap.cube"});

    // Turned on by a connection that ends: it ends with it, and the monitor shows the grade.
    let mut first = engine.client("desktop").await;
    slow(&mut first, "viewer.preview", asked.clone()).await;
    shows(&mut watcher, SWAPPED).await;
    drop(first);
    ended(&mut watcher).await;
    shows(&mut watcher, ORANGE).await;

    // Turned on by one and then by another: the first that ends leaves the other's on.
    let mut first = engine.client("desktop").await;
    let mut second = engine.client("desktop").await;
    slow(&mut first, "viewer.preview", asked.clone()).await;
    slow(&mut second, "viewer.preview", asked.clone()).await;
    drop(first);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(previewed(&mut watcher).await, asked);
    drop(second);
    ended(&mut watcher).await;

    // Turned off by its owner before it ends: nothing is left to end, and another's stays on.
    let mut first = engine.client("desktop").await;
    slow(&mut first, "viewer.preview", asked.clone()).await;
    slow(&mut first, "viewer.preview", json!({"on": false})).await;
    slow(&mut watcher, "viewer.preview", asked.clone()).await;
    drop(first);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(previewed(&mut watcher).await, asked);
}

#[tokio::test]
async fn the_luts_of_a_source_clip_are_on_its_pictures_and_the_state_says_while_colour_is_made() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let files = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("swap.cube"), swap()).unwrap();
    let picture = files.path().join("orange.png");
    still(&picture, [200, 110, 40]);
    let engine = engine(dir.path(), &["--no-proxies", "--sound", "null"]).await;
    let mut client = engine.client("desktop").await;
    let media = client
        .result("media.import", Some(json!({"paths": [picture]})))
        .await["added"][0]
        .clone();
    let scale = client.result("project.get", None).await["project"]["media"][0]["facts"]
        ["duration"]["scale"]
        .as_i64()
        .unwrap();
    let mut still = clip("$still", media.as_str().unwrap(), 0, 10);
    still["source"]["in"] = json!({"ticks": 0, "scale": scale});
    let names = client
        .apply(json!([{"op": "add_timeline", "timeline": {
            "id": "$timeline", "name": "Source", "width": 64, "height": 36,
            "rate": {"num": 25, "den": 1}, "sample_rate": 48000,
            "tracks": [{"id": "$v", "kind": "video", "name": "V1", "enabled": true,
                "locked": false, "clips": [still], "transitions": []}],
        }}]))
        .await["names"]
        .clone();
    client
        .result(
            "transport.load",
            Some(json!({"monitor": "timeline", "timeline": names["$timeline"]})),
        )
        .await;
    shows(&mut client, ORANGE).await;

    // The source clip's LUT: on its picture, in a change of colour that is made apart from the
    // monitor, which says so while it is.
    client.take_notifications();
    client
        .apply(json!([{"op": "set_media_color", "id": media,
            "lut": {"1d": null, "3d": "swap.cube"}}]))
        .await;
    shows(&mut client, SWAPPED).await;
    let states: Vec<Value> = client
        .take_notifications()
        .into_iter()
        .filter(|message| message["method"] == "transport.state")
        .map(|message| message["params"].clone())
        .collect();
    let pending = states
        .iter()
        .position(|state| state["color_pending"] == true);
    assert!(pending.is_some(), "{states:?}");
    // Told again once it is made.
    let deadline = Instant::now() + SLOW;
    loop {
        let state = client
            .result("transport.state", Some(json!({"monitor": "timeline"})))
            .await;
        if state.get("color_pending").is_none() {
            assert!(state.get("color_error").is_none(), "{state}");
            break;
        }
        assert!(Instant::now() < deadline, "{state}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Undone: the picture as it was.
    client.result("history.undo", None).await;
    shows(&mut client, ORANGE).await;
}
