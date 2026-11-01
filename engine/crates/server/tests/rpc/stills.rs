//! Stills of the Gallery through the real engine: a still is the frame of the timeline monitor,
//! never the wipe it is compared with, kept as a PNG of 16 bits or an `OpenEXR` of half floats with
//! the grades that made it and their keys; its grade is applied or appended to clips, each way of
//! keyframes one undo step; albums of the project and `PowerGrade` albums shared by projects; a
//! wipe against a still by its id; and what is not the Gallery's own refused.

use std::io::Cursor;
use std::path::Path;

use columbina_media::{Converter, Hardware, PictureFormat, VideoDecoder};
use columbina_model::{Interp, Key, Node, NodeKind, Param};
use serde_json::{json, Value};

use super::{clip, project, project_dir, start_in, support, Client, Engine, Started, Stderr};

/// Start an engine on `dir` whose `PowerGrades` are in `power`.
async fn engine_with(dir: &Path, power: &Path, args: &[&str]) -> Engine {
    match start_in(
        dir,
        args,
        Stderr::Read,
        &[("COLUMBINA_POWERGRADES", power.as_os_str())],
    )
    .await
    {
        Started::Listening(engine) => *engine,
        Started::Ended { code, stderr } => panic!("the engine ended with {code}: {stderr}"),
    }
}

/// A corrector named `$name` whose saturation is keyed from 50 at frame 2 to 0 at frame 10.
fn keyed_corrector(name: &str) -> Node {
    let mut node = Node::corrector(name);
    let NodeKind::Corrector(settings) = &mut node.kind else {
        unreachable!("a corrector")
    };
    settings.primaries.saturation = Param::Keys(vec![
        Key {
            frame: 2,
            value: 50.0,
            interp: Interp::Linear,
        },
        Key {
            frame: 10,
            value: 0.0,
            interp: Interp::Linear,
        },
    ]);
    node
}

/// The links of one node between the source and the output.
fn alone(node: &str) -> Value {
    json!([
        {"from": "source", "to": node, "kind": "rgb", "input": 0},
        {"from": node, "to": "output", "kind": "rgb", "input": 0},
    ])
}

/// What the tests of stills start from: an engine whose `PowerGrades` are its own, with the
/// media of 1080p at 25 frames a second; a timeline of 64 × 36 at 25 of two clips of it, A of its
/// frames 0–19 and B of 4–23, A keyed (its saturation from 50 at frame 2 of the media to 0 at 10)
/// and the timeline graded by a node of its own; loaded in the timeline monitor at frame 6.
struct Setup {
    engine: Engine,
    client: Client,
    dir: tempfile::TempDir,
    power: tempfile::TempDir,
    names: Value,
    media: Value,
}

async fn setup() -> Option<Setup> {
    columbina_compose::gpu_for_tests()?;
    let path = support::media(support::Sample::Fhd)?
        .path
        .to_str()
        .unwrap()
        .to_owned();
    let dir = project_dir();
    let power = project_dir();
    let engine = engine_with(
        dir.path(),
        power.path(),
        &["--no-proxies", "--sound", "null", "--test-hooks"],
    )
    .await;
    let mut client = engine.client("desktop").await;
    let media = client
        .result("media.import", Some(json!({"paths": [path]})))
        .await["added"][0]
        .clone();
    let scale = project(&mut client).await["project"]["media"][0]["facts"]["duration"]["scale"]
        .as_i64()
        .unwrap();
    let mut a = clip("$a", media.as_str().unwrap(), 0, 20);
    a["source"]["in"] = json!({"ticks": 0, "scale": scale});
    let mut b = clip("$b", media.as_str().unwrap(), 20, 20);
    b["source"]["in"] = json!({"ticks": 4 * scale / 25, "scale": scale});
    let names = client
        .apply(json!([{"op": "add_timeline", "timeline": {
            "id": "$timeline", "name": "Stills", "width": 64, "height": 36,
            "rate": {"num": 25, "den": 1}, "sample_rate": 48000,
            "tracks": [{"id": "$v", "kind": "video", "name": "V1", "enabled": true,
                "locked": false, "clips": [a, b], "transitions": []}],
        }}]))
        .await["names"]
        .clone();
    let a = json!({"level": "clip", "timeline": names["$timeline"], "clip": names["$a"]});
    let timeline = json!({"level": "timeline", "timeline": names["$timeline"]});
    client
        .apply(json!([
            {"op": "add_node", "grade": a, "node": keyed_corrector("$sat"), "links": alone("$sat")},
            {"op": "add_node", "grade": timeline, "node": Node::corrector("$look"),
                "links": alone("$look")},
        ]))
        .await;
    client
        .result(
            "transport.load",
            Some(json!({"monitor": "timeline", "timeline": names["$timeline"]})),
        )
        .await;
    seek(&mut client, 6).await;
    Some(Setup {
        engine,
        client,
        dir,
        power,
        names,
        media,
    })
}

