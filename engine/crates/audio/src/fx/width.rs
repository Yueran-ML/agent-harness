//! Stereo Width (p. 3975): the spread of the left and right of a stereo signal, from 0 (mono)
//! through 1 (as it is) to 2 (wider), with Diffusion and Sparkle (C-X11).
//!
//! # Mid and side
//!
//! For the left `L` and right `R` of a sample, the mid `M = (L + R)/2` and the side
//! `S = (L − R)/2`; the side heard is
//!
//! ```text
//! S1 = S + d · (AP(S) − S)          Diffusion d = diffusion/100
//! S2 = S1 + k · HP(S1)              Sparkle k = sparkle/100 · WIDTH_SPARKLE_GAIN
//! L' = M + w · S2,  R' = M − w · S2  Width w
//! ```
//!
//! where `AP` is a chain of Schroeder all-passes (delays [`WIDTH_DIFFUSION_DELAYS`], feedback
//! [`WIDTH_DIFFUSION_FEEDBACK`]): the side keeps its spectrum and loses its shape in time, "more
//! complexity" (p. 3975); and `HP` is a high-pass of two poles, Butterworth, at
//! [`WIDTH_SPARKLE_HZ`] (at most 0.45 of the rate), which adds the high frequencies of the side,
//! up to +6 dB of them: "more high frequencies to the spread". The mid is never touched: what is
//! added and taken away cancels in `L' + R' = 2M`.
//!
//! Width 0 gives the same bits on both sides (`M` each); width 1 with no diffusion and no
//! sparkle gives the input to the bit (worked out as `L`, `R` themselves, not as `M ± S`, which
//! rounds).
//!
//! # Channels
//!
//! The left and the right of the layout, wherever they are (stereo, 3.0, 4.0, 5.0, 5.1, 7.0,
//! 7.1); the other channels, the centre, the LFE and the surrounds, pass to the bit. A layout of
//! no left and right (mono, an Adaptive track) is [`Unsupported`]: the plugin is off on a mono
//! track (p. 3975), and the graph leaves it out.
//!
//! # Changes while the sound plays
//!
//! Width, Diffusion and Sparkle ramp over 10 ms in their values ([`Smoothed`]); lanes are taken
//! as they are. The all-passes and the high-pass run all the time, whatever their amounts, so
//! that an amount turned up finds them where they would be. The switch of the insert crossfades
//! over 10 ms; off, the sound passes to the bit.

use super::biquad::Coefficients;
use super::common::{decay_samples, finite, lane, mix, params, ParamInfo};
use crate::constants::{
    samples, EQ_MAX_FREQUENCY_FRACTION, SMOOTHING, WIDTH_DIFFUSION_DELAYS,
    WIDTH_DIFFUSION_FEEDBACK, WIDTH_SPARKLE_GAIN, WIDTH_SPARKLE_HZ,
};
use crate::node::{Block, Node, ParamId, Role, Spec, Unsupported};
use crate::rt::{flush, Domain, Smoothed};

params! {
    /// A parameter of [`StereoWidth`], in the units of the model's `StereoWidthFx`.
    pub enum Param {
        /// The insert on (1) or off (0).
        On => ParamInfo::switch("on", true),
        /// From 0, mono, through 1, as it is, to 2.
        Width => ParamInfo::number("width", 0.0, 2.0, 1.0),
        /// Percent, 0 to 100.
        Diffusion => ParamInfo::number("diffusion", 0.0, 100.0, 0.0),
        /// Percent, 0 to 100.
        Sparkle => ParamInfo::number("sparkle", 0.0, 100.0, 0.0),
    }
}

/// The settings of a Stereo Width: the model's `StereoWidthFx`, and the switch of its insert.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// The insert is on.
    pub on: bool,
    /// 0 to 2.
    pub width: f32,
    /// 0 to 100.
    pub diffusion: f32,
    /// 0 to 100.
    pub sparkle: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            on: true,
            width: 1.0,
            diffusion: 0.0,
            sparkle: 0.0,
        }
    }
}

impl Settings {
    /// The value of `param`, as [`Node::set`] takes it.
    #[must_use]
    pub fn value(&self, param: Param) -> f32 {
        match param {
            Param::On => f32::from(u8::from(self.on)),
            Param::Width => self.width,
            Param::Diffusion => self.diffusion,
            Param::Sparkle => self.sparkle,
        }
    }

    /// Every parameter and its value: a Stereo Width set so has these settings.
    #[must_use]
    pub fn params(&self) -> [(ParamId, f32); 4] {
        [Param::On, Param::Width, Param::Diffusion, Param::Sparkle]
            .map(|param| (param.id(), self.value(param)))
    }
}

