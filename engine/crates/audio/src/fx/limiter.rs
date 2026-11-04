//! Limiter (pp. 3964–3966): a limiter of true peaks that looks 64 samples ahead
//! ([`LIMITER_LOOKAHEAD`]), with Input Level (−18 to +18 dB), Soft, Ceiling (−24 to 0 dBTP,
//! −1 for a new one), Release (0.01 to 1000 ms, 100 for a new one) and a sidechain (C-X5).
//!
//! # True peaks
//!
//! The level heard is the true peak of the sound ([`TruePeak`]): each channel oversampled 4 times
//! by a polyphase interpolator of 12 taps a phase (a sinc windowed by Kaiser, β =
//! [`LIMITER_KAISER_BETA`], each phase summing to 1; the first phase is the sample itself), the
//! largest of the four points of each interval between two samples, the largest of the
//! channels, the LFE too. Below a fifth of the rate the four points read the peak within
//! 0.05 dB; at the Nyquist frequency they may read it 0.69 dB low (the step of an eighth of a
//! turn between two points), which is also the reading of a meter that oversamples 4 times
//! (BS.1770, annex 2).
//!
//! # The gain (Signalsmith, "Designing a straightforward limiter", 2022)
//!
//! For each interval, the gain it needs: `r = min(1, ceiling / peak)`. Then, in the attenuation
//! `a = 1 − r`:
//!
//! 1. the largest `a` of the last [`HOLD`] intervals (a moving maximum);
//! 2. the release: what is held falls back to it by `α = e^(−1/(τ·rate))` a sample, τ the
//!    Release, and rises to it at once;
//! 3. the mean of the last [`ATTACK`] values, all alike (a box), or weighted by a raised
//!    cosine with Soft on (a gentler attack, the reference's §10.6): the gain is `1 − mean`.
//!
//! A mean of values each at least the attenuation that two intervals need is at least that
//! attenuation, so the gain at the output of a sample is no more than the gain that the intervals
//! on either side of it need, when the sound is delayed by [`ATTACK`] + [`TRUE_PEAK_DELAY`] −
//! 1 = 64 samples, the lookahead: no sample of the output goes over the ceiling, and no point
//! read between two samples. The gain moves in a straight line (or the raised cosine) over the
//! 59 samples before a peak, and so has no corner.
//!
//! Input Level is applied before the limiter; a sound that needs no limiting passes with the
//! gain 1 exactly, to the bit, delayed by the lookahead.
//!
//! # Sidechain
//!
//! With its sidechain on and a sidechain in the block, the limiter hears the true peak of the
//! sidechain, as it comes, not the input (the sidechain is not raised by Input Level): another
//! track ducks this one under the ceiling (p. 3965). Listen lets out the sidechain, delayed as
//! the sound is, in place of the sound (each channel its own when they are as many, else their
//! mean in each); it is for the monitor and is never automated. A block without its sidechain is
//! heard as one with none. Changing over is a crossfade of 10 ms of the attenuations heard.
//!
//! # Changes while the sound plays
//!
//! Input Level and Ceiling ramp over 10 ms in their amplitude, Release in its logarithm
//! ([`Smoothed`]); lanes of automation are taken as they are. The switches (the insert, Soft, the
//! sidechain) crossfade over 10 ms. Off, the insert lets out its input delayed by 64 samples,
//! and runs on underneath; the latency is the same whatever is set (C-X12).
//!
//! # Input that is not a number
//!
//! A sample of ±∞ or NaN is taken as 0: a limiter cannot bring an infinity under its ceiling, and
//! a NaN would stay in the release for ever.

use super::common::{finite, gain_of, lane, mix, params, ParamInfo};
use crate::constants::{
    samples, FX_SETTLE_TIME_CONSTANTS, LIMITER_KAISER_BETA, LIMITER_LOOKAHEAD, SMOOTHING,
};
use crate::node::{Block, Node, ParamId, Spec, Unsupported};
use crate::rt::{flush, seqlock, Domain, SeqReader, SeqWriter, Smoothed};
use crate::Layout;

