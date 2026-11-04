//! The level meters of tracks and buses (plan T9; p. 3688, p. 3887–3888, p. 3979–3980, p. 162):
//! IEC 60268-18, Digital VU and Custom, before or after the fader.
//!
//! # Two halves
//!
//! On the thread of the mix, a [`LevelTap`] at each point where a level is taken (before and
//! after the fader of each track and bus, T4b) sums each block into cells of 10 ms
//! ([`METER_CELL`]): the peak and the mean square of each channel. It publishes the last
//! [`LEVEL_CELLS_KEPT`] cells, each in a seqlock of its own
//! ([`rt::cells`](mod@crate::rt::cells): one cell of 50 words written each 10 ms), so that a
//! reader 30 times a second loses no peak that falls between two of its reads. It takes no memory once made and never waits.
//!
//! Off it, a [`LevelReader`] gives the cells in order to [`Ballistics`], which makes what the
//! meter shows: the bar by the detector of the meter type (the peak of the samples that falls
//! 20 dB in 1.7 s, the VU, or the RMS of 300 ms) and the peak indicator (held, then falling).
//! The ballistics work cell by cell, so that what they show does not depend on when they are
//! read; a cell's peak is taken as at its end.
//!
//! # The meter types (p. 162, p. 3887–3888; the reference's §12.5 and C-L1–C-L4)
//!
//! - IEC 60268-18 (the default): the bar is the sample peak, rising at once and falling 20 dB in
//!   1.7 s; the peak indicator holds 2 s and falls as fast.
//! - Digital VU: the bar is the VU of the RMS (two critically damped stages, 99 % in 300 ms
//!   either way); the peak indicator is the sample peak, held 2 s and falling 20 dB in 0.5 s.
//! - Custom: the level detector (Sample Program, VU, RMS), the decay of the bar of the sample
//!   peak and of the indicator (seconds for 20 dB), and the peak indicator (Off, Short hold +
//!   fall, Medium hold + fall, Medium hold, Long hold, Infinite hold).

use crate::constants::{
    samples, CUSTOM_DECAY_RANGE, DIGITAL_VU_PEAK_FALL_TIME, LEVEL_CELLS_KEPT, MAX_CHANNELS,
    METER_CELL, METER_FLOOR_DB, METER_IEC_MIDDLE_DB, PEAK_HOLD, PEAK_HOLD_LONG, PEAK_HOLD_SHORT,
    PPM_FALL_DB, PPM_FALL_TIME, RMS_WINDOW, VU_STAGE,
};
use crate::rt::{cells, CellReader, CellWriter, Words};

/// What a level tap publishes of 10 ms: for each channel, the largest size of a sample and the
/// mean of the squares. Channels past the tap's are 0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LevelCell {
    /// The peak of each channel, as a gain of amplitude.
    pub peaks: [f32; MAX_CHANNELS],
    /// The mean square of each channel.
    pub squares: [f32; MAX_CHANNELS],
}

impl Default for LevelCell {
    fn default() -> Self {
        LevelCell {
            peaks: [0.0; MAX_CHANNELS],
            squares: [0.0; MAX_CHANNELS],
        }
    }
}

impl Words for LevelCell {
    const WORDS: usize = 2 * MAX_CHANNELS;
    fn word(&self, index: usize) -> u32 {
        (self.peaks, self.squares).word(index)
    }
    fn from_words(word: impl Fn(usize) -> u32) -> Self {
        let (peaks, squares) = <([f32; MAX_CHANNELS], [f32; MAX_CHANNELS])>::from_words(word);
        LevelCell { peaks, squares }
    }
}

/// The half of a level meter that runs on the thread of the mix (see the module). Made by
/// [`level_tap`], off it.
#[derive(Debug)]
pub struct LevelTap {
    channels: usize,
    cell: usize,
    filled: usize,
    peaks: [f32; MAX_CHANNELS],
    sums: [f64; MAX_CHANNELS],
    writer: CellWriter<LevelCell>,
}

/// A tap of the levels of `channels` (at most [`MAX_CHANNELS`]) at `rate`, and its first reader.
/// Off the thread of the mix: it takes its memory here.
#[must_use]
pub fn level_tap(rate: u32, channels: usize) -> (LevelTap, LevelReader) {
    let (writer, reader) = cells(LEVEL_CELLS_KEPT, LevelCell::default());
    let cell = samples(METER_CELL, rate).max(1);
    let tap = LevelTap {
        channels: channels.min(MAX_CHANNELS),
        cell,
        filled: 0,
        peaks: [0.0; MAX_CHANNELS],
        sums: [0.0; MAX_CHANNELS],
        writer,
    };
    let reader = LevelReader {
        reader,
        seconds: METER_CELL,
    };
    (tap, reader.with_cell(rate, cell))
}

