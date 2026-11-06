//! The meters, the loudness and the analyses of sound on the engine as its clients see it (plan
//! T9b): `audio.analyze` of the bus that is exported reads what ffmpeg's `ebur128` reads of the
//! exported file; `audio.normalize` of two clips at −30 and −20 LUFS brings them to −23,
//! Independent and Relative, and applied is one step to undo; `audio.level_match`; a job
//! cancelled, and the params refused; and, on an engine that plays, `audio.meters` of a sine of
//! −23 dBFS read before and after a fader of −6 dB and the loudness of the bus heard, with Pause
//! and Reset of `audio.loudness`.

use std::path::Path;
use std::time::Duration;

use columbina_model::{Clip, Project, Time, Timeline, Track, TrackAudio, TrackKind};
use serde_json::{json, Value};

use super::{project, Client};

/// Samples a second of the media and of the timelines.
const RATE: u32 = 48_000;

/// A WAV of floats at 48 kHz, of `channels` channels and `frames` frames, sample `sample(n,
/// channel)`.
pub(super) fn wav(path: &Path, channels: u16, frames: u32, sample: impl Fn(u32, u16) -> f64) {
    let data = frames * u32::from(channels) * 4;
    let mut bytes = Vec::with_capacity(44 + data as usize);
    bytes.extend(b"RIFF");
    bytes.extend((36 + data).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16_u32.to_le_bytes());
    bytes.extend(3_u16.to_le_bytes());
    bytes.extend(channels.to_le_bytes());
    bytes.extend(RATE.to_le_bytes());
    bytes.extend((RATE * u32::from(channels) * 4).to_le_bytes());
    bytes.extend((channels * 4).to_le_bytes());
    bytes.extend(32_u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(data.to_le_bytes());
    for n in 0..frames {
        for channel in 0..channels {
            #[allow(clippy::cast_possible_truncation, reason = "a sample of floats")]
            bytes.extend((sample(n, channel) as f32).to_le_bytes());
        }
    }
    std::fs::write(path, bytes).unwrap();
}

/// A sine of 1 kHz at a peak of `db` dBFS at sample `n`.
fn sine(db: f64, n: u32) -> f64 {
    10_f64.powf(db / 20.0) * (std::f64::consts::TAU * 1000.0 * f64::from(n) / f64::from(RATE)).sin()
}

/// Noise of −0.5 to 0.5 at sample `n` of channel `channel`, the same each time.
fn noise(n: u32, channel: u16) -> f64 {
    let mut state = u64::from(n).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ u64::from(channel);
    state ^= state >> 29;
    state = state.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    state ^= state >> 32;
    #[allow(clippy::cast_precision_loss, reason = "noise")]
    let value = (state >> 11) as f64 / (1_u64 << 53) as f64;
    value - 0.5
}

/// A time of the media at 48 kHz, `seconds` from its start.
fn at(seconds: f64) -> Time {
    #[allow(clippy::cast_possible_truncation, reason = "whole samples")]
    Time {
        ticks: (seconds * f64::from(RATE)).round() as i64,
        scale: RATE,
    }
}

/// A stereo track of sound `id`, its clips `clips`.
fn track(id: &str, clips: Vec<Clip>) -> Track {
    Track {
        color: None,
        id: id.into(),
        kind: TrackKind::Audio,
        name: id.into(),
        enabled: true,
        locked: false,
        clips,
        transitions: vec![],
        language: String::new(),
        cues: vec![],
        audio: TrackAudio::default(),
    }
}

/// The timeline `id` at 25 frames a second of `tracks`, out to B1.
fn timeline(id: &str, tracks: Vec<Track>) -> Timeline {
    Timeline {
        id: id.into(),
        name: id.into(),
        rate: columbina_model::Rate { num: 25, den: 1 },
        width: 64,
        height: 36,
        sample_rate: RATE,
        tracks,
        grade: columbina_model::Grade::default(),
        buses: vec![columbina_model::Bus::first()],
        vcas: vec![],
        track_groups: vec![],
        audio: columbina_model::TimelineAudio::default(),
    }
}

/// The media of `paths` imported, their ids.
async fn import(client: &mut Client, paths: &[&Path]) -> Vec<String> {
    let paths: Vec<&str> = paths.iter().map(|path| path.to_str().unwrap()).collect();
    let added = client
        .result("media.import", Some(json!({ "paths": paths })))
        .await["added"]
        .clone();
    added
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap().to_owned())
        .collect()
}

