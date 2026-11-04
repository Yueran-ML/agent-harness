//! The Dynamics of a track or a bus strip (plan T8; Reference Manual 20, pp. 3861–3866; the
//! interaction reference §8 and C-D1–C-D7): an Expander/Gate, a Compressor with a sidechain and
//! a Limiter, one after the other, each on or off, and a make-up gain after them.
//!
//! # How the gain is made
//!
//! Feed-forward, in decibels (D. Giannoulis, M. Massberg, J. D. Reiss, "Digital Dynamic Range
//! Compressor Design — A Tutorial and Analysis", JAES 60(6), 2012). For each sample:
//!
//! 1. The level `L` is the peak of the sample over the channels, the LFE left out, in dBFS
//!    ([`DYNAMICS_LEVEL_FLOOR_DB`] for silence). One gain is worked out from it and put on every
//!    channel, the LFE too: the channels are linked, so that the image does not move.
//! 2. Each module turns the level that reaches it into a gain by its static curve
//!    ([`expander_gain_db`], [`gate_gain_db`], [`compressor_gain_db`], [`limiter_gain_db`]), and
//!    its detector follows that gain in time (below). The Compressor hears `L` plus the gain of
//!    the Expander/Gate, or, keyed by its sidechain, the peak of the sidechain; the Limiter hears
//!    `L` plus the gains of both before it.
//! 3. The Compressor's Mix puts the sound before it back: its gain is 20·log10(m + (1 − m)·g)
//!    for a Mix of `100·m` percent, 0 being all compressed (p. 3864).
//! 4. The gains add up in decibels, the Make Up Gain multiplies, and the gain goes on the input.
//!
//! # The detector
//!
//! Each module follows its gain in the direction that the module acts in, its *attack*: down for
//! the Compressor and the Limiter (more reduction), up for the Expander/Gate (it opens). Its Hold
//! is a window: the static gain `t` is that of the loudest level of the last `H + 1` samples
//! (the static curves rise with the level, so that this is the largest gain of the window, turned
//! so that the attack is up), a function of those samples alone, so that it keeps no phase of
//! its own. The detector keeps a held gain `h` and its output `y`:
//!
//! ```text
//! h = t                     if t ≥ h      (a new peak, or one held in the window)
//! h = t + αR·(h − t)        otherwise     (Release)
//! y = h + αA·(y − h)        if h ≥ y      (Attack)
//! y = h                     otherwise
//! ```
//!
//! with α = exp(−1 / (τ·rate)): a step of the gain is gone 63 % of the way (1 − 1/e,
//! [`crate::constants::DYNAMICS_TIME_CONSTANT_FRACTION`]) in the attack or release time set,
//! the release counted after the hold. The peak is held through the dips of a wave, so that a
//! steady tone is made steady with little ripple. For the Expander/Gate, Hold keeps the gate
//! open after the sound falls below the threshold (the reference's reading of p. 3863, §16
//! item 6). The windows take their memory in `prepare`, for the longest hold (4 s).
//!
//! # Changes while the sound plays
//!
//! The numbers (thresholds, ratios, the knee, the range, the mix, the times, the make-up) ramp
//! over 10 ms ([`Smoothed`]). The switches (Dynamics, each module, Expander or Gate, the
//! sidechain, Listen) cross-fade over 10 ms: a module's gain is multiplied by its weight in
//! decibels, Dynamics' by its weight in the linear gain, Listen's between the output and the
//! sidechain. A module that is off and whose weight has come to 0 sleeps: its detector stops and
//! its window forgets. It wakes, when it is turned on again, where its partner is (Expander and
//! Gate, the Compressor keyed by its own sound and by the sidechain), or else at the static gain
//! of a peak of what it hears that falls 400 dB a second
//! ([`DYNAMICS_WAKE_FALL_DB_PER_SECOND`]), which bridges the dips of a wave: so that a gate
//! turned on in a loud passage does not first shut. All of it is a function of the sound of the
//! last second, the same in a node started long ago as in a node just started. Every parameter
//! but Listen may be automated (T14); a lane's NaN keeps the value set.
//!
//! # Listen
//!
//! With Listen on, the sidechain is let out in place of the sound (p. 3866), channel by channel
//! when they are as many, else their mean on every channel. Listen is for the monitor: the graph
//! sets it on the monitored path only, never in an export, and no lane sets it
//! ([`Param::is_automatable`]).
//!
//! # Bad input
//!
//! A level is heard within −200 and +200 dBFS, so that an infinite sample does not stick; a
//! block whose sidechain is missing (or shorter than the spec's) is heard as one with none.

use crate::constants::{
    samples, DYNAMICS_LEVEL_CEILING_DB, DYNAMICS_LEVEL_FLOOR_DB, DYNAMICS_SETTLE_TIME_CONSTANTS,
    DYNAMICS_WAKE_FALL_DB_PER_SECOND, MAX_CHANNELS, SMOOTHING,
};
use crate::node::{Block, Layout, Node, ParamId, Spec, Unsupported};
use crate::rt::{flush, seqlock, Domain, SeqReader, SeqWriter, Smoothed, Words};

// --- Parameters ------------------------------------------------------------------------------

/// A parameter of [`Dynamics`], set by [`Node::set`] with its [`Param::id`], in the units of the
/// model's `Dynamics` (`columbina-model`, `audio/fx.rs`): decibels, ratios, percent,
/// milliseconds, and switches as 0 or 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Param {
    /// Dynamics on (p. 3862).
    On,
    /// The Expander/Gate on (p. 3863).
    ExpanderOn,
    /// Expander (0) or Gate (1).
    ExpanderMode,
    /// The threshold of the Expander/Gate, dB, −50 to 0.
    ExpanderThresholdDb,
    /// The most it lowers, dB, 0 to 60.2.
    ExpanderRangeDb,
    /// The ratio of the Expander, 1:1.1 to 1:3 written 1.1 to 3.
    ExpanderRatio,
    /// Its attack, ms, 0 to 100: how fast it opens.
    ExpanderAttackMs,
    /// Its hold, ms, 0 to 4000: how long it stays open after the sound falls below.
    ExpanderHoldMs,
    /// Its release, ms, 50 to 4000: how fast it shuts.
    ExpanderReleaseMs,
    /// The Compressor on (p. 3864).
    CompressorOn,
    /// Its threshold, dB, −50 to 0.
    CompressorThresholdDb,
    /// Its ratio, 1:1 to 10:1 written 1 to 10.
    CompressorRatio,
    /// The width of its knee, dB, 0 to 20.
    CompressorKneeDb,
    /// Its Mix, percent, 0 (all compressed) to 100 (none).
    CompressorMix,
    /// Its attack, ms, 0 to 100.
    CompressorAttackMs,
    /// Its hold, ms, 0 to 4000.
    CompressorHoldMs,
    /// Its release, ms, 50 to 4000.
    CompressorReleaseMs,
    /// Its sidechain on: keyed by the sidechain, when there is one (pp. 3865–3866).
    SidechainOn,
    /// Listen to the sidechain.
    SidechainListen,
    /// The Limiter on (p. 3866).
    LimiterOn,
    /// Its threshold, the most it lets out, dB, −50 to 0.
    LimiterThresholdDb,
    /// Its attack, ms, 0 to 100.
    LimiterAttackMs,
    /// Its hold, ms, 0 to 4000.
    LimiterHoldMs,
    /// Its release, ms, 50 to 4000.
    LimiterReleaseMs,
    /// The Make Up Gain after it all, dB, −20 to 20.
    MakeupDb,
}

