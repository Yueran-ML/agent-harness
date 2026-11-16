//! The Fusion view of the timeline monitor through the real engine (`viewer.fusion`): the
//! picture of a node of the composition of a clip, alone, fitted and not graded, in place of the
//! frame of the timeline; told in the state of the monitor with where it lies; black where the
//! playhead is not on the clip; drawn again as the composition is edited; and the same picture
//! of a frame drawn off screen (`view.sample` and `scopes.statistics` with `fusion`), in a
//! headless engine too.

use std::time::{Duration, Instant};

use columbina_model::fusion::{Composition, Input, Kind, Link, Node, NodeKind};
use serde_json::{json, Value};

use super::{clip, engine, project_dir, Client};

/// How long a first frame of a new shader may take in a build for tests.
const SLOW: Duration = Duration::from_secs(300);

/// 75% of white, as a picture of 8 bits has it.
const BAR: f64 = 191.0 / 255.0;

/// The red of the clip that has the composition, and the teal of the clip over it.
const RED: [f64; 3] = [BAR, 0.0, 0.0];
const TEAL: [f64; 3] = [0.0, BAR, BAR];

/// The blue of the Background of the composition, and what comes out of the composition: half
/// the red of the clip and half that blue.
const BLUE: [f64; 3] = [0.0, 0.0, 1.0];
const MIXED: [f64; 3] = [BAR / 2.0, 0.0, 0.5];

/// A point of the frame of 128 × 72 that the red clip does not cover as the timeline shows it
/// (it lies in the lower right quarter), and the middle of the frame.
const CORNER: (f64, f64) = (10.5, 10.5);
const MIDDLE: (f64, f64) = (64.5, 36.5);

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

fn near(value: &Value, expected: [f64; 3], within: f64) -> bool {
    (0..3).all(|i| (value[i].as_f64().unwrap() - expected[i]).abs() <= within)
}

/// A timeline of 128 × 72 of 80 frames. On its first track, `$a`, a red colour clip over the
/// frames 0 to 49, put in the lower right quarter and halved by its grade, whose composition
/// lays it over a blue Background (`bg`) by a Blend of a half (`over`) and has a Rectangle
/// (`r`) that is not linked. Over it, `$b`, a teal colour clip over all 80 frames, which has
/// no composition and covers the frame. The names of the transaction.
async fn composed(client: &mut Client) -> Value {
    let mut red = clip("$a", "unused", 0, 50);
    red["source"] = json!({"kind": "color", "color": "#bf0000ff"});
    let quarter = json!({"x": 0.25, "y": 0.25, "scale": 0.5, "rotation": 0.0, "opacity": 1.0});
    red["transform"] = quarter;
    let mut teal = clip("$b", "unused", 0, 80);
    teal["source"] = json!({"kind": "color", "color": "#00bfbfff"});
    let tracks: Vec<Value> = [red, teal]
        .into_iter()
        .enumerate()
        .map(|(index, clip)| {
            json!({"id": format!("$v{index}"), "kind": "video", "name": format!("V{index}"),
                "enabled": true, "locked": false, "clips": [clip], "transitions": []})
        })
        .collect();
    let names = client
        .apply(json!([{"op": "add_timeline", "timeline": {
            "id": "$timeline", "name": "Composed", "width": 128, "height": 72,
            "rate": {"num": 25, "den": 1}, "sample_rate": 48000, "tracks": tracks,
        }}]))
        .await["names"]
        .clone();
    let mut behind = Node::new("bg", Kind::Background);
    if let NodeKind::Background(background) = &mut behind.kind {
        background.color.b = 1.0.into();
    }
    let mut over = Node::new("over", Kind::Merge);
    if let NodeKind::Merge(merge) = &mut over.kind {
        merge.blend = 0.5.into();
    }
    let composition = Composition {
        nodes: vec![
            Node::new("in", Kind::MediaIn),
            behind,
            over,
            Node::new("r", Kind::Rectangle),
            Node::new("out", Kind::MediaOut),
        ],
        links: vec![
            Link::new("bg", "over", Input::Background),
            Link::new("in", "over", Input::Foreground),
            Link::new("over", "out", Input::Input),
        ],
    };
    client
        .apply(json!([{"op": "set_fusion", "timeline": names["$timeline"],
            "clip": names["$a"], "value": serde_json::to_value(&composition).unwrap()}]))
        .await;
    // The grade of the clip halves it.
    let mut node = serde_json::to_value(columbina_model::Node::corrector("$node")).unwrap();
    node["params"]["primaries"]["gain"] = json!({"y": 0.5, "r": 0.5, "g": 0.5, "b": 0.5});
    let grade = json!({"level": "clip", "timeline": names["$timeline"], "clip": names["$a"]});
    client
        .apply(
            json!([{"op": "add_node", "grade": grade, "node": node, "links": [
            {"from": "source", "to": "$node", "kind": "rgb", "input": 0},
            {"from": "$node", "to": "output", "kind": "rgb", "input": 0}]}]),
        )
        .await;
    names
}

