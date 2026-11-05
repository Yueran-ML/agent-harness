//! The null test of the plan's convention 4 (T6), on the engine as its clients see it: a
//! timeline played with `--sound capture=<path>` (frame `n` of a session's file is sample
//! `from + n` of the timeline) and the same timeline exported as PCM of floats, the one taken
//! from the other is nothing but zeros. The timelines have every kind of what the graph does
//! (EQ, Dynamics, Limiter, Noise Reduction, Ducker, Pitch, a send to a bus, a VCA, a lane of the
//! fader, a crossfade, a clip at 0.8 times its speed with its pitch kept and one at 1.5 times,
//! effects on clips (T4c: a Pitch on the slow one, a Limiter on one in the crossfade), mono,
//! stereo and 5.1 tracks), and their media are WAV, not Matroska (T3's note).
//!
//! Playback must start where it is asked: a session whose graph is not built and prerolled within
//! 2 s falls back to the clock of the system and starts its sound again elsewhere (T5), outside
//! the promise of the null test. The workspace's `Cargo.toml` therefore builds the DSP
//! (`columbina-audio`, `columbina-stretch`, `rustfft`, `realfft`, `nnnoiseless`) optimised in
//! debug builds too; unoptimised, the preroll of these timelines took about 5 s. The sessions'
//! starts are read from the digest (`audio_sample` less `audio_samples`): the first, started as
//! the device is first made ready, may start a sample after its frame. `COLUMBINA_REQUIRE_GPU=1`
//! makes a machine without a card fail the test instead of skipping it, and
//! `COLUMBINA_NULL_KEEP=<folder>` keeps the captures and the exports there.

use std::path::Path;
use std::time::Duration;

use columbina_audio::graph::{Graph, Options, Purpose};
use columbina_model::audio::fx::{Ducker, GainFx, LimiterFx, NoiseReductionFx, PitchFx};
use columbina_model::audio::{BusFormat, TrackFormat};
use columbina_model::{
    Alignment, Bus, Clip, CrossfadeLevel, Effect, Insert, Lane, LaneKind, LanePoint, Output,
    Project, Rgba, Send, Source, SourceRef, Speed, Time, Timeline, Track, TrackAudio, TrackKind,
    Transition, TransitionKind, Vca,
};
use serde_json::{json, Value};

use super::{project, Client};

/// Samples a second of the timelines and of the capture.
const RATE: usize = 48_000;

