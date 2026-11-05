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

// The review of T4c.

/// `frames` samples of the monitor from sample `at`, the run started with at most `most` samples
/// of preroll, as a graph swapped in or a grain of scrubbing is (T5, T18).
fn within(
    graph: &mut Graph,
    feed: &mut dyn ClipFeed,
    (at, most): (i64, u32),
    frames: usize,
) -> Vec<Vec<f32>> {
    graph.start_within(at, most, feed).unwrap();
    let output = graph.monitor().unwrap();
    let channels = graph.output(output).channels();
    let mut played = vec![Vec::with_capacity(frames); channels];
    while played[0].len() < frames {
        let n = (frames - played[0].len()).min(graph.max_block());
        graph.process(feed, n).unwrap();
        for (channel, played) in played.iter_mut().enumerate() {
            played.extend_from_slice(&graph.output(output).channel(channel)[..n]);
        }
    }
    played
}

/// The preroll of a grain of scrubbing, 100 ms (T18).
const GRAIN_PREROLL: u32 = 4_800;

#[test]
fn a_run_with_a_short_preroll_into_a_clip_with_a_pitch_is_heard_from_its_first_sample() {
    let pitch = Effect::Pitch(PitchFx {
        semitones: 7,
        ..PitchFx::default()
    });
    let project = project(250, vec![on(pitch)]);
    let at = 5 * SECOND as i64 + 77;
    let frames = 5 * SECOND;
    let export = exported(&project, 0.5, at as usize + frames);
    let mut graph = built(&project, Purpose::Export);
    let heard = within(
        &mut graph,
        &mut feed(tone(440.0, 0.5)),
        (at, GRAIN_PREROLL),
        frames,
    );
    // Not the silence of the latency of the Pitch (6304 samples, 1504 past a preroll of 4800).
    for window in heard[0][..1_504].chunks(150) {
        assert!(window.iter().any(|s| s.abs() > 0.05), "a silence in it");
    }
    // Once the Pitch has settled from where it started, a preroll before the start: the export.
    let settled =
        pitch::settle_for(&pitch::Settings::default(), RATE) as usize - GRAIN_PREROLL as usize;
    for channel in 0..2 {
        let most = most_apart(
            &heard[channel][settled..],
            &export[channel][at as usize + settled..],
        );
        assert!(most < 3.16e-5, "{most} apart");
    }
}

#[test]
fn a_run_with_a_short_preroll_in_the_middle_of_a_clip_with_effects_settles_to_the_export() {
    for stretched in [false, true] {
        let project = busy(stretched, false);
        let mut graph = built(&project, Purpose::Export);
        let frames = START + LONG as usize * FRAME;
        let from_0 = whole(&mut graph, &mut feed(wavering), 0, frames);
        let at = 5 * SECOND + 77;
        let late = within(
            &mut graph,
            &mut feed(wavering),
            (at as i64, GRAIN_PREROLL),
            frames - at,
        );
        // Settled once its effects have, from where they started, a preroll before the start.
        let settled = graph.settle() as usize - GRAIN_PREROLL as usize;
        assert!(late[0][..1_000].iter().any(|s| s.abs() > 0.05));
        for channel in 0..2 {
            let most = most_apart(&from_0[channel][at + settled..], &late[channel][settled..]);
            assert!(most < 3.16e-5, "stretched {stretched}: {most} apart");
        }
    }
}

