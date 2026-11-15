//! The mathematics of retiming (Reference Manual of Resolve 20, pp. 1070–1082): where a clip
//! that changes its speed on the way reads its source. A clip that does so holds its mapping
//! ([`crate::project::Clip::retime`]) and reads its picture through it ([`crate::span`]); a clip
//! of one speed holds none, and its speed and its direction instead. The sound of a clip that
//! holds one is not stretched by it yet: it is not heard.
//!
//! # One mapping
//!
//! A [`Retime`] is a list of speed points ([`SpeedPoint`]), and nothing else is kept. A speed
//! point is at a frame of the clip, counted from its first, and gives the speed
//! ([`SignedSpeed`]) of the segment that begins there and lasts to the next speed point, or to
//! the end of the clip: above 0 forward, below 0 backward, and 0 a freeze. The first speed point
//! is at frame 0. A frame that a speed point is at is of the segment that begins there.
//!
//! The Retime Speed curve ([`Course::speed_curve`]) and the Retime Frame curve
//! ([`Course::frame_curve`]) of the manual (pp. 1079–1080) are both read from the points, and
//! neither is kept: their control points are the speed points, one for one (p. 1078).
//!
//! # The path
//!
//! Put one after another, the segments are a path through the source, without a jump: each
//! segment goes on from the place that the one before it stopped at, up the source at its speed,
//! or down it, or not at all. This is the Retime Frame curve, a line from control point to
//! control point, which falls where a segment plays backward (p. 1079).
//!
//! The clip begins in its media ([`crate::project::Clip::source_in`]) at the lowest place of the
//! path within the clip, so no frame of the clip shows what is before it. A clip that plays
//! forward begins there, and one that plays backward ends there, as a clip of one speed does
//! ([`crate::span`]). A clip that goes forward and then further back than it came shows its
//! first frame above where it begins in its media.
//!
//! A frame of a segment that goes forward, or of a freeze, shows the place of the path where the
//! frame begins. A frame of a segment that goes backward shows the place where the frame ends:
//! so a clip of `length` frames that plays backward shows at its frame `k` what one that plays
//! forward shows at its frame `length - 1 - k`, as [`crate::span::source_tick`] has it, and a
//! clip that goes forward and then backward shows the last frame of its way up once more as the
//! first of its way down. A frame before the first of the clip, or after its last, as a
//! transition shows it, is counted as the first segment and the last one go.
//!
//! # Exactly
//!
//! The speeds of a mapping are counted in one unit, the least that all of them are whole
//! numbers of ([`Course::unit`]), and a place is a whole number of that unit of a frame. The
//! place of a frame is brought to ticks of the media once, rounded down. So a mapping of one
//! speed gives the ticks that [`crate::span`] gives, to the tick, and a speed point that is
//! added where the speed does not change moves no frame. A count too large for an `i64` is
//! `i64::MAX`, and one too far below nought `i64::MIN`.
//!
//! Where the head or the tail of a clip is moved ([`Retime::from_frame`],
//! [`Retime::with_length`]), the lowest place of its path may be another, and the clip then
//! begins that much later in its media, or earlier: so many units of a frame, brought to ticks
//! once, rounded down. Where that is a whole number of ticks, as it is where a unit of a frame
//! is one, and where the lowest place stays, every frame that the clip keeps shows the tick that
//! it showed. Elsewhere a frame shows that tick or the one before it, as a frame of a clip of
//! one speed does that is cut at a rate whose frames do not last whole ticks: a place of the
//! media is a whole number of ticks, and so is where a clip begins in it.
//!
//! # The way back
//!
//! [`Course::frame_of`] gives the frame that a place of the media is shown in: the frame of a
//! segment that shows the place, or the nearest place below it that the segment shows. Where
//! several frames do, in a freeze, in slow motion, or where the path comes by a second time, it
//! is the first of them.
//!
//! # The changes
//!
//! Each gives a new mapping, and the length of the clip where it changes: a speed point is
//! added ([`Retime::with_point`]); a segment is given a speed, and keeps its frames
//! ([`Retime::with_speed`]) or what it shows ([`Retime::with_speed_rippled`]); a frame is frozen
//! ([`Retime::with_freeze`]); a segment is turned round ([`Retime::with_segment_reversed`]); and
//! the clip is put back to its own speed ([`Retime::reset`]). Whether the clips after it make
//! room for a clip that grows, or are covered by it, is for the timeline to say (p. 1075).
//!
//! An edit of the timeline that moves the head of a clip, or cuts it in two, takes the mapping
//! from a frame on ([`Retime::from_frame`]), and one that moves its tail takes it to a length
//! ([`Retime::with_length`]): each says how much later the clip then begins in its media.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use crate::ops::{OpError, LARGEST};
use crate::time::{Rate, Speed};

/// The most speed points that a clip may have. Ours: the manual gives no limit.
pub const MOST_POINTS: usize = 1024;

fn out(what: &'static str) -> OpError {
    OpError::OutOfRange { what }
}

/// How fast a segment plays its source, and which way, as a fraction: 2/1 plays it twice as
/// fast, 1/2 at half its speed, -1/1 backward at its own speed, and 0/1 not at all, which is a
/// freeze.
///
/// Its `den` may not be 0: one that is read with a `den` of 0 is refused, and one that is made
/// in code is refused by [`Retime::check`]. A speed that plays is from 1/100 to 100 either way,
/// as the speed of a clip is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "RawSpeed")]
pub struct SignedSpeed {
    /// The seconds of the source that are played in `den` seconds: below 0, backward.
    pub num: i64,
    /// The seconds that `num` seconds of the source are played in.
    pub den: u32,
}

/// A speed of a segment as it is written, before its numbers are looked at.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSpeed {
    num: i64,
    den: u32,
}

impl TryFrom<RawSpeed> for SignedSpeed {
    type Error = String;

    fn try_from(raw: RawSpeed) -> Result<Self, Self::Error> {
        if raw.den == 0 {
            return Err("a speed of a segment has a `den` of 0".into());
        }
        Ok(Self {
            num: raw.num,
            den: raw.den,
        })
    }
}

impl Default for SignedSpeed {
    fn default() -> Self {
        Self::ONE
    }
}

impl SignedSpeed {
    /// As the source is, forward.
    pub const ONE: SignedSpeed = SignedSpeed { num: 1, den: 1 };

    /// Not at all: the segment holds one picture.
    pub const FREEZE: SignedSpeed = SignedSpeed { num: 0, den: 1 };

    /// The speed of a clip, forward or backward.
    #[must_use]
    pub fn of(speed: Speed, reverse: bool) -> Self {
        let num = i64::from(speed.num);
        Self {
            num: if reverse { -num } else { num },
            den: speed.den,
        }
    }

    /// Whether it holds one picture.
    #[must_use]
    pub fn is_freeze(self) -> bool {
        self.num == 0
    }

    /// Whether it plays its source backward.
    #[must_use]
    pub fn is_backward(self) -> bool {
        self.num < 0
    }

    /// The same speed the other way. A freeze stays a freeze.
    #[must_use]
    pub fn reversed(self) -> Self {
        Self {
            num: self.num.saturating_neg(),
            den: self.den,
        }
    }

    /// How fast it plays, whichever way; none for a freeze, or for a speed that
    /// [`Retime::check`] refuses for a `den` of 0 or a `num` that a [`Speed`] does not hold.
    #[must_use]
    pub fn magnitude(self) -> Option<Speed> {
        let num = u32::try_from(self.num.unsigned_abs()).ok()?;
        (num != 0 && self.den != 0).then_some(Speed { num, den: self.den })
    }

    /// The speed as the Retime controls show it, in hundredths of the speed of the source: 100
    /// as the source is, 50 at half its speed, 0 in a freeze, and below 0 backward (p. 1072).
    #[must_use]
    pub fn percent(self) -> f64 {
        let size = u32::try_from(self.num.unsigned_abs()).map_or(f64::INFINITY, f64::from);
        let percent = 100.0 * size / f64::from(self.den);
        if self.num < 0 {
            -percent
        } else {
            percent
        }
    }

    /// How many times as long a time-stretcher makes the sound that the segment plays: 2 at half
    /// the speed, and a half at twice the speed, whichever way it plays. None for a freeze,
    /// which plays no sound to be stretched.
    #[must_use]
    pub fn stretch(self) -> Option<f64> {
        let speed = self.magnitude()?;
        Some(f64::from(speed.den) / f64::from(speed.num))
    }

    /// Check that it is a freeze, or plays from 1/100 to 100 times as fast as its source.
    fn check(self) -> Result<(), OpError> {
        let (num, den) = (u128::from(self.num.unsigned_abs()), u128::from(self.den));
        if den != 0 && (num == 0 || (num * 100 >= den && num <= den * 100)) {
            Ok(())
        } else {
            Err(out("speed"))
        }
    }
}

/// A speed point: where a segment of a clip begins, and how fast the segment plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpeedPoint {
    /// The frame of the clip that the segment begins at, counted from the first frame of the
    /// clip.
    pub frame: i64,
    /// How fast the segment plays, to the next speed point or to the end of the clip. It may be
    /// left out, as the source is.
    #[serde(default)]
    pub speed: SignedSpeed,
}

/// The time mapping of a clip: its speed points, in the order of their frames, the first at
/// frame 0 (see the module). Left out, it is one speed point: the clip plays as its source is.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Retime {
    /// The speed points.
    pub points: Vec<SpeedPoint>,
}

impl Default for Retime {
    fn default() -> Self {
        Self::constant(Speed::ONE, false)
    }
}

/// A mapping that was changed, with the length that the clip then has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Retimed {
    /// The new mapping.
    pub retime: Retime,
    /// How many frames the clip lasts with it.
    pub length: i64,
}

/// A mapping of a clip whose head or tail was moved, with what the clip then needs so that the
/// frames that it keeps show what they showed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trimmed {
    /// The new mapping. It may be of one speed, which a clip keeps as its speed.
    pub retime: Retime,
    /// How many frames the clip lasts with it.
    pub length: i64,
    /// How many ticks later the clip begins in its media: earlier, where it is below 0. It is
    /// how far the lowest place of the path moved, rounded down (see the module).
    pub later: i64,
}

/// The greatest number that divides both.
fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// How many frames a segment of `frames` frames at the speed `old` lasts at the speed `new`, to
/// show what it showed: `frames * old / new`, whichever way each plays, rounded down so that it
/// shows no more than it did, and at least 1. Where either is a freeze, as many as it lasted.
fn lasting(frames: i64, old: SignedSpeed, new: SignedSpeed) -> i128 {
    // Below 2^124, since frames <= 2^53, a speed that was checked has a num below 2^39, and a
    // den is below 2^32.
    let whole = i128::from(frames) * i128::from(old.num.unsigned_abs()) * i128::from(new.den);
    let per = i128::from(old.den) * i128::from(new.num.unsigned_abs());
    if whole == 0 || per == 0 {
        return i128::from(frames);
    }
    (whole / per).max(1)
}

/// A frame, or a length, `by` frames on.
fn moved(frame: i64, by: i128) -> Result<i64, OpError> {
    i64::try_from(i128::from(frame) + by).map_err(|_| OpError::TooLarge { what: "length" })
}

impl Retime {
    /// The mapping of a clip of one speed, forward or backward.
    #[must_use]
    pub fn constant(speed: Speed, reverse: bool) -> Self {
        Self {
            points: vec![SpeedPoint {
                frame: 0,
                speed: SignedSpeed::of(speed, reverse),
            }],
        }
    }

    /// The mapping of a clip that is a freeze frame from end to end (Clip > Freeze Frame,
    /// p. 1071): it shows where it begins in its media. The frame to freeze is chosen by where
    /// the clip begins, which is not kept here.
    #[must_use]
    pub fn still() -> Self {
        Self {
            points: vec![SpeedPoint {
                frame: 0,
                speed: SignedSpeed::FREEZE,
            }],
        }
    }

