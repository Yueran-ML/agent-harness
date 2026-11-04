//! The processing of sound that a clip, a track or a bus may have, as settings: the equalizer of
//! a clip (4 bands) and of a strip (6 bands), the Dynamics of a strip, the Ducker of a track, and
//! the Fairlight FX that go in its inserts and on clips. Each has the defaults and the ranges of
//! `DaVinci Resolve` (Reference Manual 20, Ch. 177–181), and where the manual gives none, those
//! of the studio's interaction reference (§8–§10, §15), which the engine and the desktop share.
//!
//! Every field may be left out where it is read, as its default. A setting out of its range is
//! refused as [`OpError::OutOfRange`] naming what it is of: `eq`, `dynamics`, `ducker` or the
//! kind of the effect.

use serde::{Deserialize, Serialize};

use crate::ops::OpError;
use crate::project::Id;

/// Whether `value` is a finite number from `low` to `high`.
pub(crate) fn within(value: f64, low: f64, high: f64) -> bool {
    value.is_finite() && (low..=high).contains(&value)
}

fn out(what: &'static str) -> OpError {
    OpError::OutOfRange { what }
}

/// Refuse as `what` unless every one of `checks` holds.
fn all(what: &'static str, checks: &[bool]) -> Result<(), OpError> {
    if checks.iter().all(|c| *c) {
        Ok(())
    } else {
        Err(out(what))
    }
}

const fn yes() -> bool {
    true
}

/// What a band of an equalizer does (pp. 3827–3829, 3868–3869).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EqKind {
    /// Raises or lowers what is below its frequency.
    LowShelf,
    /// Raises or lowers what is about its frequency (Peaking).
    Bell,
    /// Raises or lowers what is above its frequency.
    HighShelf,
    /// Lets through what is above its frequency.
    HighPass,
    /// Lets through what is below its frequency.
    LowPass,
    /// Takes away what is about its frequency.
    Notch,
}

/// The stretch of frequencies that a middle band of the equalizer of a strip is in, as its
/// L, ML, MH and H buttons choose (p. 3869). The bounds are the studio's (the plan of phase 3):
/// L 20–200 Hz, ML 100 Hz–1 kHz, MH 500 Hz–5 kHz, H 2–20 kHz.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EqRange {
    /// 20–200 Hz.
    Low,
    /// 100 Hz–1 kHz.
    MediumLow,
    /// 500 Hz–5 kHz.
    MediumHigh,
    /// 2–20 kHz.
    High,
}

impl EqRange {
    /// Its lowest and highest frequency, in hertz.
    #[must_use]
    pub fn bounds(self) -> (f64, f64) {
        match self {
            Self::Low => (20.0, 200.0),
            Self::MediumLow => (100.0, 1000.0),
            Self::MediumHigh => (500.0, 5000.0),
            Self::High => (2000.0, 20000.0),
        }
    }
}

/// A band of an equalizer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EqBand {
    /// Whether it is on.
    pub on: bool,
    /// What it does.
    pub kind: EqKind,
    /// Its frequency, in hertz: from 20 to 20000, and within its range where it has one.
    pub freq: f64,
    /// How much it raises or lowers, in decibels, from -20 to 20. A pass and a notch have none.
    pub gain_db: f64,
    /// How narrow it is, from 0.3 to 10.3: of a bell and a notch.
    pub q: f64,
    /// How steep a pass is, in decibels an octave: 6, 12, 18 or 24.
    pub slope: u8,
    /// The stretch of frequencies that it is in: of a middle band of a strip only.
    pub range: Option<EqRange>,
}

impl Default for EqBand {
    fn default() -> Self {
        Self {
            on: true,
            kind: EqKind::Bell,
            freq: 1000.0,
            gain_db: 0.0,
            q: 1.0,
            slope: 12,
            range: None,
        }
    }
}

/// An equalizer: of a clip, 4 bands, the outer two off and the middle two on (p. 3827); of a
/// strip, 6 bands, the outer two off and the middle four on (p. 3868), with a gain after them
/// (p. 3867). The middle bands are on at no gain, and change nothing until they are moved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Equalizer {
    /// Whether it is on: off for a new clip and a new strip.
    #[serde(default)]
    pub on: bool,
    /// Its bands, in order.
    pub bands: Vec<EqBand>,
    /// The gain after it, in decibels, from -20 to 20.
    #[serde(default)]
    pub gain_db: f64,
}

