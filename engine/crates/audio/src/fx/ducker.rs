//! Ducker (pp. 3940–3941; Edit page pp. 979–981), the Track FX of a track: while the tracks and
//! buses that are its sources sound, it lowers the track by a set amount, "without
//! compressing" — no ratio and no knee, the same amount whatever the level of the sources.
//!
//! # What it hears
//!
//! The graph sums its sources, each taken before its fader and its mute (C-D5), lined up with the
//! track and their LFE left out, into the sidechain of the node (`graph/strip.rs`). The node
//! takes, for each channel of the sidechain, the mean of the squares of its last 10 ms
//! ([`DUCKER_RMS_WINDOW`]), and the loudest channel: the sources sound when that is at or above
//! −45 dBFS ([`DUCKER_THRESHOLD_DBFS`]), and stop sounding when it falls under −48 dBFS, 3 dB of
//! hysteresis ([`DUCKER_HYSTERESIS_DB`]) that keeps a source hovering at the threshold from
//! switching the Ducker on and off (plan T11; C-X2), or when it has stayed between the two for
//! 0.5 s since it was last at the threshold ([`DUCKER_HYSTERESIS_LATCH`]), so that room tone
//! inside the hysteresis does not hold the track down through a whole pause. A sample of the
//! sidechain that is not a number (±∞, NaN) is heard as silence.
//!
//! The squares are summed as whole numbers (each square of a sample in units of 2⁻⁴⁰, the
//! square of a sample being exact in `f64`), so that the sum over the window is exact: it does
//! not drift, and two nodes that heard the same last 10 ms hold the same sum, however long ago
//! they started and however their blocks were cut.
//!
//! # What it does
//!
//! A level between 0 (not ducked) and 1 (ducked by the amount) follows the sources: while they
//! sound, and for Hold after they stop, it rises by one part of Rise a sample; then it falls by
//! one part of Recovery a sample. The hold is at least the rise, so that every duck goes all the
//! way down and the level forgets where it started (T11's review, P2-3); at the defaults (a hold
//! of 150 ms, a rise of 10) that changes nothing. The gain is the depth (the gain of −Amount dB) to the power of
//! that level, so that the decibels go down and up in straight lines (C-X2) and a change of the
//! amount while ducked is a change of the depth. Not ducked, the gain is 1 exactly and the sound
//! passes to the bit; every channel of the track takes the same gain, the LFE too.
//!
//! The track is delayed by the lookahead ([`Ducker::latency`], which the graph makes up for): the
//! gain that the sources call for at a sample is applied to the sound of the lookahead before
//! it, so that the track is already going down when the sources start (p. 3941). The lookahead
//! is fixed when the node is made ([`super::common::Kind::Fixed`]): a change of it is a change of
//! the graph.
//!
//! # Changes while the sound plays
//!
//! Amount ramps over 10 ms in its gain ([`Smoothed`]), and the switch of the Ducker crossfades over
//! 10 ms between the sound as it is and the sound ducked; their lanes of automation are taken as
//! they are, a value at each sample. Rise, Hold and Recovery change the pace of the level, not the
//! level, and are taken at once, from their lanes too: a hold made shorter than the quiet so far
//! ends there, and one made longer after it has ended does not duck again. Off, the Ducker still
//! hears its sources and
//! still delays the track by the lookahead, so that it sounds at once as it would have when it is
//! turned on again.
//!
//! # Settling
//!
//! A Ducker started late sounds as one that played from earlier once its ramps have ended, its
//! lookahead and window have been filled again, its hold has run out, and its level has gone all
//! the way up or down ([`settle_for`]): its level, a straight line between two ends, forgets where
//! it was only at an end. For sources that sound, or stay quiet, for at least a rise or a recovery
//! now and then, as speech does.

use super::common::{finite, gain_of, lane, mix, params, ParamInfo};
use crate::constants::{
    samples, DUCKER_HYSTERESIS_DB, DUCKER_HYSTERESIS_LATCH, DUCKER_RMS_WINDOW,
    DUCKER_THRESHOLD_DBFS, MAX_CHANNELS, SMOOTHING,
};
use crate::node::{Block, Node, ParamId, Spec, Unsupported};
use crate::rt::{seqlock, Domain, SeqReader, SeqWriter, Smoothed};
use crate::Layout;