/// Waits for the end of job `job` (as `audio.analysis` and `render.progress` tell it), and
/// returns what it was told then; the jobs' progress told, in order.
pub(super) async fn ended(client: &mut Client, method: &str, job: &Value) -> (Value, Vec<Value>) {
    let mut pending = client.take_notifications().into_iter();
    let mut running = Vec::new();
    loop {
        let message = match pending.next() {
            Some(value) => value,
            None => tokio::time::timeout(Duration::from_secs(120), client.message())
                .await
                .expect("the job ends"),
        };
        if message["method"] == method && message["params"]["job"] == *job {
            if message["params"]["state"] == "running" {
                running.push(message["params"].clone());
            } else {
                return (message["params"].clone(), running);
            }
        }
    }
}

/// The integrated loudness and the true peak that LGPL ffmpeg's `ebur128` reads of `path`
/// (convention 5), if an ffmpeg runs.
fn ffmpeg_reads(path: &Path) -> Option<(f64, f64)> {
    let lgpl = r"F:\agent harness\_review\toolchain\ffmpeg-lgpl-8.1\bin\ffmpeg.exe";
    let program = std::env::var("FFMPEG").unwrap_or_else(|_| {
        if Path::new(lgpl).exists() {
            lgpl.to_owned()
        } else {
            "ffmpeg".to_owned()
        }
    });
    let output = std::process::Command::new(&program)
        .args(["-hide_banner", "-nostats", "-i"])
        .arg(path)
        .args(["-af", "ebur128=peak=true", "-f", "null", "-"])
        .output();
    let Ok(output) = output else {
        assert_ne!(
            std::env::var("COLUMBINA_REQUIRE_FFMPEG").as_deref(),
            Ok("1"),
            "COLUMBINA_REQUIRE_FFMPEG=1 and no ffmpeg runs"
        );
        return None;
    };
    let text = String::from_utf8_lossy(&output.stderr);
    let summary = &text[text.rfind("Summary:").expect("a summary")..];
    let read = |key: &str| -> f64 {
        summary
            .lines()
            .find_map(|line| line.trim().strip_prefix(key).map(str::to_owned))
            .and_then(|rest| rest.split_whitespace().next()?.parse().ok())
            .unwrap()
    };
    Some((read("I:"), read("Peak:")))
}

fn close(got: &Value, want: f64, tolerance: f64, what: &str) {
    let value = got.as_f64().unwrap_or_else(|| panic!("{what}: {got}"));
    assert!(
        (value - want).abs() <= tolerance,
        "{what}: {value} is not {want} ± {tolerance}"
    );
}