async fn seek(client: &mut Client, frame: i64) {
    client.take_notifications();
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
}

async fn digest(client: &mut Client) -> Value {
    client
        .result(
            "transport.frame_digest",
            Some(json!({"monitor": "timeline"})),
        )
        .await
}

/// The R'G'B' of 16 bits of a PNG, and its size.
fn png16(path: &Path) -> (u32, u32, Vec<u16>) {
    let bytes = std::fs::read(path).unwrap();
    let mut reader = png::Decoder::new(Cursor::new(&bytes)).read_info().unwrap();
    let info = reader.info();
    assert_eq!(
        (info.color_type, info.bit_depth),
        (png::ColorType::Rgb, png::BitDepth::Sixteen)
    );
    let (width, height) = (info.width, info.height);
    let mut data = vec![0; reader.output_buffer_size().unwrap()];
    reader.next_frame(&mut data).unwrap();
    let words = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_be_bytes(*pair))
        .collect();
    (width, height, words)
}

fn half(half: u16) -> f32 {
    let exponent = i32::from((half >> 10) & 0x1F);
    let fraction = f32::from(half & 0x3FF);
    let magnitude = match exponent {
        0 => fraction * 2.0_f32.powi(-24),
        _ => (1.0 + fraction / 1024.0) * 2.0_f32.powi(exponent - 15),
    };
    if half & 0x8000 == 0 {
        magnitude
    } else {
        -magnitude
    }
}

/// The R'G'B'A of half floats of an `OpenEXR`, as media reads it, and its size.
fn exr(path: &Path) -> (u32, u32, Vec<f32>) {
    let mut decoder = VideoDecoder::open(path, Hardware::Off).unwrap();
    let picture = Converter::new()
        .native(&decoder.decoded_at(0).unwrap())
        .unwrap();
    assert_eq!(picture.format, PictureFormat::RgbaF16);
    let row = picture.width as usize * 8;
    let values = picture
        .luma
        .chunks(picture.luma_stride)
        .take(picture.height as usize)
        .flat_map(|line| {
            line[..row]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| half(u16::from_le_bytes(*pair)))
                .collect::<Vec<_>>()
        })
        .collect();
    (picture.width, picture.height, values)
}

/// The file of a still in a gallery.
fn still_file(root: &Path, id: &str, extension: &str) -> std::path::PathBuf {
    root.join("stills").join(format!("{id}.{extension}"))
}

