//! T18a: the stabilizer of a clip through the engine, by a client that is not the desktop, on a
//! clip of a textured picture shaken by known shifts of whole pixels: the motion measured and
//! written as the analysis of its stabilizer, one step to undo; the picture held still where
//! the frames are drawn off screen; a measure stopped, which writes nothing; and the measures
//! refused.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::{clip, engine, project, project_dir, support, Client, WAIT};

/// The size of the frames, which are measured as they are: they have fewer than 720 lines.
const W: i64 = 256;
const H: i64 = 192;

/// How far the picture of a frame is moved, across and down, in whole pixels, frame after
/// frame and again: the first frame is not moved, and the fourth is moved 5 pixels right.
const SHAKES: [[i64; 2]; 8] = [
    [0, 0],
    [2, -1],
    [4, -1],
    [5, 0],
    [3, 2],
    [1, 3],
    [-1, 2],
    [-1, 0],
];

/// How far the picture of frame `k` is moved.
fn shake(k: i64) -> [i64; 2] {
    SHAKES[usize::try_from(k).unwrap() % SHAKES.len()]
}

fn to_f64(value: i64) -> f64 {
    f64::from(i32::try_from(value).unwrap())
}

/// The bright square of the picture, as it lies in the first frame: from (116, 84), 24 pixels
/// a side, about the middle of the frame.
const SQUARE: [i64; 3] = [116, 84, 24];

/// The luma of the point `(x, y)` of the picture: the square, and a texture of three waves
/// about it, darker than the square everywhere.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a value from 0.07 to 0.83 made a byte"
)]
fn picture(x: i64, y: i64) -> u8 {
    let [left, top, side] = SQUARE;
    if (left..left + side).contains(&x) && (top..top + side).contains(&y) {
        return 240;
    }
    let (u, v) = (to_f64(x), to_f64(y));
    let value = 0.45
        + 0.15 * (0.35 * u + 0.2 * v).sin()
        + 0.15 * (-0.25 * u + 0.4 * v).sin()
        + 0.08 * (0.13 * u - 0.07 * v + 1.0).sin();
    (value * 255.0).round() as u8
}

/// A clip of `frames` frames of the picture shaken by [`SHAKES`], losslessly encoded.
fn synthetic(dir: &Path, frames: i64) -> Option<PathBuf> {
    let ffmpeg = support::ffmpeg_program()?;
    let path = dir.join("shaken.mkv");
    let mut child = std::process::Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "rawvideo"])
        .args([
            "-pix_fmt",
            "gray",
            "-s",
            &format!("{W}x{H}"),
            "-r",
            "25",
            "-i",
            "-",
        ])
        .args(["-c:v", "ffv1", "-pix_fmt", "gray"])
        .arg(&path)
        .stdin(Stdio::piped())
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take().unwrap();
    for k in 0..frames {
        let [across, down] = shake(k);
        let mut bytes = Vec::with_capacity(usize::try_from(W * H).unwrap());
        for y in 0..H {
            for x in 0..W {
                bytes.push(picture(x - across, y - down));
            }
        }
        std::io::Write::write_all(&mut stdin, &bytes).unwrap();
    }
    drop(stdin);
    assert!(child.wait().unwrap().success());
    Some(path)
}

/// The controls that hold the picture where it was at the first frame: Translation, Camera
/// Lock, no Zoom.
fn locked() -> Value {
    json!({"method": "translation", "camera_lock": true, "zoom": false,
        "cropping_ratio": 0.5, "smooth": 0.25, "strength": 1.0})
}

struct Fixture {
    client: Client,
    timeline: Value,
    clip: Value,
    /// A clip of a colour, over the other.
    colour: Value,
    _engine: super::Engine,
    _dir: tempfile::TempDir,
}

