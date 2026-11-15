//! A clip with speed points (T10c): its sound goes along their path. Each segment that plays
//! sounds as a clip of its speed and direction placed there does, to the bit, resampled or with
//! its pitch kept, and a freeze is silent; a fade and the keys of the gain are those of the
//! whole clip, the keys read at the time of its source; its equalizer and its effects run
//! through a speed point as through any other sample; it plays from anywhere and backward as
//! other clips do; and no memory is taken on the thread of the mix.

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
use columbina_audio::graph::{Graph, Options, Purpose, Warp};
use columbina_audio::testing::{realtime, CountingAllocator};
use columbina_model::audio::fx::{EqKind, LimiterFx};
use columbina_model::retime::{Retime, SignedSpeed, SpeedPoint};
use columbina_model::{
    AudioKey, AudioParam, Clip, Effect, Insert, KeyInterp, Project, Speed, Time, TrackKind,
};

use common::{cents, peak_frequency, RATE};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// The samples of a frame of the sample timeline (25 frames a second at 48 kHz).
const FRAME: i64 = 1920;

/// The sample project with its track of sound only (A1, stereo), its media a minute long, and
/// `clips` on the track.
fn project(clips: Vec<Clip>) -> Project {
    let mut project = Project::sample();
    project.timelines[0]
        .tracks
        .retain(|track| track.kind == TrackKind::Audio);
    project.media[0].facts.duration = Time {
        ticks: 60 * 15_360,
        scale: 15_360,
    };
    project.timelines[0].tracks[0].clips = clips;
    project
}

/// A clip of m1 from `tenths` tenths of a second into it, on `length` frames from frame
/// `start` of the timeline.
fn clip(id: &str, tenths: i64, start: i64, length: i64) -> Clip {
    let begins = Time {
        ticks: tenths * 1536,
        scale: 15_360,
    };
    Clip::of_media(id, "m1", begins, start, length)
}

/// Speed points, each a frame and the `num` and `den` of its speed.
fn points(points: &[(i64, i64, u32)]) -> Retime {
    Retime {
        points: points
            .iter()
            .map(|&(frame, num, den)| SpeedPoint {
                frame,
                speed: SignedSpeed { num, den },
            })
            .collect(),
    }
}

/// c2 on frames 25 to 85, samples 48000 to 163200: from 2 s into m1 it plays 20 frames, holds
/// for 10, goes back for 20 at half the speed, and plays on for 10 at twice the speed. Its
/// speed points are at samples 86400, 105600 and 144000.
fn winding() -> Clip {
    let mut clip = clip("c2", 20, 25, 60);
    clip.set_mapping(Some(points(&[
        (0, 1, 1),
        (20, 0, 1),
        (30, -1, 2),
        (50, 2, 1),
    ])));
    clip
}

/// The clips of one speed that the segments of [`winding`] are, each where its segment is and
/// from the lowest place of its segment: the way back and the way on both lie from 2.4 s of
/// m1, 10 frames above where the clip begins.
fn straight() -> Vec<Clip> {
    let up = clip("s1", 20, 25, 20);
    let mut back = clip("s2", 24, 55, 20);
    back.speed = Speed { num: 1, den: 2 };
    back.reverse = true;
    let mut on = clip("s3", 24, 75, 10);
    on.speed = Speed { num: 2, den: 1 };
    vec![up, back, on]
}

fn built(project: &Project) -> Graph {
    Graph::build(project, 0, Options::new(4096, Purpose::Export)).unwrap()
}

/// Two tones in the media, at its sample `n`: 440 Hz, and a quieter one that is no multiple of
/// it; the right channel a radian on.
fn tones(n: i64, channel: usize) -> f32 {
    let t = std::f64::consts::TAU * n as f64 / f64::from(RATE);
    let shift = if channel == 1 { 1.0 } else { 0.0 };
    (0.3 * (440.0 * t + shift).sin() + 0.1 * (1234.5 * t).sin()) as f32
}