params! {
    /// A parameter of [`Ducker`], in the units of the model's `Ducker` (C-X1).
    pub enum Param {
        /// The Ducker on (1) or off (0): off for a new one.
        On => ParamInfo::switch("on", false),
        /// How far it lowers the track, dB, 0 to 30, 2.7 for a new one (p. 3941).
        AmountDb => ParamInfo::number("amount_db", 0.0, 30.0, 2.7),
        /// How far ahead of the sources it starts, ms, 0 to 100, 15 for a new one: fixed when the
        /// node is made, as it is the latency.
        LookaheadMs => ParamInfo::fixed("lookahead_ms", 0.0, 100.0, 15.0),
        /// The time to go down by the amount, ms, 1 to 1000, 10 for a new one.
        RiseMs => ParamInfo::number("rise_ms", 1.0, 1000.0, 10.0),
        /// The time it stays down after the sources stop, ms, 0 to 4000, 150 for a new one.
        HoldMs => ParamInfo::number("hold_ms", 0.0, 4000.0, 150.0),
        /// The time to come back up by the amount, ms, 10 to 4000, 750 for a new one.
        RecoveryMs => ParamInfo::number("recovery_ms", 10.0, 4000.0, 750.0),
    }
}

/// The settings of a Ducker: the model's `Ducker` but its sources, which the graph routes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// It is on.
    pub on: bool,
    /// How far it lowers the track, dB.
    pub amount_db: f32,
    /// How far ahead of the sources it starts, ms.
    pub lookahead_ms: f32,
    /// The time to go down, ms.
    pub rise_ms: f32,
    /// The time it stays down after the sources stop, ms.
    pub hold_ms: f32,
    /// The time to come back up, ms.
    pub recovery_ms: f32,
}

impl Default for Settings {
    fn default() -> Self {
        let default = |param: Param| param.info().default;
        Settings {
            on: false,
            amount_db: default(Param::AmountDb),
            lookahead_ms: default(Param::LookaheadMs),
            rise_ms: default(Param::RiseMs),
            hold_ms: default(Param::HoldMs),
            recovery_ms: default(Param::RecoveryMs),
        }
    }
}

impl Settings {
    /// The value of `param`, as [`Node::set`] takes it.
    #[must_use]
    pub fn value(&self, param: Param) -> f32 {
        match param {
            Param::On => f32::from(u8::from(self.on)),
            Param::AmountDb => self.amount_db,
            Param::LookaheadMs => self.lookahead_ms,
            Param::RiseMs => self.rise_ms,
            Param::HoldMs => self.hold_ms,
            Param::RecoveryMs => self.recovery_ms,
        }
    }

    /// Every parameter that a Ducker takes while it plays, and its value: a Ducker of the same
    /// lookahead set so has these settings.
    #[must_use]
    pub fn params(&self) -> [(ParamId, f32); 5] {
        [
            Param::On,
            Param::AmountDb,
            Param::RiseMs,
            Param::HoldMs,
            Param::RecoveryMs,
        ]
        .map(|param| (param.id(), self.value(param)))
    }

    /// The value of `param`, kept in its range; its default where it is not a number.
    fn clamped(&self, param: Param) -> f32 {
        param
            .info()
            .clamp(self.value(param))
            .unwrap_or(param.info().default)
    }
}

/// The samples of `ms` milliseconds at `rate`, as a count to the nearest.
fn samples_of(ms: f32, rate: u32) -> usize {
    samples(f64::from(ms) / 1000.0, rate)
}

/// The samples of the lookahead of `settings` at `rate`: its latency.
#[must_use]
pub fn lookahead_for(settings: &Settings, rate: u32) -> u32 {
    let ms = settings.clamped(Param::LookaheadMs);
    u32::try_from(samples_of(ms, rate)).unwrap_or(u32::MAX)
}

/// The samples after which a Ducker of `settings` at `rate`, started late, sounds as one that
/// played from earlier: its ramps (10 ms), its lookahead, its window (10 ms), the latch of its
/// hysteresis (0.5 s), its hold (at least its rise), and a whole rise and recovery; on or off
/// alike, since a lane may turn it on. See the module.
#[must_use]
pub fn settle_for(settings: &Settings, rate: u32) -> u32 {
    let time = |param: Param| samples_of(settings.clamped(param), rate);
    let total = samples(SMOOTHING, rate)
        + time(Param::LookaheadMs)
        + samples(DUCKER_RMS_WINDOW, rate)
        + samples(DUCKER_HYSTERESIS_LATCH, rate)
        + time(Param::HoldMs).max(time(Param::RiseMs))
        + time(Param::RiseMs)
        + time(Param::RecoveryMs);
    u32::try_from(total).unwrap_or(u32::MAX)
}