/// The view of the node `node` of the clip named `clip`, as a method takes it.
fn view(names: &Value, clip: &str, node: Option<&str>) -> Value {
    let mut view = json!({"clip": names[clip]});
    if let Some(node) = node {
        view["node"] = json!(node);
    }
    view
}

/// The Fusion view as it is told of a clip of a colour of the timeline of 128 × 72, whose
/// composition is of the size of the timeline and fills the frame.
fn told(names: &Value, clip: &str, node: Option<&str>, shown: bool) -> Value {
    let mut told = view(names, clip, node);
    told["shown"] = json!(shown);
    told["canvas"] = json!([128, 72]);
    told["size"] = json!({"width": 128, "height": 72});
    told["picture"] = json!({"x": 0.0, "y": 0.0, "width": 128.0, "height": 72.0});
    told
}

/// `view.sample` of frame `frame` of the timeline, drawn off screen, about `at`: of the frame
/// of the timeline, or with `fusion` of the picture of a node of a composition.
async fn sampled(
    client: &mut Client,
    names: &Value,
    frame: i64,
    at: (f64, f64),
    fusion: Option<Value>,
) -> Value {
    let mut params = json!({"timeline": names["$timeline"], "frame": frame,
        "x": at.0, "y": at.1, "size": 3});
    if let Some(fusion) = fusion {
        params["fusion"] = fusion;
    }
    slow(client, "view.sample", params).await
}

