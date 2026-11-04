//! The signals of the tests of Noise Reduction (T12), made in code: white and pink noise of a
//! given RMS, a voice-like train of harmonics that comes and goes, and the measures of what a
//! node keeps of each.

#![allow(
    dead_code,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "the helpers of the tests, each file using some"
)]

use columbina_audio::testing::Rng;

/// White noise of `rms`, uniform, from `seed`.
pub fn white(seed: u64, rms: f32, frames: usize) -> Vec<f32> {
    let mut rng = Rng::new(seed);
    let peak = rms * 3.0_f32.sqrt();
    (0..frames).map(|_| peak * rng.signed()).collect()
}

/// Pink noise of `rms` (−3 dB an octave): white noise through P. Kellet's filter of seven poles,
/// scaled to the RMS.
pub fn pink(seed: u64, rms: f32, frames: usize) -> Vec<f32> {
    let mut rng = Rng::new(seed);
    let mut b = [0.0_f64; 7];
    let mut out: Vec<f64> = (0..frames)
        .map(|_| {
            let white = f64::from(rng.signed());
            b[0] = 0.99886 * b[0] + white * 0.055_517_9;
            b[1] = 0.99332 * b[1] + white * 0.075_075_9;
            b[2] = 0.96900 * b[2] + white * 0.153_852;
            b[3] = 0.86650 * b[3] + white * 0.310_485_6;
            b[4] = 0.55000 * b[4] + white * 0.532_952_2;
            b[5] = -0.7616 * b[5] - white * 0.016_898;
            let pink = b[0] + b[1] + b[2] + b[3] + b[4] + b[5] + b[6] + white * 0.5362;
            b[6] = white * 0.115_926;
            pink
        })
        .collect();
    let power = out.iter().map(|x| x * x).sum::<f64>() / frames as f64;
    let scale = f64::from(rms) / power.sqrt();
    for x in &mut out {
        *x *= scale;
    }
    out.into_iter().map(|x| x as f32).collect()
}

/// The length of a stretch of the voice, and of the silence after it, in seconds: it comes and
/// goes every half second (plan T12).
pub const STRETCH: f64 = 0.5;

/// Whether the voice speaks at the sample `n` at `rate`.
pub fn speaks(n: usize, rate: u32) -> bool {
    ((n as f64 / f64::from(rate) / STRETCH) as u64).is_multiple_of(2)
}

/// A voice-like sound at `rate`: the harmonics of 200 Hz, the h-th of 1/h, up to 3 kHz,
/// modulated at 4 Hz between 0.5 and 1 in amplitude, on for half a second and off for half a
/// second from the start, of `rms` while it is on.
pub fn voice(rate: u32, rms: f32, frames: usize) -> Vec<f32> {
    let mut out: Vec<f64> = (0..frames)
        .map(|n| {
            if !speaks(n, rate) {
                return 0.0;
            }
            let t = n as f64 / f64::from(rate);
            let tone: f64 = (1..=15)
                .map(|h| (std::f64::consts::TAU * 200.0 * f64::from(h) * t).sin() / f64::from(h))
                .sum();
            tone * (0.75 + 0.25 * (std::f64::consts::TAU * 4.0 * t).sin())
        })
        .collect();
    let on: Vec<f64> = out
        .iter()
        .enumerate()
        .filter(|(n, _)| speaks(*n, rate))
        .map(|(_, x)| *x)
        .collect();
    let power = on.iter().map(|x| x * x).sum::<f64>() / on.len() as f64;
    let scale = f64::from(rms) / power.sqrt();
    for x in &mut out {
        *x *= scale;
    }
    out.into_iter().map(|x| x as f32).collect()
}

/// The signal plus the noise.
pub fn sum(a: &[f32], b: &[f32]) -> Vec<f32> {
    a.iter().zip(b).map(|(a, b)| a + b).collect()
}

/// The decibels of a ratio of powers.
pub fn db(ratio: f64) -> f64 {
    10.0 * ratio.log10()
}

/// The stretches of silence of the voice from `from` seconds on, each without its first
/// `guard` seconds (where the tail of the voice may still be), as ranges of samples.
pub fn silences(rate: u32, frames: usize, from: f64, guard: f64) -> Vec<std::ops::Range<usize>> {
    stretches(rate, frames, from, guard, false)
}

/// The stretches of the voice from `from` seconds on, each without its first and last `guard`
/// seconds.
pub fn speeches(rate: u32, frames: usize, from: f64, guard: f64) -> Vec<std::ops::Range<usize>> {
    stretches(rate, frames, from, guard, true)
}

fn stretches(
    rate: u32,
    frames: usize,
    from: f64,
    guard: f64,
    speaking: bool,
) -> Vec<std::ops::Range<usize>> {
    let samples = |seconds: f64| (seconds * f64::from(rate)).round() as usize;
    let mut out = Vec::new();
    let mut k = 0_u64;
    loop {
        let start = k as f64 * STRETCH;
        let end = start + STRETCH;
        k += 1;
        if samples(end) > frames {
            break;
        }
        if start < from || (k % 2 == 1) != speaking {
            continue;
        }
        let tail = if speaking { guard } else { 0.0 };
        out.push(samples(start + guard)..samples(end - tail));
    }
    out
}

/// The power of `output`, `latency` behind, over `ranges` of the input, against the power of
/// `input` over them: what is left, in dB.
pub fn left(
    input: &[f32],
    output: &[f32],
    latency: usize,
    ranges: &[std::ops::Range<usize>],
) -> f64 {
    let (mut before, mut after) = (0.0_f64, 0.0_f64);
    for range in ranges {
        for n in range.clone() {
            before += f64::from(input[n]).powi(2);
            after += f64::from(output[n + latency]).powi(2);
        }
    }
    db(after / before)
}

/// How much of `clean` is in `output` over `ranges`, `latency` behind: the gain of the least
/// squares fit of `clean` to it, in dB (the noise, not like the voice, falls out of it).
pub fn kept(
    clean: &[f32],
    output: &[f32],
    latency: usize,
    ranges: &[std::ops::Range<usize>],
) -> f64 {
    let (mut cross, mut power) = (0.0_f64, 0.0_f64);
    for range in ranges {
        for n in range.clone() {
            cross += f64::from(clean[n]) * f64::from(output[n + latency]);
            power += f64::from(clean[n]).powi(2);
        }
    }
    20.0 * (cross / power).log10()
}