/// A level of a half all through the media.
fn level(_: i64, _: usize) -> f32 {
    0.5
}

/// A feed of `signal(sample of the media, channel)`, whichever clip reads it.
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

/// `frames` samples of the monitor of `graph` from sample `at`, played through `warp`, in
/// blocks of `block`: a channel after another.
fn play_through(
    graph: &mut Graph,
    warp: Warp,
    feed: &mut dyn ClipFeed,
    (at, frames): (i64, usize),
    block: usize,
) -> Vec<Vec<f32>> {
    graph.set_warp(warp);
    graph.start(at, feed).unwrap();
    let output = graph.monitor().unwrap();
    let channels = graph.output(output).channels();
    let mut played = vec![Vec::with_capacity(frames); channels];
    let mut done = 0;
    while done < frames {
        let n = block.min(frames - done).min(graph.max_block());
        graph.process(feed, n).unwrap();
        for (channel, played) in played.iter_mut().enumerate() {
            played.extend_from_slice(&graph.output(output).channel(channel)[..n]);
        }
        done += n;
    }
    played
}

/// [`play_through`] as the timeline is.
fn play(
    graph: &mut Graph,
    feed: &mut dyn ClipFeed,
    span: (i64, usize),
    block: usize,
) -> Vec<Vec<f32>> {
    play_through(graph, Warp::IDENTITY, feed, span, block)
}

/// Checks that `got` and `want` are the same samples, to the bit; `from` is the sample of the
/// timeline of the first.
fn same(got: &[Vec<f32>], want: &[Vec<f32>], from: i64, what: &str) {
    assert_eq!(got.len(), want.len(), "{what}");
    for (channel, (got, want)) in got.iter().zip(want).enumerate() {
        assert_eq!(got.len(), want.len(), "{what}");
        for (n, (got, want)) in got.iter().zip(want).enumerate() {
            assert_eq!(
                got.to_bits(),
                want.to_bits(),
                "{what}: sample {} of channel {channel}: {got} and {want}",
                from + n as i64
            );
        }
    }
}

/// The largest size of a sample of `signal`.
fn most(signal: &[f32]) -> f32 {
    signal.iter().fold(0.0, |most, s| most.max(s.abs()))
}

#[test]
fn each_segment_sounds_as_a_clip_of_its_speed_and_direction_placed_there() {
    for corrected in [false, true] {
        let what = format!("pitch kept: {corrected}");
        let mut retimed = winding();
        retimed.pitch_correction = corrected;
        let mut plain = straight();
        for clip in &mut plain {
            clip.pitch_correction = corrected;
        }
        // From before the clip to after it, the blocks cut across its speed points.
        let (from, frames) = (40_000, 130_000);
        let mut graph = built(&project(vec![retimed]));
        // A chain for each segment that plays: the freeze has none.
        assert_eq!(graph.clips().len(), 3, "{what}");
        assert_eq!(graph.unplayable(), [] as [String; 0]);
        let got = play(&mut graph, &mut feed(tones), (from, frames), 1_000);
        let want = play(
            &mut built(&project(plain)),
            &mut feed(tones),
            (from, frames),
            4_096,
        );
        same(&got, &want, from, &what);
        // It is heard on each segment that plays, and silent in the freeze and about the clip.
        let part =
            |range: Range<i64>| &got[0][(range.start - from) as usize..(range.end - from) as usize];
        assert!(most(part(48_000..86_400)) > 0.2, "{what}");
        assert!(most(part(105_600..144_000)) > 0.2, "{what}");
        assert!(most(part(144_000..163_200)) > 0.1, "{what}");
        assert_eq!(most(part(86_400..105_600)).to_bits(), 0, "{what}");
        assert_eq!(most(part(from..48_000)).to_bits(), 0, "{what}");
        assert_eq!(most(part(163_200..170_000)).to_bits(), 0, "{what}");
        // At half the speed the tone is an octave down, or where it was with its pitch kept.
        let slow = peak_frequency(part(107_000..143_000));
        let wanted = if corrected { 440.0 } else { 220.0 };
        println!("{what}: {slow:.2} Hz at half the speed");
        assert!(cents(slow, wanted).abs() < 20.0, "{what}: {slow} Hz");
        // Started in the middle of the way back, it sounds what it sounds there from the start.
        let middle = 120_000;
        let later = play(&mut graph, &mut feed(tones), (middle, 30_000), 480);
        let there = (middle - from) as usize;
        let earlier: Vec<Vec<f32>> = got
            .iter()
            .map(|channel| channel[there..there + 30_000].to_vec())
            .collect();
        same(
            &later,
            &earlier,
            middle,
            &format!("{what}, from the middle"),
        );
    }
}

