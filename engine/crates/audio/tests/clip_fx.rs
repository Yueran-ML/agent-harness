//! The effects of a clip (p. 3855, the interaction reference §6.3, §10.1; T4c): after its
//! equalizer and before its volume and pan, made as an insert's are, their latency made up so that
//! the clip still sounds on its own samples; the same Gain on a clip as on the track that holds
//! only it, to the bit; a Pitch an octave up; the same bits however the run is cut into blocks,
//! and from a start in the middle once it settles; a Listen only in a preview; an effect that
//! cannot work on the channels of its track skipped; and no memory taken on the thread of the mix.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "sample numbers and levels of tests"
)]

mod common;

use std::ops::Range;

use columbina_audio::feed::{ClipFeed, FeedError};
use columbina_audio::fx::{limiter, pitch};
use columbina_audio::graph::{Graph, Options, Purpose, Warp};
use columbina_audio::testing::{realtime, CountingAllocator, Rng};
use columbina_audio::{Layout, Node, Spec};
use columbina_model::audio::fx::{
    DeEsserFx, GainFx, LimiterFx, NoiseReductionFx, PitchFx, StereoWidthFx,
};
use columbina_model::audio::TrackFormat;
use columbina_model::{Effect, Insert, Project, Speed, Time, TrackKind};

use common::{cents, peak_frequency, RATE};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

const SECOND: usize = RATE as usize;
/// The samples of a frame of the sample timeline (25 frames a second).
const FRAME: usize = SECOND / 25;
/// Where the clip starts: a second in, frame 25.
const START: usize = SECOND;

/// An effect of a clip, on.
fn on(effect: Effect) -> Insert {
    Insert {
        id: String::new(),
        on: true,
        effect,
    }
}

/// The sample project with its track of sound only (A1, stereo), its clip c2 from 1 s for
/// `frames` frames, with the effects `effects`; its media a minute long.
fn project(frames: i64, effects: Vec<Insert>) -> Project {
    let mut project = Project::sample();
    project.timelines[0]
        .tracks
        .retain(|track| track.kind == TrackKind::Audio);
    project.media[0].facts.duration = Time {
        ticks: 60 * 15_360,
        scale: 15_360,
    };
    let clip = &mut project.timelines[0].tracks[0].clips[0];
    clip.start = (START / FRAME) as i64;
    clip.length = frames;
    clip.effects = effects;
    project
}

fn built(project: &Project, purpose: Purpose) -> Graph {
    Graph::build(project, 0, Options::new(4096, purpose)).unwrap()
}

/// A tone of `hertz` and amplitude `amplitude` in the media, at its sample `n` (the right channel
/// a radian on).
fn tone(hertz: f64, amplitude: f64) -> impl Fn(i64, usize) -> f32 {
    move |n, channel| {
        let phase = std::f64::consts::TAU * hertz * n as f64 / f64::from(RATE)
            + if channel == 1 { 1.0 } else { 0.0 };
        (amplitude * phase.sin()) as f32
    }
}

/// A feed of `signal(sample of the media, channel)`.
fn feed(
    signal: impl Fn(i64, usize) -> f32,
) -> impl FnMut(usize, Range<i64>, &mut [&mut [f32]]) -> Result<(), FeedError> {
    move |_, range, out| {
        for (channel, out) in out.iter_mut().enumerate() {
            for (n, sample) in out.iter_mut().enumerate() {
                *sample = signal(range.start + n as i64, channel);
            }
        }
        Ok(())
    }
}

/// `frames` samples of the monitor from sample `at`, in blocks as `cut` says.
fn play(
    graph: &mut Graph,
    feed: &mut dyn ClipFeed,
    (at, frames): (i64, usize),
    cut: &mut dyn FnMut() -> usize,
) -> Vec<Vec<f32>> {
    graph.start(at, feed).unwrap();
    let output = graph.monitor().unwrap();
    let channels = graph.output(output).channels();
    let mut played = vec![Vec::with_capacity(frames); channels];
    let mut done = 0;
    while done < frames {
        let n = cut().min(frames - done).min(graph.max_block());
        graph.process(feed, n).unwrap();
        for (channel, played) in played.iter_mut().enumerate() {
            played.extend_from_slice(&graph.output(output).channel(channel)[..n]);
        }
        done += n;
    }
    played
}

fn whole(graph: &mut Graph, feed: &mut dyn ClipFeed, at: i64, frames: usize) -> Vec<Vec<f32>> {
    play(graph, feed, (at, frames), &mut || 4096)
}

