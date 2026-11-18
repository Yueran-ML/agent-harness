//! `track.start` with `planar` (T19): the region of a Planar Tracker of a composition (Reference
//! Manual 20, Chapter 83, pp. 1760–1766, and the node, pp. 2749–2758) followed through the frames
//! of its clip as a plane, and the homography of each frame written into its path.
//!
//! A track of a Planar Tracker goes as a track of a Tracker does (`fusion`): it is checked
//! against the project as it is, a native gesture of the connection that asked is reserved, it
//! is answered at once and runs on a thread of its own; what it found so far is written as the
//! next change of that gesture as often as a window's track is, `track.progress` told each time
//! and `track.state` at its end; and it is ended between frames by `track.stop`, a change of the
//! project by anyone else, Undo or Redo, the connection going, and a frame that cannot be
//! decoded. What differs:
//!
//! - **What is followed.** The region of the node where it is at the playhead: as it was drawn
//!   at the reference frame, moved by the motion of the path there
//!   (`columbina_model::fusion::PlanarTracker::motion_at`). It is followed by the cloud tracker
//!   (`columbina_track::CloudTracker`), its features found within it and the motion of the plane
//!   fitted to those that agree, as a homography with the motion `perspective` and as a shift, a
//!   turn and a size with the others; of the motion, the parts that the node's `motion` names
//!   are kept, about the middle of the region as it was drawn.
//! - **What is written.** Each frame at which the plane was found, and the frame that it started
//!   from, become frames of the path: the motion from the reference frame put after the
//!   homography that the path has there, so that a path tracked from the reference frame has the
//!   motion itself. Those frames are overwritten and the others kept, by a patch of the path
//!   (`set_fusion_node` with `params: {"path": {...}}`) of the frames that it has not as they
//!   are found; the inverse of each write gives the frames that it put `null`, which takes them
//!   away. A frame at which the plane is lost is not written, and the track goes on: the path is
//!   read across it.
//! - **Where.** A place of the picture, `[x, y]` in shares from its lower left, lies at
//!   `x · W − ½` and `(1 − y) · H − ½` of a frame `W × H` as the tracker counts it (the middle of
//!   the pixel in column `i` and row `j` at `[i, j]`), and a homography of places is one of those
//!   pixels through that map and its inverse ([`to_pixels`], [`to_shares`]).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use columbina_model::fusion::{
    homography_of, Composition, Homography, NodeKind, Place, PlanarTracker, LEAST_REGION_POINTS,
};
use columbina_model::{Media, Project};
use columbina_track::{
    CloudOptions, CloudTracker, Components, Direction, Luma, Mat3, Motion, Point, Region,
    TrackError,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use super::{
    clip_of, commit_interval, end_of, on_source, picture_of, reason_of, refused, shown_frames,
    Frames, Shown, COMMIT_EVERY,
};
use crate::rpc::{self, Fault};
use crate::session::{Kind, OwnedGesture, State};
use crate::tracking::analysis_size;

/// What the numbers of a homography of a path are written to: the ninth decimal, a millionth of
/// a pixel of a picture a thousand pixels wide, so that a long path is not sent with seventeen
/// digits a number at each write.
const DECIMALS: f64 = 1e9;

/// What `planar` names: the Planar Tracker `node` of the composition of the clip `clip` of
/// `timeline`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Aim {
    timeline: String,
    clip: String,
    node: String,
}

/// A track of a Planar Tracker, as `track.start` asks for it, but for the revision that it was
/// asked at.
pub(super) struct Start {
    /// What is tracked.
    pub(super) aim: Aim,
    /// The playhead, as the keys of the clip count frames.
    pub(super) frame: i64,
    /// Which way, on the timeline.
    pub(super) direction: Direction,
}

/// What a track of a Planar Tracker reads and where it writes, fixed when it starts.
pub(super) struct Plan {
    job: String,
    owner: u64,
    kind: Kind,
    timeline: String,
    clip: String,
    node: String,
    media: Media,
    /// The upright frames' analysis size: the frames that the plane is followed on.
    pub(super) size: [usize; 2],
    pub(super) first: i64,
    pub(super) last: i64,
    /// The playhead, as the keys of the clip count frames: where each way of the track starts.
    start: i64,
    /// Which way, in the source.
    pub(super) direction: Direction,
    /// Where each source frame is first shown on the timeline.
    shown: BTreeMap<i64, i64>,
    /// The region where it is at `start`, in pixels of the frames read as the tracker counts
    /// them.
    region: Region,
    /// The motion of the plane from the reference frame to `start`, of those pixels: what the
    /// motion that the tracker finds from `start` on is put after.
    base: Mat3,
    /// The homography of the path at the reference frame, of places: what each motion found is
    /// put after to be written into the path.
    rest: Mat3,
    /// The homography of the path at `start`, as it is written there.
    at_start: Homography,
    /// The middle of the region as it was drawn, in pixels: what the parts of a motion that are
    /// kept are about.
    anchor: Point,
    /// Whether the motion is fitted as a homography.
    perspective: bool,
    /// The parts of the motion that are kept.
    components: Components,
}

