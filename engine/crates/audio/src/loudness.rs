//! Loudness after ITU-R BS.1770-4/-5 and EBU Tech 3341/3342 (plan T9): Momentary, Short Term,
//! their maxima, Integrated, Loudness Range and the true peak, the same code on the bus that is
//! monitored and over a whole render (`crate::analysis`).
//!
//! # Two halves
//!
//! The thread of the mix runs a [`LoudnessTap`]: it weights each channel by the K filter of
//! BS.1770 (annex 1: the shelf of the head and the high pass of RLB, their coefficients for any
//! rate as libebur128 works them out), sums the weighted power of every channel into a cell of
//! 10 ms ([`METER_CELL`]), with the true peak (`fx::true_peak`, the interpolator that the Limiter
//! hears by) and the sample peak of the cell, and publishes the last
//! [`LOUDNESS_CELLS_KEPT`] cells, each in a seqlock of its own
//! ([`rt::cells`](mod@crate::rt::cells)). It takes no memory once made and never waits.
//!
//! Another thread reads them ([`LoudnessReader`]) into a [`Loudness`], which keeps what a meter
//! keeps: the windows of 400 ms and 3 s, the maxima, and the histograms of the blocks that make
//! the integrated loudness and the loudness range. It takes its memory when made and none after,
//! so that it could run on the thread of the mix too; it outlives the graph (a graph swapped in
//! gives a new tap, read into the same `Loudness`, which only [`Loudness::reset`] clears: Lock
//! Metering to Transport, p. 3986).
//!
//! # The measure (BS.1770-4/-5 annex 1; Tech 3341 V4; Tech 3342 V4)
//!
//! - The loudness of a stretch is −0.691 + 10·log10 of the sum over the channels of their weight
//!   ([`weight`]: 1, 1.41 for the surrounds, 0 for the LFE) times the mean square of the
//!   K-weighted channel.
//! - Momentary: the last 400 ms; Short Term: the last 3 s; both ungated, worked out at every cell,
//!   silence counted before the start.
//! - Integrated: blocks of 400 ms every 100 ms from the start, an incomplete last block dropped;
//!   the blocks above −70 LUFS, then those above their power mean less 10 LU, power-averaged.
//!   BS.1770-1 had no gate: [`Reading::integrated_ungated`].
//! - Loudness Range: the short-term loudness every 100 ms once 3 s have been heard; those above
//!   −70 LUFS, then those above their power mean less 20 LU; the 95th percentile less the 10th,
//!   each the value of rank ⌊n·p + 0.5⌋ (from 1, at least 1, in whole numbers), as ffmpeg's
//!   `ebur128` takes them at both ends (`f_ebur128.c`; convention 5). Tech 3342's code and
//!   libebur128 take the value at round((n − 1)·p) from 0 instead, which differs by a value: it
//!   tells only over a short sound (10 s at −60 then −75: 3.8 LU against 3.5).
//! - A sample that is not a finite number (∞, NaN) is heard as silence, so that the filters and
//!   the peaks go on (the review of T9).
//! - The histograms have bins of 0.01 LU ([`LOUDNESS_BIN`]) that keep the count and the sum of
//!   the energy of their blocks: the means are exact, and a gate is placed to the bin.

use crate::constants::{
    samples, LOUDNESS_ABSOLUTE_GATE, LOUDNESS_BIN, LOUDNESS_BLOCK_STEP, LOUDNESS_CEILING,
    LOUDNESS_CELLS_KEPT, LOUDNESS_MOMENTARY, LOUDNESS_RANGE_GATE, LOUDNESS_RANGE_PERCENTILES,
    LOUDNESS_RELATIVE_GATE, LOUDNESS_SHORT_TERM, LOUDNESS_SURROUND_WEIGHT, MAX_CHANNELS,
    METER_CELL,
};
use crate::fx::true_peak::{TruePeak, DELAY};
use crate::node::{Layout, Role};
use crate::rt::{cells, CellReader, CellWriter, Words};