    /// The one speed of the mapping, and whether it plays backward, if it has one speed point
    /// that is not a freeze.
    #[must_use]
    pub fn as_constant(&self) -> Option<(Speed, bool)> {
        match self.points.as_slice() {
            [only] if only.frame == 0 => only
                .speed
                .magnitude()
                .map(|speed| (speed, only.speed.is_backward())),
            _ => None,
        }
    }

    /// The unit that the speeds are counted in: the least number that every `den` divides. None
    /// if a `den` is 0, or if the unit is more than a `u32` holds.
    fn unit(&self) -> Option<u32> {
        let mut unit = 1_u64;
        for point in &self.points {
            let den = u64::from(point.speed.den);
            if den == 0 {
                return None;
            }
            // Below 2^64, since each is below 2^32.
            unit = unit / gcd(unit, den) * den;
            if unit > u64::from(u32::MAX) {
                return None;
            }
        }
        u32::try_from(unit).ok()
    }

    /// Check the mapping, as [`Retime::check`] does, and give the unit of its speeds.
    fn checked(&self, length: i64) -> Result<u32, OpError> {
        if length > LARGEST {
            return Err(OpError::TooLarge { what: "length" });
        }
        let (Some(first), Some(last)) = (self.points.first(), self.points.last()) else {
            return Err(out("retime"));
        };
        let in_order = self
            .points
            .windows(2)
            .all(|pair| pair[0].frame < pair[1].frame);
        if self.points.len() > MOST_POINTS || first.frame != 0 || !in_order || last.frame >= length
        {
            return Err(out("retime"));
        }
        for point in &self.points {
            point.speed.check()?;
        }
        self.unit().ok_or_else(|| out("retime_unit"))
    }

    /// Check that it is the mapping of a clip of `length` frames.
    ///
    /// # Errors
    /// [`OpError::TooLarge`] of `length` where the clip lasts more than [`LARGEST`] frames.
    /// [`OpError::OutOfRange`] of `retime` where it has no speed point, or more than
    /// [`MOST_POINTS`], or its first is not at frame 0, or two are not in the order of their
    /// frames, each after the one before, or one is at the end of the clip or after it; of
    /// `speed` where a speed has a `den` of 0, or plays and is not from 1/100 to 100 either way;
    /// and of `retime_unit` where its speeds have no unit that a `u32` holds, which every `den`
    /// divides.
    pub fn check(&self, length: i64) -> Result<(), OpError> {
        self.checked(length).map(|_| ())
    }

    /// The course of a clip of `length` frames through its source: the mapping, counted out for
    /// reading.
    ///
    /// # Errors
    /// The mapping is not one of a clip of `length` frames, as [`Retime::check`] says.
    pub fn course(&self, length: i64) -> Result<Course, OpError> {
        let unit = self.checked(length)?;
        let mut segments = Vec::with_capacity(self.points.len());
        let (mut at, mut lowest, mut highest) = (0_i128, 0_i128, 0_i128);
        for (index, point) in self.points.iter().enumerate() {
            let to = self.points.get(index + 1).map_or(length, |next| next.frame);
            // Below 2^71 in size, since a speed that was checked has a num below 2^39.
            let step = i128::from(point.speed.num) * i128::from(unit / point.speed.den);
            segments.push(Segment {
                from: point.frame,
                to,
                speed: point.speed,
                at,
                step,
            });
            // The frames of all the segments are no more than 2^53: the sum is below 2^124.
            at += step * i128::from(to - point.frame);
            lowest = lowest.min(at);
            highest = highest.max(at);
        }
        for segment in &mut segments {
            segment.at -= lowest;
        }
        Ok(Course {
            segments,
            unit,
            highest: highest - lowest,
            length,
        })
    }

    /// The most frames that a clip of this mapping may last and show no more than `span` ticks
    /// of `scale` of its media, from the lowest place that it shows to the highest, on a
    /// timeline of `rate`; and no more than [`LARGEST`]. 0 if it shows more even where it ends
    /// with the frame of its last speed point. For a mapping of one speed it is
    /// [`crate::span::frames_for_span`].
    ///
    /// # Errors
    /// The mapping is not one of any clip, as [`Retime::check`] says.
    pub fn frames_for_span(&self, span: i64, rate: Rate, scale: u32) -> Result<i64, OpError> {
        let Some(last) = self.points.last() else {
            return Err(out("retime"));
        };
        let fits = |length: i64| -> Result<bool, OpError> {
            Ok(self.course(length)?.span(rate, scale) <= span)
        };
        // What a clip shows only grows with its length: the most that fits is found by halves.
        let (mut fitting, mut too_many) = (last.frame.saturating_add(1), LARGEST);
        if !fits(fitting)? {
            return Ok(0);
        }
        if fitting >= too_many || fits(too_many)? {
            return Ok(too_many);
        }
        while too_many - fitting > 1 {
            let middle = fitting + (too_many - fitting) / 2;
            if fits(middle)? {
                fitting = middle;
            } else {
                too_many = middle;
            }
        }
        Ok(fitting)
    }

    /// Which speed point begins the segment that frame `at` of a clip of `length` frames is in.
    fn segment_index(&self, at: i64, length: i64) -> Result<usize, OpError> {
        self.check(length)?;
        if !(0..length).contains(&at) {
            return Err(out("frame"));
        }
        Ok(self
            .points
            .partition_point(|point| point.frame <= at)
            .saturating_sub(1))
    }

    /// The mapping with a speed point at frame `at` of a clip of `length` frames (Add Speed
    /// Point, p. 1076). The segment that the frame is in becomes two of its speed, and no frame
    /// shows another place than it did. Where a speed point is at the frame already, the mapping
    /// is as it was.
    ///
    /// # Errors
    /// The mapping is not one of the clip, as [`Retime::check`] says; the clip has no frame `at`
    /// ([`OpError::OutOfRange`] of `frame`); or it has [`MOST_POINTS`] speed points already (of
    /// `retime`).
    pub fn with_point(&self, at: i64, length: i64) -> Result<Retime, OpError> {
        let index = self.segment_index(at, length)?;
        let before = self.points[index];
        let mut points = self.points.clone();
        if before.frame != at {
            if points.len() >= MOST_POINTS {
                return Err(out("retime"));
            }
            points.insert(
                index + 1,
                SpeedPoint {
                    frame: at,
                    speed: before.speed,
                },
            );
        }
        Ok(Retime { points })
    }

    /// The mapping with the segment that frame `at` of a clip of `length` frames is in at
    /// `speed`, over the frames that it had: the clip lasts as long as it did, the segment shows
    /// more of the source or less, and what follows it begins further on in the source, or less
    /// far. This is how the speed of a whole clip is set where the clip keeps its length.
    ///
    /// # Errors
    /// The mapping is not one of the clip, before or after, as [`Retime::check`] says; or the
    /// clip has no frame `at` ([`OpError::OutOfRange`] of `frame`).
    pub fn with_speed(&self, at: i64, speed: SignedSpeed, length: i64) -> Result<Retime, OpError> {
        let index = self.segment_index(at, length)?;
        let mut points = self.points.clone();
        points[index].speed = speed;
        let retime = Retime { points };
        retime.check(length)?;
        Ok(retime)
    }

    /// The mapping with the segment that frame `at` of a clip of `length` frames is in at
    /// `speed`, showing what it showed: the segment lasts fewer frames where it plays faster and
    /// more where it plays slower, the speed points after it come sooner or later by as much,
    /// and so does the end of the clip (pp. 1074 and 1076). The frames that it then lasts are
    /// rounded down, so that it shows no more of the source than it did, but are at least one.
    /// Where the segment is a freeze, or `speed` is one, it keeps its frames, as
    /// [`Retime::with_speed`] has it: a freeze shows nothing that could last longer.
    ///
    /// # Errors
    /// The mapping is not one of the clip, as [`Retime::check`] says; the clip has no frame `at`
    /// ([`OpError::OutOfRange`] of `frame`); `speed` is not one that a segment may have (of
    /// `speed`); or the clip would last more than [`LARGEST`] frames ([`OpError::TooLarge`] of
    /// `length`).
    pub fn with_speed_rippled(
        &self,
        at: i64,
        speed: SignedSpeed,
        length: i64,
    ) -> Result<Retimed, OpError> {
        let index = self.segment_index(at, length)?;
        speed.check()?;
        let old = self.points[index];
        let to = self.points.get(index + 1).map_or(length, |next| next.frame);
        let frames = to - old.frame;
        let by = lasting(frames, old.speed, speed) - i128::from(frames);
        let mut points = self.points.clone();
        points[index].speed = speed;
        for point in &mut points[index + 1..] {
            point.frame = moved(point.frame, by)?;
        }
        let retimed = Retimed {
            retime: Retime { points },
            length: moved(length, by)?,
        };
        retimed.retime.check(retimed.length)?;
        Ok(retimed)
    }

    /// The mapping with the picture of frame `at` of a clip of `length` frames held for `hold`
    /// frames more (Freeze Frame of the Retime controls, pp. 1075–1076): two speed points, one
    /// where the freeze begins and one where the clip plays on at the speed that it had there.
    /// Everything after comes `hold` frames later, and the clip lasts `hold` frames more, so it
    /// shows all that it showed.
    ///
    /// In a segment that goes forward the freeze begins at the frame itself, and the frame
    /// after the freeze shows the held picture once more before the clip moves on. In one that
    /// goes backward, whose frames show where they end, it begins at the frame after; held at
    /// the last frame of the clip, it lasts to the new end, with no speed point after it.
    ///
    /// # Errors
    /// The mapping is not one of the clip, as [`Retime::check`] says; the clip has no frame `at`
    /// ([`OpError::OutOfRange`] of `frame`); `hold` is less than one frame (of `hold`); the
    /// mapping would have more than [`MOST_POINTS`] speed points (of `retime`); or the clip
    /// would last more than [`LARGEST`] frames ([`OpError::TooLarge`] of `length`).
    pub fn with_freeze(&self, at: i64, hold: i64, length: i64) -> Result<Retimed, OpError> {
        let index = self.segment_index(at, length)?;
        if hold < 1 {
            return Err(out("hold"));
        }
        let old = self.points[index];
        let begins = if old.speed.is_backward() {
            at.saturating_add(1)
        } else {
            at
        };
        let resumes = begins.saturating_add(hold);
        let longer = length.saturating_add(hold);
        let mut points = self.points.clone();
        for point in &mut points[index + 1..] {
            point.frame = point.frame.saturating_add(hold);
        }
        let mut next = index + 1;
        if old.frame == begins {
            points[index].speed = SignedSpeed::FREEZE;
        } else {
            points.insert(
                next,
                SpeedPoint {
                    frame: begins,
                    speed: SignedSpeed::FREEZE,
                },
            );
            next += 1;
        }
        // Where the freeze ends a segment, the speed point of the next one is there already;
        // where it ends the clip, there is nothing to play on.
        if resumes < longer && points.get(next).is_none_or(|point| point.frame != resumes) {
            points.insert(
                next,
                SpeedPoint {
                    frame: resumes,
                    speed: old.speed,
                },
            );
        }
        let retimed = Retimed {
            retime: Retime { points },
            length: longer,
        };
        retimed.retime.check(retimed.length)?;
        Ok(retimed)
    }

    /// The mapping with the segment that frame `at` of a clip of `length` frames is in played
    /// the other way at its speed (Reverse Segment, pp. 1074 and 1076). The path has no jump: a
    /// segment that is turned backward goes back from where the one before it stopped, over what
    /// was shown, and what follows goes on from where it stops. A clip of one segment shows
    /// what it showed, the last first. A freeze stays as it is.
    ///
    /// # Errors
    /// The mapping is not one of the clip, as [`Retime::check`] says; or the clip has no frame
    /// `at` ([`OpError::OutOfRange`] of `frame`).
    pub fn with_segment_reversed(&self, at: i64, length: i64) -> Result<Retime, OpError> {
        let index = self.segment_index(at, length)?;
        let mut points = self.points.clone();
        let speed = points[index].speed.reversed();
        points[index].speed = speed;
        Ok(Retime { points })
    }

