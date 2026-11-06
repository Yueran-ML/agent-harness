//! What an analysis measures of a timeline (plan T9b): a strip over a range of the timeline,
//! whatever latency the paths after it have; a clip as its track takes it, after its Volume and
//! before its pan, through its equalizer and effects, without its fades and what its track does
//! (a gain found is a change of its Volume: a second Normalize finds none), the other tracks not read; and Normalize of two clips at −30
//! and −20 LUFS to −23, Independent and Relative.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "samples and levels of tests"
)]

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;

use columbina_audio::analysis::{
    alone, analyze, normalize_clips, run_strip, run_strips, Analysis, Analyzer, Level, SetLevel,
    Volume,
};
use columbina_audio::feed::FeedError;
use columbina_audio::graph::{node_of, Context, Graph, Made, Options, Owner, Point, Purpose};
use columbina_audio::node::Layout;
use columbina_audio::standards::Measure;
use columbina_audio::testing::Delay;
use columbina_model::audio::fx::GainFx;
use columbina_model::{AudioParam, Effect, Insert, Project, TrackKind};

const RATE: u32 = 48_000;

/// The sample project with its track of sound only, A1 (clip c2, 4 s from the start of the
/// timeline and of its media), and A2 like it (clip c3), both out to B1.
fn two_tracks() -> Project {
    let mut project = Project::sample();
    let timeline = &mut project.timelines[0];
    timeline
        .tracks
        .retain(|track| track.kind == TrackKind::Audio);
    let mut second = timeline.tracks[0].clone();
    second.id = "A2".into();
    second.clips[0].id = "c3".into();
    timeline.tracks.push(second);
    project
}

/// A feed of `signal(clip, sample of the media, channel)`, and whether each clip was read.
type Signal = fn(&str, i64, usize) -> f32;

fn feed_of(
    graph: &Graph,
    signal: Signal,
    read: Rc<Cell<[bool; 2]>>,
) -> impl FnMut(usize, Range<i64>, &mut [&mut [f32]]) -> Result<(), FeedError> {
    let ids: Vec<String> = graph.clips().iter().map(|c| c.clip.clone()).collect();
    move |clip, range, out| {
        let id = &ids[clip];
        let mut seen = read.get();
        seen[usize::from(id == "c3")] = true;
        read.set(seen);
        for (channel, out) in out.iter_mut().enumerate() {
            for (sample, value) in (range.start..).zip(out.iter_mut()) {
                *value = signal(id, sample, channel);
            }
        }
        Ok(())
    }
}

/// A sine of 1 kHz at a peak of `db` dBFS at sample `n` of 48 kHz.
fn sine(db: f64, n: i64) -> f32 {
    let t = n as f64 / f64::from(RATE);
    (10_f64.powf(db / 20.0) * (std::f64::consts::TAU * 1000.0 * t).sin()) as f32
}

/// A Gain of `n` dB is a delay of `n` samples; the rest as [`node_of`].
fn delays(effect: &Effect, context: &Context) -> Option<Made> {
    match effect {
        Effect::Gain(gain) => Some(Made {
            reading: None,
            node: Box::new(Delay::new(gain.gain_db as usize)),
            param: |_| None,
            settle: 0,
            switched: false,
        }),
        _ => node_of(effect, context),
    }
}

fn gain_insert(db: f64) -> Insert {
    Insert {
        id: "fx1".into(),
        on: true,
        effect: Effect::Gain(GainFx { gain_db: db }),
    }
}

/// The analysis of clip `clip` of `project` as Normalize measures it, the clips' sound
/// `signal`; and its samples of the timeline.
fn of_clip(project: &Project, clip: &str, signal: Signal) -> (Analysis, Range<i64>) {
    let (alone, track, range) = alone(project, 0, clip).unwrap();
    let mut graph = Graph::build(&alone, 0, Options::new(4096, Purpose::Analysis)).unwrap();
    let strip = graph.strip_of(&Owner::Track(track)).unwrap();
    let read = Rc::new(Cell::new([false; 2]));
    let mut feed = feed_of(&graph, signal, Rc::clone(&read));
    let mut analyzer = Analyzer::new(RATE, graph.layout_of(strip), true);
    let ran = run_strip(
        &mut graph,
        &mut feed,
        (strip, Point::BeforeFader),
        range.clone(),
        &mut |sound| {
            analyzer.take(sound);
            true
        },
    )
    .unwrap();
    assert!(ran);
    // Only the clip's own track is read.
    assert_eq!(read.get(), [clip == "c2", clip == "c3"]);
    (analyzer.finish(), range)
}

