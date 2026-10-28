//! Where a clip reads its source, at any speed, forward and backward, for random clips: at its
//! own speed as the first engine read it, in order, within what it shows, and as far as its
//! speed takes it.

use columbina_model::span::{
    frames_for_span, source_sample, source_span, source_tick, ticks_of_scaled,
};
use columbina_model::{Clip, Rate, Speed, Time};
use proptest::prelude::*;

const RATES: [Rate; 5] = [
    Rate { num: 25, den: 1 },
    Rate {
        num: 30000,
        den: 1001,
    },
    Rate {
        num: 24000,
        den: 1001,
    },
    Rate { num: 60, den: 1 },
    Rate { num: 1, den: 1 },
];

const SCALES: [u32; 5] = [15360, 90000, 1000, 48000, 1];

fn rate() -> impl Strategy<Value = Rate> {
    prop::sample::select(&RATES[..])
}

fn scale() -> impl Strategy<Value = u32> {
    prop::sample::select(&SCALES[..])
}

/// A speed from 1/100 to 100.
fn speed() -> impl Strategy<Value = Speed> {
    prop_oneof![
        Just(Speed::ONE),
        (1u32..=100, 1u32..=100).prop_map(|(num, den)| Speed { num, den }),
        (1u32..=100).prop_map(|den| Speed { num: 1, den }),
        (1u32..=100).prop_map(|num| Speed { num, den: 1 }),
    ]
}

/// A clip of `length` frames from `ticks` in a source of `scale`.
fn clip(ticks: i64, scale: u32, length: i64, speed: Speed, reverse: bool) -> Clip {
    let mut clip = Clip::of_media("c1", "m1", Time { ticks, scale }, 0, length);
    clip.speed = speed;
    clip.reverse = reverse;
    clip
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn at_its_own_speed_a_clip_reads_its_source_as_the_first_engine_did(
        rate in rate(),
        scale in scale(),
        ticks in 0i64..1_000_000,
        length in 1i64..10_000,
        k in 0i64..10_000,
    ) {
        let k = k % length;
        let forward = clip(ticks, scale, length, Speed::ONE, false);
        prop_assert_eq!(source_span(&forward, rate, scale), rate.ticks_of(length, scale));
        prop_assert_eq!(source_tick(&forward, k, rate, scale), ticks + rate.ticks_of(k, scale));
    }

    #[test]
    fn a_clip_reads_its_source_in_order_and_within_what_it_shows(
        rate in rate(),
        scale in scale(),
        speed in speed(),
        ticks in 0i64..1_000_000,
        length in 1i64..5_000,
        k in 0i64..5_000,
    ) {
        let k = k % length;
        let forward = clip(ticks, scale, length, speed, false);
        let backward = clip(ticks, scale, length, speed, true);
        let span = source_span(&forward, rate, scale);
        prop_assert_eq!(source_span(&backward, rate, scale), span);
        for clip in [&forward, &backward] {
            let at = source_tick(clip, k, rate, scale);
            prop_assert!(at >= ticks && at <= ticks + span, "{at} of [{ticks}, {ticks} + {span}]");
        }
        if k + 1 < length {
            let (now, next) = (source_tick(&forward, k, rate, scale), source_tick(&forward, k + 1, rate, scale));
            prop_assert!(now <= next);
            let (now, next) = (source_tick(&backward, k, rate, scale), source_tick(&backward, k + 1, rate, scale));
            prop_assert!(now >= next);
        }
        // Backward, a clip shows the frames that it shows forward, last first.
        prop_assert_eq!(
            source_tick(&backward, k, rate, scale),
            source_tick(&forward, length - 1 - k, rate, scale)
        );
        // Its last frame starts before the end of what it shows, where a frame lasts a tick or
        // more.
        let last = source_tick(&forward, length - 1, rate, scale);
        if ticks_of_scaled(1, speed, rate, scale) >= 1 {
            prop_assert!(last < ticks + span);
        }
    }

    #[test]
    fn a_clip_n_times_as_fast_shows_n_times_as_much_of_its_source(
        rate in rate(),
        scale in scale(),
        speed in speed(),
        length in 1i64..5_000,
        n in 1u32..=8,
    ) {
        let faster = Speed { num: speed.num * n, den: speed.den };
        let once = ticks_of_scaled(length, speed, rate, scale);
        let n_times = ticks_of_scaled(length, faster, rate, scale);
        // Each is rounded down: by less than a tick, and n of them by less than n.
        let error = n_times - i64::from(n) * once;
        prop_assert!((0..i64::from(n)).contains(&error), "{n_times} against {n} x {once}");
    }

    #[test]
    fn the_frames_for_a_span_are_the_most_whose_ticks_fit_in_it(
        rate in rate(),
        scale in scale(),
        speed in speed(),
        span in 0i64..10_000_000,
    ) {
        let frames = frames_for_span(span, speed, rate, scale);
        prop_assert!(frames >= 0);
        prop_assert!(ticks_of_scaled(frames, speed, rate, scale) <= span);
        prop_assert!(ticks_of_scaled(frames + 1, speed, rate, scale) > span);
    }

    #[test]
    fn a_clip_reads_the_samples_of_its_source_in_order_from_where_it_begins(
        rate in rate(),
        speed in speed(),
        reverse in any::<bool>(),
        seconds in 0i64..100,
        start in 0i64..1000,
        length in 1i64..500,
        sample_rate in prop::sample::select(vec![44100u32, 48000, 96000]),
        media_rate in prop::sample::select(vec![22050u32, 44100, 48000]),
        at in 0i64..1_000_000,
    ) {
        let mut clip = clip(seconds * 15360, 15360, length, speed, reverse);
        clip.start = start;
        let first = samples(start, rate, sample_rate);
        let end = samples(start + length, rate, sample_rate);
        let at = first + at % (end - first);
        let read = |sample| source_sample(&clip, rate, sample, sample_rate, media_rate);
        let begins = seconds * i64::from(media_rate);
        let (start_of_clip, end_of_clip) = (read(first), read(end - 1));
        if reverse {
            prop_assert!(start_of_clip >= end_of_clip);
            prop_assert_eq!(end_of_clip, begins);
        } else {
            prop_assert!(start_of_clip <= end_of_clip);
            prop_assert_eq!(start_of_clip, begins);
        }
        prop_assert!(read(at) >= begins);
        if at + 1 < end {
            if reverse {
                prop_assert!(read(at) >= read(at + 1));
            } else {
                prop_assert!(read(at) <= read(at + 1));
            }
        }
        // At its own speed, a second of the timeline is a second of the source.
        if speed == Speed::ONE && !reverse {
            let seconds_in = (at - first) / i64::from(sample_rate);
            let read_in = (read(at) - begins) / i64::from(media_rate);
            prop_assert!((seconds_in - read_in).abs() <= 1);
        }
    }
}

/// The sample of the timeline that frame `frame` starts at, rounded down.
fn samples(frame: i64, rate: Rate, sample_rate: u32) -> i64 {
    frame * i64::from(sample_rate) * i64::from(rate.den) / i64::from(rate.num)
}