#[test]
fn a_dry_pitch_and_an_untouching_limiter_leave_a_reversed_or_warped_clip_as_it_is_to_the_bit() {
    let effects = || {
        vec![
            on(Effect::Pitch(PitchFx {
                semitones: 12,
                dry_wet: 0.0,
                ..PitchFx::default()
            })),
            on(Effect::Limiter(LimiterFx::default())),
        ]
    };
    for reverse in [false, true] {
        let mut with = project(150, effects());
        let mut plain = project(150, vec![]);
        for project in [&mut with, &mut plain] {
            project.timelines[0].tracks[0].clips[0].reverse = reverse;
        }
        // As it is, and at twice its speed either way, from 4 s.
        for speed in [1, 2, -2] {
            let at = 4 * SECOND as i64;
            let sound = |project: &Project| {
                let mut graph = built(project, Purpose::Export);
                graph.set_warp(Warp::new(speed, at, at));
                whole(&mut graph, &mut feed(tone(440.0, 0.1)), at, 2 * SECOND)
            };
            let (with, plain) = (sound(&with), sound(&plain));
            assert!(plain[0].iter().any(|s| s.abs() > 0.05));
            for channel in 0..2 {
                assert_eq!(
                    differs(&with[channel], &plain[channel]),
                    None,
                    "reverse {reverse}, speed {speed}"
                );
            }
        }
    }
}

/// [`project`] with a second track of sound, A2, whose clip c3 sounds from 20 s to 30 s with a
/// Pitch; A1's clip c2 has no effects.
fn far_pitch() -> Project {
    let mut project = project(100, vec![]);
    let mut second = project.timelines[0].tracks[0].clone();
    second.id = "A2".into();
    second.clips[0].id = "c3".into();
    second.clips[0].start = 20 * 25;
    second.clips[0].length = 250;
    second.clips[0].effects = vec![on(Effect::Pitch(PitchFx {
        semitones: 3,
        ..PitchFx::default()
    }))];
    project.timelines[0].tracks.push(second);
    project
}

#[test]
fn the_preroll_of_a_start_is_what_the_clips_under_way_there_need() {
    let project = far_pitch();
    let mut graph = built(&project, Purpose::Export);
    let pitch_settle = pitch::settle_for(&pitch::Settings::default(), RATE);
    assert!(graph.preroll() >= pitch_settle);
    // Far from the Pitch, and before it starts its effects: none.
    for at in [0, 3 * SECOND as i64, 20 * SECOND as i64 - 7_000] {
        assert_eq!(graph.preroll_at(at), 0, "{at}");
    }
    // In it, its settling; just after it starts, back to where its effects start.
    assert!(graph.preroll_at(25 * SECOND as i64) >= pitch_settle);
    let just = 20 * SECOND as i64 + 1_000;
    assert_eq!(i64::from(graph.preroll_at(just)), 1_000 + 6_304);
    // What the strips need, they need anywhere: a Limiter on A1.
    let mut limited = project.clone();
    limited.timelines[0].tracks[0].audio.strip.inserts = vec![Insert {
        id: "fx1".into(),
        on: true,
        effect: Effect::Limiter(LimiterFx::default()),
    }];
    let strips = limiter::settle_for(&limiter::Settings::default(), RATE);
    assert_eq!(built(&limited, Purpose::Export).preroll_at(0), strips);
    // A clip's track that drives a Ducker on A2: its effects settle, then the Ducker.
    let mut ducking = far_pitch();
    let timeline = &mut ducking.timelines[0];
    timeline.tracks[1].clips[0].effects.clear();
    timeline.tracks[0].clips[0].effects = vec![on(Effect::Gain(GainFx { gain_db: -3.0 }))];
    timeline.tracks[1].audio.strip.track_fx = Some(columbina_model::audio::fx::Ducker {
        on: true,
        sources: vec![columbina_model::SourceRef::Track("A1".into())],
        ..columbina_model::audio::fx::Ducker::default()
    });
    let ducked = built(&ducking, Purpose::Export);
    let strips = ducked.preroll_at(0);
    assert!(strips > 0);
    assert_eq!(ducked.preroll_at(3 * SECOND as i64), strips + 480);
    // Wherever a run starts, it sounds as the export once settled; before the Pitch, to the bit.
    let frames = 32 * SECOND;
    let export = whole(&mut graph, &mut feed(tone(440.0, 0.5)), 0, frames);
    for at in [3 * SECOND, 20 * SECOND - 7_000, just as usize, 25 * SECOND] {
        let late = whole(
            &mut graph,
            &mut feed(tone(440.0, 0.5)),
            at as i64,
            frames - at,
        );
        for channel in 0..2 {
            if at <= just as usize {
                assert_eq!(
                    differs(&export[channel][at..], &late[channel]),
                    None,
                    "{at}"
                );
            } else {
                let most = most_apart(&export[channel][at..], &late[channel]);
                assert!(most < 3.16e-5, "{at}: {most}");
            }
        }
    }
}