/// What the window of the Ducker reads after each block (the grey and the yellow lines, p. 3941):
/// the peak of the track as it comes in, the RMS of the sources (their loudest channel) at the
/// end of the block, in dBFS (−200 for silence), and the deepest reduction of the block, in dB
/// (0 or below).
pub type Meter = (f32, f32, f32);

/// The floor of the meters, in dB.
const METER_FLOOR_DB: f32 = -200.0;

/// The unit of the squares summed: 2⁻⁴⁰ of a full scale squared.
const SQUARE_UNIT: f64 = 1_099_511_627_776.0;

/// The square of `x`, in whole units of [`SQUARE_UNIT`]: exact for any sample, the loudest
/// (above +72 dBFS) kept at the largest; 0 for a sample that is not a number.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a float to an integer saturates, and a square is not negative"
)]
fn square(x: f32) -> u64 {
    let x = finite(x);
    (x * x * SQUARE_UNIT) as u64
}

/// The gain of amplitude of `db`, in `f32`: what a ramp holds.
#[allow(clippy::cast_possible_truncation, reason = "a gain of 0 to −30 dB")]
fn amplitude(db: f32) -> f32 {
    gain_of(f64::from(db)) as f32
}

/// The decibels of a gain of amplitude, at least the floor of the meters.
#[allow(clippy::cast_possible_truncation, reason = "a level")]
fn meter_db(gain: f64) -> f32 {
    if gain > 0.0 && gain.is_finite() {
        ((20.0 * gain.log10()) as f32).max(METER_FLOOR_DB)
    } else {
        METER_FLOOR_DB
    }
}

/// Ducker, a [`Node`] of the latency of its lookahead, keyed by a sidechain of 0 to 24 channels.
#[derive(Debug)]
pub struct Ducker {
    rate: u32,
    channels: usize,
    sidechain: usize,
    on: bool,
    amount_db: f32,
    rise_ms: f32,
    hold_ms: f32,
    recovery_ms: f32,
    /// The Ducker on, from 0 to 1.
    wet: Smoothed,
    /// The gain of −Amount.
    depth: Smoothed,
    /// The lookahead, in ms as made and in samples.
    lookahead_ms: f32,
    lookahead: usize,
    /// The track over the lookahead, a channel after another, and the slot of the sample that
    /// leaves.
    delay: Box<[f32]>,
    at: usize,
    /// The window of the RMS, in samples; the squares over it, a channel of the sidechain after
    /// another, and the slot of the oldest; the sum of each channel's.
    window: usize,
    squares: Box<[u64]>,
    squares_at: usize,
    sums: [u128; MAX_CHANNELS],
    /// The sums of a window at the threshold, and at the threshold less the hysteresis.
    on_sum: f64,
    off_sum: f64,
    /// The sources sound; the samples since their level was last at the threshold, and the
    /// samples for which the hysteresis may hold them as sounding after it.
    loud: bool,
    since_on: u64,
    latch: u64,
    /// The samples since the sources last sounded, and whether it still holds: until more of them
    /// than the hold have gone by, the hold as it is at each sample.
    quiet: u64,
    holding: bool,
    /// How far it is ducked, from 0 to 1.
    level: f64,
    meter: SeqWriter<Meter>,
    meter_reader: SeqReader<Meter>,
}

impl Ducker {
    /// A Ducker of `settings`, at once with no ramp.
    #[must_use]
    pub fn new(settings: &Settings) -> Self {
        let value = |param: Param| settings.clamped(param);
        let (meter, meter_reader) = seqlock((METER_FLOOR_DB, METER_FLOOR_DB, 0.0));
        Ducker {
            rate: 0,
            channels: 0,
            sidechain: 0,
            on: settings.on,
            amount_db: value(Param::AmountDb),
            rise_ms: value(Param::RiseMs),
            hold_ms: value(Param::HoldMs),
            recovery_ms: value(Param::RecoveryMs),
            wet: Smoothed::new(f32::from(u8::from(settings.on)), Domain::Linear),
            depth: Smoothed::new(amplitude(-value(Param::AmountDb)), Domain::Linear),
            lookahead_ms: value(Param::LookaheadMs),
            lookahead: 0,
            delay: Box::default(),
            at: 0,
            window: 1,
            squares: Box::default(),
            squares_at: 0,
            sums: [0; MAX_CHANNELS],
            on_sum: 0.0,
            off_sum: 0.0,
            loud: false,
            since_on: 0,
            latch: 0,
            quiet: 0,
            holding: false,
            level: 0.0,
            meter,
            meter_reader,
        }
    }

