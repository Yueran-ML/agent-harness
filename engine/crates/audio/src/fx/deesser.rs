//! De-Esser (p. 3948): takes the sibilance ("s", "ts") out of speech by lowering a band of high
//! frequencies while it stands out, and nothing else (C-X7).
//!
//! # The band
//!
//! The band about the Target Frequency (2 to 16 kHz, 6 kHz for a new one): Narrow Band, a
//! band-pass of the RBJ cookbook a third of an octave wide; Wide Band, an octave wide (the
//! band-pass of constant 0 dB peak, Q = 2^(N/2)/(2^N − 1) for N octaves,
//! [`DEESSER_BANDWIDTHS`]); All High Frequency, what is above the target ("all audio above the
//! source frequency"): the sound less its low-pass of two poles, Butterworth, at the target
//! ([`above`]), heard through a high-pass of two poles, Butterworth, at the target, whose 12 dB
//! an octave below it keep a voice from being heard as sibilance (`1 − LP` falls by only 6 dB an
//! octave: a tone of 1 kHz is 12.5 dB under it at 6 kHz). Narrow and Wide hear their own band.
//! The band is what the effect lowers: for a gain `g` of the band,
//!
//! ```text
//! y = x − (1 − g) · band(x)
//! ```
//!
//! so that `g = 1` gives the input to the bit; at the target of a band-pass (whose gain is 1
//! there, of no phase) the sound is `g · x`; and with All High it is `g · x + (1 − g) · LP(x)`,
//! the sound above the target lowered by `g` and below it kept. (A high-pass in place of `1 −
//! LP` would not do: its phase at the target, a quarter of a turn, would keep the sound there
//! from being lowered at all.) Listen to Ess Only lets out `(1 − g) · band(x)`, what is taken
//! away.
//!
//! # When it acts (we decide; the manual has no threshold)
//!
//! The effect follows the peaks of the band and of the whole sound, the largest of the channels,
//! the LFE left out, each by a follower of the Reaction's attack and release
//! ([`DEESSER_REACTION`]: 10/80, 2/40 and 0.5/20 ms; it rises toward a larger peak by its attack
//! and falls toward a smaller by its release, as one pole each). Sibilance is the band standing
//! out of the sound: with `ρ` the level of the band over the level of the whole in dB, the
//! reduction is
//!
//! ```text
//! reduction = Amount/100 · DEESSER_MAX_REDUCTION_DB · clamp((ρ − T) / (0 − T), 0, 1)
//! ```
//!
//! with `T` = [`DEESSER_RELATIVE_THRESHOLD_DB`]: nothing while the band is more than 12 dB under
//! the whole (a voice, a tone of 1 kHz under a band at 6 kHz, which an octave-wide band-pass
//! lowers by 18 dB), the most when it is all of it (a pure ess). It does not depend on how loud
//! the sound is, only on its balance, but for a band under [`DEESSER_FLOOR_DBFS`], which is left
//! alone (all of the reduction from 10 dB above it): the click of a sound that starts out of
//! silence is high, and far down. Every channel takes the same gain.
//!
//! # Changes while the sound plays
//!
//! The target ramps over 10 ms in its logarithm, and its lane ramps the same (a filter whose
//! frequency jumps at every sample is not stable; as the EQ), the filters designed again at each
//! sample while it moves. Amount ramps in its value and its lane is taken as it is. The filters
//! of the three ranges run all the time and the Range crossfades between them over 10 ms (what
//! is lowered and what is heard alike); the Reaction changes
//! the followers at once (their speed, not the sound). The insert and Listen crossfade over 10
//! ms; Listen is for the monitor and never automated. A sample of input that is not a number is
//! taken as 0 by the filters and the followers.

use super::biquad::Coefficients;
use super::common::{finite, lane, mix, params, ParamInfo, SETTLE_RESIDUE};
use crate::constants::{
    samples, DEESSER_BANDWIDTHS, DEESSER_FLOOR_DBFS, DEESSER_MAX_REDUCTION_DB, DEESSER_REACTION,
    DEESSER_RELATIVE_THRESHOLD_DB, EQ_MAX_FREQUENCY_FRACTION, FX_SETTLE_TIME_CONSTANTS, SMOOTHING,
};
use crate::node::{Block, Node, ParamId, Spec, Unsupported};
use crate::rt::{flush, seqlock, Domain, SeqReader, SeqWriter, Smoothed};
use crate::Layout;

