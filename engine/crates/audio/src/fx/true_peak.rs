//! The true peak of a channel (BS.1770, annex 2, with an interpolator of our own): the largest
//! size of the sound between its samples, read by oversampling. The Limiter hears by it (T10),
//! and the meters of the true peak read by it (T9).
//!
//! # The interpolator
//!
//! [`TRUE_PEAK_OVERSAMPLING`] = 8 points in each interval between two samples. The first is the
//! sample itself; each of the 7 others is a polyphase phase of [`TRUE_PEAK_TAPS`] = 48 taps: the
//! sinc at the distance of each of the 48 samples around the point, ±24, windowed by Kaiser of β
//! = [`TRUE_PEAK_KAISER_BETA`] = 7, the phase scaled to sum to 1. Measured (the tests): every
//! phase is within 0.005 dB of 1 and of its phase up to 0.45 of the rate; the phase half way
//! between two samples is −2.5 dB at 0.48 of the rate, where only what is nearer the Nyquist
//! frequency than 1.2 kHz at 48 kHz is read low. Between two of the 8 points, a sine of 0.45 of
//! the rate may peak 0.14 dB over the larger (cos(π · 0.45 / 8)); the Limiter keeps a margin for
//! it ([`crate::constants::LIMITER_TRUE_PEAK_MARGIN_DB`]). The filter of BS.1770's annex 2 (4
//! points, 12 taps) is −5 dB at 0.45 of the rate: hard-clipped noise read by it, or by a meter of
//! 4 points, reads up to a few dB under its true peak.
//!
//! The interval read lags the sample pushed by [`DELAY`] = 24 samples: pushing `x[n]` reads the
//! interval from `x[n − 24]` to `x[n − 23]`.
//!
//! Nothing here allocates: the taps are kept in the detector.

use crate::constants::{TRUE_PEAK_KAISER_BETA, TRUE_PEAK_OVERSAMPLING, TRUE_PEAK_TAPS};

/// The samples by which the interval read lags the sample pushed: half the taps.
pub const DELAY: usize = TRUE_PEAK_TAPS / 2;

/// The taps of the phases that read between the samples: `PHASES[k − 1][i]` weighs
/// `x[n − 47 + i]` for the point `k/8` of the way from `x[n − 24]` to `x[n − 23]`.
pub type Phases = [[f64; TRUE_PEAK_TAPS]; TRUE_PEAK_OVERSAMPLING - 1];

/// The modified Bessel function of the first kind and order 0, by its series.
fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let mut k = 1.0;
    while term > 1e-17 * sum {
        term *= (x / (2.0 * k)).powi(2);
        sum += term;
        k += 1.0;
    }
    sum
}

/// The phases of the interpolator (see the module).
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    reason = "the indices of 48 taps and 8 points"
)]
pub fn phases() -> Phases {
    let half = DELAY as f64;
    let beta = TRUE_PEAK_KAISER_BETA;
    std::array::from_fn(|phase| {
        let point = (phase + 1) as f64 / TRUE_PEAK_OVERSAMPLING as f64;
        let mut taps: [f64; TRUE_PEAK_TAPS] = std::array::from_fn(|i| {
            let u = (half - 1.0) + point - i as f64;
            let sinc = (std::f64::consts::PI * u).sin() / (std::f64::consts::PI * u);
            let r = u / half;
            sinc * bessel_i0(beta * (1.0 - r * r).max(0.0).sqrt()) / bessel_i0(beta)
        });
        let sum: f64 = taps.iter().sum();
        for tap in &mut taps {
            *tap /= sum;
        }
        taps
    })
}

/// The true peak of one channel, sample by sample.
#[derive(Clone, Copy, Debug)]
pub struct TruePeak {
    phases: Phases,
    /// The last 48 samples, twice over, so that they can be read in order from any start: the
    /// sample `n − 47 + i` at `at + 1 + i`.
    history: [f64; 2 * TRUE_PEAK_TAPS],
    /// Where the newest sample is in the first half.
    at: usize,
}

impl Default for TruePeak {
    fn default() -> Self {
        TruePeak::new()
    }
}

impl TruePeak {
    /// A detector that has heard silence.
    #[must_use]
    pub fn new() -> Self {
        TruePeak {
            phases: phases(),
            history: [0.0; 2 * TRUE_PEAK_TAPS],
            at: TRUE_PEAK_TAPS - 1,
        }
    }

    /// Back to silence.
    pub fn reset(&mut self) {
        self.history = [0.0; 2 * TRUE_PEAK_TAPS];
        self.at = TRUE_PEAK_TAPS - 1;
    }

    /// The taps of the point `point` (1 to 7) of the 8 of an interval.
    #[must_use]
    pub fn taps(&self, point: usize) -> &[f64; TRUE_PEAK_TAPS] {
        &self.phases[point - 1]
    }

    /// Pushes `x`: the largest size of the 8 points from `x[n − 24]` to `x[n − 23]`.
    #[inline]
    pub fn push(&mut self, x: f64) -> f64 {
        self.at = (self.at + 1) % TRUE_PEAK_TAPS;
        self.history[self.at] = x;
        self.history[self.at + TRUE_PEAK_TAPS] = x;
        let window = &self.history[self.at + 1..=self.at + TRUE_PEAK_TAPS];
        let mut peak = window[TRUE_PEAK_TAPS - 1 - DELAY].abs();
        for taps in &self.phases {
            let point: f64 = taps.iter().zip(window).map(|(t, x)| t * x).sum();
            peak = peak.max(point.abs());
        }
        peak
    }

    /// The true peak of `signal`, silence before and after it, as a gain of amplitude.
    #[must_use]
    #[allow(clippy::cast_possible_truncation, reason = "a level")]
    pub fn of(signal: &[f32]) -> f32 {
        let mut detector = TruePeak::new();
        let mut peak = 0.0_f64;
        for &x in signal {
            peak = peak.max(detector.push(f64::from(x)));
        }
        for _ in 0..DELAY {
            peak = peak.max(detector.push(0.0));
        }
        peak as f32
    }
}
