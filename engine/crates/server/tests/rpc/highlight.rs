//! The Highlight modes of the timeline monitor through the real engine (`viewer.highlight`, pp.
//! 3170–3171): the picture where the key of the node chosen is and grey where it is not, the key
//! itself in black and white, a node without a key shown alone; told in the state of the monitor,
//! in the preview alone; and the key of a node sampled with `view.sample`.

use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::{clip, engine, project_dir, Client};

/// How long a first frame of a new shader may take: FXC compiles a grade with a qualifier slowly
/// in a build for tests.
const SLOW: Duration = Duration::from_secs(300);

/// 75% of white, as a picture of 8 bits has it.
const BAR: f64 = 191.0 / 255.0;

/// A colour clip of `colour` in a quarter of the canvas.
fn bar(id: &str, colour: &str, (x, y): (f64, f64)) -> Value {
    let mut made = clip(id, "unused", 0, 50);
    made["source"] = json!({"kind": "color", "color": colour});
    made["transform"] = json!({"x": x, "y": y, "scale": 0.5, "rotation": 0.0, "opacity": 1.0});
    made
}

/// Colour bars of 75% of 128 × 72, red, green, blue and grey, each a clip on a track of its own,
/// loaded in the timeline monitor; their names.
async fn bars(client: &mut Client) -> Value {
    let tracks: Vec<Value> = [
        bar("$red", "#bf0000ff", (-0.25, -0.25)),
        bar("$green", "#00bf00ff", (0.25, -0.25)),
        bar("$blue", "#0000bfff", (-0.25, 0.25)),
        bar("$grey", "#bfbfbfff", (0.25, 0.25)),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, clip)| {
        json!({"id": format!("$v{index}"), "kind": "video", "name": format!("V{index}"),
            "enabled": true, "locked": false, "clips": [clip], "transitions": []})
    })
    .collect();
    let names = client
        .apply(json!([{"op": "add_timeline", "timeline": {
            "id": "$timeline", "name": "Highlight", "width": 128, "height": 72,
            "rate": {"num": 25, "den": 1}, "sample_rate": 48000, "tracks": tracks,
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

/// The middles of the quarters: red, green, blue, grey.
const QUARTERS: [(f64, f64); 4] = [(32.5, 18.5), (96.5, 18.5), (32.5, 54.5), (96.5, 54.5)];

/// The answer to a method that may be long in coming, keeping the notifications that come
/// before it.
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

fn near(value: &Value, expected: [f64; 3], within: f64) -> bool {
    (0..3).all(|i| (value[i].as_f64().unwrap() - expected[i]).abs() <= within)
}

/// The colours of the middles of the quarters that the timeline monitor shows, once they are
/// `wanted` (within 4e-3), waiting while the monitor draws.
async fn shows(client: &mut Client, wanted: [[f64; 3]; 4]) {
    let deadline = Instant::now() + SLOW;
    loop {
        let mut seen = Vec::new();
        for (x, y) in QUARTERS {
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

/// A corrector whose settings `settings` are, put between the source and the output of `grade`
/// (a node of the timeline's grade after the other); its id.
async fn add_corrector(client: &mut Client, grade: &Value, settings: Value, after: &str) -> String {
    let mut node = serde_json::to_value(columbina_model::Node::corrector("$node")).unwrap();
    merge(&mut node["params"], &settings);
    let names = client
        .apply(
            json!([{"op": "add_node", "grade": grade, "node": node, "links": [
            {"from": after, "to": "$node", "kind": "rgb", "input": 0},
            {"from": "$node", "to": "output", "kind": "rgb", "input": 0}]}]),
        )
        .await["names"]
        .clone();
    names["$node"].as_str().unwrap().to_owned()
}

#[tokio::test]
#[allow(clippy::too_many_lines, reason = "each mode, then the refusals")]
async fn highlight_shows_the_key_of_the_node_chosen_on_the_monitor_alone() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let engine = engine(dir.path(), &["--no-proxies", "--sound", "null"]).await;
    let mut client = engine.client("desktop").await;
    let names = bars(&mut client).await;
    let timeline_id = names["$timeline"].clone();
    let (red, blue) = ([BAR, 0.0, 0.0], [0.0, 0.0, BAR]);

    // The timeline's grade: n1 keys the light (luma from 40%: the green and the grey) and
    // lifts their blue by 0.25; n2, without a key, takes the colour away from it all.
    let timeline_grade = json!({"level": "timeline", "timeline": timeline_id});
    let qualifier = {
        let mut q = serde_json::to_value(columbina_model::Qualifier::default()).unwrap();
        q["mode"] = json!("lum");
        q["lum"]["low"] = json!(40);
        q
    };
    let n1 = add_corrector(
        &mut client,
        &timeline_grade,
        json!({"qualifier": qualifier, "primaries": {"offset": {"b": 50}}}),
        "source",
    )
    .await;
    let n2 = add_corrector(
        &mut client,
        &timeline_grade,
        json!({"primaries": {"saturation": 25}}),
        &n1,
    )
    .await;
    let half_sat = |rgb: [f64; 3]| {
        let y = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
        rgb.map(|c| y + (c - y) * 0.5)
    };
    let graded = [
        half_sat(red),
        half_sat([0.0, BAR, 0.25]),
        half_sat(blue),
        half_sat([BAR, BAR, BAR + 0.25]),
    ];
    shows(&mut client, graded).await;

    // Highlight of n1: the light as they are graded, the rest a flat grey.
    let on = slow(
        &mut client,
        "viewer.highlight",
        json!({"mode": "highlight", "grade": timeline_grade, "node": n1}),
    )
    .await;
    assert_eq!(
        on,
        json!({"monitor": "timeline", "mode": "highlight", "grade": timeline_grade, "node": n1})
    );
    shows(&mut client, [[0.5; 3], graded[1], [0.5; 3], graded[3]]).await;
    let state = client
        .result("transport.state", Some(json!({"monitor": "timeline"})))
        .await;
    assert_eq!(state["highlight"]["mode"], "highlight", "{state}");
    assert_eq!(state["highlight"]["node"], json!(n1), "{state}");

    // Highlight BW: the key itself, white and black, the output transform and the rest left
    // out.
    slow(
        &mut client,
        "viewer.highlight",
        json!({"mode": "bw", "grade": timeline_grade, "node": n1}),
    )
    .await;
    shows(&mut client, [[0.0; 3], [1.0; 3], [0.0; 3], [1.0; 3]]).await;

    // A node without a key is shown alone: n2 after n1's change, every bar desaturated; its
    // key, white throughout.
    slow(
        &mut client,
        "viewer.highlight",
        json!({"mode": "highlight", "grade": timeline_grade, "node": n2}),
    )
    .await;
    shows(&mut client, graded).await;
    slow(
        &mut client,
        "viewer.highlight",
        json!({"mode": "bw", "grade": timeline_grade, "node": n2}),
    )
    .await;
    shows(&mut client, [[1.0; 3]; 4]).await;

    // A node of the red clip's grade that keys its red: BW shows its layer alone, white, and
    // black where it is not; Highlight shows the others as they are.
    let red_grade = json!({"level": "clip", "timeline": timeline_id, "clip": names["$red"]});
    let reds = {
        let mut q = serde_json::to_value(columbina_model::Qualifier::default()).unwrap();
        q["hue"]["center"] = json!(0);
        q["hue"]["width"] = json!(10);
        q["sat"]["low"] = json!(50);
        q
    };
    let n3 = add_corrector(
        &mut client,
        &red_grade,
        json!({"qualifier": reds, "primaries": {"gain": {"y": 0.5, "r": 0.5, "g": 0.5, "b": 0.5}}}),
        "source",
    )
    .await;
    slow(
        &mut client,
        "viewer.highlight",
        json!({"mode": "bw", "grade": red_grade, "node": n3}),
    )
    .await;
    shows(&mut client, [[1.0; 3], [0.0; 3], [0.0; 3], [0.0; 3]]).await;
    slow(
        &mut client,
        "viewer.highlight",
        json!({"mode": "highlight", "grade": red_grade, "node": n3}),
    )
    .await;
    // With n3 the red clip is graded too: its key is full on the red, so Highlight shows the red
    // as the whole frame has it.
    let with_n3 = [
        half_sat([BAR * 0.5, 0.0, 0.0]),
        graded[1],
        graded[2],
        graded[3],
    ];
    shows(&mut client, with_n3).await;

    // The key at a node, sampled: n3's where the red is, and where it is not; n1's.
    let sampled = slow(
        &mut client,
        "view.sample",
        json!({"timeline": timeline_id, "frame": 0, "x": 32.5, "y": 18.5, "size": 3,
            "grade": red_grade, "node": n3}),
    )
    .await;
    assert!(
        (sampled["node"]["key"].as_f64().unwrap() - 1.0).abs() < 1e-3,
        "{sampled}"
    );
    assert!(near(&sampled["node"]["input"], red, 2e-3), "{sampled}");
    for ((x, y), key) in QUARTERS.into_iter().zip([0.0, 1.0, 0.0, 1.0]) {
        let sampled = slow(
            &mut client,
            "view.sample",
            json!({"monitor": "timeline", "x": x, "y": y, "grade": timeline_grade, "node": n1}),
        )
        .await;
        assert!(
            (sampled["node"]["key"].as_f64().unwrap() - key).abs() < 1e-3,
            "{sampled}"
        );
    }

    // Off: the picture as it was, and the state says nothing of it.
    let off = slow(&mut client, "viewer.highlight", json!({"mode": "off"})).await;
    assert_eq!(off, json!({"monitor": "timeline", "mode": "off"}));
    shows(&mut client, with_n3).await;
    let state = client
        .result("transport.state", Some(json!({"monitor": "timeline"})))
        .await;
    assert!(state.get("highlight").is_none(), "{state}");
    // A node that is not there shows the picture as it is.
    slow(
        &mut client,
        "viewer.highlight",
        json!({"mode": "bw", "grade": timeline_grade, "node": "n999"}),
    )
    .await;
    shows(&mut client, with_n3).await;

    // Refused: the source monitor, a mode on without its node, a mode that is not one.
    for (params, reason) in [
        (
            json!({"monitor": "source", "mode": "bw", "grade": timeline_grade, "node": n1}),
            Some("monitor"),
        ),
        (json!({"mode": "bw", "grade": timeline_grade}), Some("node")),
        (json!({"mode": "difference"}), None),
    ] {
        let error = slow_error(&mut client, "viewer.highlight", params.clone()).await;
        assert_eq!(error["code"], -32602, "{params}: {error}");
        if let Some(reason) = reason {
            assert_eq!(error["data"]["reason"], reason, "{params}: {error}");
        }
    }
}

#[tokio::test]
async fn a_headless_engine_has_no_highlight_and_samples_a_key_off_screen() {
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
    let error = client
        .error("viewer.highlight", Some(json!({"mode": "off"})))
        .await;
    assert_eq!(error["code"], -32601, "{error}");
    let mut orange = clip("$c", "unused", 0, 10);
    orange["source"] = json!({"kind": "color", "color": "#c86e28ff"});
    let names = client
        .apply(json!([{"op": "add_timeline", "timeline": {
            "id": "$timeline", "name": "Headless", "width": 64, "height": 36,
            "rate": {"num": 25, "den": 1}, "sample_rate": 48000, "tracks": [
                {"id": "$v", "kind": "video", "name": "V1", "enabled": true, "locked": false,
                 "transitions": [], "clips": [orange]},
            ],
        }}]))
        .await["names"]
        .clone();
    let grade = json!({"level": "clip", "timeline": names["$timeline"], "clip": names["$c"]});
    // An orange that a key of luma from 50% keeps none of, and from 30% all of.
    for (low, key) in [(50, 0.0), (30, 1.0)] {
        let mut q = serde_json::to_value(columbina_model::Qualifier::default()).unwrap();
        q["mode"] = json!("lum");
        q["lum"]["low"] = json!(low);
        let node = add_corrector(&mut client, &grade, json!({ "qualifier": q }), "source").await;
        let sampled = slow(
            &mut client,
            "view.sample",
            json!({"timeline": names["$timeline"], "frame": 2, "x": 10.5, "y": 10.5,
                "grade": grade, "node": node}),
        )
        .await;
        assert!(
            (sampled["node"]["key"].as_f64().unwrap() - key).abs() < 1e-3,
            "{sampled}"
        );
        // The figures of the key at the node, too.
        let figures = slow(
            &mut client,
            "scopes.statistics",
            json!({"timeline": names["$timeline"], "frame": 2, "grade": grade, "node": node,
                "at": "key"}),
        )
        .await;
        assert!(
            (figures["red"]["mean"].as_f64().unwrap() - key).abs() < 1e-3,
            "{figures}"
        );
        client
            .apply(json!([{"op": "remove_node", "grade": grade, "id": node}]))
            .await;
    }
}
