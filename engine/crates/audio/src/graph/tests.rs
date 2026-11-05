//! Tests of what is private to the graph: the resampler of a clip, moved with its test from the
//! mixer of phase 2 (`server/src/play/mix.rs`).

use super::clip::{radius, resampled};

#[test]
#[allow(
    clippy::cast_precision_loss,
    clippy::float_cmp,
    reason = "taps and samples far within an f64; the case that is given back as it is"
)]
fn a_sample_between_samples_is_a_windowed_sinc_of_those_about_it() {
    use std::f64::consts::PI;
    let left: Vec<f32> = (0..2048_i32)
        .map(|n| {
            #[allow(clippy::cast_possible_truncation)]
            let value = (0.4 * (f64::from(n) * 0.37).sin()) as f32;
            value
        })
        .collect();
    let right: Vec<f32> = (0..2048_i32)
        .map(|n| {
            #[allow(clippy::cast_possible_truncation)]
            let value = (0.3 * (f64::from(n) * 1.91).cos()) as f32;
            value
        })
        .collect();
    let pcm = [left.as_slice(), right.as_slice()];
    for cutoff in [1.0, 0.91875, 0.5, 1.0 / 16.0] {
        for phase in [0.0, 0.25, 0.5, 0.875] {
            if phase == 0.0 && cutoff == 1.0 {
                continue;
            }
            let index = 1000;
            let reach = radius(cutoff);
            let (mut sum, mut weights) = ([0.0; 2], 0.0);
            for tap in 1 - reach..=reach {
                let x = tap as f64 - phase;
                let angle = PI * cutoff * x;
                let sinc = if angle.abs() < 1e-12 {
                    1.0
                } else {
                    angle.sin() / angle
                };
                let weight = sinc * (0.5 + 0.5 * (PI * x / reach as f64).cos());
                let at = usize::try_from(index + tap).unwrap();
                sum[0] += f64::from(pcm[0][at]) * weight;
                sum[1] += f64::from(pcm[1][at]) * weight;
                weights += weight;
            }
            let mut got = [0.0; 2];
            resampled(&pcm, 1000, phase, cutoff, &mut got);
            for side in 0..2 {
                let want = sum[side] / weights;
                assert!(
                    (f64::from(got[side]) - want).abs() < 1e-5,
                    "{cutoff} at {phase}: {} != {want}",
                    got[side]
                );
            }
        }
    }
    // Unity speed at a whole sample gives the sample itself.
    let mut got = [0.0; 2];
    resampled(&pcm, 77, 0.0, 1.0, &mut got);
    assert_eq!(got, [pcm[0][77], pcm[1][77]]);
}

// The cursor of a lane (T14).

mod cursor {
    use std::time::Instant;

    use columbina_model::{Lane, LaneKind, LanePoint, LaneScale, Time};
    use proptest::prelude::*;

    use super::super::automation::Cursor;

    fn lane_of(points: &[(i64, f64)], scale: u32, switch: bool) -> Lane {
        Lane {
            points: points
                .iter()
                .map(|&(ticks, value)| LanePoint {
                    at: Time { ticks, scale },
                    value,
                })
                .collect(),
            kind: if switch {
                LaneKind::Switch
            } else {
                LaneKind::Continuous
            },
            scale: None,
        }
    }

