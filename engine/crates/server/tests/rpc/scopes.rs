//! The scopes of the monitors and the library of LUTs through the real engine: a scope
//! subscribed is sent pictures of what its monitor shows, by its settings, at most 30 a second,
//! and none once it is unsubscribed; the statistics of a frame; and the LUTs of the roots, with
//! nothing outside them.

use std::path::Path;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{json, Value};

use super::{
    clip, engine, project_dir, start_in, support, transport_engine, Client, Started, Stderr,
};

/// A colour clip of `colour`, `length` frames from 0, made `scale` as large and moved by
/// `(x, y)` shares of the canvas.
fn colour(id: &str, colour: &str, length: i64, scale: f64, (x, y): (f64, f64)) -> Value {
    let mut made = clip(id, "unused", 0, length);
    made["source"] = json!({"kind": "color", "color": colour});
    made["transform"] = json!({"x": x, "y": y, "scale": scale, "rotation": 0.0, "opacity": 1.0});
    made
}

/// A timeline of 128 × 72 at `rate` frames a second, a track for each clip, loaded in the
/// timeline monitor; its id.
async fn timeline(client: &mut Client, rate: i64, clips: Vec<Value>) -> Value {
    let tracks: Vec<Value> = clips
        .into_iter()
        .enumerate()
        .map(|(index, clip)| {
            json!({"id": format!("$v{index}"), "kind": "video", "name": format!("V{index}"),
                "enabled": true, "locked": false, "clips": [clip], "transitions": []})
        })
        .collect();
    let names = client
        .apply(json!([{"op": "add_timeline", "timeline": {
            "id": "$timeline", "name": "Scoped", "width": 128, "height": 72,
            "rate": {"num": rate, "den": 1}, "sample_rate": 48000, "tracks": tracks,
        }}]))
        .await["names"]
        .clone();
    let id = names["$timeline"].clone();
    client
        .result(
            "transport.load",
            Some(json!({"monitor": "timeline", "timeline": id})),
        )
        .await;
    client.take_notifications();
    id
}

/// Colour bars of 75%: red, green, blue and grey, one in each quarter of the canvas.
fn bars(length: i64) -> Vec<Value> {
    vec![
        colour("$red", "#bf0000ff", length, 0.5, (-0.25, -0.25)),
        colour("$green", "#00bf00ff", length, 0.5, (0.25, -0.25)),
        colour("$blue", "#0000bfff", length, 0.5, (-0.25, 0.25)),
        colour("$grey", "#bfbfbfff", length, 0.5, (0.25, 0.25)),
    ]
}

/// The next `scopes.frame` of `scope`, of those that came or come: its params and its pixels.
async fn next_frame(client: &mut Client, scope: &Value) -> (Value, Vec<u8>) {
    let deadline = Instant::now() + super::WAIT;
    loop {
        assert!(Instant::now() < deadline, "no frame of {scope}");
        let message = if client.notifications.is_empty() {
            client.message().await
        } else {
            client.notifications.remove(0)
        };
        if message["method"] == "scopes.frame" && message["params"]["scope_id"] == *scope {
            let params = message["params"].clone();
            let rgba = base64::engine::general_purpose::STANDARD
                .decode(params["rgba"].as_str().unwrap())
                .unwrap();
            return (params, rgba);
        }
    }
}

/// The `scopes.frame` of `scope` that come within `time`, and when each came.
async fn frames_for(client: &mut Client, scope: &Value, time: Duration) -> Vec<(Value, Instant)> {
    let end = Instant::now() + time;
    let mut frames = Vec::new();
    for message in std::mem::take(&mut client.notifications) {
        if message["method"] == "scopes.frame" && message["params"]["scope_id"] == *scope {
            frames.push((message["params"].clone(), Instant::now()));
        }
    }
    while let Ok(message) =
        tokio::time::timeout_at(end.into(), super::read_message(&mut client.reader)).await
    {
        let message = message.expect("the engine closed the connection");
        if message["method"] == "scopes.frame" && message["params"]["scope_id"] == *scope {
            frames.push((message["params"].clone(), Instant::now()));
        }
    }
    frames
}

