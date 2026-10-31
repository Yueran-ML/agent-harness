//! The arithmetic of the scopes, done alike on the card (`scopes.wgsl`) and here, and the scopes
//! and statistics of a frame worked out on the processor: the reference that the card's are held
//! to.
//!
//! A channel is made an integer code first ([`code`]: a signal of 1 is [`ONE`]); from there on,
//! luma, chroma, levels and bins are integer arithmetic, which the card does as the processor
//! does, so the counts of the card are these exactly. Only the step from a float to a code is in
//! floats: a multiple by a power of two, exact, and a rounding to even, which both do alike.

use columbina_model::Gamma;

use super::graticule::{hlg_scene, pq_nits};
use super::{Bins, Channel, Chroma, Kind, Primaries, Signal, Statistics, Tones, WaveformChannels};

/// The code of a signal of 1: a channel is made an integer of 1/16384 of it.
pub(crate) const ONE: i32 = 1 << 14;
/// [`ONE`] as a float.
const ONE_FLOAT: f32 = 16384.0;
/// The range that a channel is held to before its luma and chroma are made: −1 to 2, so that
/// the sums stay within 32 bits.
pub(crate) const WIDE: (f32, f32) = (-1.0, 2.0);
/// No bin: the point falls outside the scope (or is not counted in it).
pub(crate) const NONE: u32 = u32::MAX;
/// How many codes the table of linear light has: one for each code from 0 to [`ONE`].
pub(crate) const TABLE: usize = 16385;
/// Linear light in the table, a value of 1 being this.
const LINEAR_ONE: f64 = 32768.0;
/// The matrix to XYZ is of integers of 1/8192.
const MATRIX_ONE: f64 = 8192.0;
/// The size of a coefficient of the matrix to XYZ that the sums of 32 bits still hold.
pub(crate) const MATRIX_LARGEST: f64 = 2.0;
/// The chromaticity diagram spans x from 0 to 0.8 and y from 0 to 0.9 (as 4/5 and 9/10).
pub(crate) const CIE_X: f64 = 0.8;
pub(crate) const CIE_Y: f64 = 0.9;

/// The code of a channel: held to `low..=high`, as an integer of 1/16384.
#[allow(
    clippy::cast_possible_truncation,
    reason = "at most 2 × 16384, made an integer"
)]
pub(crate) fn code(value: f32, low: f32, high: f32) -> i32 {
    // max, then min, as the card does: a NaN becomes `low`.
    (value.max(low).min(high) * ONE_FLOAT).round_ties_even() as i32
}

/// The codes of R', G' and B', in the wide range.
pub(crate) fn wide(pixel: [f32; 4]) -> [i32; 3] {
    [0, 1, 2].map(|channel| code(pixel[channel], WIDE.0, WIDE.1))
}

/// The level (or bin) of a code among `levels`, from 0 for a signal of 0 to `levels − 1` for 1:
/// rounded, so that 1024 levels are the codes of 10 bits.
#[allow(clippy::cast_sign_loss, reason = "held to 0..=16384 first")]
pub(crate) fn level(code: i32, levels: u32) -> u32 {
    (code.clamp(0, ONE) as u32 * (levels - 1) + (1 << 13)) >> 14
}

/// The luma coefficients as integers of 1/32768, and those of chroma (1 / (2 (1 − K))).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Coefficients {
    pub kr: i32,
    pub kg: i32,
    pub kb: i32,
    pub cb: i32,
    pub cr: i32,
}

impl Coefficients {
    /// The integers of `luma` (red, green, blue): green is what red and blue leave of 1.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "coefficients from 0 to 1, of 1/32768"
    )]
    pub fn of(luma: [f32; 3]) -> Coefficients {
        let integer = |k: f32| (f64::from(k) * 32768.0).round() as i32;
        let kr = integer(luma[0]);
        let kb = integer(luma[2]);
        // 1 / (2 (1 − K)) of 1/32768, from the K of integers.
        let chroma =
            |k: i32| (f64::from(1u32 << 30) / f64::from(2 * ((1 << 15) - k))).round() as i32;
        Coefficients {
            kr,
            kg: (1 << 15) - kr - kb,
            kb,
            cb: chroma(kb),
            cr: chroma(kr),
        }
    }

    /// The luma code of codes of R', G' and B'.
    pub fn luma(self, [r, g, b]: [i32; 3]) -> i32 {
        (self.kr * r + self.kg * g + self.kb * b + (1 << 14)) >> 15
    }

    /// The codes of Cb and Cr of codes of R', G' and B' whose luma is `y`.
    pub fn chroma(self, [r, _, b]: [i32; 3], y: i32) -> (i32, i32) {
        (
            ((b - y) * self.cb + (1 << 14)) >> 15,
            ((r - y) * self.cr + (1 << 14)) >> 15,
        )
    }
}

