//! The rules of the transport that are pure: which frame is shown at a time of the clock, a
//! change of speed that keeps the place, where a step or a seek lands, what a scrubbing seek
//! shows first, and which sample of sound a place is.

use columbina_media::{Rate, Time};

/// Nanoseconds in a second.
pub const NANOS: i128 = 1_000_000_000;

/// A playhead that moves at a speed, from a place at a time of a clock.
///
/// A place is counted in units of `1 / (den × 10⁹)` of a frame of the rate `num/den`, so that a
/// clock in nanoseconds moves it by `speed × num` units each nanosecond, with no rounding: frame
/// `n` is the places from `n × den × 10⁹` up to the next frame. The frame shown at a time is the
/// frame of the place then, rounded down: `floor(t × rate)` frames from where it started, at
/// speed 1. Playing forward from frame `n` starts at the start of `n`; playing backward from it
/// starts at its end; so `n` is shown for a whole frame either way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clock {
    rate: Rate,
    speed: i32,
    /// The place at `since`.
    place: i128,
    /// The time of the clock, in nanoseconds, at which the playhead was at `place`.
    since: i128,
}

impl Clock {
    /// A playhead at `frame` at time `now`, to move at `speed` (not 0).
    pub fn starting(frame: i64, speed: i32, rate: Rate, now: i128) -> Clock {
        let unit = unit(rate);
        let place = if speed < 0 {
            (i128::from(frame) + 1) * unit - 1
        } else {
            i128::from(frame) * unit
        };
        Clock {
            rate,
            speed,
            place,
            since: now,
        }
    }

    /// A playhead at `place` at time `now`, to move at `speed` (not 0).
    pub fn at(place: i128, speed: i32, rate: Rate, now: i128) -> Clock {
        Clock {
            rate,
            speed,
            place,
            since: now,
        }
    }

    pub fn speed(&self) -> i32 {
        self.speed
    }

    pub fn rate(&self) -> Rate {
        self.rate
    }

    /// How long `frame` has been shown by time `now`, in nanoseconds of the media, if it was
    /// shown from its time on: how late it is when it is shown only then. 0 before its time.
    pub fn late(&self, frame: i64, now: i128) -> i128 {
        let unit = unit(self.rate);
        let place = self.place(now);
        let into = if self.speed < 0 {
            (i128::from(frame) + 1) * unit - 1 - place
        } else {
            place - i128::from(frame) * unit
        };
        into.max(0) / i128::from(self.rate.num)
    }

    /// The place at time `now`.
    pub fn place(&self, now: i128) -> i128 {
        self.place + i128::from(self.speed) * (now - self.since) * i128::from(self.rate.num)
    }

    /// The frame shown at time `now`.
    pub fn frame(&self, now: i128) -> i64 {
        frame_of(self.place(now), self.rate)
    }

    /// The same playhead from time `now` on at another speed (not 0): where it is then, it
    /// stays, so that the place does not jump.
    pub fn with_speed(&self, speed: i32, now: i128) -> Clock {
        Clock {
            rate: self.rate,
            speed,
            place: self.place(now),
            since: now,
        }
    }

    /// The time at which `frame` is first shown, moving as the playhead moves: at or before
    /// `since` for a frame that it has reached already.
    pub fn when(&self, frame: i64) -> i128 {
        let unit = unit(self.rate);
        let step = i128::from(self.speed.unsigned_abs()) * i128::from(self.rate.num);
        let to_go = if self.speed < 0 {
            self.place - ((i128::from(frame) + 1) * unit - 1)
        } else {
            i128::from(frame) * unit - self.place
        };
        self.since + ceil_div(to_go, step)
    }

    /// The nanoseconds of the media from frame 0 to the place at time `now`.
    pub fn media_nanos(&self, now: i128) -> i128 {
        self.place(now).div_euclid(i128::from(self.rate.num))
    }
}