impl Equalizer {
    /// The equalizer of a new clip: Hi-Pass at 80 Hz and Lo-Pass at 16 kHz, off, and bells at
    /// 300 Hz and 3 kHz (C-E5).
    #[must_use]
    pub fn clip() -> Self {
        let band = |on, kind, freq| EqBand {
            on,
            kind,
            freq,
            ..EqBand::default()
        };
        Self {
            on: false,
            bands: vec![
                band(false, EqKind::HighPass, 80.0),
                band(true, EqKind::Bell, 300.0),
                band(true, EqKind::Bell, 3000.0),
                band(false, EqKind::LowPass, 16000.0),
            ],
            gain_db: 0.0,
        }
    }

    /// The equalizer of a new strip: High Pass at 80 Hz and Low Pass at 16 kHz, off, and bells
    /// at 100 Hz, 500 Hz, 2 kHz and 8 kHz in the ranges L, ML, MH and H (C-E1, C-E2).
    #[must_use]
    pub fn strip() -> Self {
        let band = |on, kind, freq, range| EqBand {
            on,
            kind,
            freq,
            range,
            ..EqBand::default()
        };
        Self {
            on: false,
            bands: vec![
                band(false, EqKind::HighPass, 80.0, None),
                band(true, EqKind::Bell, 100.0, Some(EqRange::Low)),
                band(true, EqKind::Bell, 500.0, Some(EqRange::MediumLow)),
                band(true, EqKind::Bell, 2000.0, Some(EqRange::MediumHigh)),
                band(true, EqKind::Bell, 8000.0, Some(EqRange::High)),
                band(false, EqKind::LowPass, 16000.0, None),
            ],
            gain_db: 0.0,
        }
    }

    /// Check it as the equalizer of a clip (4 bands) or of a strip (6 bands).
    ///
    /// # Errors
    /// It has another number of bands, a band of a kind that its place does not have, or a
    /// setting out of its range: `eq`.
    pub fn check(&self, strip: bool) -> Result<(), OpError> {
        use EqKind::{Bell, HighPass, HighShelf, LowPass, LowShelf, Notch};
        let count = if strip { 6 } else { 4 };
        all(
            "eq",
            &[self.bands.len() == count, within(self.gain_db, -20.0, 20.0)],
        )?;
        for (at, band) in self.bands.iter().enumerate() {
            let outer = at == 0 || at == count - 1;
            let kinds: &[EqKind] = match (strip, at) {
                (true, 0) => &[LowShelf, Bell, HighShelf, HighPass],
                (true, 5) => &[LowPass, LowShelf, Bell, HighShelf],
                (false, 0 | 3) => &[HighShelf, HighPass, Bell, Notch, LowPass, LowShelf],
                _ => &[LowShelf, Bell, Notch, HighShelf],
            };
            let range = match (strip && !outer, band.range) {
                (true, Some(range)) => {
                    let (low, high) = range.bounds();
                    within(band.freq, low, high)
                }
                (false, None) => true,
                _ => false,
            };
            let slope = if strip {
                matches!(band.slope, 6 | 12 | 18 | 24)
            } else {
                band.slope == 12
            };
            all(
                "eq",
                &[
                    kinds.contains(&band.kind),
                    range,
                    slope,
                    within(band.freq, 20.0, 20000.0),
                    within(band.gain_db, -20.0, 20.0),
                    within(band.q, 0.3, 10.3),
                ],
            )?;
        }
        Ok(())
    }
}

/// Whether the first module of Dynamics expands or gates (p. 3863): one module with one set of
/// controls, switched between the two.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateMode {
    /// Expands what is below the threshold, by the ratio.
    #[default]
    Expander,
    /// Lowers what is below the threshold by the range.
    Gate,
}