    /// The mapping of a clip put back to the speed of its source (Reset to 100%, p. 1074), with
    /// the frames that a clip of `length` frames then lasts, to show at that speed, forward and
    /// once, what it showed of its source from the lowest place to the highest: rounded down,
    /// and at least one.
    ///
    /// # Errors
    /// The mapping is not one of the clip, as [`Retime::check`] says; or the clip would last
    /// more than [`LARGEST`] frames ([`OpError::TooLarge`] of `length`).
    pub fn reset(&self, length: i64) -> Result<Retimed, OpError> {
        let course = self.course(length)?;
        let frames = (course.highest / i128::from(course.unit)).max(1);
        let length = i64::try_from(frames)
            .ok()
            .filter(|frames| *frames <= LARGEST)
            .ok_or(OpError::TooLarge { what: "length" })?;
        Ok(Retimed {
            retime: Retime::default(),
            length,
        })
    }

    /// The mapping without the speed points at frame `length` or after it: that of the clip cut
    /// to `length` frames at its end, as where a clip that grew is not given the room. It has
    /// no speed point left where `length` is less than one frame.
    #[must_use]
    pub fn cut_to(&self, length: i64) -> Retime {
        Retime {
            points: self
                .points
                .iter()
                .filter(|point| point.frame < length)
                .copied()
                .collect(),
        }
    }

    /// The mapping of a clip of `length` frames from its frame `from` on: that of a clip whose
    /// head is moved to the frame, and of the second half of one that is cut in two there. The
    /// speed points before the frame are dropped, the segment that the frame is in begins the
    /// clip, and the speed points after it come `from` frames sooner. Where `from` is below 0
    /// the head is moved out: the first segment goes on before what was the first frame, and
    /// every other speed point comes so many frames later.
    ///
    /// Frame `k` of the clip, from `from` on, is frame `k - from` of the new one, and shows the
    /// place that it showed where the clip begins [`Trimmed::later`] ticks of `scale` later in
    /// its media, on a timeline of `rate`: to the tick, or the tick before it where the lowest
    /// place of the path moved by no whole number of ticks (see the module).
    ///
    /// # Errors
    /// The mapping is not one of the clip, as [`Retime::check`] says; the clip has no frame
    /// `from`, which is its end or after it ([`OpError::OutOfRange`] of `frame`); or the clip
    /// would last more than [`LARGEST`] frames ([`OpError::TooLarge`] of `length`).
    pub fn from_frame(
        &self,
        from: i64,
        length: i64,
        rate: Rate,
        scale: u32,
    ) -> Result<Trimmed, OpError> {
        let course = self.course(length)?;
        if from >= length {
            return Err(out("frame"));
        }
        let first = self
            .points
            .partition_point(|point| point.frame <= from)
            .saturating_sub(1);
        let mut points = Vec::with_capacity(self.points.len() - first);
        points.push(SpeedPoint {
            frame: 0,
            speed: self.points[first].speed,
        });
        for point in &self.points[first + 1..] {
            points.push(SpeedPoint {
                frame: moved(point.frame, -i128::from(from))?,
                speed: point.speed,
            });
        }
        let retime = Retime { points };
        let length = moved(length, -i128::from(from))?;
        let onward = retime.course(length)?;
        let later = course.later(from, &onward, rate, scale);
        Ok(Trimmed {
            retime,
            length,
            later,
        })
    }

    /// The mapping of a clip of `length` frames made to last `to` frames at its end: that of a
    /// clip whose tail is moved, and of the first half of one that is cut in two. A shorter clip
    /// loses the speed points at its new end and after it ([`Retime::cut_to`]); in a longer one
    /// the last segment goes on.
    ///
    /// Every frame that the clip keeps shows the place that it showed where the clip begins
    /// [`Trimmed::later`] ticks of `scale` later in its media, on a timeline of `rate`, as
    /// [`Retime::from_frame`] has it: the lowest place of the path is another where the tail
    /// that is cut away, or the one that is added, goes below the rest.
    ///
    /// # Errors
    /// The mapping is not one of the clip, as [`Retime::check`] says; `to` is less than one
    /// frame ([`OpError::OutOfRange`] of `length`); or it is more than [`LARGEST`]
    /// ([`OpError::TooLarge`] of `length`).
    pub fn with_length(
        &self,
        to: i64,
        length: i64,
        rate: Rate,
        scale: u32,
    ) -> Result<Trimmed, OpError> {
        let course = self.course(length)?;
        if to < 1 {
            return Err(out("length"));
        }
        let retime = if to < length {
            self.cut_to(to)
        } else {
            self.clone()
        };
        let onward = retime.course(to)?;
        let later = course.later(0, &onward, rate, scale);
        Ok(Trimmed {
            retime,
            length: to,
            later,
        })
    }
}

/// A segment of a clip: the frames from one speed point to the next, at one speed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    /// Its first frame, where its speed point is.
    pub from: i64,
    /// The frame after its last: where the next speed point is, or the end of the clip.
    pub to: i64,
    /// How fast it plays, and which way.
    pub speed: SignedSpeed,
    /// The place of the path where the segment begins, in units of a frame, above the lowest
    /// place of the path within the clip.
    at: i128,
    /// How far the path goes with each frame of the segment, in units of a frame: below 0,
    /// down the source.
    step: i128,
}

impl Segment {
    /// The place of the path at `frame`, as the segment goes: also before the segment and
    /// after it. One too far to count is the most or the least that there is.
    fn place(&self, frame: i128) -> i128 {
        let along = self.step.saturating_mul(frame - i128::from(self.from));
        self.at.saturating_add(along)
    }

    /// The place that frame `k` shows, as the segment goes: where the frame begins, or, in a
    /// segment that goes backward, where it ends.
    fn shown(&self, k: i64) -> i128 {
        self.place(i128::from(k) + i128::from(self.step < 0))
    }
}

/// A step of the Retime Speed curve (p. 1080): the speed of a segment over its frames.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct SpeedStep {
    /// The first frame of the segment.
    pub from: i64,
    /// The frame after its last.
    pub to: i64,
    /// Its speed in hundredths of the speed of the source ([`SignedSpeed::percent`]).
    pub percent: f64,
}

/// A control point of the Retime Frame curve (p. 1079): the place of the source at a frame of
/// the clip.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct FramePoint {
    /// The frame of the clip: that of a speed point, or the end of the clip.
    pub frame: i64,
    /// The place of the source there, in frames of the rate of the timeline, above the lowest
    /// place that the clip shows: at the speed of the source, one for each frame of the clip.
    pub source: f64,
}

/// `place * times / per`, rounded down. None where it is more than an `i128` holds, or `per`
/// is 0.
fn scaled(place: i128, times: u64, per: u64) -> Option<i128> {
    if per == 0 {
        return None;
    }
    let (whole, rest) = (
        place.div_euclid(i128::from(per)),
        place.rem_euclid(i128::from(per)),
    );
    // The rest is below `per`: its product with `times` is below 2^128, which is more than an
    // i128 holds, so it is counted without a sign.
    let part = rest.unsigned_abs() * u128::from(times) / u128::from(per);
    whole
        .checked_mul(i128::from(times))?
        .checked_add(i128::try_from(part).ok()?)
}

/// The first of the frames from `from` to the one before `to` that `holds` for, where it holds
/// for every frame after one that it holds for; `to` if it holds for none of them.
fn first(from: i64, to: i64, holds: impl Fn(i64) -> bool) -> i64 {
    let (mut low, mut high) = (from, to);
    while low < high {
        let middle = low + (high - low) / 2;
        if holds(middle) {
            high = middle;
        } else {
            low = middle + 1;
        }
    }
    low
}

/// The course of a clip through its source: a mapping that was checked, counted out for a clip
/// of a length ([`Retime::course`]). It is a copy, so that a reader may keep it without the
/// clip.
///
/// A place of the media is given in ticks of its scale, counted from where the clip begins in
/// its media. The rate is that of the timeline, and neither of its numbers is 0.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Course {
    /// The segments, in the order of their frames: at least one.
    segments: Vec<Segment>,
    /// How many units of a place make a frame.
    unit: u32,
    /// The highest place of the path within the clip, above the lowest.
    highest: i128,
    /// How many frames the clip lasts.
    length: i64,
}

impl Course {
    /// The segments of the clip, in the order of their frames.
    #[must_use]
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// How many frames the clip lasts.
    #[must_use]
    pub fn length(&self) -> i64 {
        self.length
    }

    /// The unit that the speeds and the places are counted in: so many make a speed of 1, and a
    /// frame of the source at the rate of the timeline.
    #[must_use]
    pub fn unit(&self) -> u32 {
        self.unit
    }

    /// The segment that frame `k` of the clip is in: the one whose speed point is at the frame,
    /// or the last before it. A frame before the clip is of the first segment, and one after it
    /// of the last.
    #[must_use]
    pub fn segment_of(&self, k: i64) -> &Segment {
        let after = self.segments.partition_point(|segment| segment.from <= k);
        &self.segments[after.saturating_sub(1)]
    }

    /// The speed of the segment that frame `k` is in.
    #[must_use]
    pub fn speed_at(&self, k: i64) -> SignedSpeed {
        self.segment_of(k).speed
    }

    /// How many times as long a time-stretcher makes the sound of the clip at frame `k`
    /// ([`SignedSpeed::stretch`]): it is the same for all the frames of a segment
    /// ([`Course::segment_of`]), and changes at a speed point. None in a freeze, which has no
    /// sound; in a segment that goes backward ([`Course::speed_at`]) the sound is read backward.
    #[must_use]
    pub fn stretch_at(&self, k: i64) -> Option<f64> {
        self.speed_at(k).stretch()
    }

    /// The place that frame `k` shows, in units ([`Course::unit`]) of a frame of the source at
    /// the rate of the timeline, above the lowest place that the clip shows.
    #[must_use]
    pub fn place(&self, k: i64) -> i128 {
        self.segment_of(k).shown(k)
    }

    /// A place of the path in ticks of `scale`, rounded down, on a timeline of `rate`.
    fn ticks(&self, place: i128, rate: Rate, scale: u32) -> i64 {
        // Each is below 2^64, as the product of two numbers below 2^32.
        let times = u64::from(rate.den) * u64::from(scale);
        let per = u64::from(self.unit) * u64::from(rate.num);
        match scaled(place, times, per) {
            Some(ticks) => {
                i64::try_from(ticks).unwrap_or(if ticks < 0 { i64::MIN } else { i64::MAX })
            }
            None if place < 0 => i64::MIN,
            None => i64::MAX,
        }
    }

    /// The tick that frame `k` shows, counted as `segment` goes.
    fn shows(&self, segment: &Segment, k: i64, rate: Rate, scale: u32) -> i64 {
        self.ticks(segment.shown(k), rate, scale)
    }

    /// The place in its media that frame `k` of the clip shows, in ticks of `scale` after where
    /// the clip begins in its media, on a timeline of `rate`. For a mapping of one speed it is
    /// what [`crate::span::ticks_of_scaled`] counts for the frame, or, backward, for frame
    /// `length - 1 - k`.
    #[must_use]
    pub fn offset(&self, k: i64, rate: Rate, scale: u32) -> i64 {
        self.shows(self.segment_of(k), k, rate, scale)
    }

    /// The place in its media that frame `k` of a clip that begins at `begins` shows, in ticks
    /// of `scale`: for a mapping of one speed, [`crate::span::source_tick`]. (Backward, that
    /// one takes a frame so far before the clip that `length - 1 - k` is more than an `i64`
    /// holds for the furthest frame that there is; here it is counted as it stands.)
    #[must_use]
    pub fn tick(&self, begins: i64, k: i64, rate: Rate, scale: u32) -> i64 {
        begins.saturating_add(self.offset(k, rate, scale))
    }

