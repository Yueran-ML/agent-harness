//! Soft Clipper (pp. 3973–3974): rounds off what goes over 0 dBFS ([`SOFT_CLIP_DBFS`]).
//!
//! Threshold is, as the manual has it, a gain before the clipper ("introduces input gain to the
//! signal prior to hitting the clipper"), 0 to +24 dB; Shape goes from a hard clip (0) to a soft
//! one (100, a new one's); Output Level is a gain after it, −24 to +12 dB (C-X10). For a sample
//! `x`, with `g` the gain of Threshold, `s` the Shape over 100 and `o` the gain of Output Level:
//!
//! ```text
//! y = o · ((1 − s) · clamp(g·x, −1, 1) + s · tanh(g·x))
//! ```
//!
//! so that no shape lets out more than 0 dBFS before Output Level, and the soft one is the
//! hyperbolic tangent, which is the identity at small levels (−0.03 dB at −20 dBFS) and bends
//! more as the level rises (the reference, §10.11). Every channel alike, no latency, no
//! oversampling (what a clip adds above the Nyquist frequency folds back, as in a plain clipper).
//!
//! The gains ramp over 10 ms in their amplitude and the shape in its value ([`Smoothed`]); lanes
//! of automation are taken as they are. The switch of the insert crossfades over 10 ms; off, the
//! sound passes to the bit.

use super::common::{lane, mix, params, ParamInfo};
use crate::constants::{samples, SMOOTHING, SOFT_CLIP_DBFS};
use crate::node::{Block, Node, ParamId, Spec, Unsupported};
use crate::rt::{Domain, Smoothed};
use crate::Layout;

params! {
    /// A parameter of [`SoftClipper`], in the units of the model's `SoftClipperFx`.
    pub enum Param {
        /// The insert on (1) or off (0).
        On => ParamInfo::switch("on", true),
        /// The gain before the clipper, dB, 0 to 24 (named Threshold; C-X10).
        ThresholdDb => ParamInfo::number("threshold_db", 0.0, 24.0, 0.0),
        /// From 0, a hard clip, to 100, all soft.
        Shape => ParamInfo::number("shape", 0.0, 100.0, 100.0),
        /// The gain after it, dB, −24 to 12.
        OutputDb => ParamInfo::number("output_db", -24.0, 12.0, 0.0),
    }
}

/// The settings of a Soft Clipper: the model's `SoftClipperFx`, and the switch of its insert.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// The insert is on.
    pub on: bool,
    /// The gain before the clipper, dB.
    pub threshold_db: f32,
    /// 0 (hard) to 100 (soft).
    pub shape: f32,
    /// The gain after it, dB.
    pub output_db: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            on: true,
            threshold_db: 0.0,
            shape: 100.0,
            output_db: 0.0,
        }
    }
}

impl Settings {
    /// The value of `param`, as [`Node::set`] takes it.
    #[must_use]
    pub fn value(&self, param: Param) -> f32 {
        match param {
            Param::On => f32::from(u8::from(self.on)),
            Param::ThresholdDb => self.threshold_db,
            Param::Shape => self.shape,
            Param::OutputDb => self.output_db,
        }
    }

    /// Every parameter and its value: a Soft Clipper set so has these settings.
    #[must_use]
    pub fn params(&self) -> [(ParamId, f32); 4] {
        [Param::On, Param::ThresholdDb, Param::Shape, Param::OutputDb]
            .map(|param| (param.id(), self.value(param)))
    }
}

/// The samples after which a Soft Clipper of `settings` at `rate`, started late, sounds as one
/// that played from earlier: its ramps, 10 ms.
#[must_use]
pub fn settle_for(_settings: &Settings, rate: u32) -> u32 {
    u32::try_from(samples(SMOOTHING, rate)).unwrap_or(u32::MAX)
}

/// The output of the clipper for `x` (the gain of Threshold applied), of `shape` from 0 (hard) to
/// 1 (soft), before Output Level: the curve of the window.
#[must_use]
pub fn curve(x: f64, shape: f64) -> f64 {
    let level = f64::from(crate::db_to_gain(SOFT_CLIP_DBFS));
    let hard = x.clamp(-level, level);
    let soft = level * (x / level).tanh();
    mix(hard, soft, shape)
}