/// The Expander/Gate of Dynamics (p. 3863).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Expander {
    /// Whether it is on: off for a new strip.
    pub on: bool,
    /// Whether it expands or gates.
    pub mode: GateMode,
    /// Below which it works, in decibels: from -50 to 0, -35 for a new strip.
    pub threshold_db: f64,
    /// How far it lowers at most, in decibels: from 0 to 60.2, 18 for a new strip.
    pub range_db: f64,
    /// How much it expands, 1:1.1 to 1:3.0 written as 1.1 to 3.0: 1.1 for a new strip.
    pub ratio: f64,
    /// Milliseconds from 0 to 100: 1.4 for a new strip.
    pub attack_ms: f64,
    /// Milliseconds from 0 to 4000: 0 for a new strip.
    pub hold_ms: f64,
    /// Milliseconds from 50 to 4000: 93 for a new strip.
    pub release_ms: f64,
}

impl Default for Expander {
    fn default() -> Self {
        Self {
            on: false,
            mode: GateMode::Expander,
            threshold_db: -35.0,
            range_db: 18.0,
            ratio: 1.1,
            attack_ms: 1.4,
            hold_ms: 0.0,
            release_ms: 93.0,
        }
    }
}

/// Where a sidechain, or a ducker, takes the sound that drives it from: a track or a bus of its
/// timeline, before its fader (pp. 3865, 3940). It is written `{"track": "A2"}` or
/// `{"bus": "B2"}`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceRef {
    /// A track of sound.
    Track(Id),
    /// A bus.
    Bus(Id),
}

/// The sidechain of a Compressor or of a Limiter (pp. 3865–3866, 3932, 3965).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Sidechain {
    /// Whether it drives the module: off for a new strip.
    pub on: bool,
    /// Whether only the sound that drives it is listened to: it is never rendered.
    pub listen: bool,
    /// What drives it, or none.
    pub source: Option<SourceRef>,
}

/// The Compressor of Dynamics (pp. 3863–3866).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Compressor {
    /// Whether it is on: off for a new strip.
    pub on: bool,
    /// Above which it works, in decibels: from -50 to 0, -15 for a new strip.
    pub threshold_db: f64,
    /// How much it compresses, 1:1 to 10:1 written as 1 to 10: 2 for a new strip.
    pub ratio: f64,
    /// The width of its knee, in decibels: from 0 to 20, 0 (hard) for a new strip.
    pub knee_db: f64,
    /// How much of the sound before it is mixed back, in percent: from 0 (all compressed, as a
    /// new strip has it) to 100 (none), as `DaVinci` turns it (p. 3864).
    pub mix: f64,
    /// Milliseconds from 0 to 100: 1.4 for a new strip.
    pub attack_ms: f64,
    /// Milliseconds from 0 to 4000: 0 for a new strip.
    pub hold_ms: f64,
    /// Milliseconds from 50 to 4000: 93 for a new strip.
    pub release_ms: f64,
    /// Its sidechain.
    pub sidechain: Sidechain,
}

impl Default for Compressor {
    fn default() -> Self {
        Self {
            on: false,
            threshold_db: -15.0,
            ratio: 2.0,
            knee_db: 0.0,
            mix: 0.0,
            attack_ms: 1.4,
            hold_ms: 0.0,
            release_ms: 93.0,
            sidechain: Sidechain::default(),
        }
    }
}

/// The Limiter of Dynamics (p. 3866), which limits sample peaks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DynamicsLimiter {
    /// Whether it is on: off for a new strip.
    pub on: bool,
    /// The most it lets out, in decibels: from -50 to 0, -21 for a new strip.
    pub threshold_db: f64,
    /// Milliseconds from 0 to 100: 0.71 for a new strip.
    pub attack_ms: f64,
    /// Milliseconds from 0 to 4000: 0 for a new strip.
    pub hold_ms: f64,
    /// Milliseconds from 50 to 4000: 90 for a new strip.
    pub release_ms: f64,
}

impl Default for DynamicsLimiter {
    fn default() -> Self {
        Self {
            on: false,
            threshold_db: -21.0,
            attack_ms: 0.71,
            hold_ms: 0.0,
            release_ms: 90.0,
        }
    }
}

