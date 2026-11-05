//! The timeline played at a speed other than 1, and scrubbed (T18), on the clock of a simulated
//! device ([`session::Sim`]): at 2, 4 and 8 times, forward and backward, a tone sounds as many
//! times higher, backward at 1 the media plays reversed to the bit, every sample out is where
//! the warp puts it on the timeline (clips, fades and lanes), a change of speed while it plays is
//! a crossfade with no click and no new session, and grains of scrubbing sound 60 ms each, faded
//! 5 ms at either end, where they are sent.

mod common;
mod session;

use std::ops::Range;

use columbina_audio::feed::{ClipFeed, FeedError};
use columbina_audio::graph::{Graph, Options, Purpose, Warp};
use columbina_model::{Lane, LaneKind, LanePoint, Project, Time};

use common::{cents, peak_frequency};
use session::{largest_step, one_track, Sim, Tone, RATE};

/// A tone at a quarter of full scale.
const TONE: Tone = Tone {
    hz: 440.0,
    amplitude: 0.5,
};

/// A level of a half.
const LEVEL: Tone = Tone {
    hz: 0.0,
    amplitude: 0.5,
};

/// 15 s: far enough from both ends of the clip of [`one_track`] (0 to 30 s) for 1.5 s at 8 times
/// either way.
const MIDDLE: i64 = 15 * RATE as i64;

/// The left side of `sim` after `ticks` more runs of its device.
fn played(sim: &mut Sim, ticks: usize) -> Vec<f32> {
    for _ in 0..ticks {
        sim.tick();
    }
    sim.left()
}

#[test]
fn at_two_four_and_eight_times_forward_and_backward_a_tone_sounds_as_many_times_higher() {
    for speed in [2_i64, 4, 8, -1, -2, -8] {
        let mut sim = Sim::at_speed(&one_track(), MIDDLE, TONE, speed);
        let left = played(&mut sim, 150);
        let measured = peak_frequency(&left[2_000..]);
        #[allow(clippy::cast_precision_loss, reason = "a speed")]
        let want = 440.0 * speed.unsigned_abs() as f64;
        println!(
            "at {speed}: {measured:.3} Hz ({:+.3} cents)",
            cents(measured, want)
        );
        assert!(
            cents(measured, want).abs() < 5.0,
            "at {speed}: {measured} Hz, not {want}"
        );
        // The level of the tone within 1 % (the filter of shuttling, of 3 zero crossings, lets
        // its band ripple some 0.35 % at 8).
        let peak = left[2_000..]
            .iter()
            .fold(0.0_f32, |most, s| most.max(s.abs()));
        println!("at {speed}: the tone's peak {peak}");
        assert!((peak - 0.5).abs() <= 0.005, "at {speed}: {peak}");
        assert_eq!(sim.counters.underruns(), 0, "at {speed}");
        assert_eq!(sim.control.shared.short(), 0, "at {speed}");
    }
}

#[test]
fn a_tone_that_a_speed_puts_above_half_the_rate_is_filtered_out_not_folded_back() {
    // 5 kHz at 8 times is 40 kHz, above the 24 kHz that 48 kHz holds: unfiltered it would fold
    // back to 8 kHz.
    let tone = Tone {
        hz: 5_000.0,
        amplitude: 0.5,
    };
    for speed in [8_i64, -8] {
        let mut sim = Sim::at_speed(&one_track(), MIDDLE, tone, speed);
        let left = played(&mut sim, 60);
        let rms = (left[2_000..].iter().map(|s| s * s).sum::<f32>()
            / f32::from(u16::try_from(left.len() - 2_000).unwrap_or(u16::MAX)))
        .sqrt();
        #[allow(clippy::cast_possible_truncation, reason = "decibels of a test")]
        let db = 20.0 * f64::from(rms / (0.5 / std::f32::consts::SQRT_2)).log10();
        println!("5 kHz at {speed}: {db:.1} dB");
        assert!(db < -40.0, "at {speed}: {db} dB");
    }
}

/// A chirp of 1 kHz at [`MIDDLE`] in the media, rising 250 Hz a second.
#[derive(Clone, Copy)]
struct Chirp;

impl Chirp {
    /// The seconds of sample `sample` of the media from [`MIDDLE`].
    fn seconds(sample: i64) -> f64 {
        #[allow(clippy::cast_precision_loss, reason = "seconds of a test")]
        let t = (sample - MIDDLE) as f64 / f64::from(RATE);
        t
    }