/// The offset of the loudness of BS.1770: −0.691 dB, so that a sine of 1 kHz at 0 dBFS in one
/// channel reads −3.01 LKFS.
pub const OFFSET: f64 = -0.691;

/// The loudness, in LUFS, of a mean weighted power; −∞ for silence.
#[must_use]
pub fn lufs(energy: f64) -> f64 {
    if energy > 0.0 {
        OFFSET + 10.0 * energy.log10()
    } else {
        f64::NEG_INFINITY
    }
}

/// The mean weighted power of a loudness in LUFS.
#[must_use]
pub fn energy(lufs: f64) -> f64 {
    10.0_f64.powf((lufs - OFFSET) / 10.0)
}

/// The weight of the power of a channel (BS.1770-4/-5, table 4): 1.41 for the surrounds at 60°
/// to 120° from the front (Ls and Rs of 5.1, the side surrounds of 7.1), 0 for the LFE, which
/// loudness leaves out (p. 3985), and 1 for every other (the front, the one surround of LCRS
/// behind, the rear surrounds of 7.1, mono, the channels of an Adaptive track).
#[must_use]
pub fn weight(role: Role) -> f64 {
    match role {
        Role::Lfe => 0.0,
        Role::LeftSurround | Role::RightSurround => LOUDNESS_SURROUND_WEIGHT,
        _ => 1.0,
    }
}

/// The rank, from 1, of the value at `percent` (whole percents) of `count` sorted values:
/// ⌊count · percent / 100 + 0.5⌋, at least 1, in whole numbers, as ffmpeg's `ebur128` takes it
/// (`f_ebur128.c`, at both ends of the loudness range; see the module).
#[must_use]
pub fn percentile_rank(count: u64, percent: u64) -> u64 {
    ((count * percent + 50) / 100).max(1)
}

/// `x`, or silence when it is not a finite number.
#[inline]
pub(crate) fn finite(x: f32) -> f32 {
    if x.is_finite() {
        x
    } else {
        0.0
    }
}

/// The samples of a cell at `rate`: 10 ms, to the nearest sample.
#[must_use]
pub fn cell_frames(rate: u32) -> usize {
    samples(METER_CELL, rate).max(1)
}

/// The two stages of the K filter, as one filter of order 4 in two biquads: the shelf (stage 1)
/// and the high pass (stage 2, RLB). The coefficients of BS.1770 are given at 48 kHz; these are
/// the analogue prototypes that give them, taken to `rate` by the bilinear transform
/// (libebur128, `ebur128_init_filter`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KFilter {
    /// The numerator and the denominator (a0 = 1) of the shelf.
    pub shelf: ([f64; 3], [f64; 3]),
    /// The numerator and the denominator of the high pass.
    pub high_pass: ([f64; 3], [f64; 3]),
}

impl KFilter {
    /// The K filter at `rate`.
    #[must_use]
    #[allow(non_snake_case, reason = "the names of the cookbook")]
    pub fn new(rate: u32) -> Self {
        let rate = f64::from(rate);
        let f0 = 1_681.974_450_955_533;
        let G = 3.999_843_853_973_347;
        let Q = 0.707_175_236_955_419_6;
        let K = (std::f64::consts::PI * f0 / rate).tan();
        let Vh = 10.0_f64.powf(G / 20.0);
        let Vb = Vh.powf(0.499_666_774_154_541_6);
        let a0 = 1.0 + K / Q + K * K;
        let shelf = (
            [
                (Vh + Vb * K / Q + K * K) / a0,
                2.0 * (K * K - Vh) / a0,
                (Vh - Vb * K / Q + K * K) / a0,
            ],
            [1.0, 2.0 * (K * K - 1.0) / a0, (1.0 - K / Q + K * K) / a0],
        );
        let f0 = 38.135_470_876_024_44;
        let Q = 0.500_327_037_323_877_3;
        let K = (std::f64::consts::PI * f0 / rate).tan();
        let a0 = 1.0 + K / Q + K * K;
        let high_pass = (
            [1.0, -2.0, 1.0],
            [1.0, 2.0 * (K * K - 1.0) / a0, (1.0 - K / Q + K * K) / a0],
        );
        KFilter { shelf, high_pass }
    }

