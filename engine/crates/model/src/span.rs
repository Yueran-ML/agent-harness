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
//!
//! # A clip whose speed changes on the way
//!
//! A clip with speed points ([`Clip::retime`], see [`crate::retime`]) shows the places of their
//! path: the place of each frame ([`source_tick`], [`Reading`]), what it shows from the lowest
//! place to the highest ([`source_span`]), and how far that lies in its media ([`shown`]) are
//! counted from its course here, so that whoever asks of a clip is answered for either kind.
//! Its speed is 1/1 and it does not play backward, as far as those fields say: what is counted
//! from a speed alone ([`ticks_of_scaled`], [`frames_for_span`]) is not of such a clip, and
//! [`frames_for`] is asked of the clip instead. Its sound is not read through its speed points
//! yet: [`source_sample`] reads a clip of one speed.

use crate::ops::LARGEST;
use crate::project::Clip;
use crate::retime::Course;
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
/// media: [`ticks_of_scaled`] of its length at its speed. With speed points it is what lies
/// from the lowest place of their path to the highest ([`Course::span`]), and nothing where
/// they are not those of the clip, which no operation leaves so.
#[must_use]
pub fn source_span(clip: &Clip, rate: Rate, scale: u32) -> i64 {
    match &clip.retime {
        Some(retime) => retime
            .course(clip.length)
            .map_or(0, |course| course.span(rate, scale)),
        None => ticks_of_scaled(clip.length, clip.speed, rate, scale),
    }
}

/// The place in its media, in ticks of `scale`, that frame `k` of a clip shows, counted from
/// its first frame, at the rate of its timeline: forward, where it begins and
/// [`ticks_of_scaled`] of `k` frames; backward, what frame `length - 1 - k` shows forward. A
/// clip of a title or a colour begins at 0. With speed points it is the place of their path at
/// the frame ([`Course::tick`]): one picture all through a freeze, and lower places frame by
/// frame where a segment plays backward.
#[must_use]
pub fn source_tick(clip: &Clip, k: i64, rate: Rate, scale: u32) -> i64 {
    Reading::of(clip).tick(k, rate, scale)
}

/// How a clip reads its source, as [`source_tick`] counts it: where it begins, how long it
/// lasts, at what speed and which way, or along which course. It is a copy of those fields of
/// the clip, so that a reader may keep it without the clip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reading {
    /// Where it begins in its source, in ticks; 0 for a title or a colour.
    pub begins: i64,
    /// How many frames it lasts.
    pub length: i64,
    /// How fast it plays its source, where it has no speed points.
    pub speed: Speed,
    /// Whether it plays its source backward, where it has no speed points.
    pub reverse: bool,
    /// The course of a clip that has speed points, which it is read by, and not by its speed.
    /// None too for speed points that are not those of the clip, which no operation leaves so:
    /// such a clip is read at its speed.
    course: Option<Course>,
}

impl Reading {
    /// How `clip` reads its source.
    #[must_use]
    pub fn of(clip: &Clip) -> Reading {
        let course = clip
            .retime
            .as_ref()
            .and_then(|retime| retime.course(clip.length).ok());
        Reading {
            begins: clip.source_in().map_or(0, |time| time.ticks),
            length: clip.length,
            speed: clip.speed,
            reverse: clip.reverse,
            course,
        }
    }

    /// The course of the clip through its source, if it has speed points.
    #[must_use]
    pub fn course(&self) -> Option<&Course> {
        self.course.as_ref()
    }

    /// The lowest place in its source that a frame of the clip shows, and the highest, in ticks
    /// of `scale`, at the rate of its timeline: what its first frame and its last show,
    /// whichever way it plays, or the ends of what its course shows ([`Course::shown`]).
    #[must_use]
    pub fn ends(&self, rate: Rate, scale: u32) -> (i64, i64) {
        if let Some(course) = &self.course {
            let (lowest, highest) = course.shown(0, 0, rate, scale);
            return (
                self.begins.saturating_add(lowest),
                self.begins.saturating_add(highest),
            );
        }
        let first = self.tick(0, rate, scale);
        let last = self.tick(self.length.saturating_sub(1), rate, scale);
        (first.min(last), first.max(last))
    }

