//! `track.stabilize` (T18a): the motion of the whole picture of a clip measured frame by frame
//! for its stabilizer (Reference Manual of Resolve 20, pp. 633–634 and 1064–1065), and written
//! into the clip as the analysis of its stabilizer (`columbina_model::stabilize`).
//!
//! It goes as a track does (the module above): it is checked against the project as it is, a
//! native gesture of the connection that asked is reserved, it is answered at once and runs on
//! a thread of its own, `track.progress` tells how far it went and `track.state` how it ended;
//! and `track.stop`, a change of the project by anyone else, Undo or Redo, and the connection
//! going end it. What differs:
//!
//! - **What is read.** Every frame of the source from the first that the clip shows to the
//!   last, in the order of the source, whatever the playhead and whichever way the clip plays:
//!   the frames of its media as a track of a window of the clip reads them (its proxy where it
//!   has a usable one, upright, at the analysis size, as luma).
//! - **What is followed.** The whole picture, by the cloud tracker
//!   (`columbina_track::CloudTracker`) over a region that holds the frame wherever the picture
//!   goes within a frame of it, its motion fitted as a homography for Perspective and as a
//!   similarity for Similarity and Translation. The tracker is started anew every [`RESTART`]
//!   frames from the frame that it reached, so that its features are found anew all over the
//!   frame as the picture moves across it; and where it has nothing to follow (a frame of one
//!   colour), it is started at the next frame that has. A frame at which the picture is lost
//!   holds the motion of the frame before it.
//! - **What is written.** Nothing while it goes on: once every frame is measured, the clip's
//!   stabilizer is set whole by `set_stabilize`, as the project has it (or as a clip has one at
//!   first), its Method the one that the motion was measured for and its analysis what was
//!   measured: one step to undo. A job that is stopped, or that fails, writes nothing, as the
//!   stabilizer works out the whole clip at once and has no use for a part of it.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use columbina_model::stabilize::{numbers_of, Analysis, Method, Stabilize};
use columbina_model::{Media, Project, TrackKind};
use columbina_track::{CloudOptions, CloudTracker, Luma, Mat3, Motion, Region, TrackError};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use super::{
    clip_of, end_of, picture_of, reason_of, refused, shown_of, Frames, Shown, COMMIT_EVERY,
};
use crate::rpc::Fault;
use crate::session::{Kind, OwnedGesture, State};
use crate::tracking::analysis_size;

/// How many frames the cloud tracker follows the picture from one frame before it is started
/// anew from the frame that it reached.
const RESTART: usize = 50;

/// The most features of the cloud: more than a window's, as the whole picture is followed.
const FEATURES: usize = 200;

/// What the shift of a motion is written to: a ten-thousandth of a pixel.
const PIXELS: f64 = 1e4;

/// What its zoom, its turn, its aspect and its shear are written to.
const FACTORS: f64 = 1e8;

/// What its perspective, per pixel, is written to.
const PER_PIXEL: f64 = 1e12;

/// What `track.stabilize` takes, as it is written.
pub(super) const TAKES: &str = "{timeline,clip,revision,method?}";

/// `track.stabilize` as it is written: the clip `clip` of `timeline`, the revision that it was
/// asked at, and the Method to measure the motion for (that of the clip's stabilizer, or
/// Perspective where it has none, where it is left out).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    timeline: String,
    clip: String,
    pub(super) revision: u64,
    #[serde(default)]
    method: Option<Method>,
}

/// What a measure of a clip reads and where it writes, fixed when it starts.
pub(super) struct Plan {
    job: String,
    owner: u64,
    kind: Kind,
    timeline: String,
    clip: String,
    media: Media,
    /// The upright frames' analysis size: the frames that the motion is measured on.
    size: [usize; 2],
    /// The first frame of the source that the clip shows, as its keys count frames.
    first: i64,
    /// The last.
    last: i64,
    /// Where each source frame is first shown on the timeline.
    shown: BTreeMap<i64, i64>,
    /// The Method that the motion is measured for.
    method: Method,
}

