//! The sound of an export (plan T6): the graph of playback, the bus rendered as it is, the same
//! to the bit as what playback gives the device (the null test of convention 4, here on the
//! threads of T5 run on a clock of their own; the engine's own is in the server's tests), the
//! gain and the limiter of Audio Normalization after it, another rate, and Normalize and Optimize
//! to a standard, measured by ffmpeg's `ebur128` too (convention 5).

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::float_cmp,
    reason = "samples and levels of tests; the gain is checked to the bit"
)]

use columbina_audio::analysis::{analyze, Analysis, Level};
use columbina_audio::deliver::{measure, normalization, Aim, Bounce, Finish, Mode, Render, Rerate};
use columbina_audio::feed::ClipFeed;
use columbina_audio::graph::{route, Graph, Options, Purpose};
use columbina_audio::standards::{self, Measure};
use columbina_audio::stream::Resampler;
use columbina_audio::Layout;
use columbina_model::audio::fx::{
    DeEsserFx, DeHummerFx, GainFx, LimiterFx, NoiseReductionFx, PitchFx, SoftClipperFx,
    StereoWidthFx,
};
use columbina_model::audio::BusFormat;
use columbina_model::{Effect, Insert, Output, Project, SourceRef};

mod session;

use session::{one_track, sixteen_tracks, Sim, Tone, RATE};

const TONE: Tone = Tone {
    hz: 440.0,
    amplitude: 0.2,
};

// --- Helpers ---------------------------------------------------------------------------------

fn graph(project: &Project, purpose: Purpose) -> Graph {
    Graph::build(project, 0, Options::new(4096, purpose)).unwrap()
}

/// Runs of 1 to 4096 samples, the same each time for a seed.
fn cuts(seed: u64) -> impl FnMut() -> usize {
    let mut state = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    move || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let bits = (state >> 33) as usize;
        if bits.is_multiple_of(3) {
            1 + bits % 7
        } else {
            1 + bits % 4096
        }
    }
}

/// `frames` samples of the export of output `output` (the monitored bus where none) of
/// `project` from `at`, with `finish` after it, at `rate`, taken in runs that `cut` says: a
/// vector for each channel.
fn exported(
    project: &Project,
    output: Option<&str>,
    (at, frames): (i64, usize),
    finish: Finish,
    rate: u32,
    cut: &mut dyn FnMut() -> usize,
) -> Vec<Vec<f32>> {
    let mut graph = graph(project, Purpose::Export);
    let output = output.map_or_else(
        || graph.monitor().unwrap(),
        |bus| graph.output_of(bus).unwrap(),
    );
    let mut tone = TONE;
    let mut render = Render::start(&mut graph, &mut tone, (output, finish), at, rate).unwrap();
    let mut channels = vec![Vec::new(); render.layout().channels()];
    let mut done = 0;
    while done < frames {
        let count = cut().min(frames - done);
        for (into, run) in channels
            .iter_mut()
            .zip(render.take(&mut graph, &mut tone, count).unwrap())
        {
            into.extend(run);
        }
        done += count;
    }
    channels
}

/// The monitored bus of `project` folded into two sides as playback folds it for the device
/// (`play/engine.rs`: each side from 0, each channel of a gain that is not 0 added in its
/// order), and clipped as the device's copy is: left and right, interleaved.
fn as_the_device_hears(project: &Project, channels: &[Vec<f32>]) -> Vec<f32> {
    let graph = graph(project, Purpose::Export);
    let layout = graph.outputs()[graph.monitor().unwrap()].layout;
    let matrix = route::convert(layout, Layout::Stereo);
    let frames = channels[0].len();
    let mut out = Vec::with_capacity(2 * frames);
    for n in 0..frames {
        for side in 0..2 {
            let mut sum = 0.0_f32;
            for (channel, samples) in channels.iter().enumerate() {
                let gain = matrix.gain(side, channel);
                if gain != 0.0 {
                    sum += gain * samples[n];
                }
            }
            out.push(sum.clamp(-1.0, 1.0));
        }
    }
    out
}

/// What playback gives the device of `project` from `at` over `runs` runs of 10 ms (left and
/// right interleaved; frame `n` is sample `at + n` of the timeline), and that nothing was short.
fn played(project: &Project, at: i64, runs: usize) -> Vec<f32> {
    let mut sim = Sim::of(project, at, TONE);
    for _ in 0..runs {
        sim.tick();
    }
    assert_eq!(sim.counters.underruns(), 0, "the clips were read in time");
    assert_eq!(sim.control.shared.short(), 0, "the device was never short");
    assert_eq!(sim.counters.swaps(), 0);
    sim.played
}