    /// The place in its source, in ticks of `scale`, that frame `k` of the clip shows, counted
    /// from its first frame, at the rate of its timeline: [`source_tick`].
    #[must_use]
    pub fn tick(&self, k: i64, rate: Rate, scale: u32) -> i64 {
        if let Some(course) = &self.course {
            return course.tick(self.begins, k, rate, scale);
        }
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

/// What a clip shows of its media, with the frames that a transition shows of it beyond its
/// ends: each a count of ticks after where the clip begins in its media.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shown {
    /// The lowest place that a frame shows.
    pub first: i64,
    /// The highest place that a frame shows.
    pub last: i64,
    /// Where what the frames show ends: the highest place that a frame lasts to.
    pub end: i64,
}

/// What a clip shows of its media, in ticks of `scale` after where it begins in it, at the rate
/// of its timeline, with `before` frames more before its first and `after` more after its last,
/// as a transition shows them. It lies inside its media where the lowest place is not before
/// the start of the media, the highest is before its end, and what is shown ends no later than
/// the media does.
///
/// A clip of one speed shows from [`ticks_of_scaled`] of the frames before it, below nought, to
/// that of the frames to its last, and ends at that of all its frames; one that plays backward
/// shows what is before its start at its end. A clip with speed points shows the places of its
/// course ([`Course::shown`], [`Course::reach`]): none where they are not those of the clip.
#[must_use]
pub fn shown(clip: &Clip, before: i64, after: i64, rate: Rate, scale: u32) -> Option<Shown> {
    if let Some(retime) = &clip.retime {
        let course = retime.course(clip.length).ok()?;
        let (first, last) = course.shown(before, after, rate, scale);
        return Some(Shown {
            first,
            last,
            end: course.reach(before, after, rate, scale),
        });
    }
    let offset = |frames: i64| ticks_of_scaled(frames, clip.speed, rate, scale);
    // A clip that plays backward shows what is before its start at its end.
    let (before, after) = if clip.reverse {
        (after, before)
    } else {
        (before, after)
    };
    let frames = clip.length.saturating_add(after);
    Some(Shown {
        first: offset(-before),
        last: offset(frames - 1),
        end: offset(frames),
    })
}

/// The most frames that `clip` may last, playing as it does, and show no more than `span` ticks
/// of `scale` of its media, on a timeline of `rate`: [`frames_for_span`] at its speed, or, with
/// speed points, as many as their path lets it ([`crate::retime::Retime::frames_for_span`]),
/// and none where they are those of no clip.
#[must_use]
pub fn frames_for(clip: &Clip, span: i64, rate: Rate, scale: u32) -> i64 {
    match &clip.retime {
        Some(retime) => retime.frames_for_span(span, rate, scale).unwrap_or(0),
        None => frames_for_span(span, clip.speed, rate, scale),
    }
}

/// The sample of its media, at `media_rate` samples a second, that sample `timeline_sample` of
/// a timeline of `rate` frames and `sample_rate` samples a second is read from for a clip of
/// sound, rounded down.
///
/// It reads the clip at its speed, forward or backward: the speed points of a clip
/// ([`Clip::retime`]) are not read here, and the sound of a clip that has them is not played.
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
    use crate::retime::{Retime, SignedSpeed, SpeedPoint};
    use crate::time::Time;

    const SCALE: u32 = 15360;
    const PAL: Rate = Rate { num: 25, den: 1 };

    /// A clip of 10 frames from 100 ticks of 1/25 of a second into its media, a tick a frame
    /// of 25 a second, with speed points, each a frame and the `num` and `den` of its speed.
    fn pointed(points: &[(i64, i64, u32)]) -> Clip {
        let begins = Time {
            ticks: 100,
            scale: 25,
        };
        let mut clip = Clip::of_media("c1", "m1", begins, 0, 10);
        clip.retime = Some(Retime {
            points: points
                .iter()
                .map(|&(frame, num, den)| SpeedPoint {
                    frame,
                    speed: SignedSpeed { num, den },
                })
                .collect(),
        });
        clip
    }

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