#[test]
fn a_strip_is_measured_over_its_range_whatever_the_latency_of_the_paths_after_it() {
    let mut project = two_tracks();
    // A2 takes 64 samples (a test delay): the outputs wait for it, and A1 is 64 ahead of them.
    project.timelines[0].tracks[1]
        .audio
        .strip
        .inserts
        .push(gain_insert(64.0));
    project.timelines[0].tracks[0].audio.strip.fader_db = -6.0;
    let options = Options::new(4096, Purpose::Analysis).with_maker(delays);
    let mut graph = Graph::build(&project, 0, options).unwrap();
    assert_eq!(graph.latency(), 64);
    let a1 = graph.strip_of(&Owner::Track("A1".into())).unwrap();
    let a2 = graph.strip_of(&Owner::Track("A2".into())).unwrap();
    let b1 = graph.strip_of(&Owner::Bus("B1".into())).unwrap();
    assert_eq!(
        (
            graph.strip_lead(a1),
            graph.strip_lead(a2),
            graph.strip_lead(b1)
        ),
        (64, 0, 0)
    );
    let ramp: Signal = |clip, n, _| {
        if clip == "c2" {
            (n % 4801) as f32 / 4801.0
        } else {
            0.0
        }
    };
    let mut feed = feed_of(&graph, ramp, Rc::new(Cell::new([false; 2])));
    for (strip, point, scale) in [
        (a1, Point::BeforeFader, 1.0),
        (a1, Point::AfterFader, 10_f32.powf(-6.0 / 20.0)),
        (b1, Point::AfterFader, 10_f32.powf(-6.0 / 20.0)),
    ] {
        let mut got = Vec::new();
        run_strip(
            &mut graph,
            &mut feed,
            (strip, point),
            10_000..30_000,
            &mut |sound| {
                got.extend_from_slice(sound[0]);
                true
            },
        )
        .unwrap();
        assert_eq!(got.len(), 20_000);
        for (n, &value) in (10_000..).zip(&got) {
            let want = ramp("c2", n, 0) * scale;
            assert!(
                (value - want).abs() <= 1e-6,
                "{strip} {point:?} at {n}: {value} {want}"
            );
        }
    }
    // The three in one run, each over the same range of the timeline whatever its lead.
    let mut got = [Vec::new(), Vec::new(), Vec::new()];
    let points = [
        (a1, Point::BeforeFader),
        (a2, Point::AfterFader),
        (b1, Point::AfterFader),
    ];
    let ran = run_strips(
        &mut graph,
        &mut feed,
        &points,
        10_000..30_000,
        &mut |at, sound| {
            got[at].extend_from_slice(sound[0]);
            true
        },
    )
    .unwrap();
    assert!(ran);
    let fader = 10_f32.powf(-6.0 / 20.0);
    for (index, scale) in [(0, 1.0), (1, 0.0), (2, fader)] {
        assert_eq!(got[index].len(), 20_000);
        for (n, &value) in (10_000..).zip(&got[index]) {
            let want = ramp("c2", n, 0) * scale;
            assert!(
                (value - want).abs() <= 1e-6,
                "point {index} at {n}: {value} {want}"
            );
        }
    }
    // A sink that says no more stops the run.
    let mut blocks = 0;
    let ran = run_strip(
        &mut graph,
        &mut feed,
        (a1, Point::AfterFader),
        0..100_000,
        &mut |_| {
            blocks += 1;
            blocks < 3
        },
    )
    .unwrap();
    assert!(!ran);
    assert_eq!(blocks, 3);
}