/// How many pixels of a picture are lit.
fn lit(rgba: &[u8]) -> usize {
    rgba.as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| pixel[..3].iter().any(|&channel| channel > 8))
        .count()
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one scope, from subscribed to unsubscribed"
)]
async fn a_scope_of_colour_bars_is_sent_by_its_settings_and_stops_when_unsubscribed() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let engine = engine(dir.path(), &["--no-proxies", "--sound", "null"]).await;
    let mut client = engine.client("desktop").await;
    timeline(&mut client, 25, bars(50)).await;

    // A waveform as it is made: its answer, then a picture of its size, of frame 0.
    let subscribed = client
        .result(
            "scopes.subscribe",
            Some(json!({"view": "timeline", "settings": {"kind": "waveform"}})),
        )
        .await;
    let scope = subscribed["scope_id"].clone();
    assert_eq!(scope, "scope-1", "{subscribed}");
    assert_eq!(subscribed["size"], json!({"width": 512, "height": 256}));
    assert_eq!(subscribed["graticule"]["width"], 512);
    assert_ne!(subscribed["graticule"]["lines"], json!([]));
    assert_eq!(subscribed["settings"]["levels"], 256);
    assert_eq!(subscribed["settings"]["signal"]["transfer"], "rec709");
    let (frame, waveform) = next_frame(&mut client, &scope).await;
    assert_eq!(
        (
            &frame["view"],
            &frame["frame"],
            &frame["width"],
            &frame["height"]
        ),
        (&json!("timeline"), &json!(0), &json!(512), &json!(256)),
        "{frame}"
    );
    assert_eq!(waveform.len(), 512 * 256 * 4);
    assert!(lit(&waveform) > 0);

    // Other settings: a vectorscope with no graticule, whose picture lights the four colours
    // and no more.
    let set = client
        .result(
            "scopes.set",
            Some(json!({"scope_id": scope, "settings": {
                "kind": "vectorscope", "size": 128, "look": {"graticule": 0}}})),
        )
        .await;
    assert_eq!(set["size"], json!({"width": 128, "height": 128}));
    assert_eq!(set["settings"]["targets"], "75");
    let (frame, vectors) = next_frame(&mut client, &scope).await;
    assert_eq!(
        (&frame["width"], &frame["height"]),
        (&json!(128), &json!(128))
    );
    assert_eq!(vectors.len(), 128 * 128 * 4);
    let shown = lit(&vectors);
    assert!((4..=64).contains(&shown), "{shown} pixels lit");

    // Paused, nothing new is shown, and nothing more is sent.
    let more = frames_for(&mut client, &scope, Duration::from_millis(400)).await;
    assert!(more.is_empty(), "{more:?}");

    // What is not a scope of this connection, or not a setting, is refused.
    let other = engine.client("agent").await;
    let mut other = other;
    let error = other
        .error("scopes.unsubscribe", Some(json!({"scope_id": scope})))
        .await;
    assert_eq!(error["code"], -32602);
    assert_eq!(
        error["data"],
        json!({"reason": "missing", "scope_id": "scope-1"})
    );
    let error = client
        .error(
            "scopes.set",
            Some(json!({"scope_id": scope, "settings": {"kind": "vectorscope", "size": 7}})),
        )
        .await;
    assert_eq!(error["data"]["reason"], "settings", "{error}");
    let error = client
        .error(
            "scopes.subscribe",
            Some(json!({"view": "timeline", "settings": {"kind": "spectrum"}})),
        )
        .await;
    assert_eq!(error["data"]["reason"], "settings", "{error}");
    let error = client
        .error(
            "scopes.subscribe",
            Some(json!({"view": "program", "settings": {"kind": "waveform"}})),
        )
        .await;
    assert_eq!(error["code"], -32602, "{error}");

    // Unsubscribed, no frame of it follows the answer, though the monitor plays.
    let answered = client
        .result("scopes.unsubscribe", Some(json!({"scope_id": scope})))
        .await;
    assert_eq!(answered, json!({}));
    client.take_notifications();
    client
        .result(
            "transport.play",
            Some(json!({"monitor": "timeline", "speed": 1})),
        )
        .await;
    let after = frames_for(&mut client, &scope, Duration::from_millis(600)).await;
    assert!(after.is_empty(), "{after:?}");
    let error = client
        .error("scopes.unsubscribe", Some(json!({"scope_id": scope})))
        .await;
    assert_eq!(error["data"]["reason"], "missing");
}

