//! A clip's change of speed that keeps its pitch, and its pitch moved (p. 3826–3827, T13): the
//! pitch within 5 cents and the length to the sample, the speed without it as before, the same
//! bits from the same project, wherever the run starts and however it is cut into blocks, the
//! sound where its mapping puts it, and no memory taken on the thread of the mix; and a track's
//! Pitch insert in the graph.

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
use columbina_audio::fx::pitch;
use columbina_audio::graph::{Graph, Options, Owner, Purpose};
use columbina_audio::testing::{realtime, CountingAllocator, Rng};
use columbina_model::audio::fx::PitchFx;
use columbina_model::{ClipPitch, Effect, Insert, Project, Speed, Time, TrackKind};

use common::{cents, level_holds_across, peak_frequency, RATE};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

const SECOND: usize = RATE as usize;
/// The samples of a frame of the sample timeline (25 frames a second).
const FRAME: usize = SECOND / 25;

/// The sample project with its track of sound only (A1, stereo), its clip c2 lasting `frames`
/// frames from the start at `speed`, Pitch Correction as `corrected`, its pitch moved by
/// `pitch`; its media a minute long.
fn project(frames: i64, speed: (u32, u32), corrected: bool, pitch: (i32, f64)) -> Project {
    let mut project = Project::sample();
    project.timelines[0]
        .tracks
        .retain(|track| track.kind == TrackKind::Audio);
    project.media[0].facts.duration = Time {
        ticks: 60 * 15_360,
        scale: 15_360,
    };
    let clip = &mut project.timelines[0].tracks[0].clips[0];
    clip.length = frames;
    clip.speed = Speed {
        num: speed.0,
        den: speed.1,
    };
    clip.pitch_correction = corrected;
    clip.pitch = ClipPitch {
        semitones: pitch.0,
        cents: pitch.1,
    };
    project
}

fn built(project: &Project) -> Graph {
    Graph::build(project, 0, Options::new(4096, Purpose::Export)).unwrap()
}