#[test]
fn a_clip_is_measured_after_its_volume_and_before_its_pan_alone_on_its_track() {
    let mut project = two_tracks();
    let track = &mut project.timelines[0].tracks[0];
    let clip = &mut track.clips[0];
    clip.gain_db = AudioParam::Fixed(-12.0);
    clip.pan = AudioParam::Fixed(0.7);
    clip.fade_in = 25;
    let strip = &mut track.audio.strip;
    strip.trim_db = 5.0;
    strip.invert = true;
    strip.fader_db = -10.0;
    strip.inserts.push(gain_insert(6.0));
    strip.eq.on = true;
    strip.eq.gain_db = -4.0;
    strip.dynamics.on = true;
    strip.dynamics.compressor.on = true;
    track.audio.mute = true;
    // A2 loud, soloed: neither is heard in the measure of c2.
    project.timelines[0].tracks[1].audio.solo = true;
    let signal: Signal = |clip, n, _| {
        if clip == "c2" {
            sine(-23.0, n)
        } else {
            0.5
        }
    };
    let (measured, range) = of_clip(&project, "c2", signal);
    assert_eq!(range, 0..192_000);
    // The clip's sound after its Volume of −12 dB, to the bit: nothing of its pan or fade, nor of
    // its track.
    let gain = 10_f32.powf(-12.0 / 20.0);
    let source: Vec<f32> = (0..192_000).map(|n| sine(-23.0, n) * gain).collect();
    let want = analyze(RATE, Layout::Stereo, true, &[&source, &source]);
    assert_eq!(measured, want);
    // Its equalizer comes before its Volume, and is heard (the coordinator's decision for T9b):
    // its gain of +6 dB on a tone that its bands, at no gain, leave as it is.
    project.timelines[0].tracks[0].clips[0].eq.on = true;
    project.timelines[0].tracks[0].clips[0].eq.gain_db = 6.0;
    let (equalized, _) = of_clip(&project, "c2", signal);
    println!(
        "c2 alone: I {:?}, TP {}; through its EQ of +6 dB: I {:?}",
        measured.integrated, measured.true_peak, equalized.integrated
    );
    assert!((measured.integrated.unwrap() + 35.0).abs() < 0.05);
    assert!((equalized.integrated.unwrap() + 29.0).abs() < 0.05);
    // So are its effects, which come before its Volume too: the project measured keeps them.
    project.timelines[0].tracks[0].clips[0]
        .effects
        .push(gain_insert(6.0));
    let (effected, _) = of_clip(&project, "c2", signal);
    assert!(
        (effected.integrated.unwrap() + 23.0).abs() < 0.05,
        "{effected:?}"
    );
    let (kept, _, _) = alone(&project, 0, "c2").unwrap();
    let clip = &kept.timelines[0].tracks[0].clips[0];
    assert_eq!(clip.effects.len(), 1);
    assert!(clip.eq.on);
    assert_eq!(clip.gain_db, AudioParam::Fixed(-12.0));
    assert_eq!(kept.timelines[0].tracks[0].audio.strip.inserts.len(), 0);
    // The project itself is as it was.
    assert_eq!(
        project.timelines[0].tracks[0].clips[0].gain_db,
        AudioParam::Fixed(-12.0)
    );
    // No such clip, or one of a track of picture.
    assert!(alone(&project, 0, "c9").is_none());
    assert!(alone(&Project::sample(), 0, "c1").is_none());
}

#[test]
fn normalize_brings_clips_at_minus_30_and_minus_20_lufs_to_minus_23_independent_and_relative() {
    let project = two_tracks();
    let signal: Signal = |clip, n, _| {
        if clip == "c2" {
            sine(-30.0, n)
        } else {
            sine(-20.0, n)
        }
    };
    let (quiet, first) = of_clip(&project, "c2", signal);
    let (loud, second) = of_clip(&project, "c3", signal);
    // Together, one after the other in the order of the timeline.
    let quiet_sound: Vec<f32> = first.map(|n| sine(-30.0, n)).collect();
    let loud_sound: Vec<f32> = second.map(|n| sine(-20.0, n)).collect();
    let mut both = Analyzer::new(RATE, Layout::Stereo, false);
    both.take(&[&quiet_sound, &quiet_sound]);
    both.take(&[&loud_sound, &loud_sound]);
    let together = both.finish();
    let level = Level::Loudness(Measure::Gated);
    let volumes = [Volume::fixed(0.0), Volume::fixed(0.0)];
    let each = [quiet, loud];
    let independent = normalize_clips(
        level,
        SetLevel::Independent,
        -23.0,
        &each,
        &together,
        &volumes,
    );
    let relative = normalize_clips(level, SetLevel::Relative, -23.0, &each, &together, &volumes);
    println!("Independent {independent:?}, Relative {relative:?}");
    assert!((independent[0].gain.unwrap() - 7.0).abs() <= 0.1);
    assert!((independent[1].gain.unwrap() + 3.0).abs() <= 0.1);
    // Relative: the two as one programme (−22.6 LUFS), one gain for both.
    let gain = relative[0].gain.unwrap();
    assert_eq!(relative[1].gain, Some(gain));
    let energy = 10_f64.powf(-3.0).midpoint(10_f64.powf(-2.0));
    assert!(
        (gain - (-23.0 - 10.0 * energy.log10())).abs() <= 0.1,
        "{gain}"
    );
    assert!(independent.iter().chain(&relative).all(|n| !n.clamped));
    // Relative leaves a silent clip as it is (the review of T9b).
    let silence = vec![0.0_f32; 96_000];
    let silent = analyze(RATE, Layout::Stereo, false, &[&silence, &silence]);
    let with_silence = normalize_clips(
        level,
        SetLevel::Relative,
        -23.0,
        &[each[0], silent],
        &together,
        &volumes,
    );
    assert_eq!(with_silence[0].gain, relative[0].gain);
    assert_eq!(with_silence[1].gain, None);
    // A Volume of keys is as low and as high as they go.
    let keys: AudioParam = serde_json::from_value(serde_json::json!({"keys": [
        {"at": {"ticks": 0, "scale": 48000}, "value": -6.0},
        {"at": {"ticks": 48000, "scale": 48000}, "value": 20.0}]}))
    .unwrap();
    assert_eq!(
        Volume::of(&keys),
        Volume {
            lowest: -6.0,
            highest: 20.0
        }
    );
    let clamped = normalize_clips(
        level,
        SetLevel::Independent,
        -23.0,
        &each,
        &together,
        &[Volume::of(&keys), Volume::fixed(0.0)],
    );
    assert!(clamped[0].clamped && !clamped[1].clamped);
}