    /// The ticks of `scale` of its media that the clip shows, from the lowest place of its path
    /// to the highest, on a timeline of `rate`: for a mapping of one speed,
    /// [`crate::span::source_span`].
    #[must_use]
    pub fn span(&self, rate: Rate, scale: u32) -> i64 {
        self.ticks(self.highest, rate, scale)
    }

    /// The lowest and the highest place that the frames of the clip show, with `before` frames
    /// more before its first and `after` more after its last, as a transition shows them: in
    /// ticks of `scale` after where the clip begins in its media, on a timeline of `rate`. The
    /// clip lies inside its media where the lowest is not before the start of the media and
    /// the highest is before its end.
    #[must_use]
    pub fn shown(&self, before: i64, after: i64, rate: Rate, scale: u32) -> (i64, i64) {
        let earliest = 0_i64.saturating_sub(before.max(0));
        let latest = (self.length - 1).saturating_add(after.max(0));
        let count = self.segments.len();
        let (mut lowest, mut highest) = (i64::MAX, i64::MIN);
        for (index, segment) in self.segments.iter().enumerate() {
            // A segment goes one way: its first frame and its last show its ends.
            let from = if index == 0 { earliest } else { segment.from };
            let to = if index + 1 == count {
                latest
            } else {
                segment.to - 1
            };
            for frame in [from, to] {
                let tick = self.shows(segment, frame, rate, scale);
                lowest = lowest.min(tick);
                highest = highest.max(tick);
            }
        }
        (lowest, highest)
    }

    /// The highest place that the path of the clip comes to, with `before` frames more before
    /// its first frame and `after` more after its last, as [`Course::shown`] has them: where
    /// what its frames show ends, in ticks of `scale` after where the clip begins in its media,
    /// on a timeline of `rate`. It is where the last frame of a way up ends, and where the
    /// first frame of a way down begins. For a mapping of one speed that plays forward it is
    /// [`crate::span::ticks_of_scaled`] of the frames of the clip and those after it, and
    /// backward of its frames and those before it. The clip shows no more than its media has
    /// where this is no later than the end of the media.
    #[must_use]
    pub fn reach(&self, before: i64, after: i64, rate: Rate, scale: u32) -> i64 {
        let earliest = -i128::from(before.max(0));
        let ends = i128::from(self.length) + i128::from(after.max(0));
        let count = self.segments.len();
        let mut highest = i128::MIN;
        for (index, segment) in self.segments.iter().enumerate() {
            // A segment goes one way: the path is highest in it where it begins or where it
            // ends.
            let from = if index == 0 {
                earliest
            } else {
                i128::from(segment.from)
            };
            let to = if index + 1 == count {
                ends
            } else {
                i128::from(segment.to)
            };
            highest = highest.max(segment.place(from)).max(segment.place(to));
        }
        self.ticks(highest, rate, scale)
    }

    /// How many ticks of `scale` later a clip begins in its media, on a timeline of `rate`,
    /// where it goes on from frame `from` of this course as `onward` does: `onward` is the
    /// course of this mapping from that frame on, or cut at its end, so its frame 0 is frame
    /// `from` of this one, its path is this one's from there, and each of its speeds is one of
    /// these. It is how far the lowest place of that path is above the lowest of this one,
    /// rounded down.
    fn later(&self, from: i64, onward: &Course, rate: Rate, scale: u32) -> i64 {
        // The unit of `onward` divides this one, since its speeds are among these.
        let times = i128::from(self.unit / onward.unit.max(1));
        let here = self.segment_of(from).place(i128::from(from));
        let there = onward.segments[0].at.saturating_mul(times);
        self.ticks(here.saturating_sub(there), rate, scale)
    }

    /// The first frame of `segment` in which the place `tick` is on show: the frame that shows
    /// the place, or the nearest place below it that the segment shows. None where the segment
    /// does not come by the place.
    fn frame_in(&self, segment: &Segment, tick: i64, rate: Rate, scale: u32) -> Option<i64> {
        let shows = |k: i64| self.shows(segment, k, rate, scale);
        let (from, to) = (segment.from, segment.to);
        match segment.step.cmp(&0) {
            Ordering::Equal => (shows(from) == tick).then_some(from),
            Ordering::Greater => {
                // Its frames show higher and higher places, up to where it stops.
                let stops = self.ticks(segment.place(i128::from(to)), rate, scale);
                let above = first(from, to, |k| shows(k) > tick);
                if above == from {
                    return None;
                }
                let found = shows(above - 1);
                // Where it stops, and above, only a place that a frame shows itself is on show.
                if tick >= stops && found != tick {
                    return None;
                }
                Some(first(from, above, |k| shows(k) >= found))
            }
            Ordering::Less => {
                // Its frames show lower and lower places, down from where it begins: there, and
                // above, only a place that a frame shows itself is on show.
                let begins = self.ticks(segment.place(i128::from(from)), rate, scale);
                let found = first(from, to, |k| shows(k) <= tick);
                (found < to && (tick < begins || shows(found) == tick)).then_some(found)
            }
        }
    }

    /// The first frame of the clip in which the place `tick` of its media is on show, in ticks
    /// of `scale` after where the clip begins in its media, on a timeline of `rate`. A place is
    /// on show in the frame that shows it, and in the frame that shows the nearest place below
    /// it that its segment shows, as a picture is on show until the next one is. None where the
    /// clip does not come by the place: below where it begins in its media, or above the last
    /// place that a frame shows at the top of its path.
    ///
    /// So [`Course::offset`] of the frame is `tick` or below it. In a freeze, and in slow
    /// motion, where several frames show one place, it is the first of them; where the clip
    /// turns and comes by the place a second time, it is the frame of the first time, which
    /// may show a place below it though a later frame shows the place itself. A clip that
    /// plays backward from end to end is found as one that plays forward is: each place that
    /// it shows leads back to the first frame that shows it.
    #[must_use]
    pub fn frame_of(&self, tick: i64, rate: Rate, scale: u32) -> Option<i64> {
        self.segments
            .iter()
            .find_map(|segment| self.frame_in(segment, tick, rate, scale))
    }

    /// The Retime Speed curve (p. 1080): a step for each segment, level at its speed. It is
    /// read from the speed points, and nothing of it is kept.
    #[must_use]
    pub fn speed_curve(&self) -> Vec<SpeedStep> {
        self.segments
            .iter()
            .map(|segment| SpeedStep {
                from: segment.from,
                to: segment.to,
                percent: segment.speed.percent(),
            })
            .collect()
    }

