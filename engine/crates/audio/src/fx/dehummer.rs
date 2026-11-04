//! De-Hummer (p. 3949): takes out the hum of the mains, a fundamental of 50 or 60 Hz (40 to 70 on
//! the knob) and its harmonics up to the fourth (C-X8).
//!
//! # The notches
//!
//! Four cuts one after the other, at `k · f` for `k` = 1 to 4 ([`DEHUMMER_NOTCHES`]; at most 0.45
//! of the rate), each the sound less a part `d_k` of its band about the centre:
//!
//! ```text
//! y = x − d_k · BP_k(x),   d_k = 1 − 10^(−depth_k/20)
//! ```
//!
//! with `BP_k` the band-pass of the RBJ cookbook of constant 0 dB peak and Q 30 ([`DEHUMMER_Q`]),
//! whose gain is 1 at its centre, of no phase: the cut there is `depth_k` exactly, and `d = 1`
//! would be the cookbook's notch. One biquad each, `b = a − d · b_BP`, whose poles are the
//! band-pass's whatever the depth: as narrow at any depth (its −3 dB points f0/Q apart once deep),
//! where a peaking EQ of the same Q cut by 40 dB widens its skirt to 1/(AQ) and takes 0.2 dB off
//! 50 Hz with its notch at 100 Hz. Amount sets how deep, to at most [`DEHUMMER_MAX_DEPTH_DB`];
//! Slope shares it out among the harmonics:
//!
//! ```text
//! depth_k = Amount/100 · 40 dB · w_k,   t = 2·Slope − 1,   p_k = (k − 1)/3
//! w_k = 1 − max(0, −t) · p_k − max(0, t) · (1 − p_k)
//! ```
//!
//! so that Slope 0 cuts the fundamental the most (1, 2/3, 1/3, 0), 0.5 cuts them all alike, and 1
//! cuts the highest the most (0, 1/3, 2/3, 1), as the manual has it. A cut of 0 dB is the identity
//! to the bit (`d = 0`: its coefficients b are its a), so that Amount 0 passes the sound to the
//! bit. Listen
//! to Hum Only lets out the input less the output: the hum taken away.
//!
//! Each notch is narrow (Q 30 is 1.7 Hz at 50 Hz): a tone of 1 kHz is not touched (within
//! 0.001 dB), nor is speech beside the hum.
//!
//! # Changes while the sound plays
//!
//! Frequency ramps over 10 ms in its logarithm, Amount and Slope in their values ([`Smoothed`]),
//! and their lanes ramp the same (a filter whose coefficients jump at every sample is not stable;
//! as the EQ); while they move the notches are designed again at each sample. The insert and
//! Listen crossfade over 10 ms; Listen is for the monitor and never automated. A sample of input
//! that is not a number is taken as 0 by the notches.

use super::biquad::Coefficients;
use super::common::{finite, gain_of, lane, mix, params, ParamInfo, SETTLE_RESIDUE};
use super::deesser::band_pass;
use crate::constants::{
    samples, DEHUMMER_MAX_DEPTH_DB, DEHUMMER_NOTCHES, DEHUMMER_Q, EQ_MAX_FREQUENCY_FRACTION,
    SMOOTHING,
};
use crate::node::{Block, Node, ParamId, Spec, Unsupported};
use crate::rt::{Domain, Smoothed};
use crate::Layout;

params! {
    /// A parameter of [`DeHummer`], in the units of the model's `DeHummerFx`.
    pub enum Param {
        /// The insert on (1) or off (0).
        On => ParamInfo::switch("on", true),
        /// The fundamental, Hz, 40 to 70 (C-X8).
        FrequencyHz => ParamInfo::number("frequency_hz", 40.0, 70.0, 50.0),
        /// Percent, 0 to 100: up to 40 dB deep.
        Amount => ParamInfo::number("amount", 0.0, 100.0, 50.0),
        /// From 0, toward the fundamental, to 1, toward the harmonics.
        Slope => ParamInfo::number("slope", 0.0, 1.0, 0.5),
        /// Listen to Hum Only: for the monitor, never automated.
        Listen => ParamInfo::monitor("listen"),
    }
}

/// The settings of a De-Hummer: the model's `DeHummerFx`, and the switch of its insert.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// The insert is on.
    pub on: bool,
    /// The fundamental, Hz.
    pub frequency_hz: f32,
    /// Percent.
    pub amount: f32,
    /// 0 to 1.
    pub slope: f32,
    /// Listen to Hum Only.
    pub listen: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            on: true,
            frequency_hz: 50.0,
            amount: 50.0,
            slope: 0.5,
            listen: false,
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
            Param::FrequencyHz => self.frequency_hz,
            Param::Amount => self.amount,
            Param::Slope => self.slope,
            Param::Listen => switch(self.listen),
        }
    }

    /// Every parameter and its value: a De-Hummer set so has these settings.
    #[must_use]
    pub fn params(&self) -> [(ParamId, f32); 5] {
        [
            Param::On,
            Param::FrequencyHz,
            Param::Amount,
            Param::Slope,
            Param::Listen,
        ]
        .map(|param| (param.id(), self.value(param)))
    }
}