#[tokio::test]
async fn the_analysis_of_the_bus_exported_reads_what_ffmpeg_reads_of_the_file() {
    let dir = super::project_dir();
    let sounds = tempfile::tempdir().unwrap();
    // 12 s of pink noise (Paul Kellet's filter of the noise) whose level steps every second (a
    // programme that the gates act on), with a burst near full scale for the true peak. Pink, as
    // programmes are: ffmpeg's true peak oversamples 4 times and reads noise that is white up to
    // the Nyquist frequency some 0.3 dB under the 8 times of the engine (the Limiter's, T10).
    let programme = sounds.path().join("programme.wav");
    let pink: Vec<Vec<f64>> = (0..2_u16)
        .map(|channel| {
            let mut b = [0.0_f64; 3];
            (0..12 * RATE)
                .map(|n| {
                    let white = noise(n, channel);
                    b[0] = 0.997_65 * b[0] + white * 0.099_046;
                    b[1] = 0.963 * b[1] + white * 0.296_516_4;
                    b[2] = 0.57 * b[2] + white * 1.052_691_3;
                    b[0] + b[1] + b[2] + white * 0.184_8
                })
                .collect()
        })
        .collect();
    wav(&programme, 2, 12 * RATE, |n, channel| {
        let second = n / RATE;
        let level = [0.3, 0.05, 0.2, 0.01, 0.4, 0.1][second as usize % 6];
        let burst = if (5 * RATE..5 * RATE + 2400).contains(&n) {
            3.6
        } else {
            1.0
        };
        level * burst * pink[usize::from(channel)][n as usize]
    });
    let engine = super::engine(
        dir.path(),
        &["--headless", "--no-proxies", "--sound", "null"],
    )
    .await;
    let mut client = engine.client("agent").await;
    let media = import(&mut client, &[&programme]).await;
    let mut a1 = track(
        "A1",
        vec![Clip::of_media("c1", media[0].clone(), at(0.0), 0, 300)],
    );
    a1.audio.strip.fader_db = -3.0;
    client
        .apply(json!([{"op": "add_timeline", "timeline": timeline("tl", vec![a1])}]))
        .await;
    let file = dir.path().join("programme-out.wav");
    let job = client
        .result(
            "render.start",
            Some(json!({"timeline": "tl", "path": file, "video": null,
                "audio": {"codec": "pcm_f32le"}})),
        )
        .await["job"]
        .clone();
    let (rendered, _) = ended(&mut client, "render.progress", &job).await;
    assert_eq!(rendered["state"], "done", "{rendered}");
    let job = client
        .result(
            "audio.analyze",
            Some(json!({"timeline": "tl", "targets": [{"bus": "B1"}, {"track": "A1"}, {"clip": "c1"}]})),
        )
        .await["job"]
        .clone();
    assert!(job.as_str().unwrap().starts_with("audio-"));
    let (done, running) = ended(&mut client, "audio.analysis", &job).await;
    println!("audio.analyze: {done}");
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["method"], "audio.analyze");
    assert_eq!(done["standard"], "ebu-r128");
    assert_ne!(running.len(), 0);
    // The bus and the track in one pass of 12 s, the clip in another.
    assert_eq!(running.last().unwrap()["total"], json!(24.0));
    let results = done["results"].as_array().unwrap();
    assert_eq!(results[0]["target"], json!({"bus": "B1"}));
    assert_eq!(results[1]["target"], json!({"track": "A1"}));
    assert_eq!(results[2]["target"], json!({"clip": "c1"}));
    // The bus and the track after its fader carry the same; the clip is measured before it.
    assert_eq!(results[0]["integrated"], results[1]["integrated"]);
    close(
        &results[2]["integrated"],
        results[0]["integrated"].as_f64().unwrap() + 3.0,
        0.011,
        "the clip, before the fader of −3 dB",
    );
    if let Some((integrated, true_peak)) = ffmpeg_reads(&file) {
        println!(
            "ffmpeg ebur128 of the export: I {integrated} LUFS, TP {true_peak} dBTP; the engine: I {}, TP {}",
            results[0]["integrated"], results[0]["true_peak"]
        );
        close(
            &results[0]["integrated"],
            integrated,
            0.1,
            "the integrated loudness",
        );
        close(&results[0]["true_peak"], true_peak, 0.2, "the true peak");
    }
}

/// An engine with the media of two clips of 4 s on A1, a sine of 1 kHz at −30 dBFS (−30 LUFS in
/// stereo) from 0 s and one at −20 dBFS from 4 s, and its client.
async fn two_clips(dir: &Path, sounds: &Path) -> (super::Engine, Client) {
    let quiet = sounds.join("quiet.wav");
    let loud = sounds.join("loud.wav");
    wav(&quiet, 2, 4 * RATE, |n, _| sine(-30.0, n));
    wav(&loud, 2, 4 * RATE, |n, _| sine(-20.0, n));
    let engine = super::engine(dir, &["--headless", "--no-proxies", "--sound", "null"]).await;
    let mut client = engine.client("agent").await;
    let media = import(&mut client, &[&quiet, &loud]).await;
    let a1 = track(
        "A1",
        vec![
            Clip::of_media("c1", media[0].clone(), at(0.0), 0, 100),
            Clip::of_media("c2", media[1].clone(), at(0.0), 100, 100),
        ],
    );
    client
        .apply(json!([{"op": "add_timeline", "timeline": timeline("tl", vec![a1])}]))
        .await;
    (engine, client)
}

/// The Volume of clip `clip` of timeline `tl`.
async fn volume(client: &mut Client, clip: &str) -> Value {
    let project: Project =
        serde_json::from_value(project(client).await["project"].clone()).unwrap();
    let timeline = project.timelines.iter().find(|t| t.id == "tl").unwrap();
    let clip = timeline
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
        .find(|c| c.id == clip)
        .unwrap();
    serde_json::to_value(&clip.gain_db).unwrap()
}