/// A timeline of [`W`] × [`H`] of the clip of `frames` frames of the picture shaken, its
/// stabilizer given the controls of [`locked`], and over it a clip of a colour that is off;
/// and a client that is the agent.
async fn fixture(frames: i64) -> Option<Fixture> {
    let dir = project_dir();
    let path = synthetic(dir.path(), frames)?;
    let engine = engine(dir.path(), &["--no-proxies", "--sound", "null"]).await;
    let mut client = engine.client("agent").await;
    let media = client
        .result("media.import", Some(json!({"paths": [path]})))
        .await["added"][0]
        .clone();
    let scale = project(&mut client).await["project"]["media"][0]["facts"]["duration"]["scale"]
        .as_i64()
        .unwrap();
    let mut shaken = clip("$clip", media.as_str().unwrap(), 0, frames);
    shaken["source"]["in"] = json!({"ticks": 0, "scale": scale});
    let mut colour = clip("$colour", "unused", 0, frames);
    colour["source"] = json!({"kind": "color", "color": "#bf0000ff"});
    colour["enabled"] = json!(false);
    let tracks: Vec<Value> = [shaken, colour]
        .into_iter()
        .enumerate()
        .map(|(index, clip)| {
            json!({"id": format!("$v{index}"), "name": format!("V{index}"), "kind": "video",
                "enabled": true, "locked": false, "clips": [clip], "transitions": []})
        })
        .collect();
    let names = client
        .apply(json!([{"op": "add_timeline", "timeline": {
            "id": "$timeline", "name": "Shaken", "width": W, "height": H,
            "rate": {"num": 25, "den": 1}, "sample_rate": 48000, "tracks": tracks}}]))
        .await["names"]
        .clone();
    client
        .apply(
            json!([{"op": "set_stabilize", "timeline": names["$timeline"],
            "clip": names["$clip"], "value": locked()}]),
        )
        .await;
    Some(Fixture {
        client,
        timeline: names["$timeline"].clone(),
        clip: names["$clip"].clone(),
        colour: names["$colour"].clone(),
        _engine: engine,
        _dir: dir,
    })
}

impl Fixture {
    async fn revision(&mut self) -> Value {
        project(&mut self.client).await["revision"].clone()
    }

    /// The params of a measure of the shaken clip at the revision of the project, with each
    /// field of `more`.
    async fn asked(&mut self, more: &Value) -> Value {
        let revision = self.revision().await;
        let mut params =
            json!({"timeline": self.timeline, "clip": self.clip, "revision": revision});
        for (name, value) in more.as_object().unwrap() {
            params[name] = value.clone();
        }
        params
    }

    /// The stabilizer of the shaken clip, as the project has it.
    async fn stabilizer(&mut self) -> Value {
        let project = project(&mut self.client).await;
        project["project"]["timelines"][0]["tracks"][0]["clips"][0]["stabilize"].clone()
    }

    /// The colour of the frame `frame` of the timeline about `(x, y)`, drawn off screen.
    async fn sampled(&mut self, frame: i64, at: (f64, f64)) -> [f64; 3] {
        let params = json!({"timeline": self.timeline, "frame": frame, "x": at.0, "y": at.1,
            "size": 3});
        let rgb = slow(&mut self.client, "view.sample", params).await["rgb"].clone();
        [0_usize, 1, 2].map(|channel| rgb[channel].as_f64().unwrap())
    }
}

/// How long a first frame of a new shader may take in a build for tests.
const SLOW: Duration = Duration::from_secs(300);

/// The result of a method that may be long in coming, keeping the notifications before it.
async fn slow(client: &mut Client, method: &str, params: Value) -> Value {
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
            assert!(message.get("error").is_none(), "{message}");
            return message["result"].clone();
        }
        client.notifications.push(message);
    }
}

/// The notifications up to the end of the measure `job`, and that end.
async fn until_ended(client: &mut Client, job: &Value) -> (Vec<Value>, Value) {
    let deadline = tokio::time::Instant::now() + WAIT * 4;
    let mut seen = client.take_notifications();
    loop {
        if let Some(end) = seen
            .iter()
            .find(|m| m["method"] == "track.state" && &m["params"]["job"] == job)
        {
            let end = end["params"].clone();
            return (seen, end);
        }
        assert!(tokio::time::Instant::now() < deadline, "no end of {job}");
        seen.push(client.message().await);
    }
}