    /// The Retime Frame curve (p. 1079): the place of the source at each speed point and at the
    /// end of the clip, with a straight line from each to the next. It rises where the clip
    /// goes forward, by one frame of the source for each frame of the clip at the speed of the
    /// source, is level in a freeze, and falls where the clip goes backward. It is read from
    /// the speed points, and nothing of it is kept.
    #[must_use]
    pub fn frame_curve(&self) -> Vec<FramePoint> {
        let unit = f64::from(self.unit);
        let point = |frame: i64, place: i128| {
            #[allow(
                clippy::cast_precision_loss,
                reason = "a place on a curve that is drawn"
            )]
            let source = place as f64 / unit;
            FramePoint { frame, source }
        };
        let mut curve: Vec<FramePoint> = self
            .segments
            .iter()
            .map(|segment| point(segment.from, segment.at))
            .collect();
        if let Some(last) = self.segments.last() {
            curve.push(point(last.to, last.place(i128::from(last.to))));
        }
        curve
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::float_cmp,
        reason = "a speed in hundredths and a place on a curve are those numbers as written"
    )]

    use super::*;
    use crate::project::Clip;
    use crate::span;
    use crate::time::Time;

    const SCALE: u32 = 15360;
    const PAL: Rate = Rate { num: 25, den: 1 };
    const NTSC: Rate = Rate {
        num: 30000,
        den: 1001,
    };

    /// A mapping of speed points, each a frame and the `num` and `den` of its speed.
    fn retime(points: &[(i64, i64, u32)]) -> Retime {
        Retime {
            points: points
                .iter()
                .map(|&(frame, num, den)| SpeedPoint {
                    frame,
                    speed: SignedSpeed { num, den },
                })
                .collect(),
        }
    }

    /// What each frame of a clip of `length` frames shows, in frames of its source after where
    /// it begins: on a timeline of 25 frames a second, in ticks of 1/25 of a second.
    fn shows(retime: &Retime, length: i64) -> Vec<i64> {
        let course = retime.course(length).unwrap();
        (0..length).map(|k| course.offset(k, PAL, 25)).collect()
    }

    fn range(what: &'static str) -> OpError {
        OpError::OutOfRange { what }
    }

    #[test]
    fn a_mapping_of_one_speed_reads_its_media_as_a_clip_of_that_speed_does() {
        let speeds = [
            Speed::ONE,
            Speed { num: 2, den: 1 },
            Speed { num: 1, den: 2 },
            Speed {
                num: 24000,
                den: 1001,
            },
            Speed { num: 137, den: 250 },
            Speed { num: 1, den: 100 },
            Speed { num: 100, den: 1 },
        ];
        let length = 100;
        for speed in speeds {
            for reverse in [false, true] {
                for (rate, scale) in [(PAL, SCALE), (PAL, 25), (NTSC, 1000), (NTSC, 90000)] {
                    let source_in = Time { ticks: 1000, scale };
                    let mut clip = Clip::of_media("c1", "m1", source_in, 0, length);
                    clip.speed = speed;
                    clip.reverse = reverse;
                    let mapping = Retime::constant(speed, reverse);
                    let course = mapping.course(length).unwrap();
                    // The frames of the clip, and those that a transition shows about it.
                    for k in -5..length + 5 {
                        assert_eq!(
                            course.tick(1000, k, rate, scale),
                            span::source_tick(&clip, k, rate, scale),
                            "frame {k} at {speed:?}, backward {reverse}, {rate:?} in {scale}"
                        );
                    }
                    assert_eq!(
                        course.span(rate, scale),
                        span::source_span(&clip, rate, scale)
                    );
                    assert_eq!(
                        mapping.frames_for_span(7777, rate, scale).unwrap(),
                        span::frames_for_span(7777, speed, rate, scale),
                        "{speed:?}, backward {reverse}, {rate:?} in {scale}"
                    );
                    assert_eq!(mapping.as_constant(), Some((speed, reverse)));
                }
            }
        }
        assert_eq!(Retime::default(), Retime::constant(Speed::ONE, false));
        assert_eq!(Retime::still().as_constant(), None);
        assert_eq!(retime(&[(0, 1, 1), (4, 1, 1)]).as_constant(), None);
    }

    #[test]
    fn a_speed_point_added_where_the_speed_does_not_change_moves_no_frame() {
        // At 29.97 frames a second in thousandths of a second no frame lasts a whole number of
        // ticks: a place that was rounded at a speed point would be a tick out after it.
        let mappings = [
            retime(&[(0, 1, 1)]),
            retime(&[(0, 137, 250)]),
            retime(&[(0, -137, 250)]),
            retime(&[(0, 1, 3), (7, -2, 7), (20, 0, 1), (26, 5, 2), (40, -1, 1)]),
        ];
        let length = 60;
        for mapping in mappings {
            let course = mapping.course(length).unwrap();
            let read = |course: &Course| -> Vec<i64> {
                (-3..length + 3)
                    .map(|k| course.offset(k, NTSC, 1000))
                    .collect()
            };
            let before = read(&course);
            for at in 0..length {
                let pointed = mapping.with_point(at, length).unwrap();
                let after = read(&pointed.course(length).unwrap());
                assert_eq!(after, before, "a speed point at {at} of {mapping:?}");
                let had = mapping.points.iter().any(|point| point.frame == at);
                let more = usize::from(!had);
                assert_eq!(pointed.points.len(), mapping.points.len() + more);
                assert_eq!(
                    pointed.course(length).unwrap().span(NTSC, 1000),
                    course.span(NTSC, 1000)
                );
            }
        }
        // At its own speed the places are those of a clip that has no speed point at all.
        let pointed = retime(&[(0, 1, 1), (1, 1, 1), (2, 1, 1), (3, 1, 1)]);
        let course = pointed.course(10).unwrap();
        let ticks: Vec<i64> = (0..5).map(|k| course.offset(k, NTSC, 1000)).collect();
        assert_eq!(ticks, [0, 33, 66, 100, 133]);
    }

    #[test]
    fn a_frame_on_a_speed_point_is_of_the_segment_that_begins_there() {
        let mapping = retime(&[(0, 1, 1), (4, 2, 1), (8, 1, 2)]);
        let course = mapping.course(12).unwrap();
        let speed = |num, den| SignedSpeed { num, den };
        assert_eq!(course.speed_at(3), speed(1, 1));
        assert_eq!(course.speed_at(4), speed(2, 1));
        assert_eq!(course.speed_at(7), speed(2, 1));
        assert_eq!(course.speed_at(8), speed(1, 2));
        assert_eq!(course.speed_at(11), speed(1, 2));
        // Before the clip is of its first segment, and after it of its last.
        assert_eq!(course.speed_at(-1), speed(1, 1));
        assert_eq!(course.speed_at(12), speed(1, 2));
        assert_eq!(course.speed_at(i64::MIN), speed(1, 1));
        assert_eq!(course.speed_at(i64::MAX), speed(1, 2));
        let segment = course.segment_of(4);
        assert_eq!((segment.from, segment.to), (4, 8));
        assert_eq!(course.segments().len(), 3);
        assert_eq!((course.length(), course.unit()), (12, 2));
        // Frame 4 shows where the first segment stopped, and goes on at twice the speed; frame
        // 8 shows where the second stopped, and each picture after it is shown twice.
        assert_eq!(
            shows(&mapping, 12),
            [0, 1, 2, 3, 4, 6, 8, 10, 12, 12, 13, 13]
        );
        // Places are counted in halves of a frame, as the slowest speed needs.
        assert_eq!(course.place(4), 8);
        assert_eq!(course.place(9), 25);
        // A frame that a transition shows goes on as the segment at that end does.
        assert_eq!(course.offset(-2, PAL, 25), -2);
        assert_eq!(course.offset(13, PAL, 25), 14);
        assert_eq!(course.shown(0, 0, PAL, 25), (0, 13));
        assert_eq!(course.shown(2, 3, PAL, 25), (-2, 15));
        assert_eq!(course.span(PAL, 25), 14);
    }

    #[test]
    fn a_freeze_holds_a_picture_and_the_clip_plays_on_from_it() {
        // Forward: frame 4 is held for 3 frames more, and nothing of the clip is lost.
        let forward = retime(&[(0, 1, 1)]);
        let frozen = forward.with_freeze(4, 3, 10).unwrap();
        assert_eq!(frozen.retime, retime(&[(0, 1, 1), (4, 0, 1), (7, 1, 1)]));
        assert_eq!(frozen.length, 13);
        assert_eq!(
            shows(&frozen.retime, 13),
            [0, 1, 2, 3, 4, 4, 4, 4, 5, 6, 7, 8, 9]
        );
        let course = frozen.retime.course(13).unwrap();
        assert!(course.speed_at(4).is_freeze() && course.speed_at(6).is_freeze());
        assert!(!course.speed_at(3).is_freeze() && !course.speed_at(7).is_freeze());
        assert_eq!(course.span(PAL, 25), 10);
        // Backward: the picture of frame 4 is the place where the frame ends.
        let backward = retime(&[(0, -1, 1)]);
        assert_eq!(shows(&backward, 10), [9, 8, 7, 6, 5, 4, 3, 2, 1, 0]);
        let frozen = backward.with_freeze(4, 3, 10).unwrap();
        assert_eq!(frozen.retime, retime(&[(0, -1, 1), (5, 0, 1), (8, -1, 1)]));
        assert_eq!(
            shows(&frozen.retime, 13),
            [9, 8, 7, 6, 5, 5, 5, 5, 4, 3, 2, 1, 0]
        );
        // At a speed point the point becomes the freeze, and the speed comes back after it.
        let pointed = retime(&[(0, 1, 1), (4, 2, 1)]);
        let frozen = pointed.with_freeze(4, 2, 8).unwrap();
        assert_eq!(frozen.retime, retime(&[(0, 1, 1), (4, 0, 1), (6, 2, 1)]));
        assert_eq!(shows(&frozen.retime, 10), [0, 1, 2, 3, 4, 4, 4, 6, 8, 10]);
        // At the last frame of a segment that goes backward the next speed point ends the
        // freeze.
        let turning = retime(&[(0, -1, 1), (5, 1, 1)]);
        assert_eq!(shows(&turning, 10), [4, 3, 2, 1, 0, 0, 1, 2, 3, 4]);
        let frozen = turning.with_freeze(4, 2, 10).unwrap();
        assert_eq!(frozen.retime, retime(&[(0, -1, 1), (5, 0, 1), (7, 1, 1)]));
        assert_eq!(
            shows(&frozen.retime, 12),
            [4, 3, 2, 1, 0, 0, 0, 0, 1, 2, 3, 4]
        );
        // The last frame of a clip is held to its new end.
        let frozen = forward.with_freeze(9, 2, 10).unwrap();
        assert_eq!(frozen.retime, retime(&[(0, 1, 1), (9, 0, 1), (11, 1, 1)]));
        assert_eq!(shows(&frozen.retime, 12)[8..], [8, 9, 9, 9]);
        let frozen = backward.with_freeze(9, 2, 10).unwrap();
        assert_eq!(frozen.retime, retime(&[(0, -1, 1), (10, 0, 1)]));
        assert_eq!(frozen.length, 12);
        assert_eq!(shows(&frozen.retime, 12)[8..], [1, 0, 0, 0]);
        // A clip that is a freeze frame from end to end shows where it begins.
        assert_eq!(shows(&Retime::still(), 5), [0; 5]);
        assert_eq!(Retime::still().course(5).unwrap().span(PAL, SCALE), 0);
    }

    #[test]
    fn a_segment_turned_backward_goes_back_over_what_was_shown() {
        // A clip of one segment shows what it showed, the last first.
        let forward = retime(&[(0, 1, 1)]);
        let backward = forward.with_segment_reversed(3, 10).unwrap();
        assert_eq!(backward, retime(&[(0, -1, 1)]));
        assert_eq!(shows(&backward, 10), [9, 8, 7, 6, 5, 4, 3, 2, 1, 0]);
        assert_eq!(backward.with_segment_reversed(0, 10).unwrap(), forward);
        // A later segment goes back from where the one before it stopped: the last picture of
        // the way up is the first of the way down.
        let pointed = forward.with_point(6, 10).unwrap();
        let turned = pointed.with_segment_reversed(7, 10).unwrap();
        assert_eq!(turned, retime(&[(0, 1, 1), (6, -1, 1)]));
        assert_eq!(shows(&turned, 10), [0, 1, 2, 3, 4, 5, 5, 4, 3, 2]);
        let course = turned.course(10).unwrap();
        assert_eq!(course.span(PAL, 25), 6);
        assert_eq!(course.shown(0, 0, PAL, 25), (0, 5));
        // What follows goes on from where it stops.
        let on = retime(&[(0, 1, 1), (4, -2, 1), (6, 1, 1)]);
        assert_eq!(shows(&on, 10), [0, 1, 2, 3, 2, 0, 0, 1, 2, 3]);
        // A clip that goes further back than it came begins in its media at the lowest place
        // that it shows, and its first frame is above it.
        let deep = retime(&[(0, 1, 1), (2, -1, 1)]);
        assert_eq!(shows(&deep, 10), [6, 7, 7, 6, 5, 4, 3, 2, 1, 0]);
        let course = deep.course(10).unwrap();
        assert_eq!(course.span(PAL, 25), 8);
        assert_eq!(course.shown(0, 0, PAL, 25), (0, 7));
        // A freeze has no way to turn.
        let still = Retime::still();
        assert_eq!(still.with_segment_reversed(0, 10).unwrap(), still);
    }

    #[test]
    fn a_place_is_found_in_the_first_frame_that_shows_it() {
        let find = |mapping: &Retime, length: i64, tick: i64| {
            mapping.course(length).unwrap().frame_of(tick, PAL, 25)
        };
        // Twice as fast: every other place is shown, and one between is of the frame before it.
        let fast = retime(&[(0, 2, 1)]);
        for tick in 0..20 {
            assert_eq!(find(&fast, 10, tick), Some(tick / 2), "{tick}");
        }
        assert_eq!(find(&fast, 10, 20), None);
        assert_eq!(find(&fast, 10, -1), None);
        // Half as fast: each place is shown twice, and found in the first of the two frames.
        let slow = retime(&[(0, 1, 2)]);
        for tick in 0..5 {
            assert_eq!(find(&slow, 10, tick), Some(2 * tick), "{tick}");
        }
        assert_eq!(find(&slow, 10, 5), None);
        // Backward: each place is shown once, and found.
        let backward = retime(&[(0, -1, 1)]);
        for tick in 0..10 {
            assert_eq!(find(&backward, 10, tick), Some(9 - tick), "{tick}");
        }
        assert_eq!(find(&backward, 10, 10), None);
        // A freeze: the first frame that holds the picture.
        let frozen = retime(&[(0, 1, 1), (4, 0, 1), (7, 1, 1)]);
        assert_eq!(find(&frozen, 13, 3), Some(3));
        assert_eq!(find(&frozen, 13, 4), Some(4));
        assert_eq!(find(&frozen, 13, 5), Some(8));
        assert_eq!(find(&frozen, 13, 9), Some(12));
        assert_eq!(find(&frozen, 13, 10), None);
        // A freeze at the highest place holds the place itself.
        let held = retime(&[(0, 1, 1), (5, 0, 1)]);
        assert_eq!(find(&held, 8, 5), Some(5));
        assert_eq!(find(&held, 8, 6), None);
        // Forward and back over it: the way up comes first.
        let turned = retime(&[(0, 1, 1), (6, -1, 1)]);
        assert_eq!(find(&turned, 10, 3), Some(3));
        assert_eq!(find(&turned, 10, 5), Some(5));
        assert_eq!(find(&turned, 10, 6), None);
        // Back first and then forward: the way down comes first.
        let returning = retime(&[(0, -1, 1), (5, 1, 1)]);
        assert_eq!(find(&returning, 10, 4), Some(0));
        assert_eq!(find(&returning, 10, 0), Some(4));
    }

    #[test]
    fn a_frame_and_the_place_that_it_shows_lead_back_to_each_other() {
        // Each with its length, and whether it goes one way from end to end.
        let mappings = [
            (retime(&[(0, 1, 1)]), 40, true),
            (retime(&[(0, 137, 250)]), 40, true),
            (retime(&[(0, -3, 1)]), 40, true),
            (retime(&[(0, 2, 1), (9, 0, 1), (15, 1, 3)]), 40, true),
            (retime(&[(0, -1, 2), (11, -5, 3), (30, 0, 1)]), 40, true),
            (
                retime(&[(0, 1, 3), (7, -2, 7), (20, 0, 1), (26, 5, 2), (40, -1, 1)]),
                60,
                false,
            ),
        ];
        for (mapping, length, one_way) in mappings {
            for (rate, scale) in [(PAL, 25), (PAL, SCALE), (NTSC, 1000)] {
                let course = mapping.course(length).unwrap();
                // From a frame: its place is on show in it, and in no frame after it first.
                for k in 0..length {
                    let tick = course.offset(k, rate, scale);
                    let found = course.frame_of(tick, rate, scale);
                    let found = found.unwrap_or_else(|| panic!("frame {k} of {mapping:?}"));
                    assert!(found <= k, "frame {k} of {mapping:?}");
                    assert!(course.offset(found, rate, scale) <= tick);
                    if one_way {
                        // The place leads back to the first frame that shows it.
                        assert_eq!(course.offset(found, rate, scale), tick);
                        let earlier = (0..found).any(|j| course.offset(j, rate, scale) == tick);
                        assert!(!earlier, "frame {k} of {mapping:?}");
                    }
                }
                // From a place: every place below the highest is on show in some frame, which
                // shows the place or one below it.
                let span = course.span(rate, scale);
                let (lowest, highest) = course.shown(0, 0, rate, scale);
                assert!(lowest == 0 && highest <= span, "{mapping:?}");
                for tick in (0..span).step_by(7) {
                    let found = course.frame_of(tick, rate, scale);
                    let found = found.unwrap_or_else(|| panic!("place {tick} of {mapping:?}"));
                    assert!((0..length).contains(&found));
                    assert!(course.offset(found, rate, scale) <= tick);
                }
                assert_eq!(course.frame_of(-1, rate, scale), None);
                assert_eq!(course.frame_of(span + 1, rate, scale), None);
            }
        }
    }

    #[test]
    fn both_curves_are_read_from_the_speed_points() {
        let course = retime(&[(0, 1, 1), (4, 2, 1), (8, 1, 2)])
            .course(12)
            .unwrap();
        let step = |from, to, percent| SpeedStep { from, to, percent };
        assert_eq!(
            course.speed_curve(),
            [step(0, 4, 100.0), step(4, 8, 200.0), step(8, 12, 50.0)]
        );
        // A control point for each speed point, and one where the clip ends: steeper where the
        // clip is faster.
        let point = |frame, source| FramePoint { frame, source };
        assert_eq!(
            course.frame_curve(),
            [
                point(0, 0.0),
                point(4, 4.0),
                point(8, 12.0),
                point(12, 14.0)
            ]
        );
        // Backward the speed is below 0, and the left control point is the higher.
        let course = retime(&[(0, -1, 1)]).course(10).unwrap();
        assert_eq!(course.speed_curve(), [step(0, 10, -100.0)]);
        assert_eq!(course.frame_curve(), [point(0, 10.0), point(10, 0.0)]);
        // A freeze is no speed, and level.
        let course = retime(&[(0, 1, 1), (4, 0, 1), (7, 3, 4)])
            .course(11)
            .unwrap();
        assert_eq!(
            course.speed_curve(),
            [step(0, 4, 100.0), step(4, 7, 0.0), step(7, 11, 75.0)]
        );
        assert_eq!(
            course.frame_curve(),
            [point(0, 0.0), point(4, 4.0), point(7, 4.0), point(11, 7.0)]
        );
        // What is written of them is plain numbers.
        let written = serde_json::to_value(course.speed_curve()[2]).unwrap();
        assert_eq!(
            written,
            serde_json::json!({ "from": 7, "to": 11, "percent": 75.0 })
        );
        let written = serde_json::to_value(course.frame_curve()[3]).unwrap();
        assert_eq!(written, serde_json::json!({ "frame": 11, "source": 7.0 }));
    }

    #[test]
    fn the_sound_of_a_segment_is_stretched_by_one_over_its_speed() {
        let course = retime(&[(0, 2, 1), (4, 1, 2), (8, 0, 1), (12, -4, 1)])
            .course(16)
            .unwrap();
        assert_eq!(course.stretch_at(0), Some(0.5));
        assert_eq!(course.stretch_at(3), Some(0.5));
        assert_eq!(course.stretch_at(4), Some(2.0));
        // A freeze has no sound to stretch.
        assert_eq!(course.stretch_at(8), None);
        // Backward the sound is as long as forward, and read the other way.
        assert_eq!(course.stretch_at(12), Some(0.25));
        assert!(course.speed_at(12).is_backward() && !course.speed_at(4).is_backward());
        let speed = |num, den| SignedSpeed { num, den };
        assert_eq!(speed(-4, 1).percent(), -400.0);
        assert_eq!(speed(1, 8).percent(), 12.5);
        assert_eq!(speed(0, 3).percent(), 0.0);
        assert_eq!(speed(-3, 2).magnitude(), Some(Speed { num: 3, den: 2 }));
        assert_eq!(speed(0, 3).magnitude(), None);
        assert_eq!(speed(i64::MIN, 1).magnitude(), None);
        assert_eq!(speed(i64::MIN, 1).reversed(), speed(i64::MAX, 1));
        assert_eq!(
            SignedSpeed::of(Speed { num: 3, den: 2 }, true),
            speed(-3, 2)
        );
        assert_eq!(SignedSpeed::of(Speed::ONE, false), SignedSpeed::ONE);
        assert_eq!(SignedSpeed::default(), SignedSpeed::ONE);
        assert!(SignedSpeed::FREEZE.is_freeze() && !SignedSpeed::ONE.is_freeze());
    }

    #[test]
    fn a_change_of_speed_keeps_the_frames_of_a_segment_or_what_it_shows() {
        let speed = |num, den| SignedSpeed { num, den };
        let pointed = retime(&[(0, 1, 1), (4, 1, 1), (8, 1, 1)]);
        assert_eq!(shows(&pointed, 12), [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
        // Its frames kept: it shows twice as much, and what follows begins further on.
        let kept = pointed.with_speed(5, speed(2, 1), 12).unwrap();
        assert_eq!(kept, retime(&[(0, 1, 1), (4, 2, 1), (8, 1, 1)]));
        assert_eq!(shows(&kept, 12), [0, 1, 2, 3, 4, 6, 8, 10, 12, 13, 14, 15]);
        // What it shows kept: it lasts half as long, and what follows shows what it showed.
        let faster = pointed.with_speed_rippled(5, speed(2, 1), 12).unwrap();
        assert_eq!(faster.retime, retime(&[(0, 1, 1), (4, 2, 1), (6, 1, 1)]));
        assert_eq!(faster.length, 10);
        assert_eq!(shows(&faster.retime, 10), [0, 1, 2, 3, 4, 6, 8, 9, 10, 11]);
        let slower = pointed.with_speed_rippled(5, speed(1, 2), 12).unwrap();
        assert_eq!(slower.retime, retime(&[(0, 1, 1), (4, 1, 2), (12, 1, 1)]));
        assert_eq!(slower.length, 16);
        assert_eq!(
            shows(&slower.retime, 16),
            [0, 1, 2, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 9, 10, 11]
        );
        // The last segment moves the end of the clip alone, and the first all the rest.
        let last = pointed.with_speed_rippled(11, speed(1, 4), 12).unwrap();
        assert_eq!(last.retime, retime(&[(0, 1, 1), (4, 1, 1), (8, 1, 4)]));
        assert_eq!(last.length, 24);
        let first = pointed.with_speed_rippled(0, speed(4, 1), 12).unwrap();
        assert_eq!(first.retime, retime(&[(0, 4, 1), (1, 1, 1), (5, 1, 1)]));
        assert_eq!(first.length, 9);
        // The other way at twice the speed lasts half as long too.
        let turned = pointed.with_speed_rippled(5, speed(-2, 1), 12).unwrap();
        assert_eq!(turned.retime, retime(&[(0, 1, 1), (4, -2, 1), (6, 1, 1)]));
        assert_eq!(turned.length, 10);
        // Frames are rounded down, so that no more is shown than was; but there is one.
        let whole = retime(&[(0, 1, 1)]);
        let odd = whole.with_speed_rippled(0, speed(2, 1), 5).unwrap();
        assert_eq!((odd.retime, odd.length), (retime(&[(0, 2, 1)]), 2));
        let one = whole.with_speed_rippled(0, speed(100, 1), 1).unwrap();
        assert_eq!((one.retime, one.length), (retime(&[(0, 100, 1)]), 1));
        // To a freeze or from one, the frames stay.
        let frozen = pointed.with_speed_rippled(5, speed(0, 1), 12).unwrap();
        assert_eq!(frozen.retime, retime(&[(0, 1, 1), (4, 0, 1), (8, 1, 1)]));
        assert_eq!(frozen.length, 12);
        let thawed = frozen
            .retime
            .with_speed_rippled(5, speed(3, 1), 12)
            .unwrap();
        assert_eq!(thawed.retime, retime(&[(0, 1, 1), (4, 3, 1), (8, 1, 1)]));
        assert_eq!(thawed.length, 12);
        // A clip that grew and is not given the room is cut at its end.
        assert_eq!(slower.retime.cut_to(12), retime(&[(0, 1, 1), (4, 1, 2)]));
        assert_eq!(slower.retime.cut_to(13).points.len(), 3);
        assert_eq!(slower.retime.cut_to(0), retime(&[]));
    }

    #[test]
    fn a_clip_put_back_to_its_speed_lasts_as_long_as_what_it_showed() {
        let reset = |mapping: &Retime, length: i64| mapping.reset(length).unwrap();
        let plain = |length: i64| Retimed {
            retime: Retime::default(),
            length,
        };
        assert_eq!(reset(&retime(&[(0, 2, 1)]), 50), plain(100));
        assert_eq!(reset(&retime(&[(0, -2, 1)]), 50), plain(100));
        // Rounded down: 51 frames at half the speed show 25 frames and a half.
        assert_eq!(reset(&retime(&[(0, 1, 2)]), 51), plain(25));
        // Forward 6 frames and back over 4 of them: 6 frames of the source.
        assert_eq!(reset(&retime(&[(0, 1, 1), (6, -1, 1)]), 10), plain(6));
        // A freeze frame is one frame.
        assert_eq!(reset(&Retime::still(), 10), plain(1));
        assert_eq!(reset(&Retime::default(), 10), plain(10));
        let long = retime(&[(0, 100, 1)]);
        assert_eq!(
            long.reset(LARGEST / 2).unwrap_err(),
            OpError::TooLarge { what: "length" }
        );
    }

    #[test]
    fn the_frames_that_fit_in_a_span_of_media_are_counted() {
        // Twice as fast for 10 frames, then half as fast: 20 frames of the source, and half a
        // frame more with each frame after.
        let mapping = retime(&[(0, 2, 1), (10, 1, 2)]);
        let fit = |span: i64| mapping.frames_for_span(span, PAL, 25).unwrap();
        assert_eq!(fit(25), 21);
        assert_eq!(fit(26), 23);
        assert_eq!(fit(20), 11);
        // Not even with the frame of its last speed point.
        assert_eq!(fit(19), 0);
        assert_eq!(fit(-1), 0);
        assert_eq!(fit(i64::MAX), LARGEST);
        // A clip that ends in a freeze shows no more however long it lasts.
        let held = retime(&[(0, 1, 1), (5, 0, 1)]);
        assert_eq!(held.frames_for_span(5, PAL, 25).unwrap(), LARGEST);
        assert_eq!(held.frames_for_span(4, PAL, 25).unwrap(), 0);
        // Forward 6 frames and then back: no more is shown until it is back where it began,
        // 12 frames in, and then a frame more with each frame.
        let turned = retime(&[(0, 1, 1), (6, -1, 1)]);
        assert_eq!(turned.frames_for_span(6, PAL, 25).unwrap(), 12);
        assert_eq!(turned.frames_for_span(10, PAL, 25).unwrap(), 16);
        assert_eq!(turned.frames_for_span(5, PAL, 25).unwrap(), 0);
        // A mapping that is none has no length.
        assert_eq!(
            retime(&[]).frames_for_span(100, PAL, 25).unwrap_err(),
            range("retime")
        );
        assert_eq!(
            retime(&[(0, 101, 1)])
                .frames_for_span(100, PAL, 25)
                .unwrap_err(),
            range("speed")
        );
    }

    #[test]
    fn the_longest_clip_is_counted_without_overflow() {
        // Twice as fast for half of the longest clip that there is, then half as fast: the
        // last frame is 2^53 + 2^51 - 1 frames into the source.
        let half = 1_i64 << 52;
        let mapping = retime(&[(0, 2, 1), (half, 1, 2)]);
        let course = mapping.course(LARGEST).unwrap();
        assert_eq!(course.offset(half, PAL, 25), 1 << 53);
        assert_eq!(
            course.offset(LARGEST - 1, PAL, 25),
            (1 << 53) + (1 << 51) - 1
        );
        assert_eq!(
            course.frame_of((1 << 53) + (1 << 51) - 1, PAL, 25),
            Some(LARGEST - 1)
        );
        assert_eq!(course.frame_of((1 << 53) - 2, PAL, 25), Some(half - 1));
        assert_eq!(course.span(PAL, 25), (1 << 53) + (1 << 51) - 1);
        // In the finest ticks the place is more than an i64 holds: it is the most there is.
        let finest = Rate {
            num: 1,
            den: u32::MAX,
        };
        assert_eq!(course.offset(LARGEST - 1, finest, u32::MAX), i64::MAX);
        assert_eq!(course.offset(-LARGEST, finest, u32::MAX), i64::MIN);
        assert_eq!(course.span(finest, u32::MAX), i64::MAX);
        assert_eq!(
            course.shown(LARGEST, LARGEST, finest, u32::MAX),
            (i64::MIN, i64::MAX)
        );
        // The furthest frames that can be asked for, forward and backward, as a clip of one
        // speed counts them: as far before a clip that plays backward as the frame that it
        // stands for, `length - 1 - k`, is one that an i64 holds.
        for (speed, reverse) in [
            (Speed::ONE, false),
            (Speed::ONE, true),
            (Speed { num: 100, den: 1 }, false),
            (Speed { num: 100, den: 1 }, true),
            (Speed { num: 1, den: 100 }, true),
        ] {
            let source_in = Time {
                ticks: 0,
                scale: 25,
            };
            let mut clip = Clip::of_media("c1", "m1", source_in, 0, LARGEST);
            clip.speed = speed;
            clip.reverse = reverse;
            let course = Retime::constant(speed, reverse).course(LARGEST).unwrap();
            let furthest = i64::MIN + LARGEST;
            for k in [furthest, -LARGEST, -1, 0, LARGEST - 1, LARGEST, i64::MAX] {
                assert_eq!(
                    course.offset(k, PAL, 25),
                    span::source_tick(&clip, k, PAL, 25),
                    "frame {k} at {speed:?}, backward {reverse}"
                );
            }
        }
        // A change that would make the clip longer than the longest is refused.
        let fastest = retime(&[(0, 100, 1)]);
        let slowest = SignedSpeed { num: 1, den: 100 };
        assert_eq!(
            fastest
                .with_speed_rippled(0, slowest, LARGEST / 2)
                .unwrap_err(),
            OpError::TooLarge { what: "length" }
        );
        assert_eq!(
            fastest.with_freeze(0, 2, LARGEST - 1).unwrap_err(),
            OpError::TooLarge { what: "length" }
        );
        assert_eq!(
            fastest.with_freeze(0, i64::MAX, 10).unwrap_err(),
            OpError::TooLarge { what: "length" }
        );
        assert_eq!(
            fastest.with_freeze(0, 1, LARGEST - 1).unwrap().length,
            LARGEST
        );
    }

    #[test]
    fn a_mapping_that_is_not_one_of_the_clip_is_refused_and_told_why() {
        let refused = |mapping: &Retime, length: i64| mapping.check(length).unwrap_err();
        // No speed point, or none at the first frame.
        assert_eq!(refused(&retime(&[]), 10), range("retime"));
        assert_eq!(refused(&retime(&[(1, 1, 1)]), 10), range("retime"));
        assert_eq!(
            refused(&retime(&[(-1, 1, 1), (0, 1, 1)]), 10),
            range("retime")
        );
        // Two at one frame, or one before the one before it.
        assert_eq!(
            refused(&retime(&[(0, 1, 1), (4, 2, 1), (4, 1, 1)]), 10),
            range("retime")
        );
        assert_eq!(
            refused(&retime(&[(0, 1, 1), (6, 2, 1), (4, 1, 1)]), 10),
            range("retime")
        );
        // One at the end of the clip, or after it; and a clip of no frame has no mapping.
        assert_eq!(
            refused(&retime(&[(0, 1, 1), (10, 2, 1)]), 10),
            range("retime")
        );
        assert_eq!(
            refused(&retime(&[(0, 1, 1), (11, 2, 1)]), 10),
            range("retime")
        );
        assert_eq!(refused(&Retime::default(), 0), range("retime"));
        assert_eq!(refused(&Retime::default(), -5), range("retime"));
        retime(&[(0, 1, 1), (9, 2, 1)]).check(10).unwrap();
        Retime::default().check(1).unwrap();
        // Too many of them.
        let many = |count: i64| Retime {
            points: (0..count)
                .map(|frame| SpeedPoint {
                    frame,
                    speed: SignedSpeed::ONE,
                })
                .collect(),
        };
        let most = i64::try_from(MOST_POINTS).unwrap();
        many(most).check(5000).unwrap();
        assert_eq!(refused(&many(most + 1), 5000), range("retime"));
        // A speed that is none, or too slow or too fast either way.
        for (num, den) in [(1, 0), (0, 0), (1, 101), (101, 1), (-1, 101), (-101, 1)] {
            let mapping = retime(&[(0, 1, 1), (4, num, den)]);
            assert_eq!(refused(&mapping, 10), range("speed"), "{num}/{den}");
        }
        assert_eq!(refused(&retime(&[(0, i64::MIN, 1)]), 10), range("speed"));
        assert_eq!(
            refused(&retime(&[(0, i64::MAX, u32::MAX)]), 10),
            range("speed")
        );
        for (num, den) in [
            (1, 100),
            (100, 1),
            (-1, 100),
            (-100, 1),
            (0, 1),
            (0, 7),
            (6, 4),
        ] {
            retime(&[(0, 1, 1), (4, num, den)]).check(10).unwrap();
        }
        let widest = retime(&[(0, i64::from(u32::MAX), u32::MAX)]);
        widest.check(10).unwrap();
        // Speeds that no unit of 32 bits counts: 65537 and 65539 share no factor.
        let apart = retime(&[(0, 65537, 65537), (4, 65539, 65539)]);
        assert_eq!(refused(&apart, 10), range("retime_unit"));
        let near = retime(&[(0, 65537, 65537), (4, 65535, 65535)]);
        near.check(10).unwrap();
        assert_eq!(near.course(10).unwrap().unit(), 65537 * 65535);
        // A clip longer than the longest.
        assert_eq!(
            refused(&Retime::default(), LARGEST + 1),
            OpError::TooLarge { what: "length" }
        );
        Retime::default().check(LARGEST).unwrap();
        // What is refused is not counted out either.
        assert_eq!(retime(&[]).course(10).unwrap_err(), range("retime"));
        assert_eq!(apart.course(10).unwrap_err(), range("retime_unit"));
    }

    #[test]
    fn a_change_that_the_clip_has_no_frame_or_no_room_for_is_refused() {
        let mapping = retime(&[(0, 1, 1), (4, 2, 1)]);
        let speed = |num, den| SignedSpeed { num, den };
        for at in [-1, 10, 11, i64::MIN, i64::MAX] {
            assert_eq!(mapping.with_point(at, 10).unwrap_err(), range("frame"));
            assert_eq!(
                mapping.with_speed(at, speed(1, 2), 10).unwrap_err(),
                range("frame")
            );
            assert_eq!(
                mapping.with_speed_rippled(at, speed(1, 2), 10).unwrap_err(),
                range("frame")
            );
            assert_eq!(mapping.with_freeze(at, 5, 10).unwrap_err(), range("frame"));
            assert_eq!(
                mapping.with_segment_reversed(at, 10).unwrap_err(),
                range("frame")
            );
        }
        // A speed point where one is already leaves the mapping as it was.
        assert_eq!(mapping.with_point(0, 10).unwrap(), mapping);
        assert_eq!(mapping.with_point(4, 10).unwrap(), mapping);
        assert_eq!(
            mapping.with_point(9, 10).unwrap(),
            retime(&[(0, 1, 1), (4, 2, 1), (9, 2, 1)])
        );
        // A speed that no segment may have.
        assert_eq!(
            mapping.with_speed(5, speed(101, 1), 10).unwrap_err(),
            range("speed")
        );
        assert_eq!(
            mapping.with_speed_rippled(5, speed(1, 0), 10).unwrap_err(),
            range("speed")
        );
        assert_eq!(
            mapping
                .with_speed(5, speed(65539, 65539), 10)
                .unwrap()
                .with_speed(0, speed(65537, 65537), 10)
                .unwrap_err(),
            range("retime_unit")
        );
        // A freeze of no frame.
        assert_eq!(mapping.with_freeze(5, 0, 10).unwrap_err(), range("hold"));
        assert_eq!(mapping.with_freeze(5, -3, 10).unwrap_err(), range("hold"));
        // A mapping that is not one of the clip is not changed.
        let wrong = retime(&[(0, 1, 1), (12, 2, 1)]);
        assert_eq!(wrong.with_point(5, 10).unwrap_err(), range("retime"));
        assert_eq!(wrong.with_freeze(5, 1, 10).unwrap_err(), range("retime"));
        assert_eq!(wrong.reset(10).unwrap_err(), range("retime"));
        // No room for a speed point more.
        let most = i64::try_from(MOST_POINTS).unwrap();
        let full = Retime {
            points: (0..most)
                .map(|frame| SpeedPoint {
                    frame: 2 * frame,
                    speed: SignedSpeed::ONE,
                })
                .collect(),
        };
        full.check(5000).unwrap();
        assert_eq!(full.with_point(1, 5000).unwrap_err(), range("retime"));
        assert_eq!(full.with_freeze(1, 1, 5000).unwrap_err(), range("retime"));
        assert_eq!(full.with_point(2, 5000).unwrap(), full);
    }

    #[test]
    fn a_mapping_is_written_and_read_back_and_what_is_left_out_is_as_the_source_is() {
        let mapping = retime(&[(0, 1, 1), (4, 0, 1), (7, -1, 2)]);
        let written = serde_json::to_value(&mapping).unwrap();
        assert_eq!(
            written,
            serde_json::json!({
                "points": [
                    { "frame": 0, "speed": { "num": 1, "den": 1 } },
                    { "frame": 4, "speed": { "num": 0, "den": 1 } },
                    { "frame": 7, "speed": { "num": -1, "den": 2 } }
                ]
            })
        );
        assert_eq!(serde_json::from_value::<Retime>(written).unwrap(), mapping);
        // No mapping, and a speed point without a speed, play as the source is.
        let read: Retime = serde_json::from_str("{}").unwrap();
        assert_eq!(read, Retime::default());
        let read: Retime = serde_json::from_str(r#"{ "points": [{ "frame": 0 }] }"#).unwrap();
        assert_eq!(read, Retime::default());
        // A speed with a `den` of 0 is refused as it is read.
        let none = r#"{ "points": [{ "frame": 0, "speed": { "num": 1, "den": 0 } }] }"#;
        let error = serde_json::from_str::<Retime>(none).unwrap_err();
        assert!(error.to_string().contains("a `den` of 0"), "{error}");
        // So is a speed point without a frame, and a word that is not known.
        assert!(serde_json::from_str::<Retime>(r#"{ "points": [{}] }"#).is_err());
        assert!(serde_json::from_str::<Retime>(r#"{ "points": [], "curve": [] }"#).is_err());
        assert!(serde_json::from_str::<SpeedPoint>(r#"{ "frame": 0, "ease": 1 }"#).is_err());
        let extra = r#"{ "num": 1, "den": 1, "percent": 100 }"#;
        assert!(serde_json::from_str::<SignedSpeed>(extra).is_err());
    }

    #[test]
    fn a_clip_from_a_frame_on_shows_what_it_showed_from_there() {
        // Forward, backward, with a freeze, and all of them mixed; each with its length.
        let mappings = [
            (retime(&[(0, 1, 1)]), 40),
            (retime(&[(0, 137, 250)]), 40),
            (retime(&[(0, -137, 250)]), 40),
            (retime(&[(0, 2, 1), (9, 0, 1), (15, 1, 3)]), 40),
            (retime(&[(0, -1, 2), (11, -5, 3), (30, 0, 1)]), 40),
            (
                retime(&[(0, 1, 3), (7, -2, 7), (20, 0, 1), (26, 5, 2), (40, -1, 1)]),
                60,
            ),
        ];
        for (mapping, length) in mappings {
            let course = mapping.course(length).unwrap();
            let unit = course.unit();
            // Where a unit of a frame lasts a whole number of ticks, every frame is on its
            // tick. At 29.97 frames a second in thousandths of a second, and at 25 in ticks
            // of 1/15360, it lasts none: a frame is on its tick or on the one before.
            let grids = [
                (PAL, 25 * unit, true),
                (NTSC, 30000 * unit, true),
                (NTSC, 1000, false),
                (PAL, SCALE, false),
            ];
            for (rate, scale, whole) in grids {
                // Also with the head moved out, where the first segment goes on before it.
                for from in -3..length {
                    let trimmed = mapping.from_frame(from, length, rate, scale).unwrap();
                    assert_eq!(trimmed.length, length - from);
                    let onward = trimmed.retime.course(trimmed.length).unwrap();
                    // The frames that are kept, and those that a transition shows after them.
                    for k in from..length + 3 {
                        let was = course.tick(1000, k, rate, scale);
                        let is = onward.tick(1000 + trimmed.later, k - from, rate, scale);
                        let what = format!("frame {k} from {from} of {mapping:?}, {rate:?}");
                        if whole {
                            assert_eq!(is, was, "{what} in {scale}");
                        } else {
                            assert!(is == was || is == was - 1, "{what}: {is} for {was}");
                        }
                    }
                }
            }
        }
        // A clip that plays backward from end to end begins in its media where its last frame
        // is, which a head that moves leaves where it was: every frame is on its tick at any
        // rate.
        let backward = retime(&[(0, -137, 250)]);
        let course = backward.course(40).unwrap();
        for from in -3..40 {
            let trimmed = backward.from_frame(from, 40, NTSC, 1000).unwrap();
            assert_eq!((&trimmed.retime, trimmed.later), (&backward, 0));
            let onward = trimmed.retime.course(trimmed.length).unwrap();
            for k in from..43 {
                assert_eq!(
                    onward.tick(1000, k - from, NTSC, 1000),
                    course.tick(1000, k, NTSC, 1000),
                    "frame {k} from {from}"
                );
            }
        }
    }

    #[test]
    fn the_head_of_a_clip_is_moved_in_and_out() {
        let mapping = retime(&[(0, 1, 1), (4, 2, 1), (8, 1, 2)]);
        assert_eq!(
            shows(&mapping, 12),
            [0, 1, 2, 3, 4, 6, 8, 10, 12, 12, 13, 13]
        );
        // From frame 5: the segment that it is in begins the clip, 6 frames of the source on.
        let trimmed = mapping.from_frame(5, 12, PAL, 25).unwrap();
        assert_eq!(trimmed.retime, retime(&[(0, 2, 1), (3, 1, 2)]));
        assert_eq!((trimmed.length, trimmed.later), (7, 6));
        assert_eq!(shows(&trimmed.retime, 7), [0, 2, 4, 6, 6, 7, 7]);
        // From a speed point: it is the first.
        let trimmed = mapping.from_frame(8, 12, PAL, 25).unwrap();
        assert_eq!(trimmed.retime, retime(&[(0, 1, 2)]));
        assert_eq!((trimmed.length, trimmed.later), (4, 12));
        assert_eq!(
            trimmed.retime.as_constant(),
            Some((Speed { num: 1, den: 2 }, false))
        );
        // From frame 0 it is as it was.
        let trimmed = mapping.from_frame(0, 12, PAL, 25).unwrap();
        assert_eq!(
            (&trimmed.retime, trimmed.length, trimmed.later),
            (&mapping, 12, 0)
        );
        // Moved out by 2 frames: the first segment goes on before, and the clip begins 2 frames
        // of the source earlier.
        let trimmed = mapping.from_frame(-2, 12, PAL, 25).unwrap();
        assert_eq!(trimmed.retime, retime(&[(0, 1, 1), (6, 2, 1), (10, 1, 2)]));
        assert_eq!((trimmed.length, trimmed.later), (14, -2));
        assert_eq!(
            shows(&trimmed.retime, 14),
            [0, 1, 2, 3, 4, 5, 6, 8, 10, 12, 14, 14, 15, 15]
        );
        // A clip that goes up and then further down than it came begins where it ends: with
        // its head moved past the way up, it is one that plays backward, from where it began.
        let deep = retime(&[(0, 1, 1), (2, -1, 1)]);
        assert_eq!(shows(&deep, 10), [6, 7, 7, 6, 5, 4, 3, 2, 1, 0]);
        let trimmed = deep.from_frame(4, 10, PAL, 25).unwrap();
        assert_eq!(trimmed.retime, retime(&[(0, -1, 1)]));
        assert_eq!((trimmed.length, trimmed.later), (6, 0));
        assert_eq!(shows(&trimmed.retime, 6), [5, 4, 3, 2, 1, 0]);
        // The clip has no such frame, or would last too long.
        assert_eq!(
            mapping.from_frame(12, 12, PAL, 25).unwrap_err(),
            range("frame")
        );
        assert_eq!(
            mapping.from_frame(i64::MAX, 12, PAL, 25).unwrap_err(),
            range("frame")
        );
        assert_eq!(
            mapping.from_frame(-LARGEST, 12, PAL, 25).unwrap_err(),
            OpError::TooLarge { what: "length" }
        );
        assert_eq!(
            mapping.from_frame(i64::MIN, 12, PAL, 25).unwrap_err(),
            OpError::TooLarge { what: "length" }
        );
        let wrong = retime(&[(0, 1, 1), (12, 2, 1)]);
        assert_eq!(
            wrong.from_frame(5, 12, PAL, 25).unwrap_err(),
            range("retime")
        );
    }

    #[test]
    fn the_tail_of_a_clip_is_moved_in_and_out() {
        let mapping = retime(&[(0, 1, 1), (4, 2, 1), (8, 1, 2)]);
        // Shorter, the speed points at its new end and after it go; longer, its last segment
        // goes on. A clip that only goes up begins where it did.
        let shorter = mapping.with_length(8, 12, PAL, 25).unwrap();
        assert_eq!(shorter.retime, retime(&[(0, 1, 1), (4, 2, 1)]));
        assert_eq!((shorter.length, shorter.later), (8, 0));
        let shortest = mapping.with_length(4, 12, PAL, 25).unwrap();
        assert_eq!(shortest.retime.as_constant(), Some((Speed::ONE, false)));
        let longer = mapping.with_length(14, 12, PAL, 25).unwrap();
        assert_eq!(
            (&longer.retime, longer.length, longer.later),
            (&mapping, 14, 0)
        );
        assert_eq!(shows(&longer.retime, 14)[10..], [13, 13, 14, 14]);
        // A clip that ends on a way down further than it came begins where it ends: with its
        // tail moved in it begins later in its media, and with its tail moved out, earlier.
        let deep = retime(&[(0, 1, 1), (2, -1, 1)]);
        assert_eq!(shows(&deep, 10), [6, 7, 7, 6, 5, 4, 3, 2, 1, 0]);
        let shorter = deep.with_length(5, 10, PAL, 25).unwrap();
        assert_eq!(
            (&shorter.retime, shorter.length, shorter.later),
            (&deep, 5, 5)
        );
        assert_eq!(shows(&shorter.retime, 5), [1, 2, 2, 1, 0]);
        let longer = deep.with_length(12, 10, PAL, 25).unwrap();
        assert_eq!(
            (&longer.retime, longer.length, longer.later),
            (&deep, 12, -2)
        );
        assert_eq!(
            shows(&longer.retime, 12),
            [8, 9, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0]
        );
        // Every frame that is kept is on its tick where a unit of a frame lasts whole ticks,
        // and on it or on the one before elsewhere.
        let mixed = retime(&[(0, 1, 3), (7, -2, 7), (20, 0, 1), (26, 5, 2), (40, -1, 1)]);
        let course = mixed.course(60).unwrap();
        for (rate, scale, whole) in [(NTSC, 30000 * 42, true), (NTSC, 1000, false)] {
            for to in 1..70 {
                let trimmed = mixed.with_length(to, 60, rate, scale).unwrap();
                let onward = trimmed.retime.course(to).unwrap();
                for k in -3..to.min(60) {
                    let was = course.tick(1000, k, rate, scale);
                    let is = onward.tick(1000 + trimmed.later, k, rate, scale);
                    if whole {
                        assert_eq!(is, was, "frame {k} of {to}");
                    } else {
                        assert!(is == was || is == was - 1, "frame {k} of {to}");
                    }
                }
            }
        }
        // A clip of no frame, or of too many.
        assert_eq!(
            mapping.with_length(0, 12, PAL, 25).unwrap_err(),
            range("length")
        );
        assert_eq!(
            mapping.with_length(LARGEST + 1, 12, PAL, 25).unwrap_err(),
            OpError::TooLarge { what: "length" }
        );
        mapping.with_length(LARGEST, 12, PAL, 25).unwrap();
        let wrong = retime(&[(0, 1, 1), (12, 2, 1)]);
        assert_eq!(
            wrong.with_length(5, 12, PAL, 25).unwrap_err(),
            range("retime")
        );
    }

    #[test]
    fn what_a_clip_shows_ends_at_the_highest_place_of_its_path() {
        let mapping = retime(&[(0, 1, 1), (4, 2, 1), (8, 1, 2)]);
        let course = mapping.course(12).unwrap();
        assert_eq!(course.reach(0, 0, PAL, 25), 14);
        assert_eq!(course.reach(0, 0, PAL, 25), course.span(PAL, 25));
        // With 3 frames more after it, at half the speed, it goes a frame and a half on.
        assert_eq!(course.reach(2, 3, PAL, 25), 15);
        // Backward, the frames before the clip are the highest.
        let backward = retime(&[(0, -1, 1)]).course(10).unwrap();
        assert_eq!(backward.reach(0, 0, PAL, 25), 10);
        assert_eq!(backward.reach(2, 3, PAL, 25), 12);
        // Up 6 frames and down again: where it turns.
        let turned = retime(&[(0, 1, 1), (6, -1, 1)]).course(10).unwrap();
        assert_eq!(turned.reach(0, 0, PAL, 25), 6);
        assert_eq!(turned.shown(0, 0, PAL, 25), (0, 5));
        // A freeze frame goes nowhere.
        assert_eq!(Retime::still().course(5).unwrap().reach(4, 4, PAL, 25), 0);
        // Of one speed, it is the ticks of the frames, as a clip of that speed counts them.
        for (speed, reverse) in [
            (Speed { num: 137, den: 250 }, false),
            (Speed { num: 137, den: 250 }, true),
        ] {
            let course = Retime::constant(speed, reverse).course(40).unwrap();
            for (rate, scale) in [(PAL, SCALE), (NTSC, 1000)] {
                let (before, after) = if reverse { (5, 2) } else { (2, 5) };
                assert_eq!(
                    course.reach(before, after, rate, scale),
                    span::ticks_of_scaled(45, speed, rate, scale)
                );
            }
        }
    }
}
