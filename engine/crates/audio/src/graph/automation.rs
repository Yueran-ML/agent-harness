//! The lanes of automation as the mix reads them (plan T14, "求值"): a cursor for each lane that
//! fills a block with the value of the lane at each of its samples, walking the lane's points as
//! the samples go on rather than looking them up again at each sample.
//!
//! The values are those of [`Lane::value_at`] at the samples of the timeline, to the bit: the
//! same comparisons of ticks, counted exactly in 128 bits, and the same arithmetic of doubles
//! between two points. A run that goes on from where the last block ended keeps its place; one
//! that starts elsewhere (a start, a seek) finds it again by halving. Nothing here allocates,
//! locks or waits.
//!
//! While automation is recorded (T16), a lane gives way to the value of a control that is
//! touched, or is moved by an offset ([`Live`], [`Held`]), after its cursor has filled the block.

use columbina_model::{Lane, LaneKind, LaneScale};

use crate::constants::{FADER_MAX_DB, SILENCE_DB};
use crate::rt::{Domain, Smoothed};

/// Where a cursor is in its lane: `upper` is the first point whose time is after the sample
/// `next`, which the next block is expected to start at.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cursor {
    next: i64,
    upper: usize,
}

impl Cursor {
    /// Forget where it is: the next block finds its place in its lane again, as after a start or
    /// a seek. For a lane put in place of another while the graph plays (T5).
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            next: i64::MIN,
            upper: 0,
        }
    }
}

/// A time of `ticks` of scale `scale` as ticks of `of`: `ticks · of`, so that two times of two
/// scales are compared exactly (as [`Lane::value_at`] compares them).
fn place(ticks: i64, of: u32) -> i128 {
    i128::from(ticks) * i128::from(of)
}

impl Cursor {
    /// Fills `out` with the values of `lane` at the samples from `start` on, of a timeline of
    /// `rate` samples a second: at sample `n`, `lane.value_at(Time { ticks: n, scale: rate })`,
    /// as a single.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        reason = "a value of a parameter, and a share of the way, as `Lane::value_at` works them"
    )]
    pub(crate) fn fill(&mut self, lane: &Lane, start: i64, rate: u32, out: &mut [f32]) {
        let points = &lane.points;
        let (Some(first), Some(last)) = (points.first(), points.last()) else {
            out.fill(0.0);
            return;
        };
        let len = points.len();
        let scale = i128::from(first.at.scale);
        // The sample as ticks of the lane, times the rate: `now`, as `value_at` counts it.
        let mut now = i128::from(start) * scale;
        if self.next != start {
            self.upper = points.partition_point(|p| place(p.at.ticks, rate) <= now);
        }
        let mut done = 0;
        while done < out.len() {
            while self.upper < len && place(points[self.upper].at.ticks, rate) <= now {
                self.upper += 1;
            }
            let left = out.len() - done;
            // How many samples are left before the next point: those whose `now` is below it.
            let run = if self.upper < len {
                let limit = place(points[self.upper].at.ticks, rate);
                let samples = (limit - now + scale - 1) / scale;
                usize::try_from(samples).unwrap_or(usize::MAX).min(left)
            } else {
                left
            };
            let into = &mut out[done..done + run];
            if self.upper == 0 {
                into.fill(first.value as f32);
            } else if self.upper == len {
                into.fill(last.value as f32);
            } else {
                let (from, to) = (&points[self.upper - 1], &points[self.upper]);
                match lane.kind {
                    LaneKind::Switch => into.fill(from.value as f32),
                    LaneKind::Continuous => {
                        let base = place(from.at.ticks, rate);
                        let span = (place(to.at.ticks, rate) - base) as f64;
                        // As `LaneScale::between` works it out: straight in the values, or in
                        // their logarithms.
                        let log = lane.scale == Some(LaneScale::Log);
                        let (low, rise) = if log {
                            let low = from.value.ln();
                            (low, to.value.ln() - low)
                        } else {
                            (from.value, to.value - from.value)
                        };
                        let curve = |share: f64| {
                            let value = low + rise * share;
                            if log {
                                value.exp()
                            } else {
                                value
                            }
                        };
                        let offset = now - base;
                        let step = i64::try_from(scale).unwrap_or(i64::MAX);
                        // Counted in 64 bits where the whole run fits, as it nearly always does;
                        // a count of either width is the same double.
                        let end = offset + scale * i128::try_from(run).unwrap_or(i128::MAX);
                        if let (Ok(mut offset), Ok(_)) = (i64::try_from(offset), i64::try_from(end))
                        {
                            for value in into.iter_mut() {
                                let share = (offset as f64 / span).clamp(0.0, 1.0);
                                *value = curve(share) as f32;
                                offset += step;
                            }
                        } else {
                            let mut offset = offset;
                            for value in into.iter_mut() {
                                let share = (offset as f64 / span).clamp(0.0, 1.0);
                                *value = curve(share) as f32;
                                offset += scale;
                            }
                        }
                    }
                }
            }
            now += scale * i128::try_from(run).unwrap_or(0);
            done += run;
        }
        self.next = start.saturating_add(i64::try_from(out.len()).unwrap_or(i64::MAX));
    }
}