#[tokio::test]
async fn a_scope_is_sent_at_most_30_frames_a_second_while_its_monitor_plays_at_60() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let engine = engine(dir.path(), &["--no-proxies", "--sound", "null"]).await;
    let mut client = engine.client("desktop").await;
    timeline(&mut client, 60, bars(600)).await;
    let scope = client
        .result(
            "scopes.subscribe",
            Some(json!({"view": "timeline", "settings": {"kind": "histogram"}})),
        )
        .await["scope_id"]
        .clone();
    let (first, _) = next_frame(&mut client, &scope).await;
    assert_eq!(first["frame"], 0);
    client
        .result(
            "transport.play",
            Some(json!({"monitor": "timeline", "speed": 1})),
        )
        .await;
    let time = Duration::from_secs(2);
    let frames = frames_for(&mut client, &scope, time).await;
    let shown: Vec<i64> = frames
        .iter()
        .map(|(frame, _)| frame["frame"].as_i64().unwrap())
        .collect();
    // At most one every 1/30 s (and one owed at the start), and as many as the client reads.
    assert!(frames.len() <= 62, "{} frames: {shown:?}", frames.len());
    assert!(frames.len() >= 30, "{} frames: {shown:?}", frames.len());
    assert!(shown.windows(2).all(|pair| pair[0] < pair[1]), "{shown:?}");
    // Frames of the monitor were passed over: it shows 60 a second.
    let span = shown.last().unwrap() - shown.first().unwrap();
    #[allow(clippy::cast_possible_wrap)]
    let sent = frames.len() as i64;
    assert!(span > sent, "{shown:?}");
    // No two closer than the client's reading allows them to be judged: on average 1/30 s
    // or more apart.
    let (_, from) = frames.first().unwrap();
    let (_, to) = frames.last().unwrap();
    #[allow(clippy::cast_precision_loss)]
    let rate = (frames.len() - 1) as f64 / to.duration_since(*from).as_secs_f64();
    eprintln!(
        "{} frames of the scope in {time:?}, {rate:.1} a second, of frames {shown:?}",
        frames.len()
    );
    assert!(rate <= 31.5, "{rate} frames a second");
}

