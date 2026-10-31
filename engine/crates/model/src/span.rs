//! Where a clip reads its source: the place in its media that each of its frames shows, and the
//! sample of its media that each sample of its sound is read from, at its speed, forward or
//! backward.
//!
//! A clip of `length` frames at `speed` on a timeline of `rate` shows the ticks of its media that
//! its frames last at that rate, `speed` times over, rounded down: its span. Its frame `k` shows
//! the place `floor(k * speed * rate.den * scale / rate.num)` ticks after where it begins in its
//! media. A clip that plays backward shows the same frames, the last first: its frame `k` shows
//! what its frame `length - 1 - k` shows forward. At its own speed and forward, a clip reads its
//! media as [`Rate::ticks_of`] counts it, as the first engine did.
//!
//! A frame before the first of a clip or after its last, as a transition shows it, is counted
//! the same way: it shows what is before where the clip begins, or after what it shows.
//!
//! Every count is made exactly, and one too large for an `i64` is `i64::MAX`, and one too far
//! below nought `i64::MIN`.

use crate::ops::LARGEST;
use crate::project::Clip;
use crate::time::{Rate, Speed, Time};

/// The ticks of `scale` that `frames` frames of `rate` show at `speed`, rounded down:
/// `frames * speed * rate.den * scale / rate.num`.
#[must_use]
pub fn ticks_of_scaled(frames: i64, speed: Speed, rate: Rate, scale: u32) -> i64 {
    // Below 2^127 in size, since |frames| <= 2^63 and rate.den * scale < 2^64.
    let whole = i128::from(frames) * i128::from(rate.den) * i128::from(scale);
    let num = i128::from(speed.num);
    let den = i128::from(speed.den) * i128::from(rate.num);
    // whole * num / den, without the product, which may not fit: (q * den + r) * num / den.
    let (q, r) = (whole.div_euclid(den), whole.rem_euclid(den));
    let ticks = q
        .checked_mul(num)
        .and_then(|ticks| ticks.checked_add(r * num / den));
    saturated(ticks, frames < 0)
}

/// The ticks of its media that a clip shows, at the rate of its timeline, in the scale of its
/// media: [`ticks_of_scaled`] of its length at its speed.
#[must_use]
pub fn source_span(clip: &Clip, rate: Rate, scale: u32) -> i64 {
    ticks_of_scaled(clip.length, clip.speed, rate, scale)
}

/// The place in its media, in ticks of `scale`, that frame `k` of a clip shows, counted from
/// its first frame, at the rate of its timeline: forward, where it begins and
/// [`ticks_of_scaled`] of `k` frames; backward, what frame `length - 1 - k` shows forward. A
/// clip of a title or a colour begins at 0.
#[must_use]
pub fn source_tick(clip: &Clip, k: i64, rate: Rate, scale: u32) -> i64 {
    Reading::of(clip).tick(k, rate, scale)
}

/// How a clip reads its source, as [`source_tick`] counts it: where it begins, how long it
/// lasts, at what speed and which way. It is a copy of those fields of the clip, so that a
/// reader may keep it without the clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reading {
    /// Where it begins in its source, in ticks; 0 for a title or a colour.
    pub begins: i64,
    /// How many frames it lasts.
    pub length: i64,
    /// How fast it plays its source.
    pub speed: Speed,
    /// Whether it plays its source backward.
    pub reverse: bool,
}

impl Reading {
    /// How `clip` reads its source.
    #[must_use]
    pub fn of(clip: &Clip) -> Reading {
        Reading {
            begins: clip.source_in().map_or(0, |time| time.ticks),
            length: clip.length,
            speed: clip.speed,
            reverse: clip.reverse,
        }
    }

    /// The place in its source, in ticks of `scale`, that frame `k` of the clip shows, counted
    /// from its first frame, at the rate of its timeline: [`source_tick`].
    #[must_use]
    pub fn tick(&self, k: i64, rate: Rate, scale: u32) -> i64 {
        let frame = if self.reverse {
            self.length.saturating_sub(1).saturating_sub(k)
        } else {
            k
        };
        let offset = ticks_of_scaled(frame, self.speed, rate, scale);
        saturated(
            i128::from(self.begins).checked_add(offset.into()),
            offset < 0,
        )
    }
}