/// The notifications up to the first progress or the end of the measure `job`.
async fn first_progress(client: &mut Client, job: &Value) -> Vec<Value> {
    let mut seen = client.take_notifications();
    loop {
        if seen.iter().any(|m| {
            (m["method"] == "track.progress" || m["method"] == "track.state")
                && &m["params"]["job"] == job
        }) {
            return seen;
        }
        seen.push(client.message().await);
    }
}

/// Points of the frame within the square, three pixels from the middle of each of its sides,
/// as it lies in the first frame.
fn within_the_square() -> [(f64, f64); 4] {
    let [left, top, side] = SQUARE.map(to_f64);
    let (middle_x, middle_y) = (left + side / 2.0, top + side / 2.0);
    [
        (left + 3.5, middle_y + 0.5),
        (left + side - 2.5, middle_y + 0.5),
        (middle_x + 0.5, top + 3.5),
        (middle_x + 0.5, top + side - 2.5),
    ]
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "the measure, what it wrote, its undo, and the frames drawn"
)]
async fn a_shaken_picture_is_measured_held_still_and_its_analysis_is_one_step_to_undo() {
    let Some(mut f) = fixture(30).await else {
        return;
    };
    let before = f.stabilizer().await;
    assert_eq!(before, locked());
    let params = f.asked(&json!({})).await;
    // What was told of the changes that made the timeline is not of the measure.
    f.client.take_notifications();
    let started = f.client.result("track.stabilize", Some(params)).await;
    assert_eq!(
        (&started["first"], &started["last"]),
        (&json!(0), &json!(29))
    );
    assert_eq!(started["frame_size"], json!([W, H]));
    // The Method of the clip's stabilizer.
    assert_eq!(started["method"], "translation");
    let (seen, end) = until_ended(&mut f.client, &started["job"]).await;
    assert_eq!(end["phase"], "done", "{end}");
    assert_eq!(end["frames"], 30);
    assert_eq!(end["lost"], 0);
    assert_eq!(end["analysed"], json!([0, 29]));
    assert_eq!(end["method"], "translation");
    // Written at the end, and not while it went on: every client is told of the change after
    // the last progress.
    let changed = seen.iter().position(|m| m["method"] == "project.changed");
    let told = seen.iter().rposition(|m| m["method"] == "track.progress");
    assert!(changed.is_some(), "{seen:?}");
    assert!(changed > told, "{seen:?}");
    let stabilizer = f.stabilizer().await;
    let analysis = &stabilizer["analysis"];
    assert_eq!(analysis["method"], "translation");
    assert_eq!(analysis["size"], json!([W, H]));
    let path = analysis["path"].as_object().unwrap();
    assert_eq!(path.len(), 30);
    // Each frame is measured where the picture was moved: its shift in the first two numbers.
    for k in 0..30 {
        let motion = &path[&k.to_string()];
        let [across, down] = shake(k);
        let (pan, tilt) = (motion[0].as_f64().unwrap(), motion[1].as_f64().unwrap());
        assert!(
            (pan - to_f64(across)).abs() < 0.1 && (tilt - to_f64(down)).abs() < 0.1,
            "frame {k}: measured {motion}, moved by {across}, {down}"
        );
    }
    // The controls are those that the clip had.
    let mut controls = stabilizer.clone();
    controls.as_object_mut().unwrap().remove("analysis");
    assert_eq!(controls, locked());
    // One step to undo takes the analysis away, and Redo puts it back.
    f.client.result("history.undo", None).await;
    assert_eq!(f.stabilizer().await, before);
    f.client.result("history.redo", None).await;
    assert_eq!(f.stabilizer().await, stabilizer);

    // Drawn off screen, the square is where it was at the first frame at every frame.
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let mut first = Vec::new();
    for at in within_the_square() {
        first.push(f.sampled(0, at).await);
    }
    for k in [1, 3, 5, 12, 29] {
        for (at, wanted) in within_the_square().into_iter().zip(&first) {
            let seen = f.sampled(k, at).await;
            let off = (0..3)
                .map(|c| (seen[c] - wanted[c]).abs())
                .fold(0.0, f64::max);
            assert!(off < 0.03, "frame {k} at {at:?}: {seen:?}, not {wanted:?}");
        }
    }
    // Without its analysis the picture shakes: at frame 3, moved 5 pixels right, the left of
    // the square is not where it was.
    f.client.result("history.undo", None).await;
    let [left, ..] = within_the_square();
    let still = f.sampled(0, left).await;
    let moved = f.sampled(3, left).await;
    let off = (0..3)
        .map(|c| (moved[c] - still[c]).abs())
        .fold(0.0, f64::max);
    assert!(off > 0.08, "{moved:?} and {still:?}");
}