/// The ramps of 10 ms in which two parameters following the same lane from different values come
/// within 1e-6 of each other: the widest difference, in the domain of its ramp, is Amount's, 100,
/// and ⌈ln(100/1e-6)⌉ = 19 (the frequency's is ln(70/40) = 0.56, the slope's 1; as the EQ's
/// `LANE_SETTLE`).
pub const LANE_SETTLE: u64 = 19;

/// The depth of each notch, in dB, at `amount` percent and `slope` (see the module).
#[must_use]
#[allow(clippy::cast_precision_loss, reason = "four notches")]
pub fn depths(amount: f64, slope: f64) -> [f64; DEHUMMER_NOTCHES] {
    let t = 2.0 * slope - 1.0;
    let last = (DEHUMMER_NOTCHES - 1) as f64;
    std::array::from_fn(|k| {
        let p = k as f64 / last;
        let weight = 1.0 - (-t).max(0.0) * p - t.max(0.0) * (1.0 - p);
        amount / 100.0 * f64::from(DEHUMMER_MAX_DEPTH_DB) * weight
    })
}

/// The cut of `depth` dB at `centre` Hz: the sound less `1 − 10^(−depth/20)` of its band-pass of
/// Q 30.
#[must_use]
pub fn cut(rate: f64, centre: f64, depth: f64) -> Coefficients {
    let band = band_pass(rate, centre, f64::from(DEHUMMER_Q));
    let part = 1.0 - gain_of(-depth);
    Coefficients {
        b0: 1.0 - part * band.b0,
        b1: band.a1 - part * band.b1,
        b2: band.a2 - part * band.b2,
        ..band
    }
}

/// The notches at `rate` for a fundamental of `freq` Hz, `amount` and `slope`.
#[must_use]
#[allow(clippy::cast_precision_loss, reason = "four notches")]
pub fn notches(rate: u32, freq: f64, amount: f64, slope: f64) -> [Coefficients; DEHUMMER_NOTCHES] {
    let rate = f64::from(rate);
    let depths = depths(amount, slope);
    std::array::from_fn(|k| {
        let centre = (freq * (k + 1) as f64).min(EQ_MAX_FREQUENCY_FRACTION * rate);
        cut(rate, centre, depths[k])
    })
}

/// The samples after which a De-Hummer of `settings` at `rate`, started late, sounds within −90
/// dBFS of one that played from earlier: its ramps (10 ms), and [`LANE_SETTLE`] of them for
/// parameters following a lane; and each notch, one after the other, to 1e-6 by its poles, which
/// are the same at any depth, so that a lane of Amount costs nothing more.
#[must_use]
pub fn settle_for(settings: &Settings, rate: u32) -> u32 {
    let freq = Param::FrequencyHz
        .info()
        .clamp(settings.frequency_hz)
        .unwrap_or(Param::FrequencyHz.info().default);
    settle_of(freq, rate)
}

fn settle_of(freq: f32, rate: u32) -> u32 {
    if rate == 0 {
        return 0;
    }
    let ramp = u64::try_from(samples(SMOOTHING, rate)).unwrap_or(u64::MAX);
    let notches: u64 = notches(rate, f64::from(freq), 0.0, 0.5)
        .iter()
        .map(|notch| u64::from(notch.decay(SETTLE_RESIDUE)))
        .sum();
    u32::try_from(ramp * (1 + LANE_SETTLE) + notches).unwrap_or(u32::MAX)
}

/// The past of a channel: its last two inputs, and the last two outputs of each notch.
#[derive(Clone, Copy, Debug, Default)]
struct Past {
    input: [f64; 2],
    outputs: [[f64; 2]; DEHUMMER_NOTCHES],
}

/// De-Hummer, a [`Node`] of no latency.
#[derive(Clone, Debug)]
pub struct DeHummer {
    rate: u32,
    on: bool,
    listen: bool,
    wet: Smoothed,
    listen_weight: Smoothed,
    /// The frequency, the amount and the slope as set (their lanes ramp to their values).
    set: [f32; 3],
    frequency: Smoothed,
    amount: Smoothed,
    slope: Smoothed,
    notches: [Coefficients; DEHUMMER_NOTCHES],
    /// The frequency, amount and slope (their bits) the notches are designed for.
    designed: Option<[u32; 3]>,
    past: Vec<Past>,
}