    fn at(sample: i64) -> f32 {
        let t = Chirp::seconds(sample);
        // The phase of 1000 t + 125 t².
        let phase = std::f64::consts::TAU * (1_000.0 * t + 125.0 * t * t);
        #[allow(clippy::cast_possible_truncation, reason = "a sample")]
        let sample = (0.5 * phase.sin()) as f32;
        sample
    }

    /// Its frequency at sample `sample` of the media.
    fn hz(sample: i64) -> f64 {
        1_000.0 + 250.0 * Chirp::seconds(sample)
    }
}

impl ClipFeed for Chirp {
    fn read(
        &mut self,
        _: usize,
        range: Range<i64>,
        out: &mut [&mut [f32]],
    ) -> Result<(), FeedError> {
        for out in out.iter_mut() {
            for (sample, value) in (range.start..).zip(out.iter_mut()) {
                *value = Chirp::at(sample);
            }
        }
        Ok(())
    }
}

/// What `project` plays from `from` at `speed` for `frames`, its clips read through `feed`, as
/// the graph gives it (no device).
fn through(
    project: &Project,
    (from, speed): (i64, i64),
    feed: &mut dyn ClipFeed,
    frames: usize,
) -> Vec<f32> {
    let mut graph = Graph::build(project, 0, Options::new(4096, Purpose::Export)).unwrap();
    graph.set_warp(Warp::new(speed, from, from));
    graph.start(from, feed).unwrap();
    let mut left = Vec::with_capacity(frames);
    while left.len() < frames {
        let n = (frames - left.len()).min(1_000);
        graph.process(feed, n).unwrap();
        left.extend_from_slice(&graph.output(graph.monitor().unwrap()).channel(0)[..n]);
    }
    left
}

#[test]
fn backward_at_speed_one_the_media_plays_reversed_to_the_bit_and_a_chirp_falls() {
    // Each sample out at −1 is the sample of the timeline as many before where it started, the
    // same bits as playing forward gives it there.
    let project = one_track();
    let frames = 3 * RATE as usize;
    let back = through(&project, (MIDDLE, -1), &mut Chirp, frames);
    let start = MIDDLE - i64::try_from(frames).unwrap() + 1;
    let forward = through(&project, (start, 1), &mut Chirp, frames);
    for (n, sample) in back.iter().enumerate() {
        assert_eq!(
            sample.to_bits(),
            forward[frames - 1 - n].to_bits(),
            "sample {n} out"
        );
    }
    // The chirp falls through what is heard, as it rose in the media.
    let window = 16_384;
    let early = peak_frequency(&back[..window]);
    let late = peak_frequency(&back[frames - window..]);
    let middle = |first: usize| {
        let sample = MIDDLE - i64::try_from(first + window / 2).unwrap();
        Chirp::hz(sample)
    };
    println!(
        "backward: {early:.1} Hz at first ({:.1} in the media), {late:.1} Hz at last ({:.1})",
        middle(0),
        middle(frames - window)
    );
    assert!(early > late + 500.0, "{early} Hz, then {late} Hz");
    assert!((early - middle(0)).abs() < 20.0);
    assert!((late - middle(frames - window)).abs() < 20.0);
    // And forward it rises.
    let early = peak_frequency(&forward[..window]);
    let late = peak_frequency(&forward[frames - window..]);
    assert!(late > early + 500.0, "{early} Hz, then {late} Hz");
}

/// A single 1 in the media, at its sample `at`.
#[derive(Clone, Copy)]
struct Click {
    at: i64,
}

impl ClipFeed for Click {
    fn read(
        &mut self,
        _: usize,
        range: Range<i64>,
        out: &mut [&mut [f32]],
    ) -> Result<(), FeedError> {
        for out in out.iter_mut() {
            out.fill(0.0);
            if range.contains(&self.at) {
                out[usize::try_from(self.at - range.start).unwrap()] = 1.0;
            }
        }
        Ok(())
    }
}

#[test]
fn a_click_in_the_media_sounds_at_the_sample_out_that_the_warp_puts_it_at() {
    for speed in [1_i64, 2, 3, 4, 8, -1, -2, -8] {
        let from = MIDDLE - 7 * speed;
        let click = MIDDLE + 5_000 * speed + 3;
        let left = through(
            &one_track(),
            (from, speed),
            &mut Click { at: click },
            48_000,
        );
        let (loudest, _) = left
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .unwrap();
        let loudest = i64::try_from(loudest).unwrap();
        // Sample n out is sample from + speed · n of the timeline.
        let at = Warp::new(speed, from, from).timeline(from + loudest);
        println!("at {speed}: the click at sample {loudest} out, {at} of the timeline");
        assert!(
            (at - click).abs() <= speed.abs() / 2,
            "at {speed}: {at}, not {click}"
        );
    }
}