/// The count of the parameters.
pub const PARAMS: usize = 25;

impl Param {
    /// Every parameter, in the order of their ids.
    pub const ALL: [Param; PARAMS] = [
        Param::On,
        Param::ExpanderOn,
        Param::ExpanderMode,
        Param::ExpanderThresholdDb,
        Param::ExpanderRangeDb,
        Param::ExpanderRatio,
        Param::ExpanderAttackMs,
        Param::ExpanderHoldMs,
        Param::ExpanderReleaseMs,
        Param::CompressorOn,
        Param::CompressorThresholdDb,
        Param::CompressorRatio,
        Param::CompressorKneeDb,
        Param::CompressorMix,
        Param::CompressorAttackMs,
        Param::CompressorHoldMs,
        Param::CompressorReleaseMs,
        Param::SidechainOn,
        Param::SidechainListen,
        Param::LimiterOn,
        Param::LimiterThresholdDb,
        Param::LimiterAttackMs,
        Param::LimiterHoldMs,
        Param::LimiterReleaseMs,
        Param::MakeupDb,
    ];

    /// Its id for [`Node::set`] and the lanes of automation: its place in [`Param::ALL`].
    #[must_use]
    #[allow(clippy::cast_possible_truncation, reason = "25 parameters")]
    pub const fn id(self) -> ParamId {
        ParamId(self as u32)
    }

    /// The parameter of `id`.
    #[must_use]
    pub fn from_id(id: ParamId) -> Option<Param> {
        Param::ALL.get(usize::try_from(id.0).ok()?).copied()
    }

    /// Where it is in the model's `Dynamics`, as a path of its fields.
    #[must_use]
    pub const fn path(self) -> &'static str {
        match self {
            Param::On => "on",
            Param::ExpanderOn => "expander.on",
            Param::ExpanderMode => "expander.mode",
            Param::ExpanderThresholdDb => "expander.threshold_db",
            Param::ExpanderRangeDb => "expander.range_db",
            Param::ExpanderRatio => "expander.ratio",
            Param::ExpanderAttackMs => "expander.attack_ms",
            Param::ExpanderHoldMs => "expander.hold_ms",
            Param::ExpanderReleaseMs => "expander.release_ms",
            Param::CompressorOn => "compressor.on",
            Param::CompressorThresholdDb => "compressor.threshold_db",
            Param::CompressorRatio => "compressor.ratio",
            Param::CompressorKneeDb => "compressor.knee_db",
            Param::CompressorMix => "compressor.mix",
            Param::CompressorAttackMs => "compressor.attack_ms",
            Param::CompressorHoldMs => "compressor.hold_ms",
            Param::CompressorReleaseMs => "compressor.release_ms",
            Param::SidechainOn => "compressor.sidechain.on",
            Param::SidechainListen => "compressor.sidechain.listen",
            Param::LimiterOn => "limiter.on",
            Param::LimiterThresholdDb => "limiter.threshold_db",
            Param::LimiterAttackMs => "limiter.attack_ms",
            Param::LimiterHoldMs => "limiter.hold_ms",
            Param::LimiterReleaseMs => "limiter.release_ms",
            Param::MakeupDb => "makeup_db",
        }
    }

    /// Whether a lane of automation may set it: all but Listen, which is for the monitor and
    /// never in an export.
    #[must_use]
    pub const fn is_automatable(self) -> bool {
        !matches!(self, Param::SidechainListen)
    }

    /// Whether it is a switch, on at 0.5 and above.
    #[must_use]
    pub const fn is_switch(self) -> bool {
        matches!(
            self,
            Param::On
                | Param::ExpanderOn
                | Param::ExpanderMode
                | Param::CompressorOn
                | Param::SidechainOn
                | Param::SidechainListen
                | Param::LimiterOn
        )
    }

    /// Its range, to which a value set is clamped (p. 3863–3866; C-D1, C-D2, C-D4).
    #[must_use]
    pub const fn range(self) -> (f32, f32) {
        match self {
            Param::ExpanderThresholdDb
            | Param::CompressorThresholdDb
            | Param::LimiterThresholdDb => (-50.0, 0.0),
            Param::ExpanderRangeDb => (0.0, 60.2),
            Param::ExpanderRatio => (1.1, 3.0),
            Param::ExpanderAttackMs
            | Param::CompressorAttackMs
            | Param::LimiterAttackMs
            | Param::CompressorMix => (0.0, 100.0),
            Param::ExpanderHoldMs | Param::CompressorHoldMs | Param::LimiterHoldMs => (0.0, 4000.0),
            Param::ExpanderReleaseMs | Param::CompressorReleaseMs | Param::LimiterReleaseMs => {
                (50.0, 4000.0)
            }
            Param::CompressorRatio => (1.0, 10.0),
            Param::CompressorKneeDb => (0.0, 20.0),
            Param::MakeupDb => (-20.0, 20.0),
            Param::On
            | Param::ExpanderOn
            | Param::ExpanderMode
            | Param::CompressorOn
            | Param::SidechainOn
            | Param::SidechainListen
            | Param::LimiterOn => (0.0, 1.0),
        }
    }

    /// `value` clamped to the range; a switch as 0 or 1.
    #[must_use]
    pub fn clamp(self, value: f32) -> f32 {
        if self.is_switch() {
            if value >= 0.5 {
                1.0
            } else {
                0.0
            }
        } else {
            let (low, high) = self.range();
            value.clamp(low, high)
        }
    }
}

