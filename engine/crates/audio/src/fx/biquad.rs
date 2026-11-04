//! Filters of two poles and two zeros (biquads), designed by the formulas of the RBJ Audio EQ
//! Cookbook (Robert Bristow-Johnson) in `f64`, and run in direct form I in `f64`.
//!
//! A filter here is
//!
//! ```text
//! y[n] = b0·x[n] + b1·x[n−1] + b2·x[n−2] − a1·y[n−1] − a2·y[n−2]
//! ```
//!
//! with the coefficients divided by a0. The first-order filters are biquads with `b2 = a2 = 0`.
//!
//! # Why direct form I
//!
//! The state of direct form I is the signal itself: the last two samples in and out. A filter
//! whose coefficients move from sample to sample (a frequency ramped over 10 ms) reads the same
//! past as before, where the state of a transposed form is made with the old coefficients and
//! is wrong for the new ones. And a filter switched in while the sound plays (a band turned on,
//! its type changed) starts from the real past of the sound and not from silence. In `f64` the
//! noise of the form is far below anything heard.

use std::f64::consts::{FRAC_1_SQRT_2, PI, TAU};

use crate::rt::flush;

/// The coefficients of a biquad, divided by a0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coefficients {
    /// The feedforward of x\[n\].
    pub b0: f64,
    /// The feedforward of x\[n−1\].
    pub b1: f64,
    /// The feedforward of x\[n−2\].
    pub b2: f64,
    /// The feedback of y\[n−1\].
    pub a1: f64,
    /// The feedback of y\[n−2\].
    pub a2: f64,
}

/// The parts of a design: ω0, its cosine and sine.
struct Omega {
    cos: f64,
    sin: f64,
    w0: f64,
}

impl Omega {
    fn new(rate: f64, freq: f64) -> Self {
        let w0 = TAU * freq / rate;
        Omega {
            cos: w0.cos(),
            sin: w0.sin(),
            w0,
        }
    }
}