    #[allow(clippy::cast_possible_truncation, reason = "as the strip reads it")]
    fn expected(lane: &Lane, n: i64, rate: u32) -> f32 {
        lane.value_at(Time {
            ticks: n,
            scale: rate,
        }) as f32
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1024))]

        /// Block after block, from wherever a run starts, the cursor gives what `value_at` gives
        /// at each sample, to the bit: of lanes of any scale (their points among the samples
        /// read, of a scale of 7 and of 1000 too), with points at one time, of switches, and of
        /// frequencies on a scale of logarithms, read at any rate, before their first point and
        /// after their last.
        #[test]
        fn a_cursor_reads_a_lane_as_value_at_does_at_every_sample(
            mut points in prop::collection::vec((-1000i64..400_000, 0.0f64..1.0), 1..14),
            jump in any::<bool>(),
            shape in 0usize..3,
            scale in prop::sample::select(vec![1000u32, 44_100, 48_000, 96_000, 15_360, 7]),
            rate in prop::sample::select(vec![44_100u32, 48_000, 96_000]),
            runs in prop::collection::vec((-50_000i64..420_000, prop::collection::vec(1usize..3000, 1..6)), 1..4),
        ) {
            // The points at the samples drawn, as ticks of their scale, so that they fall among
            // the samples that are read whatever the scale.
            points.retain(|p| p.0 >= 0);
            prop_assume!(!points.is_empty());
            let ticks = |sample: i64| sample * i64::from(scale) / i64::from(rate);
            let mut points: Vec<(i64, f64)> = points
                .iter()
                .map(|&(sample, share)| {
                    let value = match shape {
                        // Of a frequency, from 20 Hz to 20 kHz.
                        2 => 20.0 * 1000f64.powf(share),
                        _ => -100.0 + 110.0 * share,
                    };
                    (ticks(sample), value)
                })
                .collect();
            points.sort_by_key(|p| p.0);
            if jump && points.len() > 2 {
                points[2].0 = points[1].0;
            }
            let mut lane = lane_of(&points, scale, shape == 1);
            if shape == 2 {
                lane.scale = Some(LaneScale::Log);
            }
            let mut cursor = Cursor::default();
            let mut out = [0f32; 3000];
            for (start, blocks) in &runs {
                let mut at = *start;
                for &frames in blocks {
                    cursor.fill(&lane, at, rate, &mut out[..frames]);
                    for (k, got) in out[..frames].iter().enumerate() {
                        let n = at + i64::try_from(k).unwrap();
                        let want = expected(&lane, n, rate);
                        prop_assert_eq!(got.to_bits(), want.to_bits(), "sample {}", n);
                    }
                    at += i64::try_from(frames).unwrap();
                }
            }
        }
    }

    /// Points a whole lane apart from a run of samples, and the times of points far out, read in
    /// 128 bits.
    #[test]
    fn a_cursor_reads_points_far_out_on_the_timeline() {
        // 2^53 - 1 ticks of 4e9 a second, 25 days on: past what 64 bits count of ticks times
        // the rate.
        let far = (1i64 << 53) - 1;
        let lane = lane_of(&[(0, -10.0), (far, 0.0)], 4_000_000_000, false);
        let end = far / 4_000_000_000 * 48_000;
        let mut cursor = Cursor::default();
        let mut out = [0f32; 64];
        for start in [0, 1 << 30, end - 32, end + 1000] {
            cursor.fill(&lane, start, 48_000, &mut out);
            for (k, got) in out.iter().enumerate() {
                let n = start + i64::try_from(k).unwrap();
                assert_eq!(got.to_bits(), expected(&lane, n, 48_000).to_bits(), "{n}");
            }
        }
    }

    /// What a lane costs at each sample, read by `value_at` (T4b) and by the cursor (T14): a
    /// lane of 200 points over 10 s, read for 60 s in blocks of 480. Run with
    /// `cargo test --release -p columbina-audio --lib cost_of_a_lane -- --ignored --nocapture`.
    #[test]
    #[ignore = "a measurement, not a check"]
    #[allow(clippy::cast_precision_loss, reason = "a measurement")]
    fn cost_of_a_lane_per_sample() {
        let points: Vec<(i64, f64)> = (0..200)
            .map(|k| (k * 2400, if k % 2 == 0 { -20.0 } else { 0.0 }))
            .collect();
        for switch in [false, true] {
            let lane = lane_of(&points, 48_000, switch);
            let samples = 48_000 * 60;
            let mut out = [0f32; 480];
            let mut sum = 0.0f64;
            let begun = Instant::now();
            for block in 0..samples / 480 {
                let start = i64::from(block) * 480;
                for (n, value) in (start..).zip(out.iter_mut()) {
                    *value = expected(&lane, n, 48_000);
                }
                sum += f64::from(out[17]);
            }
            let old = begun.elapsed().as_nanos() as f64 / f64::from(samples);
            let mut cursor = Cursor::default();
            let begun = Instant::now();
            for block in 0..samples / 480 {
                cursor.fill(&lane, i64::from(block) * 480, 48_000, &mut out);
                sum += f64::from(out[17]);
            }
            let new = begun.elapsed().as_nanos() as f64 / f64::from(samples);
            println!(
                "{}: value_at {old:.2} ns a sample, the cursor {new:.2} ns ({:.1} times as fast) [{sum}]",
                if switch { "switch" } else { "continuous" },
                old / new
            );
        }
    }
}