params! {
    /// A parameter of [`Limiter`], in the units of the model's `LimiterFx`.
    pub enum Param {
        /// The insert on (1) or off (0).
        On => ParamInfo::switch("on", true),
        /// The gain before the limiter, dB, −18 to 18 (p. 3965).
        InputDb => ParamInfo::number("input_db", -18.0, 18.0, 0.0),
        /// A gentler attack.
        Soft => ParamInfo::switch("soft", false),
        /// The highest true peak let out, dB, −24 to 0 (C-X5).
        CeilingDb => ParamInfo::number("ceiling_db", -24.0, 0.0, -1.0),
        /// The release, ms, 0.01 to 1000 (p. 3965).
        ReleaseMs => ParamInfo::number("release_ms", 0.01, 1000.0, 100.0),
        /// The sidechain drives it.
        SidechainOn => ParamInfo::switch("sidechain.on", false),
        /// Listen to the sidechain: for the monitor, never automated.
        SidechainListen => ParamInfo::monitor("sidechain.listen"),
    }
}

/// The settings of a Limiter: the model's `LimiterFx` (its sidechain's source aside, which the
/// graph routes), and the switch of its insert.
#[derive(Clone, Copy, Debug, PartialEq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "the switches of the model's LimiterFx"
)]
pub struct Settings {
    /// The insert is on.
    pub on: bool,
    /// The gain before it, dB.
    pub input_db: f32,
    /// A gentler attack.
    pub soft: bool,
    /// The highest true peak let out, dB.
    pub ceiling_db: f32,
    /// The release, ms.
    pub release_ms: f32,
    /// The sidechain drives it.
    pub sidechain_on: bool,
    /// Listen to the sidechain.
    pub sidechain_listen: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            on: true,
            input_db: 0.0,
            soft: false,
            ceiling_db: -1.0,
            release_ms: 100.0,
            sidechain_on: false,
            sidechain_listen: false,
        }
    }
}

impl Settings {
    /// The value of `param`, as [`Node::set`] takes it.
    #[must_use]
    pub fn value(&self, param: Param) -> f32 {
        let switch = |on: bool| f32::from(u8::from(on));
        match param {
            Param::On => switch(self.on),
            Param::InputDb => self.input_db,
            Param::Soft => switch(self.soft),
            Param::CeilingDb => self.ceiling_db,
            Param::ReleaseMs => self.release_ms,
            Param::SidechainOn => switch(self.sidechain_on),
            Param::SidechainListen => switch(self.sidechain_listen),
        }
    }

    /// Every parameter and its value: a Limiter set so has these settings.
    #[must_use]
    pub fn params(&self) -> [(ParamId, f32); 7] {
        [
            Param::On,
            Param::InputDb,
            Param::Soft,
            Param::CeilingDb,
            Param::ReleaseMs,
            Param::SidechainOn,
            Param::SidechainListen,
        ]
        .map(|param| (param.id(), self.value(param)))
    }
}

/// The taps of each phase of the interpolator of the true peak.
pub const TRUE_PEAK_TAPS: usize = 12;

/// The points read in each interval between two samples: the oversampling.
pub const TRUE_PEAK_POINTS: usize = 4;

/// The samples by which the interval read lags the sample pushed: the interval from `n − 6` to
/// `n − 5` is read when `n` comes in.
pub const TRUE_PEAK_DELAY: usize = TRUE_PEAK_TAPS / 2;

/// The values of the attenuation over which the mean is taken: the attack, in samples.
pub const ATTACK: usize = LIMITER_LOOKAHEAD - TRUE_PEAK_DELAY + 1;

/// The intervals over which the largest attenuation is held: one more than [`ATTACK`], so that
/// the gain at a sample is held down for the intervals on both sides of it.
pub const HOLD: usize = ATTACK + 1;

/// The taps of the phases of the interpolator that read between the samples (the first phase
/// being the sample itself): `taps[k − 1][i]` weighs `x[n − 11 + i]` for the point `k/4` of the
/// way from `x[n − 6]` to `x[n − 5]`.
type Phases = [[f64; TRUE_PEAK_TAPS]; TRUE_PEAK_POINTS - 1];