/// [`one_track`] with its fader on a lane: −∞ until sample `step` of the timeline, 0 dB from
/// the next, and a fade out of the clip over its last second.
fn stepped(step: i64) -> Project {
    let mut project = one_track();
    let track = &mut project.timelines[0].tracks[0];
    let point = |ticks: i64, value: f64| LanePoint {
        at: Time { ticks, scale: RATE },
        value,
    };
    track.audio.lanes.insert(
        "fader_db".into(),
        Lane {
            points: vec![point(step, -100.0), point(step + 1, 0.0)],
            kind: LaneKind::Continuous,
            scale: None,
        },
    );
    track.clips[0].fade_out = 25;
    project
}

#[test]
fn every_sample_out_is_the_sample_of_the_timeline_that_the_warp_puts_it_at() {
    // A level through a lane of the fader and the fade out of the clip: each sample out at a
    // speed is what playing at speed 1 gives at its sample of the timeline.
    let end = 30 * i64::from(RATE);
    let step = end - 2 * i64::from(RATE);
    let project = stepped(step);
    let mut level = LEVEL;
    let all = through(
        &project,
        (0, 1),
        &mut level,
        usize::try_from(end + 4_800).unwrap(),
    );
    for speed in [2_i64, 3, -1, -4] {
        let from = if speed > 0 { step - 9_000 } else { end + 1_000 };
        let frames = 30_000;
        let left = through(&project, (from, speed), &mut level, frames);
        let warp = Warp::new(speed, from, from);
        let mut heard = 0;
        for (n, sample) in left.iter().enumerate() {
            let at = warp.timeline(from + i64::try_from(n).unwrap());
            let want = usize::try_from(at).ok().and_then(|at| all.get(at)).copied();
            let want = want.unwrap_or(0.0);
            assert!(
                (sample - want).abs() < 1e-5,
                "at {speed}, sample {n} out ({at} of the timeline): {sample}, not {want}"
            );
            heard += usize::from(sample.abs() > 1e-3);
        }
        assert!(heard > 1_000, "at {speed}: {heard}");
    }
}

#[test]
fn a_change_of_speed_while_it_plays_is_a_crossfade_with_no_click_and_no_new_session() {
    for (from, to) in [(1_i64, 2_i64), (2, 1), (1, -1), (4, -8), (-2, 8)] {
        let project = one_track();
        let mut before = Sim::at_speed(&project, MIDDLE, TONE, from);
        let mut sim = Sim::at_speed(&project, MIDDLE, TONE, from);
        let mut at = 0;
        let mut warp = Warp::IDENTITY;
        for tick in 0..60 {
            if tick == 30 {
                at = sim.speed_swap(&project, to, 2_400);
                warp = sim.warp;
            }
            before.tick();
            sim.tick();
        }
        // What plays at the new speed from where the change puts the timeline.
        let mut after = Sim::through(
            &project,
            at,
            TONE,
            (columbina_audio::graph::node_of, warp, None),
        );
        let after = played(&mut after, 60);
        let at = usize::try_from(at - MIDDLE).unwrap();
        let fade = 480;
        let got = sim.left();
        let old = before.left();
        let differs = |range: Range<usize>, other: &[f32], offset: usize| {
            range
                .map(|n| (got[n] - other[n - offset]).abs())
                .fold(0.0, f32::max)
        };
        assert!(differs(0..at, &old, 0) < 1e-6, "{from} → {to}: before");
        assert!(
            differs(at + fade..got.len(), &after, at) < 1e-6,
            "{from} → {to}: after"
        );
        let normal = largest_step(&old).max(largest_step(&after));
        let steps = largest_step(&got);
        println!("{from} → {to} at {at}: the largest step {steps}, {normal} at either speed");
        assert!(
            steps <= normal * 1.1,
            "{from} → {to}: {steps} > {normal} · 1.1"
        );
        assert_eq!(sim.counters.swaps(), 1);
        assert_eq!(sim.counters.underruns(), 0);
        assert_eq!(sim.control.shared.short(), 0);
    }
}