/// The Dynamics of a strip (pp. 3861–3866): Expander/Gate, Compressor and Limiter, each on or
/// off, and a make-up gain after them; all of it off for a new strip.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Dynamics {
    /// Whether it is on.
    pub on: bool,
    /// Its Expander/Gate.
    pub expander: Expander,
    /// Its Compressor.
    pub compressor: Compressor,
    /// Its Limiter.
    pub limiter: DynamicsLimiter,
    /// The gain after it, in decibels, from -20 to 20.
    pub makeup_db: f64,
}

impl Dynamics {
    /// Check its settings.
    ///
    /// # Errors
    /// One is out of its range: `dynamics`.
    pub fn check(&self) -> Result<(), OpError> {
        let e = &self.expander;
        let c = &self.compressor;
        let l = &self.limiter;
        let times = |attack, hold, release| {
            within(attack, 0.0, 100.0) && within(hold, 0.0, 4000.0) && within(release, 50.0, 4000.0)
        };
        all(
            "dynamics",
            &[
                within(e.threshold_db, -50.0, 0.0),
                within(e.range_db, 0.0, 60.2),
                within(e.ratio, 1.1, 3.0),
                times(e.attack_ms, e.hold_ms, e.release_ms),
                within(c.threshold_db, -50.0, 0.0),
                within(c.ratio, 1.0, 10.0),
                within(c.knee_db, 0.0, 20.0),
                within(c.mix, 0.0, 100.0),
                times(c.attack_ms, c.hold_ms, c.release_ms),
                within(l.threshold_db, -50.0, 0.0),
                times(l.attack_ms, l.hold_ms, l.release_ms),
                within(self.makeup_db, -20.0, 20.0),
            ],
        )
    }
}

/// The Ducker of a track (pp. 3940–3941), its one Track FX: it lowers the track by a set amount
/// while its sources sound, "without compressing".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Ducker {
    /// Whether it is on.
    pub on: bool,
    /// The tracks and buses whose sound, summed, ducks the track.
    pub sources: Vec<SourceRef>,
    /// How far it lowers, in decibels: from 0 to 30, 2.7 for a new one.
    pub amount_db: f64,
    /// Milliseconds from 0 to 100: 15 for a new one.
    pub lookahead_ms: f64,
    /// Milliseconds from 1 to 1000: 10 for a new one.
    pub rise_ms: f64,
    /// Milliseconds from 0 to 4000: 150 for a new one.
    pub hold_ms: f64,
    /// Milliseconds from 10 to 4000: 750 for a new one.
    pub recovery_ms: f64,
}

impl Default for Ducker {
    fn default() -> Self {
        Self {
            on: false,
            sources: vec![],
            amount_db: 2.7,
            lookahead_ms: 15.0,
            rise_ms: 10.0,
            hold_ms: 150.0,
            recovery_ms: 750.0,
        }
    }
}

impl Ducker {
    /// Check its settings, but not its sources, which the routing of its timeline checks.
    ///
    /// # Errors
    /// One is out of its range: `ducker`.
    pub fn check(&self) -> Result<(), OpError> {
        all(
            "ducker",
            &[
                within(self.amount_db, 0.0, 30.0),
                within(self.lookahead_ms, 0.0, 100.0),
                within(self.rise_ms, 1.0, 1000.0),
                within(self.hold_ms, 0.0, 4000.0),
                within(self.recovery_ms, 10.0, 4000.0),
            ],
        )
    }
}

/// How wide the band of a De-Esser is (p. 3948).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EssRange {
    /// About a third of an octave about the target.
    Narrow,
    /// About an octave about the target.
    #[default]
    Wide,
    /// All above the target.
    AllHigh,
}

/// How quickly a De-Esser acts (p. 3948).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reaction {
    /// Attack 10 ms, release 80 ms.
    Slow,
    /// Attack 2 ms, release 40 ms.
    #[default]
    Medium,
    /// Attack 0.5 ms, release 20 ms.
    Fast,
}