/// A WAV of 16 bits at 48 kHz, of `channels` channels and `frames` frames, sample `sample(n,
/// channel)`.
fn wav(path: &Path, channels: u16, frames: u32, sample: impl Fn(u32, u16) -> f64) {
    let data = frames * u32::from(channels) * 2;
    let mut bytes = Vec::with_capacity(44 + data as usize);
    bytes.extend(b"RIFF");
    bytes.extend((36 + data).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16_u32.to_le_bytes());
    bytes.extend(1_u16.to_le_bytes());
    bytes.extend(channels.to_le_bytes());
    bytes.extend(48_000_u32.to_le_bytes());
    bytes.extend((48_000 * u32::from(channels) * 2).to_le_bytes());
    bytes.extend((channels * 2).to_le_bytes());
    bytes.extend(16_u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(data.to_le_bytes());
    for n in 0..frames {
        for channel in 0..channels {
            #[allow(clippy::cast_possible_truncation, reason = "within 16 bits")]
            let code = (sample(n, channel) * 32_767.0)
                .round()
                .clamp(-32_768.0, 32_767.0) as i16;
            bytes.extend(code.to_le_bytes());
        }
    }
    std::fs::write(path, bytes).unwrap();
}

/// A sine of `hz` at sample `n` of 48 kHz.
fn sine(hz: f64, n: u32) -> f64 {
    (std::f64::consts::TAU * hz * f64::from(n) / 48_000.0).sin()
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

/// Three WAVs of 8 s in `dir`: mono (a voice-like tone that comes and goes, over noise), stereo
/// (two tones and noise) and 5.1 (a tone on each channel).
fn media(dir: &Path) -> Vec<String> {
    let frames = 8 * 48_000;
    let mono = dir.join("voice.wav");
    wav(&mono, 1, frames, |n, _| {
        let gate = if (n / 12_000) % 2 == 0 { 0.4 } else { 0.05 };
        gate * sine(220.0, n) + 0.02 * noise(n, 0)
    });
    let stereo = dir.join("music.wav");
    wav(&stereo, 2, frames, |n, channel| {
        let hz = if channel == 0 { 440.0 } else { 660.0 };
        0.25 * sine(hz, n) + 0.05 * noise(n, channel + 1)
    });
    let six = dir.join("room.wav");
    wav(&six, 6, frames, |n, channel| {
        0.1 * sine(100.0 * f64::from(channel + 1), n)
    });
    [mono, stereo, six]
        .iter()
        .map(|path| path.to_str().unwrap().to_owned())
        .collect()
}

/// A time of the timeline at 48 kHz, `seconds` from its start.
fn at(seconds: f64) -> Time {
    #[allow(clippy::cast_possible_truncation, reason = "whole samples")]
    Time {
        ticks: (seconds * 48_000.0).round() as i64,
        scale: 48_000,
    }
}

/// A clip of media `media` from `seconds_in` of it, on frames `start` to `start + length`.
fn clip(id: &str, media: &str, seconds_in: f64, start: i64, length: i64) -> Clip {
    Clip::of_media(id, media, at(seconds_in), start, length)
}

/// A track of sound of `format`, its clips `clips`.
fn track(id: &str, format: TrackFormat, clips: Vec<Clip>) -> Track {
    let channels = match format {
        TrackFormat::Mono => 1,
        TrackFormat::FiveOne => 6,
        _ => 2,
    };
    Track {
        id: id.into(),
        kind: TrackKind::Audio,
        name: id.into(),
        enabled: true,
        locked: false,
        clips,
        transitions: vec![],
        language: String::new(),
        cues: vec![],
        audio: TrackAudio {
            format,
            channels,
            ..TrackAudio::default()
        },
    }
}

fn insert(id: &str, effect: Effect) -> Insert {
    Insert {
        id: id.into(),
        on: true,
        effect,
    }
}

/// The timeline `id` of 8 s at 25 frames a second over the media `media` (as [`media`] makes
/// them), with everything of the graph on it; without Noise Reduction and with a short Limiter
/// where `quick`, so that it settles within the 10 s of the preroll.
#[allow(clippy::too_many_lines, reason = "one track after another")]
fn timeline(id: &str, media: &[String], quick: bool) -> Timeline {
    let (voice, music, room) = (&media[0], &media[1], &media[2]);
    let mut picture = Clip::of_media(format!("{id}pic"), voice.clone(), at(0.0), 0, 200);
    picture.source = Source::Color {
        color: Rgba([255, 0, 0, 255]),
    };
    let pictures = Track {
        id: format!("{id}v"),
        kind: TrackKind::Video,
        name: "Picture".into(),
        enabled: true,
        locked: false,
        clips: vec![picture],
        transitions: vec![],
        language: String::new(),
        cues: vec![],
        audio: TrackAudio::default(),
    };
    // A: the voice, mono, EQ and a compressor, its fader on a lane, in the VCA.
    let mut a = track(
        &format!("{id}a"),
        TrackFormat::Mono,
        vec![clip(&format!("{id}a1"), voice, 0.0, 0, 200)],
    );
    let strip = &mut a.audio.strip;
    strip.eq.on = true;
    strip.dynamics.on = true;
    strip.dynamics.compressor.on = true;
    strip.vcas = vec![format!("{id}vca")];
    a.audio.lanes.insert(
        "fader_db".into(),
        Lane {
            points: vec![
                LanePoint {
                    at: at(0.0),
                    value: -12.0,
                },
                LanePoint {
                    at: at(4.0),
                    value: 0.0,
                },
            ],
            kind: LaneKind::Continuous,
            scale: None,
        },
    );
    // B: the music, stereo, two clips crossfaded at 4 s, a Limiter (and Noise Reduction), a
    // send after the fader to the bus of effects, in the VCA.
    // The second clip has a Limiter of its own, 64 samples late, through the crossfade (T4c).
    let mut b2 = clip(&format!("{id}b2"), music, 4.0, 100, 100);
    b2.effects.push(insert(
        "",
        Effect::Limiter(LimiterFx {
            input_db: 9.0,
            ceiling_db: -3.0,
            release_ms: 30.0,
            ..LimiterFx::default()
        }),
    ));
    let mut b = track(
        &format!("{id}b"),
        TrackFormat::Stereo,
        vec![clip(&format!("{id}b1"), music, 0.0, 0, 100), b2],
    );
    b.transitions = vec![Transition {
        id: format!("{id}x"),
        kind: TransitionKind::Crossfade,
        left: format!("{id}b1"),
        right: format!("{id}b2"),
        length: 10,
        align: Alignment::Center,
        fade_in_level_db: CrossfadeLevel::default(),
        fade_out_level_db: CrossfadeLevel::default(),
    }];
    let strip = &mut b.audio.strip;
    strip.inserts.push(insert(
        "fx1",
        Effect::Limiter(LimiterFx {
            input_db: 6.0,
            ceiling_db: -6.0,
            release_ms: if quick { 50.0 } else { 100.0 },
            ..LimiterFx::default()
        }),
    ));
    if !quick {
        strip.inserts.push(insert(
            "fx2",
            Effect::NoiseReduction(NoiseReductionFx::default()),
        ));
    }
    strip.sends.push(Send {
        bus: format!("{id}fx"),
        on: true,
        pre: false,
        level_db: -6.0,
        pan: columbina_model::Pan::default(),
    });
    strip.vcas = vec![format!("{id}vca")];
    // C: the room, 5.1, ducked by the voice, its pitch up.
    let mut c = track(
        &format!("{id}c"),
        TrackFormat::FiveOne,
        vec![clip(&format!("{id}c1"), room, 0.0, 0, 200)],
    );
    c.audio.strip.track_fx = Some(Ducker {
        on: true,
        sources: vec![SourceRef::Track(format!("{id}a"))],
        amount_db: 12.0,
        ..Ducker::default()
    });
    c.audio.strip.inserts.push(insert(
        "fx1",
        Effect::Pitch(PitchFx {
            semitones: 2,
            cents: 0.0,
            dry_wet: 100.0,
        }),
    ));
    // D: the music again at 0.8 times its speed with its pitch kept, and a Pitch of its own a
    // fourth down (T4c), then at 1.5 times.
    let mut slow = clip(&format!("{id}d1"), music, 0.0, 0, 100);
    slow.speed = Speed { num: 4, den: 5 };
    slow.pitch_correction = true;
    slow.effects.push(insert(
        "",
        Effect::Pitch(PitchFx {
            semitones: -5,
            cents: 0.0,
            dry_wet: 100.0,
        }),
    ));
    let mut fast = clip(&format!("{id}d2"), music, 1.0, 100, 100);
    fast.speed = Speed { num: 3, den: 2 };
    let mut d = track(&format!("{id}d"), TrackFormat::Stereo, vec![slow, fast]);
    d.audio
        .strip
        .inserts
        .push(insert("fx1", Effect::Gain(GainFx { gain_db: -3.0 })));
    let mut effects = Bus::first();
    effects.id = format!("{id}fx");
    effects.name = "Effects".into();
    effects.format = BusFormat::Stereo;
    effects.strip.outputs = vec![Output {
        bus: "B1".into(),
        on: true,
    }];
    effects.strip.eq.on = true;
    Timeline {
        id: id.into(),
        name: id.into(),
        rate: columbina_model::Rate { num: 25, den: 1 },
        width: 64,
        height: 36,
        sample_rate: 48_000,
        tracks: vec![pictures, a, b, c, d],
        grade: columbina_model::Grade::default(),
        buses: vec![Bus::first(), effects],
        vcas: vec![Vca {
            id: format!("{id}vca"),
            name: "VCA 1".into(),
            fader_db: -2.0,
            mute: false,
            solo: false,
            vcas: vec![],
            lanes: columbina_model::Lanes::new(),
        }],
        track_groups: vec![],
        audio: columbina_model::TimelineAudio::default(),
    }
}

async fn action(client: &mut Client, method: &str, mut params: Value) -> Value {
    params["monitor"] = json!("timeline");
    client.result(method, Some(params)).await
}

/// Plays the timeline loaded from frame `frame` until `seconds` of its sound were heard, and
/// pauses; checks that nothing was short, dropped or swapped; the sample of the timeline that
/// the session started at (its digest's `audio_sample` less `audio_samples`, at one rate): the
/// first frame of its capture. A session started as the device is first made ready starts where
/// the clock is by then, a sample or so after the frame.
async fn play(engine: &super::Engine, client: &mut Client, frame: i64, seconds: f64) -> usize {
    client.take_notifications();
    action(client, "transport.seek", json!({"frame": frame})).await;
    client
        .until_state("at the frame", |state| {
            state["monitor"] == "timeline" && state["shown"] == frame && state["exact"] == true
        })
        .await;
    action(client, "transport.play", json!({"speed": 1})).await;
    client
        .until_state("its sound", |state| {
            state["monitor"] == "timeline" && state["sound"] == "on"
        })
        .await;
    #[allow(clippy::cast_possible_truncation, reason = "a count of samples")]
    let wanted = (seconds * 48_000.0) as i64;
    let started = std::time::Instant::now();
    let digest = loop {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let digest = action(client, "transport.frame_digest", json!({})).await;
        if digest["audio_samples"].as_i64().unwrap_or(0) >= wanted {
            break digest;
        }
        let told: Vec<String> = engine
            .stderr()
            .lines()
            .filter(|line| line.contains("sound"))
            .map(str::to_owned)
            .collect();
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "{digest} {told:?}"
        );
    };
    action(client, "transport.pause", json!({})).await;
    assert_eq!(digest["audio_short_runs"], 0, "{digest}");
    assert_eq!(digest["audio_underruns"], 0, "{digest}");
    assert_eq!(digest["audio_swaps"], 0, "{digest}");
    assert_eq!(digest["audio_capture_dropped"], 0, "{digest}");
    let from = digest["audio_sample"].as_i64().unwrap() - digest["audio_samples"].as_i64().unwrap();
    usize::try_from(from).unwrap()
}

/// Exports `timeline` (frames `range`) as PCM of floats to `path` and waits for it; the time it
/// took.
async fn export(
    client: &mut Client,
    timeline: &str,
    path: &Path,
    range: Option<(i64, i64)>,
) -> Duration {
    let mut params = json!({"timeline": timeline, "path": path, "video": null,
        "audio": {"codec": "pcm_f32le"}});
    if let Some((from, to)) = range {
        params["range"] = json!({"from": from, "to": to});
    }
    let started = std::time::Instant::now();
    let job = client.result("render.start", Some(params)).await["job"].clone();
    let mut pending = client.take_notifications().into_iter();
    loop {
        let message = match pending.next() {
            Some(value) => value,
            None => client.message().await,
        };
        if message["method"] == "render.progress"
            && message["params"]["job"] == job
            && message["params"]["state"] != "running"
        {
            assert_eq!(message["params"]["state"], "done", "{message}");
            return started.elapsed();
        }
    }
}

/// The samples of a WAV of floats that the engine wrote, interleaved.
fn floats_of(path: &Path) -> Vec<f32> {
    let mut decoder = columbina_media::AudioDecoder::open_unmixed(path, 0, None).unwrap();
    let mut data = vec![];
    while let Some(run) = decoder.next().unwrap() {
        data.extend(run.data);
    }
    data
}

/// The frames of a capture, left and right interleaved.
fn captured(path: &Path) -> Vec<f32> {
    std::fs::read(path)
        .unwrap()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|word| f32::from_le_bytes(*word))
        .collect()
}