/// The Expander/Gate of [`Settings`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExpanderSettings {
    /// On.
    pub on: bool,
    /// Gating, not expanding.
    pub gate: bool,
    /// dB.
    pub threshold_db: f32,
    /// dB.
    pub range_db: f32,
    /// 1.1 to 3.
    pub ratio: f32,
    /// ms.
    pub attack_ms: f32,
    /// ms.
    pub hold_ms: f32,
    /// ms.
    pub release_ms: f32,
}

/// The Compressor of [`Settings`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompressorSettings {
    /// On.
    pub on: bool,
    /// dB.
    pub threshold_db: f32,
    /// 1 to 10.
    pub ratio: f32,
    /// dB.
    pub knee_db: f32,
    /// Percent, 0 all compressed.
    pub mix: f32,
    /// ms.
    pub attack_ms: f32,
    /// ms.
    pub hold_ms: f32,
    /// ms.
    pub release_ms: f32,
    /// Keyed by the sidechain.
    pub sidechain: bool,
    /// Listen to the sidechain.
    pub listen: bool,
}

/// The Limiter of [`Settings`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LimiterSettings {
    /// On.
    pub on: bool,
    /// dB.
    pub threshold_db: f32,
    /// ms.
    pub attack_ms: f32,
    /// ms.
    pub hold_ms: f32,
    /// ms.
    pub release_ms: f32,
}

/// Every parameter of [`Dynamics`] at once, as the model's `Dynamics` holds them; the default is
/// a new strip's (p. 3863–3866; C-D1–C-D7): everything off.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Dynamics on.
    pub on: bool,
    /// The Expander/Gate.
    pub expander: ExpanderSettings,
    /// The Compressor.
    pub compressor: CompressorSettings,
    /// The Limiter.
    pub limiter: LimiterSettings,
    /// The Make Up Gain, dB.
    pub makeup_db: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            on: false,
            expander: ExpanderSettings {
                on: false,
                gate: false,
                threshold_db: -35.0,
                range_db: 18.0,
                ratio: 1.1,
                attack_ms: 1.4,
                hold_ms: 0.0,
                release_ms: 93.0,
            },
            compressor: CompressorSettings {
                on: false,
                threshold_db: -15.0,
                ratio: 2.0,
                knee_db: 0.0,
                mix: 0.0,
                attack_ms: 1.4,
                hold_ms: 0.0,
                release_ms: 93.0,
                sidechain: false,
                listen: false,
            },
            limiter: LimiterSettings {
                on: false,
                threshold_db: -21.0,
                attack_ms: 0.71,
                hold_ms: 0.0,
                release_ms: 90.0,
            },
            makeup_db: 0.0,
        }
    }
}

fn switch(on: bool) -> f32 {
    if on {
        1.0
    } else {
        0.0
    }
}

impl Settings {
    /// The value of `param`, in the units of [`Node::set`].
    #[must_use]
    pub fn value(&self, param: Param) -> f32 {
        let (e, c, l) = (&self.expander, &self.compressor, &self.limiter);
        match param {
            Param::On => switch(self.on),
            Param::ExpanderOn => switch(e.on),
            Param::ExpanderMode => switch(e.gate),
            Param::ExpanderThresholdDb => e.threshold_db,
            Param::ExpanderRangeDb => e.range_db,
            Param::ExpanderRatio => e.ratio,
            Param::ExpanderAttackMs => e.attack_ms,
            Param::ExpanderHoldMs => e.hold_ms,
            Param::ExpanderReleaseMs => e.release_ms,
            Param::CompressorOn => switch(c.on),
            Param::CompressorThresholdDb => c.threshold_db,
            Param::CompressorRatio => c.ratio,
            Param::CompressorKneeDb => c.knee_db,
            Param::CompressorMix => c.mix,
            Param::CompressorAttackMs => c.attack_ms,
            Param::CompressorHoldMs => c.hold_ms,
            Param::CompressorReleaseMs => c.release_ms,
            Param::SidechainOn => switch(c.sidechain),
            Param::SidechainListen => switch(c.listen),
            Param::LimiterOn => switch(l.on),
            Param::LimiterThresholdDb => l.threshold_db,
            Param::LimiterAttackMs => l.attack_ms,
            Param::LimiterHoldMs => l.hold_ms,
            Param::LimiterReleaseMs => l.release_ms,
            Param::MakeupDb => self.makeup_db,
        }
    }

    /// Sets `param` to `value`, clamped to its range.
    pub fn set(&mut self, param: Param, value: f32) {
        let value = param.clamp(value);
        let on = value >= 0.5;
        let (e, c, l) = (&mut self.expander, &mut self.compressor, &mut self.limiter);
        match param {
            Param::On => self.on = on,
            Param::ExpanderOn => e.on = on,
            Param::ExpanderMode => e.gate = on,
            Param::ExpanderThresholdDb => e.threshold_db = value,
            Param::ExpanderRangeDb => e.range_db = value,
            Param::ExpanderRatio => e.ratio = value,
            Param::ExpanderAttackMs => e.attack_ms = value,
            Param::ExpanderHoldMs => e.hold_ms = value,
            Param::ExpanderReleaseMs => e.release_ms = value,
            Param::CompressorOn => c.on = on,
            Param::CompressorThresholdDb => c.threshold_db = value,
            Param::CompressorRatio => c.ratio = value,
            Param::CompressorKneeDb => c.knee_db = value,
            Param::CompressorMix => c.mix = value,
            Param::CompressorAttackMs => c.attack_ms = value,
            Param::CompressorHoldMs => c.hold_ms = value,
            Param::CompressorReleaseMs => c.release_ms = value,
            Param::SidechainOn => c.sidechain = on,
            Param::SidechainListen => c.listen = on,
            Param::LimiterOn => l.on = on,
            Param::LimiterThresholdDb => l.threshold_db = value,
            Param::LimiterAttackMs => l.attack_ms = value,
            Param::LimiterHoldMs => l.hold_ms = value,
            Param::LimiterReleaseMs => l.release_ms = value,
            Param::MakeupDb => self.makeup_db = value,
        }
    }

    /// Every parameter with its value, to [`Node::set`] one by one.
    #[must_use]
    pub fn params(&self) -> [(ParamId, f32); PARAMS] {
        Param::ALL.map(|param| (param.id(), self.value(param)))
    }

    /// The same, every value clamped to its range.
    #[must_use]
    pub fn clamped(&self) -> Settings {
        let mut clamped = *self;
        for param in Param::ALL {
            clamped.set(param, self.value(param));
        }
        clamped
    }
}

// --- The static curves -----------------------------------------------------------------------