/// The modified Bessel function of the first kind and order 0, by its series.
fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let mut k = 1.0;
    while term > 1e-17 * sum {
        term *= (x / (2.0 * k)).powi(2);
        sum += term;
        k += 1.0;
    }
    sum
}

/// The phases of the interpolator: sin(πu)/(πu) at the distance `u` of each tap from the point,
/// windowed by Kaiser over ±6, each phase scaled to sum to 1.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    reason = "the indices of a dozen taps and four points"
)]
fn phases() -> Phases {
    let half = TRUE_PEAK_DELAY as f64;
    let beta = LIMITER_KAISER_BETA;
    std::array::from_fn(|phase| {
        let point = (phase + 1) as f64 / TRUE_PEAK_POINTS as f64;
        let mut taps: [f64; TRUE_PEAK_TAPS] = std::array::from_fn(|i| {
            let u = (half - 1.0) + point - i as f64;
            let sinc = (std::f64::consts::PI * u).sin() / (std::f64::consts::PI * u);
            let r = u / half;
            sinc * bessel_i0(beta * (1.0 - r * r).max(0.0).sqrt()) / bessel_i0(beta)
        });
        let sum: f64 = taps.iter().sum();
        for tap in &mut taps {
            *tap /= sum;
        }
        taps
    })
}

/// The true peak of one channel, sample by sample: the largest size of the four points of the
/// interval from `x[n − 6]` to `x[n − 5]` when `x[n]` is pushed (BS.1770, annex 2, with an
/// interpolator of our own; see the module).
#[derive(Clone, Copy, Debug)]
pub struct TruePeak {
    phases: Phases,
    /// The last 12 samples, the oldest first.
    history: [f64; TRUE_PEAK_TAPS],
}

impl Default for TruePeak {
    fn default() -> Self {
        TruePeak::new()
    }
}

impl TruePeak {
    /// A detector that has heard silence.
    #[must_use]
    pub fn new() -> Self {
        TruePeak {
            phases: phases(),
            history: [0.0; TRUE_PEAK_TAPS],
        }
    }

    /// Back to silence.
    pub fn reset(&mut self) {
        self.history = [0.0; TRUE_PEAK_TAPS];
    }

    /// Pushes `x`: the largest size of the four points from `x[n − 6]` to `x[n − 5]`.
    #[inline]
    pub fn push(&mut self, x: f64) -> f64 {
        self.history.copy_within(1.., 0);
        self.history[TRUE_PEAK_TAPS - 1] = x;
        let mut peak = self.history[TRUE_PEAK_TAPS - 1 - TRUE_PEAK_DELAY].abs();
        for taps in &self.phases {
            let point: f64 = taps.iter().zip(&self.history).map(|(t, x)| t * x).sum();
            peak = peak.max(point.abs());
        }
        peak
    }

    /// The true peak of `signal`, silence before and after it, as a gain of amplitude.
    #[must_use]
    #[allow(clippy::cast_possible_truncation, reason = "a level")]
    pub fn of(signal: &[f32]) -> f32 {
        let mut detector = TruePeak::new();
        let mut peak = 0.0_f64;
        for &x in signal {
            peak = peak.max(detector.push(f64::from(x)));
        }
        for _ in 0..TRUE_PEAK_DELAY {
            peak = peak.max(detector.push(0.0));
        }
        peak as f32
    }
}

