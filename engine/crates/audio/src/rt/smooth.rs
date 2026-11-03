//! The smoothing of a parameter changed while the sound plays: a straight ramp of 10 ms
//! ([`SMOOTHING`]) from where the value was to where it is set, the gain in the linear domain and
//! the frequency in the logarithmic one.
//!
//! Each value is worked out from the samples since the change, not added up step by step, so the
//! values do not depend on how the samples are asked for: [`Smoothed::fill`] of a block gives the
//! same bits as [`Smoothed::next_value`] for each of its samples.

use crate::constants::{samples, SMOOTHING};

/// The domain in which a value ramps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Domain {
    /// Straight in the value: a gain, a mix, a pan.
    Linear,
    /// Straight in the logarithm of the value: a frequency, a time. The values must be above 0.
    Log,
}

/// A parameter that ramps to each new value.
#[derive(Clone, Copy, Debug)]
pub struct Smoothed {
    domain: Domain,
    /// Where the ramp starts, in the domain.
    from: f64,
    /// Where it ends, in the domain.
    to: f64,
    /// The value it ends on, as it was set.
    target: f32,
    /// The samples given since the change.
    step: u32,
    /// The samples of a ramp: 0 before [`Smoothed::prepare`].
    len: u32,
}

impl Smoothed {
    /// A parameter at `value`, not ramping. It changes at once until it is prepared.
    #[must_use]
    pub fn new(value: f32, domain: Domain) -> Self {
        let at = into(domain, value);
        Smoothed {
            domain,
            from: at,
            to: at,
            target: value,
            step: 0,
            len: 0,
        }
    }

    /// Ramps over 10 ms at `rate`, and ends any ramp under way.
    pub fn prepare(&mut self, rate: u32) {
        self.len = u32::try_from(samples(SMOOTHING, rate)).unwrap_or(u32::MAX);
        self.finish();
    }

    /// The samples of a ramp.
    #[must_use]
    pub fn ramp_len(&self) -> u32 {
        self.len
    }

    /// Ramps from the value now to `target`, reached at the 10 ms-th sample given from now. The
    /// same target again changes nothing.
    pub fn set(&mut self, target: f32) {
        if target.to_bits() == self.target.to_bits() {
            return;
        }
        self.from = self.at();
        self.to = into(self.domain, target);
        self.target = target;
        self.step = 0;
    }

    /// Goes to `value` at once.
    pub fn jump(&mut self, value: f32) {
        self.target = value;
        self.to = into(self.domain, value);
        self.finish();
    }

    /// Ends the ramp under way at its target.
    pub fn finish(&mut self) {
        self.from = self.to;
        self.step = self.len;
    }

    /// Is ramping.
    #[must_use]
    pub fn is_ramping(&self) -> bool {
        self.step < self.len
    }

    /// The value it ramps to.
    #[must_use]
    pub fn target(&self) -> f32 {
        self.target
    }

    /// The value of the last sample given.
    #[must_use]
    pub fn current(&self) -> f32 {
        if self.is_ramping() {
            out(self.domain, self.at())
        } else {
            self.target
        }
    }

    /// The value of the next sample.
    pub fn next_value(&mut self) -> f32 {
        if self.step + 1 < self.len {
            self.step += 1;
            out(self.domain, self.at())
        } else {
            self.step = self.len;
            self.target
        }
    }

    /// The values of the next `values.len()` samples.
    pub fn fill(&mut self, values: &mut [f32]) {
        for value in values {
            *value = self.next_value();
        }
    }

    /// Where the ramp is, in the domain.
    fn at(&self) -> f64 {
        if self.is_ramping() {
            self.from + (self.to - self.from) * f64::from(self.step) / f64::from(self.len)
        } else {
            self.to
        }
    }
}

fn into(domain: Domain, value: f32) -> f64 {
    match domain {
        Domain::Linear => f64::from(value),
        Domain::Log => f64::from(value).ln(),
    }
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "the values were f32 to start with"
)]
fn out(domain: Domain, at: f64) -> f32 {
    match domain {
        Domain::Linear => at as f32,
        Domain::Log => at.exp() as f32,
    }
}