/// The gain, dB, of an Expander at `threshold` and `ratio` (1:`ratio`) for a level of `level`
/// dBFS: below the threshold the output falls `ratio` dB for each dB of the input, lowered by
/// `range` dB at most.
#[must_use]
pub fn expander_gain_db(level: f64, threshold: f64, ratio: f64, range: f64) -> f64 {
    if level >= threshold {
        0.0
    } else {
        ((level - threshold) * (ratio - 1.0)).max(-range)
    }
}

/// The gain, dB, of a Gate at `threshold` with `range`: shut by `range` below the threshold.
#[must_use]
pub fn gate_gain_db(level: f64, threshold: f64, range: f64) -> f64 {
    if level >= threshold {
        0.0
    } else {
        -range
    }
}

/// The gain, dB, of a Compressor at `threshold`, `ratio` (`ratio`:1) and a knee `knee` dB wide,
/// for a level of `level` dBFS (Giannoulis 2012, eq. 4): none below the knee, a quadratic through
/// it, and above it the output rises 1/`ratio` dB for each dB.
#[must_use]
pub fn compressor_gain_db(level: f64, threshold: f64, ratio: f64, knee: f64) -> f64 {
    let over = level - threshold;
    if knee <= 0.0 || 2.0 * over.abs() > knee {
        if over <= 0.0 {
            0.0
        } else {
            over * (1.0 / ratio - 1.0)
        }
    } else {
        let into = over + knee / 2.0;
        (1.0 / ratio - 1.0) * into * into / (2.0 * knee)
    }
}

/// The gain, dB, of a Limiter at `threshold`: down to the threshold, and no further.
#[must_use]
pub fn limiter_gain_db(level: f64, threshold: f64) -> f64 {
    (threshold - level).min(0.0)
}

/// A Compressor's gain `gain` dB with `mix` percent of the sound before it put back (p. 3864): 0
/// leaves the gain as it is, 100 makes it 0 dB.
#[must_use]
pub fn mixed_gain_db(gain: f64, mix: f64) -> f64 {
    if mix <= 0.0 {
        gain
    } else {
        let dry = (mix / 100.0).min(1.0);
        20.0 * (dry + (1.0 - dry) * db_gain(gain)).log10()
    }
}

/// The output level, dBFS, of Dynamics set as `settings` for a steady level of each of
/// `inputs_db`, as the graph of its window draws it (p. 3861): the modules that are on one after
/// the other, then the Make Up Gain; the input itself where Dynamics is off. The Compressor's
/// curve is that of its own sound, even when its sidechain keys it.
#[must_use]
pub fn curve(settings: &Settings, inputs_db: &[f32]) -> Vec<f32> {
    let s = settings.clamped();
    inputs_db
        .iter()
        .map(|&input| {
            if !s.on {
                return input;
            }
            let level = f64::from(input);
            let (e, c, l) = (&s.expander, &s.compressor, &s.limiter);
            let mut gain = 0.0;
            if e.on {
                gain += if e.gate {
                    gate_gain_db(level, e.threshold_db.into(), e.range_db.into())
                } else {
                    expander_gain_db(
                        level,
                        e.threshold_db.into(),
                        e.ratio.into(),
                        e.range_db.into(),
                    )
                };
            }
            if c.on {
                let compressed = compressor_gain_db(
                    level + gain,
                    c.threshold_db.into(),
                    c.ratio.into(),
                    c.knee_db.into(),
                );
                gain += mixed_gain_db(compressed, c.mix.into());
            }
            if l.on {
                gain += limiter_gain_db(level + gain, l.threshold_db.into());
            }
            #[allow(clippy::cast_possible_truncation, reason = "levels of a graph")]
            let output = (level + gain + f64::from(s.makeup_db)) as f32;
            output
        })
        .collect() // rt-ok: the curve is drawn off the thread of the mix
}

/// The gain of `db` decibels.
fn db_gain(db: f64) -> f64 {
    10.0_f64.powf(db / 20.0)
}

/// The level of a peak `peak`, dBFS, within the floor and the ceiling: silence, NaN and an
/// infinite sample are heard as a level of their own and never stick.
fn level_db(peak: f64) -> f64 {
    if peak > 0.0 {
        (20.0 * peak.log10()).clamp(DYNAMICS_LEVEL_FLOOR_DB, DYNAMICS_LEVEL_CEILING_DB)
    } else {
        DYNAMICS_LEVEL_FLOOR_DB
    }
}

// --- What the window shows -------------------------------------------------------------------

/// Where the ball of the graph of Dynamics is (p. 3861), and the gain of each module, at the
/// loudest sample of the last block: the level in and out, dBFS, and the gains, dB. Written by
/// the thread of the mix, read by any ([`Dynamics::ball`]). Before any sound, both levels are
/// the floor, −200 dBFS.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ball {
    /// The level that Dynamics hears.
    pub input_db: f32,
    /// The level it lets out: what it outputs, the sidechain while it is listened to.
    pub output_db: f32,
    /// The gain of the Expander/Gate.
    pub expander_db: f32,
    /// The gain of the Compressor.
    pub compressor_db: f32,
    /// The gain of the Limiter.
    pub limiter_db: f32,
}

impl Default for Ball {
    #[allow(clippy::cast_possible_truncation, reason = "−200 is an f32")]
    fn default() -> Self {
        Ball {
            input_db: DYNAMICS_LEVEL_FLOOR_DB as f32,
            output_db: DYNAMICS_LEVEL_FLOOR_DB as f32,
            expander_db: 0.0,
            compressor_db: 0.0,
            limiter_db: 0.0,
        }
    }
}

impl Words for Ball {
    const WORDS: usize = 5;

    fn word(&self, index: usize) -> u32 {
        [
            self.input_db,
            self.output_db,
            self.expander_db,
            self.compressor_db,
            self.limiter_db,
        ][index]
            .to_bits()
    }

    fn from_words(word: impl Fn(usize) -> u32) -> Self {
        Ball {
            input_db: f32::from_bits(word(0)),
            output_db: f32::from_bits(word(1)),
            expander_db: f32::from_bits(word(2)),
            compressor_db: f32::from_bits(word(3)),
            limiter_db: f32::from_bits(word(4)),
        }
    }
}

// --- The node --------------------------------------------------------------------------------

/// A switch and its weight, cross-faded over 10 ms.
#[derive(Clone, Copy, Debug, Default)]
struct Switch {
    on: bool,
    weight: f64,
}

/// `weight` moved toward 1 or 0 by `step`, landing on it.
fn follow(weight: f64, on: bool, step: f64) -> f64 {
    let target = if on { 1.0 } else { 0.0 };
    let gap = target - weight;
    if gap.abs() <= step {
        target
    } else {
        weight + step.copysign(gap)
    }
}

