//! What the Color page of the desktop asks of the engine besides its palettes, through the real
//! engine: Bypass Color of the timeline monitor (`viewer.bypass`, p. 2987), and a thumbnail of a
//! frame as it is graded (`timeline.thumbnail`, p. 3026).

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

/// An orange, as a colour clip of 8 bits has it.
const ORANGE: [f64; 3] = [200.0 / 255.0, 110.0 / 255.0, 40.0 / 255.0];

/// A timeline of 64 × 36 with an orange colour clip of 10 frames in its upper left quarter,
/// whose own grade halves it, and the timeline's grade adds a quarter to red; its names, and
/// those of the nodes of the two grades (`$clip_node`, `$timeline_node`).
async fn graded(client: &mut Client) -> Value {
    let mut orange = clip("$c", "unused", 0, 10);
    orange["source"] = json!({"kind": "color", "color": "#c86e28ff"});
    orange["transform"] =
        json!({"x": -0.25, "y": -0.25, "scale": 0.5, "rotation": 0.0, "opacity": 1.0});
    let names = client
        .apply(json!([{"op": "add_timeline", "timeline": {
            "id": "$timeline", "name": "Graded", "width": 64, "height": 36,
            "rate": {"num": 25, "den": 1}, "sample_rate": 48000, "tracks": [
                {"id": "$v", "kind": "video", "name": "V1", "enabled": true, "locked": false,
                 "transitions": [], "clips": [orange]},
            ],
        }}]))
        .await["names"]
        .clone();
    let mut names = names;
    for (place, grade, settings) in [
        (
            "$clip_node",
            json!({"level": "clip", "timeline": names["$timeline"], "clip": names["$c"]}),
            json!({"primaries": {"gain": {"y": 0.5, "r": 0.5, "g": 0.5, "b": 0.5}}}),
        ),
        (
            "$timeline_node",
            json!({"level": "timeline", "timeline": names["$timeline"]}),
            json!({"primaries": {"offset": {"r": 50}}}),
        ),
    ] {
        let mut node = serde_json::to_value(columbina_model::Node::corrector("$node")).unwrap();
        merge(&mut node["params"], &settings);
        let added = client
            .apply(
                json!([{"op": "add_node", "grade": grade, "node": node, "links": [
                {"from": "source", "to": "$node", "kind": "rgb", "input": 0},
                {"from": "$node", "to": "output", "kind": "rgb", "input": 0}]}]),
            )
            .await;
        names[place] = added["names"]["$node"].clone();
    }
    names
}

/// `patch` put into `base`, field by field where both are objects.
fn merge(base: &mut Value, patch: &Value) {
    match (base, patch) {
        (Value::Object(base), Value::Object(patch)) => {
            for (name, value) in patch {
                merge(base.entry(name.clone()).or_insert(Value::Null), value);
            }
        }
        (base, patch) => *base = patch.clone(),
    }
}

/// The orange as the grades make it: halved by the clip's, a quarter added to red by the
/// timeline's.
fn halved() -> [f64; 3] {
    ORANGE.map(|channel| channel * 0.5)
}

fn near(value: &Value, expected: [f64; 3], within: f64) -> bool {
    (0..3).all(|i| (value[i].as_f64().unwrap() - expected[i]).abs() <= within)
}