/// The plan of the measure `job` of the connection `owner` of `kind` that `request` asks for,
/// of `project` as it is; or why there is none: no such clip (`target`), and a clip of sound, a
/// title, a colour, a still or media that the project has not, which have no picture of a
/// source that moves, or a clip that shows no frame of its media (`untrackable`).
pub(super) fn plan(
    project: &Project,
    request: &Request,
    owner: u64,
    kind: Kind,
    job: String,
) -> Result<Plan, Fault> {
    let (timeline, clip) = clip_of(project, &request.timeline, &request.clip)
        .ok_or_else(|| refused("target", "no such clip"))?;
    let unsteady = |why: &str| {
        refused(
            "untrackable",
            &format!("this clip cannot be stabilized: {why}"),
        )
    };
    let of_picture = timeline.tracks.iter().any(|track| {
        track.kind == TrackKind::Video && track.clips.iter().any(|each| each.id == clip.id)
    });
    if !of_picture {
        return Err(unsteady("a clip of sound has no picture"));
    }
    let (media, video) = picture_of(project, clip, &unsteady)?;
    let Shown { map, first, last } = shown_of(project, timeline, clip, (media, video), &unsteady)?;
    if last < first {
        return Err(unsteady("the clip shows no frame of its media"));
    }
    let method = request
        .method
        .or_else(|| clip.stabilize.as_ref().map(|stabilize| stabilize.method))
        .unwrap_or_default();
    Ok(Plan {
        job,
        owner,
        kind,
        timeline: timeline.id.clone(),
        clip: clip.id.clone(),
        media: media.clone(),
        size: analysis_size(video),
        first,
        last,
        shown: map,
        method,
    })
}

/// What `track.stabilize` answers at once of `plan`: the job, the first and the last frame of
/// the source that it measures, the size of the frames that it measures, and the Method.
pub(super) fn answer(plan: &Plan) -> Value {
    json!({"job": plan.job, "first": plan.first, "last": plan.last, "frame_size": plan.size,
        "method": plan.method})
}

/// What a measure has found: for each frame measured, the motion of the picture since the
/// first frame, in pixels of the frames read as the tracker counts them (the middle of the
/// pixel in column `i` and row `j` at `[i, j]`); and at how many frames the picture was lost.
struct Measured {
    path: BTreeMap<i64, Mat3>,
    lost: usize,
}

/// A region of frames of `size` that holds the frame wherever the picture goes within a frame
/// of where it was: the frame, and as much again on each side.
fn whole(size: [usize; 2]) -> Region {
    let (width, height) = extent(size);
    Region::rect(-width, -height, 2.0 * width, 2.0 * height)
}

/// The width and the height of frames of `size`.
#[allow(clippy::cast_precision_loss, reason = "sides of at most 16384")]
fn extent(size: [usize; 2]) -> (f64, f64) {
    (size[0] as f64, size[1] as f64)
}

/// Measure the motion of the whole picture through the frames of `plan` that `read` gives, in
/// the order of the source (see the module); `each` hears of each frame as it is done, with
/// what was found so far. Stop is looked at between frames, and a measure that was stopped
/// gives what it found before.
///
/// # Errors
/// [`TrackError::MissingFrame`] where `read` has no frame, which is so once Stop is set too,
/// and [`TrackError::FrameSize`] where a frame is not of the size of the first.
fn measure(
    plan: &Plan,
    read: &mut dyn FnMut(i64) -> Option<Luma>,
    stop: &AtomicBool,
    each: &mut dyn FnMut(i64, &Measured),
) -> Result<Measured, TrackError> {
    let options = CloudOptions {
        perspective: plan.method == Method::Perspective,
        max_features: FEATURES,
        ..CloudOptions::default()
    };
    let region = whole(plan.size);
    let mut measured = Measured {
        path: BTreeMap::new(),
        lost: 0,
    };
    // The motion of the frame that the tracker started from, and of the frame last measured.
    let (mut base, mut motion) = (Mat3::IDENTITY, Mat3::IDENTITY);
    let mut tracker: Option<CloudTracker> = None;
    let mut since = 0;
    for frame in plan.first..=plan.last {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let luma = read(frame).ok_or(TrackError::MissingFrame(frame))?;
        // The frame that the tracker is started from anew, where it is.
        let anew = if let Some(running) = tracker.as_mut() {
            since += 1;
            // The frame is kept only where the tracker is started anew from it.
            let (stepped, anew) = if since >= RESTART {
                (running.step(&luma)?, Some(luma))
            } else {
                (running.step_owned(luma)?, None)
            };
            if stepped.lost {
                measured.lost += 1;
            } else {
                motion = stepped.motion.mul(&base);
            }
            anew
        } else {
            if frame != plan.first {
                measured.lost += 1;
            }
            Some(luma)
        };
        measured.path.insert(frame, motion);
        if let Some(luma) = anew {
            tracker = CloudTracker::new(&luma, region.clone(), options.clone()).ok();
            base = motion;
            since = 0;
        }
        each(frame, &measured);
    }
    Ok(measured)
}