/// The frequencies that a Noise Reduction works on (C-X4): all, or those that its presets
/// De-Rumble, De-Hiss and De-Rumble and Hiss set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoiseBand {
    /// Every frequency.
    #[default]
    Full,
    /// Below 250 Hz (De-Rumble).
    Rumble,
    /// Above 4 kHz (De-Hiss).
    Hiss,
    /// Both (De-Rumble and Hiss).
    RumbleAndHiss,
}

/// What a Noise Reduction takes the noise from (p. 3970; the modes are the studio's).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoiseMode {
    /// Follows the noise as it goes.
    #[default]
    AutoSpeech,
    /// The noise that it learned from a stretch of it.
    Manual,
    /// The voice, found by `RNNoise`: only Dry/Wet and Level apply.
    Voice,
}

/// Gain (pp. 3963–3964).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GainFx {
    /// Decibels from -24 to 24.
    pub gain_db: f64,
}

/// Limiter (pp. 3964–3966): of true peaks, looking 64 samples ahead.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LimiterFx {
    /// The gain before it, in decibels, from -18 to 18.
    pub input_db: f64,
    /// Whether it attacks softly.
    pub soft: bool,
    /// The highest true peak that it lets out, in decibels, from -24 to 0: -1 for a new one.
    pub ceiling_db: f64,
    /// Milliseconds from 0.01 to 1000: 100 for a new one.
    pub release_ms: f64,
    /// Its sidechain.
    pub sidechain: Sidechain,
}

impl Default for LimiterFx {
    fn default() -> Self {
        Self {
            input_db: 0.0,
            soft: false,
            ceiling_db: -1.0,
            release_ms: 100.0,
            sidechain: Sidechain::default(),
        }
    }
}

/// Soft Clipper (pp. 3973–3974).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SoftClipperFx {
    /// The gain before it clips at 0 dBFS (named Threshold), in decibels, from 0 to 24.
    pub threshold_db: f64,
    /// From 0, a hard clip, to 100, all soft: 100 for a new one.
    pub shape: f64,
    /// The gain after it, in decibels, from -24 to 12.
    pub output_db: f64,
}

impl Default for SoftClipperFx {
    fn default() -> Self {
        Self {
            threshold_db: 0.0,
            shape: 100.0,
            output_db: 0.0,
        }
    }
}

/// Stereo Width (p. 3975).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StereoWidthFx {
    /// From 0, mono, through 1, as it is, to 2.
    pub width: f64,
    /// Percent from 0 to 100.
    pub diffusion: f64,
    /// Percent from 0 to 100.
    pub sparkle: f64,
}

impl Default for StereoWidthFx {
    fn default() -> Self {
        Self {
            width: 1.0,
            diffusion: 0.0,
            sparkle: 0.0,
        }
    }
}

/// De-Esser (p. 3948).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DeEsserFx {
    /// Its target, in hertz, from 2000 to 16000: 6000 for a new one.
    pub frequency_hz: f64,
    /// How wide its band is.
    pub range: EssRange,
    /// Percent from 0 to 100, up to 20 dB of reduction: 50 for a new one.
    pub amount: f64,
    /// How quickly it acts.
    pub reaction: Reaction,
    /// Whether only what it takes away is listened to.
    pub listen: bool,
}

impl Default for DeEsserFx {
    fn default() -> Self {
        Self {
            frequency_hz: 6000.0,
            range: EssRange::Wide,
            amount: 50.0,
            reaction: Reaction::Medium,
            listen: false,
        }
    }
}

/// De-Hummer (p. 3949).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DeHummerFx {
    /// The frequency of the hum, in hertz, from 40 to 70: 50 for a new one.
    pub frequency_hz: f64,
    /// Percent from 0 to 100, up to 40 dB deep: 50 for a new one.
    pub amount: f64,
    /// From 0, toward the fundamental, to 1, toward the harmonics: 0.5 for a new one.
    pub slope: f64,
    /// Whether only the hum is listened to.
    pub listen: bool,
}

impl Default for DeHummerFx {
    fn default() -> Self {
        Self {
            frequency_hz: 50.0,
            amount: 50.0,
            slope: 0.5,
            listen: false,
        }
    }
}

/// The most bins of a learned noise.
pub const LONGEST_PROFILE: usize = 4097;