/// The frame of a picture of `rate` frames a second and `duration` long that the place `tick`,
/// in ticks of the scale of its duration, is in, as its picture is read for that place: frame
/// `n` begins at the tick that [`Rate::ticks_of`] counts for `n` frames, rounded down as it is,
/// so the tick there is of frame `n`, and the one before it of frame `n - 1`. A place before the
/// start is in the first frame, and one at the end or after it in the last.
#[must_use]
pub fn media_frame(tick: i64, rate: Rate, duration: Time) -> i64 {
    let tick = tick.clamp(0, (duration.ticks - 1).max(0));
    // Frame n begins at floor(n * scale * den / num): the frame of a tick is the most n whose
    // beginning is no later, ceil((tick + 1) * num / (scale * den)) - 1.
    let over = (i128::from(tick) + 1) * i128::from(rate.num);
    let under = i128::from(duration.scale) * i128::from(rate.den);
    if under == 0 {
        return 0;
    }
    i64::try_from((over - 1) / under).unwrap_or(i64::MAX)
}

/// The most frames of `rate` at `speed` whose ticks of `scale`, as [`ticks_of_scaled`] counts
/// them, are no more than `span`, and no more than [`LARGEST`]; 0 if `span` is below 0. A clip
/// that begins `span` ticks before the end of its media lasts at most so many frames.
#[must_use]
pub fn frames_for_span(span: i64, speed: Speed, rate: Rate, scale: u32) -> i64 {
    if span < 0 {
        return 0;
    }
    // The ticks only grow with the frames: the most that fit is found by halves.
    let (mut fits, mut too_many) = (0, LARGEST);
    if ticks_of_scaled(too_many, speed, rate, scale) <= span {
        return too_many;
    }
    while too_many - fits > 1 {
        let middle = fits + (too_many - fits) / 2;
        if ticks_of_scaled(middle, speed, rate, scale) <= span {
            fits = middle;
        } else {
            too_many = middle;
        }
    }
    fits
}

/// The sample of its media, at `media_rate` samples a second, that sample `timeline_sample` of
/// a timeline of `rate` frames and `sample_rate` samples a second is read from for a clip of
/// sound, rounded down.
///
/// The clip lies on the samples from the one that its first frame starts at to the one that the
/// frame after its last starts at, each rounded down. Forward, the sample of the timeline that
/// is `d` samples into it is read from the place `d / sample_rate * speed` seconds after where
/// it begins in its media; backward, the sample `d` samples from its end is.
#[must_use]
pub fn source_sample(
    clip: &Clip,
    rate: Rate,
    timeline_sample: i64,
    sample_rate: u32,
    media_rate: u32,
) -> i64 {
    let samples_at = |frame: i64| {
        let whole = i128::from(frame) * i128::from(sample_rate) * i128::from(rate.den);
        whole.div_euclid(i128::from(rate.num))
    };
    let first = samples_at(clip.start);
    let mut d = i128::from(timeline_sample) - first;
    if clip.reverse {
        d = samples_at(clip.end()) - first - 1 - d;
    }
    let (begins, scale) = clip
        .source_in()
        .map_or((0, 1), |time| (time.ticks, time.scale));
    // begins * media_rate / scale + d * speed * media_rate / sample_rate, each part divided on
    // its own so that no product overflows, and their remainders added.
    let scale = i128::from(scale);
    let into = i128::from(begins) * i128::from(media_rate);
    let (q_in, r_in) = (into.div_euclid(scale), into.rem_euclid(scale));
    let per = i128::from(clip.speed.den) * i128::from(sample_rate);
    let Some(along) = d
        .checked_mul(i128::from(clip.speed.num))
        .and_then(|along| along.checked_mul(i128::from(media_rate)))
    else {
        return if d < 0 { i64::MIN } else { i64::MAX };
    };
    let (q_along, r_along) = (along.div_euclid(per), along.rem_euclid(per));
    let carry = i128::from(r_in * per + r_along * scale >= scale * per);
    saturated(
        q_in.checked_add(q_along)
            .and_then(|sum| sum.checked_add(carry)),
        q_along < 0,
    )
}