#[test]
fn the_channels_of_sources_of_different_formats_go_by_their_roles() {
    use super::build::routes;
    use crate::Layout::{Adaptive, Lcrs, Mono, Stereo, Surround51, Surround71};
    // Mono into stereo: both sides; into 5.1: the centre (the LFE is no channel of a sidechain:
    // L R C Ls Rs are 0 to 4).
    assert_eq!(routes(Mono, Stereo), [(0, 0), (0, 1)]);
    assert_eq!(routes(Mono, Surround51), [(0, 2)]);
    // Stereo into stereo, one to one; into mono, both to it.
    assert_eq!(routes(Stereo, Stereo), [(0, 0), (1, 1)]);
    assert_eq!(routes(Stereo, Mono), [(0, 0), (1, 0)]);
    // 5.1 into stereo: the centre to both sides, the surrounds to their sides.
    assert_eq!(
        routes(Surround51, Stereo),
        [(0, 0), (1, 1), (2, 0), (2, 1), (3, 0), (4, 1)]
    );
    // 7.1 into 5.1: the rears to the surrounds of their sides.
    assert_eq!(
        routes(Surround71, Surround51),
        [(0, 0), (1, 1), (2, 2), (3, 3), (4, 4), (5, 3), (6, 4)]
    );
    // The surround of LCRS into 5.1: both surrounds.
    assert_eq!(
        routes(Lcrs, Surround51),
        [(0, 0), (1, 1), (2, 2), (3, 3), (3, 4)]
    );
    // An Adaptive track's channels by their numbers, round the channels of the other.
    assert_eq!(routes(Adaptive(3), Stereo), [(0, 0), (1, 1), (2, 0)]);
    assert_eq!(routes(Stereo, Adaptive(3)), [(0, 0), (1, 1)]);
}