/// The coefficient of a one-pole smoother of a time in ms, kept until the time changes.
#[derive(Clone, Copy, Debug, Default)]
struct Coefficient {
    ms: u32,
    alpha: f64,
    known: bool,
}

impl Coefficient {
    fn get(&mut self, ms: f32, rate: u32) -> f64 {
        if !self.known || self.ms != ms.to_bits() {
            self.ms = ms.to_bits();
            self.known = true;
            self.alpha = if ms > 0.0 && rate > 0 {
                (-1000.0 / (f64::from(ms) * f64::from(rate))).exp()
            } else {
                0.0
            };
        }
        self.alpha
    }
}

/// The loudest of the levels of the last `hold + 1` samples, this one with them: the Hold of a
/// module, a function of those samples alone, whatever came before them (a monotonic deque,
/// each sample pushed and popped once). Its memory is taken in `prepare` for the longest hold.
#[derive(Clone, Debug, Default)]
struct Window {
    /// A ring of (the sample, its level), the levels falling from the front.
    entries: Vec<(u32, f32)>,
    head: usize,
    len: usize,
    now: u32,
}

impl Window {
    /// Room for a hold of `hold` samples.
    fn prepare(&mut self, hold: usize) {
        self.entries = vec![(0, 0.0); hold + 1]; // rt-ok: taken in prepare
        self.clear();
    }

    /// Forgets every sample.
    fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
        self.now = 0;
    }

    /// The loudest of `level` and the levels of the `hold` samples before it.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "levels in dB, to a millionth"
    )]
    fn push(&mut self, level: f64, hold: u32) -> f64 {
        let capacity = self.entries.len();
        if capacity == 0 {
            return level;
        }
        let hold = hold.min(u32::try_from(capacity - 1).unwrap_or(u32::MAX));
        while self.len > 0 && self.now.wrapping_sub(self.entries[self.head].0) > hold {
            self.head = (self.head + 1) % capacity;
            self.len -= 1;
        }
        let stored = level as f32;
        while self.len > 0 && self.entries[(self.head + self.len - 1) % capacity].1 <= stored {
            self.len -= 1;
        }
        self.entries[(self.head + self.len) % capacity] = (self.now, stored);
        self.len += 1;
        let (at, loudest) = self.entries[self.head];
        let current = at == self.now;
        self.now = self.now.wrapping_add(1);
        if current {
            level
        } else {
            f64::from(loudest)
        }
    }
}

/// The detector of a module (see the module's "The detector").
#[derive(Clone, Copy, Debug, Default)]
struct Detector {
    held: f64,
    out: f64,
    awake: bool,
    attack: Coefficient,
    release: Coefficient,
}

/// The times of a module, ms.
#[derive(Clone, Copy, Debug)]
struct Times {
    attack: f32,
    hold: f32,
    release: f32,
}

impl Detector {
    /// Stops; the next step wakes it.
    fn stop(&mut self) {
        self.awake = false;
    }

    /// Its output, if it is awake, else `otherwise`.
    fn out_or(&self, otherwise: f64) -> f64 {
        if self.awake {
            self.out
        } else {
            otherwise
        }
    }

    /// The output for `target`, the static gain of the loudest level of the hold (turned so
    /// that the attack is up), waking at `wake`.
    fn step(&mut self, target: f64, wake: f64, times: Times, rate: u32) -> f64 {
        if !self.awake {
            self.held = wake;
            self.out = wake;
            self.awake = true;
        }
        if target >= self.held {
            self.held = target;
        } else {
            let release = self.release.get(times.release, rate);
            self.held = target + flush(release * (self.held - target));
        }
        if self.held >= self.out {
            let attack = self.attack.get(times.attack, rate);
            self.out = self.held + flush(attack * (self.out - self.held));
        } else {
            self.out = self.held;
        }
        self.out
    }
}

/// The samples of a hold of `ms`.
fn hold_samples(ms: f32, rate: u32) -> u32 {
    u32::try_from(samples(f64::from(ms.max(0.0)) / 1000.0, rate)).unwrap_or(u32::MAX)
}

/// A peak that falls [`DYNAMICS_WAKE_FALL_DB_PER_SECOND`], as a linear amplitude: what a module
/// woken hears, through the dips of a wave. It forgets what it held in at most 400 dB of fall,
/// so that two followers of the same sound are the same within a second.
#[derive(Clone, Copy, Debug, Default)]
struct Follower {
    peak: f64,
    fall: f64,
    floor: f64,
    ceiling: f64,
}

impl Follower {
    fn prepare(&mut self, rate: u32) {
        self.fall = 10.0_f64.powf(-DYNAMICS_WAKE_FALL_DB_PER_SECOND / 20.0 / f64::from(rate));
        self.floor = 10.0_f64.powf(DYNAMICS_LEVEL_FLOOR_DB / 20.0);
        self.ceiling = 10.0_f64.powf(DYNAMICS_LEVEL_CEILING_DB / 20.0);
        self.peak = 0.0;
    }

    fn push(&mut self, peak: f64) {
        let fallen = self.peak * self.fall;
        let fallen = if fallen <= self.floor { 0.0 } else { fallen };
        self.peak = fallen.max(peak).min(self.ceiling);
    }

    fn level(&self) -> f64 {
        level_db(self.peak)
    }
}

/// The Dynamics of a strip, a node of any layout in and the same out, with a sidechain of any
/// channels (see the module). No latency: it does not look ahead.
#[derive(Debug)]
pub struct Dynamics {
    rate: u32,
    channels: usize,
    /// The channels that it hears: all but the LFE.
    heard: [bool; MAX_CHANNELS],
    sidechain: usize,
    /// Whether the block has its sidechain: the spec's has one and the block gives it.
    keyable: bool,
    /// The numbers, by the index of their [`Param`]; the make-up as a linear gain. The switches'
    /// places are not used.
    knobs: [Smoothed; PARAMS],
    /// The switches, by the index of their [`Param`]; the numbers' places are not used.
    switches: [Switch; PARAMS],
    /// The step of a cross-fade of a switch: 1 over the samples of 10 ms.
    step: f64,
    expander: Detector,
    gate: Detector,
    own: Detector,
    keyed: Detector,
    limiter: Detector,
    /// The holds: of the level that the Expander/Gate hears, of the Compressor's own level, of
    /// the sidechain's, and of the Limiter's.
    heard_hold: Window,
    own_hold: Window,
    keyed_hold: Window,
    limiter_hold: Window,
    /// What a module woken hears, of the sound and of the sidechain.
    wake: Follower,
    wake_keyed: Follower,
    ball: SeqWriter<Ball>,
    ball_reader: SeqReader<Ball>,
}