/// The eight numbers of `motion` as an analysis is written: its shift to a ten-thousandth of a
/// pixel, its zoom, turn, aspect and shear to the eighth decimal, and its perspective to the
/// twelfth, so that a long analysis is not written with seventeen digits a number.
fn written(motion: &Motion) -> [f64; 8] {
    let numbers = numbers_of(motion);
    let to = [
        PIXELS, PIXELS, FACTORS, FACTORS, FACTORS, FACTORS, PER_PIXEL, PER_PIXEL,
    ];
    // Adding 0 makes a zero that is below 0 the zero.
    std::array::from_fn(|at| (numbers[at] * to[at]).round() / to[at] + 0.0)
}

/// The analysis of `measured`, frames of `size` measured for `method`: each motion made one of
/// pixels from the top left corner of the frame, half a pixel from where the tracker counts
/// them, and written as the eight numbers of the motion about the middle of the frame
/// ([`written`]). A motion that turns the picture over, which no camera makes, is taken for
/// the one before it.
fn analysis_of(measured: &Measured, size: [usize; 2], method: Method) -> Analysis {
    let (width, height) = extent(size);
    let middle = [width / 2.0, height / 2.0];
    let (to_edges, to_middles) = (Mat3::translation(0.5, 0.5), Mat3::translation(-0.5, -0.5));
    let mut held = Motion::IDENTITY;
    let path = measured
        .path
        .iter()
        .map(|(&frame, motion)| {
            let moved = to_edges.mul(motion).mul(&to_middles);
            if let Some(found) = Motion::from_matrix(&moved, middle) {
                held = found;
            }
            (frame, written(&held))
        })
        .collect();
    let held_to = |pixels: usize| u32::try_from(pixels).unwrap_or(u32::MAX);
    Analysis::new(method, [held_to(size[0]), held_to(size[1])], path)
}

/// The operation that writes `analysis` into the stabilizer of the clip of `plan`, as `project`
/// has it: its stabilizer set whole, as it is (as a clip has one at first, where it has none),
/// with the Method that the motion was measured for and the analysis.
fn write(project: &Project, plan: &Plan, analysis: &Analysis) -> Result<Value, Fault> {
    let (_, clip) = clip_of(project, &plan.timeline, &plan.clip)
        .ok_or_else(|| refused("target", "no such clip"))?;
    let stabilize = Stabilize {
        method: plan.method,
        analysis: Some(analysis.clone()),
        ..clip.stabilize.clone().unwrap_or_default()
    };
    Ok(
        json!({"op": "set_stabilize", "timeline": plan.timeline, "clip": plan.clip,
        "value": stabilize}),
    )
}

/// `track.progress` of the measure of `plan` that has reached `frame`, done `done` frames of
/// them and lost the picture at `lost`, the project being at `revision`.
fn progress(plan: &Plan, frame: i64, done: i64, lost: usize, revision: u64) -> Value {
    let timeline = plan
        .shown
        .range(..=frame)
        .next_back()
        .or_else(|| plan.shown.range(frame..).next())
        .map(|(_, &at)| at);
    json!({"job": plan.job, "frame": frame, "timeline_frame": timeline, "done": done,
        "total": plan.last - plan.first + 1, "lost": lost, "revision": revision})
}