    /// The meters of the window, after the last block.
    #[must_use]
    pub fn meter(&self) -> SeqReader<Meter> {
        self.meter_reader.clone()
    }

    /// The amount (dB, where its ramp goes), the rise, the hold and the recovery (ms) as set.
    #[must_use]
    pub fn heard(&self) -> (f32, f32, f32, f32) {
        (self.amount_db, self.rise_ms, self.hold_ms, self.recovery_ms)
    }

    /// The settings of a running Ducker: what it is set to, with its lookahead as made.
    fn settings(&self) -> Settings {
        Settings {
            on: self.on,
            amount_db: self.amount_db,
            lookahead_ms: self.lookahead_ms,
            rise_ms: self.rise_ms,
            hold_ms: self.hold_ms,
            recovery_ms: self.recovery_ms,
        }
    }

    /// Hears the sample `n` of the sidechain, `present` or silent: whether the sources sound,
    /// and the largest sum of a window.
    fn hear(&mut self, sidechain: &[&[f32]], present: bool, n: usize) -> u128 {
        let window = self.window;
        let mut largest = 0;
        for (channel, sum) in self.sums.iter_mut().enumerate().take(self.sidechain) {
            let x = if present { sidechain[channel][n] } else { 0.0 };
            let slot = &mut self.squares[channel * window + self.squares_at];
            *sum -= u128::from(*slot);
            *slot = square(x);
            *sum += u128::from(*slot);
            largest = largest.max(*sum);
        }
        self.squares_at = (self.squares_at + 1) % window;
        #[allow(clippy::cast_precision_loss, reason = "compared with a threshold")]
        let sum = largest as f64;
        if sum >= self.on_sum {
            self.loud = true;
            self.since_on = 0;
        } else {
            self.since_on = self.since_on.saturating_add(1);
            if sum < self.off_sum || self.since_on > self.latch {
                self.loud = false;
            }
        }
        largest
    }
}

impl Default for Ducker {
    fn default() -> Self {
        Ducker::new(&Settings::default())
    }
}

/// The value at `n` of the lane of `param`, kept in its range, or `set`.
fn automated(values: Option<&[f32]>, n: usize, param: Param, set: f32) -> f32 {
    lane(values, n)
        .and_then(|value| param.info().clamp(value))
        .unwrap_or(set)
}

