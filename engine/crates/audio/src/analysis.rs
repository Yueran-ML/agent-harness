//! Offline analysis (plan T9; Analyze Audio Level, p. 3986–3987), Normalize Audio Levels
//! (p. 3840–3841) and Level Matcher (p. 3804–3805): the loudness and the peaks of a whole sound,
//! and the gain that brings it to a level.
//!
//! [`Analyzer`] runs the same [`LoudnessTap`] and [`Loudness`] as the meter of the monitored bus,
//! fed as fast as the sound is rendered (T6's blocking read) instead of by the card. What it
//! measures is pure analysis: the change of a clip's Volume that Normalize and Level Matcher
//! make is an operation of the model (T2), one transaction for all the clips, the whole curve
//! of a clip with keyframes moved by the same amount (C-C5, C-C7).

use crate::constants::{CLIP_VOLUME_RANGE, LOUDNESS_CELLS_KEPT};
use crate::loudness::{loudness_tap, Loudness, LoudnessReader, LoudnessTap, Reading};
use crate::node::Layout;
use crate::standards::{Measure, Standard};

/// What an analysis found, in LUFS, LU, dBTP and dBFS (see [`Reading`]).
pub type Analysis = Reading;

/// The analysis of a sound, fed block by block.
#[derive(Debug)]
pub struct Analyzer {
    tap: LoudnessTap,
    reader: LoudnessReader,
    loudness: Loudness,
    /// The most samples fed to the tap before its cells are read: half of what it keeps.
    chunk: usize,
    lost: u64,
}

impl Analyzer {
    /// An analysis of sound of `layout` at `rate`, reading the true peak if `true_peak` (it costs
    /// the most: 336 products a sample and a channel).
    #[must_use]
    pub fn new(rate: u32, layout: Layout, true_peak: bool) -> Self {
        let (tap, reader) = loudness_tap(rate, layout, true_peak);
        let chunk = tap.cell() * LOUDNESS_CELLS_KEPT / 2;
        Analyzer {
            tap,
            reader,
            loudness: Loudness::new(),
            chunk,
            lost: 0,
        }
    }

    /// Hears a block, planar, all channels of one length; channels past the layout's are not
    /// heard.
    pub fn take(&mut self, input: &[&[f32]]) {
        let input = &input[..input.len().min(self.tap.channels())];
        let frames = input.first().map_or(0, |channel| channel.len());
        let mut parts = [&[][..]; crate::constants::MAX_CHANNELS];
        let mut at = 0;
        while at < frames {
            let end = frames.min(at + self.chunk);
            for (part, channel) in parts.iter_mut().zip(input) {
                *part = &channel[at..end];
            }
            self.tap.take(&parts[..input.len()]);
            let loudness = &mut self.loudness;
            self.lost += self.reader.drain(|cell| loudness.push(cell));
            at = end;
        }
    }

    /// What it has found so far.
    #[must_use]
    pub fn reading(&self) -> Reading {
        self.loudness.reading()
    }

    /// The end of the sound: what it found, the peaks of the last samples counted.
    #[must_use]
    pub fn finish(mut self) -> Analysis {
        debug_assert_eq!(self.lost, 0, "an analysis reads every cell");
        let (true_peak, sample_peak) = self.tap.finish();
        self.loudness.push_peaks(true_peak, sample_peak);
        self.loudness.reading()
    }
}

/// The analysis of `channels` (planar) of `layout` at `rate`.
#[must_use]
pub fn analyze(rate: u32, layout: Layout, true_peak: bool, channels: &[&[f32]]) -> Analysis {
    let mut analyzer = Analyzer::new(rate, layout, true_peak);
    analyzer.take(channels);
    analyzer.finish()
}

/// What Normalize Audio Levels brings to the Reference Level (Normalization Mode, p. 3840).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// Sample Peak Program: the largest sample, in dBFS.
    SamplePeak,
    /// True Peak Program: the largest size between the samples, in dBTP.
    TruePeak,
    /// The integrated loudness of a standard, in LUFS.
    Loudness(Measure),
}

impl Level {
    /// The level of a loudness standard.
    #[must_use]
    pub fn of(standard: &Standard) -> Self {
        Level::Loudness(standard.measure)
    }