/// The samples after which a Limiter of `settings` at `rate`, started late, sounds within −90
/// dBFS of one that played from earlier: its ramps (10 ms); the lookahead, the interpolator, the
/// hold and the mean, which forget what came before them; and the release, by
/// [`FX_SETTLE_TIME_CONSTANTS`] of its time constant: what is held differs between the two
/// by less each sample by `α`, and 14 time constants bring a difference of gain of 1 on a sound
/// as loud as +30 dBFS under the floor.
#[must_use]
pub fn settle_for(settings: &Settings, rate: u32) -> u32 {
    let release = Param::ReleaseMs
        .info()
        .clamp(settings.release_ms)
        .unwrap_or(Param::ReleaseMs.info().default);
    settle_of(f64::from(release), rate)
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "a count of samples, clamped to the range of u32"
)]
fn settle_of(release_ms: f64, rate: u32) -> u32 {
    if rate == 0 {
        return 0;
    }
    let ramp = samples(SMOOTHING, rate);
    let fixed = (ramp + LIMITER_LOOKAHEAD + TRUE_PEAK_TAPS + HOLD + ATTACK) as f64;
    let release = (FX_SETTLE_TIME_CONSTANTS * release_ms / 1000.0 * f64::from(rate)).ceil();
    (fixed + release).min(f64::from(u32::MAX)) as u32
}

/// The factor by which what is held falls back a sample, for a release of `ms` at `rate`.
fn release_factor(ms: f32, rate: u32) -> f64 {
    (-1000.0 / (f64::from(ms) * f64::from(rate))).exp()
}

/// The weights of the mean with Soft on: a raised cosine over [`ATTACK`] values, summing to 1.
#[allow(clippy::cast_precision_loss, reason = "a window of 59")]
fn raised_cosine() -> [f64; ATTACK] {
    let mut weights: [f64; ATTACK] = std::array::from_fn(|k| {
        let x = std::f64::consts::PI * (k + 1) as f64 / (ATTACK + 1) as f64;
        x.sin().powi(2)
    });
    let sum: f64 = weights.iter().sum();
    for weight in &mut weights {
        *weight /= sum;
    }
    weights
}

/// What the meters of the window read after each block: the input's peak (Input Level applied),
/// the deepest reduction and the output's peak, in dB (a sample peak each; −200 for silence).
pub type Meter = (f32, f32, f32);

/// The floor of the meters, in dB.
const METER_FLOOR_DB: f32 = -200.0;

/// Limiter, a [`Node`] of a latency of 64 samples.
#[derive(Debug)]
#[allow(clippy::struct_excessive_bools, reason = "the switches as set")]
pub struct Limiter {
    rate: u32,
    channels: usize,
    sidechain: usize,
    on: bool,
    soft: bool,
    sidechain_on: bool,
    listen: bool,
    wet: Smoothed,
    soft_weight: Smoothed,
    side_weight: Smoothed,
    listen_weight: Smoothed,
    input: Smoothed,
    ceiling: Smoothed,
    release: Smoothed,
    /// The release and its factor last worked out.
    factor: (f32, f64),
    detectors: Vec<TruePeak>,
    side_detectors: Vec<TruePeak>,
    /// The input of each channel over the lookahead, as it came (sanitized).
    delay: Vec<[f64; LIMITER_LOOKAHEAD]>,
    /// The gain of Input Level at each of those samples.
    input_gains: [f64; LIMITER_LOOKAHEAD],
    /// The sidechain over the lookahead, for Listen.
    side_delay: Vec<[f64; LIMITER_LOOKAHEAD]>,
    /// Where the lines are: the slot of the sample that leaves.
    at: usize,
    hold: [f64; HOLD],
    hold_at: usize,
    /// What the release holds.
    held: f64,
    means: [f64; ATTACK],
    means_at: usize,
    cosine: [f64; ATTACK],
    meter: SeqWriter<Meter>,
    meter_reader: SeqReader<Meter>,
}