/// The plan of the track `job` of the connection `owner` of `kind` that `request` asks for, of
/// `project` as it is; or why there is none (see `track.start` in `PROTOCOL.md`).
pub(super) fn plan(
    project: &Project,
    request: &Start,
    owner: u64,
    kind: Kind,
    job: String,
) -> Result<Plan, Fault> {
    let aim = &request.aim;
    let (timeline, clip) = clip_of(project, &aim.timeline, &aim.clip)
        .ok_or_else(|| refused("target", "no such clip"))?;
    let planar = planar_of(clip.fusion.as_ref(), &aim.node)?;
    let untrackable = |why: &str| {
        refused(
            "untrackable",
            &format!("this Planar Tracker cannot be tracked: {why}"),
        )
    };
    let (media, video) = picture_of(project, clip, &untrackable)?;
    let Shown { map, first, last } = shown_frames(
        project,
        timeline,
        clip,
        (media, video),
        request.frame,
        &untrackable,
    )?;
    let size = analysis_size(video);
    #[allow(clippy::cast_precision_loss, reason = "frames are within 2^53")]
    let (start, reference) = (request.frame as f64, planar.reference as f64);
    let lost = || {
        refused(
            "region",
            "the plane of the Planar Tracker cannot be put back where it was at the reference \
             frame",
        )
    };
    let motion = planar.motion_at(start).ok_or_else(lost)?;
    let base = to_pixels(&motion, size);
    let drawn = region_of(&planar.region, size);
    let here: Vec<Point> = drawn.iter().map(|&at| base.apply(at)).collect();
    check_region(&here, size)?;
    let anchor = Region::Polygon(drawn).centroid().ok_or_else(lost)?;
    let at_start = as_written(&planar.homography(start)).ok_or_else(lost)?;
    Ok(Plan {
        job,
        owner,
        kind,
        timeline: aim.timeline.clone(),
        clip: aim.clip.clone(),
        node: aim.node.clone(),
        media: media.clone(),
        size,
        first,
        last,
        start: request.frame,
        direction: on_source(&map, clip, request.frame, request.direction),
        shown: map,
        region: Region::Polygon(here),
        base,
        rest: planar.homography(reference),
        at_start,
        anchor,
        perspective: planar.motion.perspective(),
        components: planar.motion.components(),
    })
}

/// The Planar Tracker `node` of `composition`; or why there is none: no composition or no such
/// node (`target`), or a node of another kind (`wrong_node`, with its `kind`).
fn planar_of<'a>(
    composition: Option<&'a Composition>,
    node: &str,
) -> Result<&'a PlanarTracker, Fault> {
    let found = composition
        .and_then(|composition| composition.node(node))
        .ok_or_else(|| refused("target", "the clip's composition has no such node"))?;
    let NodeKind::PlanarTracker(planar) = &found.kind else {
        let data = json!({"reason": "wrong_node", "node": node, "kind": found.kind.kind()});
        return Err(
            Fault::new(rpc::REFUSED, format!("{node} is not a planar tracker")).with_data(data),
        );
    };
    Ok(planar)
}

/// The width and the height of frames of `size`.
#[allow(clippy::cast_precision_loss, reason = "sides of at most 16384")]
fn extent(size: [usize; 2]) -> (f64, f64) {
    (size[0] as f64, size[1] as f64)
}

/// The map from places of the picture, in shares from its lower left corner, `y` up, to pixels
/// of frames of `size` as the tracker counts them, `y` down (see the module).
fn shares_to_pixels(size: [usize; 2]) -> Mat3 {
    let (width, height) = extent(size);
    Mat3([
        [width, 0.0, -0.5],
        [0.0, -height, height - 0.5],
        [0.0, 0.0, 1.0],
    ])
}

/// The map from pixels of frames of `size` as the tracker counts them to places of the picture:
/// [`shares_to_pixels`] the other way.
fn pixels_to_shares(size: [usize; 2]) -> Mat3 {
    let (width, height) = extent(size);
    Mat3([
        [1.0 / width, 0.0, 0.5 / width],
        [0.0, -1.0 / height, (height - 0.5) / height],
        [0.0, 0.0, 1.0],
    ])
}

/// `homography`, of places of the picture, as the same map of pixels of frames of `size` as the
/// tracker counts them, its corner 1.
fn to_pixels(homography: &Mat3, size: [usize; 2]) -> Mat3 {
    shares_to_pixels(size)
        .mul(homography)
        .mul(&pixels_to_shares(size))
        .normalized()
}

/// `homography`, of pixels of frames of `size` as the tracker counts them, as the same map of
/// places of the picture, its corner 1: [`to_pixels`] the other way.
fn to_shares(homography: &Mat3, size: [usize; 2]) -> Mat3 {
    pixels_to_shares(size)
        .mul(homography)
        .mul(&shares_to_pixels(size))
        .normalized()
}

/// The places of `region` as pixels of frames of `size` as the tracker counts them.
fn region_of(region: &[Place], size: [usize; 2]) -> Vec<Point> {
    let map = shares_to_pixels(size);
    region.iter().map(|at| map.apply([at.x, at.y])).collect()
}

/// Twice the area of the polygon `points`, by the shoelace formula: of one sign or the other by
/// the way that it goes round.
fn twice_the_area(points: &[Point]) -> f64 {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
        .sum()
}