/// The values of the parameters at a sample: the numbers, and the switches' targets.
#[derive(Clone, Copy, Debug)]
struct Values {
    knobs: [f32; PARAMS],
    on: [bool; PARAMS],
}

impl Dynamics {
    /// Dynamics set as `settings`.
    #[must_use]
    pub fn new(settings: &Settings) -> Self {
        let settings = settings.clamped();
        let (ball, ball_reader) = seqlock(Ball::default()); // rt-ok: made off the thread of the mix
        let knobs = Param::ALL.map(|param| {
            let value = if param == Param::MakeupDb {
                crate::db_to_gain(settings.makeup_db)
            } else {
                settings.value(param)
            };
            Smoothed::new(value, Domain::Linear)
        });
        let switches = Param::ALL.map(|param| {
            let on = settings.value(param) >= 0.5;
            Switch {
                on,
                weight: if on { 1.0 } else { 0.0 },
            }
        });
        Dynamics {
            rate: 0,
            channels: 0,
            heard: [true; MAX_CHANNELS],
            sidechain: 0,
            keyable: false,
            knobs,
            switches,
            step: f64::INFINITY,
            expander: Detector::default(),
            gate: Detector::default(),
            own: Detector::default(),
            keyed: Detector::default(),
            limiter: Detector::default(),
            heard_hold: Window::default(),
            own_hold: Window::default(),
            keyed_hold: Window::default(),
            limiter_hold: Window::default(),
            wake: Follower::default(),
            wake_keyed: Follower::default(),
            ball,
            ball_reader,
        }
    }

    /// The reader of the ball and the gains, for the window (any thread).
    #[must_use]
    pub fn ball(&self) -> SeqReader<Ball> {
        self.ball_reader.clone()
    }

    /// The targets of the parameters now: what they ramp or cross-fade to.
    #[must_use]
    pub fn settings(&self) -> Settings {
        let mut settings = Settings::default();
        for param in Param::ALL {
            let index = param as usize;
            let value = if param.is_switch() {
                switch(self.switches[index].on)
            } else if param == Param::MakeupDb {
                crate::gain_to_db(self.knobs[index].target())
            } else {
                self.knobs[index].target()
            };
            settings.set(param, value);
        }
        settings
    }