    /// The gain of the filter at `frequency` Hz, at `rate`, as a power.
    #[must_use]
    pub fn power_at(&self, frequency: f64, rate: u32) -> f64 {
        let w = std::f64::consts::TAU * frequency / f64::from(rate);
        let stage = |(b, a): ([f64; 3], [f64; 3])| {
            let response = |c: [f64; 3]| {
                let re = c[0] + c[1] * w.cos() + c[2] * (2.0 * w).cos();
                let im = -c[1] * w.sin() - c[2] * (2.0 * w).sin();
                re * re + im * im
            };
            response(b) / response(a)
        };
        stage(self.shelf) * stage(self.high_pass)
    }
}

/// The state of the K filter of one channel: the last two inputs, outputs of the shelf, and
/// outputs of the high pass (direct form I).
#[derive(Clone, Copy, Debug, Default)]
struct KState([f64; 6]);

impl KState {
    #[inline]
    #[allow(
        clippy::many_single_char_names,
        reason = "the names of a difference equation"
    )]
    fn push(&mut self, filter: &KFilter, x: f64) -> f64 {
        let [x1, x2, s1, s2, k1, k2] = self.0;
        let (b, a) = filter.shelf;
        let s = b[0] * x + b[1] * x1 + b[2] * x2 - a[1] * s1 - a[2] * s2;
        let (b, a) = filter.high_pass;
        let k = b[0] * s + b[1] * s1 + b[2] * s2 - a[1] * k1 - a[2] * k2;
        self.0 = [x, x1, crate::rt::flush(s), s1, crate::rt::flush(k), k1];
        k
    }
}

/// What the thread of the mix publishes of 10 ms of a bus: the mean of the weighted power of its
/// channels after the K filter, and the true peak and the sample peak of its channels, as gains
/// of amplitude.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cell {
    /// The sum over the channels of their weight times their mean square after the K filter.
    pub energy: f64,
    /// The largest size between the samples of any channel (8 points to an interval), the
    /// intervals read 24 samples late (`fx::true_peak`); 0 when the tap does not read it.
    pub true_peak: f32,
    /// The largest size of a sample of any channel.
    pub sample_peak: f32,
}

impl Words for Cell {
    const WORDS: usize = 4;
    fn word(&self, index: usize) -> u32 {
        (self.energy, self.true_peak, self.sample_peak).word(index)
    }
    fn from_words(word: impl Fn(usize) -> u32) -> Self {
        let (energy, true_peak, sample_peak) = <(f64, f32, f32)>::from_words(word);
        Cell {
            energy,
            true_peak,
            sample_peak,
        }
    }
}

/// The half of the loudness that runs on the thread of the mix (see the module). Made by
/// [`loudness_tap`], off it.
#[derive(Debug)]
pub struct LoudnessTap {
    channels: usize,
    weights: [f64; MAX_CHANNELS],
    filter: KFilter,
    states: [KState; MAX_CHANNELS],
    /// The true peak of each channel, when the tap reads it.
    true_peaks: Box<[TruePeak]>,
    /// The samples of a cell, and how many of the cell under way have been heard.
    cell: usize,
    filled: usize,
    /// The sum of the squares of each channel through the cell under way, and its peaks.
    sums: [f64; MAX_CHANNELS],
    true_peak: f64,
    sample_peak: f32,
    writer: CellWriter<Cell>,
}