    /// Whether its analysis must read the true peak.
    #[must_use]
    pub fn needs_true_peak(self) -> bool {
        self == Level::TruePeak
    }

    /// The level of `analysis`, if it has one: none for silence, nor a loudness for a sound
    /// shorter than a block of 400 ms.
    #[must_use]
    pub fn of_analysis(self, analysis: &Analysis) -> Option<f64> {
        let level = match self {
            Level::SamplePeak => Some(analysis.sample_peak),
            Level::TruePeak => Some(analysis.true_peak),
            Level::Loudness(Measure::Gated) => analysis.integrated,
            Level::Loudness(Measure::Ungated) => analysis.integrated_ungated,
        };
        level.filter(|level| level.is_finite())
    }
}

/// Set Level (p. 3841): how the gain of several clips is found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetLevel {
    /// All the clips as one: one gain for all, from the level of them together (for a peak, the
    /// highest of them; for a loudness, the clips heard one after another in the order of the
    /// timeline, C-C6). The default (C-C5).
    Relative,
    /// Each clip by its own level.
    Independent,
}

/// The gain in dB that brings `analysis` to `target` by `level`; `None` when it has no level
/// (silence, or too short for its loudness), and its Volume is left as it is.
#[must_use]
pub fn normalize(level: Level, target: f64, analysis: &Analysis) -> Option<f64> {
    level
        .of_analysis(analysis)
        .map(|measured| target - measured)
}

/// The Volume of a clip before Normalize, in dB: its lowest and its highest value (its one value,
/// or those of its keyframes), to tell whether a gain takes it out of the range of a Volume.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Volume {
    /// The lowest.
    pub lowest: f64,
    /// The highest.
    pub highest: f64,
}

impl Volume {
    /// One value over all the clip.
    #[must_use]
    pub fn fixed(db: f64) -> Self {
        Volume {
            lowest: db,
            highest: db,
        }
    }
}

/// What Normalize does to a clip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Normalized {
    /// The gain in dB added to its Volume (the whole curve moved by it); `None` when it has no
    /// level and is left as it is.
    pub gain: Option<f64>,
    /// Whether the Volume, or a keyframe of it, moved by the gain would go out of
    /// [`CLIP_VOLUME_RANGE`] and is clamped there, so that the clip does not come to the target:
    /// the dialog tells the user which (Relative most of all, where one gain is for all).
    pub clamped: bool,
}

/// The gains of Normalize Audio Levels for clips: Independent, the gain of each by its own
/// analysis (`each`); Relative, the one gain of their analysis `together` for every clip; and
/// for each, whether its Volume (`volumes`, one for each clip) moved by it would be clamped.
///
/// # Panics
///
/// When `volumes` and `each` are not of one length.
#[must_use]
pub fn normalize_clips(
    level: Level,
    set: SetLevel,
    target: f64,
    each: &[Analysis],
    together: &Analysis,
    volumes: &[Volume],
) -> Vec<Normalized> {
    assert_eq!(each.len(), volumes.len(), "a Volume for each clip");
    let relative = normalize(level, target, together);
    each.iter()
        .zip(volumes)
        .map(|(analysis, volume)| {
            let gain = match set {
                SetLevel::Independent => normalize(level, target, analysis),
                SetLevel::Relative => relative,
            };
            let (low, high) = CLIP_VOLUME_RANGE;
            let clamped =
                gain.is_some_and(|gain| volume.lowest + gain < low || volume.highest + gain > high);
            Normalized { gain, clamped }
        })
        .collect()
}

/// The gain in dB that Level Matcher adds to the Volume of `clip` to match `reference` (C-C7):
/// the difference of their loudness, each by its integrated loudness, gated, or by its ungated
/// mean where it has none (shorter than a block of 400 ms, or under the gates), so that a
/// reference with pauses is not taken lower for a short clip; `None` where either is silent.
#[must_use]
pub fn level_match(reference: &Analysis, clip: &Analysis) -> Option<f64> {
    let loudness = |analysis: &Analysis| analysis.integrated.or(analysis.mean);
    let difference = loudness(reference).zip(loudness(clip)).map(|(r, c)| r - c);
    difference.filter(|difference| difference.is_finite())
}