/// How many of `a` less `b` are not 0, and the largest of them.
fn difference(a: &[f32], b: &[f32]) -> (usize, f32) {
    a.iter().zip(b).fold((0, 0.0_f32), |(count, most), (a, b)| {
        let d = a - b;
        (count + usize::from(d != 0.0), most.max(d.abs()))
    })
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "two timelines, each played twice and exported twice"
)]
async fn the_sound_played_less_the_sound_exported_is_nothing_but_zeros() {
    if columbina_compose::gpu_for_tests().is_none() {
        // Under the gate a machine without a card fails this test instead of passing it unrun.
        assert_ne!(
            std::env::var("COLUMBINA_REQUIRE_GPU").as_deref(),
            Ok("1"),
            "COLUMBINA_REQUIRE_GPU=1 and no graphics card"
        );
        return;
    }
    let dir = super::project_dir();
    let sounds = tempfile::tempdir().unwrap();
    let paths = media(sounds.path());
    let capture = dir.path().join("heard.f32");
    let sound = format!("capture={}", capture.display());
    let engine = super::engine(
        dir.path(),
        &["--sound", &sound, "--test-hooks", "--no-proxies"],
    )
    .await;
    let mut client = engine.client("agent").await;
    let added = client
        .result("media.import", Some(json!({ "paths": paths })))
        .await["added"]
        .clone();
    let media: Vec<String> = added
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap().to_owned())
        .collect();
    client.take_notifications();
    let mut runs = vec![];
    for (id, quick) in [("nullquick", true), ("nullfull", false)] {
        let made = timeline(id, &media, quick);
        client
            .apply(json!([{"op": "add_timeline", "timeline": made}]))
            .await;
        let project: Project =
            serde_json::from_value(project(&mut client).await["project"].clone()).unwrap();
        let index = project.timelines.iter().position(|t| t.id == id).unwrap();
        let settle = Graph::build(&project, index, Options::new(4096, Purpose::Export))
            .unwrap()
            .settle();
        client
            .result(
                "transport.load",
                Some(json!({"monitor": "timeline", "timeline": id})),
            )
            .await;
        // From the start for 5 s, and from 2 s for 3 s: each session into a file of its own.
        let first = play(&engine, &mut client, 0, 5.2).await;
        let second = play(&engine, &mut client, 50, 3.2).await;
        println!("{id}: sessions from samples {first} and {second}");
        assert!(first < 480, "{first}");
        assert_eq!(second, 2 * RATE, "a seek while paused plays from the frame");
        let whole = dir.path().join(format!("{id}.wav"));
        let range = dir.path().join(format!("{id}-range.wav"));
        let took = export(&mut client, id, &whole, None).await;
        let took_range = export(&mut client, id, &range, Some((50, 200))).await;
        println!(
            "{id}: settles in {settle} samples; exported in {took:?}, from 2 s in {took_range:?}"
        );
        runs.push((id, settle, whole, range, first));
    }
    drop(client);
    drop(engine);
    let files: Vec<std::path::PathBuf> = std::iter::once(capture.clone())
        .chain((2..=4).map(|n| std::path::PathBuf::from(format!("{}.{n}", capture.display()))))
        .collect();
    if let Ok(keep) = std::env::var("COLUMBINA_NULL_KEEP") {
        for entry in std::fs::read_dir(dir.path()).unwrap() {
            let entry = entry.unwrap();
            let _ = std::fs::copy(entry.path(), Path::new(&keep).join(entry.file_name()));
        }
    }
    for (run, (id, settle, whole, range, first)) in runs.into_iter().enumerate() {
        let from_start = captured(&files[2 * run]);
        let from_middle = captured(&files[2 * run + 1]);
        let whole = floats_of(&whole);
        let range = floats_of(&range);
        assert_eq!(whole.len(), 2 * 8 * RATE, "{id}");
        assert_eq!(range.len(), 2 * 6 * RATE, "{id}");
        assert!(
            from_start.len() >= 2 * 5 * RATE,
            "{id}: {}",
            from_start.len()
        );
        assert!(
            from_middle.len() >= 2 * 3 * RATE,
            "{id}: {}",
            from_middle.len()
        );
        assert!(
            whole.iter().any(|sample| sample.abs() > 0.05),
            "{id}: sound"
        );
        // Played from the start, less the export of the whole: zeros, every sample of 5 s.
        let (count, most) = difference(
            &from_start[..2 * 5 * RATE],
            &whole[2 * first..2 * (first + 5 * RATE)],
        );
        println!("{id}: from the start, {count} samples differ, by {most} at most");
        assert_eq!(count, 0, "{id}: from the start");
        // Played from 2 s, less the export of the range from 2 s: zeros too.
        let (count, most) = difference(&from_middle[..2 * 3 * RATE], &range[..2 * 3 * RATE]);
        println!("{id}: from 2 s, {count} samples differ, by {most} at most");
        assert_eq!(count, 0, "{id}: from 2 s");
        // Played from 2 s, less the export of the whole from 2 s: under −90 dBFS where the
        // preroll is the settling, under −60 dBFS where the settling is longer than 10 s.
        let floor = if settle as usize <= 10 * RATE {
            3.16e-5
        } else {
            1e-3
        };
        let (_, most) = difference(
            &from_middle[..2 * 3 * RATE],
            &whole[2 * 2 * RATE..2 * 5 * RATE],
        );
        println!("{id}: from 2 s against the whole, {most} at most (floor {floor})");
        assert!(most < floor, "{id}: {most}");
    }
}