/// A stretched clip (T13) reads its media only within its reach of where its mapping is, from
/// where it starts warming: what a reader ahead of the graph is told to have (T5). A run started
/// in the middle does the work of bringing its stretchers there in `Graph::start`, so that the
/// blocks after read only near where they play.
#[test]
fn a_stretched_clip_reads_within_its_reach_once_its_run_has_started() {
    use std::cell::RefCell;
    use std::ops::Range;
    use std::rc::Rc;

    use columbina_model::{ClipPitch, Project, Speed, Time, TrackKind};

    use super::{Graph, Options, Purpose};
    use crate::feed::FeedError;

    // Both ways in: a run that starts, and a graph swapped in with a short preroll (T5).
    for (within, (speed, reverse, corrected, pitch, at)) in
        [false, true].into_iter().flat_map(|within| {
            [
                ((4, 5), false, true, 0, 6 * 48_000),
                ((3, 2), true, true, 2, 3 * 48_000 + 17),
                ((1, 1), false, false, 5, 5 * 48_000 + 999),
                ((7, 3), false, true, 0, 1),
            ]
            .map(|case| (within, case))
        })
    {
        let mut project = Project::sample();
        project.timelines[0]
            .tracks
            .retain(|track| track.kind == TrackKind::Audio);
        project.media[0].facts.duration = Time {
            ticks: 60 * 15_360,
            scale: 15_360,
        };
        let clip = &mut project.timelines[0].tracks[0].clips[0];
        clip.length = 250;
        clip.speed = Speed {
            num: speed.0,
            den: speed.1,
        };
        clip.reverse = reverse;
        clip.pitch_correction = corrected;
        clip.pitch = ClipPitch {
            semitones: pitch,
            cents: 0.0,
        };
        let mut graph = Graph::build(&project, 0, Options::new(480, Purpose::Export)).unwrap();
        let reads: Rc<RefCell<Vec<Range<i64>>>> = Rc::default();
        let seen = Rc::clone(&reads);
        let mut feed = move |_: usize, range: Range<i64>, out: &mut [&mut [f32]]| {
            for (channel, out) in out.iter_mut().enumerate() {
                for (n, sample) in out.iter_mut().enumerate() {
                    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
                    let x = (range.start + i64::try_from(n).unwrap()) as f64;
                    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
                    let value = (0.5 * (x * 0.05 + channel as f64).sin()) as f32;
                    *sample = value;
                }
            }
            seen.borrow_mut().push(range);
            Ok::<(), FeedError>(())
        };
        if within {
            graph.start_within(at, 480, &mut feed).unwrap();
        } else {
            graph.start(at, &mut feed).unwrap();
        }
        reads.borrow_mut().clear();
        let chain = &graph.clips[0];
        assert!(chain.stretch.is_some(), "{speed:?}");
        let (reach, busy_from, to) = (chain.reads_within(), chain.busy_from(), chain.to);
        let mapping = chain.mapping;
        let mut block = at;
        while block < to {
            graph.process(&mut feed, 480).unwrap();
            let end = block + 480;
            let (a, b) = (mapping.at(block).0, mapping.at(end - 1).0);
            let (low, high) = (a.min(b) - reach, a.max(b) + reach + 1);
            for range in reads.borrow_mut().drain(..) {
                assert!(
                    end > busy_from,
                    "{speed:?}: a read before it warms, at {block}"
                );
                assert!(
                    range.start >= low && range.end <= high,
                    "{speed:?} backward {reverse}: {range:?} read in the block at {block}, not within {low}..{high}"
                );
            }
            block = end;
        }
    }
}

/// The sample project with its track of sound only, its clip ten seconds long at `speed`, Pitch
/// Correction as `corrected`, its pitch moved by `semitones`, and a Pitch of its own.
fn clip_with_a_pitch(
    speed: (u32, u32),
    corrected: bool,
    semitones: i32,
) -> columbina_model::Project {
    use columbina_model::audio::fx::PitchFx;
    use columbina_model::{ClipPitch, Effect, Insert, Project, Speed, Time, TrackKind};

    let mut project = Project::sample();
    project.timelines[0]
        .tracks
        .retain(|track| track.kind == TrackKind::Audio);
    project.media[0].facts.duration = Time {
        ticks: 60 * 15_360,
        scale: 15_360,
    };
    let clip = &mut project.timelines[0].tracks[0].clips[0];
    clip.length = 250;
    clip.speed = Speed {
        num: speed.0,
        den: speed.1,
    };
    clip.pitch_correction = corrected;
    clip.pitch = ClipPitch {
        semitones,
        cents: 0.0,
    };
    clip.effects = vec![Insert {
        id: String::new(),
        on: true,
        effect: Effect::Pitch(PitchFx {
            semitones: 4,
            ..PitchFx::default()
        }),
    }];
    project
}

