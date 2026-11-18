//! T24: a window tracked through the engine on a clip of known motion, one step to undo, and
//! stopped, resumed, or ended by an undo; the Tracker palette's edits.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use columbina_track::{Mat3, TrackData};
use serde_json::{json, Value};

use super::{clip, engine, project, project_dir, support, Client, WAIT};

#[path = "../../../track/tests/support/mod.rs"]
pub(super) mod scene;

const W: usize = 256;
const H: usize = 192;

/// A clip of `truths.len()` frames of a textured scene moved by each truth, losslessly encoded.
fn synthetic(dir: &Path, truths: &[Mat3]) -> Option<PathBuf> {
    let ffmpeg = support::ffmpeg_program()?;
    let scene = scene::Scene::new(1, W, H);
    let path = dir.join("moving.mkv");
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
    for truth in truths {
        let luma = scene::render(&scene, W, H, truth, scene::Look::default(), None);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let bytes: Vec<u8> = luma
            .data()
            .iter()
            .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
            .collect();
        std::io::Write::write_all(&mut stdin, &bytes).unwrap();
    }
    drop(stdin);
    assert!(child.wait().unwrap().success());
    Some(path)
}

/// The motion of each frame: a pan, a turn and a zoom about the middle, `pace` of it a frame.
fn truths(n: i32, pace: f64) -> Vec<Mat3> {
    (0..n)
        .map(|k| {
            let k = f64::from(k) * pace;
            scene::similarity(
                [128.0, 96.0],
                1.0 + 0.004 * k,
                0.01 * k,
                [1.5 * k, -0.8 * k],
            )
        })
        .collect()
}

struct Fixture {
    client: Client,
    target: Value,
    node: Value,
    _engine: super::Engine,
    _dir: tempfile::TempDir,
}

/// A timeline of the clip, its grade a corrector with a Linear window over the middle of the
/// frame (x 80–176, y 48–144) and a second, untracked, Circular window.
async fn fixture(frames: i32, pace: f64) -> Option<Fixture> {
    let dir = project_dir();
    let path = synthetic(dir.path(), &truths(frames, pace))?;
    let engine = engine(dir.path(), &["--no-proxies", "--sound", "null"]).await;
    let mut client = engine.client("desktop").await;
    let media = client
        .result("media.import", Some(json!({"paths": [path]})))
        .await["added"][0]
        .clone();
    let scale = project(&mut client).await["project"]["media"][0]["facts"]["duration"]["scale"]
        .as_i64()
        .unwrap();
    let mut c = clip("$clip", media.as_str().unwrap(), 0, i64::from(frames));
    c["source"]["in"] = json!({"ticks": 0, "scale": scale});
    let names = client
        .apply(json!([{"op":"add_timeline","timeline":{
            "id":"$timeline","name":"Track","width":W,"height":H,
            "rate":{"num":25,"den":1},"sample_rate":48000,
            "tracks":[{"id":"$v","name":"V1","kind":"video","enabled":true,"locked":false,
                "clips":[c],"transitions":[]}]}}]))
        .await["names"]
        .clone();
    let target = json!({"level":"clip","timeline":names["$timeline"],"clip":names["$clip"]});
    let mut node = columbina_model::Node::corrector("$node");
    let columbina_model::NodeKind::Corrector(c) = &mut node.kind else {
        unreachable!()
    };
    c.windows.push(columbina_model::grade::Window::new(
        "w1",
        columbina_model::grade::WindowShape::linear(),
    ));
    c.windows.push(columbina_model::grade::Window::new(
        "w2",
        columbina_model::grade::WindowShape::circular(),
    ));
    let added = client
        .apply(json!([{"op":"add_node","grade":target,"node":node,"links":[
            {"from":"source","to":"$node","kind":"rgb","input":0},
            {"from":"$node","to":"output","kind":"rgb","input":0}]}]))
        .await;
    Some(Fixture {
        client,
        target,
        node: added["names"]["$node"].clone(),
        _engine: engine,
        _dir: dir,
    })
}