/// Check that the region where it is when the track starts, `points` in pixels of frames of
/// `size`, is one that can be followed: of three places at least, all of finite numbers, a pixel
/// in area at least, and reaching within the frame; or refuse it (`region`).
fn check_region(points: &[Point], size: [usize; 2]) -> Result<(), Fault> {
    let refusal = || {
        refused(
            "region",
            "the region of the Planar Tracker has no area within the picture where the track \
             starts",
        )
    };
    if points.len() < LEAST_REGION_POINTS || !points.iter().flatten().all(|at| at.is_finite()) {
        return Err(refusal());
    }
    let (width, height) = extent(size);
    let low = points.iter().fold([f64::INFINITY; 2], |least, at| {
        [least[0].min(at[0]), least[1].min(at[1])]
    });
    let high = points.iter().fold([f64::NEG_INFINITY; 2], |most, at| {
        [most[0].max(at[0]), most[1].max(at[1])]
    });
    let meets = high[0] >= 0.0 && high[1] >= 0.0 && low[0] <= width - 1.0 && low[1] <= height - 1.0;
    if !meets || twice_the_area(points).abs() < 2.0 {
        return Err(refusal());
    }
    Ok(())
}

/// The parts of `motion`, of pixels, that `plan` keeps, about its anchor: the whole of it with
/// Perspective. None where the parts cannot be had: the motion turns the plane over.
fn kept(plan: &Plan, motion: &Mat3) -> Option<Mat3> {
    if plan.components == Components::ALL {
        return Some(*motion);
    }
    let parts = Motion::from_matrix(motion, plan.anchor)?;
    Some(parts.masked(plan.components).matrix(plan.anchor))
}

/// `homography`, of places, as a path writes it: its numbers to the ninth decimal
/// ([`DECIMALS`]). None where its corner is 0 or a number is not finite.
fn as_written(homography: &Mat3) -> Option<Homography> {
    // Adding 0 makes a zero that is below 0 the zero.
    homography_of(homography)
        .map(|numbers| numbers.map(|number| (number * DECIMALS).round() / DECIMALS + 0.0))
}

/// What the path of `plan` has at a frame at which the tracker found the plane moved by `step`
/// since the frame that it started from, of pixels: the motion from the reference frame (`step`
/// after the motion at the start), the parts of it that are kept, made one of places and put
/// after the homography at the reference frame, as it is written. None where the parts cannot
/// be had, or it is not of finite numbers.
fn sample_of(plan: &Plan, step: &Mat3) -> Option<Homography> {
    let motion = kept(plan, &step.mul(&plan.base))?;
    as_written(&to_shares(&motion, plan.size).mul(&plan.rest))
}

/// What a track of a Planar Tracker has found.
#[derive(Default)]
struct Found {
    /// The homographies found, by the frame, as they are written.
    path: BTreeMap<i64, Homography>,
    /// At how many frames the plane was found, the frame that it started from not counted.
    frames: usize,
    /// At how many frames it was lost.
    lost: usize,
    /// Whether it was found at a frame other than the one it started from.
    stepped: bool,
    /// How many homographies were put into the path, so that what was written is told from
    /// what is not.
    changes: u64,
}

impl Found {
    /// A way of the track that starts at `start`, where the path has `at_start`: written there.
    fn begin(&mut self, start: i64, at_start: Homography) {
        self.path.insert(start, at_start);
        self.changes += 1;
    }

    /// The plane at `frame`: what the path has there, or none where it was lost.
    fn found(&mut self, frame: i64, sample: Option<Homography>) {
        match sample {
            Some(sample) => {
                self.path.insert(frame, sample);
                self.frames += 1;
                self.stepped = true;
                self.changes += 1;
            }
            None => self.lost += 1,
        }
    }

    /// The first and the last frame written into the path, or none.
    fn range(&self) -> Option<(i64, i64)> {
        let (&low, _) = self.path.first_key_value()?;
        let (&high, _) = self.path.last_key_value()?;
        Some((low, high))
    }
}

/// Follow the plane of `plan` through the frames that `read` gives, way after way as its
/// direction says, into `found`; `each_frame` hears of each frame as it is done, with what was
/// found so far. Stop is looked at between frames.
///
/// # Errors
/// [`TrackError::MissingFrame`] where `read` has no frame, which is so once Stop is set too; and
/// those of [`CloudTracker::new`], where the region holds nothing that can be followed.
fn follow(
    plan: &Plan,
    found: &mut Found,
    read: &mut dyn FnMut(i64) -> Option<Luma>,
    stop: &AtomicBool,
    each_frame: &mut dyn FnMut(i64, &Found),
) -> Result<(), TrackError> {
    let ways: &[(i64, Option<usize>)] = match plan.direction {
        Direction::Forward => &[(1, None)],
        Direction::Reverse => &[(-1, None)],
        Direction::ForwardAndBack => &[(1, None), (-1, None)],
        Direction::OneForward => &[(1, Some(1))],
        Direction::OneReverse => &[(-1, Some(1))],
    };
    let options = CloudOptions {
        perspective: plan.perspective,
        ..CloudOptions::default()
    };
    for &(way, most) in ways {
        let end = if way > 0 { plan.last } else { plan.first };
        let at_start = read(plan.start).ok_or(TrackError::MissingFrame(plan.start))?;
        let mut tracker = CloudTracker::new(&at_start, plan.region.clone(), options.clone())?;
        found.begin(plan.start, plan.at_start);
        let mut frame = plan.start;
        let mut done = 0;
        while frame != end && most.is_none_or(|m| done < m) {
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            frame += way;
            let luma = read(frame).ok_or(TrackError::MissingFrame(frame))?;
            let stepped = tracker.step_owned(luma)?;
            done += 1;
            let sample = (!stepped.lost)
                .then(|| sample_of(plan, &stepped.motion))
                .flatten();
            found.found(frame, sample);
            each_frame(frame, found);
        }
    }
    Ok(())
}