/// The largest difference of two runs of samples.
fn most_apart(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f32::max)
}

/// Sixteen tracks of every kind (T5's) with every effect of T10, T12 and T13 in an insert, a clip
/// at 0.8 times its speed with its pitch kept (T13), a Limiter on the bus of 5.1, and that bus out
/// to a second main output of 5.1 too.
fn every_effect() -> Project {
    let mut project = sixteen_tracks();
    let timeline = &mut project.timelines[0];
    let insert = |effect: Effect| Insert {
        id: String::new(),
        on: true,
        effect,
    };
    let inserts: [(usize, Effect); 9] = [
        (
            0,
            Effect::Limiter(LimiterFx {
                input_db: 6.0,
                ceiling_db: -3.0,
                ..LimiterFx::default()
            }),
        ),
        (1, Effect::NoiseReduction(NoiseReductionFx::default())),
        (2, Effect::Gain(GainFx { gain_db: 3.0 })),
        (5, Effect::DeEsser(DeEsserFx::default())),
        (6, Effect::DeHummer(DeHummerFx::default())),
        (7, Effect::SoftClipper(SoftClipperFx::default())),
        (9, Effect::StereoWidth(StereoWidthFx::default())),
        (13, Effect::Gain(GainFx { gain_db: -4.0 })),
        (
            10,
            Effect::Pitch(PitchFx {
                semitones: 3,
                cents: 20.0,
                dry_wet: 100.0,
            }),
        ),
    ];
    let slow = &mut timeline.tracks[4].clips[0];
    slow.speed = columbina_model::Speed { num: 4, den: 5 };
    slow.pitch_correction = true;
    for (n, (track, effect)) in inserts.into_iter().enumerate() {
        let mut insert = insert(effect);
        insert.id = format!("fx{}", n + 1);
        timeline.tracks[track].audio.strip.inserts.push(insert);
    }
    let mut b3 = columbina_model::Bus::first();
    b3.id = "B3".into();
    b3.name = "Main 2".into();
    b3.format = BusFormat::FiveOne;
    timeline.buses.push(b3);
    let b2 = timeline
        .buses
        .iter_mut()
        .find(|bus| bus.id == "B2")
        .unwrap();
    b2.strip.outputs.push(Output {
        bus: "B3".into(),
        on: true,
    });
    b2.strip.inserts.push(Insert {
        id: "fx1".into(),
        on: true,
        effect: Effect::Limiter(LimiterFx::default()),
    });
    project
}

/// `project` with the clip of each track 30 s long, of media of 60 s.
fn long(mut project: Project) -> Project {
    for track in &mut project.timelines[0].tracks {
        track.clips[0].length = 30 * 25;
    }
    project.media[0].facts.duration = columbina_model::Time {
        ticks: 60 * i64::from(RATE),
        scale: RATE,
    };
    project
}

// --- An export is what playback plays --------------------------------------------------------

#[test]
fn an_export_is_what_playback_gives_the_device_to_the_bit() {
    for (name, project) in [
        ("one track", one_track()),
        ("sixteen tracks", sixteen_tracks()),
        ("every effect", every_effect()),
    ] {
        for at in [0, 96_000 + 17] {
            let frames = 48_000;
            let played = played(&project, at, frames / 480 + 8);
            let exported = exported(
                &project,
                None,
                (at, frames),
                Finish::default(),
                RATE,
                &mut cuts(7),
            );
            let heard = as_the_device_hears(&project, &exported);
            let differ = heard
                .iter()
                .zip(&played)
                .filter(|(a, b)| *a - *b != 0.0)
                .count();
            assert_eq!(differ, 0, "{name} from {at}: {differ} samples differ");
            assert!(
                heard.iter().any(|sample| sample.abs() > 0.01),
                "{name}: there is sound"
            );
        }
    }
}