impl LevelTap {
    /// The channels it hears.
    #[must_use]
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Another reader, off the thread of the mix; it reads the cells published from now on.
    #[must_use]
    pub fn reader(&self, rate: u32) -> LevelReader {
        LevelReader {
            reader: self.writer.reader(),
            seconds: METER_CELL,
        }
        .with_cell(rate, self.cell)
    }

    /// Hears a block: `input` planar, a slice for each channel (more than the tap's are not
    /// heard, fewer are silence), all of one length. Publishes each cell that it completes. Does
    /// not allocate, lock or wait.
    pub fn take(&mut self, input: &[&[f32]]) {
        let frames = input.first().map_or(0, |channel| channel.len());
        let channels = self.channels.min(input.len());
        let mut at = 0;
        while at < frames {
            let end = frames.min(at + self.cell - self.filled);
            for (channel, samples) in input[..channels].iter().enumerate() {
                let mut peak = self.peaks[channel];
                let mut sum = self.sums[channel];
                for &x in &samples[at..end] {
                    let x = crate::loudness::finite(x);
                    peak = peak.max(x.abs());
                    sum += f64::from(x) * f64::from(x);
                }
                self.peaks[channel] = peak;
                self.sums[channel] = sum;
            }
            self.filled += end - at;
            at = end;
            if self.filled == self.cell {
                self.publish();
            }
        }
    }

    /// Ends the cell under way and publishes it.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        reason = "the samples of 10 ms; a level is an f32"
    )]
    fn publish(&mut self) {
        let mut cell = LevelCell {
            peaks: self.peaks,
            squares: [0.0; MAX_CHANNELS],
        };
        for (square, sum) in cell.squares.iter_mut().zip(&self.sums) {
            *square = (sum / self.cell as f64) as f32;
        }
        self.writer.push(cell);
        self.filled = 0;
        self.peaks = [0.0; MAX_CHANNELS];
        self.sums = [0.0; MAX_CHANNELS];
    }

    /// Clears the cell under way (when the graph is reset); the cells published stay. Does not
    /// allocate.
    pub fn reset(&mut self) {
        self.filled = 0;
        self.peaks = [0.0; MAX_CHANNELS];
        self.sums = [0.0; MAX_CHANNELS];
    }
}

/// A reader of a [`LevelTap`]: the cells that it has not read yet.
#[derive(Clone, Debug)]
pub struct LevelReader {
    reader: CellReader<LevelCell>,
    seconds: f64,
}

impl LevelReader {
    fn with_cell(self, rate: u32, cell: usize) -> Self {
        #[allow(clippy::cast_precision_loss, reason = "the samples of 10 ms")]
        let seconds = cell as f64 / f64::from(rate);
        LevelReader { seconds, ..self }
    }

    /// The seconds of a cell: 10 ms, to the sample.
    #[must_use]
    pub fn cell_seconds(&self) -> f64 {
        self.seconds
    }

    /// Gives `cell` each cell published since the last call, in order, and says how many were
    /// lost (more than [`LEVEL_CELLS_KEPT`] published in between). Not on the thread of the mix:
    /// it may spin while a write is under way.
    pub fn drain(&mut self, cell: impl FnMut(&LevelCell)) -> u64 {
        self.reader.drain(cell)
    }
}

/// What the bar of a meter follows (Custom's Level detector, p. 162).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detector {
    /// Sample Program: the peak of the samples, rising at once and falling at the decay.
    SamplePeak,
    /// VU: the RMS through two critically damped stages, 99 % of a step in 300 ms either way.
    Vu,
    /// RMS: the mean square over the last 300 ms.
    Rms,
}

/// The scale of a meter (Custom's Scale, p. 162; C-L7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    /// IEC 60268-18: −60 to −20 dB in the lower half, −20 to 0 dB in the upper.
    Iec,
    /// Quasi Linear: linear in dB from −60 to 0.
    QuasiLinear,
}

/// The peak indicator (Custom's Peak Indicator, p. 162; C-L4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeakIndicator {
    /// None.
    Off,
    /// Held 1 s, then falling at the decay.
    ShortHoldFall,
    /// Held 2 s, then falling at the decay: the default.
    MediumHoldFall,
    /// Held 2 s, then back to the peak of the moment.
    MediumHold,
    /// Held 5 s, then back to the peak of the moment.
    LongHold,
    /// Held until the meter is reset.
    InfiniteHold,
}