/// The delays of the all-passes at `rate`, in samples, at least 1.
fn delays(rate: u32) -> [usize; WIDTH_DIFFUSION_DELAYS.len()] {
    WIDTH_DIFFUSION_DELAYS.map(|seconds| samples(seconds, rate).max(1))
}

/// The high-pass of Sparkle at `rate`.
fn sparkle_filter(rate: u32) -> Coefficients {
    let rate = f64::from(rate);
    let freq = f64::from(WIDTH_SPARKLE_HZ).min(EQ_MAX_FREQUENCY_FRACTION * rate);
    Coefficients::high_pass(rate, freq, std::f64::consts::FRAC_1_SQRT_2)
}

/// The samples after which a Stereo Width at `rate`, started late, sounds within −90 dBFS of one
/// that played from earlier: its ramps (10 ms); each all-pass, whose state falls by its feedback
/// in each of its delays, to 1e-6; and the high-pass of Sparkle to 1e-6, one after the other.
/// The same for any settings: the filters run whatever the amounts.
#[must_use]
pub fn settle_for(_settings: &Settings, rate: u32) -> u32 {
    if rate == 0 {
        return 0;
    }
    let ramp = u64::try_from(samples(SMOOTHING, rate)).unwrap_or(u64::MAX);
    let rounds = u64::from(decay_samples(WIDTH_DIFFUSION_FEEDBACK));
    let all_passes: u64 = delays(rate)
        .iter()
        .map(|&delay| u64::try_from(delay).unwrap_or(u64::MAX) * rounds)
        .sum();
    let sparkle = u64::from(sparkle_filter(rate).decay(super::common::SETTLE_RESIDUE));
    u32::try_from(ramp + all_passes + sparkle).unwrap_or(u32::MAX)
}

/// A Schroeder all-pass of a delay line: y\[n\] = −g·x\[n\] + x\[n−D\] + g·y\[n−D\], kept as
/// v\[n\] = x\[n\] + g·v\[n−D\], y\[n\] = v\[n−D\] − g·v\[n\].
#[derive(Clone, Debug, Default)]
struct AllPass {
    line: Vec<f64>,
    at: usize,
}

impl AllPass {
    #[inline]
    fn tick(&mut self, x: f64) -> f64 {
        let g = WIDTH_DIFFUSION_FEEDBACK;
        let delayed = self.line[self.at];
        let v = flush(x + g * delayed);
        self.line[self.at] = v;
        self.at += 1;
        if self.at == self.line.len() {
            self.at = 0;
        }
        delayed - g * v
    }
}

/// Stereo Width, a [`Node`] of no latency.
#[derive(Clone, Debug)]
pub struct StereoWidth {
    rate: u32,
    on: bool,
    wet: Smoothed,
    width: Smoothed,
    diffusion: Smoothed,
    sparkle: Smoothed,
    /// The channels of the left and the right.
    sides: (usize, usize),
    all_passes: Vec<AllPass>,
    high_pass: Coefficients,
    /// The last two sides into the high-pass and out of it.
    high_past: ([f64; 2], [f64; 2]),
}

impl StereoWidth {
    /// A Stereo Width of `settings`, at once with no ramp.
    #[must_use]
    pub fn new(settings: &Settings) -> Self {
        let value = |param: Param| {
            param
                .info()
                .clamp(settings.value(param))
                .unwrap_or(param.info().default)
        };
        StereoWidth {
            rate: 0,
            on: settings.on,
            wet: Smoothed::new(f32::from(u8::from(settings.on)), Domain::Linear),
            width: Smoothed::new(value(Param::Width), Domain::Linear),
            diffusion: Smoothed::new(value(Param::Diffusion), Domain::Linear),
            sparkle: Smoothed::new(value(Param::Sparkle), Domain::Linear),
            sides: (0, 1),
            all_passes: Vec::new(), // rt-ok: empty, before the graph is built
            high_pass: Coefficients::IDENTITY,
            high_past: ([0.0; 2], [0.0; 2]),
        }
    }

    /// The side `s` through Diffusion `d` and Sparkle `k`.
    #[inline]
    fn side(&mut self, s: f64, d: f64, k: f64) -> f64 {
        let mut diffused = s;
        for all_pass in &mut self.all_passes {
            diffused = all_pass.tick(diffused);
        }
        let s1 = mix(s, diffused, d);
        let (input, output) = &mut self.high_past;
        let high = self.high_pass.tick(s1, *input, output);
        *input = [s1, input[0]];
        s1 + k * high
    }
}