impl DeHummer {
    /// A De-Hummer of `settings`, at once with no ramp.
    #[must_use]
    pub fn new(settings: &Settings) -> Self {
        let value = |param: Param| {
            param
                .info()
                .clamp(settings.value(param))
                .unwrap_or(param.info().default)
        };
        let switch = |on: bool| f32::from(u8::from(on));
        let set = [
            value(Param::FrequencyHz),
            value(Param::Amount),
            value(Param::Slope),
        ];
        DeHummer {
            rate: 0,
            on: settings.on,
            listen: settings.listen,
            wet: Smoothed::new(switch(settings.on), Domain::Linear),
            listen_weight: Smoothed::new(switch(settings.listen), Domain::Linear),
            set,
            frequency: Smoothed::new(set[0], Domain::Log),
            amount: Smoothed::new(set[1], Domain::Linear),
            slope: Smoothed::new(set[2], Domain::Linear),
            notches: [Coefficients::IDENTITY; DEHUMMER_NOTCHES],
            designed: None,
            past: Vec::new(), // rt-ok: empty, before the graph is built
        }
    }
}

impl DeHummer {
    /// The frequency (Hz), the amount (%) and the slope at the last sample processed: where
    /// their ramps are.
    #[must_use]
    pub fn heard(&self) -> (f32, f32, f32) {
        (
            self.frequency.current(),
            self.amount.current(),
            self.slope.current(),
        )
    }
}

impl Default for DeHummer {
    fn default() -> Self {
        DeHummer::new(&Settings::default())
    }
}

impl Node for DeHummer {
    fn prepare(&mut self, spec: &Spec) -> Result<(), Unsupported> {
        if spec.layout != spec.output
            || spec.layout == Layout::Empty
            || spec.sidechain > 0
            || spec.rate == 0
        {
            return Err(Unsupported::from(spec));
        }
        self.rate = spec.rate;
        for ramp in [
            &mut self.wet,
            &mut self.listen_weight,
            &mut self.frequency,
            &mut self.amount,
            &mut self.slope,
        ] {
            ramp.prepare(spec.rate);
        }
        self.past = vec![Past::default(); spec.inputs()]; // rt-ok: when the graph is built
        self.reset();
        Ok(())
    }

    #[allow(clippy::cast_possible_truncation, reason = "the samples are f32")]
    fn process(&mut self, ctx: &Block, input: &[&[f32]], output: &mut [&mut [f32]]) {
        let automation = &ctx.automation;
        let [on_lane, frequency_lane, amount_lane, slope_lane] =
            [Param::On, Param::FrequencyHz, Param::Amount, Param::Slope]
                .map(|param| automation.get(param.id()));
        let switch = |on: bool| f32::from(u8::from(on));
        for n in 0..ctx.frames {
            let on = lane(on_lane, n).map_or(self.on, |on| on >= 0.5);
            self.wet.set(switch(on));
            let wet = f64::from(self.wet.next_value());
            self.listen_weight.set(switch(self.listen));
            let listen = f64::from(self.listen_weight.next_value());
            // Each value, set or of a lane, starts a ramp of 10 ms from where the parameter is.
            let target = |values, param: Param, set: f32| {
                lane(values, n)
                    .and_then(|value| param.info().clamp(value))
                    .unwrap_or(set)
            };
            self.frequency
                .set(target(frequency_lane, Param::FrequencyHz, self.set[0]));
            self.amount
                .set(target(amount_lane, Param::Amount, self.set[1]));
            self.slope
                .set(target(slope_lane, Param::Slope, self.set[2]));
            let now = [
                self.frequency.next_value(),
                self.amount.next_value(),
                self.slope.next_value(),
            ];
            let bits = now.map(f32::to_bits);
            if self.designed != Some(bits) {
                self.notches = notches(
                    self.rate,
                    f64::from(now[0]),
                    f64::from(now[1]),
                    f64::from(now[2]),
                );
                self.designed = Some(bits);
            }
            for ((output, input), past) in output.iter_mut().zip(input).zip(&mut self.past) {
                let x = finite(input[n]);
                let mut value = x;
                let mut before = past.input;
                for (notch, outputs) in self.notches.iter().zip(&mut past.outputs) {
                    let last = *outputs;
                    value = notch.tick(value, before, outputs);
                    before = last;
                }
                past.input = [x, past.input[0]];
                let raw = f64::from(input[n]);
                let heard = mix(value, x - value, listen);
                output[n] = mix(raw, heard, wet) as f32;
            }
        }
    }

    fn latency(&self) -> u32 {
        0
    }

    /// [`settle_for`] the frequency set, or the one it ramps from where that is lower.
    fn settle(&self) -> u32 {
        settle_of(
            self.frequency.target().min(self.frequency.current()),
            self.rate,
        )
    }

    fn reset(&mut self) {
        let switch = |on: bool| f32::from(u8::from(on));
        self.wet.jump(switch(self.on));
        self.listen_weight.jump(switch(self.listen));
        self.frequency.jump(self.set[0]);
        self.amount.jump(self.set[1]);
        self.slope.jump(self.set[2]);
        self.designed = None;
        self.past.fill(Past::default());
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
            Param::FrequencyHz => self.set[0] = value,
            Param::Amount => self.set[1] = value,
            Param::Slope => self.set[2] = value,
            Param::Listen => self.listen = value >= 0.5,
        }
    }
}
