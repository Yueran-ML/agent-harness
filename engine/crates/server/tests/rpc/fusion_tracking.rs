//! T17b: the patterns of a Tracker of a composition followed through the engine by a client
//! that is not the desktop, on a clip of a textured square of known motion: the paths written
//! as one step to undo and holding the picture still, a track backward, a track stopped, and
//! the tracks refused.

#![allow(
    clippy::float_cmp,
    reason = "a size and an angle that a single pattern leaves as they are, to the bit"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use columbina_model::fusion::{
    Composition, Input, Kind, Link, Node, NodeKind, Pattern, Tracker, TrackerOperation,
};
use serde_json::{json, Value};

use super::{clip, engine, project, project_dir, support, Client, WAIT};

/// The size of the frames, which are tracked as they are: they have fewer than 720 lines.
const W: i64 = 256;
const H: i64 = 192;
const WIDTH: f64 = 256.0;
const HEIGHT: f64 = 192.0;

/// The side of the square, in pixels.
const SIDE: i64 = 40;

/// How far a place of a path may be from the middle of the square, in pixels of the frames.
const WITHIN: f64 = 0.25;

/// How the square moves: where its top left corner is at each frame.
type Course = fn(i64) -> [i64; 2];

/// From (60, 110), 3 pixels to the right and 2 up a frame.
fn rising(k: i64) -> [i64; 2] {
    [60 + 3 * k, 110 - 2 * k]
}

/// From (20, 70), a pixel to the right a frame for 100 frames, and back.
fn swinging(k: i64) -> [i64; 2] {
    let phase = k % 200;
    [20 + phase.min(200 - phase), 70]
}

fn to_f64(value: i64) -> f64 {
    f64::from(i32::try_from(value).unwrap())
}

/// Where the middle of the square is at frame `k` of `course`, as a place of a path: the square
/// covers the pixels from its corner, so that its middle is half its side further.
fn middle(course: Course, k: i64) -> [f64; 2] {
    let [x, y] = course(k);
    let half = to_f64(SIDE) / 2.0;
    [
        (to_f64(x) + half) / WIDTH,
        1.0 - (to_f64(y) + half) / HEIGHT,
    ]
}

/// How far apart two places are, in pixels of the frames.
fn apart(place: [f64; 2], other: [f64; 2]) -> f64 {
    ((place[0] - other[0]) * WIDTH).hypot((place[1] - other[1]) * HEIGHT)
}

/// The luma of the pixel `(x, y)` of frame `k`: a dark ground, and the square, textured within.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a value from 0.2 to 0.9 made a byte"
)]
fn luma(course: Course, k: i64, x: i64, y: i64) -> u8 {
    let [left, top] = course(k);
    let (across, down) = (x - left, y - top);
    if !(0..SIDE).contains(&across) || !(0..SIDE).contains(&down) {
        return 20;
    }
    let (across, down) = (to_f64(across), to_f64(down));
    let value =
        0.55 + 0.2 * (0.45 * across + 0.2 * down).sin() + 0.15 * (0.3 * down - 0.25 * across).cos();
    (value * 255.0).round() as u8
}

/// A clip of `frames` frames of the square moving along `course`, losslessly encoded.
fn synthetic(dir: &Path, frames: i64, course: Course) -> Option<PathBuf> {
    let ffmpeg = support::ffmpeg_program()?;
    let path = dir.join("square.mkv");
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
        let mut bytes = Vec::with_capacity(usize::try_from(W * H).unwrap());
        for y in 0..H {
            for x in 0..W {
                bytes.push(luma(course, k, x, y));
            }
        }
        std::io::Write::write_all(&mut stdin, &bytes).unwrap();
    }
    drop(stdin);
    assert!(child.wait().unwrap().success());
    Some(path)
}

/// A pattern on the square at frame `k` of `course`, as a client puts one: its box a little
/// larger than the square, and a search area that it moves well within.
fn on_square(course: Course, k: i64) -> Pattern {
    let [x, y] = middle(course, k);
    let mut pattern = Pattern::new(x, y);
    pattern.pattern.width = 48.0 / WIDTH;
    pattern.pattern.height = 48.0 / HEIGHT;
    pattern.search.width = 0.4;
    pattern.search.height = 0.5;
    pattern
}