/// What the vectorscope counts, as integers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Vector {
    pub size: u32,
    /// The zoom, of 1/16.
    pub zoom: i32,
    pub tones: Tones,
    /// The luma codes that part the shadows, the mid-tones and the highlights.
    pub low: i32,
    pub high: i32,
}

impl Vector {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "a zoom of 1 to 8, and ranges from 0 to 1"
    )]
    pub fn of(size: u32, zoom: f32, tones: Tones, low: f32, high: f32) -> Vector {
        Vector {
            size,
            zoom: (zoom * 16.0).round() as i32,
            tones,
            low: (f64::from(low) * f64::from(ONE)).round() as i32,
            high: (f64::from(high) * f64::from(ONE)).round() as i32,
        }
    }

    /// The bin of a pixel's codes (R', G', B' in the wide range): rows from the top (Cr up),
    /// columns from the left (Cb right); [`NONE`] where it is not of the tones shown or falls
    /// outside, zoomed. A Cb or Cr of exactly 0.5 (the edge) is in the last bin.
    #[allow(
        clippy::cast_possible_wrap,
        clippy::cast_sign_loss,
        reason = "a size of at most 1024, and a bin checked to be in it"
    )]
    pub fn bin(self, coefficients: Coefficients, rgb: [i32; 3]) -> u32 {
        let y = coefficients.luma(rgb);
        let shown = match self.tones {
            Tones::All => true,
            Tones::Low => y < self.low,
            Tones::Mid => y >= self.low && y <= self.high,
            Tones::High => y > self.high,
        };
        if !shown {
            return NONE;
        }
        let (cb, cr) = coefficients.chroma(rgb, y);
        let n = self.size as i32;
        let x = ((((cb * self.zoom) >> 4) * n) >> 14) + n / 2;
        let up = ((((cr * self.zoom) >> 4) * n) >> 14) + n / 2;
        // The edge (a Cb or Cr of 0.5, of a bar of 100 %) is in the last bin.
        if x < 0 || x > n || up < 0 || up > n {
            return NONE;
        }
        (n - 1 - up.min(n - 1)) as u32 * self.size + x.min(n - 1) as u32
    }
}

/// What the chromaticity diagram counts, as integers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Chromaticity {
    pub width: u32,
    pub height: u32,
    /// R'G'B' to XYZ, of 1/8192, by rows.
    pub matrix: [i32; 9],
    /// The linear light of each code from 0 to [`ONE`], of 1/32768.
    pub table: Vec<u32>,
}

impl Chromaticity {
    pub fn of(width: u32, height: u32, space: &Primaries, transfer: Gamma) -> Chromaticity {
        Chromaticity {
            width,
            height,
            matrix: integer_matrix(&space.to_xyz()),
            table: linear_table(transfer),
        }
    }

    /// The bin of a pixel's codes (R', G', B' in the wide range): rows from the top (y up),
    /// columns from the left (x right); [`NONE`] for black, and where x or y are outside the
    /// diagram.
    #[allow(
        clippy::cast_possible_wrap,
        clippy::cast_sign_loss,
        reason = "codes held to 0..=16384, linear light of at most 32768, and sizes of at most 1024"
    )]
    pub fn bin(&self, rgb: [i32; 3]) -> u32 {
        let light = rgb.map(|code| self.table[code.clamp(0, ONE) as usize] as i32);
        let tristimulus = |row: usize| {
            let m = &self.matrix[row * 3..row * 3 + 3];
            (m[0] * light[0] + m[1] * light[1] + m[2] * light[2]) >> 12
        };
        let [cie_x, cie_y, cie_z] = [0, 1, 2].map(tristimulus);
        let sum = cie_x + cie_y + cie_z;
        if sum <= 0 || cie_x < 0 || cie_y < 0 {
            return NONE;
        }
        let (width, height) = (self.width as i32, self.height as i32);
        let column = cie_x * width * 5 / (sum * 4);
        let row = cie_y * height * 10 / (sum * 9);
        if column >= width || row >= height {
            return NONE;
        }
        (height - 1 - row) as u32 * self.width + column as u32
    }
}