impl PeakIndicator {
    /// How long it holds, if it lets go at all, and whether it falls after.
    fn hold(self) -> Option<(f64, bool)> {
        match self {
            PeakIndicator::Off | PeakIndicator::InfiniteHold => None,
            PeakIndicator::ShortHoldFall => Some((PEAK_HOLD_SHORT, true)),
            PeakIndicator::MediumHoldFall => Some((PEAK_HOLD, true)),
            PeakIndicator::MediumHold => Some((PEAK_HOLD, false)),
            PeakIndicator::LongHold => Some((PEAK_HOLD_LONG, false)),
        }
    }
}

/// The meter type of the project (Project Settings › Fairlight › Audio Metering, p. 162).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum MeterType {
    /// IEC 60268-18, the default (Q15).
    #[default]
    Iec60268_18,
    /// Digital VU.
    DigitalVu,
    /// Custom.
    Custom {
        /// The level detector.
        detector: Detector,
        /// The scale.
        scale: Scale,
        /// Seconds for 20 dB of fall, clamped to 0.1–5 (C-L2).
        decay: f64,
        /// The peak indicator.
        peak: PeakIndicator,
    },
}

impl MeterType {
    /// The detector of the bar.
    #[must_use]
    pub fn detector(&self) -> Detector {
        match *self {
            MeterType::Iec60268_18 => Detector::SamplePeak,
            MeterType::DigitalVu => Detector::Vu,
            MeterType::Custom { detector, .. } => detector,
        }
    }

    /// The scale.
    #[must_use]
    pub fn scale(&self) -> Scale {
        match *self {
            MeterType::Custom { scale, .. } => scale,
            _ => Scale::Iec,
        }
    }

    /// The peak indicator.
    #[must_use]
    pub fn peak(&self) -> PeakIndicator {
        match *self {
            MeterType::Custom { peak, .. } => peak,
            _ => PeakIndicator::MediumHoldFall,
        }
    }

    /// The fall of the bar of the sample peak, in dB a second.
    #[must_use]
    pub fn bar_fall(&self) -> f64 {
        f64::from(PPM_FALL_DB) / self.decay(PPM_FALL_TIME)
    }

    /// The fall of the peak indicator, in dB a second.
    #[must_use]
    pub fn peak_fall(&self) -> f64 {
        let time = match self {
            MeterType::DigitalVu => DIGITAL_VU_PEAK_FALL_TIME,
            _ => self.decay(PPM_FALL_TIME),
        };
        f64::from(PPM_FALL_DB) / time
    }

    fn decay(&self, default: f64) -> f64 {
        match *self {
            MeterType::Custom { decay, .. } if decay.is_finite() => {
                decay.clamp(CUSTOM_DECAY_RANGE.0, CUSTOM_DECAY_RANGE.1)
            }
            _ => default,
        }
    }
}

/// Where `db` stands on a meter of `scale`, from 0 (−60 dB and under) to 1 (0 dB and over).
#[must_use]
pub fn position(db: f32, scale: Scale) -> f32 {
    let floor = METER_FLOOR_DB;
    if db.is_nan() || db <= floor {
        return 0.0;
    }
    if db >= 0.0 {
        return 1.0;
    }
    match scale {
        Scale::QuasiLinear => (db - floor) / -floor,
        Scale::Iec if db < METER_IEC_MIDDLE_DB => {
            0.5 * (db - floor) / (METER_IEC_MIDDLE_DB - floor)
        }
        Scale::Iec => 0.5 + 0.5 * (db - METER_IEC_MIDDLE_DB) / -METER_IEC_MIDDLE_DB,
    }
}

/// The decibels of a gain of amplitude; −∞ for silence.
fn db(gain: f64) -> f64 {
    20.0 * gain.log10()
}

/// The ballistics of one channel.
#[derive(Clone, Copy, Debug)]
struct Channel {
    /// The bar of the sample peak, in dB.
    bar: f64,
    /// The two stages of the VU, as gains of amplitude.
    vu: (f64, f64),
    /// The peak indicator in dB, and the cells that it still holds.
    peak: f64,
    held: u32,
}

impl Default for Channel {
    fn default() -> Self {
        Channel {
            bar: f64::NEG_INFINITY,
            vu: (0.0, 0.0),
            peak: f64::NEG_INFINITY,
            held: 0,
        }
    }
}

