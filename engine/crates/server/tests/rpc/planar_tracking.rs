//! T19: the region of a Planar Tracker of a composition followed through the engine by a client
//! that is not the desktop, on a clip of a textured box over a flat ground that moves with a
//! shift, a turn, a zoom and a tilt in perspective: the path written as one step to undo, which
//! puts the corners of the box where they went; and the tracks refused.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use columbina_model::fusion::{
    Composition, Input, Kind, Link, Node, NodeKind, Place, PlanarOperation, PlanarTracker,
};
use columbina_track::{Mat3, Point};
use serde_json::{json, Value};

// The textured scene of the tracker's tests, which the tracks of a Tracker read too.
use super::tracking::scene;
use super::{clip, engine, project, project_dir, support, Client, WAIT};

/// The size of the frames, which are tracked as they are: they have fewer than 720 lines.
const W: usize = 256;
const H: usize = 192;

/// The box at frame 0, in pixels of the frames as the tracker counts them (the middle of the
/// pixel in column `i` and row `j` at `[i, j]`): from its left to its right, and from its top to
/// its bottom.
const BOX: [f64; 4] = [72.0, 52.0, 184.0, 140.0];

/// How far a corner of the box that the path puts may be from where it went, in pixels of the
/// frames.
const WITHIN: f64 = 0.5;

/// How many frames the clip has.
const FRAMES: i32 = 16;

/// The corners of the box at frame 0.
fn corners() -> [Point; 4] {
    let [left, top, right, bottom] = BOX;
    [[left, top], [right, top], [right, bottom], [left, bottom]]
}

/// The motion of the picture at each frame since frame 0: a shift, a turn, a zoom and a tilt in
/// perspective about the middle of the frame, more of each a frame.
fn truths() -> Vec<Mat3> {
    let middle = [128.0, 96.0];
    (0..FRAMES)
        .map(|k| {
            let k = f64::from(k);
            let tilted = Mat3::translation(middle[0], middle[1])
                .mul(&Mat3([
                    [1.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0],
                    [1.5e-5 * k, -1e-5 * k, 1.0],
                ]))
                .mul(&Mat3::translation(-middle[0], -middle[1]));
            scene::similarity(middle, 1.0 + 0.004 * k, 0.006 * k, [1.5 * k, -0.8 * k]).mul(&tilted)
        })
        .collect()
}

/// The brightness of the place `at` of frame 0: the textured scene within the box, its edges
/// smoothed over a pixel, and a flat ground about it.
fn boxed(scene: &scene::Scene, at: Point) -> f64 {
    let [left, top, right, bottom] = BOX;
    let inside = (at[0] - left)
        .min(right - at[0])
        .min(at[1] - top)
        .min(bottom - at[1]);
    let cover = (inside + 0.5).clamp(0.0, 1.0);
    0.08 + cover * (scene.value(at) - 0.08)
}

/// A clip of the frames of the box moved by each of [`truths`], losslessly encoded.
fn synthetic(dir: &Path) -> Option<PathBuf> {
    let ffmpeg = support::ffmpeg_program()?;
    let scene = scene::Scene::new(5, W, H);
    let path = dir.join("plane.mkv");
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
    for truth in truths() {
        let luma = scene::render_with(
            &|at| boxed(&scene, at),
            W,
            H,
            &truth,
            scene::Look::default(),
            None,
        );
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a value from 0 to 1 made a byte"
        )]
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

/// The width and the height of the frames.
#[allow(clippy::cast_precision_loss, reason = "sides of a few hundred pixels")]
fn extent() -> (f64, f64) {
    (W as f64, H as f64)
}

/// The place of the picture at the pixel `at` as the tracker counts it.
fn place_of(at: Point) -> Point {
    let (width, height) = extent();
    [(at[0] + 0.5) / width, 1.0 - (at[1] + 0.5) / height]
}

/// The pixel as the tracker counts it at the place `place` of the picture.
fn pixel_of(place: Point) -> Point {
    let (width, height) = extent();
    [place[0] * width - 0.5, (1.0 - place[1]) * height - 0.5]
}

/// A Planar Tracker, held still, whose region is the box at frame 0 less six pixels on each
/// side.
fn on_the_box() -> PlanarTracker {
    let [left, top, right, bottom] = BOX;
    let inset = 6.0;
    let region = [
        [left + inset, bottom - inset],
        [right - inset, bottom - inset],
        [right - inset, top + inset],
        [left + inset, top + inset],
    ]
    .map(|at| {
        let [x, y] = place_of(at);
        Place { x, y }
    })
    .to_vec();
    PlanarTracker {
        operation: PlanarOperation::Steady,
        region,
        ..PlanarTracker::default()
    }
}