#[tokio::test]
async fn the_statistics_of_a_grey_frame_are_grey() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let engine = engine(dir.path(), &["--no-proxies", "--sound", "null"]).await;
    let mut client = engine.client("agent").await;
    let error = client
        .error("scopes.statistics", Some(json!({"view": "timeline"})))
        .await;
    assert_eq!(error["code"], -32004, "{error}");
    assert_eq!(error["data"]["reason"], "not_loaded", "{error}");
    timeline(
        &mut client,
        25,
        vec![colour("$grey", "#808080ff", 25, 1.0, (0.0, 0.0))],
    )
    .await;
    let figures = client
        .result("scopes.statistics", Some(json!({"view": "timeline"})))
        .await;
    assert_eq!(
        (
            &figures["view"],
            &figures["frame"],
            &figures["width"],
            &figures["height"]
        ),
        (&json!("timeline"), &json!(0), &json!(128), &json!(72)),
        "{figures}"
    );
    assert_eq!(figures["samples"], 128 * 72);
    let grey = 128.0 / 255.0;
    for channel in ["red", "green", "blue", "luma"] {
        let figures = &figures[channel];
        for what in ["min", "max", "mean"] {
            let value = figures[what].as_f64().unwrap();
            assert!((value - grey).abs() < 2e-3, "{channel} {what} {value}");
        }
        let median = figures["mean"].as_f64().unwrap();
        assert!((median - grey).abs() < 0.01);
    }
    let chroma = &figures["chroma"];
    for what in ["radius", "saturation"] {
        assert!(chroma[what].as_f64().unwrap() < 1e-3, "{chroma}");
    }
    let median = figures["percentiles"]["luma"]["50"].as_f64().unwrap();
    assert!((median - grey).abs() < 1.0 / 255.0 + 1e-6, "{median}");
    assert_eq!(figures["histogram"]["bins"], 256);
    let luma = figures["histogram"]["luma"].as_array().unwrap();
    assert_eq!(luma.len(), 256);
    assert_eq!(
        luma.iter()
            .filter(|count| count.as_u64() != Some(0))
            .count(),
        1
    );

    let fewer = client
        .result(
            "scopes.statistics",
            Some(json!({"view": "timeline", "bins": 16})),
        )
        .await;
    assert_eq!(fewer["histogram"]["red"].as_array().unwrap().len(), 16);
    let error = client
        .error(
            "scopes.statistics",
            Some(json!({"view": "timeline", "bins": 8})),
        )
        .await;
    assert_eq!(error["data"]["reason"], "bins", "{error}");
}

#[tokio::test]
async fn the_source_monitor_has_scopes_and_statistics_of_the_media_shown() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let Some((_engine, mut client)) = transport_engine(dir.path(), &[support::Sample::Fhd]).await
    else {
        return;
    };
    client
        .result(
            "transport.load",
            Some(json!({"monitor": "source", "media": "m1"})),
        )
        .await;
    let scope = client
        .result(
            "scopes.subscribe",
            Some(json!({"view": "source", "settings": {"kind": "parade", "width": 64}})),
        )
        .await["scope_id"]
        .clone();
    let (frame, rgba) = next_frame(&mut client, &scope).await;
    assert_eq!(
        (&frame["view"], &frame["frame"], &frame["width"]),
        (&json!("source"), &json!(0), &json!(192)),
        "{frame}"
    );
    assert_eq!(rgba.len(), 192 * 256 * 4);
    assert!(lit(&rgba) > 0);
    client
        .result(
            "transport.seek",
            Some(json!({"monitor": "source", "frame": 10})),
        )
        .await;
    let (frame, _) = next_frame(&mut client, &scope).await;
    assert_eq!(frame["frame"], 10, "{frame}");
    let figures = client
        .result("scopes.statistics", Some(json!({"view": "source"})))
        .await;
    assert_eq!(
        (&figures["frame"], &figures["width"], &figures["height"]),
        (&json!(10), &json!(1920), &json!(1080)),
    );
    assert_eq!(figures["samples"], 1920 * 1080);
}

#[tokio::test]
async fn a_headless_engine_has_no_scopes() {
    let dir = project_dir();
    let engine = engine(dir.path(), &["--headless"]).await;
    let mut client = engine.client("agent").await;
    for (method, params) in [
        (
            "scopes.subscribe",
            json!({"view": "timeline", "settings": {"kind": "waveform"}}),
        ),
        (
            "scopes.set",
            json!({"scope_id": "scope-1", "settings": {"kind": "waveform"}}),
        ),
        ("scopes.unsubscribe", json!({"scope_id": "scope-1"})),
        ("scopes.statistics", json!({"view": "timeline"})),
    ] {
        let error = client.error(method, Some(params)).await;
        assert_eq!(error["code"], -32601, "{method}: {error}");
    }
}