    /// The settling ([`Node::settle`]) of Dynamics set as `settings`, at `rate`: the ramp of
    /// 10 ms and, when Dynamics and a module are on, the second in which what a woken module
    /// hears is forgotten, and for each module on its hold and 18 of its time constants (attack
    /// plus release, [`DYNAMICS_SETTLE_TIME_CONSTANTS`]), the modules one after the other, as
    /// each hears the gain of those before it.
    ///
    /// [`Node::settle`] gives it for the parameters as they are set (the longer of a time and
    /// the time it ramps to). Lanes of automation are not set: the graph (T4b) asks for the
    /// settling of the slowest settings that the lanes take in the window of the pre-roll (each
    /// switch on that is on anywhere in it, each time at its longest).
    #[must_use]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a count of samples"
    )]
    pub fn settle_for(settings: &Settings, rate: u32) -> u32 {
        let s = settings.clamped();
        let rate = f64::from(rate);
        let ramp = (SMOOTHING * rate).round();
        if !s.on {
            return ramp as u32;
        }
        let module = |on: bool, attack: f32, hold: f32, release: f32| {
            if on {
                (DYNAMICS_SETTLE_TIME_CONSTANTS * f64::from(attack + release) + f64::from(hold))
                    * rate
                    / 1000.0
            } else {
                0.0
            }
        };
        let (e, c, l) = (&s.expander, &s.compressor, &s.limiter);
        let wake = if e.on || c.on || l.on {
            (DYNAMICS_LEVEL_CEILING_DB - DYNAMICS_LEVEL_FLOOR_DB) / DYNAMICS_WAKE_FALL_DB_PER_SECOND
                * rate
        } else {
            0.0
        };
        let total = ramp
            + wake
            + module(e.on, e.attack_ms, e.hold_ms, e.release_ms)
            + module(c.on, c.attack_ms, c.hold_ms, c.release_ms)
            + module(l.on, l.attack_ms, l.hold_ms, l.release_ms);
        total.ceil().min(f64::from(u32::MAX)) as u32
    }

    /// What each switch goes to, `raw` being what is set or automated: the sidechain only where
    /// the block has one, and Listen only where the sidechain keys.
    fn targets(&self, mut raw: [bool; PARAMS]) -> [bool; PARAMS] {
        raw[Param::SidechainOn as usize] &= self.keyable;
        raw[Param::SidechainListen as usize] &= raw[Param::SidechainOn as usize];
        raw
    }

    /// The switches as they are set.
    fn set_switches(&self) -> [bool; PARAMS] {
        self.switches.map(|switch| switch.on)
    }

    /// Whether nothing moves through the block and Dynamics is off and not listened to: the
    /// output is the input.
    fn idle(&self, lanes: &[Option<&[f32]>; PARAMS]) -> bool {
        let targets = self.targets(self.set_switches());
        lanes.iter().all(Option::is_none)
            && self.knobs.iter().all(|knob| !knob.is_ramping())
            && self
                .switches
                .iter()
                .zip(targets)
                .all(|(switch, on)| switch.weight.to_bits() == f64::from(u8::from(on)).to_bits())
            && !targets[Param::On as usize]
            && !targets[Param::SidechainListen as usize]
    }

    /// The values at sample `n` of the block, the ramps moved on by one. A lane's NaN keeps the
    /// value set.
    fn values(&mut self, lanes: &[Option<&[f32]>; PARAMS], n: usize) -> Values {
        let mut values = Values {
            knobs: [0.0; PARAMS],
            on: [false; PARAMS],
        };
        for (index, param) in Param::ALL.into_iter().enumerate() {
            let lane = lanes[index]
                .map(|lane| lane[n])
                .filter(|value| !value.is_nan());
            if param.is_switch() {
                values.on[index] = lane.map_or(self.switches[index].on, |value| value >= 0.5);
            } else {
                let smoothed = self.knobs[index].next_value();
                values.knobs[index] = match lane {
                    None => smoothed,
                    Some(value) if param == Param::MakeupDb => {
                        crate::db_to_gain(param.clamp(value))
                    }
                    Some(value) => param.clamp(value),
                };
            }
        }
        values.on = self.targets(values.on);
        for (switch, &on) in self.switches.iter_mut().zip(&values.on) {
            switch.weight = follow(switch.weight, on, self.step);
        }
        values
    }

    /// Whether the block has its sidechain, of `given` channels: one shorter than the spec's is
    /// heard as none. Whether the graph gives the sidechain is not a parameter: the sidechain and
    /// Listen go at once to what it allows.
    fn take_sidechain(&mut self, given: usize) {
        debug_assert!(
            given == 0 || given >= self.sidechain,
            "a sidechain of {given} channels for a spec of {}",
            self.sidechain
        );
        let keyable = self.sidechain > 0 && given >= self.sidechain;
        if keyable != self.keyable {
            self.keyable = keyable;
            let targets = self.targets(self.set_switches());
            for param in [Param::SidechainOn, Param::SidechainListen] {
                let index = param as usize;
                self.switches[index].weight = f64::from(u8::from(targets[index]));
            }
        }
    }

    fn weight(&self, param: Param) -> f64 {
        self.switches[param as usize].weight
    }

    /// Stops every module.
    fn stop(&mut self) {
        for detector in [
            &mut self.expander,
            &mut self.gate,
            &mut self.own,
            &mut self.keyed,
            &mut self.limiter,
        ] {
            detector.stop();
        }
        for window in [
            &mut self.heard_hold,
            &mut self.own_hold,
            &mut self.keyed_hold,
            &mut self.limiter_hold,
        ] {
            window.clear();
        }
    }

    /// The gains, dB, of the three modules at a sample of level `level` and of sidechain level
    /// `side`.
    #[allow(
        clippy::too_many_lines,
        reason = "the three modules, one after the other"
    )]
    fn gains(&mut self, v: &Values, level: f64, side: f64) -> [f64; 3] {
        let knob = |param: Param| f64::from(v.knobs[param as usize]);
        let on = |param: Param| v.on[param as usize];
        let times = |attack: Param, hold: Param, release: Param| Times {
            attack: v.knobs[attack as usize],
            hold: v.knobs[hold as usize],
            release: v.knobs[release as usize],
        };
        let rate = self.rate;
        let woken = self.wake.level();

        let mut expanded = 0.0;
        if self.weight(Param::ExpanderOn) > 0.0 || on(Param::ExpanderOn) {
            let threshold = knob(Param::ExpanderThresholdDb);
            let range = knob(Param::ExpanderRangeDb);
            let ratio = knob(Param::ExpanderRatio);
            let t = times(
                Param::ExpanderAttackMs,
                Param::ExpanderHoldMs,
                Param::ExpanderReleaseMs,
            );
            let held = self.heard_hold.push(level, hold_samples(t.hold, rate));
            let gating = self.weight(Param::ExpanderMode);
            let expanding = if gating < 1.0 || !on(Param::ExpanderMode) {
                let target = expander_gain_db(held, threshold, ratio, range);
                let wake = self
                    .gate
                    .out_or(expander_gain_db(woken, threshold, ratio, range));
                self.expander.step(target, wake, t, rate)
            } else {
                self.expander.stop();
                0.0
            };
            let gate = if gating > 0.0 || on(Param::ExpanderMode) {
                let target = gate_gain_db(held, threshold, range);
                let wake = self.expander.out_or(gate_gain_db(woken, threshold, range));
                self.gate.step(target, wake, t, rate)
            } else {
                self.gate.stop();
                0.0
            };
            expanded =
                self.weight(Param::ExpanderOn) * ((1.0 - gating) * expanding + gating * gate);
        } else {
            self.expander.stop();
            self.gate.stop();
            self.heard_hold.clear();
        }

        let mut compressed = 0.0;
        if self.weight(Param::CompressorOn) > 0.0 || on(Param::CompressorOn) {
            let threshold = knob(Param::CompressorThresholdDb);
            let ratio = knob(Param::CompressorRatio);
            let knee = knob(Param::CompressorKneeDb);
            let t = times(
                Param::CompressorAttackMs,
                Param::CompressorHoldMs,
                Param::CompressorReleaseMs,
            );
            let window = hold_samples(t.hold, rate);
            let keying = self.weight(Param::SidechainOn);
            let own = if keying < 1.0 || !on(Param::SidechainOn) {
                let held = self.own_hold.push(level + expanded, window);
                let target = -compressor_gain_db(held, threshold, ratio, knee);
                let wake = self.keyed.out_or(-compressor_gain_db(
                    woken + expanded,
                    threshold,
                    ratio,
                    knee,
                ));
                self.own.step(target, wake, t, rate)
            } else {
                self.own.stop();
                self.own_hold.clear();
                0.0
            };
            let keyed = if keying > 0.0 || on(Param::SidechainOn) {
                let held = self.keyed_hold.push(side, window);
                let target = -compressor_gain_db(held, threshold, ratio, knee);
                let wake = self.own.out_or(-compressor_gain_db(
                    self.wake_keyed.level(),
                    threshold,
                    ratio,
                    knee,
                ));
                self.keyed.step(target, wake, t, rate)
            } else {
                self.keyed.stop();
                self.keyed_hold.clear();
                0.0
            };
            let reduction = (1.0 - keying) * own + keying * keyed;
            compressed = self.weight(Param::CompressorOn)
                * mixed_gain_db(-reduction, knob(Param::CompressorMix));
        } else {
            self.own.stop();
            self.keyed.stop();
            self.own_hold.clear();
            self.keyed_hold.clear();
        }

        let mut limited = 0.0;
        if self.weight(Param::LimiterOn) > 0.0 || on(Param::LimiterOn) {
            let threshold = knob(Param::LimiterThresholdDb);
            let t = times(
                Param::LimiterAttackMs,
                Param::LimiterHoldMs,
                Param::LimiterReleaseMs,
            );
            let held = self
                .limiter_hold
                .push(level + expanded + compressed, hold_samples(t.hold, rate));
            let target = -limiter_gain_db(held, threshold);
            let wake = -limiter_gain_db(woken + expanded + compressed, threshold);
            limited = -self.weight(Param::LimiterOn) * self.limiter.step(target, wake, t, rate);
        } else {
            self.limiter.stop();
            self.limiter_hold.clear();
        }
        [expanded, compressed, limited]
    }
}

impl Default for Dynamics {
    fn default() -> Self {
        Dynamics::new(&Settings::default())
    }
}