/// A composition of a `MediaIn` into the Tracker `track` of `trackers` into the `MediaOut`.
fn composition(trackers: Vec<Pattern>) -> Value {
    let mut track = Node::new("track", Kind::Tracker);
    if let NodeKind::Tracker(tracker) = &mut track.kind {
        tracker.trackers = trackers;
    }
    let composition = Composition {
        nodes: vec![
            Node::new("in", Kind::MediaIn),
            track,
            Node::new("out", Kind::MediaOut),
        ],
        links: vec![
            Link::new("in", "track", Input::Input),
            Link::new("track", "out", Input::Input),
        ],
    };
    serde_json::to_value(&composition).unwrap()
}

struct Fixture {
    client: Client,
    timeline: Value,
    clip: Value,
    /// A clip of a colour, whose composition has a Tracker too.
    colour: Value,
    _engine: super::Engine,
    _dir: tempfile::TempDir,
}

/// A timeline of the clip of `frames` frames of the square moving along `course`, whose
/// composition has the Tracker `track` of `trackers`, and over it a clip of a colour with a
/// Tracker of its own; and a client that is the agent.
async fn fixture(frames: i64, course: Course, trackers: Vec<Pattern>) -> Option<Fixture> {
    let dir = project_dir();
    let path = synthetic(dir.path(), frames, course)?;
    let engine = engine(dir.path(), &["--no-proxies", "--sound", "null"]).await;
    let mut client = engine.client("agent").await;
    let media = client
        .result("media.import", Some(json!({"paths": [path]})))
        .await["added"][0]
        .clone();
    let scale = project(&mut client).await["project"]["media"][0]["facts"]["duration"]["scale"]
        .as_i64()
        .unwrap();
    let mut square = clip("$clip", media.as_str().unwrap(), 0, frames);
    square["source"]["in"] = json!({"ticks": 0, "scale": scale});
    let mut colour = clip("$colour", "unused", 0, frames);
    colour["source"] = json!({"kind": "color", "color": "#bf0000ff"});
    let tracks: Vec<Value> = [square, colour]
        .into_iter()
        .enumerate()
        .map(|(index, clip)| {
            json!({"id": format!("$v{index}"), "name": format!("V{index}"), "kind": "video",
                "enabled": true, "locked": false, "clips": [clip], "transitions": []})
        })
        .collect();
    let names = client
        .apply(json!([{"op": "add_timeline", "timeline": {
            "id": "$timeline", "name": "Square", "width": W, "height": H,
            "rate": {"num": 25, "den": 1}, "sample_rate": 48000, "tracks": tracks}}]))
        .await["names"]
        .clone();
    client
        .apply(json!([
            {"op": "set_fusion", "timeline": names["$timeline"], "clip": names["$clip"],
                "value": composition(trackers)},
            {"op": "set_fusion", "timeline": names["$timeline"], "clip": names["$colour"],
                "value": composition(vec![Pattern::default()])},
        ]))
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

/// `params` with each field of `more` put in.
fn with(mut params: Value, more: &Value) -> Value {
    for (name, value) in more.as_object().unwrap() {
        params[name] = value.clone();
    }
    params
}

impl Fixture {
    /// The `fusion` of a track of the Tracker of the square, with each field of `more`.
    fn aim(&self, more: &Value) -> Value {
        with(
            json!({"timeline": self.timeline, "clip": self.clip, "node": "track"}),
            more,
        )
    }

    async fn revision(&mut self) -> Value {
        project(&mut self.client).await["revision"].clone()
    }

    /// Start a track of the Tracker of the square, with each field of `more` in its `fusion`,
    /// from `frame`.
    async fn start(&mut self, more: &Value, frame: i64, direction: &str) -> Value {
        let revision = self.revision().await;
        let fusion = self.aim(more);
        self.client
            .result(
                "track.start",
                Some(
                    json!({"fusion": fusion, "revision": revision, "frame": frame,
                    "direction": direction}),
                ),
            )
            .await
    }

    /// The settings of the Tracker of the square, as the project has them.
    async fn node(&mut self) -> Value {
        let project = project(&mut self.client).await;
        let clip = &project["project"]["timelines"][0]["tracks"][0]["clips"][0];
        clip["fusion"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["id"] == "track")
            .unwrap()["params"]
            .clone()
    }

    /// Its patterns.
    async fn trackers(&mut self) -> Vec<Value> {
        self.node().await["trackers"].as_array().unwrap().clone()
    }
}

/// The path of a pattern as it is written.
fn path_of(pattern: &Value) -> BTreeMap<i64, [f64; 2]> {
    pattern["path"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(frame, place)| {
            (
                frame.parse().unwrap(),
                [place[0].as_f64().unwrap(), place[1].as_f64().unwrap()],
            )
        })
        .collect()
}

/// That `path` has the frames `frames` and follows the middle of the square along `course` at
/// each, within [`WITHIN`].
fn follows(path: &BTreeMap<i64, [f64; 2]>, frames: &[i64], course: Course) {
    assert_eq!(path.keys().copied().collect::<Vec<_>>(), frames);
    for (&frame, &place) in path {
        let off = apart(place, middle(course, frame));
        assert!(off < WITHIN, "frame {frame}: {off} pixels off");
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

/// The notifications up to the first progress or the end of the track `job`.
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

#[tokio::test]
async fn a_pattern_follows_a_moving_square_and_its_path_is_one_step_to_undo() {
    let mut off = on_square(rising, 0);
    off.enabled = false;
    off.pattern.x = 0.2;
    off.pattern.y = 0.8;
    let Some(mut f) = fixture(20, rising, vec![on_square(rising, 0), off]).await else {
        return;
    };
    let before = f.trackers().await;
    let started = f.start(&json!({}), 0, "forward").await;
    assert_eq!(started["first"], 0);
    assert_eq!(started["last"], 19);
    assert_eq!(started["frame_size"], json!([W, H]));
    assert_eq!(started["direction"], "forward");
    // The pattern that is off is not followed.
    assert_eq!(started["trackers"], json!([0]));
    let (seen, end) = until_ended(&mut f.client, &started["job"]).await;
    assert_eq!(end["phase"], "done", "{end}");
    assert_eq!(end["frames"], 19);
    assert_eq!(end["lost"], 0);
    assert_eq!(end["tracked"], json!([0, 19]));
    assert_eq!(
        end["trackers"],
        json!([{"tracker": 0, "frames": 19, "lost": []}])
    );
    // The path was written while the track went on: every client saw it grow.
    assert!(seen.iter().any(|m| m["method"] == "project.changed"));
    let trackers = f.trackers().await;
    follows(&path_of(&trackers[0]), &(0..20).collect::<Vec<_>>(), rising);
    // The pattern that is off is as it was, and of the other only the path changed.
    assert_eq!(trackers[1], before[1]);
    let mut unpathed = trackers[0].clone();
    unpathed["path"] = json!({});
    assert_eq!(unpathed, before[0]);
    // Match moved, the path holds the square still: each frame is put back by as far as the
    // square moved since the reference frame, 0.
    let mut tracker: Tracker = serde_json::from_value(f.node().await).unwrap();
    tracker.operation = TrackerOperation::MatchMove;
    let [x0, y0] = rising(0);
    for k in 0..20 {
        let [x, y] = rising(k);
        let held = tracker.at(to_f64(k), (256, 192));
        let wanted = [0.5 - to_f64(x - x0) / WIDTH, 0.5 + to_f64(y - y0) / HEIGHT];
        let off = apart(held.center, wanted);
        assert!(
            off < WITHIN,
            "frame {k}: the picture is put {off} pixels off"
        );
        assert_eq!((held.size, held.angle), (1.0, 0.0));
    }
    // One step to undo takes the whole track away, and Redo puts it back.
    f.client.result("history.undo", None).await;
    assert_eq!(f.trackers().await, before);
    f.client.result("history.redo", None).await;
    assert_eq!(f.trackers().await, trackers);
}

#[tokio::test]
async fn a_pattern_is_followed_backward_from_the_last_frame() {
    let Some(mut f) = fixture(20, rising, vec![on_square(rising, 19)]).await else {
        return;
    };
    let started = f.start(&json!({"trackers": [0]}), 19, "reverse").await;
    assert_eq!(started["direction"], "reverse");
    let (_, end) = until_ended(&mut f.client, &started["job"]).await;
    assert_eq!(end["phase"], "done", "{end}");
    assert_eq!(end["frames"], 19);
    assert_eq!(end["tracked"], json!([0, 19]));
    follows(
        &path_of(&f.trackers().await[0]),
        &(0..20).collect::<Vec<_>>(),
        rising,
    );
}

#[tokio::test]
async fn a_stopped_track_keeps_what_it_found() {
    let Some(mut f) = fixture(400, swinging, vec![on_square(swinging, 0)]).await else {
        return;
    };
    let started = f.start(&json!({}), 0, "forward").await;
    let job = started["job"].clone();
    let seen = first_progress(&mut f.client, &job).await;
    assert!(
        !seen.iter().any(|m| m["method"] == "track.state"),
        "the track ended before it could be stopped: {seen:?}"
    );
    // The progress tells which patterns are still followed.
    let progress = seen
        .iter()
        .find(|m| m["method"] == "track.progress")
        .unwrap();
    assert_eq!(progress["params"]["following"], json!([0]));
    f.client
        .result("track.stop", Some(json!({"job": job})))
        .await;
    let (_, end) = until_ended(&mut f.client, &job).await;
    assert_eq!(end["phase"], "stopped", "{end}");
    assert_eq!(end["reason"], "stop");
    let reached = end["frame"].as_i64().unwrap();
    assert!(reached > 0 && reached < 399, "{end}");
    assert_eq!(end["tracked"], json!([0, reached]));
    follows(
        &path_of(&f.trackers().await[0]),
        &(0..=reached).collect::<Vec<_>>(),
        swinging,
    );
}

#[tokio::test]
async fn a_track_of_what_cannot_be_followed_is_refused_before_it_starts() {
    let mut off = on_square(rising, 0);
    off.enabled = false;
    let Some(mut f) = fixture(20, rising, vec![on_square(rising, 0), off]).await else {
        return;
    };
    // A Tracker with no patterns beside the other.
    let empty = serde_json::to_value(Node::new("empty", Kind::Tracker)).unwrap();
    f.client
        .apply(
            json!([{"op": "add_fusion_node", "timeline": f.timeline, "clip": f.clip,
            "node": empty}]),
        )
        .await;
    let before = f.trackers().await;
    let revision = f.revision().await;
    let asked = |fusion: Value| json!({"fusion": fusion, "revision": revision, "frame": 0, "direction": "forward"});
    let colour = json!({"timeline": f.timeline, "clip": f.colour, "node": "track"});
    let window = json!({"grade": {"level": "clip", "timeline": f.timeline, "clip": f.clip},
        "node": "n1", "window": "w1"});
    let stale = revision.as_i64().unwrap() - 1;
    for (params, code, reason) in [
        (asked(f.aim(&json!({"trackers": [1]}))), -32001, "tracker"),
        (
            asked(f.aim(&json!({"trackers": [0, 5]}))),
            -32001,
            "tracker",
        ),
        (asked(f.aim(&json!({"node": "in"}))), -32001, "wrong_node"),
        (asked(f.aim(&json!({"node": "n404"}))), -32001, "target"),
        (
            asked(f.aim(&json!({"node": "empty"}))),
            -32001,
            "no_trackers",
        ),
        (asked(colour), -32001, "untrackable"),
        (
            with(asked(f.aim(&json!({}))), &json!({"frame": 500})),
            -32001,
            "frame",
        ),
        (
            with(asked(f.aim(&json!({}))), &json!({"revision": stale})),
            -32001,
            "stale_revision",
        ),
        // A window and a Tracker both, neither, and a setting of a window's tracker.
        (with(asked(f.aim(&json!({}))), &window), -32602, "fusion"),
        (
            json!({"revision": revision, "frame": 0, "direction": "forward"}),
            -32602,
            "fusion",
        ),
        (
            with(asked(f.aim(&json!({}))), &json!({"tracker": "point"})),
            -32602,
            "fusion",
        ),
    ] {
        let error = f.client.error("track.start", Some(params.clone())).await;
        assert_eq!(error["code"], code, "{params}: {error}");
        assert_eq!(error["data"]["reason"], reason, "{params}: {error}");
    }
    let params = asked(f.aim(&json!({"node": "in"})));
    let error = f.client.error("track.start", Some(params)).await;
    assert_eq!(error["data"]["kind"], "media_in", "{error}");
    // Nothing was started, and nothing was written.
    assert_eq!(f.trackers().await, before);
    let started = f.start(&json!({"trackers": [0]}), 0, "one_forward").await;
    let (_, end) = until_ended(&mut f.client, &started["job"]).await;
    assert_eq!(end["phase"], "done", "{end}");
    assert_eq!(end["frames"], 1);
    follows(&path_of(&f.trackers().await[0]), &[0, 1], rising);
}
