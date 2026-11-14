//! Gain (pp. 3963–3964): raises or lowers the sound by a number of decibels, −24 to +24, 0 for a
//! new one (C-X6). Every channel alike; no latency, no state but its ramps.
//!
//! The gain ramps over 10 ms in its amplitude ([`Smoothed`]); a lane of automation is taken as it
//! is, a value at each sample. The switch of the insert crossfades over 10 ms between the input
//! and the gained sound. A gain of 0 dB, and the insert off, pass the sound to the bit.

use super::common::{lane, mix, params, ParamInfo};
use crate::constants::{samples, SMOOTHING};
use crate::node::{Block, Node, ParamId, Spec, Unsupported};
use crate::rt::{Domain, Smoothed};
use crate::Layout;

params! {
    /// A parameter of [`Gain`], in the units of the model's `GainFx`.
    pub enum Param {
        /// The insert on (1) or off (0).
        On => ParamInfo::switch("on", true),
        /// The gain, dB, −24 to 24 (C-X6).
        GainDb => ParamInfo::number("gain_db", -24.0, 24.0, 0.0),
    }
}

/// The settings of a Gain: the model's `GainFx`, and the switch of its insert.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// The insert is on.
    pub on: bool,
    /// The gain, dB.
    pub gain_db: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            on: true,
            gain_db: 0.0,
        }
    }
}

impl Settings {
    /// The value of `param`, as [`Node::set`] takes it.
    #[must_use]
    pub fn value(&self, param: Param) -> f32 {
        match param {
            Param::On => f32::from(u8::from(self.on)),
            Param::GainDb => self.gain_db,
        }
    }

    /// Every parameter and its value: a Gain set so has these settings.
    #[must_use]
    pub fn params(&self) -> [(ParamId, f32); 2] {
        [Param::On, Param::GainDb].map(|param| (param.id(), self.value(param)))
    }
}

/// The samples after which a Gain of `settings` at `rate`, started late, sounds as one that played
/// from earlier: its ramps, 10 ms.
#[must_use]
pub fn settle_for(_settings: &Settings, rate: u32) -> u32 {
    u32::try_from(samples(SMOOTHING, rate)).unwrap_or(u32::MAX)
}

/// The gain of amplitude of `db`, in `f32`: what the ramp holds.
#[allow(clippy::cast_possible_truncation, reason = "a gain of ±24 dB")]
fn amplitude(db: f32) -> f32 {
    super::common::gain_of(f64::from(db)) as f32
}

/// Gain, a [`Node`] of no latency.
#[derive(Clone, Debug)]
pub struct Gain {
    rate: u32,
    on: bool,
    /// The insert on, from 0 to 1.
    wet: Smoothed,
    /// The gain, of amplitude.
    level: Smoothed,
}

impl Gain {
    /// A Gain of `settings`, at once with no ramp.
    #[must_use]
    pub fn new(settings: &Settings) -> Self {
        let gain_db = Param::GainDb.info().clamp(settings.gain_db).unwrap_or(0.0);
        Gain {
            rate: 0,
            on: settings.on,
            wet: Smoothed::new(f32::from(u8::from(settings.on)), Domain::Linear),
            level: Smoothed::new(amplitude(gain_db), Domain::Linear),
        }
    }
}

impl Default for Gain {
    fn default() -> Self {
        Gain::new(&Settings::default())
    }
}

impl Node for Gain {
    fn prepare(&mut self, spec: &Spec) -> Result<(), Unsupported> {
        if spec.layout != spec.output
            || spec.layout == Layout::Empty
            || spec.sidechain > 0
            || spec.rate == 0
        {
            return Err(Unsupported::from(spec));
        }
        self.rate = spec.rate;
        self.wet.prepare(spec.rate);
        self.level.prepare(spec.rate);
        Ok(())
    }

    #[allow(clippy::cast_possible_truncation, reason = "the samples are f32")]
    fn process(&mut self, ctx: &Block, input: &[&[f32]], output: &mut [&mut [f32]]) {
        let on_lane = ctx.automation.get(Param::On.id());
        let gain_lane = ctx.automation.get(Param::GainDb.id());
        for n in 0..ctx.frames {
            let on = lane(on_lane, n).map_or(self.on, |on| on >= 0.5);
            self.wet.set(f32::from(u8::from(on)));
            let wet = f64::from(self.wet.next_value());
            let set = self.level.next_value();
            let gain = lane(gain_lane, n)
                .and_then(|db| Param::GainDb.info().clamp(db))
                .map_or(set, amplitude);
            let gain = mix(1.0, f64::from(gain), wet);
            for (output, input) in output.iter_mut().zip(input) {
                output[n] = (f64::from(input[n]) * gain) as f32;
            }
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
        self.level.finish();
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
            Param::GainDb => self.level.set(amplitude(value)),
        }
    }
}