#[test]
fn an_export_of_a_range_comes_within_the_floor_of_one_of_the_whole_after_its_preroll() {
    let mut slow = one_track();
    // A Limiter whose release is 1 s settles in 14 s, more than the 10 s of the preroll: within
    // −60 dBFS (the coordinator's note on T6), on sound 12 dB over its ceiling, not 30.
    slow.timelines[0].tracks[0]
        .audio
        .strip
        .inserts
        .push(Insert {
            id: "fx1".into(),
            on: true,
            effect: Effect::Limiter(LimiterFx {
                input_db: 12.0,
                ceiling_db: -12.0,
                release_ms: 1000.0,
                ..LimiterFx::default()
            }),
        });
    let mut kinds = [0, 0];
    // From 12 s, the preroll of a range starts after the start of the timeline.
    let from = 12 * RATE as usize;
    for (name, project) in [
        ("sixteen tracks", long(sixteen_tracks())),
        ("every effect", long(every_effect())),
        ("a slow limiter", slow),
    ] {
        let settle = graph(&project, Purpose::Export).settle();
        // Within −90 dBFS where the preroll is as long as the settling, −60 where it is not.
        let floor = if settle <= 10 * RATE { 3.16e-5 } else { 1e-3 };
        kinds[usize::from(settle <= 10 * RATE)] += 1;
        let whole = exported(
            &project,
            None,
            (0, from + RATE as usize),
            Finish::default(),
            RATE,
            &mut || 4096,
        );
        let range = exported(
            &project,
            None,
            (from as i64, RATE as usize),
            Finish::default(),
            RATE,
            &mut || 4096,
        );
        let mut most = 0.0_f32;
        for (whole, range) in whole.iter().zip(&range) {
            most = most.max(most_apart(&whole[from..], range));
        }
        println!("{name}: settles in {settle} samples, apart by {most} after the preroll");
        assert!(most < floor, "{name}: {most}");
        if settle <= 10 * RATE {
            // Playback from the middle comes as near to the export of the whole.
            let played = played(&project, from as i64, 108);
            let heard = as_the_device_hears(&project, &whole);
            let apart = most_apart(&heard[2 * from..], &played[..2 * RATE as usize]);
            assert!(apart < floor, "{name}: playback {apart}");
        }
    }
    assert!(
        kinds[0] > 0 && kinds[1] > 0,
        "projects that settle in more than 10 s and in less: {kinds:?}"
    );
}

#[test]
fn a_main_output_is_exported_in_its_own_channels_however_it_is_cut() {
    let project = every_effect();
    let mut graph = graph(&project, Purpose::Export);
    let output = graph.output_of("B3").unwrap();
    assert_eq!(graph.outputs()[output].layout, Layout::Surround51);
    assert!(graph.outputs()[output].main);
    let mut tone = TONE;
    graph.start(4_000, &mut tone).unwrap();
    let mut direct = vec![Vec::new(); 6];
    for _ in 0..12 {
        graph.process(&mut tone, 4096).unwrap();
        for (channel, direct) in direct.iter_mut().enumerate() {
            direct.extend_from_slice(graph.output(output).channel(channel));
        }
    }
    for seed in [1, 2] {
        let got = exported(
            &project,
            Some("B3"),
            (4_000, 12 * 4096),
            Finish::default(),
            RATE,
            &mut cuts(seed),
        );
        assert_eq!(got.len(), 6);
        for (got, want) in got.iter().zip(&direct) {
            assert!(got
                .iter()
                .zip(want)
                .all(|(a, b)| a.to_bits() == b.to_bits()));
        }
        assert!(got[2].iter().any(|sample| sample.abs() > 0.001), "C sounds");
    }
}

#[test]
fn the_listen_of_a_sidechain_is_never_exported() {
    let mut listening = sixteen_tracks();
    let strip = &mut listening.timelines[0].tracks[3].audio.strip;
    assert_eq!(
        strip.dynamics.compressor.sidechain.source,
        Some(SourceRef::Track("A2".into()))
    );
    strip.dynamics.compressor.sidechain.listen = true;
    let without = sixteen_tracks();
    let take = |project: &Project| {
        exported(
            project,
            None,
            (0, RATE as usize),
            Finish::default(),
            RATE,
            &mut || 4096,
        )
    };
    assert_eq!(take(&listening), take(&without));
    // Playback hears the sidechain.
    let heard = played(&listening, 0, 50);
    let plain = played(&without, 0, 50);
    assert!(most_apart(&heard, &plain) > 0.01);
}

// --- What is put after the bus ---------------------------------------------------------------

