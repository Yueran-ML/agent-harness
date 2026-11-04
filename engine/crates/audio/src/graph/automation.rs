//! The lanes of automation as the mix reads them (plan T14, "求值"): a cursor for each lane that
//! fills a block with the value of the lane at each of its samples, walking the lane's points as
//! the samples go on rather than looking them up again at each sample.
//!
//! The values are those of [`Lane::value_at`] at the samples of the timeline, to the bit: the
//! same comparisons of ticks, counted exactly in 128 bits, and the same arithmetic of doubles
//! between two points. A run that goes on from where the last block ended keeps its place; one
//! that starts elsewhere (a start, a seek) finds it again by halving. Nothing here allocates,
//! locks or waits.

use columbina_model::{Lane, LaneKind};

/// Where a cursor is in its lane: `upper` is the first point whose time is after the sample
/// `next`, which the next block is expected to start at.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cursor {
    next: i64,
    upper: usize,
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
                        let rise = to.value - from.value;
                        let offset = now - base;
                        let step = i64::try_from(scale).unwrap_or(i64::MAX);
                        // Counted in 64 bits where the whole run fits, as it nearly always does;
                        // a count of either width is the same double.
                        let end = offset + scale * i128::try_from(run).unwrap_or(i128::MAX);
                        if let (Ok(mut offset), Ok(_)) = (i64::try_from(offset), i64::try_from(end))
                        {
                            for value in into.iter_mut() {
                                let share = (offset as f64 / span).clamp(0.0, 1.0);
                                *value = (from.value + rise * share) as f32;
                                offset += step;
                            }
                        } else {
                            let mut offset = offset;
                            for value in into.iter_mut() {
                                let share = (offset as f64 / span).clamp(0.0, 1.0);
                                *value = (from.value + rise * share) as f32;
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
