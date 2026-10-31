//! Grades through the real engine: a transaction that grades a clip is seen in the timeline
//! monitor and in an export alike, its undo takes it away, and a LUT that cannot be read is told.

use std::path::Path;

use std::time::Duration;

use columbina_media::{Converter, Hardware, VideoDecoder};
use columbina_model::grade::{Composite, LayerMixer};
use columbina_model::{Corrector, Grade, Interp, Key, Link, Node, NodeKind, Param};
use serde_json::{json, Value};

use super::{clip, engine, project, project_dir, support, transport_engine_with, Client};

/// A timeline of one clip of orange, 64 × 36, loaded in the timeline monitor; its names.
async fn orange(client: &mut Client) -> Value {
    let mut colour = clip("$colour", "unused", 0, 30);
    colour["source"] = json!({"kind":"color","color":"#c86e28ff"});
    let names = client
        .apply(json!([{"op":"add_timeline", "timeline":{
            "id":"$timeline", "name":"Graded", "width":64, "height":36,
            "rate":{"num":25,"den":1}, "sample_rate":48000,
            "tracks":[{"id":"$v", "kind":"video", "name":"Picture", "enabled":true,
                "locked":false, "clips":[colour], "transitions":[]}]
        }}]))
        .await["names"]
        .clone();
    client
        .result(
            "transport.load",
            Some(json!({"monitor":"timeline", "timeline":names["$timeline"]})),
        )
        .await;
    names
}

async fn digest(client: &mut Client) -> Value {
    client
        .result(
            "transport.frame_digest",
            Some(json!({"monitor":"timeline"})),
        )
        .await
}

fn grey(pixel: &Value) -> bool {
    let [r, g, b] = [0, 1, 2].map(|i| pixel[i].as_i64().unwrap());
    (r - g).abs() <= 1 && (g - b).abs() <= 1
}

/// Export the whole timeline to `path`, and give its last progress.
async fn export(client: &mut Client, timeline: &Value, path: &Path) -> Value {
    client.take_notifications();
    let job = client
        .result(
            "render.start",
            Some(json!({"timeline":timeline, "path":path, "codec":"prores", "quality":"high"})),
        )
        .await["job"]
        .clone();
    loop {
        let message = if let Some(first) = client.notifications.first().cloned() {
            client.notifications.remove(0);
            first
        } else {
            client.message().await
        };
        if message["method"] == "render.progress"
            && message["params"]["job"] == job
            && message["params"]["state"] != "running"
        {
            return message["params"].clone();
        }
    }
}

/// The middle pixel of frame 0 of a film of 64 × 36.
fn middle(path: &Path) -> [u8; 4] {
    let mut decoder = VideoDecoder::open(path, Hardware::Off).unwrap();
    let rgba = Converter::new()
        .rgba_scaled(&decoder.decoded_at(0).unwrap(), 64, 36)
        .unwrap();
    rgba.data.as_chunks::<4>().0[64 * 18 + 32]
}