impl Limiter {
    /// A Limiter of `settings`, at once with no ramp.
    #[must_use]
    pub fn new(settings: &Settings) -> Self {
        let value = |param: Param| {
            param
                .info()
                .clamp(settings.value(param))
                .unwrap_or(param.info().default)
        };
        let switch = |on: bool| f32::from(u8::from(on));
        let (meter, meter_reader) = seqlock((METER_FLOOR_DB, 0.0, METER_FLOOR_DB));
        Limiter {
            rate: 0,
            channels: 0,
            sidechain: 0,
            on: settings.on,
            soft: settings.soft,
            sidechain_on: settings.sidechain_on,
            listen: settings.sidechain_listen,
            wet: Smoothed::new(switch(settings.on), Domain::Linear),
            soft_weight: Smoothed::new(switch(settings.soft), Domain::Linear),
            side_weight: Smoothed::new(0.0, Domain::Linear),
            listen_weight: Smoothed::new(0.0, Domain::Linear),
            input: Smoothed::new(amplitude(value(Param::InputDb)), Domain::Linear),
            ceiling: Smoothed::new(amplitude(value(Param::CeilingDb)), Domain::Linear),
            release: Smoothed::new(value(Param::ReleaseMs), Domain::Log),
            factor: (0.0, 0.0),
            detectors: Vec::new(), // rt-ok: empty, before the graph is built
            side_detectors: Vec::new(), // rt-ok: empty, before the graph is built
            delay: Vec::new(),     // rt-ok: empty, before the graph is built
            input_gains: [1.0; LIMITER_LOOKAHEAD],
            side_delay: Vec::new(), // rt-ok: empty, before the graph is built
            at: 0,
            hold: [0.0; HOLD],
            hold_at: 0,
            held: 0.0,
            means: [0.0; ATTACK],
            means_at: 0,
            cosine: raised_cosine(),
            meter,
            meter_reader,
        }
    }

    /// The meters of the window: input, reduction and output, in dB, after the last block.
    #[must_use]
    pub fn meter(&self) -> SeqReader<Meter> {
        self.meter_reader.clone()
    }

    /// The attenuation that a peak of `peak` needs under `ceiling`: 1 − min(1, ceiling / peak).
    #[inline]
    fn needed(peak: f64, ceiling: f64) -> f64 {
        if peak <= ceiling {
            0.0
        } else {
            1.0 - ceiling / peak
        }
    }

    /// The gain of the sample that leaves, from the attenuation `needed` by the interval that
    /// comes in, with the release `factor` and Soft at `soft`.
    #[inline]
    fn gain(&mut self, needed: f64, factor: f64, soft: f64) -> f64 {
        self.hold[self.hold_at] = needed;
        self.hold_at = (self.hold_at + 1) % HOLD;
        let held = self.hold.iter().copied().fold(0.0, f64::max);
        self.held = if held >= self.held {
            held
        } else {
            held + flush(factor * (self.held - held))
        };
        self.means[self.means_at] = self.held;
        self.means_at = (self.means_at + 1) % ATTACK;
        // The newest value weighs the last of the window; the raised cosine is symmetric, so
        // its order does not matter, but the box and it are summed in the same order.
        let mut box_sum = 0.0;
        let mut cosine_sum = 0.0;
        for (age, weight) in self.cosine.iter().enumerate() {
            let value = self.means[(self.means_at + age) % ATTACK];
            box_sum += value;
            cosine_sum += weight * value;
        }
        #[allow(clippy::cast_precision_loss, reason = "a window of 59")]
        let box_mean = box_sum / ATTACK as f64;
        1.0 - mix(box_mean, cosine_sum, soft)
    }
}

impl Limiter {
    /// The gain of Input Level and the ceiling (of amplitude) and the release (ms) at the last
    /// sample processed: where their ramps are.
    #[must_use]
    pub fn heard(&self) -> (f32, f32, f32) {
        (
            self.input.current(),
            self.ceiling.current(),
            self.release.current(),
        )
    }
}

impl Default for Limiter {
    fn default() -> Self {
        Limiter::new(&Settings::default())
    }
}

/// The gain of amplitude of `db`, in `f32`: what a ramp holds.
#[allow(clippy::cast_possible_truncation, reason = "a gain of ±24 dB")]
fn amplitude(db: f32) -> f32 {
    gain_of(f64::from(db)) as f32
}

/// The value at `n` of the lane of `param`, clamped, or `ramp`.
fn automated(values: Option<&[f32]>, n: usize, param: Param, ramp: f32) -> f32 {
    lane(values, n)
        .and_then(|value| param.info().clamp(value))
        .map_or(ramp, |value| match param {
            Param::InputDb | Param::CeilingDb => amplitude(value),
            _ => value,
        })
}