#[test]
fn a_gain_after_the_bus_scales_it_and_no_finish_leaves_it_to_the_bit() {
    let project = one_track();
    let plain = exported(
        &project,
        None,
        (0, 9600),
        Finish::default(),
        RATE,
        &mut || 960,
    );
    let mut graph = graph(&project, Purpose::Export);
    let mut tone = TONE;
    let output = graph.monitor().unwrap();
    graph.start(0, &mut tone).unwrap();
    graph.process(&mut tone, 4096).unwrap();
    assert_eq!(&plain[0][..4096], &graph.output(output).channel(0)[..4096]);
    let raised = exported(
        &project,
        None,
        (0, 9600),
        Finish {
            gain_db: 6.0,
            ceiling_db: None,
        },
        RATE,
        &mut || 960,
    );
    let gain = 10.0_f64.powf(6.0 / 20.0) as f32;
    for (raised, plain) in raised.iter().zip(&plain) {
        assert!(raised.iter().zip(plain).all(|(r, p)| *r == p * gain));
    }
}

#[test]
fn the_limiter_after_the_bus_keeps_each_sound_at_its_sample_and_its_peaks_under_the_ceiling() {
    let project = one_track();
    let finish = Finish {
        gain_db: 18.0,
        ceiling_db: Some(-1.0),
    };
    let hot = exported(
        &project,
        None,
        (0, 2 * RATE as usize),
        finish,
        RATE,
        &mut cuts(3),
    );
    let reading = analyze(RATE, Layout::Stereo, true, &[&hot[0], &hot[1]]);
    assert!(reading.true_peak <= -1.0, "{}", reading.true_peak);
    assert!(reading.true_peak > -1.5, "{}", reading.true_peak);
    // Quiet sound passes as it is: the lookahead is made up, sample n is still sample n.
    let quiet = Finish {
        gain_db: 0.0,
        ceiling_db: Some(-1.0),
    };
    let limited = exported(&project, None, (777, 9600), quiet, RATE, &mut cuts(4));
    let plain = exported(
        &project,
        None,
        (777, 9600),
        Finish::default(),
        RATE,
        &mut cuts(5),
    );
    assert_eq!(limited, plain);
}

// --- Another rate ------------------------------------------------------------------------------

#[test]
fn another_rate_is_made_as_the_device_makes_it_on_every_channel_and_not_clipped() {
    let project = one_track();
    // Stereo from 48 kHz to 44.1 kHz: as T5's stream makes it for a device (the same sound
    // within −1…1).
    let got = exported(
        &project,
        None,
        (1_000, 44_100),
        Finish::default(),
        44_100,
        &mut cuts(9),
    );
    let mut graph = graph(&project, Purpose::Export);
    let output = graph.monitor().unwrap();
    let mut tone = TONE;
    let mut resampler = Resampler::new(RATE, 44_100, 2, 4096);
    resampler.start(1_000);
    graph.start(resampler.first_input(), &mut tone).unwrap();
    let mut device = vec![0.0_f32; 2 * 44_100];
    let mut made = 0;
    while made < 44_100 {
        made += resampler.pull(&mut device[2 * made..]);
        if made == 44_100 {
            break;
        }
        let count = resampler.room().clamp(1, 4096);
        graph.process(&mut tone, count).unwrap();
        let sound = graph.output(output);
        resampler.push(&sound.channel(0)[..count], &sound.channel(1)[..count]);
    }
    for (n, frame) in device.as_chunks::<2>().0.iter().enumerate() {
        assert_eq!([got[0][n], got[1][n]], [frame[0], frame[1]], "frame {n}");
    }
    // Six channels of 96 kHz from 48 kHz, louder than 1: the tone at its pitch, not clipped.
    let mut hot = Rerate::new(48_000, 96_000, 6);
    hot.start(0);
    let first = hot.first_input();
    let tone: Vec<f32> = (first..first + 50_000)
        .map(|n| 2.0 * (std::f64::consts::TAU * 1_000.0 * n as f64 / 48_000.0).sin() as f32)
        .collect();
    let mut out = vec![Vec::new(); 6];
    let mut at = 0;
    while out[0].len() < 96_000 {
        if hot.pull(96_000 - out[0].len(), &mut out) == 0 {
            let run = [&tone[at..at + 1000]; 6];
            hot.push(&run);
            at += 1000;
        }
    }
    let peak = out[5].iter().fold(0.0_f32, |most, x| most.max(x.abs()));
    assert!((1.98..2.01).contains(&peak), "{peak}");
    let rises = out[5][96..96_000 - 96]
        .windows(2)
        .filter(|pair| pair[0] < 0.0 && pair[1] >= 0.0)
        .count();
    assert!(rises.abs_diff(998) <= 1, "{rises}");
    // The same rate: nothing done.
    let same = exported(
        &project,
        None,
        (500, 4800),
        Finish::default(),
        RATE,
        &mut cuts(2),
    );
    let plain = exported(
        &project,
        None,
        (500, 4800),
        Finish::default(),
        RATE,
        &mut || 4800,
    );
    assert_eq!(same, plain);
}