/// Measure `plan` with `gesture` until it ends or `stop` is set, and write what it measured:
/// what a measure of a clip does on its thread (see the module).
#[allow(
    clippy::too_many_lines,
    reason = "one measure from its frames to its end"
)]
pub(super) fn run(
    state: &Arc<Mutex<State>>,
    plan: &Plan,
    mut gesture: OwnedGesture,
    stop: &Arc<AtomicBool>,
) {
    let opened = Frames::open(&plan.media, plan.size, plan.first);
    let unreadable = opened.as_ref().err().map(|fault| fault.message.clone());
    let mut frames = opened.ok();
    let mut done = 0_i64;
    let mut lost = 0_usize;
    let mut reached = plan.first;
    let mut stopped_by: Option<String> = None;
    let mut last_told = Instant::now();
    let outcome = match frames.as_mut() {
        None => Err(TrackError::MissingFrame(plan.first)),
        Some(frames) => {
            let mut read = |f: i64| {
                if stop.load(Ordering::Relaxed) {
                    return None;
                }
                frames.get(f)
            };
            measure(plan, &mut read, stop, &mut |frame, measured| {
                done += 1;
                reached = frame;
                lost = measured.lost;
                if last_told.elapsed() < COMMIT_EVERY {
                    return;
                }
                last_told = Instant::now();
                let locked = state.blocking_lock();
                // Nothing is written while it goes on: whether the project is still as it was
                // asked of, and the connection still there, is looked at as often as a track
                // writes.
                let checked = if locked.peers.contains_key(&plan.owner) {
                    locked.check_owned_gesture(plan.owner, &gesture)
                } else {
                    Err(refused(
                        "disconnected",
                        "the connection that asked has gone",
                    ))
                };
                match checked {
                    Ok(()) => {
                        let revision = locked.history.revision();
                        let told = progress(plan, frame, done, measured.lost, revision);
                        locked.track_tell(plan.owner, "track.progress", &told);
                    }
                    Err(fault) => {
                        stopped_by = Some(reason_of(&fault));
                        stop.store(true, Ordering::Relaxed);
                    }
                }
            })
        }
    };
    let mut locked = state.blocking_lock();
    let stopped = stop.load(Ordering::Relaxed);
    let mut written = None::<u64>;
    let complete = !stopped && stopped_by.is_none();
    if let (Ok(measured), true) = (&outcome, complete) {
        if locked.peers.contains_key(&plan.owner) {
            let analysis = analysis_of(measured, plan.size, plan.method);
            let result = locked.track_write(plan.owner, plan.kind, &mut gesture, |project| {
                write(project, plan, &analysis)
            });
            match result {
                Ok((revision, _)) => written = Some(revision),
                Err(fault) => stopped_by = Some(reason_of(&fault)),
            }
        } else {
            stopped_by = Some("disconnected".into());
        }
    }
    let user_stop = stopped && stopped_by.is_none();
    let report = json!({"job": plan.job, "frames": done, "lost": lost, "frame": reached,
        "revision": written.unwrap_or(locked.history.revision()),
        "analysed": written.map(|_| [plan.first, plan.last]), "method": plan.method});
    let state_value = end_of(
        outcome.as_ref().err(),
        stopped_by.as_deref(),
        user_stop,
        unreadable.as_deref(),
        &report,
    );
    locked.track_end(plan.owner, &plan.job, gesture, &state_value);
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::float_cmp,
        reason = "numbers that are written rounded, and compared as they were written"
    )]

    use columbina_model::retime::{Retime, SignedSpeed, SpeedPoint};
    use columbina_model::Source;

    use super::*;

    fn reason(fault: &Fault) -> &str {
        fault.data.as_ref().unwrap()["reason"].as_str().unwrap()
    }

    fn asked(clip: &str, method: Option<&str>) -> Request {
        let mut request = json!({"timeline": "t1", "clip": clip, "revision": 0});
        if let Some(method) = method {
            request["method"] = json!(method);
        }
        serde_json::from_value(request).unwrap()
    }

    fn planned(project: &Project, request: &Request) -> Result<Plan, Fault> {
        plan(project, request, 7, Kind::Agent, "track-1".into())
    }

    #[test]
    fn a_measure_goes_over_every_frame_that_the_clip_shows_for_the_method_asked_or_its_own() {
        let mut project = Project::sample();
        let plan = planned(&project, &asked("c1", None)).unwrap();
        assert_eq!((plan.first, plan.last, plan.size), (0, 99, [1280, 720]));
        assert_eq!(plan.method, Method::Perspective);
        assert_eq!(
            answer(&plan),
            json!({"job": "track-1", "first": 0, "last": 99, "frame_size": [1280, 720],
                "method": "perspective"})
        );
        // The Method of the clip's stabilizer, or the one asked for.
        project.timelines[0].tracks[0].clips[0].stabilize = Some(Stabilize {
            method: Method::Translation,
            ..Stabilize::default()
        });
        assert_eq!(
            planned(&project, &asked("c1", None)).unwrap().method,
            Method::Translation
        );
        let similar = planned(&project, &asked("c1", Some("similarity"))).unwrap();
        assert_eq!(similar.method, Method::Similarity);
        // A clip played backward along speed points goes over the same frames of its source,
        // forward.
        project.timelines[0].tracks[0].clips[0].set_mapping(Some(Retime {
            points: vec![SpeedPoint {
                frame: 0,
                speed: SignedSpeed { num: -1, den: 1 },
            }],
        }));
        let backward = planned(&project, &asked("c1", None)).unwrap();
        assert_eq!((backward.first, backward.last), (0, 99));
        assert_eq!(backward.shown[&99], 0);
    }

    #[test]
    fn a_measure_of_what_has_no_moving_picture_is_refused() {
        let project = Project::sample();
        let mut colour = project.clone();
        colour.timelines[0].tracks[0].clips[0].source = Source::Color {
            color: columbina_model::Rgba([0, 0, 0, 255]),
        };
        let mut still = project.clone();
        still.media[0].facts.video.as_mut().unwrap().still = true;
        for (project, clip, wanted) in [
            (&project, "c404", "target"),
            (&project, "c2", "untrackable"),
            (&colour, "c1", "untrackable"),
            (&still, "c1", "untrackable"),
        ] {
            let fault = planned(project, &asked(clip, None))
                .err()
                .unwrap_or_else(|| panic!("{clip} is measured"));
            assert_eq!(reason(&fault), wanted, "{clip}");
        }
        // What it takes, and nothing else.
        let wrong = json!({"timeline": "t1", "clip": "c1", "revision": 0, "frame": 3});
        assert!(serde_json::from_value::<Request>(wrong).is_err());
        let affine = json!({"timeline": "t1", "clip": "c1", "revision": 0, "method": "affine"});
        assert!(serde_json::from_value::<Request>(affine).is_err());
    }

    /// Frames of 160 × 120.
    const SIZE: [usize; 2] = [160, 120];

    /// How far the picture of frame `k` is moved: across and down, in pixels, by less than
    /// two pixels from a frame to the next.
    fn shake(k: i64) -> [f64; 2] {
        let t = f64::from(i32::try_from(k).unwrap());
        [2.0 * (0.5 * t).sin() + 0.3 * t, 1.5 * (0.4 * t).cos() - 1.5]
    }

    /// A frame of a textured picture moved by `by`, or of one grey where it is none.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "a sample from 0.1 to 0.9, as the tracker takes it"
    )]
    fn frame(by: Option<[f64; 2]>) -> Luma {
        let mut data = Vec::with_capacity(SIZE[0] * SIZE[1]);
        for y in 0..120_u32 {
            for x in 0..160_u32 {
                let value = match by {
                    None => 0.5,
                    Some([across, down]) => {
                        let (u, v) = (f64::from(x) - across, f64::from(y) - down);
                        0.5 + 0.15 * (0.35 * u + 0.2 * v).sin()
                            + 0.15 * (-0.25 * u + 0.4 * v).sin()
                            + 0.1 * (0.13 * u - 0.07 * v + 1.0).sin()
                    }
                };
                data.push(value as f32);
            }
        }
        Luma::new(SIZE[0], SIZE[1], data).unwrap()
    }

    /// A plan of the frames `0` to `last` of frames of [`SIZE`], measured for `method`.
    fn shaken(last: i64, method: Method) -> Plan {
        Plan {
            job: "track-1".into(),
            owner: 7,
            kind: Kind::Agent,
            timeline: "t1".into(),
            clip: "c1".into(),
            media: Project::sample().media[0].clone(),
            size: SIZE,
            first: 0,
            last,
            shown: (0..=last).map(|frame| (frame, frame)).collect(),
            method,
        }
    }

    /// Where the motion of `measured` at `frame` puts the point `[x, y]`.
    fn put(measured: &Measured, frame: i64, point: [f64; 2]) -> [f64; 2] {
        measured.path[&frame].apply(point)
    }

    #[test]
    fn the_whole_picture_is_followed_through_the_frames_and_the_tracker_started_anew() {
        let last = i64::try_from(RESTART).unwrap() + 20;
        let mut heard = Vec::new();
        let measured = measure(
            &shaken(last, Method::Similarity),
            &mut |k| Some(frame(Some(shake(k)))),
            &AtomicBool::new(false),
            &mut |k, _| heard.push(k),
        )
        .unwrap();
        assert_eq!(heard, (0..=last).collect::<Vec<_>>());
        assert_eq!(measured.lost, 0);
        assert_eq!(
            measured.path.keys().copied().collect::<Vec<_>>(),
            (0..=last).collect::<Vec<_>>()
        );
        // Each point of the first frame is put where the picture moved it, past the frame at
        // which the tracker was started anew too.
        for k in 0..=last {
            let [across, down] = shake(k);
            let [x0, y0] = shake(0);
            for point in [[30.0, 20.0], [80.0, 60.0], [140.0, 100.0]] {
                let [x, y] = put(&measured, k, point);
                let off = (x - point[0] - (across - x0)).hypot(y - point[1] - (down - y0));
                assert!(off < 0.2, "frame {k}: {off} pixels off");
            }
        }
    }

    #[test]
    fn frames_of_one_colour_hold_the_motion_until_there_is_a_picture_to_follow() {
        let measured = measure(
            &shaken(12, Method::Perspective),
            &mut |k| Some(frame((k >= 3).then(|| shake(k)))),
            &AtomicBool::new(false),
            &mut |_, _| {},
        )
        .unwrap();
        // Frames 1 and 2 have nothing to follow, and frame 3 nothing to be followed from.
        assert_eq!(measured.lost, 3);
        for k in 0..=3 {
            assert_eq!(measured.path[&k], Mat3::IDENTITY, "frame {k}");
        }
        let [x3, y3] = shake(3);
        for k in 4..=12 {
            let [across, down] = shake(k);
            let [x, y] = put(&measured, k, [80.0, 60.0]);
            let off = (x - 80.0 - (across - x3)).hypot(y - 60.0 - (down - y3));
            assert!(off < 0.2, "frame {k}: {off} pixels off");
        }
    }

    #[test]
    fn a_measure_ends_between_frames_where_it_is_stopped_or_a_frame_is_missing() {
        let stop = AtomicBool::new(false);
        let measured = measure(
            &shaken(30, Method::Translation),
            &mut |k| Some(frame(Some(shake(k)))),
            &stop,
            &mut |k, _| {
                if k == 4 {
                    stop.store(true, Ordering::Relaxed);
                }
            },
        )
        .unwrap();
        assert_eq!(
            measured.path.keys().copied().collect::<Vec<_>>(),
            [0, 1, 2, 3, 4]
        );
        let error = measure(
            &shaken(30, Method::Translation),
            &mut |k| (k < 6).then(|| frame(Some(shake(k)))),
            &AtomicBool::new(false),
            &mut |_, _| {},
        )
        .err();
        assert_eq!(error, Some(TrackError::MissingFrame(6)));
    }

    #[test]
    fn an_analysis_is_of_pixels_from_the_corner_about_the_middle_and_written_rounded() {
        let mut measured = Measured {
            path: BTreeMap::new(),
            lost: 0,
        };
        // A shift, and a turn of a quarter about the middle of the first pixel as the tracker
        // counts it, which is half a pixel from the corner.
        measured
            .path
            .insert(10, Mat3::translation(1.234_567_89, -2.0));
        let turn = Motion {
            rotate: std::f64::consts::FRAC_PI_2,
            ..Motion::IDENTITY
        };
        measured.path.insert(11, turn.matrix([0.0, 0.0]));
        let analysis = analysis_of(&measured, SIZE, Method::Similarity);
        assert_eq!(analysis.size(), [160, 120]);
        assert_eq!(analysis.method(), Method::Similarity);
        assert_eq!(
            analysis.path()[&10],
            [1.2346, -2.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0]
        );
        // The turn is about the point half a pixel from the corner: about the middle of the
        // frame, it is the same turn and a shift.
        let about = columbina_model::stabilize::motion_of(&analysis.path()[&11]);
        let moved = about.matrix([80.0, 60.0]).apply([0.5, 0.5]);
        assert!(
            (moved[0] - 0.5).abs() < 1e-3 && (moved[1] - 0.5).abs() < 1e-3,
            "{moved:?}"
        );
        assert!((about.rotate - std::f64::consts::FRAC_PI_2).abs() < 1e-7);
        // No zero is written below 0.
        assert!(analysis
            .path()
            .values()
            .flatten()
            .all(|number| *number != 0.0 || number.is_sign_positive()));
    }
}