/// Noise Reduction (p. 3970).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NoiseReductionFx {
    /// What it takes the noise from.
    pub mode: NoiseMode,
    /// Decibels from 0 to 40: 10 for a new one.
    pub threshold_db: f64,
    /// Milliseconds from 50 to 5000: 1000 for a new one.
    pub attack_ms: f64,
    /// Percent from 0 to 100: 50 for a new one.
    pub sensitivity: f64,
    /// From 1 to 20: 4 for a new one.
    pub ratio: f64,
    /// Percent from 0 to 100: 25 for a new one.
    pub frequency_smoothing: f64,
    /// Whether it smooths over time: on for a new one.
    pub time_smoothing: bool,
    /// Percent from 0, dry, to 100, wet: 100 for a new one.
    pub dry_wet: f64,
    /// Decibels from -6 to 18.
    pub level_db: f64,
    /// Whether only the noise that it takes away is listened to.
    pub listen: bool,
    /// The spectrum of the noise that it learned, as the power of each bin (`Manual`), or none:
    /// at most [`LONGEST_PROFILE`] finite numbers of no less than 0.
    pub profile: Option<Vec<f64>>,
    /// The frequencies that it works on: all, or those of the presets De-Rumble (below 250 Hz)
    /// and De-Hiss (above 4 kHz), as the presets set them (C-X4). Not automated.
    pub band: NoiseBand,
}

impl Default for NoiseReductionFx {
    fn default() -> Self {
        Self {
            mode: NoiseMode::AutoSpeech,
            threshold_db: 10.0,
            attack_ms: 1000.0,
            sensitivity: 50.0,
            ratio: 4.0,
            frequency_smoothing: 25.0,
            time_smoothing: true,
            dry_wet: 100.0,
            level_db: 0.0,
            listen: false,
            profile: None,
            band: NoiseBand::Full,
        }
    }
}

/// Pitch (p. 3971), and the pitch of a clip (p. 3826).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PitchFx {
    /// Whole semitones from -12 to 12.
    pub semitones: i32,
    /// Cents from -100 to 100.
    pub cents: f64,
    /// Percent from 0, dry, to 100, wet: 100 for a new one.
    pub dry_wet: f64,
}

impl Default for PitchFx {
    fn default() -> Self {
        Self {
            semitones: 0,
            cents: 0.0,
            dry_wet: 100.0,
        }
    }
}

/// A Fairlight FX of the studio, with its settings: written as an object that names it in
/// `kind`, beside its settings (`{"kind": "gain", "gain_db": -3}`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Effect {
    /// Gain.
    Gain(GainFx),
    /// Limiter.
    Limiter(LimiterFx),
    /// Soft Clipper.
    SoftClipper(SoftClipperFx),
    /// Stereo Width.
    StereoWidth(StereoWidthFx),
    /// De-Esser.
    DeEsser(DeEsserFx),
    /// De-Hummer.
    DeHummer(DeHummerFx),
    /// Noise Reduction.
    NoiseReduction(NoiseReductionFx),
    /// Pitch.
    Pitch(PitchFx),
}

