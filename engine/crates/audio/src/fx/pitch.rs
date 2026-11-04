//! Pitch (p. 3971): moves the pitch of a track's sound without changing its speed, by Semitones
//! (−12 to 12) and Cents (−100 to 100), mixed with the dry sound by Dry/Wet (C-X9). The same
//! library as a clip's Pitch and Pitch Correction: Signalsmith Stretch, run in segments
//! ([`super::stretch`]) every [`crate::constants::STRETCH_RESET_INTERVAL`] of the timeline.
//!
//! # Latency
//!
//! The stretch is fed its input [`Config::input_latency`] ahead of the time it works on and gives
//! its output [`Config::output_latency`] behind it, and it is worked out in chunks of
//! [`CHUNK`] samples, each once its input is all in, a crossfade ahead where two segments meet:
//! the node's output at a sample answers its input [`Pitch::latency`] = `CHUNK` + the crossfade +
//! both latencies before (about 131 ms at 48 kHz), fixed from `prepare` on. The dry sound is the input delayed as much, so that every mix of the two is
//! aligned, and so is the bypass (Insert off), which still runs the stretch underneath.
//!
//! # Settling
//!
//! A segment under way when the node starts is stretched from there, late, and the stretchers
//! that start soon after are handed silence for the input before the start. Once every segment
//! that sounds started after the input it reads, the node sounds as one started earlier, to the
//! bit: [`settle_for`].
//!
//! # Parameters
//!
//! Semitones and Cents are ramped over 10 ms and taken by the stretch where it works out a chunk
//! (the library moves to a new pitch at its next block); Dry/Wet is ramped and the switch
//! crossfaded over 10 ms, sample by sample. A lane that is not a number leaves the setting as it
//! is; an input that is not a number is taken as silence.

use columbina_stretch::Config;

use super::common::{finite, lane, mix, params, ParamInfo};
use super::denoise::Switch;
use super::stretch::{Grid, Input, Plan, Stretcher, Transpose, CHUNK};
use crate::feed::FeedError;
use crate::node::{Block, Node, ParamId, Planar, Spec, Unsupported};
use crate::rt::{Domain, Smoothed};
use crate::Layout;

params! {
    /// A parameter of [`Pitch`], in the units of the model's `PitchFx`.
    pub enum Param {
        /// The insert on (1) or off (0).
        On => ParamInfo::switch("on", true),
        /// Semitones, −12 to 12 (p. 3971).
        Semitones => ParamInfo::number("semitones", -12.0, 12.0, 0.0),
        /// Cents, −100 to 100 (C-X9).
        Cents => ParamInfo::number("cents", -100.0, 100.0, 0.0),
        /// Dry/Wet, percent, 0 (dry) to 100 (wet).
        DryWet => ParamInfo::number("dry_wet", 0.0, 100.0, 100.0),
    }
}

/// The settings of a Pitch: the model's `PitchFx`, and the switch of its insert.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// The insert is on.
    pub on: bool,
    /// Semitones.
    pub semitones: f32,
    /// Cents.
    pub cents: f32,
    /// Dry/Wet, percent.
    pub dry_wet: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            on: true,
            semitones: 0.0,
            cents: 0.0,
            dry_wet: 100.0,
        }
    }
}

impl Settings {
    /// The value of `param`, as [`Node::set`] takes it.
    #[must_use]
    pub fn value(&self, param: Param) -> f32 {
        match param {
            Param::On => f32::from(u8::from(self.on)),
            Param::Semitones => self.semitones,
            Param::Cents => self.cents,
            Param::DryWet => self.dry_wet,
        }
    }

    /// Every parameter and its value: a Pitch set so has these settings.
    #[must_use]
    pub fn params(&self) -> [(ParamId, f32); 4] {
        [Param::On, Param::Semitones, Param::Cents, Param::DryWet]
            .map(|param| (param.id(), self.value(param)))
    }
}

/// The factor of frequency of `semitones` and `cents`.
#[must_use]
pub fn factor(semitones: f32, cents: f32) -> f32 {
    2.0_f32.powf((semitones + cents / 100.0) / 12.0)
}

/// The latency of a Pitch whose stretch works as `config` and crossfades over `fade`: a chunk, the
/// crossfade (the sound of both segments over it is worked out before it starts) and both
/// latencies of the library.
fn latency_of(config: &Config, fade: i64) -> i64 {
    i64::try_from(CHUNK + config.input_latency + config.output_latency)
        .unwrap_or(i64::MAX)
        .saturating_add(fade)
}