/// The colours that the timeline monitor shows at [`CORNER`] and at [`MIDDLE`], once they are
/// `wanted` (within 4e-3), waiting while the monitor draws.
async fn shows(client: &mut Client, wanted: [[f64; 3]; 2]) {
    let deadline = Instant::now() + SLOW;
    loop {
        let mut seen = Vec::new();
        for (x, y) in [CORNER, MIDDLE] {
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

async fn state(client: &mut Client) -> Value {
    client
        .result("transport.state", Some(json!({"monitor": "timeline"})))
        .await
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "each node, then a frame off the clip, the figures, and the refusals"
)]
async fn a_frame_drawn_off_screen_shows_the_picture_of_a_node_of_a_composition() {
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
    // A headless engine has no monitor to show the view on.
    let error = client
        .error("viewer.fusion", Some(json!({"clip": null})))
        .await;
    assert_eq!(error["code"], -32601, "{error}");
    let names = composed(&mut client).await;
    // Without `fusion`, the frame of the timeline: the clip on top.
    for at in [CORNER, MIDDLE] {
        let frame = sampled(&mut client, &names, 5, at, None).await;
        assert!(near(&frame["rgb"], TEAL, 4e-3), "{frame}");
        assert!(frame.get("fusion").is_none(), "{frame}");
    }
    // With it, the picture of the node, over the whole frame: not the other layer, and not
    // the transform or the grade of the clip. With no node, and with the MediaOut, what comes
    // out of the composition.
    for (node, wanted) in [
        (None, MIXED),
        (Some("out"), MIXED),
        (Some("over"), MIXED),
        (Some("bg"), BLUE),
        (Some("in"), RED),
    ] {
        for at in [CORNER, MIDDLE] {
            let fusion = view(&names, "$a", node);
            let picture = sampled(&mut client, &names, 5, at, Some(fusion)).await;
            assert!(near(&picture["rgb"], wanted, 4e-3), "{node:?}: {picture}");
            assert_eq!(picture["fusion"], told(&names, "$a", node, true));
            assert_eq!(picture["timeline"], names["$timeline"]);
            assert_eq!(picture["width"], 128);
            assert_eq!(picture["height"], 72);
        }
    }
    // A mask that is not linked: white where it is, and black about it.
    let mask = Some(view(&names, "$a", Some("r")));
    let inside = sampled(&mut client, &names, 5, MIDDLE, mask.clone()).await;
    assert!(near(&inside["rgb"], [1.0; 3], 4e-3), "{inside}");
    let outside = sampled(&mut client, &names, 5, CORNER, mask).await;
    assert!(near(&outside["rgb"], [0.0; 3], 4e-3), "{outside}");
    // A clip without a composition shows its source, by the nodes of the composition that a
    // clip has at first too.
    for node in [None, Some("MediaIn1"), Some("MediaOut1")] {
        let fusion = Some(view(&names, "$b", node));
        let source = sampled(&mut client, &names, 60, CORNER, fusion).await;
        assert!(near(&source["rgb"], TEAL, 4e-3), "{node:?}: {source}");
    }
    // A frame that the clip is not on: black, and the view says that it is not shown.
    let fusion = Some(view(&names, "$a", Some("bg")));
    let beyond = sampled(&mut client, &names, 60, MIDDLE, fusion).await;
    assert!(near(&beyond["rgb"], [0.0; 3], 4e-3), "{beyond}");
    assert_eq!(beyond["fusion"], told(&names, "$a", Some("bg"), false));
    // The figures of the scopes are of the same picture.
    let figures = slow(
        &mut client,
        "scopes.statistics",
        json!({"timeline": names["$timeline"], "frame": 5,
            "fusion": view(&names, "$a", Some("bg"))}),
    )
    .await;
    assert!(
        (figures["blue"]["mean"].as_f64().unwrap() - 1.0).abs() < 4e-3,
        "{figures}"
    );
    assert!(
        figures["red"]["mean"].as_f64().unwrap().abs() < 4e-3,
        "{figures}"
    );
    assert_eq!(figures["fusion"], told(&names, "$a", Some("bg"), true));
    // Refused: what the timeline has not, and a `fusion` that is not of a frame off screen.
    let frame = json!({"timeline": names["$timeline"], "frame": 5, "x": 10.5, "y": 10.5});
    let grade = json!({"level": "clip", "timeline": names["$timeline"], "clip": names["$a"]});
    for (more, reason) in [
        (json!({"fusion": {"clip": "c404"}}), "clip"),
        (json!({"fusion": view(&names, "$a", Some("n404"))}), "node"),
        (json!({"fusion": view(&names, "$b", Some("bg"))}), "node"),
        (
            json!({"fusion": view(&names, "$a", None), "grade": grade, "node": "n1"}),
            "fusion",
        ),
    ] {
        let mut params = frame.clone();
        for (name, value) in more.as_object().unwrap() {
            params[name] = value.clone();
        }
        let error = slow_error(&mut client, "view.sample", params.clone()).await;
        assert_eq!(error["code"], -32602, "{params}: {error}");
        assert_eq!(error["data"]["reason"], reason, "{params}: {error}");
    }
    let error = slow_error(
        &mut client,
        "scopes.statistics",
        json!({"timeline": names["$timeline"], "frame": 5, "fusion": {"clip": "c404"}}),
    )
    .await;
    assert_eq!(error["code"], -32602, "{error}");
    assert_eq!(error["data"]["reason"], "clip", "{error}");
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "the view on, edited, off its clip and off, then the refusals"
)]
async fn the_fusion_view_shows_the_picture_of_a_node_on_the_monitor_and_tells_where_it_lies() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let engine = engine(dir.path(), &["--no-proxies", "--sound", "null"]).await;
    let mut client = engine.client("desktop").await;
    let names = composed(&mut client).await;
    let load = json!({"monitor": "timeline", "timeline": names["$timeline"]});
    client.result("transport.load", Some(load.clone())).await;
    client.take_notifications();
    // The frame of the timeline, and no view in its state.
    shows(&mut client, [TEAL, TEAL]).await;
    assert!(state(&mut client).await.get("fusion").is_none());
    // The Background of the composition of the red clip: over the whole frame, alone.
    let on = slow(&mut client, "viewer.fusion", view(&names, "$a", Some("bg"))).await;
    let lies = told(&names, "$a", Some("bg"), true);
    assert_eq!(on, json!({"monitor": "timeline", "fusion": lies}));
    shows(&mut client, [BLUE, BLUE]).await;
    assert_eq!(state(&mut client).await["fusion"], lies);
    // The state was told with the view as it came on.
    let notes = client.take_notifications();
    assert!(
        notes
            .iter()
            .any(|note| note["method"] == "transport.state" && note["params"]["fusion"] == lies),
        "{notes:?}"
    );
    // The source of the clip, not graded and not moved; and what comes out of the composition.
    slow(&mut client, "viewer.fusion", view(&names, "$a", Some("in"))).await;
    shows(&mut client, [RED, RED]).await;
    slow(&mut client, "viewer.fusion", view(&names, "$a", None)).await;
    shows(&mut client, [MIXED, MIXED]).await;
    assert_eq!(
        state(&mut client).await["fusion"],
        told(&names, "$a", None, true)
    );
    // A frame off screen without `fusion` is of the timeline, whatever the monitor shows.
    let frame = sampled(&mut client, &names, 5, CORNER, None).await;
    assert!(near(&frame["rgb"], TEAL, 4e-3), "{frame}");
    // An edit of the composition is drawn at once: the Background made green.
    slow(&mut client, "viewer.fusion", view(&names, "$a", Some("bg"))).await;
    shows(&mut client, [BLUE, BLUE]).await;
    client
        .apply(
            json!([{"op": "set_fusion_node", "timeline": names["$timeline"],
            "clip": names["$a"], "id": "bg",
            "params": {"color": {"r": 0.0, "g": 1.0, "b": 0.0, "a": 1.0}}}]),
        )
        .await;
    shows(&mut client, [[0.0, 1.0, 0.0]; 2]).await;
    // The playhead off the clip: black, the view on and not shown; and on it again.
    let seek = |frame: i64| Some(json!({"monitor": "timeline", "frame": frame}));
    client.result("transport.seek", seek(60)).await;
    shows(&mut client, [[0.0; 3]; 2]).await;
    assert_eq!(
        state(&mut client).await["fusion"],
        told(&names, "$a", Some("bg"), false)
    );
    client.result("transport.seek", seek(5)).await;
    shows(&mut client, [[0.0, 1.0, 0.0]; 2]).await;
    assert_eq!(state(&mut client).await["fusion"], lies);
    // Refused, and the view is as it was: the source monitor, what the timeline has not, and
    // params that are not the method's.
    for (params, reason) in [
        (
            json!({"clip": names["$a"], "monitor": "source"}),
            Some("monitor"),
        ),
        (json!({"clip": "c404"}), Some("clip")),
        (view(&names, "$a", Some("n404")), Some("node")),
        (view(&names, "$b", Some("bg")), Some("node")),
        (json!({}), None),
        (json!({"node": "bg"}), None),
        (json!({"clip": names["$a"], "at": "input"}), None),
    ] {
        let error = slow_error(&mut client, "viewer.fusion", params.clone()).await;
        assert_eq!(error["code"], -32602, "{params}: {error}");
        if let Some(reason) = reason {
            assert_eq!(error["data"]["reason"], reason, "{params}: {error}");
        }
    }
    assert_eq!(state(&mut client).await["fusion"], lies);
    // A sample of the monitor takes no `fusion`: it is of what the monitor shows.
    let error = slow_error(
        &mut client,
        "view.sample",
        json!({"monitor": "timeline", "x": 10.5, "y": 10.5,
            "fusion": view(&names, "$a", None)}),
    )
    .await;
    assert_eq!(error["code"], -32602, "{error}");
    assert_eq!(error["data"]["reason"], "fusion", "{error}");
    // Off: the frame of the timeline, and the state says nothing of the view.
    let off = slow(&mut client, "viewer.fusion", json!({"clip": null})).await;
    assert_eq!(off, json!({"monitor": "timeline", "fusion": null}));
    shows(&mut client, [TEAL, TEAL]).await;
    assert!(state(&mut client).await.get("fusion").is_none());
    // A timeline that is loaded shows its frame: the view does not outlast the one before.
    slow(&mut client, "viewer.fusion", view(&names, "$a", Some("in"))).await;
    shows(&mut client, [RED, RED]).await;
    client.result("transport.load", Some(load)).await;
    shows(&mut client, [TEAL, TEAL]).await;
    assert!(state(&mut client).await.get("fusion").is_none());
}