impl Effect {
    /// The name of its kind, as it is written.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Gain(_) => "gain",
            Self::Limiter(_) => "limiter",
            Self::SoftClipper(_) => "soft_clipper",
            Self::StereoWidth(_) => "stereo_width",
            Self::DeEsser(_) => "de_esser",
            Self::DeHummer(_) => "de_hummer",
            Self::NoiseReduction(_) => "noise_reduction",
            Self::Pitch(_) => "pitch",
        }
    }

    /// The sidechain that it has, if it has one.
    #[must_use]
    pub fn sidechain(&self) -> Option<&Sidechain> {
        match self {
            Self::Limiter(fx) => Some(&fx.sidechain),
            _ => None,
        }
    }

    /// Check its settings, but not the source of its sidechain, which the routing of its
    /// timeline checks.
    ///
    /// # Errors
    /// One is out of its range: the name of its kind.
    pub fn check(&self) -> Result<(), OpError> {
        let what = self.name();
        let checks: Vec<bool> = match self {
            Self::Gain(fx) => vec![within(fx.gain_db, -24.0, 24.0)],
            Self::Limiter(fx) => vec![
                within(fx.input_db, -18.0, 18.0),
                within(fx.ceiling_db, -24.0, 0.0),
                within(fx.release_ms, 0.01, 1000.0),
            ],
            Self::SoftClipper(fx) => vec![
                within(fx.threshold_db, 0.0, 24.0),
                within(fx.shape, 0.0, 100.0),
                within(fx.output_db, -24.0, 12.0),
            ],
            Self::StereoWidth(fx) => vec![
                within(fx.width, 0.0, 2.0),
                within(fx.diffusion, 0.0, 100.0),
                within(fx.sparkle, 0.0, 100.0),
            ],
            Self::DeEsser(fx) => vec![
                within(fx.frequency_hz, 2000.0, 16000.0),
                within(fx.amount, 0.0, 100.0),
            ],
            Self::DeHummer(fx) => vec![
                within(fx.frequency_hz, 40.0, 70.0),
                within(fx.amount, 0.0, 100.0),
                within(fx.slope, 0.0, 1.0),
            ],
            Self::NoiseReduction(fx) => vec![
                within(fx.threshold_db, 0.0, 40.0),
                within(fx.attack_ms, 50.0, 5000.0),
                within(fx.sensitivity, 0.0, 100.0),
                within(fx.ratio, 1.0, 20.0),
                within(fx.frequency_smoothing, 0.0, 100.0),
                within(fx.dry_wet, 0.0, 100.0),
                within(fx.level_db, -6.0, 18.0),
                fx.profile.as_ref().is_none_or(|bins| {
                    bins.len() <= LONGEST_PROFILE && bins.iter().all(|b| b.is_finite() && *b >= 0.0)
                }),
            ],
            Self::Pitch(fx) => vec![
                (-12..=12).contains(&fx.semitones),
                within(fx.cents, -100.0, 100.0),
                within(fx.dry_wet, 0.0, 100.0),
            ],
        };
        all(what, &checks)
    }
}