#[test]
fn a_fade_of_the_clip_goes_on_through_a_speed_point() {
    // c2 slows to half its speed 10 frames in, in the middle of its fade in of 20 frames; it
    // fades out over its last 20.
    let mut retimed = clip("c2", 20, 25, 60);
    retimed.set_mapping(Some(points(&[(0, 1, 1), (10, 1, 2)])));
    (retimed.fade_in, retimed.fade_out) = (20, 20);
    let mut plain = clip("c2", 20, 25, 60);
    (plain.fade_in, plain.fade_out) = (20, 20);
    // A level sounds the same at any speed: what is left is the gain of the clip, which is
    // that of a clip of one speed with the same fades.
    let (from, frames) = (25 * FRAME, 60 * FRAME as usize);
    let got = play(
        &mut built(&project(vec![retimed])),
        &mut feed(level),
        (from, frames),
        1_000,
    );
    let want = play(
        &mut built(&project(vec![plain])),
        &mut feed(level),
        (from, frames),
        1_000,
    );
    for (n, (got, want)) in got[0].iter().zip(&want[0]).enumerate() {
        assert!((got - want).abs() < 1e-5, "sample {n}: {got} and {want}");
    }
    // At the speed point it has risen half the way, and it does not start again after it.
    let full = got[0][30 * FRAME as usize];
    assert!(full > 0.1, "{full}");
    let point = 10 * FRAME as usize;
    for n in [point - 1, point, point + 1, point + 100] {
        let share = got[0][n] / full;
        assert!((share - 0.5).abs() < 0.01, "sample {n}: {share}");
    }
    let rising = &got[0][..20 * FRAME as usize];
    assert!(rising.windows(2).all(|pair| pair[1] >= pair[0] - 1e-6));
}

#[test]
fn the_keys_of_the_gain_are_read_at_the_time_of_the_source_through_a_slow_segment() {
    // c2 plays 0.4 s of m1 from 2 s of it in 10 frames, and the next 0.4 s in 20 frames at
    // half the speed. Its gain falls from 0 dB at 2 s of m1 to -20 dB at 2.8 s.
    let mut fixed = clip("c2", 20, 25, 30);
    fixed.set_mapping(Some(points(&[(0, 1, 1), (10, 1, 2)])));
    let key = |tenths, value| AudioKey {
        at: Time {
            ticks: tenths,
            scale: 10,
        },
        value,
        interp: KeyInterp::Linear,
    };
    let mut keyed = fixed.clone();
    keyed.gain_db = AudioParam::Keys(vec![key(20, 0.0), key(28, -20.0)]);
    let (from, frames) = (25 * FRAME, 30 * FRAME as usize);
    let got = play(
        &mut built(&project(vec![keyed])),
        &mut feed(level),
        (from, frames),
        1_000,
    );
    let flat = play(
        &mut built(&project(vec![fixed])),
        &mut feed(level),
        (from, frames),
        1_000,
    );
    let point = 10 * FRAME as usize;
    for n in (0..frames).step_by(97) {
        // The seconds of the source that the sample reads, after where the clip begins.
        let seconds = if n < point {
            n as f64 / 48_000.0
        } else {
            0.4 + (n - point) as f64 / 96_000.0
        };
        let wanted = 10f64.powf(-20.0 * seconds / 0.8 / 20.0);
        let gain = f64::from(got[0][n]) / f64::from(flat[0][n]);
        assert!(
            (gain - wanted).abs() < 1e-3,
            "sample {n}: {gain}, not {wanted}"
        );
    }
}