params! {
    /// A parameter of [`DeEsser`], in the units of the model's `DeEsserFx`.
    pub enum Param {
        /// The insert on (1) or off (0).
        On => ParamInfo::switch("on", true),
        /// The target, Hz, 2000 to 16000 (C-X7).
        FrequencyHz => ParamInfo::number("frequency_hz", 2_000.0, 16_000.0, 6_000.0),
        /// The [`Range::code`].
        Range => ParamInfo::choice("range", 3, 1),
        /// Percent, 0 to 100: up to 20 dB of reduction.
        Amount => ParamInfo::number("amount", 0.0, 100.0, 50.0),
        /// The [`Reaction::code`].
        Reaction => ParamInfo::choice("reaction", 3, 1),
        /// Listen to Ess Only: for the monitor, never automated.
        Listen => ParamInfo::monitor("listen"),
    }
}

/// How wide the band is (the model's `EssRange`, in its order).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Range {
    /// A third of an octave about the target.
    Narrow,
    /// An octave about the target.
    Wide,
    /// All above the target.
    AllHigh,
}

impl Range {
    /// Every range, in the order of their codes.
    pub const ALL: [Range; 3] = [Range::Narrow, Range::Wide, Range::AllHigh];

    /// Its code for [`Node::set`]: its place in [`Range::ALL`].
    #[must_use]
    pub fn code(self) -> f32 {
        f32::from(self as u8)
    }
}

/// How quickly it acts (the model's `Reaction`, in its order).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Reaction {
    /// Attack 10 ms, release 80 ms.
    Slow,
    /// Attack 2 ms, release 40 ms.
    Medium,
    /// Attack 0.5 ms, release 20 ms.
    Fast,
}

impl Reaction {
    /// Every reaction, in the order of their codes.
    pub const ALL: [Reaction; 3] = [Reaction::Slow, Reaction::Medium, Reaction::Fast];

    /// Its code for [`Node::set`]: its place in [`Reaction::ALL`].
    #[must_use]
    pub fn code(self) -> f32 {
        f32::from(self as u8)
    }
}

/// The settings of a De-Esser: the model's `DeEsserFx`, and the switch of its insert.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// The insert is on.
    pub on: bool,
    /// The target, Hz.
    pub frequency_hz: f32,
    /// How wide the band is.
    pub range: Range,
    /// Percent.
    pub amount: f32,
    /// How quickly it acts.
    pub reaction: Reaction,
    /// Listen to Ess Only.
    pub listen: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            on: true,
            frequency_hz: 6_000.0,
            range: Range::Wide,
            amount: 50.0,
            reaction: Reaction::Medium,
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
            Param::Range => self.range.code(),
            Param::Amount => self.amount,
            Param::Reaction => self.reaction.code(),
            Param::Listen => switch(self.listen),
        }
    }

    /// Every parameter and its value: a De-Esser set so has these settings.
    #[must_use]
    pub fn params(&self) -> [(ParamId, f32); 6] {
        [
            Param::On,
            Param::FrequencyHz,
            Param::Range,
            Param::Amount,
            Param::Reaction,
            Param::Listen,
        ]
        .map(|param| (param.id(), self.value(param)))
    }
}

/// The ramps of 10 ms in which two targets following the same lane from different values come
/// within 1e-6 of each other in their logarithm: the widest difference is ln(16000/2000) = 2.08,
/// and ⌈ln(2.08/1e-6)⌉ = 15 (as the EQ's `LANE_SETTLE`).
pub const LANE_SETTLE: u64 = 15;

/// The Q of a band-pass `octaves` wide between its points of half power.
#[must_use]
pub fn band_q(octaves: f64) -> f64 {
    let n = octaves.exp2();
    n.sqrt() / (n - 1.0)
}

/// The band-pass of the RBJ cookbook of constant 0 dB peak at `freq` Hz of `q`: b0 = α, b1 = 0,
/// b2 = −α over a0 = 1 + α, a1 = −2 cos ω0, a2 = 1 − α.
#[must_use]
pub fn band_pass(rate: f64, freq: f64, q: f64) -> Coefficients {
    let w0 = std::f64::consts::TAU * freq / rate;
    let alpha = w0.sin() / (2.0 * q);
    let a0 = 1.0 + alpha;
    Coefficients {
        b0: alpha / a0,
        b1: 0.0,
        b2: -alpha / a0,
        a1: -2.0 * w0.cos() / a0,
        a2: (1.0 - alpha) / a0,
    }
}