/// What a value set live does to a lane of the graph while it plays (T16, the recording of
/// automation; [`super::Graph::live`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Live {
    /// The lane gives way to this value, in the units of the model, a control that is touched:
    /// over 10 ms from the lane where it was not held (at once for a switch), and a value that
    /// moves ramps over 10 ms from where it was.
    Hold(f32),
    /// The lane is moved by this many decibels (Trim, pp. 3900–3901), over 10 ms; where the lane
    /// is at −∞ (−100 dB and below) it stays there, and it goes no higher than +10 dB.
    Offset(f32),
    /// The lane takes its place back over so many samples, on a straight line from what was
    /// heard (Snap's glide, p. 3905); at once where it is 0, and for a switch.
    Release(u32),
}

/// How a lane is held live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Off,
    Hold,
    Offset,
}

/// The value held live in place of a lane, or the offset of it, and how far it has taken the
/// lane's place (its weight, 0 to 1), each worked out from the samples since it changed, so that
/// it does not depend on how the blocks are cut.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Held {
    mode: Mode,
    value: Smoothed,
    weight: Smoothed,
}

impl Held {
    /// Nothing held, its ramps of 10 ms at `rate`.
    pub(crate) fn new(rate: u32) -> Self {
        let mut value = Smoothed::new(0.0, Domain::Linear);
        let mut weight = Smoothed::new(0.0, Domain::Linear);
        value.prepare(rate);
        weight.prepare(rate);
        Self {
            mode: Mode::Off,
            value,
            weight,
        }
    }

    /// Whether anything is held.
    pub(crate) fn is_held(&self) -> bool {
        self.mode != Mode::Off
    }

    /// Takes `live`, for a lane of `kind`.
    pub(crate) fn take(&mut self, live: Live, kind: LaneKind) {
        let switch = kind == LaneKind::Switch;
        let (mode, value) = match live {
            Live::Hold(value) => (Mode::Hold, value),
            Live::Offset(offset) => (Mode::Offset, offset),
            Live::Release(samples) => {
                if self.mode == Mode::Off {
                    return;
                }
                if switch || samples == 0 {
                    self.weight.jump(0.0);
                    self.mode = Mode::Off;
                } else {
                    self.weight.glide(0.0, samples);
                }
                return;
            }
        };
        if !value.is_finite() {
            return;
        }
        if self.mode == mode {
            self.value.set(value);
        } else {
            self.value.jump(value);
            self.mode = mode;
        }
        if switch {
            self.weight.jump(1.0);
        } else {
            self.weight.set(1.0);
        }
    }

    /// Puts what is held in place of the values of a lane of `kind` through a block.
    pub(crate) fn apply(&mut self, kind: LaneKind, values: &mut [f32]) {
        match self.mode {
            Mode::Off => return,
            Mode::Hold => {
                for value in values.iter_mut() {
                    let weight = self.weight.next_value();
                    let held = self.value.next_value();
                    *value = if weight >= 1.0 || (kind == LaneKind::Switch && weight >= 0.5) {
                        held
                    } else if kind == LaneKind::Switch {
                        *value
                    } else {
                        *value + (held - *value) * weight
                    };
                }
            }
            Mode::Offset => {
                for value in values.iter_mut() {
                    let weight = self.weight.next_value();
                    let offset = self.value.next_value();
                    if *value > SILENCE_DB {
                        *value = (*value + offset * weight).min(FADER_MAX_DB);
                    }
                }
            }
        }
        if !self.weight.is_ramping() && self.weight.target() <= 0.0 {
            self.mode = Mode::Off;
        }
    }

    /// Nothing held, at once.
    pub(crate) fn reset(&mut self) {
        self.mode = Mode::Off;
        self.weight.jump(0.0);
    }
}