/// The analysis of clip c2 of [`two_tracks`] whose Volume is `volume`, its sound a sine of −23
/// dBFS.
fn with_volume(volume: AudioParam) -> Analysis {
    let mut project = two_tracks();
    project.timelines[0].tracks[0].clips[0].gain_db = volume;
    of_clip(&project, "c2", |_, n, _| sine(-23.0, n)).0
}

/// `volume` moved by `db`, keys and all (as `nudge_clip_gain` moves it).
fn moved(volume: &AudioParam, db: f64) -> AudioParam {
    match volume {
        AudioParam::Fixed(value) => AudioParam::Fixed(value + db),
        AudioParam::Keys(keys) => AudioParam::Keys(
            keys.iter()
                .map(|key| columbina_model::AudioKey {
                    value: key.value + db,
                    ..key.clone()
                })
                .collect(),
        ),
    }
}

#[test]
fn a_clip_normalized_twice_is_moved_once_whatever_its_volume() {
    let keys: AudioParam = serde_json::from_value(serde_json::json!({"keys": [
        {"at": {"ticks": 0, "scale": 48000}, "value": -10.0},
        {"at": {"ticks": 96000, "scale": 48000}, "value": 2.0}]}))
    .unwrap();
    for (volume, level, target) in [
        (AudioParam::Fixed(6.0), Level::SamplePeak, -9.0),
        (
            AudioParam::Fixed(6.0),
            Level::Loudness(Measure::Gated),
            -23.0,
        ),
        (keys, Level::SamplePeak, -9.0),
    ] {
        let first = with_volume(volume.clone());
        let gain = normalize_clips(
            level,
            SetLevel::Independent,
            target,
            std::slice::from_ref(&first),
            &first,
            &[Volume::of(&volume)],
        )[0]
        .gain
        .unwrap();
        // The Volume moved by the gain, the whole curve: the clip comes to the target, and a
        // second Normalize moves it by nothing.
        let after = with_volume(moved(&volume, gain));
        let again = normalize_clips(
            level,
            SetLevel::Independent,
            target,
            std::slice::from_ref(&after),
            &after,
            &[Volume::of(&moved(&volume, gain))],
        )[0]
        .gain
        .unwrap();
        println!("{volume:?} by {level:?} to {target}: {gain} dB, then {again} dB");
        assert!(again.abs() < 0.01, "{again}");
        let reached = level.of_analysis(&after).unwrap();
        assert!((reached - target).abs() < 0.01, "{reached}");
    }
    // The gain found for a clip at +6 dB is 6 dB less than for one at 0 dB.
    let at_zero = with_volume(AudioParam::Fixed(0.0)).sample_peak;
    let at_six = with_volume(AudioParam::Fixed(6.0)).sample_peak;
    assert!((at_six - at_zero - 6.0).abs() < 0.01);
}
