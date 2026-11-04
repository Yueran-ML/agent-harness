//! What the tests of the effects of T10 share: signals, measures, and the checks of a change
//! without zipper and of the work of the thread of the mix.

#![allow(
    dead_code,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "each test binary uses some of these; sample numbers and levels of tests"
)]

use std::f64::consts::TAU;

use columbina_audio::testing::{realtime, render, Allocations, Run};
use columbina_audio::{Automated, Automation, Block, Node, ParamId, Planar, Spec};

pub const RATE: u32 = 48_000;

/// A sine of `freq` Hz and `amplitude` at 48 kHz, from `phase` radians.
pub fn sine_at(freq: f64, amplitude: f64, phase: f64, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|n| (amplitude * (TAU * freq * n as f64 / f64::from(RATE) + phase).sin()) as f32)
        .collect()
}

/// The amplitude of the component of `freq` Hz of `signal` at 48 kHz: its transform at that
/// frequency, exactly, over the signal (best over a whole number of periods).
pub fn amplitude_at(signal: &[f32], freq: f64) -> f64 {
    let w = TAU * freq / f64::from(RATE);
    let (mut re, mut im) = (0.0, 0.0);
    for (n, &x) in signal.iter().enumerate() {
        let phase = w * n as f64;
        re += f64::from(x) * phase.cos();
        im -= f64::from(x) * phase.sin();
    }
    2.0 * re.hypot(im) / signal.len() as f64
}

/// Decibels of a gain of amplitude.
pub fn db(gain: f64) -> f64 {
    20.0 * gain.log10()
}

/// The largest size of `signal`.
pub fn peak(signal: &[f32]) -> f32 {
    signal.iter().fold(0.0, |peak, x| peak.max(x.abs()))
}

/// The largest step between two samples of `signal`.
pub fn largest_step(signal: &[f32]) -> f32 {
    signal
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).abs())
        .fold(0.0, f32::max)
}

/// The output of `node`, prepared for `spec`, through `run`, processed whole.
pub fn process<N: Node>(mut node: N, spec: &Spec, run: &Run) -> Vec<Vec<f32>> {
    let spec = spec.with_max_block(run.frames.max(1));
    node.prepare(&spec).expect("the node takes the spec");
    render(&mut node, &spec, run, |left| left)
}

/// Checks that a change of `param` to `value` at the sample `at` of `input` (a tone, the change
/// at one of its peaks) makes no step larger than 1.1 times the largest step of the steady
/// output of any node on its way: `make(t)` for `t` from 0 (the node before) to 1 (after), in
/// `steps`. The node starts as `make(0)`; the output is processed in blocks of 480.
pub fn no_zipper<N: Node>(
    make: impl Fn(f32) -> N,
    spec: &Spec,
    input: &[Vec<f32>],
    steps: usize,
    change: (ParamId, f32),
    at: usize,
) {
    let frames = input[0].len();
    let run = Run::new(input.to_vec());
    let bound = (0..steps)
        .map(|k| {
            let t = k as f32 / (steps - 1) as f32;
            let output = process(make(t), spec, &run);
            output
                .iter()
                .map(|channel| largest_step(&channel[frames / 2..]))
                .fold(0.0, f32::max)
        })
        .fold(0.0, f32::max);
    let mut node = make(0.0);
    let spec = spec.with_max_block(512);
    node.prepare(&spec).unwrap();
    let output = render(
        &mut node,
        &spec,
        &run.with_change(at, change.0, change.1),
        |_| 480,
    );
    for channel in &output {
        let measured = largest_step(&channel[at - 2_000..]);
        assert!(
            measured <= 1.1 * bound,
            "a step of {measured}, the steady ones at most {bound}"
        );
    }
}