/// The decibels of a gain of amplitude, at least the floor of the meters.
#[allow(clippy::cast_possible_truncation, reason = "a level")]
fn meter_db(gain: f64) -> f32 {
    if gain > 0.0 {
        ((20.0 * gain.log10()) as f32).max(METER_FLOOR_DB)
    } else {
        METER_FLOOR_DB
    }
}

impl Node for Limiter {
    fn prepare(&mut self, spec: &Spec) -> Result<(), Unsupported> {
        if spec.layout != spec.output || spec.layout == Layout::Empty || spec.rate == 0 {
            return Err(Unsupported::from(spec));
        }
        self.rate = spec.rate;
        self.channels = spec.inputs();
        self.sidechain = spec.sidechain;
        for ramp in [
            &mut self.wet,
            &mut self.soft_weight,
            &mut self.side_weight,
            &mut self.listen_weight,
            &mut self.input,
            &mut self.ceiling,
            &mut self.release,
        ] {
            ramp.prepare(spec.rate);
        }
        self.detectors = vec![TruePeak::new(); self.channels]; // rt-ok: when the graph is built
        self.side_detectors = vec![TruePeak::new(); self.sidechain]; // rt-ok: when the graph is built
        self.delay = vec![[0.0; LIMITER_LOOKAHEAD]; self.channels]; // rt-ok: when the graph is built
        self.side_delay = vec![[0.0; LIMITER_LOOKAHEAD]; self.sidechain]; // rt-ok: when the graph is built
        self.reset();
        Ok(())
    }