/// The operation that writes the frames of `found` that the path of the node of `plan`, as
/// `project` has it, has not as they were found: a patch of its path, the other frames kept.
/// None where it has each of them so already.
fn write(project: &Project, plan: &Plan, found: &Found) -> Result<Option<Value>, Fault> {
    let (_, clip) = clip_of(project, &plan.timeline, &plan.clip)
        .ok_or_else(|| refused("target", "no such clip"))?;
    let planar = planar_of(clip.fusion.as_ref(), &plan.node)?;
    let patch: serde_json::Map<String, Value> = found
        .path
        .iter()
        .filter(|(frame, sample)| planar.path.get(frame) != Some(sample))
        .map(|(frame, sample)| (frame.to_string(), json!(sample)))
        .collect();
    if patch.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        json!({"op": "set_fusion_node", "timeline": plan.timeline, "clip": plan.clip,
            "id": plan.node, "params": {"path": patch}}),
    ))
}

/// `track.progress` of the track of `plan` that has reached `frame`, done `done` of the `total`
/// frames that it goes over and lost the plane at `lost`, the project being at `revision`.
fn progress(plan: &Plan, frame: i64, done: (i64, i64), lost: usize, revision: u64) -> Value {
    let timeline = plan
        .shown
        .range(..=frame)
        .next_back()
        .or_else(|| plan.shown.range(frame..).next())
        .map(|(_, &at)| at);
    json!({"job": plan.job, "frame": frame, "timeline_frame": timeline, "done": done.0,
        "total": done.1, "lost": lost, "revision": revision})
}

/// What `track.start` with `planar` answers at once of `plan`: the job, the first and the last
/// frame of the source that it may go over, the size of the frames that it reads, and the way
/// in the source.
pub(super) fn answer(plan: &Plan) -> Value {
    json!({"job": plan.job, "first": plan.first, "last": plan.last, "frame_size": plan.size,
        "direction": plan.direction})
}

/// Write what `found` has into the path of the node of `plan` as the next change of `gesture`,
/// where the path has it not as it was found yet, and tell `track.progress`. Gives the revision
/// and the size of the operation as it was sent, 0 where nothing was written.
fn tell(
    state: &mut State,
    plan: &Plan,
    gesture: &mut OwnedGesture,
    found: &Found,
    (reached, done, total): (i64, i64, i64),
) -> Result<(u64, usize), Fault> {
    let (revision, bytes) = if let Some(op) = write(state.history.project(), plan, found)? {
        state.track_write(plan.owner, plan.kind, gesture, |_| Ok(op))?
    } else {
        state.check_owned_gesture(plan.owner, gesture)?;
        (state.history.revision(), 0)
    };
    let told = progress(plan, reached, (done, total), found.lost, revision);
    state.track_tell(plan.owner, "track.progress", &told);
    Ok((revision, bytes))
}