impl Node for Dynamics {
    fn prepare(&mut self, spec: &Spec) -> Result<(), Unsupported> {
        if spec.layout == Layout::Empty
            || spec.output != spec.layout
            || spec.sidechain > MAX_CHANNELS
            || spec.rate == 0
        {
            return Err(Unsupported::from(spec));
        }
        self.rate = spec.rate;
        self.channels = spec.inputs();
        self.sidechain = spec.sidechain;
        self.keyable = spec.sidechain > 0;
        self.heard = [true; MAX_CHANNELS];
        if let Some(lfe) = spec.layout.lfe() {
            self.heard[lfe] = false;
        }
        for knob in &mut self.knobs {
            knob.prepare(spec.rate);
        }
        let ramp = self.knobs[0].ramp_len();
        self.step = if ramp == 0 {
            f64::INFINITY
        } else {
            1.0 / f64::from(ramp)
        };
        let longest = samples(
            f64::from(Param::ExpanderHoldMs.range().1) / 1000.0,
            spec.rate,
        );
        self.heard_hold.prepare(longest);
        self.own_hold.prepare(longest);
        self.limiter_hold.prepare(longest);
        self.keyed_hold
            .prepare(if spec.sidechain > 0 { longest } else { 0 });
        self.wake.prepare(spec.rate);
        self.wake_keyed.prepare(spec.rate);
        self.reset();
        Ok(())
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::too_many_lines,
        reason = "samples, a mean of at most 24 channels, and a sample's work in one place"
    )]
    fn process(&mut self, ctx: &Block, input: &[&[f32]], output: &mut [&mut [f32]]) {
        let frames = ctx.frames;
        if frames == 0 {
            return;
        }
        self.take_sidechain(ctx.sidechain.len());
        let sidechain: &[&[f32]] = if self.keyable {
            &ctx.sidechain[..self.sidechain]
        } else {
            &[]
        };
        let lanes: [Option<&[f32]>; PARAMS] = std::array::from_fn(|index| {
            let param = Param::ALL[index];
            if param.is_automatable() {
                ctx.automation.get(param.id())
            } else {
                None
            }
        });
        let heard = self.heard;
        let peak_at = |n: usize| -> f64 {
            input
                .iter()
                .zip(heard)
                .filter(|(_, heard)| *heard)
                .fold(0.0_f64, |peak, (channel, _)| {
                    peak.max(f64::from(channel[n].abs()))
                })
        };
        let side_at = |n: usize| -> f64 {
            sidechain.iter().fold(0.0_f64, |peak, channel| {
                peak.max(f64::from(channel[n].abs()))
            })
        };

        if self.idle(&lanes) {
            for (out, x) in output.iter_mut().zip(input) {
                out[..frames].copy_from_slice(&x[..frames]);
            }
            let mut loudest = 0.0_f64;
            for n in 0..frames {
                let peak = peak_at(n);
                self.wake.push(peak);
                self.wake_keyed.push(side_at(n));
                loudest = loudest.max(peak);
            }
            let level = level_db(loudest) as f32;
            self.ball.write(Ball {
                input_db: level,
                output_db: level,
                ..Ball::default()
            });
            return;
        }

        let matched = sidechain.len() == self.channels;
        let mut loudest = (-1.0_f64, Ball::default());
        for n in 0..frames {
            let v = self.values(&lanes, n);
            let peak = peak_at(n);
            let side = side_at(n);
            self.wake.push(peak);
            self.wake_keyed.push(side);
            let on = self.weight(Param::On);
            let mut gain = 1.0;
            let mut gains = [0.0; 3];
            if on > 0.0 || v.on[Param::On as usize] {
                gains = self.gains(&v, level_db(peak), level_db(side));
                let makeup = f64::from(v.knobs[Param::MakeupDb as usize]);
                let dynamics = db_gain(gains[0] + gains[1] + gains[2]) * makeup;
                gain = (1.0 - on) + on * dynamics;
            } else {
                self.stop();
            }
            let listen = self.weight(Param::SidechainListen);
            let mean = if listen > 0.0 && !matched && !sidechain.is_empty() {
                sidechain
                    .iter()
                    .map(|channel| f64::from(channel[n]))
                    .sum::<f64>()
                    / sidechain.len() as f64
            } else {
                0.0
            };
            let mut out_peak = 0.0_f64;
            for (channel, (out, x)) in output.iter_mut().zip(input).enumerate() {
                let processed = f64::from(x[n]) * gain;
                out[n] = if listen > 0.0 {
                    let heard = if matched {
                        f64::from(sidechain[channel][n])
                    } else {
                        mean
                    };
                    ((1.0 - listen) * processed + listen * heard) as f32
                } else {
                    processed as f32
                };
                if heard[channel] {
                    out_peak = out_peak.max(f64::from(out[n].abs()));
                }
            }
            if peak > loudest.0 {
                loudest = (
                    peak,
                    Ball {
                        input_db: level_db(peak) as f32,
                        output_db: level_db(out_peak) as f32,
                        expander_db: gains[0] as f32,
                        compressor_db: gains[1] as f32,
                        limiter_db: gains[2] as f32,
                    },
                );
            }
        }
        self.ball.write(loudest.1);
    }

    fn latency(&self) -> u32 {
        0
    }

    /// [`Dynamics::settle_for`] the parameters as set: each switch on that is on or still
    /// fading, each time the longer of where it is and where it ramps to.
    fn settle(&self) -> u32 {
        let targets = self.targets(self.set_switches());
        let mut settings = Settings::default();
        for param in Param::ALL {
            let index = param as usize;
            let value = if param.is_switch() {
                switch(targets[index] || self.switches[index].weight > 0.0)
            } else if param == Param::MakeupDb {
                0.0
            } else {
                let knob = &self.knobs[index];
                knob.current().max(knob.target())
            };
            settings.set(param, value);
        }
        Dynamics::settle_for(&settings, self.rate)
    }

    fn reset(&mut self) {
        for knob in &mut self.knobs {
            knob.finish();
        }
        let targets = self.targets(self.set_switches());
        for (switch, on) in self.switches.iter_mut().zip(targets) {
            switch.weight = if on { 1.0 } else { 0.0 };
        }
        self.stop();
        self.wake.peak = 0.0;
        self.wake_keyed.peak = 0.0;
    }

    fn set(&mut self, param: ParamId, value: f32) {
        let Some(param) = Param::from_id(param) else {
            return;
        };
        if value.is_nan() {
            return;
        }
        let value = param.clamp(value);
        let index = param as usize;
        if param.is_switch() {
            self.switches[index].on = value >= 0.5;
        } else if param == Param::MakeupDb {
            self.knobs[index].set(crate::db_to_gain(value));
        } else {
            self.knobs[index].set(value);
        }
    }
}