#[tokio::test]
async fn candidate_analysis_and_clip_ranges_measure_without_changing_history() {
    let dir = super::project_dir();
    let sounds = tempfile::tempdir().unwrap();
    let (engine, mut client) = two_clips(dir.path(), sounds.path()).await;
    let before = project(&mut client).await;
    let revision = before["revision"].as_u64().unwrap();
    let params = json!({"timeline":"tl", "targets":[{"clip":"c1"}],
        "range":{"from":25,"to":50}, "revision":revision});
    let job = client.result("audio.analyze", Some(params.clone())).await["job"].clone();
    let (original, _) = ended(&mut client, "audio.analysis", &job).await;
    assert_eq!(original["state"], "done", "{original}");
    assert_eq!(original["results"][0]["seconds"], 1.0);
    let mut candidate = params;
    candidate["candidate"] = json!([{"edit":"nudge_clip_gain", "timeline":"tl", "clips":["c1"], "delta_db":6.0,"linked":false}]);
    let job = client
        .result("audio.analyze", Some(candidate.clone()))
        .await["job"]
        .clone();
    let (trial, _) = ended(&mut client, "audio.analysis", &job).await;
    assert_eq!(trial["state"], "done", "{trial}");
    assert_eq!(trial["candidate"], true);
    assert_eq!(trial["results"][0]["seconds"], 1.0);
    close(
        &trial["results"][0]["integrated"],
        original["results"][0]["integrated"].as_f64().unwrap() + 6.0,
        0.02,
        "candidate gain",
    );
    assert_eq!(project(&mut client).await, before);
    candidate["revision"] = json!(revision + 1);
    let error = client.error("audio.analyze", Some(candidate)).await;
    assert_eq!(error["data"]["reason"], "stale_revision");
    assert_eq!(project(&mut client).await, before);
    // The last real step added the timeline; trial analyses must not become undo steps.
    client.result("history.undo", None).await;
    let undone = project(&mut client).await;
    assert!(!undone["project"]["timelines"]
        .as_array()
        .unwrap()
        .iter()
        .any(|timeline| timeline["id"] == "tl"));
    drop(client);
    drop(engine);
}

#[tokio::test]
async fn normalize_brings_two_clips_to_minus_23_independent_and_relative_and_applied_is_one_undo() {
    let dir = super::project_dir();
    let sounds = tempfile::tempdir().unwrap();
    let (_engine, mut client) = two_clips(dir.path(), sounds.path()).await;
    // Relative, measured only: the two as one programme of −22.6 LUFS, one gain for both.
    let job = client
        .result(
            "audio.normalize",
            Some(json!({"timeline": "tl", "clips": ["c1", "c2"], "level": "ebu-r128"})),
        )
        .await["job"]
        .clone();
    let (relative, _) = ended(&mut client, "audio.analysis", &job).await;
    println!("Relative: {relative}");
    assert_eq!(relative["state"], "done", "{relative}");
    assert_eq!(relative["mode"], "relative");
    assert_eq!(relative["target"], json!(-23.0));
    let energy = 10_f64.powf(-3.0).midpoint(10_f64.powf(-2.0));
    let together = 10.0 * energy.log10();
    close(&relative["measured"], together, 0.1, "the two together");
    let gains = &relative["clips"];
    assert_eq!(gains[0]["gain_db"], gains[1]["gain_db"]);
    close(&gains[0]["gain_db"], -23.0 - together, 0.1, "the one gain");
    assert_eq!(relative["applied"], Value::Null);
    assert_eq!(volume(&mut client, "c1").await, json!(0.0));

    // Independent, applied: +7 and −3, one transaction.
    let job = client
        .result(
            "audio.normalize",
            Some(
                json!({"timeline": "tl", "clips": ["c2", "c1"], "level": "ebu-r128",
                "mode": "independent", "apply": true}),
            ),
        )
        .await["job"]
        .clone();
    let (independent, _) = ended(&mut client, "audio.analysis", &job).await;
    println!("Independent: {independent}");
    assert_eq!(independent["state"], "done", "{independent}");
    let clips = &independent["clips"];
    assert_eq!(clips[0]["clip"], "c2");
    close(&clips[0]["measured"], -20.0, 0.1, "c2");
    close(&clips[1]["measured"], -30.0, 0.1, "c1");
    close(&clips[0]["gain_db"], -3.0, 0.1, "the gain of c2");
    close(&clips[1]["gain_db"], 7.0, 0.1, "the gain of c1");
    assert_eq!(clips[0]["clamped"], false);
    let applied = &independent["applied"];
    assert!(applied["revision"].is_u64(), "{independent}");
    close(&volume(&mut client, "c1").await, 7.0, 0.1, "c1's Volume");
    close(&volume(&mut client, "c2").await, -3.0, 0.1, "c2's Volume");
    // Measured again, they read −23 (a clip is measured before its Volume, so through the gain
    // of the mix: the bus over each clip's second).
    let job = client
        .result(
            "audio.analyze",
            Some(json!({"timeline": "tl", "targets": [{"bus": "B1"}],
                "range": {"from": 25, "to": 75}})),
        )
        .await["job"]
        .clone();
    let (first, _) = ended(&mut client, "audio.analysis", &job).await;
    close(&first["results"][0]["integrated"], -23.0, 0.1, "c1 after");
    // One step to undo takes both back.
    client.result("history.undo", None).await;
    assert_eq!(volume(&mut client, "c1").await, json!(0.0));
    assert_eq!(volume(&mut client, "c2").await, json!(0.0));
}