/// The units of place in a frame of `rate`.
fn unit(rate: Rate) -> i128 {
    i128::from(rate.den) * NANOS
}

/// The frame of a place, rounded down.
pub fn frame_of(place: i128, rate: Rate) -> i64 {
    let frame = place.div_euclid(unit(rate));
    i64::try_from(frame).unwrap_or(if frame < 0 { i64::MIN } else { i64::MAX })
}

/// `over / under` rounded up; `under` is above 0.
fn ceil_div(over: i128, under: i128) -> i128 {
    -(-over).div_euclid(under)
}

/// A frame asked for, within the media: from 0 to `last`.
pub fn clamped(frame: i64, last: i64) -> i64 {
    frame.clamp(0, last.max(0))
}

/// A place within `frame`: the place itself if it is in it, else where playing at `speed`
/// starts in it.
pub fn clamped_place(place: i128, frame: i64, speed: i32, rate: Rate) -> i128 {
    if frame_of(place, rate) == frame {
        place
    } else {
        Clock::starting(frame, speed, rate, 0).place(0)
    }
}

/// Where a step of `frames` from `from` lands, within the media.
pub fn stepped(from: i64, frames: i64, last: i64) -> i64 {
    clamped(from.saturating_add(frames), last)
}

/// The last frame of media that lasts `duration` at `rate`: the frames are as many as it lasts,
/// rounded to the nearest (its duration runs to the end of its last frame), and at least one.
pub fn last_frame(duration: Time, rate: Rate) -> i64 {
    // ticks / scale × num / den frames.
    let over = i128::from(duration.ticks) * i128::from(rate.num);
    let under = i128::from(duration.scale) * i128::from(rate.den);
    if under == 0 {
        return 0;
    }
    let frames = (2 * over + under).div_euclid(2 * under);
    i64::try_from(frames - 1).unwrap_or(i64::MAX).max(0)
}

/// What a scrubbing seek to `target` shows first, given the key frame at or before it (if the
/// decoder knows one) and the frame whose picture is shown now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum First {
    /// The frame itself, at once: it is a key frame, or none is known before it, or it is
    /// shown already.
    Exact,
    /// The key frame, standing in until the frame is decoded.
    Key(i64),
    /// Nothing new: the key frame stands in already.
    Keep,
}

pub fn scrub_first(target: i64, key: Option<i64>, shown: Option<i64>) -> First {
    match key {
        _ if shown == Some(target) => First::Exact,
        None => First::Exact,
        Some(key) if key == target => First::Exact,
        Some(key) if shown == Some(key) => First::Keep,
        Some(key) => First::Key(key),
    }
}