/// A count as an `i64`: one too large is `i64::MAX`, and one too far below nought, or none that
/// overflowed below nought, `i64::MIN`.
fn saturated(count: Option<i128>, below: bool) -> i64 {
    match count {
        Some(count) => i64::try_from(count).unwrap_or(if count < 0 { i64::MIN } else { i64::MAX }),
        None if below => i64::MIN,
        None => i64::MAX,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::Time;

    const SCALE: u32 = 15360;
    const PAL: Rate = Rate { num: 25, den: 1 };

    fn clip(ticks: i64, length: i64, speed: Speed, reverse: bool) -> Clip {
        let mut clip = Clip::of_media(
            "c1",
            "m1",
            Time {
                ticks,
                scale: SCALE,
            },
            0,
            length,
        );
        clip.speed = speed;
        clip.reverse = reverse;
        clip
    }

    #[test]
    fn a_clip_twice_as_fast_shows_twice_as_much_of_its_media() {
        let fast = clip(0, 100, Speed { num: 2, den: 1 }, false);
        // 100 frames at 25 a second are 4 seconds, and 8 at twice the speed.
        assert_eq!(source_span(&fast, PAL, SCALE), 8 * 15360);
        assert_eq!(source_tick(&fast, 1, PAL, SCALE), 1228);
        assert_eq!(source_tick(&fast, 99, PAL, SCALE), 121_651);
    }

    #[test]
    fn a_clip_half_as_fast_shows_each_frame_twice_where_its_media_has_its_rate() {
        let slow = clip(0, 4, Speed { num: 1, den: 2 }, false);
        let ticks: Vec<i64> = (0..4).map(|k| source_tick(&slow, k, PAL, 25)).collect();
        assert_eq!(ticks, [0, 0, 1, 1]);
    }

    #[test]
    fn a_clip_played_backward_shows_its_frames_last_first() {
        let backward = clip(1000, 100, Speed::ONE, true);
        assert_eq!(source_tick(&backward, 0, PAL, SCALE), 1000 + 60825);
        assert_eq!(source_tick(&backward, 99, PAL, SCALE), 1000);
        // A frame after its last, as a transition shows it, is before where it begins.
        assert_eq!(source_tick(&backward, 100, PAL, SCALE), 1000 - 615);
    }

    #[test]
    fn the_frames_that_fit_before_the_end_of_media_are_counted() {
        // 10 seconds of media from 2 seconds in: 8 seconds, 200 frames, or 100 at twice the speed.
        let span = 8 * i64::from(SCALE);
        assert_eq!(frames_for_span(span, Speed::ONE, PAL, SCALE), 200);
        assert_eq!(
            frames_for_span(span, Speed { num: 2, den: 1 }, PAL, SCALE),
            100
        );
        assert_eq!(frames_for_span(span - 1, Speed::ONE, PAL, SCALE), 199);
        assert_eq!(frames_for_span(-1, Speed::ONE, PAL, SCALE), 0);
        assert_eq!(frames_for_span(i64::MAX, Speed::ONE, PAL, SCALE), LARGEST);
    }

    #[test]
    fn a_count_too_large_is_the_most_there_is() {
        let fastest = Speed {
            num: u32::MAX,
            den: 1,
        };
        let one = Rate {
            num: 1,
            den: u32::MAX,
        };
        assert_eq!(ticks_of_scaled(i64::MAX, fastest, one, u32::MAX), i64::MAX);
        assert_eq!(ticks_of_scaled(i64::MIN, fastest, one, u32::MAX), i64::MIN);
        let far = clip(i64::MAX - 1, 10, Speed::ONE, false);
        assert_eq!(source_tick(&far, 5, PAL, SCALE), i64::MAX);
    }

    #[test]
    fn a_clip_reads_its_sound_from_where_it_begins_at_its_speed() {
        // From 2 seconds in, at frame 25 of the timeline: its first sample is sample 48000.
        let mut sound = clip(2 * i64::from(SCALE), 50, Speed { num: 2, den: 1 }, false);
        sound.start = 25;
        let read = |sample| source_sample(&sound, PAL, sample, 48000, 44100);
        assert_eq!(read(48000), 88200);
        // Half a second into it is a second into its media.
        assert_eq!(read(48000 + 24000), 88200 + 44100);
        sound.reverse = true;
        let read = |sample| source_sample(&sound, PAL, sample, 48000, 44100);
        // Its last sample is read from where it begins, and its first from 4 seconds on.
        assert_eq!(read(48000 + 96000 - 1), 88200);
        assert_eq!(read(48000), 88200 + 4 * 44100 - 2);
    }

    #[test]
    fn the_frame_of_a_place_begins_where_its_ticks_are_counted_to_begin() {
        let ten = |scale: u32| Time {
            ticks: 10 * i64::from(scale),
            scale,
        };
        let ntsc = Rate {
            num: 30000,
            den: 1001,
        };
        for (rate, scale) in [(PAL, SCALE), (ntsc, 1000), (ntsc, 90000)] {
            for frame in 1..200 {
                let begins = rate.ticks_of(frame, scale);
                assert_eq!(media_frame(begins, rate, ten(scale)), frame, "{rate:?}");
                assert_eq!(media_frame(begins - 1, rate, ten(scale)), frame - 1);
            }
        }
        // 614 ticks of 15360 begin frame 1 of 25 a second, though 614 · 25 / 15360 < 1.
        assert_eq!(media_frame(614, PAL, ten(SCALE)), 1);
        // Before the start, the first frame; at the end or after it, the last.
        assert_eq!(media_frame(-5, PAL, ten(SCALE)), 0);
        assert_eq!(media_frame(10 * 15360, PAL, ten(SCALE)), 249);
        assert_eq!(media_frame(i64::MAX, PAL, ten(SCALE)), 249);
    }

    #[test]
    fn a_span_of_no_tick_holds_the_frames_that_show_less_than_a_tick() {
        // At 1/100 of the speed, a frame of 25 a second is 0.4 of a tick of 1/1000.
        let slowest = Speed { num: 1, den: 100 };
        assert_eq!(frames_for_span(0, slowest, PAL, 1000), 2);
        assert_eq!(frames_for_span(-1, slowest, PAL, 1000), 0);
    }

    #[test]
    fn a_sample_too_far_from_where_a_clip_begins_is_the_most_or_the_least_there_is() {
        let fastest = Speed {
            num: u32::MAX,
            den: 1,
        };
        let rate = Rate { num: 25, den: 1 };
        let at = |ticks: i64, start: i64, speed: Speed| {
            let mut sound = Clip::of_media("c1", "m1", Time { ticks, scale: 1 }, start, 1);
            sound.speed = speed;
            sound
        };
        // So far that the product of the samples, the speed and the rate of the media does not
        // fit in 128 bits.
        let read = |clip: &Clip, sample| source_sample(clip, rate, sample, 48000, u32::MAX);
        assert_eq!(read(&at(0, i64::MIN / 2, fastest), i64::MAX), i64::MAX);
        assert_eq!(read(&at(0, i64::MAX / 2, fastest), i64::MIN), i64::MIN);
        // So far that the product fits, and the sum with where the clip begins does not.
        let product = i128::from(u32::MAX) * i128::from(u32::MAX);
        let along = i128::MAX / product;
        let read = |clip: &Clip, sample| source_sample(clip, rate, sample, 1, u32::MAX);
        let first = i128::from(i64::MAX) - along;
        let start = i64::try_from(first * 25).unwrap();
        assert_eq!(read(&at(i64::MAX, start, fastest), i64::MAX), i64::MAX);
        let first = i128::from(i64::MIN) + along;
        let start = i64::try_from(first * 25).unwrap();
        assert_eq!(read(&at(i64::MIN, start, fastest), i64::MIN), i64::MIN);
        // So far that the sum fits in 128 bits, and not in 64.
        let twice = |clip: &Clip, sample| source_sample(clip, PAL, sample, 48000, 96000);
        assert_eq!(twice(&at(0, 0, Speed::ONE), i64::MIN), i64::MIN);
        assert_eq!(twice(&at(0, 0, Speed::ONE), i64::MAX), i64::MAX);
    }
}