/// The samples after which a Pitch at `rate`, started late, sounds as one that played from
/// earlier: a chunk, a crossfade and a seek of the library (the stretchers that start in that
/// time read silence before the start), then the longest that a segment under way lasts: its
/// warming, a period and the crossfade. The same for any settings; 0 where the library cannot be made.
#[must_use]
pub fn settle_for(_settings: &Settings, rate: u32) -> u32 {
    let Ok(config) = Config::of(rate) else {
        return 0;
    };
    let fade = fade_of(rate);
    let Grid::Timeline { period, .. } = Grid::timeline(rate, fade) else {
        return 0;
    };
    let settle = i64::try_from(CHUNK + config.seek_length).unwrap_or(i64::MAX)
        + fade
        + Stretcher::warm_of(&config)
        + period
        + fade
        + 1;
    u32::try_from(settle).unwrap_or(u32::MAX)
}

/// The samples of the crossfade between two segments at `rate`.
fn fade_of(rate: u32) -> i64 {
    i64::try_from(crate::constants::samples(
        crate::constants::STRETCH_RESET_CROSSFADE,
        rate,
    ))
    .unwrap_or(0)
}

/// What the stretch of a Pitch is fed: its input as it came, kept in a ring by the sample of the
/// timeline it plays at; silence before its start.
struct History<'a> {
    ring: &'a Planar,
    /// The sample of the timeline of the node's first sample.
    origin: i64,
    /// The sample after the last one written.
    end: i64,
    /// The latencies of the stretch, input and output.
    ahead: i64,
}

impl History<'_> {
    fn at(&self, channel: usize, x: i64) -> f32 {
        let capacity = i64::try_from(self.ring.frames()).unwrap_or(i64::MAX).max(1);
        if x < self.origin || x >= self.end || x < self.end - capacity {
            return 0.0;
        }
        let index = usize::try_from(x.rem_euclid(capacity)).unwrap_or(0);
        self.ring.channel(channel)[index]
    }
}

impl Input for History<'_> {
    fn cursor(&self, v: i64) -> i64 {
        v + self.ahead
    }

    fn fill(
        &mut self,
        range: std::ops::Range<i64>,
        into: &mut [&mut [f32]],
    ) -> Result<(), FeedError> {
        for (channel, into) in into.iter_mut().enumerate() {
            for (sample, x) in into.iter_mut().zip(range.clone()) {
                *sample = self.at(channel, x);
            }
        }
        Ok(())
    }
}

/// Pitch, a [`Node`] of a latency of about 131 ms ([`Pitch::latency`]).
#[derive(Debug)]
pub struct Pitch {
    on: Switch,
    semitones: Smoothed,
    cents: Smoothed,
    dry_wet: Smoothed,
    rate: u32,
    stretcher: Option<Stretcher>,
    latency: i64,
    ahead: i64,
    settle: u32,
    /// The input, by the sample of the timeline modulo its length.
    ring: Planar,
    /// The stretched sound of a block.
    wet: Planar,
    /// The factor of the pitch at each sample of a block.
    factors: Planar,
    grid: Grid,
    /// The sample of the timeline of the first sample since the reset.
    origin: i64,
    /// The samples processed since the reset.
    count: i64,
}

impl Pitch {
    /// A Pitch of `settings`, at once with no ramp.
    #[must_use]
    pub fn new(settings: &Settings) -> Self {
        let clamp =
            |param: Param, value: f32| param.info().clamp(value).unwrap_or(param.info().default);
        Pitch {
            on: Switch::new(settings.on),
            semitones: Smoothed::new(clamp(Param::Semitones, settings.semitones), Domain::Linear),
            cents: Smoothed::new(clamp(Param::Cents, settings.cents), Domain::Linear),
            dry_wet: Smoothed::new(
                clamp(Param::DryWet, settings.dry_wet) / 100.0,
                Domain::Linear,
            ),
            rate: 0,
            stretcher: None,
            latency: 0,
            ahead: 0,
            settle: 0,
            ring: Planar::new(0, 0),
            wet: Planar::new(0, 0),
            factors: Planar::new(0, 0),
            grid: Grid::timeline(1, 0),
            origin: 0,
            count: 0,
        }
    }
}

impl Default for Pitch {
    fn default() -> Self {
        Pitch::new(&Settings::default())
    }
}

impl Node for Pitch {
    fn prepare(&mut self, spec: &Spec) -> Result<(), Unsupported> {
        if spec.layout != spec.output
            || spec.layout == Layout::Empty
            || spec.sidechain > 0
            || spec.rate == 0
        {
            return Err(Unsupported::from(spec));
        }
        let channels = spec.inputs();
        let stretcher =
            Stretcher::new(channels, spec.rate, CHUNK).map_err(|_| Unsupported::from(spec))?;
        let config = stretcher.config();
        self.latency = latency_of(&config, stretcher.fade());
        self.ahead = i64::try_from(config.input_latency + config.output_latency).unwrap_or(0);
        self.grid = Grid::timeline(spec.rate, stretcher.fade());
        self.settle = settle_for(&Settings::default(), spec.rate);
        let back = (CHUNK + config.seek_length).max(usize::try_from(self.latency).unwrap_or(0));
        self.ring = Planar::new(channels, spec.max_block + back + 1);
        self.wet = Planar::new(channels, spec.max_block);
        self.factors = Planar::new(1, spec.max_block);
        self.stretcher = Some(stretcher);
        self.rate = spec.rate;
        self.on.prepare(spec.rate);
        self.semitones.prepare(spec.rate);
        self.cents.prepare(spec.rate);
        self.dry_wet.prepare(spec.rate);
        self.reset();
        Ok(())
    }

