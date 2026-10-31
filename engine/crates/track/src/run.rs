//! A track over a clip: the commands of the tracker (pp. 3231, 3236) writing the track data.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::cloud::{CloudOptions, CloudTracker};
use crate::data::TrackData;
use crate::image::Luma;
use crate::linalg::{Mat3, Point};
use crate::motion::{Components, Motion};
use crate::point::{PointOptions, PointTracker};
use crate::region::Region;
use crate::{Step, TrackError};

/// Which way a track goes (pp. 3231, 3236).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Track Forward (Ctrl+T): from the playhead to the last frame of the clip.
    Forward,
    /// Track Reverse (Alt+T): from the playhead to the first frame of the clip.
    Reverse,
    /// Track Forward and Back: forward to the last frame, then from the playhead back to the
    /// first.
    ForwardAndBack,
    /// Track One Frame Forward (Alt+→).
    OneForward,
    /// Track One Frame Reverse (Alt+←).
    OneReverse,
}

/// Which tracker a track uses (p. 3234). `IntelliTrack` is not offered.
#[derive(Clone, Debug, PartialEq)]
pub enum Tracker {
    /// The cloud tracker, the default.
    Cloud(CloudOptions),
    /// The point tracker, with its crosshairs where they are at the playhead.
    Point {
        /// The crosshairs, in pixels of the frame.
        points: Vec<Point>,
        /// Its settings.
        options: PointOptions,
    },
}

/// A track to make.
#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    /// The tracker.
    pub tracker: Tracker,
    /// The boxes of the motion to apply, as they are when the track starts (p. 3235).
    pub components: Components,
    /// Where the window is at `start`, as it is seen there (its own parameters, the track so far
    /// and the keyframes of Frame mode all applied).
    pub region: Region,
    /// The frame of the playhead.
    pub start: i64,
    /// The first frame of the clip.
    pub first: i64,
    /// The last frame of the clip.
    pub last: i64,
    /// Which way.
    pub direction: Direction,
}

/// How a track went.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    /// The frames tracked, the playhead's not counted.
    pub frames: usize,
    /// Those of them where the subject was lost (taken off the curves).
    pub lost: usize,
    /// Whether Stop ended it.
    pub stopped: bool,
    /// The milliseconds of tracking per frame, on average (the frames' decoding not counted).
    pub ms_per_frame: f64,
}

/// Track `job` and write it into `data`, which already holds the track so far of the window.
///
/// - The motion at the playhead is the one `data` has there (none for a fresh window), and
///   every frame tracked continues it, so a track started from the last tracked frame carries on
///   (p. 3232) and one started after moving a crosshair in Frame mode stays on the same path
///   (p. 3249). The frames tracked overwrite what was there (p. 3231); a frame where the subject
///   was lost is taken off, so the curves bridge it with a line.
/// - `frames` gives the luma of a frame of the clip; `stop` is looked at between frames (Stop,
///   Ctrl+Alt+T); `progress` hears of each frame as it is done.
///
/// # Errors
/// [`TrackError::OutOfRange`] when the playhead is outside the clip, [`TrackError::MissingFrame`]
/// when `frames` has no frame, and those of [`CloudTracker::new`] and [`PointTracker::new`].
/// Frames tracked before an error stay written.
pub fn run(
    job: &Job,
    data: &mut TrackData,
    frames: &mut dyn FnMut(i64) -> Option<Luma>,
    stop: &AtomicBool,
    progress: &mut dyn FnMut(i64, &Step),
) -> Result<Report, TrackError> {
    if job.start < job.first || job.start > job.last {
        return Err(TrackError::OutOfRange {
            start: job.start,
            first: job.first,
            last: job.last,
        });
    }
    let legs: &[(i64, Option<usize>)] = match job.direction {
        Direction::Forward => &[(1, None)],
        Direction::Reverse => &[(-1, None)],
        Direction::ForwardAndBack => &[(1, None), (-1, None)],
        Direction::OneForward => &[(1, Some(1))],
        Direction::OneReverse => &[(-1, Some(1))],
    };
    data.components = job.components;
    let mut report = Report::default();
    let mut busy = 0.0;
    for &(way, most) in legs {
        busy += leg(job, data, way, most, frames, stop, progress, &mut report)?;
        if report.stopped {
            break;
        }
    }
    if report.frames > 0 {
        #[allow(clippy::cast_precision_loss, reason = "a count of frames")]
        let per = busy / report.frames as f64;
        report.ms_per_frame = per;
    }
    Ok(report)
}

/// Either tracker, stepped alike.
enum Running {
    Cloud(CloudTracker),
    Point(PointTracker),
}

/// One way of a track; the milliseconds spent tracking.
#[allow(
    clippy::too_many_arguments,
    reason = "the parts of a track, passed on from run"
)]
fn leg(
    job: &Job,
    data: &mut TrackData,
    way: i64,
    most: Option<usize>,
    frames: &mut dyn FnMut(i64) -> Option<Luma>,
    stop: &AtomicBool,
    progress: &mut dyn FnMut(i64, &Step),
    report: &mut Report,
) -> Result<f64, TrackError> {
    let end = if way > 0 { job.last } else { job.first };
    let first = frames(job.start).ok_or(TrackError::MissingFrame(job.start))?;
    data.frame_size = [first.width(), first.height()];
    if !data.has_sample(job.start) {
        let here = data.motion_at(job.start);
        data.set_sample(job.start, here, 1.0);
    }
    let base: Mat3 = data.track_matrix(job.start);
    let mut tracker = match &job.tracker {
        Tracker::Cloud(options) => Running::Cloud(CloudTracker::new(
            &first,
            job.region.clone(),
            options.clone(),
        )?),
        Tracker::Point { points, options } => {
            data.set_point_count(points.len());
            for (i, &p) in points.iter().enumerate() {
                data.set_point(i, job.start, Some(p));
            }
            Running::Point(PointTracker::new(&first, points, options.clone())?)
        }
    };
    let mut rotate = data.motion_at(job.start).rotate;
    let mut frame = job.start;
    let mut done = 0;
    let mut busy = 0.0;
    while frame != end && most.is_none_or(|m| done < m) {
        if stop.load(Ordering::Relaxed) {
            report.stopped = true;
            break;
        }
        frame += way;
        let luma = frames(frame).ok_or(TrackError::MissingFrame(frame))?;
        let clock = Instant::now();
        let found = match &mut tracker {
            Running::Cloud(t) => t.step_owned(luma)?,
            Running::Point(t) => {
                let out = t.step_owned(luma)?;
                for (i, p) in out.points.iter().enumerate() {
                    data.set_point(i, frame, *p);
                }
                out.step
            }
        };
        busy += clock.elapsed().as_secs_f64() * 1e3;
        done += 1;
        report.frames += 1;
        let motion = (!found.lost)
            .then(|| Motion::from_matrix(&found.motion.mul(&base), data.anchor))
            .flatten();
        if let Some(mut motion) = motion {
            // Keep the rotation continuous across ±π.
            let turn = std::f64::consts::TAU;
            motion.rotate += turn * ((rotate - motion.rotate) / turn).round();
            rotate = motion.rotate;
            data.set_sample(frame, motion.masked(job.components), found.confidence);
        } else {
            report.lost += 1;
            data.remove_frame(frame);
        }
        progress(frame, &found);
    }
    Ok(busy)
}