/// The sample of sound, at `rate` samples a second, from time 0 of the file, of the place that
/// is `media_nanos` from frame 0, whose time in the file is `start`: rounded to the nearest.
pub fn sample_at(start: Time, media_nanos: i128, rate: u32) -> i64 {
    // (start.ticks / start.scale + media_nanos / 10⁹) × rate.
    let scale = i128::from(start.scale.max(1));
    let over = (i128::from(start.ticks) * NANOS + media_nanos * scale) * i128::from(rate);
    let under = scale * NANOS;
    let sample = (2 * over + under).div_euclid(2 * under);
    i64::try_from(sample).unwrap_or(if sample < 0 { i64::MIN } else { i64::MAX })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAL: Rate = Rate { num: 25, den: 1 };
    const NTSC: Rate = Rate {
        num: 30000,
        den: 1001,
    };
    const MS: i128 = 1_000_000;

    #[test]
    fn the_frame_at_a_time_is_rounded_down_from_where_it_started() {
        let clock = Clock::starting(10, 1, PAL, 5 * MS);
        // A frame of 25 a second lasts 40 ms.
        assert_eq!(clock.frame(5 * MS), 10);
        assert_eq!(clock.frame(5 * MS + 39 * MS), 10);
        assert_eq!(clock.frame(5 * MS + 40 * MS), 11);
        assert_eq!(clock.frame(5 * MS + 1000 * MS), 35);
        // 29.97: frame 30 is shown from 1.001 s on.
        let clock = Clock::starting(0, 1, NTSC, 0);
        assert_eq!(clock.frame(1001 * MS - 1), 29);
        assert_eq!(clock.frame(1001 * MS), 30);
        // A minute of 29.97 is 1798.2 frames, exactly.
        assert_eq!(clock.frame(60 * NANOS), 1798);
    }

    #[test]
    fn faster_and_backward_the_frame_moves_as_many_times_as_fast() {
        let clock = Clock::starting(100, 4, PAL, 0);
        assert_eq!(clock.frame(40 * MS - 1), 103);
        assert_eq!(clock.frame(40 * MS), 104);
        // Backward from 100, the frame is 100 for a whole frame, then 99.
        let clock = Clock::starting(100, -1, PAL, 0);
        assert_eq!(clock.frame(0), 100);
        assert_eq!(clock.frame(40 * MS - 1), 100);
        assert_eq!(clock.frame(40 * MS), 99);
        assert_eq!(clock.frame(4000 * MS), 0);
        assert_eq!(clock.frame(4040 * MS), -1);
        // At 8× a frame of 25 a second lasts 5 ms.
        let clock = Clock::starting(100, -8, PAL, 0);
        assert_eq!(clock.frame(5 * MS - 1), 100);
        assert_eq!(clock.frame(5 * MS), 99);
    }

    #[test]
    fn a_change_of_speed_keeps_the_place() {
        let clock = Clock::starting(0, 1, PAL, 0);
        let now = 1030 * MS;
        assert_eq!(clock.frame(now), 25);
        for speed in [2, 8, -1, -4, 1] {
            let changed = clock.with_speed(speed, now);
            assert_eq!(changed.place(now), clock.place(now), "{speed}");
            assert_eq!(changed.frame(now), 25, "{speed}");
            assert_eq!(changed.speed(), speed);
        }
        // 30 ms into frame 25: at 2×, frame 26 comes 5 ms later; backward, 24 comes 30 ms
        // later.
        let fast = clock.with_speed(2, now);
        assert_eq!(fast.frame(now + 5 * MS - 1), 25);
        assert_eq!(fast.frame(now + 5 * MS), 26);
        let back = clock.with_speed(-1, now);
        assert_eq!(back.frame(now + 30 * MS), 25);
        assert_eq!(back.frame(now + 30 * MS + 1), 24);
    }

    #[test]
    fn the_time_a_frame_is_first_shown_is_when_the_clock_reaches_it() {
        for (clock, frames) in [
            (Clock::starting(10, 1, PAL, 7), [10, 11, 12, 50]),
            (Clock::starting(10, 3, NTSC, 7), [10, 11, 13, 500]),
            (Clock::starting(50, -1, PAL, 7), [50, 49, 48, 0]),
            (Clock::starting(50, -8, NTSC, 7), [50, 49, 40, 1]),
        ] {
            for frame in frames {
                let when = clock.when(frame);
                assert_eq!(clock.frame(when), frame, "{clock:?}: {frame}");
                if when > 7 {
                    assert_ne!(clock.frame(when - 1), frame, "{clock:?}: {frame}");
                }
            }
        }
        // A frame that was reached already: at or before the start.
        assert!(Clock::starting(10, 1, PAL, 7).when(9) <= 7);
    }

    #[test]
    fn how_late_a_frame_is_shown_is_how_far_into_it_the_place_is() {
        let clock = Clock::starting(3, 1, PAL, 0);
        assert_eq!(clock.media_nanos(45 * MS), 165 * MS);
        // Frame 3 shown at 45 ms is 45 ms late; frame 4, 5 ms; frame 5 is not due yet.
        assert_eq!(clock.late(3, 45 * MS), 45 * MS);
        assert_eq!(clock.late(4, 45 * MS), 5 * MS);
        assert_eq!(clock.late(5, 45 * MS), 0);
        // Backward from 3: frame 2 is due at 40 ms.
        let back = Clock::starting(3, -1, PAL, 0);
        assert_eq!(back.late(2, 45 * MS), 5 * MS);
        assert_eq!(back.late(1, 45 * MS), 0);
        // A place kept within its frame, or moved to where playing starts in the frame.
        let place = clock.place(45 * MS);
        assert_eq!(clamped_place(place, 4, 1, PAL), place);
        assert_eq!(
            clamped_place(place, 2, 1, PAL),
            Clock::starting(2, 1, PAL, 0).place(0)
        );
        assert_eq!(clamped_place(place, 2, -1, PAL), 3 * NANOS - 1);
        let moved = Clock::at(place, 2, PAL, 7);
        assert_eq!((moved.frame(7), moved.speed(), moved.rate()), (4, 2, PAL));
    }

    #[test]
    fn steps_and_seeks_stay_within_the_media() {
        assert_eq!(stepped(10, 1, 99), 11);
        assert_eq!(stepped(10, -10, 99), 0);
        assert_eq!(stepped(10, -11, 99), 0);
        assert_eq!(stepped(95, 10, 99), 99);
        assert_eq!(stepped(0, i64::MAX, 99), 99);
        assert_eq!(stepped(5, i64::MIN, 99), 0);
        assert_eq!(clamped(-3, 99), 0);
        assert_eq!(clamped(1000, 99), 99);
        assert_eq!(clamped(5, 0), 0);
    }

    #[test]
    fn the_last_frame_is_one_before_as_many_frames_as_the_media_lasts() {
        let time = |ticks, scale| Time { ticks, scale };
        // 4 s at 25: 100 frames.
        assert_eq!(last_frame(time(61_440, 15_360), PAL), 99);
        // 3 s at 60.
        assert_eq!(
            last_frame(time(3 * 90_000, 90_000), Rate { num: 60, den: 1 }),
            179
        );
        // 10 s of 29.97: 299.7 frames, the last of them partly there.
        assert_eq!(last_frame(time(900_900, 90_000), NTSC), 299);
        assert_eq!(last_frame(time(0, 90_000), PAL), 0);
        assert_eq!(last_frame(time(5, 0), PAL), 0);
    }

    #[test]
    fn a_scrubbing_seek_shows_the_key_frame_first_unless_it_is_shown_or_is_the_frame() {
        assert_eq!(scrub_first(77, Some(50), None), First::Key(50));
        assert_eq!(scrub_first(77, Some(50), Some(3)), First::Key(50));
        assert_eq!(scrub_first(77, Some(50), Some(50)), First::Keep);
        assert_eq!(scrub_first(50, Some(50), Some(3)), First::Exact);
        assert_eq!(scrub_first(77, None, Some(50)), First::Exact);
        assert_eq!(scrub_first(77, Some(50), Some(77)), First::Exact);
    }

    #[test]
    fn the_sample_of_a_place_counts_from_time_0_of_the_file() {
        let start = Time {
            ticks: 0,
            scale: 15_360,
        };
        assert_eq!(sample_at(start, 0, 48_000), 0);
        assert_eq!(sample_at(start, NANOS, 48_000), 48_000);
        // Frame 1 of 25: 40 ms.
        let clock = Clock::starting(1, 1, PAL, 0);
        assert_eq!(sample_at(start, clock.media_nanos(0), 48_000), 1920);
        // A file whose frame 0 is at 1.4 s.
        let late = Time {
            ticks: 126_000,
            scale: 90_000,
        };
        assert_eq!(sample_at(late, 0, 44_100), 61_740);
        assert_eq!(sample_at(late, 100 * MS, 44_100), 66_150);
    }
}