/// What a meter shows of a channel, in dB (−∞ for silence).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shown {
    /// The bar.
    pub bar: f64,
    /// The peak indicator, if there is one.
    pub peak: Option<f64>,
}

/// What a meter shows, made from the cells of a tap (see the module). Made off the thread of the
/// mix; it takes no memory after.
#[derive(Clone, Debug)]
pub struct Ballistics {
    meter: MeterType,
    channels: usize,
    seconds: f64,
    state: [Channel; MAX_CHANNELS],
    /// The mean squares of the last 300 ms of cells, for the RMS, the cell `n` at `n % len`.
    squares: Box<[[f32; MAX_CHANNELS]]>,
    count: u64,
}

impl Ballistics {
    /// The ballistics of `meter` for `channels`, fed cells of `seconds` (a reader's
    /// [`LevelReader::cell_seconds`]).
    #[must_use]
    pub fn new(meter: MeterType, channels: usize, seconds: f64) -> Self {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "30 cells"
        )]
        let window = (RMS_WINDOW / seconds).round().max(1.0) as usize;
        Ballistics {
            meter,
            channels: channels.min(MAX_CHANNELS),
            seconds,
            state: [Channel::default(); MAX_CHANNELS],
            squares: vec![[0.0; MAX_CHANNELS]; window].into_boxed_slice(), // rt-ok: made off it
            count: 0,
        }
    }

    /// The meter type.
    #[must_use]
    pub fn meter(&self) -> MeterType {
        self.meter
    }

    /// Takes another meter type, keeping what the channels show.
    pub fn set_meter(&mut self, meter: MeterType) {
        self.meter = meter;
    }

    /// Back to silence (the meter clicked, or Reset).
    pub fn reset(&mut self) {
        self.state = [Channel::default(); MAX_CHANNELS];
        for squares in &mut self.squares {
            *squares = [0.0; MAX_CHANNELS];
        }
        self.count = 0;
    }

    /// Hears a cell.
    pub fn push(&mut self, cell: &LevelCell) {
        let len = self.squares.len();
        #[allow(clippy::cast_possible_truncation, reason = "below the length")]
        let at = (self.count % len as u64) as usize;
        for (kept, &square) in self.squares[at].iter_mut().zip(&cell.squares) {
            *kept = if square.is_finite() {
                square.max(0.0)
            } else {
                0.0
            };
        }
        self.count += 1;
        let dt = self.seconds;
        let bar_fall = self.meter.bar_fall() * dt;
        let peak_fall = self.meter.peak_fall() * dt;
        let indicator = self.meter.peak();
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "at most 500 cells"
        )]
        let hold = indicator
            .hold()
            .map_or(0, |(hold, _)| (hold / dt).round() as u32);
        let vu_step = 1.0 - (-dt / VU_STAGE).exp();
        for channel in 0..self.channels {
            let state = &mut self.state[channel];
            // A level that is not a finite number is heard as silence (the taps never give one).
            let peak = db(f64::from(cell.peaks[channel]));
            let peak = if peak.is_finite() {
                peak
            } else {
                f64::NEG_INFINITY
            };
            state.bar = peak.max(state.bar - bar_fall);
            let rms = f64::from(self.squares[at][channel]).sqrt();
            state.vu.0 += (rms - state.vu.0) * vu_step;
            state.vu.1 += (state.vu.0 - state.vu.1) * vu_step;
            if peak >= state.peak {
                state.peak = peak;
                state.held = hold;
            } else if let Some((_, falls)) = indicator.hold() {
                if state.held > 0 {
                    state.held -= 1;
                } else if falls {
                    state.peak = peak.max(state.peak - peak_fall);
                } else {
                    state.peak = peak;
                    state.held = hold;
                }
            }
        }
    }

    /// What the meter shows of `channel`.
    #[must_use]
    #[allow(clippy::cast_precision_loss, reason = "30 cells")]
    pub fn shown(&self, channel: usize) -> Shown {
        let state = &self.state[channel];
        let bar = match self.meter.detector() {
            Detector::SamplePeak => state.bar,
            Detector::Vu => db(state.vu.1),
            Detector::Rms => {
                let sum: f64 = self
                    .squares
                    .iter()
                    .map(|squares| f64::from(squares[channel]))
                    .sum();
                10.0 * (sum / self.squares.len() as f64).log10()
            }
        };
        let peak = (self.meter.peak() != PeakIndicator::Off).then_some(state.peak);
        Shown { bar, peak }
    }
}