/// The first four pixels of a frame digest, as R'G'B' of 8 bits.
fn corners(digest: &Value) -> Vec<[f32; 3]> {
    digest["pixels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pixel| {
            #[allow(clippy::cast_precision_loss, reason = "a value of 8 bits")]
            [0, 1, 2].map(|i| pixel[i].as_i64().unwrap() as f32 / 255.0)
        })
        .collect()
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "a still from a wipe to its files, its grade and a wipe against it"
)]
async fn a_still_is_the_frame_shown_with_its_grades_even_while_a_wipe_is_on() {
    let Some(Setup {
        engine,
        mut client,
        dir,
        power: _power,
        names,
        media,
    }) = setup().await
    else {
        return;
    };
    let mut watcher = engine.client("agent").await;
    let frame = digest(&mut client).await;
    let shown = corners(&frame);

    // A wipe at 0 shows the still it compares with, all green; a still is grabbed of the
    // frame, not of that.
    let green = project_dir();
    let reference = green.path().join("green.png");
    {
        let file = std::io::BufWriter::new(std::fs::File::create(&reference).unwrap());
        let mut encoder = png::Encoder::new(file, 64, 36);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&[0, 255, 0].repeat(64 * 36))
            .unwrap();
    }
    client
        .result(
            "viewer.wipe",
            Some(json!({"position": 0.0, "reference": {"kind": "still", "path": reference}})),
        )
        .await;
    assert_eq!(
        digest(&mut client).await["pixels"][0],
        json!([0, 255, 0, 255])
    );
    let grabbed = client
        .result("still.grab", Some(json!({"view": "timeline"})))
        .await;
    let id = grabbed["id"].as_str().unwrap().to_owned();
    assert_eq!(grabbed["name"], "1.1.1", "{grabbed}");
    assert_eq!(grabbed["image"]["format"], "png", "{grabbed}");
    assert_eq!(grabbed["image"]["width"], 64);
    assert_eq!(grabbed["image"]["height"], 36);
    assert_eq!(grabbed["source"]["frame"], 6);
    assert_eq!(grabbed["source"]["clip"], names["$a"]);
    assert_eq!(grabbed["source"]["media"], media);
    assert_eq!(grabbed["source"]["record_timecode"], "00:00:00:06");
    assert_eq!(grabbed["source"]["source_timecode"], "00:00:00:06");
    assert_eq!(grabbed["grades"], json!({"clip": true, "timeline": true}));
    // Others are told the gallery changed.
    let told = loop {
        let message = watcher.message().await;
        if message["method"] == "gallery.changed" {
            break message;
        }
    };
    assert_eq!(told["params"], json!({}));

    // The PNG is the frame as the viewer has it, 16 bits for each of its 8.
    let gallery = dir.path().join("gallery");
    let (width, height, words) = png16(&still_file(&gallery, &id, "png"));
    assert_eq!((width, height), (64, 36));
    for (x, pixel) in shown.iter().enumerate() {
        for c in 0..3 {
            let stored = f32::from(words[x * 3 + c]) / 65535.0;
            assert!(
                (stored - pixel[c]).abs() <= 0.6 / 255.0,
                "pixel {x}, channel {c}: {stored} of the still, {} shown",
                pixel[c]
            );
        }
    }
    assert!(still_file(&gallery, &id, "thumb.png").is_file());

    // What the still holds: the clip's grade with its keys, on the frames of its media, and
    // the timeline's; both as they were at frame 6 (the key at or before it: 50, p. 3393).
    let still = client.result("still.get", Some(json!({"still": id}))).await;
    let node = &still["grades"]["clip"]["nodes"][0];
    assert_eq!(
        node["params"]["primaries"]["saturation"]["keys"][1]["frame"], 10,
        "{still}"
    );
    assert_eq!(
        still["frozen"]["clip"]["nodes"][0]["params"]["primaries"]["saturation"],
        50.0
    );
    assert_eq!(still["level"], "clip");
    assert_eq!(
        still["keyframes"]["clip"],
        json!({"counts": "source", "rate": {"num": 25, "den": 1}, "start": 0, "last": 19,
            "frames": [2, 10], "from_start": [2, 10]})
    );
    assert_eq!(still["keyframes"]["timeline"]["counts"], "timeline");
    assert_eq!(
        still["grades"]["timeline"]["nodes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(still["source"]["clip"]["id"], names["$a"]);
    assert_eq!(
        still["source"]["clip"]["grade"]["nodes"],
        json!([]),
        "the clip is kept without its grade, which is in grades"
    );
    assert_eq!(still["source"]["media"]["id"], media);
    assert_eq!(still["source"]["track"], 1);
    assert_eq!(still["source"]["shot"], 1);
    assert_eq!(still["album"], "a1");
    assert!(still["created"].as_str().unwrap().ends_with('Z'));

    // Of half floats, the same frame, the second clip's at frame 25: "1.2.1".
    client
        .result("viewer.wipe", Some(json!({"on": false})))
        .await;
    let as_exr = client
        .result("still.grab", Some(json!({"format": "exr"})))
        .await;
    assert_eq!(as_exr["image"]["format"], "exr");
    let (width, height, values) = exr(&still_file(&gallery, as_exr["id"].as_str().unwrap(), "exr"));
    assert_eq!((width, height), (64, 36));
    for (x, pixel) in shown.iter().enumerate() {
        for c in 0..3 {
            assert!(
                (values[x * 4 + c] - pixel[c]).abs() <= 0.6 / 255.0 + 1e-3,
                "{x} {c}: {} of the still, {} shown",
                values[x * 4 + c],
                pixel[c]
            );
        }
    }
    seek(&mut client, 25).await;
    let second = client.result("still.grab", None).await;
    assert_eq!(second["name"], "1.2.1", "{second}");
    assert_eq!(second["source"]["source_frame"], 9);

    // A grade beyond white: kept as an OpenEXR of its values, which a PNG cannot hold.
    let b = json!({"level": "clip", "timeline": names["$timeline"], "clip": names["$b"]});
    let mut bright = Node::corrector("$bright");
    if let NodeKind::Corrector(settings) = &mut bright.kind {
        settings.primaries.gain.y = Param::Fixed(8.0);
    }
    client
        .apply(json!([{"op": "add_node", "grade": b, "node": bright, "links": alone("$bright")}]))
        .await;
    seek(&mut client, 26).await;
    let beyond = client.result("still.grab", None).await;
    assert_eq!(beyond["image"]["format"], "exr", "{beyond}");
    let (_, _, values) = exr(&still_file(&gallery, beyond["id"].as_str().unwrap(), "exr"));
    assert!(
        values
            .chunks(4)
            .any(|pixel| pixel[..3].iter().any(|v| *v > 1.0)),
        "a value above 1 is kept"
    );

    // A wipe against the first still by its id, at 0: the still is shown, the frame it was of.
    seek(&mut client, 6).await;
    let wiped = client
        .result(
            "viewer.wipe",
            Some(json!({"position": 0.0, "reference": {"kind": "still", "still": id}})),
        )
        .await;
    assert_eq!(wiped["wipe"]["reference"]["still"], id.as_str(), "{wiped}");
    let against = corners(&digest(&mut client).await);
    for (x, pixel) in shown.iter().enumerate() {
        for c in 0..3 {
            assert!(
                (against[x][c] - pixel[c]).abs() <= 1.5 / 255.0,
                "{x} {c}: {against:?} against the still, {shown:?} shown"
            );
        }
    }
    let error = client
        .error(
            "viewer.wipe",
            Some(json!({"reference": {"kind": "still", "still": "s999"}})),
        )
        .await;
    assert_eq!(error["code"], -32602);
    assert_eq!(error["data"]["reason"], "still", "{error}");
    let error = client
        .error(
            "viewer.wipe",
            Some(json!({"reference": {"kind": "still", "still": "../s1"}})),
        )
        .await;
    assert_eq!(error["data"]["reason"], "still", "{error}");
    client
        .result("viewer.wipe", Some(json!({"on": false})))
        .await;
}

/// The grade of a clip as the project has it.
async fn grade_of(client: &mut Client, clip: &Value) -> Value {
    let project = project(client).await;
    project["project"]["timelines"][0]["tracks"][0]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|found| found["id"] == *clip)
        .unwrap()["grade"]
        .clone()
}