/// The tap of the loudness of a bus of `layout` at `rate`, reading the true peak if `true_peak`,
/// and its first reader. Off the thread of the mix: it takes its memory here.
#[must_use]
pub fn loudness_tap(rate: u32, layout: Layout, true_peak: bool) -> (LoudnessTap, LoudnessReader) {
    let (writer, reader) = cells(LOUDNESS_CELLS_KEPT, Cell::default());
    let channels = layout.channels();
    let mut weights = [0.0; MAX_CHANNELS];
    for (weight, &role) in weights.iter_mut().zip(layout.roles()) {
        *weight = self::weight(role);
    }
    let true_peaks = if true_peak {
        vec![TruePeak::new(); channels].into_boxed_slice() // rt-ok: made off the thread of the mix
    } else {
        Box::default()
    };
    let tap = LoudnessTap {
        channels,
        weights,
        filter: KFilter::new(rate),
        states: [KState::default(); MAX_CHANNELS],
        true_peaks,
        cell: cell_frames(rate),
        filled: 0,
        sums: [0.0; MAX_CHANNELS],
        true_peak: 0.0,
        sample_peak: 0.0,
        writer,
    };
    (tap, LoudnessReader { reader })
}

impl LoudnessTap {
    /// The channels it hears.
    #[must_use]
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// The samples of a cell.
    #[must_use]
    pub fn cell(&self) -> usize {
        self.cell
    }

    /// Another reader, off the thread of the mix; it reads the cells published from now on.
    #[must_use]
    pub fn reader(&self) -> LoudnessReader {
        LoudnessReader {
            reader: self.writer.reader(),
        }
    }

    /// Hears a block: `input` planar, a slice for each channel (as many as the tap's; more are
    /// not heard, fewer are silence), all of one length. Publishes each cell that it completes.
    /// Does not allocate, lock or wait.
    pub fn take(&mut self, input: &[&[f32]]) {
        let frames = input.first().map_or(0, |channel| channel.len());
        let channels = self.channels.min(input.len());
        let mut at = 0;
        while at < frames {
            let end = frames.min(at + self.cell - self.filled);
            for (channel, samples) in input[..channels].iter().enumerate() {
                let state = &mut self.states[channel];
                let mut sum = self.sums[channel];
                let mut peak = self.sample_peak;
                for &x in &samples[at..end] {
                    let x = finite(x);
                    let y = state.push(&self.filter, f64::from(x));
                    sum += y * y;
                    peak = peak.max(x.abs());
                }
                self.sums[channel] = sum;
                self.sample_peak = peak;
                if let Some(detector) = self.true_peaks.get_mut(channel) {
                    let mut true_peak = self.true_peak;
                    for &x in &samples[at..end] {
                        true_peak = true_peak.max(detector.push(f64::from(finite(x))));
                    }
                    self.true_peak = true_peak;
                }
            }
            for channel in channels..self.channels {
                // A channel not given is silence through the filter.
                let state = &mut self.states[channel];
                let mut sum = self.sums[channel];
                for _ in at..end {
                    let y = state.push(&self.filter, 0.0);
                    sum += y * y;
                }
                self.sums[channel] = sum;
                if let Some(detector) = self.true_peaks.get_mut(channel) {
                    for _ in at..end {
                        self.true_peak = self.true_peak.max(detector.push(0.0));
                    }
                }
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
        reason = "the samples of 10 ms; a peak is an f32"
    )]
    fn publish(&mut self) {
        let mut energy = 0.0;
        for channel in 0..self.channels {
            energy += self.weights[channel] * self.sums[channel];
        }
        let cell = Cell {
            energy: energy / self.cell as f64,
            true_peak: self.true_peak as f32,
            sample_peak: self.sample_peak,
        };
        self.writer.push(cell);
        self.filled = 0;
        self.sums = [0.0; MAX_CHANNELS];
        self.true_peak = 0.0;
        self.sample_peak = 0.0;
    }

    /// Back to silence: the filters, the true peak and the cell under way are cleared; the cells
    /// published stay (on a seek, or when the graph is swapped). Does not allocate.
    pub fn reset(&mut self) {
        self.states = [KState::default(); MAX_CHANNELS];
        for detector in &mut self.true_peaks {
            detector.reset();
        }
        self.filled = 0;
        self.sums = [0.0; MAX_CHANNELS];
        self.true_peak = 0.0;
        self.sample_peak = 0.0;
    }

    /// The end of the sound: the true peak and the sample peak of the cell under way, which is
    /// never published, and of the last 24 samples, which the true peak has not read yet (it reads
    /// silence after them). The loudness of the cell under way is dropped, as BS.1770 drops an
    /// incomplete block. Not on the thread of the mix.
    #[must_use]
    #[allow(clippy::cast_possible_truncation, reason = "a peak is an f32")]
    pub fn finish(&mut self) -> (f32, f32) {
        let mut true_peak = self.true_peak;
        for detector in &mut self.true_peaks {
            for _ in 0..DELAY {
                true_peak = true_peak.max(detector.push(0.0));
            }
        }
        (true_peak as f32, self.sample_peak)
    }
}

/// A reader of a [`LoudnessTap`]: the cells that it has not read yet.
#[derive(Clone, Debug)]
pub struct LoudnessReader {
    reader: CellReader<Cell>,
}

impl LoudnessReader {
    /// Gives `cell` each cell published since the last call, in order, and says how many were
    /// lost (written over before they were read: more than [`LOUDNESS_CELLS_KEPT`] in
    /// between). Not on the thread of the mix: it may spin while a
    /// write is under way.
    pub fn drain(&mut self, mut cell: impl FnMut(Cell)) -> u64 {
        self.reader.drain(|&value| cell(value))
    }