#[test]
fn the_equalizer_and_the_effects_of_the_clip_run_through_a_speed_point() {
    // A speed point where the speed does not change: were a filter or an effect of the clip
    // started again there, or its sound a sample out, it would not be the clip without it.
    for limited in [false, true] {
        let mut plain = clip("c2", 20, 25, 50);
        plain.eq.on = true;
        for band in &mut plain.eq.bands {
            if matches!(band.kind, EqKind::Bell) {
                band.on = true;
                band.gain_db = 9.0;
            }
        }
        if limited {
            // A Limiter that works all the time: it looks ahead, and lets go slowly.
            plain.effects = vec![Insert {
                id: String::new(),
                on: true,
                effect: Effect::Limiter(LimiterFx {
                    input_db: 12.0,
                    ..LimiterFx::default()
                }),
            }];
        }
        let mut pointed = plain.clone();
        pointed.retime = Some(points(&[(0, 1, 1), (20, 1, 1)]));
        let (from, frames) = (40_000, 120_000);
        let mut graph = built(&project(vec![pointed]));
        assert_eq!(graph.clips().len(), 2);
        let got = play(&mut graph, &mut feed(tones), (from, frames), 1_000);
        let want = play(
            &mut built(&project(vec![plain])),
            &mut feed(tones),
            (from, frames),
            1_000,
        );
        assert!(most(&got[0]) > 0.2);
        same(&got, &want, from, &format!("a Limiter: {limited}"));
    }
}

#[test]
fn a_clip_with_speed_points_plays_backward_what_it_plays_forward() {
    let project = project(vec![winding()]);
    let frames = 130_000;
    // Sample `n` out at -1 is sample 169999 - n of the timeline.
    let last = 169_999;
    let back = play_through(
        &mut built(&project),
        Warp::new(-1, last, last),
        &mut feed(tones),
        (last, frames),
        1_000,
    );
    let forward = play(
        &mut built(&project),
        &mut feed(tones),
        (40_000, frames),
        1_000,
    );
    assert!(most(&back[0]) > 0.2);
    for (channel, (back, forward)) in back.iter().zip(&forward).enumerate() {
        for (n, sample) in back.iter().enumerate() {
            assert_eq!(
                sample.to_bits(),
                forward[frames - 1 - n].to_bits(),
                "sample {n} out of channel {channel}"
            );
        }
    }
}

#[test]
fn a_clip_with_speed_points_plays_with_no_memory_taken() {
    // Everything at once: its pitch kept, an equalizer, a Limiter, and keys of its gain.
    let mut clip = winding();
    clip.pitch_correction = true;
    clip.eq.on = true;
    clip.effects = vec![Insert {
        id: String::new(),
        on: true,
        effect: Effect::Limiter(LimiterFx::default()),
    }];
    let key = |tenths, value| AudioKey {
        at: Time {
            ticks: tenths,
            scale: 10,
        },
        value,
        interp: KeyInterp::Linear,
    };
    clip.gain_db = AudioParam::Keys(vec![key(20, 0.0), key(28, -6.0)]);
    let mut graph =
        Graph::build(&project(vec![clip]), 0, Options::new(480, Purpose::Preview)).unwrap();
    let mut read = feed(tones);
    graph.start(0, &mut read).unwrap();
    let mut device = vec![0.0_f32; 480 * 2];
    let mut heard = 0.0_f32;
    // Through the start of the clip, each of its speed points, and its end.
    let ((), counted) = realtime(|| {
        for _ in 0..400 {
            graph.process(&mut read, 480).unwrap();
            graph.write_device(graph.monitor().unwrap(), 480, &mut device);
            heard = device.iter().fold(heard, |most, s| most.max(s.abs()));
        }
    });
    assert!(counted.none(), "{counted:?}");
    assert!(heard > 0.1, "{heard}");
}