/// The runs of samples that are not 0 in `signal`: where each starts, and how long it is.
fn segments(signal: &[f32]) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut start = None;
    for (n, &sample) in signal.iter().enumerate() {
        match (start, sample != 0.0) {
            (None, true) => start = Some(n),
            (Some(first), false) => {
                found.push((first, n - first));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(first) = start {
        found.push((first, signal.len() - first));
    }
    found
}

#[test]
fn ten_grains_of_scrubbing_sound_60_ms_each_faded_5_ms_where_they_are_sent() {
    // A level, so that every sample of a grain is heard: the first at 0, then one every 100 ms
    // at another place, each sent 50 ms ahead of what is mixed.
    let project = one_track();
    let first = 3 * i64::from(RATE);
    let mut sim = Sim::scrubbing(&project, first, LEVEL);
    let mut sent = vec![(0_i64, first)];
    for tick in 1..110 {
        if tick % 10 == 0 && tick < 100 {
            let place = first + 7_919 * i64::from(tick);
            let at = sim.grain(&project, place, 2_400);
            sent.push((at - first, place));
        }
        sim.tick();
    }
    let left = sim.left();
    let found = segments(&left);
    println!("grains: {found:?}");
    assert_eq!(found.len(), 10);
    for (&(start, length), &(at, _)) in found.iter().zip(&sent) {
        assert_eq!(i64::try_from(start).unwrap(), at);
        assert_eq!(length, 2_880, "a grain of 60 ms at 48 kHz");
        let sounded = &left[start..start + length];
        // Straight fades of 240 samples (5 ms), the level between.
        for (k, &sample) in sounded.iter().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "a share of a fade")]
            let gain = ((k + 1).min(240).min(length - k)) as f32 / 240.0;
            assert!((sample - 0.5 * gain).abs() < 1e-6, "sample {k}: {sample}");
        }
    }
    assert_eq!(sim.counters.swaps(), 9);
    assert_eq!(sim.counters.underruns(), 0);
    assert_eq!(sim.control.shared.short(), 0);
}

#[test]
fn a_grain_sounds_the_timeline_at_its_place_and_one_sent_while_another_sounds_takes_over_without_a_click(
) {
    // A tone: each grain is the timeline where it was sent; grains every 30 ms, each taking the
    // place of the one before before it ends, crossfaded over 5 ms.
    let project = one_track();
    let first = 3 * i64::from(RATE);
    let mut sim = Sim::scrubbing(&project, first, TONE);
    let mut sent = Vec::new();
    for tick in 1..60 {
        if tick % 3 == 0 {
            let place = first + 1_013 * i64::from(tick);
            sent.push((sim.grain(&project, place, 960), place));
        }
        sim.tick();
    }
    let left = sim.left();
    for &(at, place) in &sent[..sent.len() - 1] {
        // Past the fade in, the grain is the tone at its place.
        let n = usize::try_from(at - first).unwrap() + 300;
        let want = TONE.at(place + 300);
        assert!((left[n] - want).abs() < 1e-6, "{} != {want}", left[n]);
    }
    let normal = largest_step(&played(&mut Sim::of(&project, first, TONE), 20));
    let steps = largest_step(&left);
    // A crossfade of two tones of other phases moves at most a fade's step of their difference
    // (2 · 0.5 / 240) beyond either.
    println!("grains every 30 ms: the largest step {steps}, the tone's {normal}");
    assert!(steps <= normal + 1.0 / 240.0 + 1e-6, "{steps}");
    assert_eq!(sim.counters.underruns(), 0);
    assert_eq!(sim.control.shared.short(), 0);
}

#[test]
fn the_span_of_a_warp_is_exactly_the_samples_out_that_lie_in_a_range_of_the_timeline() {
    for speed in [1_i64, 2, 3, 8, -1, -2, -3, -8] {
        for (at, timeline) in [(0, 0), (1_000, 17), (-5, 4_003), (77, -91)] {
            let warp = Warp::new(speed, at, timeline);
            assert_eq!(warp.timeline(at), timeline);
            for range in [0..1, 10..11, 3..40, -50..-7, 100..100] {
                let inside: Vec<i64> = (-20_000..20_000)
                    .filter(|&u| range.contains(&warp.timeline(u)))
                    .collect();
                let span = warp.span(range.clone());
                assert_eq!(
                    span.clone().collect::<Vec<i64>>(),
                    inside,
                    "at {speed}, {at} → {timeline}: {range:?} gave {span:?}"
                );
            }
            // Nothing of a timeline that ends at 500 plays from the end out on.
            let end = warp.end_of(500);
            let plays = |u: i64| (0..500).contains(&warp.timeline(u));
            assert!(!plays(end) && !plays(end + 1) && plays(end - 1) || !(-20_000..end).any(plays));
        }
    }
}