/// A composition of a `MediaIn` into the Planar Tracker `plane` set as `planar` is, into the
/// `MediaOut`.
fn composition(planar: PlanarTracker) -> Value {
    let mut plane = Node::new("plane", Kind::PlanarTracker);
    plane.kind = NodeKind::PlanarTracker(planar);
    let composition = Composition {
        nodes: vec![
            Node::new("in", Kind::MediaIn),
            plane,
            Node::new("out", Kind::MediaOut),
        ],
        links: vec![
            Link::new("in", "plane", Input::Input),
            Link::new("plane", "out", Input::Input),
        ],
    };
    serde_json::to_value(&composition).unwrap()
}

struct Fixture {
    client: Client,
    timeline: Value,
    clip: Value,
    /// A clip of a colour, whose composition has a Planar Tracker too.
    colour: Value,
    _engine: super::Engine,
    _dir: tempfile::TempDir,
}

/// A timeline of the clip of the box, whose composition has the Planar Tracker `plane` on the
/// box, and over it a clip of a colour with a Planar Tracker of its own; and a client that is
/// the agent.
async fn fixture() -> Option<Fixture> {
    let dir = project_dir();
    let path = synthetic(dir.path())?;
    let engine = engine(dir.path(), &["--no-proxies", "--sound", "null"]).await;
    let mut client = engine.client("agent").await;
    let media = client
        .result("media.import", Some(json!({"paths": [path]})))
        .await["added"][0]
        .clone();
    let scale = project(&mut client).await["project"]["media"][0]["facts"]["duration"]["scale"]
        .as_i64()
        .unwrap();
    let frames = i64::from(FRAMES);
    let mut plane = clip("$clip", media.as_str().unwrap(), 0, frames);
    plane["source"]["in"] = json!({"ticks": 0, "scale": scale});
    let mut colour = clip("$colour", "unused", 0, frames);
    colour["source"] = json!({"kind": "color", "color": "#bf0000ff"});
    let tracks: Vec<Value> = [plane, colour]
        .into_iter()
        .enumerate()
        .map(|(index, clip)| {
            json!({"id": format!("$v{index}"), "name": format!("V{index}"), "kind": "video",
                "enabled": true, "locked": false, "clips": [clip], "transitions": []})
        })
        .collect();
    let names = client
        .apply(json!([{"op": "add_timeline", "timeline": {
            "id": "$timeline", "name": "Plane", "width": W, "height": H,
            "rate": {"num": 25, "den": 1}, "sample_rate": 48000, "tracks": tracks}}]))
        .await["names"]
        .clone();
    client
        .apply(json!([
            {"op": "set_fusion", "timeline": names["$timeline"], "clip": names["$clip"],
                "value": composition(on_the_box())},
            {"op": "set_fusion", "timeline": names["$timeline"], "clip": names["$colour"],
                "value": composition(PlanarTracker::default())},
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
    /// The `planar` of a track of the Planar Tracker of the box, with each field of `more`.
    fn aim(&self, more: &Value) -> Value {
        with(
            json!({"timeline": self.timeline, "clip": self.clip, "node": "plane"}),
            more,
        )
    }

    async fn revision(&mut self) -> Value {
        project(&mut self.client).await["revision"].clone()
    }

    /// Start a track of the Planar Tracker of the box from `frame`.
    async fn start(&mut self, frame: i64, direction: &str) -> Value {
        let revision = self.revision().await;
        let planar = self.aim(&json!({}));
        self.client
            .result(
                "track.start",
                Some(
                    json!({"planar": planar, "revision": revision, "frame": frame,
                    "direction": direction}),
                ),
            )
            .await
    }

    /// The settings of the Planar Tracker of the box, as the project has them.
    async fn node(&mut self) -> Value {
        let project = project(&mut self.client).await;
        let clip = &project["project"]["timelines"][0]["tracks"][0]["clips"][0];
        clip["fusion"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["id"] == "plane")
            .unwrap()["params"]
            .clone()
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

#[tokio::test]
async fn a_plane_is_followed_in_perspective_and_its_path_is_one_step_to_undo() {
    let Some(mut f) = fixture().await else {
        return;
    };
    let before = f.node().await;
    assert_eq!(before["path"], json!({}));
    let started = f.start(0, "forward").await;
    assert_eq!(started["first"], 0);
    assert_eq!(started["last"], i64::from(FRAMES) - 1);
    assert_eq!(started["frame_size"], json!([W, H]));
    assert_eq!(started["direction"], "forward");
    let (seen, end) = until_ended(&mut f.client, &started["job"]).await;
    assert_eq!(end["phase"], "done", "{end}");
    assert_eq!(end["frames"], i64::from(FRAMES) - 1);
    assert_eq!(end["lost"], 0);
    assert_eq!(end["tracked"], json!([0, i64::from(FRAMES) - 1]));
    // The path was written while the track went on: every client saw it grow.
    assert!(seen.iter().any(|m| m["method"] == "project.changed"));
    assert!(seen.iter().any(|m| m["method"] == "track.progress"));
    let node = f.node().await;
    let planar: PlanarTracker = serde_json::from_value(node.clone()).unwrap();
    assert_eq!(
        planar.path.keys().copied().collect::<Vec<_>>(),
        (0..i64::from(FRAMES)).collect::<Vec<_>>()
    );
    // Of the node only the path changed.
    let mut unpathed = node.clone();
    unpathed["path"] = json!({});
    assert_eq!(unpathed, before);
    // Each frame of the path puts the corners of the box where the motion of the picture took
    // them; and held still, the plane is put back where it was at frame 0.
    let truths = truths();
    for (k, truth) in (0..).zip(&truths) {
        let moved = planar.homography(f64::from(k));
        let back = planar.at(f64::from(k)).matrix;
        for corner in corners() {
            let went = truth.apply(corner);
            let put = pixel_of(moved.apply(place_of(corner)));
            let off = (put[0] - went[0]).hypot(put[1] - went[1]);
            assert!(
                off < WITHIN,
                "frame {k}: {corner:?} is put {off} pixels off"
            );
            // From the top left of the picture, as the stabilizer counts its places.
            let (width, height) = extent();
            let seen = [(went[0] + 0.5) / width, (went[1] + 0.5) / height];
            let held = Mat3(back).apply(seen);
            let rest = [held[0] * width - 0.5, held[1] * height - 0.5];
            let off = (rest[0] - corner[0]).hypot(rest[1] - corner[1]);
            assert!(
                off < WITHIN,
                "frame {k}: {corner:?} is held {off} pixels off"
            );
        }
    }
    // One step to undo takes the whole path away, and Redo puts it back.
    f.client.result("history.undo", None).await;
    assert_eq!(f.node().await, before);
    f.client.result("history.redo", None).await;
    assert_eq!(f.node().await, node);
    // Tracked again over what it has, it writes nothing and ends done.
    let again = f.start(0, "forward").await;
    let (_, end) = until_ended(&mut f.client, &again["job"]).await;
    assert_eq!(end["phase"], "done", "{end}");
    assert_eq!(f.node().await, node);
}

#[tokio::test]
async fn a_track_of_a_planar_tracker_that_cannot_be_followed_is_refused_before_it_starts() {
    let Some(mut f) = fixture().await else {
        return;
    };
    // A Planar Tracker beside the other whose region is beyond the picture.
    let mut outside = on_the_box();
    outside.region = vec![
        Place { x: 2.0, y: 2.0 },
        Place { x: 3.0, y: 2.0 },
        Place { x: 3.0, y: 3.0 },
    ];
    let mut node = Node::new("outside", Kind::PlanarTracker);
    node.kind = NodeKind::PlanarTracker(outside);
    let node = serde_json::to_value(&node).unwrap();
    f.client
        .apply(
            json!([{"op": "add_fusion_node", "timeline": f.timeline, "clip": f.clip,
            "node": node}]),
        )
        .await;
    let before = f.node().await;
    let revision = f.revision().await;
    let asked = |planar: Value| json!({"planar": planar, "revision": revision, "frame": 0, "direction": "forward"});
    let colour = json!({"timeline": f.timeline, "clip": f.colour, "node": "plane"});
    let window = json!({"grade": {"level": "clip", "timeline": f.timeline, "clip": f.clip},
        "node": "n1", "window": "w1"});
    let tracker = json!({"timeline": f.timeline, "clip": f.clip, "node": "plane"});
    let stale = revision.as_i64().unwrap() - 1;
    for (params, code, reason) in [
        (asked(f.aim(&json!({"node": "in"}))), -32001, "wrong_node"),
        (asked(f.aim(&json!({"node": "n404"}))), -32001, "target"),
        (asked(f.aim(&json!({"node": "outside"}))), -32001, "region"),
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
        // A window and a Planar Tracker both, a Tracker and a Planar Tracker both, and a
        // setting of a window's tracker.
        (with(asked(f.aim(&json!({}))), &window), -32602, "fusion"),
        (
            with(asked(f.aim(&json!({}))), &json!({ "fusion": tracker })),
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
    assert_eq!(f.node().await, before);
    // One frame forward: the frame that it started from and the next.
    let started = f.start(0, "one_forward").await;
    let (_, end) = until_ended(&mut f.client, &started["job"]).await;
    assert_eq!(end["phase"], "done", "{end}");
    assert_eq!(end["frames"], 1);
    let planar: PlanarTracker = serde_json::from_value(f.node().await).unwrap();
    assert_eq!(planar.path.keys().copied().collect::<Vec<_>>(), [0, 1]);
}