impl Coefficients {
    /// The filter that changes nothing.
    pub const IDENTITY: Coefficients = Coefficients {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    /// The coefficients b0, b1, b2, a0, a1, a2, divided by a0.
    fn normalized(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        Coefficients {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }

    /// The peaking EQ (a bell) at `freq` Hz of `q` and `gain` (as a gain of amplitude, 1 for
    /// none): `A = √gain`, `α = sin ω0 / 2Q`.
    #[must_use]
    pub fn peaking(rate: f64, freq: f64, q: f64, gain: f64) -> Self {
        let o = Omega::new(rate, freq);
        let a = gain.sqrt();
        let alpha = o.sin / (2.0 * q);
        Self::normalized(
            1.0 + alpha * a,
            -2.0 * o.cos,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * o.cos,
            1.0 - alpha / a,
        )
    }

    /// The notch at `freq` Hz of `q`.
    #[must_use]
    pub fn notch(rate: f64, freq: f64, q: f64) -> Self {
        let o = Omega::new(rate, freq);
        let alpha = o.sin / (2.0 * q);
        Self::normalized(
            1.0,
            -2.0 * o.cos,
            1.0,
            1.0 + alpha,
            -2.0 * o.cos,
            1.0 - alpha,
        )
    }

    /// `α` of a shelf of `A` and slope `s`: sin ω0 / 2 · √((A + 1/A)(1/S − 1) + 2).
    fn shelf_alpha(o: &Omega, a: f64, slope: f64) -> f64 {
        o.sin / 2.0 * ((a + 1.0 / a) * (1.0 / slope - 1.0) + 2.0).sqrt()
    }

    /// The low shelf at `freq` Hz of `gain` (amplitude) and slope `slope` (S).
    #[must_use]
    pub fn low_shelf(rate: f64, freq: f64, gain: f64, slope: f64) -> Self {
        let o = Omega::new(rate, freq);
        let a = gain.sqrt();
        let two_root_a_alpha = 2.0 * a.sqrt() * Self::shelf_alpha(&o, a, slope);
        Self::normalized(
            a * ((a + 1.0) - (a - 1.0) * o.cos + two_root_a_alpha),
            2.0 * a * ((a - 1.0) - (a + 1.0) * o.cos),
            a * ((a + 1.0) - (a - 1.0) * o.cos - two_root_a_alpha),
            (a + 1.0) + (a - 1.0) * o.cos + two_root_a_alpha,
            -2.0 * ((a - 1.0) + (a + 1.0) * o.cos),
            (a + 1.0) + (a - 1.0) * o.cos - two_root_a_alpha,
        )
    }

    /// The high shelf at `freq` Hz of `gain` (amplitude) and slope `slope` (S).
    #[must_use]
    pub fn high_shelf(rate: f64, freq: f64, gain: f64, slope: f64) -> Self {
        let o = Omega::new(rate, freq);
        let a = gain.sqrt();
        let two_root_a_alpha = 2.0 * a.sqrt() * Self::shelf_alpha(&o, a, slope);
        Self::normalized(
            a * ((a + 1.0) + (a - 1.0) * o.cos + two_root_a_alpha),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * o.cos),
            a * ((a + 1.0) + (a - 1.0) * o.cos - two_root_a_alpha),
            (a + 1.0) - (a - 1.0) * o.cos + two_root_a_alpha,
            2.0 * ((a - 1.0) - (a + 1.0) * o.cos),
            (a + 1.0) - (a - 1.0) * o.cos - two_root_a_alpha,
        )
    }

    /// The high-pass of two poles at `freq` Hz of `q`.
    #[must_use]
    pub fn high_pass(rate: f64, freq: f64, q: f64) -> Self {
        let o = Omega::new(rate, freq);
        let alpha = o.sin / (2.0 * q);
        Self::normalized(
            f64::midpoint(1.0, o.cos),
            -(1.0 + o.cos),
            f64::midpoint(1.0, o.cos),
            1.0 + alpha,
            -2.0 * o.cos,
            1.0 - alpha,
        )
    }

    /// The low-pass of two poles at `freq` Hz of `q`.
    #[must_use]
    pub fn low_pass(rate: f64, freq: f64, q: f64) -> Self {
        let o = Omega::new(rate, freq);
        let alpha = o.sin / (2.0 * q);
        Self::normalized(
            f64::midpoint(1.0, -o.cos),
            1.0 - o.cos,
            f64::midpoint(1.0, -o.cos),
            1.0 + alpha,
            -2.0 * o.cos,
            1.0 - alpha,
        )
    }

    /// The high-pass of one pole at `freq` Hz: the bilinear transform of s / (s + 1), warped to
    /// be −3.01 dB at `freq`.
    #[must_use]
    pub fn high_pass_first(rate: f64, freq: f64) -> Self {
        let k = (Omega::new(rate, freq).w0 / 2.0).tan();
        Self::normalized(1.0, -1.0, 0.0, 1.0 + k, k - 1.0, 0.0)
    }

    /// The low-pass of one pole at `freq` Hz: the bilinear transform of 1 / (s + 1), warped to be
    /// −3.01 dB at `freq`.
    #[must_use]
    pub fn low_pass_first(rate: f64, freq: f64) -> Self {
        let k = (Omega::new(rate, freq).w0 / 2.0).tan();
        Self::normalized(k, k, 0.0, 1.0 + k, k - 1.0, 0.0)
    }

    /// The gain of amplitude at `freq` Hz: |H(e^(jω))|.
    #[must_use]
    pub fn magnitude(&self, rate: f64, freq: f64) -> f64 {
        let w = TAU * freq / rate;
        let (c1, s1) = (w.cos(), w.sin());
        let (c2, s2) = ((2.0 * w).cos(), (2.0 * w).sin());
        let numerator = (self.b0 + self.b1 * c1 + self.b2 * c2).hypot(self.b1 * s1 + self.b2 * s2);
        let denominator = (1.0 + self.a1 * c1 + self.a2 * c2).hypot(self.a1 * s1 + self.a2 * s2);
        numerator / denominator
    }

    /// The largest size of a pole: below 1 when the filter is stable.
    #[must_use]
    pub fn pole_radius(&self) -> f64 {
        let discriminant = self.a1 * self.a1 - 4.0 * self.a2;
        if discriminant < 0.0 {
            self.a2.sqrt()
        } else {
            let root = discriminant.sqrt();
            f64::midpoint(-self.a1, root)
                .abs()
                .max(f64::midpoint(-self.a1, -root).abs())
        }
    }

    /// The samples after which what is left of a state of size 1 is below `floor`: at most
    /// `n` with `r^n < floor`, r the [`Coefficients::pole_radius`]. `u32::MAX` for a filter that
    /// does not decay.
    #[must_use]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to the range of u32"
    )]
    pub fn decay(&self, floor: f64) -> u32 {
        let r = self.pole_radius();
        if r <= floor {
            1
        } else if r >= 1.0 || !r.is_finite() {
            u32::MAX
        } else {
            (floor.ln() / r.ln()).ceil().min(f64::from(u32::MAX)) as u32
        }
    }

    /// One sample `x` through the filter, whose last two inputs are `input` and last two outputs
    /// `output` (the newest first): y\[n\]. `output` is moved on, `input` is left to the caller,
    /// since it may be the output of the filter before. The sums go in pairs, b0·x + (b1·x1 −
    /// a1·y1) + (b2·x2 − a2·y2), so that a filter whose b equal its a passes the sound to the bit
    /// once its past in and out are the same, but for a sample below 1e-30 in size: the output
    /// is put to 0 there.
    #[inline]
    pub fn tick(&self, x: f64, input: [f64; 2], output: &mut [f64; 2]) -> f64 {
        let y = flush(
            self.b0 * x
                + (self.b1 * input[0] - self.a1 * output[0])
                + (self.b2 * input[1] - self.a2 * output[1]),
        );
        *output = [y, output[0]];
        y
    }
}

/// The Q of the sections of two poles of a Butterworth filter of `order` (1 to 4), the lowest
/// first; a filter of odd order has a section of one pole besides. Order 2: 1/√2; order 3: 1.0;
/// order 4: 1/(2 cos(π/8)) = 0.5412 and 1/(2 cos(3π/8)) = 1.3066.
#[must_use]
pub fn butterworth_q(order: u8) -> &'static [f64] {
    const ORDER_2: [f64; 1] = [FRAC_1_SQRT_2];
    const ORDER_3: [f64; 1] = [1.0];
    // 1/(2 cos(π/8)) and 1/(2 cos(3π/8)).
    const ORDER_4: [f64; 2] = [0.541_196_100_146_197, 1.306_562_964_876_376_5];
    match order {
        2 => &ORDER_2,
        3 => &ORDER_3,
        4 => &ORDER_4,
        _ => &[],
    }
}

/// The angle of the poles of a Butterworth filter of `order` (for the check of
/// [`butterworth_q`]): the k-th pair from the real axis is at (2k − 1)π/2n for an even order and
/// kπ/n for an odd one, and its Q is 1/(2 cos θ).
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    reason = "orders and indices of a few at most"
)]
pub fn butterworth_q_of(order: u8, pair: u8) -> f64 {
    let n = f64::from(order);
    let k = f64::from(pair);
    let theta = if order.is_multiple_of(2) {
        (2.0 * k - 1.0) * PI / (2.0 * n)
    } else {
        k * PI / n
    };
    1.0 / (2.0 * theta.cos())
}