/// [`one_track`] with its clip at 0.8 with Pitch Correction (T13) and its fader on a lane that
/// moves from −12 dB to 0 dB over the first 20 s.
fn corrected() -> Project {
    let mut project = one_track();
    let track = &mut project.timelines[0].tracks[0];
    track.clips[0].speed = columbina_model::Speed { num: 4, den: 5 };
    track.clips[0].pitch_correction = true;
    let point = |ticks: i64, value: f64| LanePoint {
        at: Time { ticks, scale: RATE },
        value,
    };
    track.audio.lanes.insert(
        "fader_db".into(),
        Lane {
            points: vec![point(0, -12.0), point(20 * i64::from(RATE), 0.0)],
            kind: LaneKind::Continuous,
            scale: None,
        },
    );
    project
}

/// The left side of `project` exported from 0 for `frames`, its clips sounding `tone`.
fn exported(project: &Project, tone: Tone, frames: usize) -> Vec<f32> {
    let mut tone = tone;
    through(project, (0, 1), &mut tone, frames)
}

#[test]
fn back_at_speed_one_a_clip_keeps_its_pitch_correction_and_its_lanes_and_sounds_as_its_export() {
    // Backward at 1 from 15 s, then forward at 1 again (L after J): from the end of the
    // crossfade each sample out is the export's at its sample of the timeline, to the bit.
    let project = corrected();
    let mut sim = Sim::at_speed(&project, MIDDLE, TONE, -1);
    let mut at = 0;
    let mut warp = Warp::IDENTITY;
    for tick in 0..120 {
        if tick == 30 {
            at = sim.speed_swap(&project, 1, 2_400);
            warp = sim.warp;
        }
        sim.tick();
    }
    let left = sim.left();
    let export = exported(&project, TONE, usize::try_from(MIDDLE).unwrap() + 60_000);
    let first = usize::try_from(at - MIDDLE).unwrap() + 480;
    let mut checked = 0;
    for (n, sample) in left.iter().enumerate().skip(first) {
        let t = warp.timeline(MIDDLE + i64::try_from(n).unwrap());
        let want = export[usize::try_from(t).unwrap()];
        assert_eq!(
            sample.to_bits(),
            want.to_bits(),
            "sample {n} out, {t} of the timeline: {sample}, not {want}"
        );
        checked += 1;
    }
    assert!(checked > 20_000, "{checked}");
    assert_eq!(sim.counters.underruns(), 0);
    assert_eq!(sim.control.shared.short(), 0);
}

#[test]
fn every_grain_keeps_the_pitch_correction_of_its_clip_and_sounds_as_its_export() {
    let project = corrected();
    let first = 3 * i64::from(RATE);
    let export = exported(&project, TONE, 10 * RATE as usize);
    let mut sim = Sim::scrubbing(&project, first, TONE);
    let mut sent = vec![(first, first)];
    for tick in 1..60 {
        if tick % 10 == 0 && tick < 40 {
            let place = first + 9_973 * i64::from(tick);
            sent.push((sim.grain(&project, place, 2_400), place));
        }
        sim.tick();
    }
    let left = sim.left();
    for &(at, place) in &sent {
        let start = usize::try_from(at - first).unwrap();
        for k in 0..2_880 {
            let gain = Sim::grain_at(0).gain(i64::try_from(k).unwrap());
            let want = export[usize::try_from(place).unwrap() + k] * gain;
            let got = left[start + k];
            assert!(
                (got - want).abs() < 1e-6,
                "the grain of {place}, sample {k}: {got}, not {want}"
            );
        }
    }
}

#[test]
fn a_grain_told_to_fade_fades_out_over_5_ms_and_the_next_grain_sounds_again() {
    let project = one_track();
    let first = 3 * i64::from(RATE);
    let mut sim = Sim::scrubbing(&project, first, LEVEL);
    for _ in 0..2 {
        sim.tick();
    }
    // Into the grain (60 ms from 0): stopped at once, it fades over 240 samples to silence.
    let sent = sim.sent();
    sim.fade();
    let mut next = 0;
    for tick in 0..20 {
        if tick == 10 {
            next = sim.grain(&project, first + 50_000, 960);
        }
        sim.tick();
    }
    let left = sim.left();
    let faded = (sent..left.len()).find(|&n| left[n] < 0.5).unwrap();
    let silent = (faded..left.len()).find(|&n| left[n] == 0.0).unwrap();
    println!("faded from {faded}, silent from {silent}");
    assert!(silent - faded <= 240, "{faded} to {silent}");
    assert!(largest_step(&left[sent..=silent]) <= 0.5 / 240.0 + 1e-6);
    let next = usize::try_from(next - first).unwrap();
    assert!(left[silent..next].iter().all(|&x| x == 0.0));
    assert!(
        (left[next + 300] - 0.5).abs() < 1e-6,
        "the next grain sounds"
    );
}