#[tokio::test]
async fn a_grade_is_seen_in_the_monitor_and_the_export_and_its_undo_takes_it_away() {
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
    let mut client = engine.client("agent").await;
    let names = orange(&mut client).await;
    let before = digest(&mut client).await;
    assert_eq!(before["pixels"][0], json!([200, 110, 40, 255]), "{before}");

    // One transaction: a corrector of saturation 0 between the source and the output.
    let mut node = Node::corrector("$grey");
    let NodeKind::Corrector(settings) = &mut node.kind else {
        unreachable!("a corrector")
    };
    settings.primaries.saturation = Param::Fixed(0.0);
    let target = json!({"level":"clip", "timeline":names["$timeline"], "clip":names["$colour"]});
    client
        .apply(
            json!([{"op":"add_node", "grade":target, "node":node, "links":[
            {"from":"source", "to":"$grey", "kind":"rgb", "input":0},
            {"from":"$grey", "to":"output", "kind":"rgb", "input":0}]}]),
        )
        .await;
    let graded = digest(&mut client).await;
    for pixel in graded["pixels"].as_array().unwrap() {
        assert!(grey(pixel), "{graded}");
    }
    let film = output.path().join("graded.mov");
    let done = export(&mut client, &names["$timeline"], &film).await;
    assert_eq!(done["state"], "done", "{done}");
    assert!(done.get("missing_luts").is_none(), "{done}");
    let exported = middle(&film);
    let shown = &graded["pixels"][0];
    assert!(grey(&json!(exported)), "{exported:?}");
    for i in 0..3 {
        let shown = u8::try_from(shown[i].as_u64().unwrap()).unwrap();
        assert!(
            exported[i].abs_diff(shown) <= 2,
            "the export {exported:?}, the monitor {graded}"
        );
    }

    // A LUT that cannot be read is told in the state of the monitor and in the export's
    // progress, and the node goes on without it.
    client.take_notifications();
    let id = client.result("project.get", None).await["project"]["timelines"][0]["tracks"][0]
        ["clips"][0]["grade"]["nodes"][0]["id"]
        .clone();
    client
        .apply(json!([{"op":"set_node", "grade":target, "id":id,
            "params":{"lut":"looks/none.cube"}}]))
        .await;
    let (state, _) = client
        .until_state("the missing LUT told", |state| {
            state["monitor"] == "timeline" && state.get("missing_luts").is_some()
        })
        .await;
    assert_eq!(
        state["missing_luts"][0]["name"], "looks/none.cube",
        "{state}"
    );
    assert_eq!(digest(&mut client).await["hash"], graded["hash"]);
    let film = output.path().join("missing.mov");
    let done = export(&mut client, &names["$timeline"], &film).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["missing_luts"][0]["name"], "looks/none.cube", "{done}");

    // Each undo takes a step away; the colour comes back as it was.
    client.take_notifications();
    client.result("history.undo", None).await;
    client.result("history.undo", None).await;
    let undone = digest(&mut client).await;
    assert_eq!(undone["hash"], before["hash"], "{undone}");
    let (state, _) = client
        .until_state("no LUT missing", |state| {
            state["monitor"] == "timeline" && state.get("missing_luts").is_none()
        })
        .await;
    assert_eq!(state["shown"], 0);
}

/// A grade of six nodes: three serial correctors, two of them in parallel into a layer mixer,
/// and one after; its contrast keyed over the first 90 frames of the media.
fn six_nodes(seed: f64) -> Grade {
    let node = |id: &str, set: &dyn Fn(&mut Corrector)| {
        let mut node = Node::corrector(id);
        if let NodeKind::Corrector(settings) = &mut node.kind {
            set(settings);
        }
        node
    };
    let contrast = Param::Keys(vec![
        Key {
            frame: 0,
            value: 1.0,
            interp: Interp::Linear,
        },
        Key {
            frame: 90,
            value: 1.3 + seed * 0.1,
            interp: Interp::Linear,
        },
    ]);
    let mut grade = Grade {
        nodes: vec![
            node("primary", &|c| {
                c.primaries.contrast = contrast.clone();
                c.primaries.lift.y = Param::Fixed(0.02);
                c.curves.y.points = vec![[0.0, 0.0], [0.3, 0.25], [0.7, 0.75], [1.0, 1.0]];
            }),
            node("saturated", &|c| {
                c.primaries.saturation = Param::Fixed(60.0);
                c.hsl.hue_vs_sat = vec![[0.0, 0.2], [0.3, 0.0], [0.6, -0.2]];
                c.soft_clip.high_soft.r = Param::Fixed(20.0);
            }),
            node("shadows", &|c| c.log.shadow.b = Param::Fixed(0.1)),
            node("cool", &|c| {
                c.primaries.temperature = Param::Fixed(-600.0);
                c.key.output.gain = Param::Fixed(0.7);
            }),
            Node {
                id: "mix".into(),
                label: String::new(),
                enabled: true,
                kind: NodeKind::LayerMixer(LayerMixer {
                    inputs: 2,
                    composite: Composite::Overlay,
                }),
            },
            node("finish", &|c| {
                c.hsl.sat_vs_lum = vec![[0.0, 0.0], [1.0, 0.1]];
            }),
        ],
        links: vec![
            Link::rgb("source", "primary"),
            Link::rgb("primary", "saturated"),
            Link::rgb("saturated", "shadows"),
            Link::rgb("saturated", "cool"),
            Link::rgb("shadows", "mix"),
            Link {
                input: 1,
                ..Link::rgb("cool", "mix")
            },
            Link::rgb("mix", "finish"),
            Link::rgb("finish", "output"),
        ],
    };
    grade.sort_links();
    grade
}