/// The colours at the middle of the orange and of the black beside it that the timeline monitor
/// shows, once they are `wanted`, waiting while it draws.
async fn shows(client: &mut Client, wanted: [[f64; 3]; 2]) {
    let deadline = Instant::now() + SLOW;
    loop {
        let mut seen = Vec::new();
        for (x, y) in [(16.5, 9.5), (48.5, 27.5)] {
            let sampled = slow(
                client,
                "view.sample",
                json!({"monitor": "timeline", "x": x, "y": y, "size": 3}),
            )
            .await;
            seen.push(sampled["rgb"].clone());
        }
        if seen
            .iter()
            .zip(wanted)
            .all(|(seen, wanted)| near(seen, wanted, 4e-3))
        {
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
async fn bypass_color_shows_the_picture_without_its_grades_and_says_so() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let engine = engine(dir.path(), &["--no-proxies", "--sound", "null"]).await;
    let mut client = engine.client("desktop").await;
    let names = graded(&mut client).await;
    client
        .result(
            "transport.load",
            Some(json!({"monitor": "timeline", "timeline": names["$timeline"]})),
        )
        .await;
    let [r, g, b] = halved();
    let graded = [[r + 0.25, g, b], [0.25, 0.0, 0.0]];
    shows(&mut client, graded).await;
    let state = client
        .result("transport.state", Some(json!({"monitor": "timeline"})))
        .await;
    assert!(state.get("bypass").is_none(), "{state}");

    // On: the orange as it is, and the black; the state says so.
    let on = slow(&mut client, "viewer.bypass", json!({"on": true})).await;
    assert_eq!(on, json!({"monitor": "timeline", "on": true}));
    shows(&mut client, [ORANGE, [0.0; 3]]).await;
    let state = client
        .result("transport.state", Some(json!({"monitor": "timeline"})))
        .await;
    assert_eq!(state["bypass"], true, "{state}");
    // A Highlight mode shows nothing of a key while the grades are bypassed.
    let grade = json!({"level": "clip", "timeline": names["$timeline"], "clip": names["$c"]});
    let node = names["$clip_node"].clone();
    assert!(node.is_string(), "{names}");
    slow(
        &mut client,
        "viewer.highlight",
        json!({"mode": "bw", "grade": grade, "node": node}),
    )
    .await;
    shows(&mut client, [ORANGE, [0.0; 3]]).await;
    slow(&mut client, "viewer.highlight", json!({"mode": "off"})).await;
    // The same again changes nothing; off shows the grades again.
    assert_eq!(
        slow(
            &mut client,
            "viewer.bypass",
            json!({"on": true, "monitor": "timeline"})
        )
        .await,
        json!({"monitor": "timeline", "on": true})
    );
    assert_eq!(
        slow(&mut client, "viewer.bypass", json!({"on": false})).await,
        json!({"monitor": "timeline", "on": false})
    );
    shows(&mut client, graded).await;
    let state = client
        .result("transport.state", Some(json!({"monitor": "timeline"})))
        .await;
    assert!(state.get("bypass").is_none(), "{state}");

    // Refused: the source monitor, and params that are not the method's.
    let error = slow_error(
        &mut client,
        "viewer.bypass",
        json!({"on": true, "monitor": "source"}),
    )
    .await;
    assert_eq!(error["code"], -32602, "{error}");
    assert_eq!(error["data"]["reason"], "monitor", "{error}");
    for params in [
        json!({}),
        json!({"on": "yes"}),
        json!({"on": true, "grades": false}),
    ] {
        let error = slow_error(&mut client, "viewer.bypass", params.clone()).await;
        assert_eq!(error["code"], -32602, "{params}: {error}");
    }
}

/// The pixels of a thumbnail: its width, height and R'G'B'A.
fn pixels(thumbnail: &Value) -> (u32, u32, Vec<u8>) {
    let png = base64::engine::general_purpose::STANDARD
        .decode(thumbnail["png"].as_str().unwrap())
        .unwrap();
    let mut reader = png::Decoder::new(Cursor::new(png)).read_info().unwrap();
    let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buffer).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    buffer.truncate(info.buffer_size());
    (info.width, info.height, buffer)
}

fn rgb_at(pixels: &(u32, u32, Vec<u8>), x: u32, y: u32) -> [f64; 3] {
    let at = ((y * pixels.0 + x) * 4) as usize;
    [0, 1, 2].map(|i| f64::from(pixels.2[at + i]) / 255.0)
}

fn close(seen: [f64; 3], wanted: [f64; 3]) -> bool {
    (0..3).all(|i| (seen[i] - wanted[i]).abs() <= 2.5 / 255.0)
}

#[tokio::test]
async fn a_thumbnail_is_of_the_frame_as_it_is_graded_or_of_a_clip_by_its_own_grades() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let engine = engine(
        dir.path(),
        &["--no-proxies", "--sound", "null", "--headless"],
    )
    .await;
    let mut client = engine.client("agent").await;
    // A headless engine has no monitor to bypass.
    let error = client
        .error("viewer.bypass", Some(json!({"on": true})))
        .await;
    assert_eq!(error["code"], -32601, "{error}");
    let names = graded(&mut client).await;
    let [r, g, b] = halved();

    // The whole frame: the orange in its quarter by both grades, the rest black and a quarter red.
    let whole = slow(
        &mut client,
        "timeline.thumbnail",
        json!({"timeline": names["$timeline"], "frame": 3, "height": 18}),
    )
    .await;
    assert_eq!(
        (&whole["width"], &whole["height"], &whole["frame"]),
        (&json!(32), &json!(18), &json!(3))
    );
    let seen = pixels(&whole);
    assert_eq!((seen.0, seen.1), (32, 18));
    assert!(
        close(rgb_at(&seen, 8, 4), [r + 0.25, g, b]),
        "{:?}",
        rgb_at(&seen, 8, 4)
    );
    assert!(
        close(rgb_at(&seen, 24, 13), [0.25, 0.0, 0.0]),
        "{:?}",
        rgb_at(&seen, 24, 13)
    );
    // Its alpha is opaque.
    assert!(seen.2.chunks(4).all(|pixel| pixel[3] == 255));

    // The clip alone: its media as its own grade makes it, over the whole picture; the
    // timeline's grade left out.
    let alone = slow(
        &mut client,
        "timeline.thumbnail",
        json!({"timeline": names["$timeline"], "frame": 3, "clip": names["$c"], "height": 9}),
    )
    .await;
    let seen = pixels(&alone);
    assert_eq!((seen.0, seen.1), (16, 9));
    for (x, y) in [(1, 1), (8, 4), (14, 7)] {
        assert!(
            close(rgb_at(&seen, x, y), [r, g, b]),
            "({x}, {y}): {:?}",
            rgb_at(&seen, x, y)
        );
    }
    // A clip on a track that is off still has its thumbnail, as it is graded; the whole frame
    // no longer shows it.
    client
        .apply(
            json!([{"op": "set_track", "timeline": names["$timeline"], "id": names["$v"],
            "enabled": false}]),
        )
        .await;
    let off = slow(
        &mut client,
        "timeline.thumbnail",
        json!({"timeline": names["$timeline"], "frame": 3, "clip": names["$c"], "height": 9}),
    )
    .await;
    let seen = pixels(&off);
    assert_eq!((seen.0, seen.1), (16, 9));
    assert!(
        close(rgb_at(&seen, 8, 4), [r, g, b]),
        "{:?}",
        rgb_at(&seen, 8, 4)
    );
    let whole = slow(
        &mut client,
        "timeline.thumbnail",
        json!({"timeline": names["$timeline"], "frame": 3, "height": 18}),
    )
    .await;
    assert!(
        close(rgb_at(&pixels(&whole), 8, 4), [0.25, 0.0, 0.0]),
        "{:?}",
        rgb_at(&pixels(&whole), 8, 4)
    );
    client
        .apply(
            json!([{"op": "set_track", "timeline": names["$timeline"], "id": names["$v"],
            "enabled": true}]),
        )
        .await;

    // No higher than the timeline.
    let tall = slow(
        &mut client,
        "timeline.thumbnail",
        json!({"timeline": names["$timeline"], "frame": 0, "height": 270}),
    )
    .await;
    assert_eq!((&tall["width"], &tall["height"]), (&json!(64), &json!(36)));

    // Refused: a timeline or clip that is not there, a frame the clip is not seen at, a
    // height out of its range.
    for (params, reason) in [
        (json!({"timeline": "t999", "frame": 0}), Some("timeline")),
        (
            json!({"timeline": names["$timeline"], "frame": 0, "clip": "c999"}),
            Some("clip"),
        ),
        (
            json!({"timeline": names["$timeline"], "frame": 20, "clip": names["$c"]}),
            Some("clip"),
        ),
        (
            json!({"timeline": names["$timeline"], "frame": 0, "height": 0}),
            Some("height"),
        ),
        (json!({"timeline": names["$timeline"]}), None),
    ] {
        let error = slow_error(&mut client, "timeline.thumbnail", params.clone()).await;
        assert_eq!(error["code"], -32602, "{params}: {error}");
        if let Some(reason) = reason {
            assert_eq!(error["data"]["reason"], reason, "{params}: {error}");
        }
    }
}