/// The gain of amplitude of `db`, in `f32`: what a ramp holds.
#[allow(clippy::cast_possible_truncation, reason = "a gain of ±24 dB")]
fn amplitude(db: f32) -> f32 {
    super::common::gain_of(f64::from(db)) as f32
}

/// Soft Clipper, a [`Node`] of no latency.
#[derive(Clone, Debug)]
pub struct SoftClipper {
    rate: u32,
    on: bool,
    wet: Smoothed,
    /// The gain before, of amplitude.
    threshold: Smoothed,
    /// The shape, 0 to 100.
    shape: Smoothed,
    /// The gain after, of amplitude.
    output: Smoothed,
}

impl SoftClipper {
    /// A Soft Clipper of `settings`, at once with no ramp.
    #[must_use]
    pub fn new(settings: &Settings) -> Self {
        let value = |param: Param| {
            param
                .info()
                .clamp(settings.value(param))
                .unwrap_or(param.info().default)
        };
        SoftClipper {
            rate: 0,
            on: settings.on,
            wet: Smoothed::new(f32::from(u8::from(settings.on)), Domain::Linear),
            threshold: Smoothed::new(amplitude(value(Param::ThresholdDb)), Domain::Linear),
            shape: Smoothed::new(value(Param::Shape), Domain::Linear),
            output: Smoothed::new(amplitude(value(Param::OutputDb)), Domain::Linear),
        }
    }
}

impl SoftClipper {
    /// The gain before (of amplitude), the shape (0 to 100) and the gain after (of amplitude) at
    /// the last sample processed: where their ramps are.
    #[must_use]
    pub fn heard(&self) -> (f32, f32, f32) {
        (
            self.threshold.current(),
            self.shape.current(),
            self.output.current(),
        )
    }
}

impl Default for SoftClipper {
    fn default() -> Self {
        SoftClipper::new(&Settings::default())
    }
}

/// The value at `n` of a lane of `param` in its smoothed domain, or the ramp's.
fn automated(values: Option<&[f32]>, n: usize, param: Param, ramp: f32) -> f32 {
    lane(values, n)
        .and_then(|value| param.info().clamp(value))
        .map_or(ramp, |value| match param {
            Param::ThresholdDb | Param::OutputDb => amplitude(value),
            _ => value,
        })
}

impl Node for SoftClipper {
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
            &mut self.threshold,
            &mut self.shape,
            &mut self.output,
        ] {
            ramp.prepare(spec.rate);
        }
        Ok(())
    }

    #[allow(clippy::cast_possible_truncation, reason = "the samples are f32")]
    fn process(&mut self, ctx: &Block, input: &[&[f32]], output: &mut [&mut [f32]]) {
        let automation = &ctx.automation;
        let [on_lane, threshold_lane, shape_lane, output_lane] =
            [Param::On, Param::ThresholdDb, Param::Shape, Param::OutputDb]
                .map(|param| automation.get(param.id()));
        for n in 0..ctx.frames {
            let on = lane(on_lane, n).map_or(self.on, |on| on >= 0.5);
            self.wet.set(f32::from(u8::from(on)));
            let wet = f64::from(self.wet.next_value());
            let threshold = automated(
                threshold_lane,
                n,
                Param::ThresholdDb,
                self.threshold.next_value(),
            );
            let shape = automated(shape_lane, n, Param::Shape, self.shape.next_value());
            let gain_out = automated(output_lane, n, Param::OutputDb, self.output.next_value());
            let (threshold, shape, gain_out) = (
                f64::from(threshold),
                f64::from(shape) / 100.0,
                f64::from(gain_out),
            );
            for (output, input) in output.iter_mut().zip(input) {
                let x = f64::from(input[n]);
                let clipped = curve(x * threshold, shape) * gain_out;
                output[n] = mix(x, clipped, wet) as f32;
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
        self.threshold.finish();
        self.shape.finish();
        self.output.finish();
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
            Param::ThresholdDb => self.threshold.set(amplitude(value)),
            Param::Shape => self.shape.set(value),
            Param::OutputDb => self.output.set(amplitude(value)),
        }
    }
}