// --- Audio Normalization ------------------------------------------------------------------------

/// An analysis whose integrated loudness is `integrated` and true peak `true_peak`.
fn reading(integrated: Option<f64>, true_peak: f64) -> Analysis {
    let mut reading = analyze(
        RATE,
        Layout::Stereo,
        true,
        &[&vec![0.0; 4800], &vec![0.0; 4800]],
    );
    reading.integrated = integrated;
    reading.true_peak = true_peak;
    reading
}

fn ebu() -> Aim {
    let standard = standards::by_id("ebu-r128").unwrap();
    Aim {
        level: Level::of(standard),
        target: f64::from(standard.target.unwrap()),
        true_peak: standard.true_peak.map(f64::from),
    }
}

#[test]
fn normalize_scales_the_gain_to_whichever_of_the_target_and_the_true_peak_comes_first() {
    // −30 LUFS with peaks at −10 dBTP: +7 meets the target at −3 dBTP.
    let found = normalization(Mode::Normalize, &ebu(), |_| {
        Ok::<_, ()>(reading(Some(-30.0), -10.0))
    })
    .unwrap();
    assert!((found.finish.gain_db - 7.0).abs() < 1e-9);
    assert_eq!(found.finish.ceiling_db, None);
    assert!(found.met);
    assert_eq!(found.measured, 1);
    // Peaks at −5 dBTP: the true peak's −1 comes first, at +4; the target is not met.
    let found = normalization(Mode::Normalize, &ebu(), |_| {
        Ok::<_, ()>(reading(Some(-30.0), -5.0))
    })
    .unwrap();
    assert!((found.finish.gain_db - 4.0).abs() < 1e-9);
    assert!(!found.met);
    // Too loud: lowered to the target.
    let found = normalization(Mode::Normalize, &ebu(), |_| {
        Ok::<_, ()>(reading(Some(-12.0), 0.5))
    })
    .unwrap();
    assert!((found.finish.gain_db + 11.0).abs() < 1e-9);
    // Silence is left as it is.
    let found = normalization(Mode::Normalize, &ebu(), |_| {
        Ok::<_, ()>(reading(None, -200.0))
    })
    .unwrap();
    assert_eq!(found.finish, Finish::default());
    assert!(!found.met);
}