/// A clip with effects (T4c) reads its media as far ahead of the timeline as its effects are late,
/// and only within what its [`super::Reach`] tells a reader ahead of the graph (T5), whichever
/// way its run starts.
#[test]
fn a_clip_with_effects_reads_where_its_reach_says_once_its_run_has_started() {
    use std::cell::RefCell;
    use std::ops::Range;
    use std::rc::Rc;

    use super::{Graph, Options, Purpose};
    use crate::feed::FeedError;

    // Stretched with its pitch kept, stretched with its pitch moved alone, and resampled.
    for (within, (speed, corrected, semitones, at)) in
        [false, true].into_iter().flat_map(|within| {
            [
                ((4, 5), true, 0, 6 * 48_000),
                ((1, 1), false, 2, 5 * 48_000 + 999),
                ((3, 2), false, 0, 2 * 48_000 + 17),
            ]
            .map(|case| (within, case))
        })
    {
        let project = clip_with_a_pitch(speed, corrected, semitones);
        let mut graph = Graph::build(&project, 0, Options::new(480, Purpose::Export)).unwrap();
        let reads: Rc<RefCell<Vec<Range<i64>>>> = Rc::default();
        let seen = Rc::clone(&reads);
        let mut feed = move |_: usize, range: Range<i64>, out: &mut [&mut [f32]]| {
            for (channel, out) in out.iter_mut().enumerate() {
                for (n, sample) in out.iter_mut().enumerate() {
                    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
                    let x = (range.start + i64::try_from(n).unwrap()) as f64;
                    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
                    let value = (0.5 * (x * 0.05 + channel as f64).sin()) as f32;
                    *sample = value;
                }
            }
            seen.borrow_mut().push(range);
            Ok::<(), FeedError>(())
        };
        if within {
            graph.start_within(at, 480, &mut feed).unwrap();
        } else {
            graph.start(at, &mut feed).unwrap();
        }
        reads.borrow_mut().clear();
        let chain = &graph.clips[0];
        let (ahead, to) = (chain.ahead(), chain.to);
        assert!(ahead > 6000, "{ahead}");
        let reach = chain.reach();
        let span = reach.span();
        assert_eq!(span.start, chain.busy_from());
        let mut block = at;
        let mut read_any = false;
        while block < span.end + 480 {
            graph.process(&mut feed, 480).unwrap();
            let end = block + 480;
            let wanted = reach.source(block..end);
            for range in reads.borrow_mut().drain(..) {
                read_any = true;
                let Some(wanted) = &wanted else {
                    panic!(
                        "{speed:?}: {range:?} read in the block at {block}, where it reads none"
                    );
                };
                assert!(
                    range.start >= wanted.start && range.end <= wanted.end,
                    "{speed:?}: {range:?} read in the block at {block}, not within {wanted:?}"
                );
            }
            block = end;
        }
        assert!(read_any);
        // What it reads at the last block that reads is what it sounds a latency later.
        assert_eq!(span.end, to - ahead);
    }
}

/// A value held live on a lane of frequencies comes in and is let go on the lane's own scale,
/// the logarithm (T16's review): as the lane is written, a glide of a frequency straight in its
/// logarithm.
#[test]
fn a_value_held_on_a_lane_of_logarithms_comes_in_and_goes_on_its_scale() {
    use super::automation::{Held, Live};
    use columbina_model::LaneKind;
    let mut held = Held::new(48_000);
    held.take(Live::Hold(10_000.0), LaneKind::Continuous);
    let mut values = vec![100.0_f32; 960];
    held.apply(LaneKind::Continuous, true, &mut values);
    // Half way through the 10 ms (the 240th sample), the geometric mean: 1 kHz.
    assert!((values[239] - 1_000.0).abs() < 1.0, "{}", values[239]);
    assert!((values[959] - 10_000.0).abs() < 1e-2);
    held.take(Live::Release(4_800), LaneKind::Continuous);
    let mut back = vec![100.0_f32; 4_800];
    held.apply(LaneKind::Continuous, true, &mut back);
    assert!((back[2_399] - 1_000.0).abs() < 1.0, "{}", back[2_399]);
    // On a lane of a straight scale, half way is the mean.
    let mut held = Held::new(48_000);
    held.take(Live::Hold(10_000.0), LaneKind::Continuous);
    let mut values = vec![100.0_f32; 480];
    held.apply(LaneKind::Continuous, false, &mut values);
    assert!((values[239] - 5_050.0).abs() < 1.0, "{}", values[239]);
}