impl StereoWidth {
    /// The width, the diffusion and the sparkle at the last sample processed: where their ramps
    /// are.
    #[must_use]
    pub fn heard(&self) -> (f32, f32, f32) {
        (
            self.width.current(),
            self.diffusion.current(),
            self.sparkle.current(),
        )
    }
}

impl Default for StereoWidth {
    fn default() -> Self {
        StereoWidth::new(&Settings::default())
    }
}

impl Node for StereoWidth {
    fn prepare(&mut self, spec: &Spec) -> Result<(), Unsupported> {
        let roles = spec.layout.roles();
        let left = roles.iter().position(|&role| role == Role::Left);
        let right = roles.iter().position(|&role| role == Role::Right);
        let (Some(left), Some(right)) = (left, right) else {
            return Err(Unsupported::from(spec));
        };
        if spec.layout != spec.output || spec.sidechain > 0 || spec.rate == 0 {
            return Err(Unsupported::from(spec));
        }
        self.rate = spec.rate;
        self.sides = (left, right);
        for ramp in [
            &mut self.wet,
            &mut self.width,
            &mut self.diffusion,
            &mut self.sparkle,
        ] {
            ramp.prepare(spec.rate);
        }
        self.all_passes = delays(spec.rate)
            .iter()
            .map(|&delay| AllPass {
                line: vec![0.0; delay], // rt-ok: when the graph is built
                at: 0,
            })
            .collect(); // rt-ok: when the graph is built
        self.high_pass = sparkle_filter(spec.rate);
        self.reset();
        Ok(())
    }

    #[allow(clippy::cast_possible_truncation, reason = "the samples are f32")]
    fn process(&mut self, ctx: &Block, input: &[&[f32]], output: &mut [&mut [f32]]) {
        let automation = &ctx.automation;
        let [on_lane, width_lane, diffusion_lane, sparkle_lane] =
            [Param::On, Param::Width, Param::Diffusion, Param::Sparkle]
                .map(|param| automation.get(param.id()));
        let value = |values, n, param: Param, ramp: f32| {
            lane(values, n)
                .and_then(|value| param.info().clamp(value))
                .unwrap_or(ramp)
        };
        let (left, right) = self.sides;
        for (channel, (output, input)) in output.iter_mut().zip(input).enumerate() {
            if channel != left && channel != right {
                output.copy_from_slice(input);
            }
        }
        for n in 0..ctx.frames {
            let on = lane(on_lane, n).map_or(self.on, |on| on >= 0.5);
            self.wet.set(f32::from(u8::from(on)));
            let wet = f64::from(self.wet.next_value());
            let width = f64::from(value(width_lane, n, Param::Width, self.width.next_value()));
            let diffusion = f64::from(value(
                diffusion_lane,
                n,
                Param::Diffusion,
                self.diffusion.next_value(),
            )) / 100.0;
            let sparkle = f64::from(value(
                sparkle_lane,
                n,
                Param::Sparkle,
                self.sparkle.next_value(),
            )) / 100.0
                * WIDTH_SPARKLE_GAIN;
            let (l, r) = (input[left][n], input[right][n]);
            let (l64, r64) = (finite(l), finite(r));
            let mid = f64::midpoint(l64, r64);
            let side = self.side(0.5 * (l64 - r64), diffusion, sparkle);
            #[allow(clippy::float_cmp, reason = "the one setting that passes the sound")]
            let neutral = width == 1.0 && diffusion == 0.0 && sparkle == 0.0;
            let (wide_l, wide_r) = if neutral {
                (f64::from(l), f64::from(r))
            } else {
                (mid + width * side, mid - width * side)
            };
            output[left][n] = mix(f64::from(l), wide_l, wet) as f32;
            output[right][n] = mix(f64::from(r), wide_r, wet) as f32;
        }
    }

    fn latency(&self) -> u32 {
        0
    }

    fn settle(&self) -> u32 {
        settle_for(&Settings::default(), self.rate)
    }

    fn reset(&mut self) {
        self.wet.jump(f32::from(u8::from(self.on)));
        self.width.finish();
        self.diffusion.finish();
        self.sparkle.finish();
        for all_pass in &mut self.all_passes {
            all_pass.line.fill(0.0);
            all_pass.at = 0;
        }
        self.high_past = ([0.0; 2], [0.0; 2]);
    }

    fn set(&mut self, param: ParamId, value: f32) {
        let Some(param) = Param::from_id(param) else {
            return;
        };
        let Some(value) = param.info().clamp(value) else {
            return;
        };
        match param {
            Param::On => self.on = value >= 0.5,
            Param::Width => self.width.set(value),
            Param::Diffusion => self.diffusion.set(value),
            Param::Sparkle => self.sparkle.set(value),
        }
    }
}