/// The frames of the keys of the saturation of a node, or its value where it has none.
fn saturation(node: &Value) -> Value {
    let saturation = &node["params"]["primaries"]["saturation"];
    match saturation.get("keys") {
        Some(keys) => json!(keys
            .as_array()
            .unwrap()
            .iter()
            .map(|key| key["frame"].clone())
            .collect::<Vec<_>>()),
        None => saturation.clone(),
    }
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "two ways of applying, three of keyframes, each one undo step"
)]
async fn a_stills_grade_is_applied_or_appended_each_way_of_keyframes_in_one_undo_step() {
    let Some(Setup {
        engine: _engine,
        mut client,
        dir: _dir,
        power: _power,
        names,
        media: _,
    }) = setup().await
    else {
        return;
    };
    let still = client.result("still.grab", None).await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let still_node = client
        .result("still.get", Some(json!({"still": still})))
        .await["grades"]["clip"]["nodes"][0]["id"]
        .clone();
    let b = json!({"level": "clip", "timeline": names["$timeline"], "clip": names["$b"]});
    client
        .apply(
            json!([{"op": "add_node", "grade": b, "node": Node::corrector("$own"),
            "links": alone("$own")}]),
        )
        .await;
    let had = grade_of(&mut client, &names["$b"]).await;
    let own = had["nodes"][0]["id"].clone();

    // A shows frames 0–19 of the media and B 4–23: by source timecode the keys stay on the
    // frames of the media (2, 10), by start frames they move by B's start (6, 14), and with
    // none the setting is what it was at the frame grabbed.
    for (keyframes, frames) in [
        ("none", json!(50.0)),
        ("source_timecode", json!([2, 10])),
        ("start_frame", json!([6, 14])),
    ] {
        for mode in ["apply", "append"] {
            client.take_notifications();
            let applied = client
                .result(
                    "still.apply",
                    Some(json!({"still": still, "to": [b], "mode": mode,
                        "keyframes": keyframes})),
                )
                .await;
            let done = applied["done"].as_array().unwrap();
            assert_eq!(done.len(), 1, "{applied}");
            assert_eq!(done[0]["op"], "set_grade");
            let grade = grade_of(&mut client, &names["$b"]).await;
            let nodes = grade["nodes"].as_array().unwrap();
            if mode == "apply" {
                // In place of B's: the still's node, its id kept.
                assert_eq!(nodes.len(), 1, "{keyframes} {mode}: {grade}");
                assert_eq!(nodes[0]["id"], still_node);
            } else {
                // After B's own, chained into the output.
                assert_eq!(nodes.len(), 2, "{keyframes} {mode}: {grade}");
                assert_eq!(nodes[0]["id"], own);
                let into_output = grade["links"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|link| link["to"] == "output")
                    .unwrap()["from"]
                    .clone();
                assert_eq!(into_output, nodes[1]["id"], "{grade}");
                // Given a new id of the engine's, as Append Node Graph gives one.
                let appended = nodes[1]["id"].as_str().unwrap();
                assert!(
                    appended.starts_with('n') && appended[1..].parse::<u64>().is_ok(),
                    "{grade}"
                );
                assert_ne!(nodes[1]["id"], still_node);
            }
            let added = nodes.last().unwrap();
            assert_eq!(saturation(added), frames, "{keyframes} {mode}: {grade}");
            // One undo step takes it all back.
            client.result("history.undo", None).await;
            assert_eq!(grade_of(&mut client, &names["$b"]).await, had);
        }
    }

    // Preserve number of nodes: B, of its own node and the still's appended, keeps its first
    // in place of the still's one.
    client
        .result(
            "still.apply",
            Some(json!({"still": still, "to": [b], "mode": "append"})),
        )
        .await;
    client
        .result(
            "still.apply",
            Some(json!({"still": still, "to": [b], "preserve_nodes": 1})),
        )
        .await;
    assert_eq!(grade_of(&mut client, &names["$b"]).await, had);
    client.result("history.undo", None).await;
    client.result("history.undo", None).await;
    assert_eq!(grade_of(&mut client, &names["$b"]).await, had);

    // The timeline's grade takes the still's timeline grade; a group's has none to take.
    let timeline = json!({"level": "timeline", "timeline": names["$timeline"]});
    let applied = client
        .result(
            "still.apply",
            Some(json!({"still": still, "to": [timeline, b], "keyframes": "none"})),
        )
        .await;
    assert!(!applied["done"].as_array().unwrap().is_empty(), "{applied}");
    assert_eq!(
        grade_of(&mut client, &names["$b"]).await["nodes"][0]["id"],
        still_node
    );
    client.result("history.undo", None).await;

    // What cannot be applied changes nothing.
    let before = project(&mut client).await["revision"].clone();
    for (params, code, reason) in [
        (json!({"still": still, "to": []}), -32001, "out_of_range"),
        (
            json!({"still": still, "to": [{"level": "clip", "timeline": names["$timeline"],
                "clip": "c999"}]}),
            -32001,
            "missing",
        ),
        (
            json!({"still": still, "to": [b], "preserve_nodes": 11}),
            -32001,
            "out_of_range",
        ),
        (json!({"still": "s999", "to": [b]}), -32602, "missing"),
        (json!({"still": "..\\s1", "to": [b]}), -32602, "missing"),
    ] {
        let error = client.error("still.apply", Some(params.clone())).await;
        assert_eq!(error["code"], code, "{params}: {error}");
        assert_eq!(error["data"]["reason"], reason, "{params}: {error}");
    }
    assert_eq!(project(&mut client).await["revision"], before);

    // Append Node to Selected Clips: A's node after B's, one step.
    let a = json!({"level": "clip", "timeline": names["$timeline"], "clip": names["$a"]});
    let from_a = grade_of(&mut client, &names["$a"]).await["nodes"][0]["id"].clone();
    let appended = client
        .result(
            "grade.append_node",
            Some(json!({"from": a, "node": from_a, "to": [b], "keyframes": "start_frame"})),
        )
        .await;
    assert_eq!(appended["done"].as_array().unwrap().len(), 1);
    let grade = grade_of(&mut client, &names["$b"]).await;
    assert_eq!(grade["nodes"].as_array().unwrap().len(), 2, "{grade}");
    assert_eq!(saturation(&grade["nodes"][1]), json!([6, 14]));
    client.result("history.undo", None).await;
    assert_eq!(grade_of(&mut client, &names["$b"]).await, had);
    // No Keyframes: the setting as it is at the playhead, frames 6 and 12 of the timeline (of
    // A's media too): the key at 2, 50, and the key at 10, 0 (p. 3393).
    for (at, value) in [(6, 50.0), (12, 0.0)] {
        client
            .result(
                "grade.append_node",
                Some(
                    json!({"from": a, "node": from_a, "to": [b], "keyframes": "none",
                    "at": at}),
                ),
            )
            .await;
        let grade = grade_of(&mut client, &names["$b"]).await;
        assert_eq!(
            saturation(&grade["nodes"][1]),
            json!(value),
            "{at}: {grade}"
        );
        client.result("history.undo", None).await;
    }
}