/// Track `plan` with `gesture` until it ends or `stop` is set: what a track of a Planar Tracker
/// does on its thread (see the module).
#[allow(
    clippy::too_many_lines,
    reason = "one track from its frames to its end"
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
    let total = match plan.direction {
        Direction::Forward => plan.last - plan.start,
        Direction::Reverse => plan.start - plan.first,
        Direction::ForwardAndBack => plan.last - plan.first,
        Direction::OneForward | Direction::OneReverse => 1,
    }
    .max(0);
    let mut found = Found::default();
    let mut done = 0_i64;
    let mut reached = plan.start;
    let mut written = None::<u64>;
    let mut stopped_by: Option<String> = None;
    let mut last_commit = Instant::now();
    let mut interval = COMMIT_EVERY;
    // The changes of `found` that were written.
    let mut committed = 0_u64;
    let outcome = match frames.as_mut() {
        None => Err(TrackError::MissingFrame(plan.start)),
        Some(frames) => {
            let mut read = |f: i64| {
                if stop.load(Ordering::Relaxed) {
                    return None;
                }
                frames.get(f)
            };
            follow(plan, &mut found, &mut read, stop, &mut |frame, found| {
                done += 1;
                reached = frame;
                if !found.stepped || last_commit.elapsed() < interval {
                    return;
                }
                last_commit = Instant::now();
                let mut locked = state.blocking_lock();
                let held = Instant::now();
                let result = if locked.peers.contains_key(&plan.owner) {
                    tell(&mut locked, plan, &mut gesture, found, (frame, done, total))
                } else {
                    Err(refused(
                        "disconnected",
                        "the connection that asked has gone",
                    ))
                };
                match result {
                    Ok((revision, bytes)) => {
                        written = Some(revision);
                        committed = found.changes;
                        interval = commit_interval(bytes, held.elapsed());
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
    // The rest of the track, unless the project was changed under it or nothing was found.
    if stopped_by.is_none() && found.stepped && found.changes != committed {
        match tell(
            &mut locked,
            plan,
            &mut gesture,
            &found,
            (reached, done, total),
        ) {
            Ok((revision, _)) => written = Some(revision),
            Err(fault) => stopped_by = Some(reason_of(&fault)),
        }
    }
    let user_stop = stop.load(Ordering::Relaxed) && stopped_by.is_none();
    let report = json!({"job": plan.job, "frames": done, "lost": found.lost, "frame": reached,
        "revision": written.unwrap_or(locked.history.revision()), "tracked": found.range()});
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
        reason = "places of pixels that floats hold whole, and homographies as they are written"
    )]

    use columbina_model::fusion::{
        matrix_of, Input, Kind as FusionKind, Link, Node, PlanarMotion, STILL,
    };

    use super::*;

    /// A homography of a little perspective, a turn and a shift, of places.
    const TILTED: Homography = [1.02, -0.03, 0.05, 0.02, 0.97, -0.04, 0.08, -0.06];

    /// A shift of `x` across and `y` up, in shares, as a path writes it.
    fn shifted(x: f64, y: f64) -> Homography {
        [1.0, 0.0, x, 0.0, 1.0, y, 0.0, 0.0]
    }

    /// Whether two points are within a billionth of a pixel of each other.
    fn near(one: Point, other: Point) -> bool {
        (one[0] - other[0]).hypot(one[1] - other[1]) < 1e-9
    }

    /// The sample project with a composition on its clip `c1`: a `MediaIn` into the Planar
    /// Tracker `plane` into the `MediaOut`, the Planar Tracker as `planar` is.
    fn project(planar: PlanarTracker) -> Project {
        let mut project = Project::sample();
        let mut node = Node::new("plane", FusionKind::PlanarTracker);
        node.kind = NodeKind::PlanarTracker(planar);
        project.timelines[0].tracks[0].clips[0].fusion = Some(Composition {
            nodes: vec![
                Node::new("in", FusionKind::MediaIn),
                node,
                Node::new("out", FusionKind::MediaOut),
            ],
            links: vec![
                Link::new("in", "plane", Input::Input),
                Link::new("plane", "out", Input::Input),
            ],
        });
        project
    }

    /// A track forward from frame `frame` of the node `node` of `c1`.
    fn aimed(node: &str, frame: i64) -> Start {
        Start {
            aim: Aim {
                timeline: "t1".into(),
                clip: "c1".into(),
                node: node.into(),
            },
            frame,
            direction: Direction::Forward,
        }
    }

    fn planned(project: &Project, start: &Start) -> Result<Plan, Fault> {
        plan(project, start, 7, Kind::Agent, "track-1".into())
    }

    fn reason(fault: &Fault) -> &str {
        fault.data.as_ref().unwrap()["reason"].as_str().unwrap()
    }

    #[test]
    fn a_homography_of_places_is_one_of_the_pixels_that_the_tracker_counts_and_back() {
        let size = [1280, 720];
        // A place is half a pixel from where the tracker counts it, and y goes down.
        let map = shares_to_pixels(size);
        assert_eq!(map.apply([0.5, 0.5]), [639.5, 359.5]);
        assert_eq!(map.apply([0.0, 1.0]), [-0.5, -0.5]);
        assert_eq!(map.apply([1.0, 0.0]), [1279.5, 719.5]);
        for (i, j) in [(0, 0), (1279, 719), (17, 400)] {
            let place = [
                (f64::from(i) + 0.5) / 1280.0,
                1.0 - (f64::from(j) + 0.5) / 720.0,
            ];
            assert!(
                near(map.apply(place), [f64::from(i), f64::from(j)]),
                "{i} {j}"
            );
            assert!(near(pixels_to_shares(size).apply(map.apply(place)), place));
        }
        // A homography of places and the same of pixels: each moves a place as the other moves
        // its pixel.
        let tilted = matrix_of(&TILTED);
        let pixels = to_pixels(&tilted, size);
        for place in [[0.1, 0.2], [0.5, 0.5], [0.9, 0.7], [-0.2, 1.3]] {
            let one = map.apply(tilted.apply(place));
            let other = pixels.apply(map.apply(place));
            assert!(
                (one[0] - other[0]).hypot(one[1] - other[1]) < 1e-7,
                "{place:?}"
            );
        }
        // And back, its corner 1.
        let back = to_shares(&pixels, size);
        for (row, wanted) in back.0.iter().zip(tilted.0) {
            for (got, number) in row.iter().zip(wanted) {
                assert!((got - number).abs() < 1e-10, "{back:?}");
            }
        }
        // The identity is the identity, but for a rounding.
        let still = to_pixels(&Mat3::IDENTITY, size);
        assert!(still.distance_on(&Mat3::IDENTITY, &[[0.0, 0.0], [1280.0, 720.0]]) < 1e-9);
        // As a path writes it, to the ninth decimal.
        let fine = matrix_of(&[
            1.0,
            0.0,
            0.123_456_789_123,
            0.0,
            1.0,
            -0.000_000_000_4,
            0.0,
            0.0,
        ]);
        assert_eq!(
            as_written(&fine),
            Some([1.0, 0.0, 0.123_456_789, 0.0, 1.0, 0.0, 0.0, 0.0])
        );
        assert_eq!(
            as_written(&Mat3([[1.0; 3], [1.0; 3], [1.0, 1.0, 0.0]])),
            None
        );
    }

    #[test]
    fn the_region_is_made_a_polygon_of_pixels_and_one_with_no_area_in_the_picture_is_refused() {
        let size = [100, 50];
        let region = region_of(
            &[
                Place { x: 0.0, y: 1.0 },
                Place { x: 1.0, y: 1.0 },
                Place { x: 0.5, y: 0.0 },
            ],
            size,
        );
        assert_eq!(region, [[-0.5, -0.5], [99.5, -0.5], [49.5, 49.5]]);
        assert_eq!(check_region(&region, size).map_err(|f| f.message), Ok(()));
        // Of a pixel at least, and reaching into the frame at its last pixel.
        let small = [[98.0, 48.0], [99.5, 48.0], [99.5, 49.5]];
        assert!(check_region(&small, size).is_ok());
        for wrong in [
            // Two places, three on a line, less than a pixel.
            vec![[0.0, 0.0], [10.0, 0.0]],
            vec![[0.0, 0.0], [10.0, 10.0], [20.0, 20.0]],
            vec![[10.0, 10.0], [10.5, 10.0], [10.5, 10.5]],
            // Beyond the right, above the top, beyond the bottom.
            vec![[200.0, 10.0], [300.0, 10.0], [250.0, 40.0]],
            vec![[10.0, -80.0], [20.0, -80.0], [15.0, -60.0]],
            vec![[10.0, 49.5], [20.0, 49.5], [15.0, 80.0]],
            // A place that is no number.
            vec![[f64::NAN, 0.0], [10.0, 0.0], [5.0, 5.0]],
        ] {
            let fault = check_region(&wrong, size).unwrap_err();
            assert_eq!(reason(&fault), "region", "{wrong:?}");
        }
    }

    #[test]
    fn a_track_starts_from_where_the_plane_is_at_the_playhead() {
        let mut planar = PlanarTracker {
            reference: 10,
            path: [(10, STILL), (20, shifted(0.1, 0.0))].into_iter().collect(),
            ..PlanarTracker::default()
        };
        let plan = planned(&project(planar.clone()), &aimed("plane", 20)).unwrap();
        assert_eq!(plan.size, [1280, 720]);
        assert_eq!((plan.first, plan.last, plan.start), (0, 99, 20));
        assert_eq!(plan.direction, Direction::Forward);
        // The region where the path has it at the playhead: a tenth of the width to the right.
        let Region::Polygon(here) = &plan.region else {
            panic!("a polygon")
        };
        for (at, place) in here.iter().zip(&planar.region) {
            let wanted = [
                (place.x + 0.1) * 1280.0 - 0.5,
                (1.0 - place.y) * 720.0 - 0.5,
            ];
            assert!((at[0] - wanted[0]).abs() < 1e-6 && (at[1] - wanted[1]).abs() < 1e-6);
        }
        // What the path has there is written there; the middle of the region as it was drawn
        // is what the parts of a motion are kept about.
        assert_eq!(plan.at_start, shifted(0.1, 0.0));
        assert_eq!(plan.rest, Mat3::IDENTITY);
        assert!(near(plan.anchor, [639.5, 359.5]));
        assert!(plan.perspective && plan.components == Components::ALL);
        // Between two frames of the path, and with a motion of a shift alone.
        planar.motion = PlanarMotion::Translation;
        let plan = planned(&project(planar.clone()), &aimed("plane", 15)).unwrap();
        assert_eq!(plan.at_start, shifted(0.05, 0.0));
        assert!(!plan.perspective);
        assert!(!plan.components.zoom && !plan.components.rotate);
        // From another reference frame: the motion is counted from what the path has there.
        planar.reference = 20;
        let plan = planned(&project(planar), &aimed("plane", 20)).unwrap();
        assert!(
            plan.base
                .distance_on(&Mat3::IDENTITY, &[[0.0, 0.0], [1280.0, 720.0]])
                < 1e-9
        );
        assert_eq!(plan.rest, matrix_of(&shifted(0.1, 0.0)));
    }

    #[test]
    fn a_track_of_what_cannot_be_followed_is_refused() {
        let project = project(PlanarTracker::default());
        for (node, frame, wanted) in [
            ("in", 5, "wrong_node"),
            ("n404", 5, "target"),
            ("plane", 100, "frame"),
            ("plane", -1, "frame"),
        ] {
            let fault = planned(&project, &aimed(node, frame))
                .err()
                .unwrap_or_else(|| panic!("{node} {frame} is tracked"));
            assert_eq!(reason(&fault), wanted, "{node} {frame}");
        }
        let fault = planned(&project, &aimed("in", 5)).err().unwrap();
        assert_eq!(fault.data.unwrap()["kind"], "media_in");
        // A region outside the picture; and one that the path takes out of it at the playhead.
        let outside = PlanarTracker {
            region: vec![
                Place { x: 2.0, y: 2.0 },
                Place { x: 3.0, y: 2.0 },
                Place { x: 3.0, y: 3.0 },
            ],
            ..PlanarTracker::default()
        };
        let fault = planned(&self::project(outside), &aimed("plane", 5))
            .err()
            .unwrap();
        assert_eq!(reason(&fault), "region");
        let gone = PlanarTracker {
            path: [(0, STILL), (50, shifted(3.0, 0.0))].into_iter().collect(),
            ..PlanarTracker::default()
        };
        assert!(planned(&self::project(gone.clone()), &aimed("plane", 0)).is_ok());
        let fault = planned(&self::project(gone), &aimed("plane", 50))
            .err()
            .unwrap();
        assert_eq!(reason(&fault), "region");
        // A plane that the path flattens at the reference frame cannot be put back.
        let flat = PlanarTracker {
            reference: 50,
            path: [(0, STILL), (50, [0.0; 8])].into_iter().collect(),
            ..PlanarTracker::default()
        };
        let fault = planned(&self::project(flat), &aimed("plane", 0))
            .err()
            .unwrap();
        assert_eq!(reason(&fault), "region");
        // A clip without a composition, and a still, which has no motion.
        let mut other = aimed("plane", 5);
        other.aim.clip = "c2".into();
        assert_eq!(reason(&planned(&project, &other).err().unwrap()), "target");
        let mut still = project.clone();
        still.media[0].facts.video.as_mut().unwrap().still = true;
        let fault = planned(&still, &aimed("plane", 5)).err().unwrap();
        assert_eq!(reason(&fault), "untrackable");
    }

    #[test]
    fn the_parts_of_the_motion_that_the_node_names_are_kept_about_the_middle_of_the_region() {
        let mut plan = by_hand(Direction::Forward);
        // A turn of a tenth of a radian about the anchor and a shift of (3, -2).
        let (sin, cos) = 0.1_f64.sin_cos();
        let anchor = plan.anchor;
        let turned = Mat3::translation(anchor[0] + 3.0, anchor[1] - 2.0)
            .mul(&Mat3([[cos, -sin, 0.0], [sin, cos, 0.0], [0.0, 0.0, 1.0]]))
            .mul(&Mat3::translation(-anchor[0], -anchor[1]));
        // All of it with Perspective.
        assert_eq!(kept(&plan, &turned), Some(turned));
        // The shift of the anchor alone with Translation.
        plan.components = PlanarMotion::Translation.components();
        let shift = kept(&plan, &turned).unwrap();
        for point in [anchor, [anchor[0] + 10.0, anchor[1]], [0.0, 0.0]] {
            let moved = shift.apply(point);
            assert!(near(moved, [point[0] + 3.0, point[1] - 2.0]), "{point:?}");
        }
        // The turn too with Translation and Rotation.
        plan.components = PlanarMotion::TranslationRotation.components();
        let both = kept(&plan, &turned).unwrap();
        assert!(both.distance_on(&turned, &[[0.0, 0.0], [160.0, 120.0]]) < 1e-9);
        // A motion that turns the plane over has no parts.
        let over = Mat3([[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
        assert_eq!(kept(&plan, &over), None);
    }

    #[test]
    fn what_is_written_is_a_patch_of_the_frames_that_the_path_has_not_as_they_were_found() {
        let planar = PlanarTracker {
            path: [(0, STILL), (5, shifted(0.5, 0.0))].into_iter().collect(),
            ..PlanarTracker::default()
        };
        let project = project(planar);
        let plan = planned(&project, &aimed("plane", 0)).unwrap();
        let mut found = Found::default();
        found.begin(0, plan.at_start);
        assert_eq!(write(&project, &plan, &found).unwrap(), None);
        found.found(1, Some(shifted(0.01, 0.0)));
        found.found(2, None);
        found.found(5, Some(shifted(0.5, 0.0)));
        assert_eq!(
            (found.frames, found.lost, found.range()),
            (2, 1, Some((0, 5)))
        );
        let op = write(&project, &plan, &found).unwrap().unwrap();
        assert_eq!(
            op,
            json!({"op": "set_fusion_node", "timeline": "t1", "clip": "c1", "id": "plane",
                "params": {"path": {"1": shifted(0.01, 0.0)}}})
        );
        // A node that is no longer there.
        let mut gone = project.clone();
        gone.timelines[0].tracks[0].clips[0].fusion = None;
        assert_eq!(reason(&write(&gone, &plan, &found).unwrap_err()), "target");
    }

    /// Frames of 160 × 120 of a textured box, from (40, 30) to (100, 80) at frame 0, that moves
    /// 2 pixels to the right and 1 down a frame, over a flat ground.
    const FRAMES: [usize; 2] = [160, 120];

    /// Where the box is moved at frame `k`.
    fn moved_by(k: i64) -> [f64; 2] {
        let k = f64::from(i32::try_from(k).unwrap());
        [2.0 * k, k]
    }

    /// Frame `k` of the box.
    fn frame_of(k: i64) -> Luma {
        let [dx, dy] = moved_by(k);
        let mut data = Vec::with_capacity(FRAMES[0] * FRAMES[1]);
        for y in 0..120_u32 {
            for x in 0..160_u32 {
                let (u, v) = (f64::from(x) - 40.0 - dx, f64::from(y) - 30.0 - dy);
                let inside = u.min(60.0 - u).min(v).min(50.0 - v);
                let cover = (inside + 0.5).clamp(0.0, 1.0);
                let texture = 0.45
                    + 0.18 * (0.6 * u).sin() * (0.45 * v).cos()
                    + 0.12 * (0.21 * u + 0.33 * v).sin()
                    + 0.1 * (0.9 * v - 0.3 * u).cos();
                #[allow(clippy::cast_possible_truncation, reason = "a sample")]
                let sample = (0.1 + cover * (texture - 0.1)) as f32;
                data.push(sample);
            }
        }
        Luma::new(FRAMES[0], FRAMES[1], data).unwrap()
    }

    /// A track of the box `direction` from frame 0 to frame 10, its region drawn within it at
    /// frame 0, the reference frame, with no path: Perspective.
    fn by_hand(direction: Direction) -> Plan {
        let drawn = vec![[44.0, 34.0], [96.0, 34.0], [96.0, 76.0], [44.0, 76.0]];
        Plan {
            job: "track-1".into(),
            owner: 7,
            kind: Kind::Agent,
            timeline: "t1".into(),
            clip: "c1".into(),
            node: "plane".into(),
            media: Project::sample().media[0].clone(),
            size: FRAMES,
            first: 0,
            last: 10,
            start: 0,
            direction,
            shown: (0..=10).map(|frame| (frame, frame)).collect(),
            anchor: Region::Polygon(drawn.clone()).centroid().unwrap(),
            region: Region::Polygon(drawn),
            base: Mat3::IDENTITY,
            rest: Mat3::IDENTITY,
            at_start: STILL,
            perspective: true,
            components: Components::ALL,
        }
    }

    #[test]
    fn a_plane_is_followed_and_its_path_puts_its_corners_where_they_went() {
        let plan = by_hand(Direction::Forward);
        let mut found = Found::default();
        let mut steps = Vec::new();
        follow(
            &plan,
            &mut found,
            &mut |frame| (0..=10).contains(&frame).then(|| frame_of(frame)),
            &AtomicBool::new(false),
            &mut |frame, _| steps.push(frame),
        )
        .unwrap();
        assert_eq!(steps, (1..=10).collect::<Vec<_>>());
        assert_eq!((found.frames, found.lost), (10, 0));
        assert_eq!(found.path[&0], STILL);
        // Each homography puts the corners of the box where they went, in pixels of the
        // frames.
        let corners = [[40.0, 30.0], [100.0, 30.0], [100.0, 80.0], [40.0, 80.0]];
        for (&frame, sample) in &found.path {
            let moved = to_pixels(&matrix_of(sample), FRAMES);
            let [dx, dy] = moved_by(frame);
            for corner in corners {
                let at = moved.apply(corner);
                let off = (at[0] - corner[0] - dx).hypot(at[1] - corner[1] - dy);
                assert!(off < 0.1, "{frame}: {corner:?} is {off} pixels off");
            }
        }
        // Stopped between frames, what was found before stays.
        let mut found = Found::default();
        let stop = AtomicBool::new(false);
        follow(
            &plan,
            &mut found,
            &mut |frame| (0..=10).contains(&frame).then(|| frame_of(frame)),
            &stop,
            &mut |frame, _| {
                if frame == 3 {
                    stop.store(true, Ordering::Relaxed);
                }
            },
        )
        .unwrap();
        assert_eq!(found.path.keys().copied().collect::<Vec<_>>(), [0, 1, 2, 3]);
        // Backward from the last frame, from where the path has the plane there.
        let mut back = by_hand(Direction::Reverse);
        back.start = 10;
        back.at_start = found_at(10);
        back.base = to_pixels(&matrix_of(&back.at_start), FRAMES);
        let Region::Polygon(drawn) = &back.region else {
            panic!("a polygon")
        };
        let here: Vec<Point> = drawn.iter().map(|&at| back.base.apply(at)).collect();
        back.region = Region::Polygon(here);
        let mut found = Found::default();
        follow(
            &back,
            &mut found,
            &mut |frame| (0..=10).contains(&frame).then(|| frame_of(frame)),
            &AtomicBool::new(false),
            &mut |_, _| {},
        )
        .unwrap();
        assert_eq!(found.frames, 10);
        let first = to_pixels(&matrix_of(&found.path[&0]), FRAMES);
        assert!(first.distance_on(&Mat3::IDENTITY, &corners) < 0.1);
        // A frame that cannot be had ends it, and what was found before stays.
        let mut found = Found::default();
        let error = follow(
            &plan,
            &mut found,
            &mut |frame| (0..=4).contains(&frame).then(|| frame_of(frame)),
            &AtomicBool::new(false),
            &mut |_, _| {},
        )
        .unwrap_err();
        assert_eq!(error, TrackError::MissingFrame(5));
        assert_eq!(found.range(), Some((0, 4)));
    }

    /// The homography of places that moves the box as it moved by frame `k`, as a path writes
    /// it.
    fn found_at(k: i64) -> Homography {
        let [dx, dy] = moved_by(k);
        as_written(&to_shares(&Mat3::translation(dx, dy), FRAMES)).unwrap()
    }

    #[test]
    fn a_track_is_of_a_window_of_a_tracker_or_of_a_planar_tracker_and_never_of_two() {
        use super::super::{Asked, Request};
        let request = |more: &Value| {
            let mut request = json!({"revision": 3, "frame": 5, "direction": "forward"});
            for (name, value) in more.as_object().unwrap() {
                request[name] = value.clone();
            }
            request
        };
        let ask = |more: &Value| {
            serde_json::from_value::<Request>(request(more))
                .unwrap()
                .asked()
        };
        let aim = json!({"timeline": "t1", "clip": "c1", "node": "plane"});
        let Ok((3, Asked::Planar(asked))) = ask(&json!({ "planar": aim })) else {
            panic!("a Planar Tracker is asked")
        };
        assert_eq!(asked.aim.node, "plane");
        assert_eq!((asked.frame, asked.direction), (5, Direction::Forward));
        let window = json!({"grade": {"level": "clip", "timeline": "t1", "clip": "c1"},
            "node": "n1", "window": "w1", "planar": aim});
        let tracker = json!({"timeline": "t1", "clip": "c1", "node": "track"});
        for wrong in [
            window,
            json!({"planar": aim, "fusion": tracker}),
            json!({"planar": aim, "tracker": "point"}),
        ] {
            let Err(fault) = ask(&wrong) else {
                panic!("{wrong} is asked")
            };
            assert_eq!(fault.code, rpc::INVALID_PARAMS, "{wrong}");
            assert_eq!(reason(&fault), "fusion", "{wrong}");
        }
        // The patterns of a Tracker are not named of a Planar Tracker.
        let named = json!({"planar": {"timeline": "t1", "clip": "c1", "node": "plane",
            "trackers": [0]}});
        assert!(serde_json::from_value::<Request>(request(&named)).is_err());
    }
}