/// A matrix of floats as integers of 1/8192.
#[allow(
    clippy::cast_possible_truncation,
    reason = "coefficients of less than 2, checked"
)]
fn integer_matrix(matrix: &[[f64; 3]; 3]) -> [i32; 9] {
    let mut integers = [0; 9];
    for (row, values) in matrix.iter().enumerate() {
        for (column, value) in values.iter().enumerate() {
            integers[row * 3 + column] = (value * MATRIX_ONE).round() as i32;
        }
    }
    integers
}

/// Linear light of a signal of `transfer`, from 0 to 1: of the display for a gamma, of the
/// scene for HLG, and of 10 000 nits for PQ.
pub(crate) fn linear(transfer: Gamma, signal: f64) -> f64 {
    let signal = signal.clamp(0.0, 1.0);
    match transfer {
        Gamma::Rec709 => signal.powf(2.4),
        Gamma::Gamma22 => signal.powf(2.2),
        Gamma::Gamma26 => signal.powf(2.6),
        Gamma::Gamma28 => signal.powf(2.8),
        Gamma::Srgb => {
            if signal <= 0.04045 {
                signal / 12.92
            } else {
                ((signal + 0.055) / 1.055).powf(2.4)
            }
        }
        Gamma::Linear => signal,
        Gamma::St2084 => pq_nits(signal) / 10_000.0,
        Gamma::Hlg => hlg_scene(signal),
    }
}

/// The table of linear light of each code from 0 to [`ONE`].
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "linear light from 0 to 1, of 1/32768"
)]
pub(crate) fn linear_table(transfer: Gamma) -> Vec<u32> {
    (0..TABLE)
        .map(|code| {
            let signal = f64::from(u32::try_from(code).unwrap_or(0)) / f64::from(ONE);
            (linear(transfer, signal) * LINEAR_ONE)
                .round()
                .clamp(0.0, LINEAR_ONE) as u32
        })
        .collect()
}

/// A frame on the processor: its pixels by rows, R', G', B' and alpha as floats.
pub(crate) struct Frame<'a> {
    pub width: u32,
    pub height: u32,
    pub pixels: &'a [[f32; 4]],
}

impl Frame<'_> {
    fn at(&self, x: u32, y: u32) -> [f32; 4] {
        self.pixels[y as usize * self.width as usize + x as usize]
    }

    /// The pixels counted, one in `step` across and down, with where they are.
    fn sampled(&self, step: u32) -> impl Iterator<Item = (u32, u32, [f32; 4])> + '_ {
        (0..self.height).step_by(step as usize).flat_map(move |y| {
            (0..self.width)
                .step_by(step as usize)
                .map(move |x| (x, y, self.at(x, y)))
        })
    }
}

/// The counts of a scope of a frame, one pixel in `step` across and down, laid out as the card
/// lays them out (see [`super::Scopes::read_counts`]).
pub(crate) fn counts(frame: &Frame, kind: &Kind, signal: &Signal, step: u32) -> Vec<u32> {
    let coefficients = Coefficients::of(signal.luma);
    match kind {
        Kind::Waveform(waveform) => {
            let planes = match waveform.channels {
                WaveformChannels::Luma => 1,
                WaveformChannels::Rgb => 3,
            };
            columns(
                frame,
                step,
                waveform.width,
                waveform.levels,
                planes,
                coefficients,
            )
        }
        Kind::Parade(parade) => columns(frame, step, parade.width, parade.levels, 3, coefficients),
        Kind::Vectorscope(scope) => {
            let vector = Vector::of(scope.size, scope.zoom, scope.tones, scope.low, scope.high);
            let mut counts = vec![0; scope.size as usize * scope.size as usize];
            for (_, _, pixel) in frame.sampled(step) {
                let bin = vector.bin(coefficients, wide(pixel));
                if bin != NONE {
                    counts[bin as usize] += 1;
                }
            }
            counts
        }
        Kind::Histogram(histogram) => histogram_counts(frame, step, histogram.bins, coefficients),
        Kind::Chromaticity(diagram) => {
            let chromaticity = Chromaticity::of(
                diagram.width,
                diagram.height,
                &diagram.space,
                signal.transfer,
            );
            let mut counts = vec![0; diagram.width as usize * diagram.height as usize];
            for (_, _, pixel) in frame.sampled(step) {
                let bin = chromaticity.bin(wide(pixel));
                if bin != NONE {
                    counts[bin as usize] += 1;
                }
            }
            counts
        }
    }
}