/// A PNG of 4 × 2 of one colour, 8 bits.
fn picture(path: &Path, rgb: [u8; 3]) {
    let file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let mut encoder = png::Encoder::new(file, 4, 2);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&rgb.repeat(8)).unwrap();
}

fn album<'a>(list: &'a Value, name: &str) -> &'a Value {
    list["albums"]
        .as_array()
        .unwrap()
        .iter()
        .find(|album| album["name"] == name)
        .unwrap_or_else(|| panic!("no album {name}: {list}"))
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "albums of two projects and their PowerGrades, from start to end"
)]
async fn albums_are_the_projects_own_and_powergrade_albums_are_shared_between_projects() {
    let power = project_dir();
    let one = project_dir();
    let two = project_dir();
    let files = project_dir();
    let red = files.path().join("red.png");
    picture(&red, [255, 0, 0]);
    let first = engine_with(one.path(), power.path(), &["--headless"]).await;
    let second = engine_with(two.path(), power.path(), &["--headless"]).await;
    let mut a = first.client("agent").await;
    let mut b = second.client("agent").await;

    // Each project starts with "Stills 1"; the PowerGrades with "PowerGrade 1", at the bottom.
    let list = a.result("gallery.list", None).await;
    let names: Vec<&str> = list["albums"]
        .as_array()
        .unwrap()
        .iter()
        .map(|album| album["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Stills 1", "PowerGrade 1"], "{list}");
    assert_eq!(album(&list, "Stills 1")["kind"], "stills");
    assert_eq!(album(&list, "PowerGrade 1")["kind"], "powergrade");

    // A picture taken in as a still, labelled, moved to a PowerGrade album of its own.
    let imported = a
        .result("still.import", Some(json!({"paths": [red]})))
        .await["added"][0]
        .clone();
    let id = imported["id"].as_str().unwrap().to_owned();
    assert_eq!(imported["album"], "a1");
    assert_eq!(imported["label"], "red");
    let looks = a
        .result("gallery.album.create", Some(json!({"kind": "powergrade"})))
        .await;
    assert_eq!(looks["name"], "PowerGrade 2", "{looks}");
    a.result(
        "gallery.album.rename",
        Some(json!({"album": looks["id"], "name": "Looks"})),
    )
    .await;
    a.result(
        "still.label",
        Some(json!({"still": id, "label": "Day exterior"})),
    )
    .await;
    let moved = a
        .result(
            "still.move",
            Some(json!({"stills": [id], "album": looks["id"]})),
        )
        .await;
    let shared = moved["moved"][&id].as_str().unwrap().to_owned();
    assert!(shared.starts_with("ps"), "{moved}");
    assert!(!one
        .path()
        .join("gallery")
        .join("stills")
        .join(format!("{id}.png"))
        .exists());

    // The other project sees it, not the first project's albums.
    let list = b.result("gallery.list", None).await;
    let looks_there = album(&list, "Looks");
    assert_eq!(looks_there["stills"][0]["id"], shared.as_str(), "{list}");
    assert_eq!(looks_there["stills"][0]["label"], "Day exterior");
    assert_eq!(album(&list, "Stills 1")["stills"], json!([]));
    let thumbnail = b
        .result(
            "gallery.thumbnail",
            Some(json!({"still": shared, "height": 1})),
        )
        .await;
    assert_eq!(
        (thumbnail["width"].as_u64(), thumbnail["height"].as_u64()),
        (Some(2), Some(1))
    );
    assert_ne!(thumbnail["png"].as_str().unwrap(), "");

    // Moved back into the second project's album: a still of it now, gone from the
    // PowerGrades.
    let back = b
        .result(
            "still.move",
            Some(json!({"stills": [shared], "album": "a1"})),
        )
        .await["moved"][&shared]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(back.starts_with('s'), "{back}");
    let list = a
        .result("gallery.list", Some(json!({"album": looks["id"]})))
        .await;
    assert_eq!(list["albums"][0]["stills"], json!([]), "{list}");

    // Exported beside its JSON, and taken in again with it.
    b.result(
        "still.export",
        Some(json!({"stills": [back], "dir": files.path(), "prefix": "Show"})),
    )
    .await;
    let exported = files.path().join(format!("Show_{back}.png"));
    assert!(exported.is_file());
    assert!(files.path().join(format!("Show_{back}.json")).is_file());
    let error = b
        .error(
            "still.export",
            Some(json!({"stills": [back], "dir": files.path(), "prefix": "Show"})),
        )
        .await;
    assert_eq!(error["data"]["reason"], "exists", "{error}");
    let again = b
        .result("still.import", Some(json!({"paths": [exported]})))
        .await["added"][0]
        .clone();
    assert_eq!(again["label"], "Day exterior", "{again}");

    // The last album of a gallery stays; another goes with its stills.
    let error = a
        .error("gallery.album.delete", Some(json!({"album": "a1"})))
        .await;
    assert_eq!(error["data"]["reason"], "last_album", "{error}");
    a.result("gallery.album.delete", Some(json!({"album": looks["id"]})))
        .await;
    b.result("still.delete", Some(json!({"stills": [back, again["id"]]})))
        .await;
    let list = b.result("gallery.list", None).await;
    assert_eq!(album(&list, "Stills 1")["stills"], json!([]), "{list}");
    assert_eq!(list["albums"].as_array().unwrap().len(), 2, "{list}");

    // Only the Gallery's own: ids that are not, paths that are not absolute, and folders
    // that a link leads elsewhere from.
    for (method, params, reason) in [
        ("still.get", json!({"still": "../s1"}), "missing"),
        ("still.get", json!({"still": "s1.json"}), "missing"),
        (
            "still.delete",
            json!({"stills": ["C:\\Windows\\win.ini"]}),
            "missing",
        ),
        ("gallery.list", json!({"album": "a1/../.."}), "missing"),
        (
            "still.move",
            json!({"stills": [again["id"]], "album": "x1"}),
            "missing",
        ),
        ("still.import", json!({"paths": ["red.png"]}), "relative"),
        (
            "still.export",
            json!({"stills": [again["id"]], "dir": "out"}),
            "relative",
        ),
        (
            "gallery.thumbnail",
            json!({"still": "s1", "height": 271}),
            "out_of_range",
        ),
    ] {
        let error = a.error(method, Some(params.clone())).await;
        assert_eq!(error["code"], -32602, "{method} {params}: {error}");
        assert_eq!(
            error["data"]["reason"], reason,
            "{method} {params}: {error}"
        );
    }
    let error = a
        .error(
            "still.export",
            Some(json!({"stills": [], "dir": files.path(), "prefix": "..\\up"})),
        )
        .await;
    assert_eq!(error["data"]["reason"], "out_of_range", "{error}");
    // Grabbing needs the timeline monitor, which a headless engine has not.
    let error = a.error("still.grab", None).await;
    assert_eq!(error["code"], -32601, "{error}");

    #[cfg(windows)]
    {
        let three = project_dir();
        let elsewhere = project_dir();
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(three.path().join("gallery"))
            .arg(elsewhere.path())
            .output()
            .unwrap();
        assert!(made.status.success(), "{made:?}");
        let third = engine_with(three.path(), power.path(), &["--headless"]).await;
        let mut c = third.client("agent").await;
        let error = c.error("gallery.list", None).await;
        assert_eq!(error["code"], -32002, "{error}");
        assert_eq!(error["data"]["reason"], "link", "{error}");
        assert_eq!(std::fs::read_dir(elsewhere.path()).unwrap().count(), 0);
        drop(c);
        drop(third);
        std::fs::remove_dir(three.path().join("gallery")).unwrap();
    }
}

