//! The LUTs of the project's Lookup Tables through the real engine (RM20 pp. 151–152, p. 3330):
//! the Input LUT on pictures of media after their input transform, the Output LUT on the frame,
//! each level's 1D before its 3D; the Color Viewer LUT on the view alone, never in the frame, its
//! scopes or an export; a LUT that cannot be read told, and found once it is there. And
//! `grade.export_lut`, which bakes a grade into a cube (p. 3413).

use std::fmt::Write as _;
use std::path::Path;
use std::time::{Duration, Instant};

use columbina_media::{Converter, Hardware, VideoDecoder};
use columbina_model::{Node, NodeKind, Param};
use serde_json::{json, Value};

use super::{clip, engine, project, project_dir, Client, WAIT};

/// A 1D LUT that halves each channel.
const HALF: &str = "LUT_1D_SIZE 2\n0 0 0\n0.5 0.5 0.5\n";

/// A 3D LUT that adds a quarter to each channel: of two points, which trilinear reads exactly.
fn lift() -> String {
    let mut text = String::from("LUT_3D_SIZE 2\n");
    for b in [0.0, 1.0] {
        for g in [0.0, 1.0] {
            for r in [0.0, 1.0] {
                let _ = writeln!(text, "{} {} {}", r + 0.25, g + 0.25, b + 0.25);
            }
        }
    }
    text
}

/// The colour of the picture and of the clip of colour: orange.
const ORANGE: [u8; 3] = [200, 110, 40];