/// The counts of a waveform: by plane (luma alone, or R, G and B), then by level from the
/// bottom, then by column.
fn columns(
    frame: &Frame,
    step: u32,
    width: u32,
    levels: u32,
    planes: u32,
    coefficients: Coefficients,
) -> Vec<u32> {
    let mut counts = vec![0; (planes * levels * width) as usize];
    for (x, _, pixel) in frame.sampled(step) {
        let column = x * width / frame.width;
        let rgb = wide(pixel);
        let values: &[i32] = if planes == 1 {
            &[coefficients.luma(rgb)]
        } else {
            &rgb
        };
        for (plane, value) in (0..).zip(values) {
            counts[((plane * levels + level(*value, levels)) * width + column) as usize] += 1;
        }
    }
    counts
}

/// The counts of a histogram: R, G, B and luma, `bins` each, then the highest count of each.
fn histogram_counts(frame: &Frame, step: u32, bins: u32, coefficients: Coefficients) -> Vec<u32> {
    let size = bins as usize;
    let mut counts = vec![0; 4 * size + 4];
    for (_, _, pixel) in frame.sampled(step) {
        let rgb = wide(pixel);
        let values = [rgb[0], rgb[1], rgb[2], coefficients.luma(rgb)];
        for (channel, value) in values.into_iter().enumerate() {
            counts[channel * size + level(value, bins) as usize] += 1;
        }
    }
    for channel in 0..4 {
        counts[4 * size + channel] = counts[channel * size..(channel + 1) * size]
            .iter()
            .copied()
            .max()
            .unwrap_or(0);
    }
    counts
}

/// The statistics of every pixel of a frame, in doubles, with a histogram of `bins`.
#[allow(
    clippy::cast_possible_truncation,
    reason = "extremes of floats of a frame"
)]
pub(crate) fn statistics(frame: &Frame, signal: &Signal, bins: u32) -> Statistics {
    let [kr, kg, kb] = signal.luma.map(f64::from);
    let mut sums = [0.0f64; 4];
    let mut lows = [f64::INFINITY; 4];
    let mut highs = [f64::NEG_INFINITY; 4];
    // Cb, Cr, their squares, and the length of chroma.
    let mut chroma = [0.0f64; 5];
    for pixel in frame.pixels {
        let [r, g, b] = [0, 1, 2].map(|channel| f64::from(pixel[channel]));
        let y = kr * r + kg * g + kb * b;
        for (channel, value) in [r, g, b, y].into_iter().enumerate() {
            sums[channel] += value;
            lows[channel] = lows[channel].min(value);
            highs[channel] = highs[channel].max(value);
        }
        let cb = (b - y) / (2.0 * (1.0 - kb));
        let cr = (r - y) / (2.0 * (1.0 - kr));
        for (sum, value) in chroma
            .iter_mut()
            .zip([cb, cr, cb * cb, cr * cr, cb.hypot(cr)])
        {
            *sum += value;
        }
    }
    let samples = frame.pixels.len() as u64;
    #[allow(clippy::cast_precision_loss, reason = "a count of pixels")]
    let count = samples as f64;
    let channel = |index: usize| Channel {
        min: lows[index] as f32,
        max: highs[index] as f32,
        mean: sums[index] / count,
    };
    let histogram = histogram_counts(frame, 1, bins, Coefficients::of(signal.luma));
    Statistics {
        samples,
        red: channel(0),
        green: channel(1),
        blue: channel(2),
        luma: channel(3),
        chroma: Chroma::of(
            [chroma[0] / count, chroma[1] / count],
            [chroma[2] / count, chroma[3] / count],
            chroma[4] / count,
        ),
        histogram: Bins::of(bins, &histogram),
    }
}