    /// The cells published that it has not read yet.
    #[must_use]
    pub fn waiting(&self) -> u64 {
        self.reader.waiting()
    }
}

/// A histogram of the loudness of blocks, in bins of [`LOUDNESS_BIN`] from the absolute gate to
/// [`LOUDNESS_CEILING`], each with the count and the sum of the energy of its blocks.
#[derive(Clone, Debug)]
struct Histogram {
    counts: Box<[u64]>,
    energies: Box<[f64]>,
    count: u64,
    energy: f64,
}

impl Histogram {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "10 000 bins"
    )]
    fn new() -> Self {
        let bins = ((LOUDNESS_CEILING - f64::from(LOUDNESS_ABSOLUTE_GATE)) / LOUDNESS_BIN).round()
            as usize
            + 1;
        Histogram {
            counts: vec![0; bins].into_boxed_slice(), // rt-ok: made off the thread of the mix
            energies: vec![0.0; bins].into_boxed_slice(), // rt-ok: made off the thread of the mix
            count: 0,
            energy: 0.0,
        }
    }

    /// The bin of a loudness above the absolute gate.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to the bins"
    )]
    fn bin(&self, lufs: f64) -> usize {
        let bin = ((lufs - f64::from(LOUDNESS_ABSOLUTE_GATE)) / LOUDNESS_BIN).floor();
        (bin.max(0.0) as usize).min(self.counts.len() - 1)
    }

    /// Counts a block of `energy`, if it is above the absolute gate.
    fn add(&mut self, energy: f64) {
        let loudness = lufs(energy);
        if loudness <= f64::from(LOUDNESS_ABSOLUTE_GATE) {
            return;
        }
        let bin = self.bin(loudness);
        self.counts[bin] += 1;
        self.energies[bin] += energy;
        self.count += 1;
        self.energy += energy;
    }

    fn clear(&mut self) {
        self.counts.fill(0);
        self.energies.fill(0.0);
        self.count = 0;
        self.energy = 0.0;
    }

    /// The first bin of the blocks that pass the gate `relative` LU under the power mean of
    /// those above the absolute gate; `None` when there is none.
    #[allow(clippy::cast_precision_loss, reason = "counts of blocks")]
    fn gate(&self, relative: f32) -> Option<usize> {
        (self.count > 0).then(|| {
            let mean = self.energy / self.count as f64;
            self.bin(lufs(mean) + f64::from(relative))
        })
    }

    /// The loudness of the power mean of the blocks that pass the gate `relative`.
    #[allow(clippy::cast_precision_loss, reason = "counts of blocks")]
    fn gated(&self, relative: f32) -> Option<f64> {
        let first = self.gate(relative)?;
        let count: u64 = self.counts[first..].iter().sum();
        let energy: f64 = self.energies[first..].iter().sum();
        (count > 0).then(|| lufs(energy / count as f64))
    }

    /// The loudness at `percent` of the blocks that pass the gate `relative`: the mean of the bin
    /// of the block of rank [`percentile_rank`] in order.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "counts of blocks"
    )]
    fn percentile(&self, relative: f32, percent: u64) -> Option<f64> {
        let first = self.gate(relative)?;
        let count: u64 = self.counts[first..].iter().sum();
        if count == 0 {
            return None;
        }
        let rank = percentile_rank(count, percent);
        let mut seen = 0;
        for bin in first..self.counts.len() {
            seen += self.counts[bin];
            if seen >= rank {
                return Some(lufs(self.energies[bin] / self.counts[bin] as f64));
            }
        }
        None
    }
}