/// What is above `freq` Hz: the sound less its low-pass of two poles, Butterworth (1 − LP, of
/// the same poles), so that the sound less this band is the low-pass itself.
#[must_use]
pub fn above(rate: f64, freq: f64) -> Coefficients {
    let low = Coefficients::low_pass(rate, freq, std::f64::consts::FRAC_1_SQRT_2);
    Coefficients {
        b0: 1.0 - low.b0,
        b1: low.a1 - low.b1,
        b2: low.a2 - low.b2,
        ..low
    }
}

/// The filters at `freq` Hz (at most 0.45 of the rate): the bands of the three ranges, and the
/// high-pass by which All High hears.
fn filters(rate: u32, freq: f32) -> [Coefficients; 4] {
    let rate = f64::from(rate);
    let freq = f64::from(freq).min(EQ_MAX_FREQUENCY_FRACTION * rate);
    [
        band_pass(rate, freq, band_q(DEESSER_BANDWIDTHS[0])),
        band_pass(rate, freq, band_q(DEESSER_BANDWIDTHS[1])),
        above(rate, freq),
        Coefficients::high_pass(rate, freq, std::f64::consts::FRAC_1_SQRT_2),
    ]
}

/// The reduction, in dB, of a band at `band_db` dBFS and `ratio_db` above the whole (0 at most),
/// at `amount` percent: none for a band under [`DEESSER_FLOOR_DBFS`], all of it from 10 dB
/// above, and between them in proportion, so that the clicks of sound starting out of silence,
/// which are high but far down, are left alone.
#[must_use]
pub fn reduction_db(band_db: f64, ratio_db: f64, amount: f64) -> f64 {
    let threshold = f64::from(DEESSER_RELATIVE_THRESHOLD_DB);
    let part = ((ratio_db - threshold) / -threshold).clamp(0.0, 1.0);
    let floor = ((band_db - f64::from(DEESSER_FLOOR_DBFS)) / 10.0).clamp(0.0, 1.0);
    amount / 100.0 * f64::from(DEESSER_MAX_REDUCTION_DB) * part * floor
}

/// The samples after which a De-Esser of `settings` at `rate`, started late, sounds within −90
/// dBFS of one that played from earlier: its ramps (10 ms), and [`LANE_SETTLE`] of them for the
/// target following a lane; the slowest of its filters to 1e-6 (they run side by side);
/// and the followers, by [`FX_SETTLE_TIME_CONSTANTS`] of the slowest release and attack of any
/// Reaction (a lane may choose it).
#[must_use]
pub fn settle_for(settings: &Settings, rate: u32) -> u32 {
    let freq = Param::FrequencyHz
        .info()
        .clamp(settings.frequency_hz)
        .unwrap_or(Param::FrequencyHz.info().default);
    settle_of(freq, rate)
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "clamped to the range of u32"
)]
fn settle_of(freq: f32, rate: u32) -> u32 {
    if rate == 0 {
        return 0;
    }
    let ramp = u64::try_from(samples(SMOOTHING, rate)).unwrap_or(u64::MAX);
    let filters = filters(rate, freq)
        .iter()
        .map(|filter| u64::from(filter.decay(SETTLE_RESIDUE)))
        .max()
        .unwrap_or(0);
    let slowest = DEESSER_REACTION
        .iter()
        .fold(0.0_f64, |slowest, &(attack, release)| {
            slowest.max(attack).max(release)
        });
    let followers = (FX_SETTLE_TIME_CONSTANTS * slowest * f64::from(rate)).ceil() as u64;
    let total = ramp * (1 + LANE_SETTLE) + filters + followers;
    u32::try_from(total).unwrap_or(u32::MAX)
}

/// What the meters of the window read after each block: the deepest reduction and the output's
/// peak, in dB (−200 for silence).
pub type Meter = (f32, f32);

/// The floor of the meters, in dB.
const METER_FLOOR_DB: f32 = -200.0;

/// The past of a channel: its last two inputs, the last two outputs of each filter, and the
/// band of the sample under way.
#[derive(Clone, Copy, Debug, Default)]
struct Past {
    input: [f64; 2],
    outputs: [[f64; 2]; 4],
    band: f64,
}