    #[allow(clippy::cast_possible_truncation, reason = "the samples are f32")]
    #[allow(
        clippy::too_many_lines,
        reason = "one pass over the sample, step by step"
    )]
    fn process(&mut self, ctx: &Block, input: &[&[f32]], output: &mut [&mut [f32]]) {
        let automation = &ctx.automation;
        let [on_lane, input_lane, soft_lane, ceiling_lane, release_lane, side_lane] = [
            Param::On,
            Param::InputDb,
            Param::Soft,
            Param::CeilingDb,
            Param::ReleaseMs,
            Param::SidechainOn,
        ]
        .map(|param| automation.get(param.id()));
        let present = self.sidechain > 0 && ctx.sidechain.len() >= self.sidechain;
        if !present {
            self.side_weight.jump(0.0);
            self.listen_weight.jump(0.0);
        }
        let switch = |on: bool| f32::from(u8::from(on));
        let mut input_peak = 0.0_f64;
        let mut lowest_gain = 1.0_f64;
        let mut output_peak = 0.0_f64;
        for n in 0..ctx.frames {
            let on = lane(on_lane, n).map_or(self.on, |on| on >= 0.5);
            self.wet.set(switch(on));
            let wet = f64::from(self.wet.next_value());
            let soft = lane(soft_lane, n).map_or(self.soft, |on| on >= 0.5);
            self.soft_weight.set(switch(soft));
            let soft = f64::from(self.soft_weight.next_value());
            let keyed = present && lane(side_lane, n).map_or(self.sidechain_on, |on| on >= 0.5);
            if present {
                self.side_weight.set(switch(keyed));
                self.listen_weight.set(switch(keyed && self.listen));
            }
            let side = f64::from(self.side_weight.next_value());
            let listen = f64::from(self.listen_weight.next_value());
            let gain_in = f64::from(automated(
                input_lane,
                n,
                Param::InputDb,
                self.input.next_value(),
            ));
            let ceiling = f64::from(automated(
                ceiling_lane,
                n,
                Param::CeilingDb,
                self.ceiling.next_value(),
            ));
            let release = automated(release_lane, n, Param::ReleaseMs, self.release.next_value());
            if release.to_bits() != self.factor.0.to_bits() {
                self.factor = (release, release_factor(release, self.rate));
            }

            // The interval that comes in, heard from the input and from the sidechain.
            let at = self.at;
            let mut peak = 0.0_f64;
            for (channel, detector) in self.detectors.iter_mut().enumerate() {
                let x = finite(input[channel][n]);
                peak = peak.max(detector.push(x * gain_in));
                input_peak = input_peak.max((x * gain_in).abs());
            }
            let mut side_peak = 0.0_f64;
            for (channel, detector) in self.side_detectors.iter_mut().enumerate() {
                let x = if present {
                    finite(ctx.sidechain[channel][n])
                } else {
                    0.0
                };
                side_peak = side_peak.max(detector.push(x));
            }
            let needed = mix(
                Limiter::needed(peak, ceiling),
                Limiter::needed(side_peak, ceiling),
                side,
            );
            let gain = self.gain(needed, self.factor.1, soft);
            lowest_gain = lowest_gain.min(gain);

            // The sample that leaves, 64 after it came in.
            let leaving_gain = self.input_gains[at];
            self.input_gains[at] = gain_in;
            let mut side_mean = 0.0;
            if listen > 0.0 {
                for line in &self.side_delay {
                    side_mean += line[at];
                }
                #[allow(clippy::cast_precision_loss, reason = "24 channels at most")]
                let count = self.side_delay.len().max(1) as f64;
                side_mean /= count;
            }
            let own = self.side_delay.len() == self.channels;
            for (channel, line) in self.delay.iter_mut().enumerate() {
                let dry = line[at];
                line[at] = finite(input[channel][n]);
                let limited = dry * leaving_gain * gain;
                let heard = if listen > 0.0 {
                    let side = if own {
                        self.side_delay[channel][at]
                    } else {
                        side_mean
                    };
                    mix(limited, side, listen)
                } else {
                    limited
                };
                let y = mix(dry, heard, wet);
                output_peak = output_peak.max(y.abs());
                output[channel][n] = y as f32;
            }
            for (channel, line) in self.side_delay.iter_mut().enumerate() {
                line[at] = if present {
                    finite(ctx.sidechain[channel][n])
                } else {
                    0.0
                };
            }
            self.at = (at + 1) % LIMITER_LOOKAHEAD;
        }
        if ctx.frames > 0 {
            self.meter.write((
                meter_db(input_peak),
                meter_db(lowest_gain),
                meter_db(output_peak),
            ));
        }
    }

    fn latency(&self) -> u32 {
        u32::try_from(LIMITER_LOOKAHEAD).unwrap_or(u32::MAX)
    }

    /// [`settle_for`] the release set, or the one it ramps from where that is longer.
    fn settle(&self) -> u32 {
        let release = self.release.target().max(self.release.current());
        settle_of(f64::from(release), self.rate)
    }

    fn reset(&mut self) {
        let switch = |on: bool| f32::from(u8::from(on));
        self.wet.jump(switch(self.on));
        self.soft_weight.jump(switch(self.soft));
        let keyed = self.sidechain > 0 && self.sidechain_on;
        self.side_weight.jump(switch(keyed));
        self.listen_weight.jump(switch(keyed && self.listen));
        self.input.finish();
        self.ceiling.finish();
        self.release.finish();
        for detector in self.detectors.iter_mut().chain(&mut self.side_detectors) {
            detector.reset();
        }
        for line in self.delay.iter_mut().chain(&mut self.side_delay) {
            *line = [0.0; LIMITER_LOOKAHEAD];
        }
        self.input_gains = [1.0; LIMITER_LOOKAHEAD];
        self.at = 0;
        self.hold = [0.0; HOLD];
        self.hold_at = 0;
        self.held = 0.0;
        self.means = [0.0; ATTACK];
        self.means_at = 0;
    }

    fn set(&mut self, param: ParamId, value: f32) {
        let Some(param) = Param::from_id(param) else {
            return;
        };
        let Some(value) = param.info().clamp(value) else {
            return;
        };
        let on = value >= 0.5;
        match param {
            Param::On => self.on = on,
            Param::InputDb => self.input.set(amplitude(value)),
            Param::Soft => self.soft = on,
            Param::CeilingDb => self.ceiling.set(amplitude(value)),
            Param::ReleaseMs => self.release.set(value),
            Param::SidechainOn => self.sidechain_on = on,
            Param::SidechainListen => self.listen = on,
        }
    }
}