impl Node for Ducker {
    fn prepare(&mut self, spec: &Spec) -> Result<(), Unsupported> {
        if spec.layout != spec.output
            || spec.layout == Layout::Empty
            || spec.rate == 0
            || spec.sidechain > MAX_CHANNELS
        {
            return Err(Unsupported::from(spec));
        }
        self.rate = spec.rate;
        self.channels = spec.inputs();
        self.sidechain = spec.sidechain;
        self.wet.prepare(spec.rate);
        self.depth.prepare(spec.rate);
        self.lookahead = samples_of(self.lookahead_ms, spec.rate);
        self.delay = vec![0.0; self.channels * self.lookahead].into_boxed_slice(); // rt-ok: when the graph is built
        self.window = samples(DUCKER_RMS_WINDOW, spec.rate).max(1);
        self.squares = vec![0; self.sidechain * self.window].into_boxed_slice(); // rt-ok: when the graph is built
        self.latch = u64::try_from(samples(DUCKER_HYSTERESIS_LATCH, spec.rate)).unwrap_or(u64::MAX);
        #[allow(
            clippy::cast_precision_loss,
            reason = "a window of a few hundred samples"
        )]
        let full = self.window as f64 * SQUARE_UNIT;
        let power = |db: f32| 10_f64.powf(f64::from(db) / 10.0);
        self.on_sum = full * power(DUCKER_THRESHOLD_DBFS);
        self.off_sum = full * power(DUCKER_THRESHOLD_DBFS - DUCKER_HYSTERESIS_DB);
        self.reset();
        Ok(())
    }

    #[allow(clippy::cast_possible_truncation, reason = "the samples are f32")]
    fn process(&mut self, ctx: &Block, input: &[&[f32]], output: &mut [&mut [f32]]) {
        let automation = &ctx.automation;
        let [on_lane, amount_lane, rise_lane, hold_lane, recovery_lane] = [
            Param::On,
            Param::AmountDb,
            Param::RiseMs,
            Param::HoldMs,
            Param::RecoveryMs,
        ]
        .map(|param| automation.get(param.id()));
        let present = self.sidechain > 0 && ctx.sidechain.len() >= self.sidechain;
        let per_ms = f64::from(self.rate) / 1000.0;
        let mut input_peak = 0.0_f64;
        let mut lowest = 1.0_f64;
        let mut largest = 0;
        for n in 0..ctx.frames {
            let on = lane(on_lane, n).map_or(self.on, |on| on >= 0.5);
            self.wet.set(f32::from(u8::from(on)));
            let wet = f64::from(self.wet.next_value());
            let depth = self.depth.next_value();
            let depth = lane(amount_lane, n)
                .and_then(|db| Param::AmountDb.info().clamp(db))
                .map_or(depth, |db| amplitude(-db));

            largest = self.hear(ctx.sidechain, present, n);
            let hold = automated(hold_lane, n, Param::HoldMs, self.hold_ms);
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a hold of at most 4 s"
            )]
            let rise = automated(rise_lane, n, Param::RiseMs, self.rise_ms);
            let rise = f64::from(rise) * per_ms;
            // At least the rise: every duck goes all the way down.
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a rise of at most 1 s"
            )]
            let hold = ((f64::from(hold) * per_ms).round() as u64).max(rise.ceil() as u64);
            if self.loud {
                self.quiet = 0;
                self.holding = true;
            } else {
                self.quiet = self.quiet.saturating_add(1);
                if self.quiet > hold {
                    self.holding = false;
                }
            }
            self.level = if self.loud || self.holding {
                (self.level + 1.0 / rise).min(1.0)
            } else {
                let recovery = automated(recovery_lane, n, Param::RecoveryMs, self.recovery_ms);
                (self.level - 1.0 / (f64::from(recovery) * per_ms)).max(0.0)
            };
            let ducked = if self.level > 0.0 {
                f64::from(depth).powf(self.level)
            } else {
                1.0
            };
            let gain = mix(1.0, ducked, wet);
            lowest = lowest.min(gain);

            let at = self.at;
            for (channel, (output, input)) in output.iter_mut().zip(input).enumerate() {
                let x = input[n];
                input_peak = input_peak.max(finite(x).abs());
                let leaving = if self.lookahead == 0 {
                    x
                } else {
                    let slot = &mut self.delay[channel * self.lookahead + at];
                    std::mem::replace(slot, x)
                };
                output[n] = (f64::from(leaving) * gain) as f32;
            }
            if self.lookahead > 0 {
                self.at = (at + 1) % self.lookahead;
            }
        }
        if ctx.frames > 0 {
            #[allow(clippy::cast_precision_loss, reason = "a level")]
            let source = largest as f64 / SQUARE_UNIT / self.window as f64;
            self.meter.write((
                meter_db(input_peak),
                meter_db(source.sqrt()),
                meter_db(lowest).min(0.0),
            ));
        }
    }

    fn latency(&self) -> u32 {
        u32::try_from(self.lookahead).unwrap_or(u32::MAX)
    }

    fn settle(&self) -> u32 {
        settle_for(&self.settings(), self.rate)
    }

    fn reset(&mut self) {
        self.wet.jump(f32::from(u8::from(self.on)));
        self.depth.finish();
        self.delay.fill(0.0);
        self.at = 0;
        self.squares.fill(0);
        self.squares_at = 0;
        self.sums = [0; MAX_CHANNELS];
        self.loud = false;
        self.since_on = 0;
        self.quiet = 0;
        self.holding = false;
        self.level = 0.0;
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
            Param::AmountDb => {
                self.amount_db = value;
                self.depth.set(amplitude(-value));
            }
            Param::LookaheadMs => {}
            Param::RiseMs => self.rise_ms = value,
            Param::HoldMs => self.hold_ms = value,
            Param::RecoveryMs => self.recovery_ms = value,
        }
    }
}