/// De-Esser, a [`Node`] of no latency.
#[derive(Debug)]
pub struct DeEsser {
    rate: u32,
    lfe: Option<usize>,
    on: bool,
    range: usize,
    reaction: usize,
    listen: bool,
    wet: Smoothed,
    listen_weight: Smoothed,
    ranges: [Smoothed; 3],
    /// The target as set (its lane ramps to its values).
    frequency_set: f32,
    frequency: Smoothed,
    amount: Smoothed,
    filters: [Coefficients; 4],
    /// The frequency (its bits) the filters are designed for.
    designed: Option<u32>,
    /// The factors of the followers, rising and falling.
    factors: [(f64, f64); 3],
    past: Vec<Past>,
    band_level: f64,
    level: f64,
    meter: SeqWriter<Meter>,
    meter_reader: SeqReader<Meter>,
}

impl DeEsser {
    /// A De-Esser of `settings`, at once with no ramp.
    #[must_use]
    pub fn new(settings: &Settings) -> Self {
        let value = |param: Param| {
            param
                .info()
                .clamp(settings.value(param))
                .unwrap_or(param.info().default)
        };
        let switch = |on: bool| f32::from(u8::from(on));
        let range = settings.range as usize;
        let (meter, meter_reader) = seqlock((0.0, METER_FLOOR_DB));
        DeEsser {
            rate: 0,
            lfe: None,
            on: settings.on,
            range,
            reaction: settings.reaction as usize,
            listen: settings.listen,
            wet: Smoothed::new(switch(settings.on), Domain::Linear),
            listen_weight: Smoothed::new(switch(settings.listen), Domain::Linear),
            ranges: std::array::from_fn(|k| Smoothed::new(switch(k == range), Domain::Linear)),
            frequency_set: value(Param::FrequencyHz),
            frequency: Smoothed::new(value(Param::FrequencyHz), Domain::Log),
            amount: Smoothed::new(value(Param::Amount), Domain::Linear),
            filters: [Coefficients::IDENTITY; 4],
            designed: None,
            factors: [(0.0, 0.0); 3],
            past: Vec::new(), // rt-ok: empty, before the graph is built
            band_level: 0.0,
            level: 0.0,
            meter,
            meter_reader,
        }
    }

    /// The meters of the window: the reduction and the output, in dB, after the last block.
    #[must_use]
    pub fn meter(&self) -> SeqReader<Meter> {
        self.meter_reader.clone()
    }

    /// The target (Hz) and the amount (%) at the last sample processed: where their ramps are.
    #[must_use]
    pub fn heard(&self) -> (f32, f32) {
        (self.frequency.current(), self.amount.current())
    }

    /// A follower of `level` toward `x` with `factors` (rising, falling).
    #[inline]
    fn follow(level: f64, x: f64, factors: (f64, f64)) -> f64 {
        let factor = if x > level { factors.0 } else { factors.1 };
        x + flush(factor * (level - x))
    }
}

impl Default for DeEsser {
    fn default() -> Self {
        DeEsser::new(&Settings::default())
    }
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

/// The choice of a lane's value at `n`, or `set`.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a code of 0 to 2"
)]
fn choice(values: Option<&[f32]>, n: usize, param: Param, set: usize) -> usize {
    lane(values, n)
        .and_then(|value| param.info().clamp(value))
        .map_or(set, |code| code as usize)
}

impl Node for DeEsser {
    fn prepare(&mut self, spec: &Spec) -> Result<(), Unsupported> {
        if spec.layout != spec.output
            || spec.layout == Layout::Empty
            || spec.sidechain > 0
            || spec.rate == 0
        {
            return Err(Unsupported::from(spec));
        }
        self.rate = spec.rate;
        self.lfe = spec.layout.lfe();
        self.wet.prepare(spec.rate);
        self.listen_weight.prepare(spec.rate);
        self.frequency.prepare(spec.rate);
        self.amount.prepare(spec.rate);
        for ramp in &mut self.ranges {
            ramp.prepare(spec.rate);
        }
        let rate = f64::from(spec.rate);
        self.factors = DEESSER_REACTION.map(|(attack, release)| {
            (
                (-1.0 / (attack * rate)).exp(),
                (-1.0 / (release * rate)).exp(),
            )
        });
        self.past = vec![Past::default(); spec.inputs()]; // rt-ok: when the graph is built
        self.reset();
        Ok(())
    }