    #[allow(clippy::cast_possible_truncation, reason = "the samples are f32")]
    fn process(&mut self, ctx: &Block, input: &[&[f32]], output: &mut [&mut [f32]]) {
        let frames = ctx.frames;
        let Pitch {
            on,
            semitones,
            cents,
            dry_wet,
            stretcher,
            latency,
            ahead,
            ring,
            wet,
            factors,
            grid,
            origin,
            count,
            ..
        } = self;
        let Some(stretcher) = stretcher.as_mut() else {
            for (output, input) in output.iter_mut().zip(input) {
                output[..frames].copy_from_slice(&input[..frames]);
            }
            return;
        };
        if frames == 0 {
            return;
        }
        let capacity = i64::try_from(ring.frames()).unwrap_or(i64::MAX).max(1);
        let base = *origin + *count;
        for (channel, input) in input.iter().enumerate().take(ring.channels()) {
            let ring = ring.channel_mut(channel);
            for (n, sample) in input[..frames].iter().enumerate() {
                let x = base + i64::try_from(n).unwrap_or(0);
                ring[usize::try_from(x.rem_euclid(capacity)).unwrap_or(0)] = finite(*sample) as f32;
            }
        }

        let semitone_lane = ctx.automation.get(Param::Semitones.id());
        let cent_lane = ctx.automation.get(Param::Cents.id());
        let factors = &mut factors.channel_mut(0)[..frames];
        for (n, factor_at) in factors.iter_mut().enumerate() {
            let set = semitones.next_value();
            let st = lane(semitone_lane, n)
                .and_then(|value| Param::Semitones.info().clamp(value))
                .unwrap_or(set);
            let set = cents.next_value();
            let ct = lane(cent_lane, n)
                .and_then(|value| Param::Cents.info().clamp(value))
                .unwrap_or(set);
            *factor_at = factor(st, ct);
        }

        let end = base + i64::try_from(frames).unwrap_or(0);
        let mut history = History {
            ring,
            origin: *origin,
            end,
            ahead: *ahead,
        };
        let plan = Plan {
            grid: *grid,
            segments: (i64::MIN, i64::MAX),
            floor: *origin - *latency,
            rate: 1.0,
        };
        let span = base - *latency..end - *latency;
        wet.write(frames, |wet| {
            // The history cannot fail.
            let _ = stretcher.render(
                &plan,
                span.clone(),
                &mut history,
                Transpose::Each(&*factors),
                Some(wet),
            );
        });

        let dry_wet_lane = ctx.automation.get(Param::DryWet.id());
        let on_lane = ctx.automation.get(Param::On.id());
        for n in 0..frames {
            let set = dry_wet.next_value();
            let weight = lane(dry_wet_lane, n)
                .and_then(|value| Param::DryWet.info().clamp(value))
                .map_or(set, |value| value / 100.0);
            let bypass = on.next(lane(on_lane, n));
            let v = span.start + i64::try_from(n).unwrap_or(0);
            for (channel, output) in output.iter_mut().enumerate().take(wet.channels()) {
                let d = f64::from(history.at(channel, v));
                let y = f64::from(wet.channel(channel)[n]);
                let processed = mix(d, y, f64::from(weight));
                output[n] = mix(d, processed, f64::from(bypass)) as f32;
            }
        }
        *count += i64::try_from(frames).unwrap_or(0);
    }

    fn latency(&self) -> u32 {
        u32::try_from(self.latency).unwrap_or(u32::MAX)
    }

    fn settle(&self) -> u32 {
        self.settle
    }

    fn reset(&mut self) {
        self.ring.silence();
        self.count = 0;
        if let Some(stretcher) = &mut self.stretcher {
            stretcher.release();
        }
        self.on.finish();
        self.semitones.finish();
        self.cents.finish();
        self.dry_wet.finish();
    }

    fn align(&mut self, origin: u64) {
        // The graph hands a sample before 0 (a preroll from the start) in its bits.
        self.origin = origin.cast_signed();
    }

    fn set(&mut self, param: ParamId, value: f32) {
        let Some(param) = Param::from_id(param) else {
            return;
        };
        let Some(value) = param.info().clamp(value) else {
            return;
        };
        match param {
            Param::On => self.on.on = value >= 0.5,
            Param::Semitones => self.semitones.set(value),
            Param::Cents => self.cents.set(value),
            Param::DryWet => self.dry_wet.set(value / 100.0),
        }
    }
}