/// The ids of the stills of an album as `gallery.list` tells it.
fn stills_of(album: &Value) -> Vec<String> {
    album["stills"]
        .as_array()
        .unwrap()
        .iter()
        .map(|still| still["id"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "deleting and restoring, then the memories, from start to end"
)]
async fn a_still_deleted_is_restored_where_it_was_and_memories_are_an_album_of_their_own() {
    let power = project_dir();
    let dir = project_dir();
    let files = project_dir();
    let engine = engine_with(dir.path(), power.path(), &["--headless"]).await;
    let mut a = engine.client("agent").await;
    let paths: Vec<_> = [[255, 0, 0], [0, 255, 0], [0, 0, 255]]
        .iter()
        .enumerate()
        .map(|(n, rgb)| {
            let path = files.path().join(format!("{n}.png"));
            picture(&path, *rgb);
            path
        })
        .collect();
    let added = a
        .result("still.import", Some(json!({"paths": paths})))
        .await["added"]
        .as_array()
        .unwrap()
        .iter()
        .map(|still| still["id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    let gallery = dir.path().join("gallery");

    // Deleted, the second is gone from its album and its files are in the recycle folder.
    let deleted = a
        .result("still.delete", Some(json!({"stills": [added[1]]})))
        .await;
    assert_eq!(deleted["deleted"], json!([added[1]]));
    let list = a.result("gallery.list", None).await;
    assert_eq!(
        stills_of(album(&list, "Stills 1")),
        [added[0].clone(), added[2].clone()]
    );
    assert!(!still_file(&gallery, &added[1], "png").exists());
    assert!(gallery
        .join("recycle")
        .join(format!("{}.png", added[1]))
        .is_file());
    // Restored, it is back where it was, under its id, its thumbnail with it.
    let restored = a
        .result("still.restore", Some(json!({"stills": [added[1]]})))
        .await;
    assert_eq!(restored["restored"][&added[1]], "a1", "{restored}");
    let list = a.result("gallery.list", None).await;
    assert_eq!(stills_of(album(&list, "Stills 1")), added);
    assert!(still_file(&gallery, &added[1], "png").is_file());
    a.result(
        "gallery.thumbnail",
        Some(json!({"still": added[1], "height": 1})),
    )
    .await;
    // What was not deleted cannot be restored, nor the same twice; nothing is restored then.
    a.result("still.delete", Some(json!({"stills": [added[0]]})))
        .await;
    for stills in [
        json!([added[1]]),
        json!([added[0], added[1]]),
        json!(["s99"]),
    ] {
        let error = a
            .error("still.restore", Some(json!({"stills": stills})))
            .await;
        assert_eq!(error["code"], -32602, "{stills}: {error}");
        assert_eq!(error["data"]["reason"], "missing", "{error}");
    }
    let list = a.result("gallery.list", None).await;
    assert_eq!(
        stills_of(album(&list, "Stills 1")),
        [added[1].clone(), added[2].clone()]
    );
    a.result("still.restore", Some(json!({"stills": [added[0]]})))
        .await;

    // The album of the memories: made once, not among the albums listed, apart from them.
    let memories = a
        .result("gallery.album.create", Some(json!({"kind": "memories"})))
        .await;
    assert_eq!(memories["kind"], "memories", "{memories}");
    assert_eq!(memories["name"], "Memories");
    let again = a
        .result("gallery.album.create", Some(json!({"kind": "memories"})))
        .await;
    assert_eq!(again["id"], memories["id"]);
    // A still dragged onto a memory is copied into it; the one in the album stays.
    let copied = a
        .result(
            "still.copy",
            Some(json!({"stills": [added[2]], "album": memories["id"]})),
        )
        .await;
    let copy = copied["copied"][&added[2]].as_str().unwrap().to_owned();
    assert_ne!(copy, added[2]);
    let list = a.result("gallery.list", None).await;
    let names: Vec<&str> = list["albums"]
        .as_array()
        .unwrap()
        .iter()
        .map(|album| album["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Stills 1", "PowerGrade 1"], "{list}");
    assert_eq!(stills_of(album(&list, "Stills 1")), added);
    assert_eq!(list["memories"]["id"], memories["id"], "{list}");
    assert_eq!(list["memories"]["kind"], "memories");
    assert_eq!(stills_of(&list["memories"]), std::slice::from_ref(&copy));
    // Named, it is listed; an import that names no album goes into the first shown.
    let by_id = a
        .result("gallery.list", Some(json!({"album": memories["id"]})))
        .await;
    assert_eq!(stills_of(&by_id["albums"][0]), std::slice::from_ref(&copy));
    let imported = a
        .result("still.import", Some(json!({"paths": [paths[0]]})))
        .await;
    assert_eq!(imported["added"][0]["album"], "a1", "{imported}");
    // The project's last album shown stays, however many memories; the memories' goes.
    let error = a
        .error("gallery.album.delete", Some(json!({"album": "a1"})))
        .await;
    assert_eq!(error["data"]["reason"], "last_album", "{error}");
    a.result(
        "gallery.album.delete",
        Some(json!({"album": memories["id"]})),
    )
    .await;
    assert_eq!(
        a.result("gallery.list", None).await["memories"],
        Value::Null
    );
    // Copied into the PowerGrades, a still of them under their own id.
    let shared = a
        .result(
            "still.copy",
            Some(json!({"stills": [added[0]], "album": "pa1"})),
        )
        .await["copied"][&added[0]]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(shared.starts_with("ps"), "{shared}");
    let error = a
        .error(
            "still.copy",
            Some(json!({"stills": ["s99"], "album": "a1"})),
        )
        .await;
    assert_eq!(error["data"]["reason"], "missing", "{error}");
}

#[tokio::test]
#[ignore = "measures how long a still of 4K takes: run alone, with --release"]
async fn a_still_of_4k_is_grabbed_in_a_time_measured() {
    columbina_compose::gpu_for_tests().expect("a card");
    let path = support::media(support::Sample::Fhd)
        .expect("the media")
        .path
        .to_str()
        .unwrap()
        .to_owned();
    let dir = project_dir();
    let power = project_dir();
    let engine = engine_with(
        dir.path(),
        power.path(),
        &["--no-proxies", "--sound", "null", "--test-hooks"],
    )
    .await;
    let mut client = engine.client("desktop").await;
    let media = client
        .result("media.import", Some(json!({"paths": [path]})))
        .await["added"][0]
        .clone();
    let scale = project(&mut client).await["project"]["media"][0]["facts"]["duration"]["scale"]
        .as_i64()
        .unwrap();
    let mut a = clip("$a", media.as_str().unwrap(), 0, 20);
    a["source"]["in"] = json!({"ticks": 0, "scale": scale});
    let names = client
        .apply(json!([{"op": "add_timeline", "timeline": {
            "id": "$timeline", "name": "UHD", "width": 3840, "height": 2160,
            "rate": {"num": 25, "den": 1}, "sample_rate": 48000,
            "tracks": [{"id": "$v", "kind": "video", "name": "V1", "enabled": true,
                "locked": false, "clips": [a], "transitions": []}],
        }}]))
        .await["names"]
        .clone();
    client
        .result(
            "transport.load",
            Some(json!({"monitor": "timeline", "timeline": names["$timeline"]})),
        )
        .await;
    seek(&mut client, 6).await;
    for format in ["png", "exr", "png", "exr", "png", "exr"] {
        let start = std::time::Instant::now();
        let still = client
            .result("still.grab", Some(json!({"format": format})))
            .await;
        let took = start.elapsed();
        let size = std::fs::metadata(still_file(
            &dir.path().join("gallery"),
            still["id"].as_str().unwrap(),
            format,
        ))
        .unwrap()
        .len();
        println!(
            "still.grab {format}: {:.0} ms, {:.1} MB",
            took.as_secs_f64() * 1000.0,
            f64::from(u32::try_from(size).unwrap()) / 1e6
        );
    }
    for line in super::logged(&engine, "still_grabbed") {
        println!("{line}");
    }
}