    #[allow(clippy::cast_possible_truncation, reason = "the samples are f32")]
    fn process(&mut self, ctx: &Block, input: &[&[f32]], output: &mut [&mut [f32]]) {
        let automation = &ctx.automation;
        let [on_lane, frequency_lane, range_lane, amount_lane, reaction_lane] = [
            Param::On,
            Param::FrequencyHz,
            Param::Range,
            Param::Amount,
            Param::Reaction,
        ]
        .map(|param| automation.get(param.id()));
        let switch = |on: bool| f32::from(u8::from(on));
        let mut deepest = 0.0_f64;
        let mut output_peak = 0.0_f64;
        for n in 0..ctx.frames {
            let on = lane(on_lane, n).map_or(self.on, |on| on >= 0.5);
            self.wet.set(switch(on));
            let wet = f64::from(self.wet.next_value());
            self.listen_weight.set(switch(self.listen));
            let listen = f64::from(self.listen_weight.next_value());
            let range = choice(range_lane, n, Param::Range, self.range);
            let mut weights = [0.0; 3];
            for (k, (ramp, weight)) in self.ranges.iter_mut().zip(&mut weights).enumerate() {
                ramp.set(switch(k == range));
                *weight = f64::from(ramp.next_value());
            }
            let factors = self.factors[choice(reaction_lane, n, Param::Reaction, self.reaction)];
            let amount = self.amount.next_value();
            let amount = lane(amount_lane, n)
                .and_then(|value| Param::Amount.info().clamp(value))
                .unwrap_or(amount);
            let target = lane(frequency_lane, n)
                .and_then(|value| Param::FrequencyHz.info().clamp(value))
                .unwrap_or(self.frequency_set);
            self.frequency.set(target);
            let frequency = self.frequency.next_value();
            if self.designed != Some(frequency.to_bits()) {
                self.filters = filters(self.rate, frequency);
                self.designed = Some(frequency.to_bits());
            }

            // The band of each channel, and the levels the followers hear.
            let mut band_peak = 0.0_f64;
            let mut peak = 0.0_f64;
            for (channel, past) in self.past.iter_mut().enumerate() {
                let x = finite(input[channel][n]);
                let mut out = [0.0; 4];
                for ((filter, outputs), out) in
                    self.filters.iter().zip(&mut past.outputs).zip(&mut out)
                {
                    *out = filter.tick(x, past.input, outputs);
                }
                past.input = [x, past.input[0]];
                let [narrow, wide, high] = weights;
                past.band = narrow * out[0] + wide * out[1] + high * out[2];
                let heard = narrow * out[0] + wide * out[1] + high * out[3];
                if Some(channel) != self.lfe {
                    band_peak = band_peak.max(heard.abs());
                    peak = peak.max(x.abs());
                }
            }
            self.band_level = DeEsser::follow(self.band_level, band_peak, factors);
            self.level = DeEsser::follow(self.level, peak, factors);
            let reduction = if self.level > 0.0 && self.band_level > 0.0 {
                reduction_db(
                    20.0 * self.band_level.log10(),
                    20.0 * (self.band_level / self.level).log10(),
                    f64::from(amount),
                )
            } else {
                0.0
            };
            deepest = deepest.max(reduction);
            let cut = 1.0 - gain_of_reduction(reduction);
            for ((output, input), past) in output.iter_mut().zip(input).zip(&self.past) {
                let x = f64::from(input[n]);
                let removed = cut * past.band;
                let heard = mix(x - removed, removed, listen);
                let y = mix(x, heard, wet);
                output_peak = output_peak.max(y.abs());
                output[n] = y as f32;
            }
        }
        if ctx.frames > 0 {
            #[allow(clippy::cast_possible_truncation, reason = "a level")]
            let reduction = -deepest as f32;
            self.meter.write((reduction, meter_db(output_peak)));
        }
    }

    fn latency(&self) -> u32 {
        0
    }

    /// [`settle_for`] the target set, or the one it ramps from where that is lower.
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
        for (k, ramp) in self.ranges.iter_mut().enumerate() {
            ramp.jump(switch(k == self.range));
        }
        self.frequency.jump(self.frequency_set);
        self.amount.finish();
        self.designed = None;
        self.past.fill(Past::default());
        self.band_level = 0.0;
        self.level = 0.0;
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a code of 0 to 2"
    )]
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
            Param::FrequencyHz => self.frequency_set = value,
            Param::Range => self.range = value as usize,
            Param::Amount => self.amount.set(value),
            Param::Reaction => self.reaction = value as usize,
            Param::Listen => self.listen = on,
        }
    }
}

/// The gain of amplitude of a reduction of `db`.
#[inline]
fn gain_of_reduction(db: f64) -> f64 {
    if db <= 0.0 {
        1.0
    } else {
        super::common::gain_of(-db)
    }
}