/// Half, then a quarter more: the Input or Output LUT of `HALF` and `lift`, 1D first.
fn halved_then_lifted(rgb: [u8; 3]) -> [u8; 3] {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    rgb.map(|v| (f64::from(v) * 0.5 + 63.75).round().min(255.0) as u8)
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

async fn digest(client: &mut Client) -> Value {
    client
        .result(
            "transport.frame_digest",
            Some(json!({"monitor": "timeline"})),
        )
        .await
}

async fn seek(client: &mut Client, frame: i64) {
    client
        .result(
            "transport.seek",
            Some(json!({"monitor": "timeline", "frame": frame})),
        )
        .await;
}

fn near(pixel: &Value, wanted: [u8; 3], within: i64) -> bool {
    (0..3).all(|c| (pixel[c].as_i64().unwrap() - i64::from(wanted[c])).abs() <= within)
}

/// The frame at `frame`, once its first pixel is `wanted` (the colour of a change of the
/// project's settings is made on a thread of its own, and taken a moment later).
async fn until_shown(client: &mut Client, frame: i64, wanted: [u8; 3]) -> Value {
    let deadline = Instant::now() + WAIT;
    loop {
        seek(client, frame).await;
        let shown = digest(client).await;
        if near(&shown["pixels"][0], wanted, 1) {
            return shown;
        }
        assert!(
            Instant::now() < deadline,
            "frame {frame} is not {wanted:?}: {shown}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The result of a method that may be long in coming the first time it is called: the shaders
/// of the scopes are compiled then, which FXC takes most of a minute over in a build for tests.
async fn slow_result(client: &mut Client, method: &str, params: Value) -> Value {
    use tokio::io::AsyncBufReadExt as _;
    let id = client.request(method, Some(params)).await;
    let deadline = Instant::now() + Duration::from_secs(300);
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

async fn set(client: &mut Client, settings: Value) {
    client
        .apply(json!([{"op": "set_color_settings", "settings": settings}]))
        .await;
}

/// An engine of a project whose folder holds `half.cube` and `lift.cube`, with a timeline of
/// 64 × 36 loaded: a still of orange for frames 0–9, a clip of orange for 10–19. Its client and
/// the names of the timeline.
async fn orange_timeline(dir: &Path, files: &Path) -> (super::Engine, Client, Value) {
    std::fs::write(dir.join("half.cube"), HALF).unwrap();
    std::fs::write(dir.join("lift.cube"), lift()).unwrap();
    let picture = files.join("orange.png");
    still(&picture, ORANGE);
    let engine = engine(dir, &["--no-proxies", "--sound", "null", "--test-hooks"]).await;
    let mut client = engine.client("desktop").await;
    let media = client
        .result("media.import", Some(json!({"paths": [picture]})))
        .await["added"][0]
        .clone();
    let scale = project(&mut client).await["project"]["media"][0]["facts"]["duration"]["scale"]
        .as_i64()
        .unwrap();
    let mut still = clip("$still", media.as_str().unwrap(), 0, 10);
    still["source"]["in"] = json!({"ticks": 0, "scale": scale});
    let mut colour = clip("$colour", "unused", 10, 10);
    colour["source"] = json!({"kind": "color", "color": "#c86e28ff"});
    let names = client
        .apply(json!([{"op": "add_timeline", "timeline": {
            "id": "$timeline", "name": "Looked", "width": 64, "height": 36,
            "rate": {"num": 25, "den": 1}, "sample_rate": 48000,
            "tracks": [{"id": "$v", "kind": "video", "name": "V1", "enabled": true,
                "locked": false, "clips": [still, colour], "transitions": []}],
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
    (engine, client, names)
}

/// Export the timeline to `path`, and give its last progress.
async fn export(client: &mut Client, timeline: &Value, path: &Path) -> Value {
    client.take_notifications();
    let job = client
        .result(
            "render.start",
            Some(json!({"timeline": timeline, "path": path, "codec": "prores", "quality": "high"})),
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

/// The middle pixel of frame `frame` of a film of 64 × 36.
fn middle(path: &Path, frame: i64) -> [u8; 4] {
    let mut decoder = VideoDecoder::open(path, Hardware::Off).unwrap();
    let rgba = Converter::new()
        .rgba_scaled(&decoder.decoded_at(frame).unwrap(), 64, 36)
        .unwrap();
    rgba.data.as_chunks::<4>().0[64 * 18 + 32]
}

/// Whether a pixel of an export is `wanted`, `within` so much: a film of ProRes 4:2:2 read back
/// differs by a few codes from the frame of 8 bits.
fn close(pixel: [u8; 4], wanted: [u8; 3], within: u8) -> bool {
    (0..3).all(|c| pixel[c].abs_diff(wanted[c]) <= within)
}

#[tokio::test]
async fn the_input_and_output_luts_are_done_where_the_manual_puts_them_each_1d_first() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let files = project_dir();
    let (_engine, mut client, names) = orange_timeline(dir.path(), files.path()).await;
    let plain_still = until_shown(&mut client, 0, ORANGE).await;
    let plain_colour = until_shown(&mut client, 10, ORANGE).await;
    let lifted = halved_then_lifted(ORANGE);
    // The other order would make it a half of a quarter more.
    assert_eq!(lifted, [164, 119, 84]);

    // The Input LUT: on the picture of media, not on the clip of colour, which is no source.
    set(
        &mut client,
        json!({"input_lut": {"1d": "half.cube", "3d": "lift.cube"}}),
    )
    .await;
    until_shown(&mut client, 0, lifted).await;
    seek(&mut client, 10).await;
    assert_eq!(digest(&mut client).await["hash"], plain_colour["hash"]);

    // The Output LUT: over the whole frame, both clips.
    set(
        &mut client,
        json!({"input_lut": {"1d": null, "3d": null},
            "output_lut": {"1d": "half.cube", "3d": "lift.cube"}}),
    )
    .await;
    until_shown(&mut client, 10, lifted).await;
    until_shown(&mut client, 0, lifted).await;
    let film = files.path().join("output.mov");
    let done = export(&mut client, &names["$timeline"], &film).await;
    assert_eq!(done["state"], "done", "{done}");
    for frame in [2, 15] {
        let exported = middle(&film, frame);
        assert!(close(exported, lifted, 4), "{frame}: {exported:?}");
    }

    // Each level's own pair: the 3D alone of the Input LUT and the 1D alone of the Output.
    set(
        &mut client,
        json!({"input_lut": {"1d": null, "3d": "lift.cube"},
            "output_lut": {"1d": "half.cube", "3d": null}}),
    )
    .await;
    // Input then output: (x + 1/4) / 2 on the picture, what is past 1 clipped by the 1D LUT,
    // which reads 0 to 1 (p. 225); x / 2 on the colour.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let input_then_output = ORANGE.map(|v| ((f64::from(v) + 63.75).min(255.0) * 0.5).round() as u8);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let halved = ORANGE.map(|v| (f64::from(v) * 0.5).round() as u8);
    until_shown(&mut client, 0, input_then_output).await;
    until_shown(&mut client, 10, halved).await;

    // None: the frame of DaVinci YRGB to the bit, as it was.
    set(
        &mut client,
        json!({"input_lut": {"3d": null}, "output_lut": {"1d": null}}),
    )
    .await;
    let back = until_shown(&mut client, 0, ORANGE).await;
    assert_eq!(back["hash"], plain_still["hash"]);
    let back = until_shown(&mut client, 10, ORANGE).await;
    assert_eq!(back["hash"], plain_colour["hash"]);
}

#[cfg(windows)]
#[tokio::test]
async fn the_viewer_lut_is_on_the_view_alone_never_in_the_frame_its_scopes_or_an_export() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let files = project_dir();
    let (_engine, mut client, names) = orange_timeline(dir.path(), files.path()).await;
    let parent = super::parent::Parent::shown();
    // In SDR, as the screen is read back in 8 bits.
    client
        .result("view.display", Some(json!({ "hdr": false })))
        .await;
    let attach = json!({ "monitor": "timeline", "parent": parent.hwnd.to_string(),
        "x": 0, "y": 0, "width": 960, "height": 540 });
    client.result("view.attach", Some(attach)).await;
    let plain = until_shown(&mut client, 0, ORANGE).await;
    // The green of the middle of what the view shows, once it is `wanted`.
    let until_green = |wanted: u8| {
        let parent = &parent;
        async move {
            let deadline = Instant::now() + WAIT;
            loop {
                let (grey, width) = parent.grey();
                let green = grey[270 * width + 480];
                if green.abs_diff(wanted) <= 2 {
                    return;
                }
                assert!(
                    Instant::now() < deadline,
                    "the view shows a green of {green}, not {wanted}"
                );
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    };
    until_green(ORANGE[1]).await;
    let measured = slow_result(
        &mut client,
        "scopes.statistics",
        json!({"view": "timeline"}),
    )
    .await;
    let mean = measured["green"]["mean"].as_f64().unwrap();

    set(
        &mut client,
        json!({"viewer_lut": {"1d": "half.cube", "3d": "lift.cube"}}),
    )
    .await;
    until_green(halved_then_lifted(ORANGE)[1]).await;
    // The frame, which the scopes measure and a still is grabbed of, is as it was.
    seek(&mut client, 0).await;
    assert_eq!(digest(&mut client).await["hash"], plain["hash"]);
    let measured = slow_result(
        &mut client,
        "scopes.statistics",
        json!({"view": "timeline"}),
    )
    .await;
    assert!(
        (measured["green"]["mean"].as_f64().unwrap() - mean).abs() < 1e-6,
        "{measured}"
    );
    let film = files.path().join("viewed.mov");
    let done = export(&mut client, &names["$timeline"], &film).await;
    assert_eq!(done["state"], "done", "{done}");
    let exported = middle(&film, 2);
    assert!(close(exported, ORANGE, 4), "{exported:?}");

    // Gone from the view as it is taken away.
    set(&mut client, json!({"viewer_lut": {"1d": null, "3d": null}})).await;
    until_green(ORANGE[1]).await;
}

#[tokio::test]
async fn a_lut_of_the_project_that_cannot_be_read_is_told_and_found_once_it_is_there() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let files = project_dir();
    let (_engine, mut client, names) = orange_timeline(dir.path(), files.path()).await;
    let plain = until_shown(&mut client, 0, ORANGE).await;
    std::fs::write(dir.path().join("broken.cube"), "LUT_1D_SIZE 4\n0 0 0\n").unwrap();
    set(
        &mut client,
        json!({"input_lut": {"3d": "looks/gone.cube"}, "viewer_lut": {"1d": "broken.cube"}}),
    )
    .await;
    let (state, _) = client
        .until_state("the missing LUTs told", |state| {
            state["monitor"] == "timeline"
                && state["missing_luts"]
                    .as_array()
                    .is_some_and(|m| m.len() == 2)
        })
        .await;
    let missing = &state["missing_luts"];
    assert_eq!(missing[0]["name"], "broken.cube", "{state}");
    assert!(
        missing[0]["message"].as_str().unwrap().contains("line"),
        "{state}"
    );
    assert_eq!(missing[1]["name"], "looks/gone.cube", "{state}");
    // The frame is as though they were none.
    seek(&mut client, 0).await;
    assert_eq!(digest(&mut client).await["hash"], plain["hash"]);
    let film = files.path().join("missing.mov");
    let done = export(&mut client, &names["$timeline"], &film).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["missing_luts"], *missing, "{done}");

    // Put in its place, it is found at the next change of the project, whatever it is.
    std::fs::create_dir(dir.path().join("looks")).unwrap();
    std::fs::write(dir.path().join("looks").join("gone.cube"), lift()).unwrap();
    std::fs::write(dir.path().join("broken.cube"), HALF).unwrap();
    let mut node = Node::corrector("$look");
    let NodeKind::Corrector(settings) = &mut node.kind else {
        unreachable!("a corrector")
    };
    settings.primaries.contrast = Param::Fixed(1.0);
    let timeline = json!({"level": "timeline", "timeline": names["$timeline"]});
    client.take_notifications();
    client
        .apply(
            json!([{"op": "add_node", "grade": timeline, "node": node, "links": [
            {"from": "source", "to": "$look", "kind": "rgb", "input": 0},
            {"from": "$look", "to": "output", "kind": "rgb", "input": 0}]}]),
        )
        .await;
    client
        .until_state("none missing", |state| {
            state["monitor"] == "timeline" && state.get("missing_luts").is_none()
        })
        .await;
    // A quarter more: 255 at most.
    until_shown(&mut client, 0, ORANGE.map(|v| v.saturating_add(64))).await;
}

#[tokio::test]
async fn a_grade_is_exported_as_a_cube_that_reads_back_as_the_grade() {
    let dir = project_dir();
    let files = project_dir();
    let engine = engine(dir.path(), &["--headless"]).await;
    let mut client = engine.client("agent").await;
    let mut colour = clip("$colour", "unused", 0, 10);
    colour["source"] = json!({"kind": "color", "color": "#c86e28ff"});
    let names = client
        .apply(json!([{"op": "add_timeline", "timeline": {
            "id": "$timeline", "name": "Baked", "width": 64, "height": 36,
            "rate": {"num": 25, "den": 1}, "sample_rate": 48000,
            "tracks": [{"id": "$v", "kind": "video", "name": "V1", "enabled": true,
                "locked": false, "clips": [colour], "transitions": []}],
        }}]))
        .await["names"]
        .clone();
    let grade = json!({"level": "clip", "timeline": names["$timeline"], "clip": names["$colour"]});
    let mut node = Node::corrector("$look");
    let NodeKind::Corrector(settings) = &mut node.kind else {
        unreachable!("a corrector")
    };
    settings.primaries.gain.r = Param::Fixed(1.2);
    settings.primaries.saturation = Param::Fixed(30.0);
    let node_id = client
        .apply(
            json!([{"op": "add_node", "grade": grade, "node": node, "links": [
            {"from": "source", "to": "$look", "kind": "rgb", "input": 0},
            {"from": "$look", "to": "output", "kind": "rgb", "input": 0}]}]),
        )
        .await["names"]["$look"]
        .clone();
    std::fs::create_dir(dir.path().join("looks")).unwrap();
    for size in [17, 33, 65] {
        let path = format!("looks/look{size}.cube");
        let answer = client
            .result(
                "grade.export_lut",
                Some(json!({"grade": grade, "size": size, "path": path})),
            )
            .await;
        assert_eq!(answer["name"], path.as_str(), "{answer}");
        assert_eq!(answer["nodes"], json!([node_id]), "{answer}");
        let text = std::fs::read(dir.path().join(&path)).unwrap();
        let lut = columbina_lut::parse_cube(&text).unwrap();
        assert_eq!(lut.cube().unwrap().size(), size);
        // White: its red by the gain, the rest towards its luma by the saturation.
        let white = lut.eval([1.0; 3], columbina_lut::Interpolation::Trilinear);
        assert!(
            white[0] > white[1] && (white[1] - white[2]).abs() < 1e-6,
            "{white:?}"
        );
    }
    // The LUTs are listed as any other, and may be put on a node by the name given.
    let listed = client.result("luts.list", None).await;
    let names_listed: Vec<&str> = listed["luts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|lut| lut["name"].as_str().unwrap())
        .collect();
    assert!(names_listed.contains(&"looks/look33.cube"), "{listed}");

    // Into a folder of the user's own, chosen; there already, only replaced when asked.
    let chosen = files.path().join("chosen.cube");
    let ask = |replace: bool| {
        json!({"grade": grade, "node": node_id, "size": 33,
        "path": chosen, "replace": replace})
    };
    let answer = client.result("grade.export_lut", Some(ask(false))).await;
    assert_eq!(answer["path"], json!(chosen), "{answer}");
    let error = client.error("grade.export_lut", Some(ask(false))).await;
    assert_eq!(error["code"], -32001, "{error}");
    assert_eq!(error["data"]["reason"], "exists", "{error}");
    client.result("grade.export_lut", Some(ask(true))).await;

    // Refused: a size Generate LUT has not, a path out of the project or into its cache, a
    // grade or node that is not there.
    for (params, reason) in [
        (
            json!({"grade": grade, "size": 32, "path": "a.cube"}),
            "size",
        ),
        (
            json!({"grade": grade, "size": 33, "path": "../a.cube"}),
            "path",
        ),
        (
            json!({"grade": grade, "size": 33, "path": "cache/a.cube"}),
            "path",
        ),
        (json!({"grade": grade, "size": 33, "path": "a.png"}), "path"),
        (
            json!({"grade": {"level": "timeline", "timeline": "t9"}, "size": 33, "path": "a.cube"}),
            "missing",
        ),
        (
            json!({"grade": grade, "node": "n99", "size": 33, "path": "a.cube"}),
            "missing",
        ),
    ] {
        let error = client.error("grade.export_lut", Some(params.clone())).await;
        assert_eq!(error["code"], -32602, "{params}: {error}");
        assert_eq!(error["data"]["reason"], reason, "{params}: {error}");
    }
    assert!(!dir.path().join("a.cube").exists());
}