async fn window(client: &mut Client, id: &str) -> Value {
    let project = project(client).await;
    let clip = &project["project"]["timelines"][0]["tracks"][0]["clips"][0];
    clip["grade"]["nodes"][0]["params"]["windows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["id"] == id)
        .unwrap()
        .clone()
}

async fn revision(client: &mut Client) -> Value {
    project(client).await["revision"].clone()
}

impl Fixture {
    async fn start(&mut self, frame: i64, direction: &str) -> Value {
        let revision = revision(&mut self.client).await;
        self.client
            .result(
                "track.start",
                Some(json!({"grade":self.target,"node":self.node,"window":"w1",
                    "revision":revision,"frame":frame,"direction":direction})),
            )
            .await
    }

    async fn edit(&mut self, window: &str, edit: Value) -> Value {
        self.client
            .result(
                "track.edit",
                Some(json!({"grade":self.target,"node":self.node,"window":window,"edit":edit})),
            )
            .await
    }
}

/// The notifications up to the end of the track `job`, and that end.
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

fn data_of(window: &Value) -> TrackData {
    serde_json::from_value(window["tracking"]["data"].clone()).unwrap()
}

/// The worst distance, over `from..=to`, between where the track puts the window's corners and
/// where the scene's motion since `start` (where the window was placed) does.
fn worst(data: &TrackData, truths: &[Mat3], start: i64, from: i64, to: i64) -> f64 {
    let corners = scene::corners(80.0, 48.0, 176.0, 144.0);
    let at = |f: i64| &truths[usize::try_from(f).unwrap()];
    let placed = at(start).inverse().unwrap();
    (from..=to)
        .map(|f| scene::error_at(data, f, &at(f).mul(&placed), &corners))
        .fold(0.0, f64::max)
}

#[tokio::test]
async fn a_window_follows_a_pan_turn_and_zoom_tracked_through_the_engine_as_one_undo_step() {
    let Some(mut f) = fixture(16, 1.0).await else {
        return;
    };
    let started = f.start(0, "forward").await;
    assert_eq!(started["first"], 0);
    assert_eq!(started["last"], 15);
    assert_eq!(started["frame_size"], json!([W, H]));
    let (seen, end) = until_ended(&mut f.client, &started["job"]).await;
    assert_eq!(end["phase"], "done", "{end}");
    assert_eq!(end["frames"], 15);
    assert_eq!(end["tracked"], json!([0, 15]));
    // The window was written while it went on: the viewer and palette saw it follow.
    assert!(seen.iter().any(|m| m["method"] == "project.changed"));
    let w1 = window(&mut f.client, "w1").await;
    let data = data_of(&w1);
    let err = worst(&data, &truths(16, 1.0), 0, 0, 15);
    // The core's own bound on noiseless frames is a twentieth of a pixel; these went through
    // 8 bits and a codec's colour conversion.
    assert!(err < 0.15, "{err}");
    // The untracked window is as it was, and the tracked one keeps its own transform.
    assert!(window(&mut f.client, "w2").await.get("tracking").is_none());
    assert_eq!(w1["transform"]["pan"], 50.0);
    // One step to undo takes the whole track away.
    f.client.result("history.undo", None).await;
    assert!(window(&mut f.client, "w1").await.get("tracking").is_none());
    f.client.result("history.redo", None).await;
    assert_eq!(data_of(&window(&mut f.client, "w1").await), data);

    // The palette's edits, each one step to undo.
    f.edit(
        "w1",
        json!({"kind":"clear","from":5,"to":9,"components":["pan"]}),
    )
    .await;
    let cleared = data_of(&window(&mut f.client, "w1").await);
    assert!(cleared
        .curve(columbina_track::Component::Pan)
        .unwrap()
        .get(&7)
        .is_none());
    assert!(cleared
        .curve(columbina_track::Component::Tilt)
        .unwrap()
        .get(&7)
        .is_some());
    let line = data.value_at(columbina_track::Component::Pan, 4)
        + (data.value_at(columbina_track::Component::Pan, 10)
            - data.value_at(columbina_track::Component::Pan, 4))
            * 3.0
            / 6.0;
    assert!((cleared.value_at(columbina_track::Component::Pan, 7) - line).abs() < 1e-9);
    f.edit("w1", json!({"kind":"mode","mode":"frame"})).await;
    assert_eq!(
        window(&mut f.client, "w1").await["tracking"]["mode"],
        "frame"
    );
    // Add Keyframe keys the tracker and the window's corners there; Delete Keyframe takes both.
    let shape = window(&mut f.client, "w1").await["shape"].clone();
    f.edit("w1", json!({"kind":"add_key","frame":8})).await;
    let keyed = window(&mut f.client, "w1").await;
    assert_eq!(
        data_of(&keyed).keys().keys().copied().collect::<Vec<_>>(),
        vec![8]
    );
    assert_eq!(keyed["shape"]["points"][0]["x"]["keys"][0]["frame"], 8);
    f.edit("w1", json!({"kind":"delete_key","frame":8})).await;
    let unkeyed = window(&mut f.client, "w1").await;
    assert!(data_of(&unkeyed).keys().is_empty());
    assert_eq!(unkeyed["shape"], shape);
    // Paste: the second window moves as the first does about its own centre.
    f.edit("w2", json!({"kind":"paste","from":"w1"})).await;
    let pasted = data_of(&window(&mut f.client, "w2").await);
    let corners = scene::corners(80.0, 48.0, 176.0, 144.0);
    for frame in [0, 3, 15] {
        let d = pasted
            .track_matrix(frame)
            .distance_on(&cleared.track_matrix(frame), &corners);
        assert!(d < 1e-6, "{frame}: {d}");
    }
    f.edit("w1", json!({"kind":"reset"})).await;
    assert!(data_of(&window(&mut f.client, "w1").await).is_empty());
    let error = f
        .client
        .error(
            "track.edit",
            Some(json!({"grade":f.target,"node":f.node,"window":"w1",
                "edit":{"kind":"paste","from":"w1"}})),
        )
        .await;
    assert_eq!(error["data"]["reason"], "nothing_to_paste");
}

#[tokio::test]
async fn a_stopped_track_keeps_what_it_tracked_and_resumes_from_there_backward_too() {
    let Some(mut f) = fixture(120, 0.25).await else {
        return;
    };
    let truths = truths(120, 0.25);
    let started = f.start(60, "forward").await;
    let job = started["job"].clone();
    first_progress(&mut f.client, &job).await;
    f.client
        .result("track.stop", Some(json!({"job":job})))
        .await;
    let (_, end) = until_ended(&mut f.client, &job).await;
    assert_eq!(end["phase"], "stopped", "{end}");
    assert_eq!(end["reason"], "stop");
    let reached = end["tracked"][1].as_i64().unwrap();
    assert!(reached > 60 && reached < 119, "{end}");
    let data = data_of(&window(&mut f.client, "w1").await);
    assert_eq!(data.tracked_range(), Some((60, reached)));
    assert!(worst(&data, &truths, 60, 60, reached) < 0.15);
    // Resumed from the last frame tracked, and back from the start (p. 3232).
    let started = f.start(reached, "forward").await;
    let (_, end) = until_ended(&mut f.client, &started["job"]).await;
    assert_eq!(end["phase"], "done", "{end}");
    let started = f.start(60, "reverse").await;
    let (_, end) = until_ended(&mut f.client, &started["job"]).await;
    assert_eq!(end["phase"], "done", "{end}");
    let data = data_of(&window(&mut f.client, "w1").await);
    assert_eq!(data.tracked_range(), Some((0, 119)));
    let err = worst(&data, &truths, 60, 0, 119);
    assert!(err < 0.3, "{err}");
    // Three tracks, three steps.
    for _ in 0..3 {
        f.client.result("history.undo", None).await;
    }
    assert!(window(&mut f.client, "w1").await.get("tracking").is_none());
}

#[tokio::test]
async fn an_undo_while_tracking_ends_the_track_and_takes_away_what_it_wrote() {
    let Some(mut f) = fixture(120, 0.25).await else {
        return;
    };
    let started = f.start(0, "forward").await;
    let job = started["job"].clone();
    // A second track cannot start while one goes on.
    let revision = revision(&mut f.client).await;
    let busy = f
        .client
        .error(
            "track.start",
            Some(
                json!({"grade":f.target,"node":f.node,"window":"w2","revision":revision,
                "frame":0,"direction":"forward"}),
            ),
        )
        .await;
    assert_eq!(busy["data"]["reason"], "busy");
    let seen = first_progress(&mut f.client, &job).await;
    assert!(
        !seen.iter().any(|m| m["method"] == "track.state"),
        "the track ended before it could be undone: {seen:?}"
    );
    f.client.result("history.undo", None).await;
    let (_, end) = until_ended(&mut f.client, &job).await;
    assert_eq!(end["phase"], "stopped", "{end}");
    assert_eq!(end["reason"], "stale_revision", "{end}");
    assert!(window(&mut f.client, "w1").await.get("tracking").is_none());
    // Untrackable requests are refused before anything starts.
    let revision = super::project(&mut f.client).await["revision"].clone();
    let outside = f
        .client
        .error(
            "track.start",
            Some(
                json!({"grade":f.target,"node":f.node,"window":"w1","revision":revision,
                "frame":500,"direction":"forward"}),
            ),
        )
        .await;
    assert_eq!(outside["data"]["reason"], "frame");
    let point = f
        .client
        .error(
            "track.start",
            Some(
                json!({"grade":f.target,"node":f.node,"window":"w1","revision":revision,
                "frame":0,"direction":"forward","tracker":"point"}),
            ),
        )
        .await;
    assert_eq!(point["data"]["reason"], "no_points");
}

#[tokio::test]
async fn the_point_tracker_follows_its_crosshairs_through_the_engine() {
    let Some(mut f) = fixture(16, 0.4).await else {
        return;
    };
    // Four crosshairs on the corners of the window's middle, in normalized picture coordinates.
    let points: Vec<[f64; 2]> = [[100.0, 70.0], [156.0, 70.0], [156.0, 122.0], [100.0, 122.0]]
        .iter()
        .map(|p| [(p[0] + 0.5) / 256.0, (p[1] + 0.5) / 192.0])
        .collect();
    f.edit("w1", json!({"kind":"crosshairs","points":points}))
        .await;
    let revision = revision(&mut f.client).await;
    let started = f
        .client
        .result(
            "track.start",
            Some(
                json!({"grade":f.target,"node":f.node,"window":"w1","revision":revision,
                "frame":0,"direction":"forward","tracker":"point"}),
            ),
        )
        .await;
    let (_, end) = until_ended(&mut f.client, &started["job"]).await;
    assert_eq!(end["phase"], "done", "{end}");
    let data = data_of(&window(&mut f.client, "w1").await);
    assert_eq!(data.points().len(), 4);
    let err = worst(&data, &truths(16, 0.4), 0, 0, 15);
    assert!(err < 0.3, "{err}");
}