/// Normalize `clips` of `tl` to a sample peak of −9 dBFS, Independent and applied; what it told.
async fn normalized(client: &mut Client, clips: &[&str]) -> Value {
    let job = client
        .result(
            "audio.normalize",
            Some(json!({"timeline": "tl", "clips": clips, "mode": "independent", "apply": true})),
        )
        .await["job"]
        .clone();
    let (done, _) = ended(client, "audio.analysis", &job).await;
    assert_eq!(done["state"], "done", "{done}");
    done
}

#[tokio::test]
async fn a_clip_with_a_volume_is_normalized_once_and_its_keys_move_together() {
    let dir = super::project_dir();
    let sounds = tempfile::tempdir().unwrap();
    let (_engine, mut client) = two_clips(dir.path(), sounds.path()).await;
    // c1 (a peak of −30 dBFS) at +6 dB; c2 (−20 dBFS) keyed from −6 to 0 dB.
    client
        .apply(json!([
            {"op": "set_clip", "timeline": "tl", "id": "c1", "gain_db": 6.0},
            {"op": "set_clip", "timeline": "tl", "id": "c2", "gain_db": {"keys": [
                {"at": {"ticks": 0, "scale": 48000}, "value": -6.0},
                {"at": {"ticks": 96000, "scale": 48000}, "value": 0.0}]}}
        ]))
        .await;
    let first = normalized(&mut client, &["c1", "c2"]).await;
    println!("first: {first}");
    close(
        &first["clips"][0]["measured"],
        -24.0,
        0.05,
        "c1 after its Volume",
    );
    close(&first["clips"][0]["gain_db"], 15.0, 0.05, "c1's gain");
    close(&first["clips"][1]["gain_db"], 11.0, 0.05, "c2's gain");
    close(&volume(&mut client, "c1").await, 21.0, 0.05, "c1's Volume");
    let keys = volume(&mut client, "c2").await;
    close(&keys["keys"][0]["value"], 5.0, 0.05, "c2's first key");
    close(&keys["keys"][1]["value"], 11.0, 0.05, "c2's second key");
    // Again: nothing to add, so nothing applied.
    let again = normalized(&mut client, &["c1", "c2"]).await;
    println!("again: {again}");
    for clip in 0..2 {
        close(&again["clips"][clip]["gain_db"], 0.0, 0.01, "a second gain");
        close(
            &again["clips"][clip]["measured"],
            -9.0,
            0.01,
            "the peak reached",
        );
    }
    // The mix over c1 peaks at the target.
    let job = client
        .result(
            "audio.analyze",
            Some(json!({"timeline": "tl", "targets": [{"bus": "B1"}], "range": {"from": 0, "to": 100}})),
        )
        .await["job"]
        .clone();
    let (mix, _) = ended(&mut client, "audio.analysis", &job).await;
    close(
        &mix["results"][0]["sample_peak"],
        -9.0,
        0.01,
        "the peak of the mix over c1",
    );
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "Level Matcher, a cancelled job, and a table of refusals"
)]
async fn level_matcher_a_cancelled_job_and_what_is_refused() {
    let dir = super::project_dir();
    let sounds = tempfile::tempdir().unwrap();
    let (_engine, mut client) = two_clips(dir.path(), sounds.path()).await;
    let job = client
        .result(
            "audio.level_match",
            Some(json!({"timeline": "tl", "reference": "c2", "clips": ["c1"], "apply": true})),
        )
        .await["job"]
        .clone();
    let (matched, _) = ended(&mut client, "audio.analysis", &job).await;
    println!("Level Matcher: {matched}");
    assert_eq!(matched["state"], "done", "{matched}");
    close(
        &matched["reference"]["measured"],
        -20.0,
        0.1,
        "the reference",
    );
    close(&matched["clips"][0]["gain_db"], 10.0, 0.1, "c1 to c2");
    close(&volume(&mut client, "c1").await, 10.0, 0.1, "c1's Volume");

    // A job stopped as it starts ends cancelled.
    let job = client
        .result(
            "audio.analyze",
            Some(json!({"timeline": "tl", "targets": [{"bus": "B1"}, {"track": "A1"}, {"clip": "c1"}, {"clip": "c2"}]})),
        )
        .await["job"]
        .clone();
    client
        .result("audio.cancel", Some(json!({"job": job})))
        .await;
    let (cancelled, _) = ended(&mut client, "audio.analysis", &job).await;
    assert_eq!(cancelled["state"], "cancelled", "{cancelled}");
    let error = client
        .error("audio.cancel", Some(json!({"job": job})))
        .await;
    assert_eq!(error["data"]["reason"], "missing", "{error}");

    for (method, params, reason) in [
        (
            "audio.analyze",
            json!({"timeline": "nope", "targets": [{"bus": "B1"}]}),
            "missing",
        ),
        (
            "audio.analyze",
            json!({"timeline": "tl", "targets": []}),
            "out_of_range",
        ),
        (
            "audio.analyze",
            json!({"timeline": "tl", "targets": [{"clip": "c9"}]}),
            "missing",
        ),
        (
            "audio.analyze",
            json!({"timeline": "tl", "targets": [{"bus": "B9"}]}),
            "missing",
        ),
        (
            "audio.analyze",
            json!({"timeline": "tl", "targets": [{"bus": "B1"}], "range": {"from": 5, "to": 5}}),
            "out_of_range",
        ),
        (
            "audio.analyze",
            json!({"timeline": "tl", "targets": [{"bus": "B1"}], "standard": "loud"}),
            "standard",
        ),
        (
            "audio.normalize",
            json!({"timeline": "tl", "clips": ["c1", "c1"]}),
            "twice",
        ),
        (
            "audio.analyze",
            json!({"timeline": "tl", "targets": [{"bus": "B1"}, {"bus": "B1"}]}),
            "twice",
        ),
        (
            "audio.normalize",
            json!({"timeline": "tl", "clips": ["c1"], "target": 3.0}),
            "out_of_range",
        ),
        (
            "audio.normalize",
            json!({"timeline": "tl", "clips": ["c1"], "level": "loud"}),
            "standard",
        ),
        (
            "audio.level_match",
            json!({"timeline": "tl", "reference": "c9", "clips": ["c1"]}),
            "missing",
        ),
    ] {
        let error = client.error(method, Some(params.clone())).await;
        assert_eq!(error["code"], -32602, "{params}: {error}");
        assert_eq!(error["data"]["reason"], reason, "{params}: {error}");
    }
    let error = client
        .error(
            "audio.normalize",
            Some(json!({"timeline": "tl", "clips": ["c1"], "extra": 1})),
        )
        .await;
    assert_eq!(error["code"], -32602, "{error}");
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    clippy::needless_range_loop,
    reason = "a timeline played, its meters read, and its loudness paused and reset"
)]
async fn the_meters_of_a_timeline_that_plays_read_its_levels_and_its_loudness() {
    if columbina_compose::gpu_for_tests().is_none() {
        assert_ne!(
            std::env::var("COLUMBINA_REQUIRE_GPU").as_deref(),
            Ok("1"),
            "COLUMBINA_REQUIRE_GPU=1 and no graphics card"
        );
        return;
    }
    let dir = super::project_dir();
    let sounds = tempfile::tempdir().unwrap();
    // A sine of 1 kHz at −23 dBFS on both sides: −23 LUFS (Tech 3341, case 1), 30 s.
    let tone = sounds.path().join("tone.wav");
    wav(&tone, 2, 30 * RATE, |n, _| sine(-23.0, n));
    let engine = super::engine(dir.path(), &["--no-proxies", "--sound", "null"]).await;
    let mut client = engine.client("desktop").await;
    let media = import(&mut client, &[&tone]).await;
    let mut a1 = track(
        "A1",
        vec![Clip::of_media("c1", media[0].clone(), at(0.0), 0, 750)],
    );
    a1.audio.strip.fader_db = -6.0;
    client
        .apply(json!([{"op": "add_timeline", "timeline": timeline("tl", vec![a1])}]))
        .await;
    // Before it plays: nothing metered of it.
    let meters = client
        .result("audio.meters", Some(json!({"timeline": "tl"})))
        .await;
    assert_eq!(meters["playing"], false, "{meters}");
    assert_eq!(meters["rate"], 30);
    client
        .result(
            "transport.load",
            Some(json!({"monitor": "timeline", "timeline": "tl"})),
        )
        .await;
    client
        .result(
            "transport.play",
            Some(json!({"monitor": "timeline", "speed": 1})),
        )
        .await;
    // 4 s of it.
    let started = std::time::Instant::now();
    let meters = loop {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let meters = client
            .result("audio.meters", Some(json!({"timeline": "tl"})))
            .await;
        if meters["loudness"]["seconds"].as_f64().unwrap_or(0.0) >= 4.0 {
            break meters;
        }
        assert!(started.elapsed() < Duration::from_secs(60), "{meters}");
    };
    println!("audio.meters: {meters}");
    assert_eq!(meters["playing"], true, "{meters}");
    assert_eq!(meters["scale"], "iec");
    let strips = meters["strips"].as_array().unwrap();
    assert_eq!(strips[0]["owner"], json!({"track": "A1"}));
    assert_eq!(strips[1]["owner"], json!({"bus": "B1"}));
    for side in 0..2 {
        close(
            &strips[0]["pre"][side]["bar"],
            -23.0,
            0.05,
            "A1 before its fader",
        );
        close(
            &strips[0]["post"][side]["bar"],
            -29.0,
            0.05,
            "A1 after its fader",
        );
        close(&strips[1]["post"][side]["bar"], -29.0, 0.05, "B1");
        close(&strips[0]["post"][side]["peak"], -29.0, 0.05, "A1's peak");
    }
    // The sum of B1's two sides, 6.02 dB over each.
    close(
        &meters["control_room"]["bar"],
        -29.0 + 6.02,
        0.05,
        "the Control Room",
    );
    let loudness = &meters["loudness"];
    assert_eq!(loudness["running"], true);
    assert_eq!(loudness["standard"], "ebu-r128");
    close(&loudness["momentary"], -29.0, 0.1, "Momentary");
    close(&loudness["short_term"], -29.0, 0.1, "Short Term");
    close(&loudness["integrated"], -29.0, 0.1, "Integrated");
    close(&loudness["true_peak"], -29.0, 0.2, "the true peak");
    assert_eq!(loudness["verdict"], "low");
    assert_eq!(meters["lost"], 0);
    // Paused, it hears nothing more; reset, it has measured nothing.
    let paused = client
        .result(
            "audio.loudness",
            Some(json!({"timeline": "tl", "action": "pause"})),
        )
        .await;
    assert_eq!(paused["running"], false);
    let seconds = paused["seconds"].as_f64().unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;
    let read = client
        .result("audio.loudness", Some(json!({"timeline": "tl"})))
        .await;
    close(&read["seconds"], seconds, 0.05, "paused");
    let reset = client
        .result(
            "audio.loudness",
            Some(json!({"timeline": "tl", "action": "reset"})),
        )
        .await;
    assert_eq!(reset["integrated"], Value::Null);
    assert_eq!(reset["seconds"], json!(0.0));
    let started = client
        .result(
            "audio.loudness",
            Some(json!({"timeline": "tl", "action": "start"})),
        )
        .await;
    assert_eq!(started["running"], true);
    client
        .result("transport.pause", Some(json!({"monitor": "timeline"})))
        .await;
}