/// What a loudness meter reads (see the module), in LUFS and LU; a loudness of silence is −∞.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    /// The last 400 ms.
    pub momentary: f64,
    /// The last 3 s.
    pub short_term: f64,
    /// The loudest Momentary since the start or the reset.
    pub momentary_max: f64,
    /// The loudest Short Term (Short Max, p. 3985).
    pub short_term_max: f64,
    /// Integrated, gated (BS.1770-2 and later); `None` until a block passes the gates.
    pub integrated: Option<f64>,
    /// Integrated without the gates, the power mean of every block (BS.1770-1); `None` before
    /// the first block of 400 ms.
    pub integrated_ungated: Option<f64>,
    /// The power mean of every cell heard, ungated, however short the sound: what Level Matcher
    /// falls back to when a clip is shorter than a block (C-C7); `None` before a cell.
    pub mean: Option<f64>,
    /// Loudness Range, in LU; `None` until a short-term value passes the gates.
    pub range: Option<f64>,
    /// The true peak of any channel, in dBTP; −∞ when the tap does not read it.
    pub true_peak: f64,
    /// The sample peak of any channel, in dBFS.
    pub sample_peak: f64,
    /// The seconds heard (cells of [`METER_CELL`]).
    pub seconds: f64,
}

/// The half of the loudness that keeps the windows, the maxima and the histograms (see the
/// module): fed [`Cell`]s in order.
#[derive(Clone, Debug)]
pub struct Loudness {
    /// The energies of the last 3 s of cells, the cell `n` at `n % len`.
    cells: Box<[f64]>,
    momentary_cells: usize,
    step_cells: u64,
    count: u64,
    momentary: f64,
    short_term: f64,
    momentary_max: f64,
    short_term_max: f64,
    blocks: Histogram,
    short_terms: Histogram,
    ungated: (f64, u64),
    mean: (f64, u64),
    true_peak: f32,
    sample_peak: f32,
}

impl Default for Loudness {
    fn default() -> Self {
        Loudness::new()
    }
}

