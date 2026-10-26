//! The numbers of frames, and the rate that they are counted by.

use columbina_model::Rate;

/// The standard rates, one of which is taken for a picture whose frames come at changing times:
/// those of the editing of Python, and the faster ones of phones.
const STANDARD: [(u32, u32); 13] = [
    (24000, 1001),
    (24, 1),
    (25, 1),
    (30000, 1001),
    (30, 1),
    (48, 1),
    (50, 1),
    (60000, 1001),
    (60, 1),
    (100, 1),
    (120_000, 1001),
    (120, 1),
    (240, 1),
];

/// The rate of a picture, from the rate that its timestamps are laid on (`r_frame_rate`) and
/// the mean rate of its frames (`avg_frame_rate`), each as `(num, den)`: the first, unless the
/// mean is more than 1% from it; then the standard rate nearest to the mean. None if neither is
/// a rate.
pub(crate) fn rate_of(real: (i32, i32), mean: (i32, i32)) -> Option<Rate> {
    let rate = |(num, den): (i32, i32)| {
        let num = u32::try_from(num).ok().filter(|&num| num > 0)?;
        let den = u32::try_from(den).ok().filter(|&den| den > 0)?;
        Some(Rate { num, den })
    };
    let (real, mean) = (rate(real), rate(mean));
    if let Some(real) = real {
        let close = mean.is_none_or(|mean| {
            // |mean - real| <= real / 100, without fractions:
            // |m.num * r.den - r.num * m.den| * 100 <= r.num * m.den.
            let mean_part = u64::from(mean.num) * u64::from(real.den);
            let real_part = u64::from(real.num) * u64::from(mean.den);
            u128::from(mean_part.abs_diff(real_part)) * 100 <= u128::from(real_part)
        });
        if close {
            return Some(real);
        }
    }
    let mean = mean?;
    let distance = |(num, den): (u32, u32)| {
        // |num / den - m.num / m.den|, over the common denominator den * m.den.
        let ours = u128::from(num) * u128::from(mean.den);
        let theirs = u128::from(mean.num) * u128::from(den);
        (
            ours.abs_diff(theirs),
            u128::from(den) * u128::from(mean.den),
        )
    };
    STANDARD
        .into_iter()
        .min_by(|&a, &b| {
            let ((a_num, a_den), (b_num, b_den)) = (distance(a), distance(b));
            (a_num * b_den).cmp(&(b_num * a_den))
        })
        .map(|(num, den)| Rate { num, den })
}

/// How the timestamps of a picture are turned into numbers of frames, and back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Timing {
    /// The time base of the stream, `num/den` seconds a tick.
    pub base: (i64, i64),
    /// The rate of the frames.
    pub rate: Rate,
    /// The timestamp of frame 0.
    pub origin: i64,
}

impl Timing {
    /// The number of the frame at `timestamp`: `(timestamp - origin) / ticks_per_frame`, rounded
    /// to the nearest, half up.
    pub fn frame_of(&self, timestamp: i64) -> i64 {
        let (num, den) = self.base;
        let over = (i128::from(timestamp) - i128::from(self.origin))
            * i128::from(num)
            * i128::from(self.rate.num);
        let under = i128::from(den) * i128::from(self.rate.den);
        saturate(rounded(over, under))
    }

    /// The timestamp of frame `frame`, rounded to the nearest tick, half up.
    pub fn timestamp_of(&self, frame: i64) -> i64 {
        let (num, den) = self.base;
        let over = i128::from(frame) * i128::from(den) * i128::from(self.rate.den);
        let under = i128::from(num) * i128::from(self.rate.num);
        saturate(i128::from(self.origin) + rounded(over, under))
    }
}

/// `over / under` rounded to the nearest, half up; `under` is above 0.
fn rounded(over: i128, under: i128) -> i128 {
    (2 * over + under).div_euclid(2 * under)
}

fn saturate(value: i128) -> i64 {
    i64::try_from(value).unwrap_or(if value < 0 { i64::MIN } else { i64::MAX })
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn rate(num: u32, den: u32) -> Rate {
        Rate { num, den }
    }

    #[test]
    fn the_rate_of_the_timestamps_is_taken_when_the_mean_is_close_to_it() {
        assert_eq!(rate_of((25, 1), (25, 1)), Some(rate(25, 1)));
        assert_eq!(rate_of((30000, 1001), (2997, 100)), Some(rate(30000, 1001)));
        assert_eq!(rate_of((25, 1), (0, 0)), Some(rate(25, 1)));
    }

    #[test]
    fn a_mean_far_from_the_rate_of_the_timestamps_is_taken_to_the_nearest_standard_rate() {
        // Fields at 50 a second, frames at 25.
        assert_eq!(rate_of((50, 1), (25, 1)), Some(rate(25, 1)));
        // A phone that films at about 29.6 frames a second.
        assert_eq!(rate_of((90000, 1), (2960, 100)), Some(rate(30000, 1001)));
        assert_eq!(rate_of((0, 1), (11990, 100)), Some(rate(120_000, 1001)));
        assert_eq!(rate_of((0, 1), (0, 1)), None);
        assert_eq!(rate_of((-1, 1), (25, -1)), None);
    }

    #[test]
    fn frames_are_numbered_from_the_origin() {
        let timing = Timing {
            base: (1, 15360),
            rate: rate(25, 1),
            origin: 1024,
        };
        assert_eq!(timing.frame_of(1024), 0);
        assert_eq!(timing.frame_of(1024 + 614), 1);
        assert_eq!(timing.frame_of(1024 + 308), 1);
        assert_eq!(timing.frame_of(1024 + 307), 0);
        assert_eq!(timing.frame_of(0), -2);
        assert_eq!(timing.timestamp_of(0), 1024);
        assert_eq!(timing.timestamp_of(1), 1024 + 614);
        assert_eq!(timing.timestamp_of(100), 1024 + 61440);
    }

    #[test]
    fn a_time_base_of_more_than_one_over_a_number_counts_too() {
        // An AVI whose ticks are frames of 29.97 a second.
        let timing = Timing {
            base: (1001, 30000),
            rate: rate(30000, 1001),
            origin: 0,
        };
        assert_eq!(timing.frame_of(7), 7);
        assert_eq!(timing.timestamp_of(7), 7);
    }

    #[test]
    fn every_frame_of_23_976_in_a_base_of_90000_is_numbered_back_to_itself() {
        let timing = Timing {
            base: (1, 90000),
            rate: rate(24000, 1001),
            origin: -7,
        };
        for frame in [-3, 0, 1, 2, 1000, 1_000_001] {
            assert_eq!(timing.frame_of(timing.timestamp_of(frame)), frame);
        }
    }
}