/// Write a file, and the folders it is in.
fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[tokio::test]
async fn the_luts_of_the_project_and_the_library_are_listed_and_nothing_outside_them() {
    let dir = project_dir();
    let library = project_dir();
    let outside = project_dir();
    let cube = "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
    write(&dir.path().join("LUTs/Film.cube"), cube);
    write(&dir.path().join("look.CUBE"), cube);
    write(&dir.path().join("notes.txt"), "not a LUT");
    write(&dir.path().join("cache/thumbs/kept.cube"), cube);
    write(&library.path().join("look.cube"), cube);
    write(&library.path().join("Deep/a/b.cube"), cube);
    write(&outside.path().join("secret.cube"), cube);
    let missing = library.path().join("none");
    // A junction out of the library, which is not followed.
    #[cfg(windows)]
    let joined = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(library.path().join("Out"))
        .arg(outside.path())
        .output()
        .is_ok_and(|made| made.status.success());
    #[cfg(not(windows))]
    let joined = std::os::unix::fs::symlink(outside.path(), library.path().join("Out")).is_ok();
    assert!(joined, "a junction or link is made");
    let roots = std::env::join_paths([library.path(), missing.as_path()]).unwrap();
    let started = start_in(
        dir.path(),
        &["--headless"],
        Stderr::Read,
        &[("COLUMBINA_LUTS", roots.as_os_str())],
    )
    .await;
    let Started::Listening(engine) = started else {
        panic!("the engine did not start");
    };
    let mut client = engine.client("agent").await;
    let listed = client.result("luts.list", None).await;
    let roots = listed["roots"].as_array().unwrap();
    assert_eq!(roots.len(), 3, "{listed}");
    assert_eq!(roots[0]["place"], "project");
    assert_eq!(roots[1]["place"], "library");
    assert_eq!(roots[1]["path"], library.path().to_str().unwrap());
    assert_eq!(roots[1]["exists"], true);
    assert_eq!(roots[2]["exists"], false);
    let names: Vec<(&str, u64, bool)> = listed["luts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|lut| {
            (
                lut["name"].as_str().unwrap(),
                lut["root"].as_u64().unwrap(),
                lut["shadowed"].as_bool().unwrap(),
            )
        })
        .collect();
    let look_shadowed = cfg!(windows);
    assert_eq!(
        names,
        [
            ("LUTs/Film.cube", 0, false),
            ("look.CUBE", 0, false),
            ("Deep/a/b.cube", 1, false),
            ("look.cube", 1, look_shadowed),
        ],
        "{listed}"
    );
    assert_eq!(listed["truncated"], false);
    let film = &listed["luts"][0];
    assert_eq!(film["size"], cube.len());
    assert!(film["modified_us"].as_i64().unwrap() > 0);

    // A folder within the roots, in each.
    let deep = client
        .result("luts.list", Some(json!({"folder": "Deep\\a"})))
        .await;
    let names: Vec<&str> = deep["luts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|lut| lut["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Deep/a/b.cube"]);
    // A folder that leads out of the roots, by its name or by a junction.
    for folder in ["..", "Deep/../..", "C:\\", "/etc", "\\\\server\\share"] {
        let error = client
            .error("luts.list", Some(json!({"folder": folder})))
            .await;
        assert_eq!(error["code"], -32602, "{folder}: {error}");
        assert_eq!(
            error["data"],
            json!({"reason": "outside_roots", "folder": folder})
        );
    }
    let out = client
        .result("luts.list", Some(json!({"folder": "Out"})))
        .await;
    assert_eq!(out["luts"], json!([]), "{out}");
    let all = client.result("luts.list", Some(json!({}))).await;
    assert!(!all.to_string().contains("secret"), "{all}");
    let error = client.error("luts.list", Some(json!({"path": "x"}))).await;
    assert_eq!(error["code"], -32602);
}