impl Loudness {
    /// A meter that has heard nothing. It takes its memory here (about 330 kB) and none after.
    #[must_use]
    pub fn new() -> Self {
        let cells = |seconds: f64| {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a few hundred cells"
            )]
            let cells = (seconds / METER_CELL).round() as usize;
            cells
        };
        Loudness {
            cells: vec![0.0; cells(LOUDNESS_SHORT_TERM)].into_boxed_slice(), // rt-ok: made off it
            momentary_cells: cells(LOUDNESS_MOMENTARY),
            step_cells: cells(LOUDNESS_BLOCK_STEP) as u64,
            count: 0,
            momentary: f64::NEG_INFINITY,
            short_term: f64::NEG_INFINITY,
            momentary_max: f64::NEG_INFINITY,
            short_term_max: f64::NEG_INFINITY,
            blocks: Histogram::new(),
            short_terms: Histogram::new(),
            ungated: (0.0, 0),
            mean: (0.0, 0),
            true_peak: 0.0,
            sample_peak: 0.0,
        }
    }

    /// Back to nothing heard (Reset, p. 3986; and a seek while Lock Metering to Transport is
    /// on). Does not allocate.
    pub fn reset(&mut self) {
        self.cells.fill(0.0);
        self.count = 0;
        self.momentary = f64::NEG_INFINITY;
        self.short_term = f64::NEG_INFINITY;
        self.momentary_max = f64::NEG_INFINITY;
        self.short_term_max = f64::NEG_INFINITY;
        self.blocks.clear();
        self.short_terms.clear();
        self.ungated = (0.0, 0);
        self.mean = (0.0, 0);
        self.true_peak = 0.0;
        self.sample_peak = 0.0;
    }

    /// The mean energy of the last `cells` cells, silence before the first.
    #[allow(clippy::cast_precision_loss, reason = "a few hundred cells")]
    fn window(&self, cells: usize) -> f64 {
        let len = self.cells.len() as u64;
        let heard = self.count.min(cells as u64);
        let mut sum = 0.0;
        for n in self.count - heard..self.count {
            #[allow(clippy::cast_possible_truncation, reason = "below the length")]
            let at = (n % len) as usize;
            sum += self.cells[at];
        }
        sum / cells as f64
    }

    /// Hears a cell.
    pub fn push(&mut self, cell: Cell) {
        let len = self.cells.len();
        #[allow(clippy::cast_possible_truncation, reason = "below the length")]
        let at = (self.count % len as u64) as usize;
        self.cells[at] = cell.energy;
        self.count += 1;
        self.mean.0 += cell.energy;
        self.mean.1 += 1;
        let momentary = self.window(self.momentary_cells);
        let short_term = self.window(len);
        self.momentary = lufs(momentary);
        self.short_term = lufs(short_term);
        self.momentary_max = self.momentary_max.max(self.momentary);
        self.short_term_max = self.short_term_max.max(self.short_term);
        if self.count.is_multiple_of(self.step_cells) {
            if self.count >= self.momentary_cells as u64 {
                self.blocks.add(momentary);
                self.ungated.0 += momentary;
                self.ungated.1 += 1;
            }
            if self.count >= len as u64 {
                self.short_terms.add(short_term);
            }
        }
        self.true_peak = self.true_peak.max(cell.true_peak);
        self.sample_peak = self.sample_peak.max(cell.sample_peak);
    }

    /// Takes the peaks of the end of the sound ([`LoudnessTap::finish`]).
    pub fn push_peaks(&mut self, true_peak: f32, sample_peak: f32) {
        self.true_peak = self.true_peak.max(true_peak);
        self.sample_peak = self.sample_peak.max(sample_peak);
    }

    /// What it reads now.
    #[must_use]
    #[allow(clippy::cast_precision_loss, reason = "counts of blocks")]
    pub fn reading(&self) -> Reading {
        let range = match (
            self.short_terms
                .percentile(LOUDNESS_RANGE_GATE, LOUDNESS_RANGE_PERCENTILES.0),
            self.short_terms
                .percentile(LOUDNESS_RANGE_GATE, LOUDNESS_RANGE_PERCENTILES.1),
        ) {
            (Some(low), Some(high)) => Some(high - low),
            _ => None,
        };
        let mean = |(energy, count): (f64, u64)| (count > 0).then(|| lufs(energy / count as f64));
        Reading {
            momentary: self.momentary,
            short_term: self.short_term,
            momentary_max: self.momentary_max,
            short_term_max: self.short_term_max,
            integrated: self.blocks.gated(LOUDNESS_RELATIVE_GATE),
            integrated_ungated: mean(self.ungated),
            mean: mean(self.mean),
            range,
            true_peak: 20.0 * f64::from(self.true_peak).log10(),
            sample_peak: 20.0 * f64::from(self.sample_peak).log10(),
            seconds: self.count as f64 * METER_CELL,
        }
    }
}