/// An effect in an insert of a strip, or on a clip: the effect, and whether it is on (not
/// bypassed, as a new one is). A bypassed effect still delays its sound as it does when it is
/// on, so that nothing moves when it is bypassed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Insert {
    /// What it is known by among the inserts of its strip (`fx1`, `fx2`, ...), which stays as
    /// it moves from one insert to another, so that the lanes of its settings stay with it
    /// (`insert.fx1.ceiling_db`). Where it is left out or empty, the engine gives an insert of a
    /// strip the next `fx<n>` of the strip; one on a clip may have none.
    #[serde(default)]
    pub id: Id,
    /// Whether it is on. It may be left out, as on.
    #[serde(default = "yes")]
    pub on: bool,
    /// The effect.
    pub effect: Effect,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_effect_is_written_with_its_kind_beside_its_settings_and_left_out_ones_are_defaults() {
        let insert: Insert = serde_json::from_value(serde_json::json!({
            "effect": { "kind": "limiter", "ceiling_db": -2.0 },
        }))
        .unwrap();
        assert!(insert.on);
        let Effect::Limiter(limiter) = &insert.effect else {
            panic!("{insert:?}")
        };
        assert_eq!(limiter.ceiling_db, -2.0);
        assert_eq!(limiter.release_ms, 100.0);
        let written = serde_json::to_value(&insert).unwrap();
        assert_eq!(written["effect"]["kind"], "limiter");
        assert_eq!(
            written["effect"]["sidechain"]["source"],
            serde_json::Value::Null
        );
        assert_eq!(serde_json::from_value::<Insert>(written).unwrap(), insert);
        for wrong in [
            serde_json::json!({ "effect": { "kind": "reverb" } }),
            serde_json::json!({ "effect": { "kind": "gain", "colour": 1 } }),
        ] {
            assert!(serde_json::from_value::<Insert>(wrong).is_err());
        }
        let source: SourceRef = serde_json::from_value(serde_json::json!({ "bus": "B2" })).unwrap();
        assert_eq!(source, SourceRef::Bus("B2".into()));
    }

    #[test]
    fn the_defaults_are_those_of_the_manual_and_within_their_ranges() {
        let dynamics = Dynamics::default();
        assert_eq!(
            (
                dynamics.expander.threshold_db,
                dynamics.expander.ratio,
                dynamics.expander.release_ms
            ),
            (-35.0, 1.1, 93.0)
        );
        assert_eq!(
            (dynamics.compressor.threshold_db, dynamics.compressor.ratio),
            (-15.0, 2.0)
        );
        assert_eq!(dynamics.limiter.attack_ms, 0.71);
        dynamics.check().unwrap();
        let ducker = Ducker::default();
        assert_eq!(
            (
                ducker.amount_db,
                ducker.lookahead_ms,
                ducker.rise_ms,
                ducker.hold_ms,
                ducker.recovery_ms
            ),
            (2.7, 15.0, 10.0, 150.0, 750.0)
        );
        ducker.check().unwrap();
        Equalizer::clip().check(false).unwrap();
        Equalizer::strip().check(true).unwrap();
        for effect in [
            Effect::Gain(GainFx::default()),
            Effect::Limiter(LimiterFx::default()),
            Effect::SoftClipper(SoftClipperFx::default()),
            Effect::StereoWidth(StereoWidthFx::default()),
            Effect::DeEsser(DeEsserFx::default()),
            Effect::DeHummer(DeHummerFx::default()),
            Effect::NoiseReduction(NoiseReductionFx::default()),
            Effect::Pitch(PitchFx::default()),
        ] {
            effect.check().unwrap();
            let written = serde_json::to_value(&effect).unwrap();
            assert_eq!(written["kind"], effect.name());
            assert_eq!(serde_json::from_value::<Effect>(written).unwrap(), effect);
        }
    }

    #[test]
    fn a_noise_reduction_works_on_all_frequencies_or_the_band_of_a_preset() {
        let read = |band: serde_json::Value| {
            serde_json::from_value::<Effect>(
                serde_json::json!({ "kind": "noise_reduction", "band": band }),
            )
        };
        let Ok(Effect::NoiseReduction(fx)) = read(serde_json::json!("rumble_and_hiss")) else {
            panic!("a band")
        };
        assert_eq!(fx.band, NoiseBand::RumbleAndHiss);
        let Ok(Effect::NoiseReduction(left_out)) =
            serde_json::from_value::<Effect>(serde_json::json!({ "kind": "noise_reduction" }))
        else {
            panic!("a noise reduction")
        };
        assert_eq!(left_out.band, NoiseBand::Full);
        assert_eq!(
            serde_json::to_value(Effect::NoiseReduction(fx)).unwrap()["band"],
            "rumble_and_hiss"
        );
        assert!(read(serde_json::json!("treble")).is_err());
    }

    #[test]
    fn a_setting_out_of_its_range_is_refused_as_what_it_is_of() {
        let out = |what| Err(OpError::OutOfRange { what });
        let mut eq = Equalizer::strip();
        eq.bands[1].freq = 300.0;
        assert_eq!(eq.check(true), out("eq"));
        let mut eq = Equalizer::strip();
        eq.bands[0].kind = EqKind::Notch;
        assert_eq!(eq.check(true), out("eq"));
        assert_eq!(Equalizer::strip().check(false), out("eq"));
        let mut eq = Equalizer::clip();
        eq.bands[1].q = 11.0;
        assert_eq!(eq.check(false), out("eq"));
        let mut dynamics = Dynamics::default();
        dynamics.compressor.ratio = 11.0;
        assert_eq!(dynamics.check(), out("dynamics"));
        let ducker = Ducker {
            amount_db: 31.0,
            ..Ducker::default()
        };
        assert_eq!(ducker.check(), out("ducker"));
        let pitch = Effect::Pitch(PitchFx {
            semitones: 13,
            ..PitchFx::default()
        });
        assert_eq!(pitch.check(), out("pitch"));
        let noise = Effect::NoiseReduction(NoiseReductionFx {
            profile: Some(vec![-1.0]),
            ..NoiseReductionFx::default()
        });
        assert_eq!(noise.check(), out("noise_reduction"));
    }
}