#[test]
fn optimize_limits_where_the_gain_takes_the_true_peak_over_gives_the_loudness_back_once_and_lowers_the_ceiling_twice(
) {
    // Room under the true peak: a gain alone, measured once.
    let found = normalization(Mode::Optimize, &ebu(), |_| {
        Ok::<_, ()>(reading(Some(-30.0), -10.0))
    })
    .unwrap();
    assert_eq!(
        found.finish,
        Finish {
            gain_db: 7.0,
            ceiling_db: None
        }
    );
    // Peaks at −5: the limiter at −1. It takes 0.4 LU off: the gain is raised by 0.4, once; a
    // true peak of −0.7 after it lowers the ceiling to −1.3, and −0.9 to −1.4; the third reading
    // is kept, and told as not met where it still goes over.
    let mut asked = vec![];
    let mut answers = vec![
        reading(Some(-30.0), -5.0),
        reading(Some(-23.4), -1.0),
        reading(Some(-23.02), -0.7),
        reading(Some(-23.05), -0.9),
        reading(Some(-23.08), -0.95),
    ]
    .into_iter();
    let found = normalization(Mode::Optimize, &ebu(), |finish| {
        asked.push(finish);
        Ok::<_, ()>(answers.next().unwrap())
    })
    .unwrap();
    let ceilings: Vec<Option<f64>> = asked.iter().map(|finish| finish.ceiling_db).collect();
    let gains: Vec<f64> = asked.iter().map(|finish| finish.gain_db).collect();
    assert_eq!(ceilings.len(), 5);
    assert_eq!(ceilings[0], None);
    let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
    assert!(near(gains[1], 7.0) && near(ceilings[1].unwrap(), -1.0));
    assert!(near(gains[2], 7.4) && near(ceilings[2].unwrap(), -1.0));
    assert!(near(gains[3], 7.4) && near(ceilings[3].unwrap(), -1.3));
    assert!(near(gains[4], 7.4) && near(ceilings[4].unwrap(), -1.4));
    assert_eq!(found.measured, 5);
    assert_eq!(found.finish, asked[4]);
    assert!(!found.met, "−0.95 dBTP is over −1");
    // Within 0.1 LU and under the ceiling at once: no more.
    let mut answers = vec![reading(Some(-30.0), -5.0), reading(Some(-23.05), -1.02)].into_iter();
    let found = normalization(Mode::Optimize, &ebu(), |_| {
        Ok::<_, ()>(answers.next().unwrap())
    })
    .unwrap();
    assert!(found.met);
    assert_eq!(found.measured, 2);
    // A standard without a highest true peak (BS.1770-4): a gain only.
    let standard = standards::by_id("bs1770-4").unwrap();
    let aim = Aim {
        level: Level::Loudness(Measure::Gated),
        target: -16.0,
        true_peak: standard.true_peak.map(f64::from),
    };
    let found = normalization(Mode::Optimize, &aim, |_| {
        Ok::<_, ()>(reading(Some(-30.0), -1.0))
    })
    .unwrap();
    assert_eq!(found.finish.ceiling_db, None);
    assert!((found.finish.gain_db - 14.0).abs() < 1e-9);
}

// --- Normalized for real, beside ffmpeg --------------------------------------------------------

/// The ffmpeg of the tests: `FFMPEG`, else the LGPL build of the toolchain (convention 5), else
/// the one on the path; `None` when none runs, which fails the test where
/// `COLUMBINA_REQUIRE_FFMPEG` is `1` (the gate) instead of letting it pass unmeasured.
fn ffmpeg_program() -> Option<String> {
    let found = any_ffmpeg();
    assert!(
        found.is_some() || std::env::var("COLUMBINA_REQUIRE_FFMPEG").as_deref() != Ok("1"),
        "COLUMBINA_REQUIRE_FFMPEG=1 and no ffmpeg runs"
    );
    found
}

/// [`ffmpeg_program`], without the requirement.
fn any_ffmpeg() -> Option<String> {
    let lgpl = r"F:\agent harness\_review\toolchain\ffmpeg-lgpl-8.1\bin\ffmpeg.exe";
    let program = std::env::var("FFMPEG").unwrap_or_else(|_| {
        if std::path::Path::new(lgpl).exists() {
            lgpl.to_owned()
        } else {
            "ffmpeg".to_owned()
        }
    });
    std::process::Command::new(&program)
        .arg("-version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|_| program)
}

/// A WAV of 32-bit floats of `channels` at `rate`.
fn wav(channels: &[Vec<f32>], rate: u32) -> Vec<u8> {
    let frames = channels[0].len();
    let count = channels.len();
    let data = u32::try_from(frames * count * 4).unwrap();
    let mut wav = Vec::with_capacity(44 + data as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&3_u16.to_le_bytes());
    wav.extend_from_slice(&u16::try_from(count).unwrap().to_le_bytes());
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&(rate * 4 * u32::try_from(count).unwrap()).to_le_bytes());
    wav.extend_from_slice(&u16::try_from(4 * count).unwrap().to_le_bytes());
    wav.extend_from_slice(&32_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data.to_le_bytes());
    for n in 0..frames {
        for channel in channels {
            wav.extend_from_slice(&channel[n].to_le_bytes());
        }
    }
    wav
}