/// Play a timeline of 4K 30p of three layers of the sample, graded or not, for 2.5 s from its
/// first frame: how fast the playhead moves, how many frames were late, and of how many.
async fn plays(client: &mut Client, scale: &Value, graded: bool) -> (f64, i64, i64) {
    let tracks: Vec<Value> = (0..3_u8)
        .map(|i| {
            // 85 frames of 30p of the 60p sample.
            let mut layer = clip(&format!("$c{i}"), "m1", 0, 85);
            layer["source"]["in"] = json!({"ticks": 0, "scale": scale});
            if graded {
                layer["grade"] = json!(six_nodes(f64::from(i)));
            }
            if i == 1 {
                layer["transform"] =
                    json!({"x":0.2, "y":0.1, "scale":0.5, "rotation":10.0, "opacity":0.8});
            } else if i == 2 {
                layer["transform"]["opacity"] = json!(0.5);
            }
            json!({"id":format!("$v{i}"), "kind":"video", "name":format!("V{i}"),
                "enabled":true, "locked":false, "clips":[layer], "transitions":[]})
        })
        .collect();
    let mut over = Node::corrector("look");
    if let NodeKind::Corrector(settings) = &mut over.kind {
        settings.primaries.contrast = Param::Fixed(1.1);
        settings.primaries.saturation = Param::Fixed(55.0);
    }
    let mut timeline = json!({
        "id":"$timeline", "name":"4K", "width":3840, "height":2160,
        "rate":{"num":30,"den":1}, "sample_rate":48000, "tracks":tracks});
    if graded {
        timeline["grade"] = json!(Grade {
            nodes: vec![over],
            links: vec![Link::rgb("source", "look"), Link::rgb("look", "output")],
        });
    }
    let names = client
        .apply(json!([{"op":"add_timeline", "timeline":timeline}]))
        .await["names"]
        .clone();
    client
        .result(
            "transport.load",
            Some(json!({"monitor":"timeline", "timeline":names["$timeline"]})),
        )
        .await;
    client.take_notifications();
    client
        .result(
            "transport.play",
            Some(json!({"monitor":"timeline", "speed":1})),
        )
        .await;
    let states = client.states_for(Duration::from_millis(2500)).await;
    let playing: Vec<_> = states
        .iter()
        .filter(|(state, _)| state["monitor"] == "timeline" && state["playing"] == true)
        .collect();
    let (first, then) = playing.first().expect("states while it plays");
    let (last, at) = playing.last().unwrap();
    let frames = last["frame"].as_i64().unwrap() - first["frame"].as_i64().unwrap();
    #[allow(clippy::cast_precision_loss)]
    let rate = frames as f64 / at.duration_since(*then).as_secs_f64();
    client
        .result("transport.pause", Some(json!({"monitor":"timeline"})))
        .await;
    (
        rate,
        last["late"].as_i64().unwrap(),
        last["frame"].as_i64().unwrap(),
    )
}

#[tokio::test]
#[ignore = "a measure of time: run it on its own, in release, under the lock of the card"]
async fn a_timeline_of_4k_of_three_graded_layers_and_a_graded_timeline_plays_at_30p() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let Some((_engine, mut client)) = transport_engine_with(
        dir.path(),
        &[support::Sample::Uhd],
        &["--sound", "null", "--test-hooks"],
    )
    .await
    else {
        return;
    };
    let got = project(&mut client).await;
    let scale = got["project"]["media"][0]["facts"]["duration"]["scale"].clone();
    let mut late = Vec::new();
    for graded in [false, true, false, true] {
        let (rate, frames_late, frames) = plays(&mut client, &scale, graded).await;
        eprintln!(
            "4K 30p, three layers of HEVC Main 10{}: the playhead moves at {rate:.1} frames a \
             second; {frames_late} of {frames} frames late",
            if graded {
                ", each graded by six nodes, and the timeline by one"
            } else {
                ""
            }
        );
        assert!(rate > 29.0, "{rate}");
        late.push((graded, frames_late));
    }
    let late_of = |graded: bool| -> i64 {
        late.iter()
            .filter(|(of, _)| *of == graded)
            .map(|(_, late)| late)
            .sum()
    };
    // Late frames are those whose picture was not decoded in time, which grades do not make
    // slower; a frame that took too long to draw would move the playhead more slowly.
    assert!(late_of(true) <= late_of(false) + 2, "{late:?}");
}