/// A tone of `hertz` and amplitude 0.5 in the media, at its sample `n` (the right channel at
/// a quarter of a turn).
fn tone(hertz: f64) -> impl Fn(i64, usize) -> f32 {
    move |n, channel| {
        let phase = std::f64::consts::TAU * hertz * n as f64 / f64::from(RATE)
            + if channel == 1 { 1.0 } else { 0.0 };
        (0.5 * phase.sin()) as f32
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

/// The first sample at which `a` and `b` differ in their bits, if any.
fn differs(a: &[f32], b: &[f32]) -> Option<usize> {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .position(|(a, b)| a.to_bits() != b.to_bits())
}

#[test]
fn played_at_0_8_with_pitch_correction_440_hz_stays_440_hz_and_lasts_the_clip_to_the_sample() {
    let project = project(300, (4, 5), true, (0, 0.0));
    let mut graph = built(&project);
    let end = 300 * FRAME;
    let out = whole(&mut graph, &mut feed(tone(440.0)), 0, end + SECOND);
    for channel in &out {
        // Over the whole clip, and over each stretch of 4 s of its source (5 s here) alone.
        for range in [
            0..end,
            0..5 * SECOND,
            5 * SECOND..10 * SECOND,
            10 * SECOND..end,
        ] {
            let measured = peak_frequency(&channel[range.clone()]);
            assert!(
                cents(measured, 440.0).abs() < 5.0,
                "{measured} Hz over {range:?}"
            );
        }
        // It sounds from its first sample to its last, and not a sample after.
        assert!(
            channel[..64].iter().any(|s| s.abs() > 0.1),
            "{:?}",
            &channel[..8]
        );
        assert!(channel[end - 64..end].iter().any(|s| s.abs() > 0.1));
        assert!(channel[end..].iter().all(|&s| s == 0.0));
    }
}

#[test]
fn played_at_1_25_without_pitch_correction_its_pitch_goes_with_its_speed_as_before() {
    let mut graph = built(&project(100, (5, 4), false, (0, 0.0)));
    let out = whole(&mut graph, &mut feed(tone(440.0)), 0, 100 * FRAME);
    let measured = peak_frequency(&out[0]);
    assert!(cents(measured, 550.0).abs() < 5.0, "{measured} Hz");
    // Pitch Correction at a speed of 1, with no pitch, is the clip as it was, to the bit.
    let mut plain = built(&project(100, (1, 1), false, (0, 0.0)));
    let mut corrected = built(&project(100, (1, 1), true, (0, 0.0)));
    let a = whole(&mut plain, &mut feed(tone(440.0)), 0, 100 * FRAME);
    let b = whole(&mut corrected, &mut feed(tone(440.0)), 0, 100 * FRAME);
    assert_eq!(differs(&a[0], &b[0]), None);
}

#[test]
fn its_pitch_moves_an_octave_and_adds_to_a_speed_that_moves_it() {
    let mut graph = built(&project(150, (1, 1), true, (12, 0.0)));
    let out = whole(&mut graph, &mut feed(tone(440.0)), 0, 150 * FRAME);
    let measured = peak_frequency(&out[0][SECOND / 2..]);
    assert!(cents(measured, 880.0).abs() < 5.0, "{measured} Hz");
    // Without Pitch Correction, 2 semitones on top of the 1.25 of its speed.
    let mut graph = built(&project(150, (5, 4), false, (2, 0.0)));
    let out = whole(&mut graph, &mut feed(tone(440.0)), 0, 150 * FRAME);
    let expected = 550.0 * 2.0_f64.powf(2.0 / 12.0);
    let measured = peak_frequency(&out[0][SECOND / 2..]);
    assert!(
        cents(measured, expected).abs() < 5.0,
        "{measured} Hz for {expected}"
    );
    // With it, 2 semitones and 50 cents down at 0.8.
    let mut graph = built(&project(150, (4, 5), true, (-2, -50.0)));
    let out = whole(&mut graph, &mut feed(tone(440.0)), 0, 150 * FRAME);
    let expected = 440.0 * 2.0_f64.powf(-2.5 / 12.0);
    let measured = peak_frequency(&out[0][SECOND / 2..]);
    assert!(
        cents(measured, expected).abs() < 5.0,
        "{measured} Hz for {expected}"
    );
}

#[test]
fn played_backward_with_pitch_correction_it_keeps_its_pitch() {
    let mut project = project(150, (4, 5), true, (0, 0.0));
    project.timelines[0].tracks[0].clips[0].reverse = true;
    let mut graph = built(&project);
    let out = whole(&mut graph, &mut feed(tone(440.0)), 0, 150 * FRAME);
    let measured = peak_frequency(&out[0]);
    assert!(cents(measured, 440.0).abs() < 5.0, "{measured} Hz");
}

#[test]
fn the_same_project_twice_stretches_to_the_same_bits() {
    let project = project(300, (3, 5), true, (1, 30.0));
    let signal =
        |n: i64, channel: usize| tone(311.0)(n, channel) + 0.3 * tone(1_250.0)(n / 3, channel);
    let mut first = built(&project);
    let mut second = built(&project);
    let a = whole(&mut first, &mut feed(signal), 0, 300 * FRAME);
    let b = whole(&mut second, &mut feed(signal), 0, 300 * FRAME);
    let again = whole(&mut first, &mut feed(signal), 0, 300 * FRAME);
    for channel in 0..2 {
        assert!(a[channel].iter().any(|s| s.abs() > 0.1));
        assert_eq!(differs(&a[channel], &b[channel]), None, "another graph");
        assert_eq!(
            differs(&a[channel], &again[channel]),
            None,
            "the same graph again"
        );
    }
}

#[test]
fn started_at_6_s_past_a_point_where_the_stretch_starts_again_it_sounds_as_one_started_at_0() {
    // At a speed of 1 its pitch moved alone: the stretch starts again at 4 s and 8 s.
    // With Pitch Correction at 0.8: at 5 s and 10 s of the timeline.
    for (speed, corrected, pitch) in [((1, 1), false, (3, 0.0)), ((4, 5), true, (0, 0.0))] {
        let project = project(300, speed, corrected, pitch);
        let mut graph = built(&project);
        let from_0 = whole(&mut graph, &mut feed(tone(523.0)), 0, 300 * FRAME);
        let from_6 = whole(
            &mut graph,
            &mut feed(tone(523.0)),
            6 * SECOND as i64,
            300 * FRAME - 6 * SECOND,
        );
        for channel in 0..2 {
            assert_eq!(
                differs(&from_0[channel][6 * SECOND..], &from_6[channel]),
                None,
                "{speed:?}, channel {channel}"
            );
        }
        // And just before a point, in its crossfade.
        let at = 8 * SECOND - 100;
        let late = whole(&mut graph, &mut feed(tone(523.0)), at as i64, 2 * SECOND);
        assert_eq!(
            differs(&from_0[0][at..at + 2 * SECOND], &late[0]),
            None,
            "{speed:?} from {at}"
        );
    }
}

#[test]
fn its_sound_does_not_depend_on_how_the_run_is_cut_into_blocks() {
    for (speed, reverse) in [((4, 5), false), ((3, 2), true), ((7, 3), false)] {
        let mut project = project(150, speed, true, (1, 0.0));
        project.timelines[0].tracks[0].clips[0].reverse = reverse;
        let mut graph = built(&project);
        let signal = |n: i64, channel: usize| {
            tone(377.0)(n, channel) * if n % 5_000 < 2_500 { 1.0 } else { 0.3 }
        };
        let reference = whole(&mut graph, &mut feed(signal), 1_000, 150 * FRAME);
        for seed in [1, 2, 3] {
            let mut rng = Rng::new(seed);
            let cut = play(
                &mut graph,
                &mut feed(signal),
                (1_000, 150 * FRAME),
                &mut || {
                    if rng.next_u64().is_multiple_of(4) {
                        rng.range(1, 4)
                    } else {
                        rng.log_range(4096)
                    }
                },
            );
            for channel in 0..2 {
                assert_eq!(
                    differs(&reference[channel], &cut[channel]),
                    None,
                    "{speed:?} backward {reverse}, seed {seed}"
                );
            }
        }
    }
}

#[test]
fn a_stretched_sound_is_where_its_mapping_puts_it() {
    // A tone that starts at 2 s of the media: at 0.8 it is heard from 2.5 s of the timeline.
    let signal = |n: i64, channel: usize| {
        if n >= 2 * SECOND as i64 {
            tone(440.0)(n, channel)
        } else {
            0.0
        }
    };
    for (speed, onset) in [((4, 5), 5 * SECOND / 2), ((5, 4), 8 * SECOND / 5)] {
        let mut graph = built(&project(150, speed, true, (0, 0.0)));
        let out = whole(&mut graph, &mut feed(signal), 0, 150 * FRAME);
        // Where its level, over windows of 2.5 ms, first reaches half its steady level.
        let window = SECOND / 400;
        let level = |at: usize| {
            (out[0][at..at + window].iter().map(|s| s * s).sum::<f32>() / window as f32).sqrt()
        };
        let steady = level(onset + SECOND / 2);
        let crossed = (0..out[0].len() - window)
            .find(|&at| level(at) > steady / 2.0)
            .unwrap();
        let error =
            (crossed as f64 + window as f64 / 2.0 - onset as f64) / f64::from(RATE) * 1_000.0;
        assert!(
            error.abs() < 10.0,
            "{speed:?}: half way at {crossed}, {error:.2} ms from {onset}"
        );
    }
}

#[test]
fn two_clips_stretched_at_once_each_take_a_stretcher_and_a_third_takes_one_given_back() {
    let mut project = project(100, (4, 5), true, (0, 0.0));
    let mut second = project.timelines[0].tracks[0].clone();
    second.id = "A2".into();
    second.clips[0].id = "c3".into();
    second.clips[0].start = 50;
    let mut third = second.clips[0].clone();
    third.id = "c4".into();
    third.start = 200;
    second.clips.push(third);
    project.timelines[0].tracks.push(second);
    let mut graph = built(&project);
    // Each clip a tone of its own: c2 440 Hz, c3 660 Hz, c4 880 Hz.
    let mut read = |clip: usize, range: Range<i64>, out: &mut [&mut [f32]]| {
        let hertz = [440.0, 660.0, 880.0][clip];
        feed(tone(hertz))(clip, range, out)
    };
    let out = whole(&mut graph, &mut read, 0, 300 * FRAME);
    let heard = |range: Range<usize>, hertz: f64| common::amplitude_at(&out[0][range], hertz);
    // A1 alone, then A1 and A2 at once, then A2 alone, nothing, then A2's second clip.
    for (range, hertz, what) in [
        (SECOND / 2..SECOND, 440.0, "c2"),
        (3 * SECOND..4 * SECOND - SECOND / 10, 440.0, "c2 beside c3"),
        (3 * SECOND..4 * SECOND - SECOND / 10, 660.0, "c3 beside c2"),
        (5 * SECOND..6 * SECOND - SECOND / 10, 660.0, "c3"),
        (9 * SECOND..12 * SECOND - SECOND / 10, 880.0, "c4"),
    ] {
        let level = heard(range.clone(), hertz);
        assert!(level > 0.3, "{what} over {range:?}: {level}");
    }
    assert!(out[0][6 * SECOND + SECOND / 10..8 * SECOND - SECOND / 2]
        .iter()
        .all(|&s| s == 0.0));
}

#[test]
fn a_graph_of_stretched_clips_plays_ten_seconds_with_no_memory() {
    let mut project = project(250, (4, 5), true, (2, 0.0));
    let mut second = project.timelines[0].tracks[0].clone();
    second.id = "A2".into();
    second.clips[0].id = "c3".into();
    second.clips[0].start = 20;
    second.clips[0].speed = Speed { num: 1, den: 1 };
    second.clips[0].pitch_correction = false;
    second.clips[0].reverse = true;
    project.timelines[0].tracks.push(second);
    let mut graph = Graph::build(&project, 0, Options::new(480, Purpose::Preview)).unwrap();
    let mut read = feed(tone(440.0));
    graph.start(3 * SECOND as i64 / 2, &mut read).unwrap();
    let mut device = vec![0.0_f32; 480 * 2];
    let ((), counted) = realtime(|| {
        for _ in 0..1_000 {
            graph.process(&mut read, 480).unwrap();
            graph.write_device(graph.monitor().unwrap(), 480, &mut device);
        }
    });
    assert!(counted.none(), "{counted:?}");
    assert_eq!(
        graph.latency(),
        0,
        "a clip's stretch reads ahead and delays nothing"
    );
}

#[test]
fn a_tone_keeps_its_level_where_the_stretch_starts_again() {
    for (speed, corrected, pitch) in [
        ((4, 5), true, (0, 0.0)),
        ((9, 10), true, (0, 0.0)),
        ((5, 4), true, (0, 0.0)),
        ((1, 1), false, (3, 0.0)),
        ((1, 1), false, (0, 30.0)),
    ] {
        let mut graph = built(&project(300, speed, corrected, pitch));
        let out = whole(&mut graph, &mut feed(tone(440.0)), 0, 300 * FRAME);
        // Every 4 s of the source: at 4 s / speed of the timeline.
        let period = 4 * SECOND * speed.1 as usize / speed.0 as usize;
        for point in [period, 2 * period] {
            level_holds_across(&out[0], point, &format!("{speed:?} {pitch:?} at {point}"));
        }
    }
}

#[test]
fn a_pitch_insert_moves_its_track_an_octave_and_the_graph_makes_up_its_delay() {
    let mut project = project(250, (1, 1), false, (0, 0.0));
    project.timelines[0].tracks[0]
        .audio
        .strip
        .inserts
        .push(Insert {
            id: "fx1".into(),
            on: true,
            effect: Effect::Pitch(PitchFx {
                semitones: 12,
                ..PitchFx::default()
            }),
        });
    let mut graph = built(&project);
    let mut node = pitch::Pitch::default();
    columbina_audio::Node::prepare(
        &mut node,
        &columbina_audio::Spec::new(RATE, 4096, columbina_audio::Layout::Stereo),
    )
    .unwrap();
    assert_eq!(graph.latency(), columbina_audio::Node::latency(&node));
    assert!(graph.settle() >= pitch::settle_for(&pitch::Settings::default(), RATE));
    let owner = Owner::Track("A1".into());
    for path in [
        "insert.fx1.semitones",
        "insert.fx1.cents",
        "insert.fx1.dry_wet",
        "insert.fx1.on",
    ] {
        assert!(graph.control(&owner, path).is_some(), "{path}");
    }
    let out = whole(&mut graph, &mut feed(tone(440.0)), 0, 250 * FRAME);
    let measured = peak_frequency(&out[0][SECOND / 2..]);
    assert!(cents(measured, 880.0).abs() < 5.0, "{measured} Hz");
    // Its delay is made up: the sound starts where the clip does.
    assert!(
        out[0][..64].iter().any(|s| s.abs() > 0.05),
        "{:?}",
        &out[0][..8]
    );
}