/// ffmpeg's integrated loudness and true peak of `channels` at `rate`.
fn ffmpeg(channels: &[Vec<f32>], rate: u32, name: &str) -> Option<(f64, f64)> {
    let program = ffmpeg_program()?;
    let path = std::env::temp_dir().join(format!(
        "columbina-deliver-{name}-{}.wav",
        std::process::id()
    ));
    std::fs::write(&path, wav(channels, rate)).unwrap();
    let output = std::process::Command::new(program)
        .args(["-hide_banner", "-nostats", "-i"])
        .arg(&path)
        .args(["-af", "ebur128=peak=true", "-f", "null", "-"])
        .output()
        .unwrap();
    let _ = std::fs::remove_file(&path);
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

/// Pink noise (Paul Kellet's filter of white noise), each channel its own; with `bursts`, 5 ms of
/// it 2.5 times as loud every half second, peaks that a gain of +16 dB takes over −1 dBTP; its RMS
/// 0.0225 (−30 LUFS in stereo). Precomputed for the first 6 s of its media, silence
/// elsewhere.
struct Pink(Vec<Vec<f32>>);

impl Pink {
    fn new(bursts: bool) -> Self {
        let channels = (0..2_u32)
            .map(|channel| {
                let mut state = 0x2545_f491_u32 ^ (channel + 1).wrapping_mul(0x9E37_79B9);
                let mut b = [0.0_f64; 3];
                let mut pink: Vec<f64> = (0..6 * RATE)
                    .map(|_| {
                        state ^= state << 13;
                        state ^= state >> 17;
                        state ^= state << 5;
                        let white = f64::from(state) / f64::from(u32::MAX) - 0.5;
                        b[0] = 0.997_65 * b[0] + white * 0.099_046;
                        b[1] = 0.963 * b[1] + white * 0.296_516_4;
                        b[2] = 0.57 * b[2] + white * 1.052_691_3;
                        b[0] + b[1] + b[2] + white * 0.184_8
                    })
                    .collect();
                for (n, x) in pink.iter_mut().enumerate() {
                    if bursts && n % 24_000 < 240 {
                        *x *= 2.5;
                    }
                }
                let rms = (pink.iter().map(|x| x * x).sum::<f64>() / pink.len() as f64).sqrt();
                for x in &mut pink {
                    *x *= 0.0225 / rms;
                }
                pink.into_iter().map(|x| x as f32).collect()
            })
            .collect();
        Pink(channels)
    }
}

impl ClipFeed for Pink {
    fn read(
        &mut self,
        _: usize,
        range: std::ops::Range<i64>,
        out: &mut [&mut [f32]],
    ) -> Result<(), columbina_audio::feed::FeedError> {
        for (channel, out) in out.iter_mut().enumerate() {
            let pink = &self.0[channel % 2];
            for (n, sample) in out.iter_mut().enumerate() {
                let at = range.start + n as i64;
                *sample = usize::try_from(at)
                    .ok()
                    .and_then(|at| pink.get(at))
                    .copied()
                    .unwrap_or(0.0);
            }
        }
        Ok(())
    }
}

/// Pink noise of −30 LUFS (with bursts in it if `bursts`) optimized to −14 LUFS and −1 dBTP, and
/// the file as the export writes it read by ffmpeg (the plan's I = −14.0 ± 0.2, TP ≤ −1.0 + 0.2;
/// convention 5's I ± 0.1, TP ± 0.2 beside our reading); what was found.
fn optimized_to_minus_14(bursts: bool) -> columbina_audio::deliver::Normalization {
    let project = one_track();
    let frames = 4 * RATE as usize;
    let pink = Pink::new(bursts);
    let measured = |finish: Finish| {
        let mut graph = graph(&project, Purpose::Analysis);
        let output = graph.monitor().unwrap();
        measure(
            &mut graph,
            &mut Pink(pink.0.clone()),
            (output, finish),
            (0, frames),
            RATE,
        )
    };
    let aim = Aim {
        level: Level::Loudness(Measure::Gated),
        target: -14.0,
        true_peak: Some(-1.0),
    };
    let before = measured(Finish::default()).unwrap();
    let integrated = before.integrated.unwrap();
    println!("before: {integrated:.2} LUFS, {:.2} dBTP", before.true_peak);
    assert!((integrated + 30.0).abs() < 0.5, "{integrated}");
    let found = normalization(Mode::Optimize, &aim, measured).unwrap();
    println!("{found:?}");
    assert!(found.met);
    let after = found
        .after
        .unwrap_or(analyze(RATE, Layout::Stereo, true, &[&[0.0], &[0.0]]));
    // The file, as the export writes it, read by ffmpeg.
    let mut graph = graph(&project, Purpose::Export);
    let output = graph.monitor().unwrap();
    let mut feed = Pink(pink.0.clone());
    let mut render = Render::start(&mut graph, &mut feed, (output, found.finish), 0, RATE).unwrap();
    let file = render.take(&mut graph, &mut feed, frames).unwrap();
    let ours = analyze(RATE, Layout::Stereo, true, &[&file[0], &file[1]]);
    if found.after.is_some() {
        assert_eq!(ours.integrated, after.integrated);
    }
    let Some((integrated, true_peak)) = ffmpeg(&file, RATE, "optimize") else {
        eprintln!("no ffmpeg: not compared");
        return found;
    };
    println!(
        "ffmpeg: {integrated} LUFS, {true_peak} dBTP; ours {:?} LUFS, {} dBTP",
        ours.integrated, ours.true_peak
    );
    assert!((integrated + 14.0).abs() <= 0.2, "{integrated}");
    assert!(true_peak <= -1.0 + 0.2, "{true_peak}");
    assert!((integrated - ours.integrated.unwrap()).abs() <= 0.1);
    assert!((true_peak - ours.true_peak).abs() <= 0.2);
    found
}

#[test]
fn optimize_brings_pink_noise_of_minus_30_lufs_to_minus_14_under_minus_1_dbtp() {
    let found = optimized_to_minus_14(false);
    // Its peaks stay under −1 dBTP at −14 LUFS: a gain alone.
    assert_eq!(found.finish.ceiling_db, None);
    assert!((found.finish.gain_db - 16.0).abs() < 0.5);
}

#[test]
fn optimize_limits_the_bursts_of_pink_noise_and_gives_back_what_the_limiter_took() {
    let found = optimized_to_minus_14(true);
    assert!(found.finish.ceiling_db.is_some());
    let gain = found.finish.gain_db;
    let before = found.before.integrated.unwrap();
    assert!(
        gain > -14.0 - before,
        "the loudness taken by the limiter given back: {gain}"
    );
    assert!(found.measured >= 2);
}

#[test]
fn normalize_brings_a_tone_to_the_target_of_a_standard_as_ffmpeg_reads_it() {
    let project = one_track();
    let frames = 4 * RATE as usize;
    let standard = standards::by_id("ebu-r128").unwrap();
    let aim = Aim {
        level: Level::of(standard),
        target: f64::from(standard.target.unwrap()),
        true_peak: standard.true_peak.map(f64::from),
    };
    let measured = |finish: Finish| {
        let mut graph = graph(&project, Purpose::Analysis);
        let output = graph.monitor().unwrap();
        let mut tone = TONE;
        measure(&mut graph, &mut tone, (output, finish), (0, frames), 44_100)
    };
    let found = normalization(Mode::Normalize, &aim, measured).unwrap();
    assert!(found.met);
    let mut graph = graph(&project, Purpose::Export);
    let output = graph.monitor().unwrap();
    let mut tone = TONE;
    let mut render =
        Render::start(&mut graph, &mut tone, (output, found.finish), 0, 44_100).unwrap();
    let file = render.take(&mut graph, &mut tone, 4 * 44_100).unwrap();
    let Some((integrated, _)) = ffmpeg(&file, 44_100, "normalize") else {
        return;
    };
    println!(
        "ffmpeg: {integrated} LUFS after {:+.2} dB",
        found.finish.gain_db
    );
    assert!((integrated + 23.0).abs() <= 0.1, "{integrated}");
}

#[test]
fn a_bounce_starts_where_it_is_told() {
    // Two starts of one bounce, as an export measured and then written: the same sound.
    let project = every_effect();
    let mut graph = graph(&project, Purpose::Export);
    let output = graph.monitor().unwrap();
    let finish = Finish {
        gain_db: 3.0,
        ceiling_db: Some(-2.0),
    };
    let mut bounce = Bounce::new(&graph, output, finish);
    let mut tone = TONE;
    let mut runs = vec![];
    for _ in 0..2 {
        bounce.start(&mut graph, &mut tone, 9_000).unwrap();
        let mut run = vec![];
        for _ in 0..4 {
            run.extend_from_slice(
                &bounce.next(&mut graph, &mut tone, 1000).unwrap().channel(1)[..1000],
            );
        }
        runs.push(run);
    }
    assert!(runs[0]
        .iter()
        .zip(&runs[1])
        .all(|(a, b)| a.to_bits() == b.to_bits()));
}