    #[test]
    fn a_clip_with_speed_points_reads_its_media_along_their_path() {
        // 4 frames forward, a picture held for 3, and 3 frames back at half the speed.
        let clip = pointed(&[(0, 1, 1), (4, 0, 1), (7, -1, 2)]);
        let ticks: Vec<i64> = (0..10).map(|k| source_tick(&clip, k, PAL, 25)).collect();
        assert_eq!(ticks, [100, 101, 102, 103, 104, 104, 104, 103, 103, 102]);
        // The frames that a transition shows go on as the first segment and the last do.
        assert_eq!(source_tick(&clip, -1, PAL, 25), 99);
        assert_eq!(source_tick(&clip, 10, PAL, 25), 102);
        // It shows 4 frames of its media, from where it begins in it.
        assert_eq!(source_span(&clip, PAL, 25), 4);
        let reading = Reading::of(&clip);
        assert!(reading.course().is_some());
        assert_eq!(reading.tick(8, PAL, 25), 103);
        assert_eq!(reading.ends(PAL, 25), (100, 104));
        let within = |first, last, end| Some(Shown { first, last, end });
        assert_eq!(shown(&clip, 0, 0, PAL, 25), within(0, 4, 4));
        assert_eq!(shown(&clip, 1, 1, PAL, 25), within(-1, 4, 4));
        // It may last 16 frames and show no more: 9 frames back at half the speed are 4 frames
        // and a half, down to where it began and half a frame below.
        assert_eq!(frames_for(&clip, 4, PAL, 25), 16);
        assert_eq!(frames_for(&clip, 3, PAL, 25), 0);
        // A clip of one speed has no course, and is asked the same.
        let plain = clip_of(100, 10);
        let reading = Reading::of(&plain);
        assert!(reading.course().is_none());
        assert_eq!(reading.ends(PAL, 25), (100, 109));
        assert_eq!(shown(&plain, 1, 1, PAL, 25), within(-1, 10, 11));
        assert_eq!(frames_for(&plain, 4, PAL, 25), 4);
    }

    /// A clip of `length` frames from `ticks` of 1/25 of a second into its media.
    fn clip_of(ticks: i64, length: i64) -> Clip {
        Clip::of_media("c1", "m1", Time { ticks, scale: 25 }, 0, length)
    }

    #[test]
    fn speed_points_of_one_speed_read_as_that_speed_of_a_clip_does() {
        let ntsc = Rate {
            num: 30000,
            den: 1001,
        };
        let speeds = [
            Speed::ONE,
            Speed { num: 2, den: 1 },
            Speed { num: 137, den: 250 },
            Speed { num: 1, den: 100 },
        ];
        for speed in speeds {
            for reverse in [false, true] {
                let plain = clip(1000, 40, speed, reverse);
                // A clip keeps one speed as its speed: here it is given as a speed point, which
                // the operations never leave, to be counted the other way.
                let mut point = clip(1000, 40, Speed::ONE, false);
                point.retime = Some(Retime::constant(speed, reverse));
                for (rate, scale) in [(PAL, SCALE), (PAL, 25), (ntsc, 1000)] {
                    let what = format!("{speed:?}, backward {reverse}, {rate:?} in {scale}");
                    for k in -3..43 {
                        assert_eq!(
                            source_tick(&point, k, rate, scale),
                            source_tick(&plain, k, rate, scale),
                            "frame {k} at {what}"
                        );
                    }
                    assert_eq!(
                        source_span(&point, rate, scale),
                        source_span(&plain, rate, scale),
                        "{what}"
                    );
                    for (before, after) in [(0, 0), (3, 0), (0, 4), (2, 5)] {
                        assert_eq!(
                            shown(&point, before, after, rate, scale),
                            shown(&plain, before, after, rate, scale),
                            "{before} before and {after} after at {what}"
                        );
                    }
                    assert_eq!(
                        frames_for(&point, 7777, rate, scale),
                        frames_for(&plain, 7777, rate, scale),
                        "{what}"
                    );
                    assert_eq!(
                        Reading::of(&point).ends(rate, scale),
                        Reading::of(&plain).ends(rate, scale),
                        "{what}"
                    );
                }
            }
        }
    }

    #[test]
    fn speed_points_that_are_not_those_of_the_clip_show_nothing() {
        // One at frame 20 of a clip of 10 frames, which no operation leaves.
        let clip = pointed(&[(0, 1, 1), (20, 2, 1)]);
        assert_eq!(shown(&clip, 0, 0, PAL, 25), None);
        assert_eq!(source_span(&clip, PAL, 25), 0);
        // The clip is read at its speed.
        assert!(Reading::of(&clip).course().is_none());
        assert_eq!(source_tick(&clip, 3, PAL, 25), 103);
        // And those of no clip let it last no frame.
        assert_eq!(frames_for(&pointed(&[]), 100, PAL, 25), 0);
    }
}