/// The project exported whole from 0 for `frames` samples, its media a tone of 440 Hz at
/// `amplitude`.
fn exported(project: &Project, amplitude: f64, frames: usize) -> Vec<Vec<f32>> {
    let mut graph = built(project, Purpose::Export);
    whole(&mut graph, &mut feed(tone(440.0, amplitude)), 0, frames)
}

/// The first sample at which `a` and `b` differ in their bits, if any.
fn differs(a: &[f32], b: &[f32]) -> Option<usize> {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .position(|(a, b)| a.to_bits() != b.to_bits())
}

/// The largest difference between `a` and `b`.
fn most_apart(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f32::max)
}

#[test]
fn a_gain_on_a_clip_sounds_as_the_same_gain_on_the_track_that_holds_only_it_to_the_bit() {
    let gain = Effect::Gain(GainFx { gain_db: -7.5 });
    let on_clip = project(100, vec![on(gain.clone())]);
    let mut on_track = project(100, vec![]);
    on_track.timelines[0].tracks[0]
        .audio
        .strip
        .inserts
        .push(Insert {
            id: "fx1".into(),
            on: true,
            effect: gain,
        });
    let frames = START + 100 * FRAME + SECOND;
    let (clip, track) = (
        exported(&on_clip, 0.5, frames),
        exported(&on_track, 0.5, frames),
    );
    let plain = exported(&project(100, vec![]), 0.5, frames);
    for channel in 0..2 {
        assert_eq!(differs(&clip[channel], &track[channel]), None, "{channel}");
        assert!(differs(&clip[channel], &plain[channel]).is_some());
    }
}

#[test]
fn a_pitch_on_a_clip_moves_it_an_octave_and_it_still_sounds_on_its_own_samples() {
    let octave = Effect::Pitch(PitchFx {
        semitones: 12,
        ..PitchFx::default()
    });
    let project_up = project(150, vec![on(octave)]);
    let graph = built(&project_up, Purpose::Export);
    // The clip makes up its latency itself: the graph is not delayed by it.
    assert_eq!(graph.latency(), 0);
    let end = START + 150 * FRAME;
    let out = exported(&project_up, 0.5, end + SECOND);
    assert!(out[0][..START].iter().all(|&s| s == 0.0), "before it");
    assert!(out[0][end..].iter().all(|&s| s == 0.0), "after it");
    assert!(
        out[0][START..START + 64].iter().any(|s| s.abs() > 0.05),
        "its first samples: {:?}",
        &out[0][START..START + 8]
    );
    let measured = peak_frequency(&out[0][START + SECOND / 2..end - SECOND / 2]);
    assert!(cents(measured, 880.0).abs() < 5.0, "{measured} Hz");
    // All dry, a Pitch is its input delayed by its latency, which the clip makes up: the clip
    // sounds as it does without it, on the same samples, to the bit; and so through a Limiter
    // that does not touch a quiet tone, 64 samples late.
    let plain = exported(&project(150, vec![]), 0.1, end + SECOND);
    for effects in [
        vec![on(Effect::Pitch(PitchFx {
            semitones: 12,
            dry_wet: 0.0,
            ..PitchFx::default()
        }))],
        vec![on(Effect::Limiter(LimiterFx::default()))],
    ] {
        let what = format!("{effects:?}");
        let out = exported(&project(150, effects), 0.1, end + SECOND);
        for channel in 0..2 {
            assert_eq!(differs(&out[channel], &plain[channel]), None, "{what}");
        }
    }
}

#[test]
fn a_pitch_after_a_limiter_on_a_clip_keeps_to_the_grid_of_the_timeline_as_on_its_track() {
    // A Pitch starts its stretch again every 4 s of the timeline. Behind a Limiter (64 samples
    // late, a quiet tone through it untouched), on the clip or on the track that holds only it,
    // the same once the Pitch has settled from where the clip starts.
    let effects = || {
        vec![
            Effect::Limiter(LimiterFx::default()),
            Effect::Pitch(PitchFx {
                semitones: 5,
                ..PitchFx::default()
            }),
        ]
    };
    let on_clip = project(250, effects().into_iter().map(on).collect());
    let mut on_track = project(250, vec![]);
    on_track.timelines[0].tracks[0].audio.strip.inserts = effects()
        .into_iter()
        .enumerate()
        .map(|(k, effect)| Insert {
            id: format!("fx{}", k + 1),
            on: true,
            effect,
        })
        .collect();
    let end = START + 250 * FRAME;
    let (clip, track) = (exported(&on_clip, 0.1, end), exported(&on_track, 0.1, end));
    let settled = START + pitch::settle_for(&pitch::Settings::default(), RATE) as usize;
    for channel in 0..2 {
        assert_eq!(
            differs(&clip[channel][settled..], &track[channel][settled..]),
            None,
            "{channel}"
        );
    }
}