#[test]
fn a_graph_built_within_a_range_has_only_the_clips_heard_there_and_sounds_as_one_with_all() {
    let mut project = far_pitch();
    // A third track, muted, with a Pitch too, under the range.
    let mut muted = project.timelines[0].tracks[1].clone();
    muted.id = "A3".into();
    muted.clips[0].id = "c4".into();
    muted.clips[0].start = 0;
    muted.audio.mute = true;
    project.timelines[0].tracks.push(muted);
    let range = 2 * SECOND as i64..3 * SECOND as i64;
    let mut all = built(&project, Purpose::Preview);
    let mut few = Graph::build(
        &project,
        0,
        Options::new(4096, Purpose::Preview).within(range.clone()),
    )
    .unwrap();
    assert_eq!(all.clips().len(), 3);
    let kept: Vec<&str> = few.clips().iter().map(|c| c.clip.as_str()).collect();
    assert_eq!(kept, ["c2"]);
    let at = range.start + 77;
    let (all, few) = (
        whole(&mut all, &mut feed(tone(440.0, 0.5)), at, SECOND / 2),
        whole(&mut few, &mut feed(tone(440.0, 0.5)), at, SECOND / 2),
    );
    assert_eq!(differs(&all[0], &few[0]), None);
}

#[test]
#[ignore = "a measure: run with --ignored --nocapture"]
fn cost_of_building_a_graph_of_a_hundred_clips_with_a_pitch() {
    let mut project = project(10, vec![]);
    let track = &mut project.timelines[0].tracks[0];
    let first = track.clips[0].clone();
    track.clips = (0..100)
        .map(|k| {
            let mut clip = first.clone();
            clip.id = format!("c{}", 100 + k);
            clip.start = 25 + 10 * k;
            clip.effects = vec![on(Effect::Pitch(PitchFx {
                semitones: 2,
                ..PitchFx::default()
            }))];
            clip
        })
        .collect();
    let _ = built(&project, Purpose::Preview);
    let (graph, whole) = measured(|| built(&project, Purpose::Preview));
    let range = 2 * SECOND as i64..3 * SECOND as i64;
    let (few, within) = measured(|| {
        Graph::build(
            &project,
            0,
            Options::new(4096, Purpose::Preview).within(range),
        )
        .unwrap()
    });
    println!(
        "100 clips with a Pitch: {} clips built in {:?}, {} MB more held; within a \
         second, {} clips in {:?}, {} MB",
        graph.clips().len(),
        whole.0,
        whole.1 / (1 << 20),
        few.clips().len(),
        within.0,
        within.1 / (1 << 20),
    );
}

/// What `make` gives, how long it took, and how much more private memory the process holds
/// after it than before (Rust and C++ alike, read from the system).
fn measured<T>(make: impl FnOnce() -> T) -> (T, (std::time::Duration, usize)) {
    let before = private_bytes();
    let started = std::time::Instant::now();
    let thing = make();
    let took = started.elapsed();
    (thing, (took, private_bytes().saturating_sub(before)))
}

/// The private memory of this process, in bytes, as PowerShell reads it.
fn private_bytes() -> usize {
    let script = format!(
        "(Get-Process -Id {}).PrivateMemorySize64",
        std::process::id()
    );
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0)
}