#[tokio::test]
async fn a_stopped_measure_writes_nothing_and_another_waits_for_it_to_end() {
    let Some(mut f) = fixture(600).await else {
        return;
    };
    let before = project(&mut f.client).await;
    let params = f.asked(&json!({"method": "similarity"})).await;
    f.client.take_notifications();
    let started = f.client.result("track.stabilize", Some(params)).await;
    assert_eq!(started["method"], "similarity");
    let job = started["job"].clone();
    let seen = first_progress(&mut f.client, &job).await;
    assert!(
        !seen.iter().any(|m| m["method"] == "track.state"),
        "the measure ended before it could be stopped: {seen:?}"
    );
    let progress = seen
        .iter()
        .find(|m| m["method"] == "track.progress")
        .unwrap();
    assert_eq!(progress["params"]["total"], 600);
    assert_eq!(progress["params"]["revision"], before["revision"]);
    // One measure at a time, a track among them.
    let params = f.asked(&json!({})).await;
    let busy = f.client.error("track.stabilize", Some(params)).await;
    assert_eq!(busy["data"]["reason"], "busy", "{busy}");
    f.client
        .result("track.stop", Some(json!({"job": job})))
        .await;
    let (seen, end) = until_ended(&mut f.client, &job).await;
    assert_eq!(end["phase"], "stopped", "{end}");
    assert_eq!(end["reason"], "stop");
    assert_eq!(end["analysed"], Value::Null);
    assert!(!seen.iter().any(|m| m["method"] == "project.changed"));
    // Nothing was written, and no step was made.
    assert_eq!(project(&mut f.client).await, before);
}

#[tokio::test]
async fn a_measure_of_what_cannot_be_stabilized_is_refused_before_it_starts() {
    let Some(mut f) = fixture(10).await else {
        return;
    };
    let before = project(&mut f.client).await;
    let revision = before["revision"].clone();
    let asked = |clip: &Value, more: &Value| {
        let mut params = json!({"timeline": f.timeline, "clip": clip, "revision": revision});
        for (name, value) in more.as_object().unwrap() {
            params[name] = value.clone();
        }
        params
    };
    let stale = revision.as_i64().unwrap() - 1;
    for (params, code, reason) in [
        (asked(&f.colour, &json!({})), -32001, Some("untrackable")),
        (asked(&json!("c404"), &json!({})), -32001, Some("target")),
        (
            asked(&f.clip, &json!({"revision": stale})),
            -32001,
            Some("stale_revision"),
        ),
        (asked(&f.clip, &json!({"method": "affine"})), -32602, None),
        (asked(&f.clip, &json!({"frame": 3})), -32602, None),
        (
            json!({"timeline": f.timeline, "clip": f.clip}),
            -32602,
            None,
        ),
    ] {
        let error = f
            .client
            .error("track.stabilize", Some(params.clone()))
            .await;
        assert_eq!(error["code"], code, "{params}: {error}");
        if let Some(reason) = reason {
            assert_eq!(error["data"]["reason"], reason, "{params}: {error}");
        }
    }
    // Nothing was started, and nothing was written: a measure may start now.
    assert_eq!(project(&mut f.client).await, before);
    let params = f.asked(&json!({"method": "perspective"})).await;
    let started = f.client.result("track.stabilize", Some(params)).await;
    let (_, end) = until_ended(&mut f.client, &started["job"]).await;
    assert_eq!(end["phase"], "done", "{end}");
    assert_eq!(f.stabilizer().await["analysis"]["method"], "perspective");
    assert_eq!(f.stabilizer().await["method"], "perspective");
}
