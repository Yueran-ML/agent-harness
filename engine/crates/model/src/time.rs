//! Time in the studio. A timeline counts whole frames at its rate; a source is cut at a time
//! of its own, a count of ticks of the time base of the stream, so that no place in it is lost
//! to rounding.

use serde::{Deserialize, Serialize};

/// A rate of frames, as a fraction: 24000/1001 is 23.976 frames a second.
///
/// Neither of its numbers may be 0. A rate with a 0 in it is refused when it is read, so every
/// rate of a project that was read is whole; one that is made in code must be made so too.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "RawRate")]
pub struct Rate {
    /// The frames in `den` seconds.
    pub num: u32,
    /// The seconds that `num` frames last.
    pub den: u32,
}

/// A rate as it is written, before its numbers are looked at.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRate {
    num: u32,
    den: u32,
}

impl TryFrom<RawRate> for Rate {
    type Error = String;

    fn try_from(raw: RawRate) -> Result<Self, Self::Error> {
        for (name, number) in [("num", raw.num), ("den", raw.den)] {
            if number == 0 {
                return Err(format!("a rate of frames has a `{name}` of 0"));
            }
        }
        Ok(Self {
            num: raw.num,
            den: raw.den,
        })
    }
}

/// A place in a source: a count of ticks, each `1/scale` of a second.
///
/// Two times are ordered by their fields, ticks first. That is their order in time only when
/// they are of the same scale: 1 tick of 1000 comes after 2 ticks of 90000, but is ordered
/// before them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Time {
    /// How many ticks it is from the start of the source.
    pub ticks: i64,
    /// How many ticks make a second.
    pub scale: u32,
}

impl Rate {
    /// The ticks that `frames` frames of this rate last in `scale`, rounded down.
    ///
    /// A count too large for an `i64` is `i64::MAX`, and one too far below nought is `i64::MIN`.
    #[must_use]
    pub fn ticks_of(self, frames: i64, scale: u32) -> i64 {
        // Below 2^127 in size, since |frames| <= 2^63 and den * scale < 2^64: it cannot overflow.
        let whole = i128::from(frames) * i128::from(self.den) * i128::from(scale);
        let most = if frames < 0 { i64::MIN } else { i64::MAX };
        i64::try_from(whole.div_euclid(i128::from(self.num))).unwrap_or(most)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn rate(num: u32, den: u32) -> Rate {
        Rate { num, den }
    }

    #[test]
    fn a_frame_of_25_a_second_lasts_614_ticks_of_15360_rounded_down() {
        assert_eq!(rate(25, 1).ticks_of(1, 15360), 614);
    }

    #[test]
    fn a_frame_of_29_97_a_second_lasts_1001_ticks_of_30000() {
        assert_eq!(rate(30000, 1001).ticks_of(1, 30000), 1001);
    }

    #[test]
    fn frames_of_23_976_a_second_add_up_without_rounding() {
        // 24000 frames at 24000/1001 last 1001 seconds exactly.
        assert_eq!(rate(24000, 1001).ticks_of(24000, 90000), 1001 * 90000);
    }

    #[test]
    fn frames_before_the_start_are_rounded_down_too() {
        assert_eq!(rate(25, 1).ticks_of(-1, 15360), -615);
    }

    #[test]
    fn a_length_too_long_to_count_is_the_most_there_is() {
        assert_eq!(rate(1, 1).ticks_of(i64::MAX, 90000), i64::MAX);
        assert_eq!(rate(1, 1).ticks_of(i64::MIN, 90000), i64::MIN);
        assert_eq!(rate(1, u32::MAX).ticks_of(i64::MAX, u32::MAX), i64::MAX);
    }
}