/// Runs `node`, prepared for `spec` (its blocks of 512 at most), through `blocks` blocks of 480
/// as the thread of the mix would, with every parameter of `changes` set now and then, a reset
/// half way, lanes of `lanes` on every third block, and the settling asked after each: what it
/// allocated and freed.
pub fn realtime_work<N: Node>(
    node: &mut N,
    spec: &Spec,
    changes: &[(ParamId, f32)],
    lanes: &[(ParamId, Vec<f32>)],
    blocks: usize,
) -> Allocations {
    const BLOCK: usize = 480;
    node.prepare(spec).unwrap();
    let mut input = Planar::new(spec.inputs(), 512);
    for channel in 0..spec.inputs() {
        input
            .channel_mut(channel)
            .copy_from_slice(&columbina_audio::testing::noise(
                50 + channel as u64,
                0.9,
                512,
            ));
    }
    let mut sidechain = Planar::new(spec.sidechain, 512);
    for channel in 0..spec.sidechain {
        sidechain
            .channel_mut(channel)
            .copy_from_slice(&columbina_audio::testing::noise(
                80 + channel as u64,
                0.9,
                512,
            ));
    }
    let mut output = Planar::new(spec.outputs(), 512);
    let automated: Vec<Automated> = lanes
        .iter()
        .map(|(param, values)| Automated {
            param: *param,
            values: &values[..BLOCK],
        })
        .collect();
    let mut processed = 0_u64;
    let ((), counted) = realtime(|| {
        for n in 0..blocks {
            if n % 5 == 0 {
                for (k, &(param, value)) in changes.iter().enumerate() {
                    if (n / 5 + k) % 2 == 0 {
                        node.set(param, value);
                    }
                }
            }
            if n == blocks / 2 {
                node.reset();
                processed = 0;
            }
            let automation = if n % 3 == 0 {
                Automation::new(&automated)
            } else {
                Automation::NONE
            };
            let block = Block::new(processed, spec.rate, BLOCK).with_automation(automation);
            processed += BLOCK as u64;
            sidechain.read(BLOCK, |sidechain| {
                input.read(BLOCK, |input| {
                    output.write(BLOCK, |output| {
                        node.process(&block.with_sidechain(sidechain), input, output);
                    });
                });
            });
            let _ = node.settle();
        }
    });
    counted
}

/// The frequency of the strongest component of `signal` at 48 kHz: the peak of the spectrum of
/// its first 65536 samples (Hann window, zero-padded where shorter), placed between the bins by
/// the parabola through the logarithms of the three around it.
pub fn peak_frequency(signal: &[f32]) -> f64 {
    const SIZE: usize = 1 << 16;
    let mut fft = columbina_audio::fft::RealFft::new(SIZE);
    let mut windowed = fft.signal();
    let length = signal.len().min(SIZE);
    for (n, (out, x)) in windowed.iter_mut().zip(signal).enumerate() {
        let hann = 0.5 - 0.5 * (TAU * n as f64 / length as f64).cos();
        *out = (f64::from(*x) * hann) as f32;
    }
    let mut spectrum = fft.spectrum();
    fft.forward(&mut windowed, &mut spectrum);
    let magnitude = |k: usize| f64::from(spectrum[k].norm()).max(1e-30).ln();
    let k = (1..spectrum.len() - 1)
        .max_by(|&a, &b| magnitude(a).total_cmp(&magnitude(b)))
        .unwrap();
    let (a, b, c) = (magnitude(k - 1), magnitude(k), magnitude(k + 1));
    let offset = 0.5 * (a - c) / (a - 2.0 * b + c);
    (k as f64 + offset) * f64::from(RATE) / SIZE as f64
}

/// The cents from `reference` to `measured`.
pub fn cents(measured: f64, reference: f64) -> f64 {
    1200.0 * (measured / reference).log2()
}

/// Checks that the level of a steady tone in `signal` (the root mean square over windows of 1 ms)
/// holds over the crossfade around the sample `point` where a stretch starts again: within
/// 0.7 (−3 dB) to 1.2 times the lowest and highest level over the half second before it (the stretch's
/// own ripple). `what` says which.
pub fn level_holds_across(signal: &[f32], point: usize, what: &str) {
    let window = RATE as usize / 1_000;
    let level = |at: usize| {
        (signal[at..at + window].iter().map(|s| s * s).sum::<f32>() / window as f32).sqrt()
    };
    let steady: Vec<f32> = (point - RATE as usize / 2..point - RATE as usize / 20)
        .step_by(window)
        .map(level)
        .collect();
    let (low, high) = steady.iter().fold((f32::MAX, 0.0_f32), |(low, high), &l| {
        (low.min(l), high.max(l))
    });
    for at in (point - 2 * window * 10..point + 2 * window * 10).step_by(window / 4) {
        let l = level(at);
        assert!(
            l >= 0.7 * low && l <= 1.2 * high,
            "{what}: a level of {l} at {at}, the steady ones {low} to {high}"
        );
    }
}