#[test]
fn the_effects_of_a_clip_count_in_the_settling_with_their_latency() {
    let graph = built(
        &project(
            100,
            vec![on(Effect::Pitch(PitchFx {
                semitones: 3,
                ..PitchFx::default()
            }))],
        ),
        Purpose::Export,
    );
    let mut node = pitch::Pitch::default();
    node.prepare(&Spec::new(RATE, 4096, Layout::Stereo))
        .unwrap();
    assert_eq!(
        graph.settle(),
        pitch::settle_for(&pitch::Settings::default(), RATE) + node.latency()
    );
    let slow = LimiterFx {
        release_ms: 1000.0,
        ..LimiterFx::default()
    };
    let graph = built(
        &project(100, vec![on(Effect::Limiter(slow.clone()))]),
        Purpose::Export,
    );
    let settings = limiter::Settings {
        release_ms: 1000.0,
        ..limiter::Settings::default()
    };
    assert_eq!(graph.settle(), limiter::settle_for(&settings, RATE) + 64);
}

/// The frames of the clip of [`busy`]: 16 s, from 1 s to 17 s.
const LONG: i64 = 400;

/// Effects of every kind of latency on a clip: a hot tone into a Limiter, a Pitch, a De-Esser and
/// a Noise Reduction where `noise` (which settles over half a minute, beyond a preroll).
fn busy(stretched: bool, noise: bool) -> Project {
    let mut project = project(
        LONG,
        vec![
            on(Effect::Limiter(LimiterFx {
                input_db: 12.0,
                release_ms: 300.0,
                ..LimiterFx::default()
            })),
            on(Effect::Pitch(PitchFx {
                semitones: -3,
                cents: 40.0,
                dry_wet: 70.0,
            })),
            on(Effect::DeEsser(DeEsserFx::default())),
        ],
    );
    if noise {
        project.timelines[0].tracks[0].clips[0]
            .effects
            .push(on(Effect::NoiseReduction(NoiseReductionFx::default())));
    }
    if stretched {
        let clip = &mut project.timelines[0].tracks[0].clips[0];
        clip.speed = Speed { num: 4, den: 5 };
        clip.pitch_correction = true;
    }
    project
}

/// A tone that comes and goes, at `n` of the media.
fn wavering(n: i64, channel: usize) -> f32 {
    tone(377.0, 0.5)(n, channel) * if n % 5_000 < 2_500 { 1.0 } else { 0.3 }
}

#[test]
fn the_effects_of_a_clip_do_not_depend_on_how_the_run_is_cut_into_blocks() {
    for stretched in [false, true] {
        let mut graph = built(&busy(stretched, true), Purpose::Export);
        let frames = START + LONG as usize * FRAME + SECOND / 2;
        let reference = whole(&mut graph, &mut feed(wavering), 0, frames);
        assert!(reference[0].iter().any(|s| s.abs() > 0.1));
        for seed in [1, 2, 3] {
            let mut rng = Rng::new(seed);
            let cut = play(&mut graph, &mut feed(wavering), (0, frames), &mut || {
                if rng.next_u64().is_multiple_of(4) {
                    rng.range(1, 4)
                } else {
                    rng.log_range(4096)
                }
            });
            for channel in 0..2 {
                assert_eq!(
                    differs(&reference[channel], &cut[channel]),
                    None,
                    "stretched {stretched}, seed {seed}"
                );
            }
        }
    }
}

#[test]
fn a_run_started_in_the_middle_of_a_clip_with_effects_sounds_as_one_started_before_it() {
    for stretched in [false, true] {
        let project = busy(stretched, false);
        let mut graph = built(&project, Purpose::Export);
        assert!(graph.settle() < 10 * RATE, "{}", graph.settle());
        let frames = START + LONG as usize * FRAME;
        let from_0 = whole(&mut graph, &mut feed(wavering), 0, frames);
        // In the middle, past a point where a Pitch starts its stretch again (12 s), and in a
        // crossfade of one (16 s), each so late that its effects start after the clip does; then
        // just before the clip, where they start as from 0.
        let preroll = graph.preroll() as usize;
        for at in [12 * SECOND + 77, 16 * SECOND - 100, START - 3] {
            assert!(at < START || at - preroll > START, "{at} {preroll}");
            let late = whole(&mut graph, &mut feed(wavering), at as i64, frames - at);
            for channel in 0..2 {
                let most = most_apart(&from_0[channel][at..], &late[channel]);
                assert!(
                    most < 3.16e-5,
                    "stretched {stretched}, from {at}: {most} apart"
                );
                if at < START {
                    assert_eq!(differs(&from_0[channel][at..], &late[channel]), None);
                }
            }
        }
    }
}

#[test]
fn a_listen_of_an_effect_of_a_clip_is_heard_in_a_preview_only() {
    let listening = project(
        100,
        vec![on(Effect::DeEsser(DeEsserFx {
            listen: true,
            ..DeEsserFx::default()
        }))],
    );
    let deaf = project(100, vec![on(Effect::DeEsser(DeEsserFx::default()))]);
    let frames = START + 100 * FRAME;
    let heard = |project: &Project, purpose| {
        let mut graph = built(project, purpose);
        whole(&mut graph, &mut feed(tone(440.0, 0.5)), 0, frames)
    };
    let plain = heard(&deaf, Purpose::Export);
    for purpose in [Purpose::Export, Purpose::Analysis] {
        assert_eq!(differs(&heard(&listening, purpose)[0], &plain[0]), None);
    }
    assert!(differs(&heard(&listening, Purpose::Preview)[0], &plain[0]).is_some());
}

#[test]
fn an_effect_that_cannot_work_on_the_channels_of_its_track_is_skipped() {
    let mut mono = project(
        100,
        vec![on(Effect::StereoWidth(StereoWidthFx {
            width: 0.0,
            ..StereoWidthFx::default()
        }))],
    );
    let audio = &mut mono.timelines[0].tracks[0].audio;
    audio.format = TrackFormat::Mono;
    audio.channels = 1;
    let mut plain = mono.clone();
    plain.timelines[0].tracks[0].clips[0].effects.clear();
    let frames = START + 100 * FRAME;
    let (with, without) = (exported(&mono, 0.5, frames), exported(&plain, 0.5, frames));
    assert_eq!(differs(&with[0], &without[0]), None);
    assert!(with[0].iter().any(|s| s.abs() > 0.1));
}

#[test]
fn a_clip_with_effects_plays_ten_seconds_with_no_memory() {
    for stretched in [false, true] {
        let mut graph = Graph::build(
            &busy(stretched, true),
            0,
            Options::new(480, Purpose::Preview),
        )
        .unwrap();
        let mut read = feed(wavering);
        graph.start(SECOND as i64 / 2, &mut read).unwrap();
        let mut device = vec![0.0_f32; 480 * 2];
        // Through the clip's start, where its effects start, and its end.
        let ((), counted) = realtime(|| {
            for _ in 0..1_000 {
                graph.process(&mut read, 480).unwrap();
                graph.write_device(graph.monitor().unwrap(), 480, &mut device);
            }
        });
        assert!(counted.none(), "stretched {stretched}: {counted:?}");
    }
}

#[test]
fn a_clip_with_effects_played_from_elsewhere_or_at_another_speed_sounds_through_them() {
    // At a speed of 1 from 3 s of the timeline (a grain of scrubbing, T18): the export there once
    // the effects have settled, which the preroll lets them.
    let project = busy(false, false);
    let shift = 3 * SECOND as i64;
    let frames = 4 * SECOND;
    let export = exported_by(&project, frames + shift as usize);
    let mut graph = built(&project, Purpose::Export);
    graph.set_warp(Warp::new(1, 0, shift));
    let shifted = whole(&mut graph, &mut feed(wavering), 0, frames);
    for channel in 0..2 {
        let most = most_apart(&shifted[channel], &export[channel][shift as usize..]);
        assert!(most < 3.16e-5, "{most} apart");
    }
    // Twice as fast either way: the effects run on the samples out, the same however they are
    // cut into blocks.
    for speed in [2, -2] {
        let mut graph = built(&project, Purpose::Export);
        let at = 6 * SECOND as i64;
        graph.set_warp(Warp::new(speed, at, at));
        let reference = whole(&mut graph, &mut feed(wavering), at, SECOND);
        assert!(reference[0].iter().any(|s| s.abs() > 0.05), "{speed}");
        let mut rng = Rng::new(7);
        let cut = play(&mut graph, &mut feed(wavering), (at, SECOND), &mut || {
            rng.log_range(4096)
        });
        assert_eq!(differs(&reference[0], &cut[0]), None, "{speed}");
    }
}

/// The project exported whole from 0 for `frames` samples, its media [`wavering`].
fn exported_by(project: &Project, frames: usize) -> Vec<Vec<f32>> {
    let mut graph = built(project, Purpose::Export);
    whole(&mut graph, &mut feed(wavering), 0, frames)
}
