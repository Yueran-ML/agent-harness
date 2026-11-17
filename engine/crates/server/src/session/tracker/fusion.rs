//! `track.start` with `fusion` (T17b): the patterns of a Tracker of a composition (Reference
//! Manual 20, Chapter 82, pp. 1724–1759) followed through the frames of its clip, each on its
//! own, and their paths written into the node.
//!
//! A track of a Tracker goes as a track of a window does (the module above): it is checked
//! against the project as it is, a native gesture of the connection that asked is reserved, it
//! is answered at once and runs on a thread of its own; the track so far is written as the next
//! change of that gesture as often as a window's track is, `track.progress` told each time and
//! `track.state` at its end; and it is ended between frames by `track.stop`, a change of the
//! project by anyone else, Undo or Redo, the connection going, and a frame that cannot be
//! decoded. What differs:
//!
//! - **What is read.** The frames of the clip's own media, as a track of a window of the clip
//!   reads them: the picture of its `MediaIn`, upright, at the analysis size. Not the picture
//!   that comes into the Tracker: the nodes before it are not drawn for the track.
//! - **What is followed.** The patterns named, or each that is on, each on its own
//!   (`columbina_track::PatternTracker`), from where it is at the playhead: where its path has
//!   it there, or where it was put where it has no path. Its box and its search area are made
//!   samples of the frames read.
//! - **Where.** A place of a path, `[x, y]` in shares of the width from the left and of the
//!   height from the bottom, lies at `x · W` and `(1 − y) · H` of a picture `W × H` that reaches
//!   from 0 to its sides, as the mathematics of the Tracker has it; the tracker counts the
//!   middle of the pixel in column `i` and row `j` as `[i, j]`, half a pixel less. The frames
//!   read are the picture of the composition scaled to the analysis size, at its aspect: a
//!   share of either is one place of the picture.
//! - **What is written.** Each frame at which a pattern was found becomes a place of its path,
//!   and so does the frame that it started from: those frames are overwritten, the others kept.
//!   A pattern lost is followed no further that way, and the frames beyond keep what they had.
//!   The list of patterns of the node is set whole by `set_fusion_node`, each pattern that is
//!   not followed, and each other setting, as the project has it.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use columbina_model::fusion::{Composition, NodeKind, Pattern, Tracker};
use columbina_model::{Media, Project};
use columbina_track::{Direction, Luma, PatternOptions, PatternTracker, Point, TrackError};
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

/// The places of a path are written to the seventh decimal: a ten-thousandth of a pixel of a
/// picture a thousand pixels wide, so that a long path is not sent with seventeen digits a
/// number at each write.
const DECIMALS: f64 = 1e7;

/// What `fusion` names: the Tracker `node` of the composition of the clip `clip` of `timeline`,
/// and, by their places in its list, the patterns of it to follow (each that is on, where they
/// are left out).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Aim {
    timeline: String,
    clip: String,
    node: String,
    #[serde(default)]
    trackers: Option<Vec<usize>>,
}

/// A track of a Tracker, as `track.start` asks for it, but for the revision that it was asked
/// at.
pub(super) struct Start {
    /// What is tracked.
    pub(super) aim: Aim,
    /// The playhead, as the keys of the clip count frames.
    pub(super) frame: i64,
    /// Which way, on the timeline.
    pub(super) direction: Direction,
}

/// What a track of a Tracker reads and where it writes, fixed when it starts.
pub(super) struct Plan {
    job: String,
    owner: u64,
    kind: Kind,
    timeline: String,
    clip: String,
    node: String,
    media: Media,
    /// The upright frames' analysis size: the frames that the patterns are followed on.
    pub(super) size: [usize; 2],
    pub(super) first: i64,
    pub(super) last: i64,
    /// The playhead, as the keys of the clip count frames: where each way of the track starts.
    start: i64,
    /// Which way, in the source.
    pub(super) direction: Direction,
    /// Where each source frame is first shown on the timeline.
    shown: BTreeMap<i64, i64>,
    /// The patterns followed, by their places in the list of the node, in order.
    pub(super) followed: Vec<usize>,
    /// Where each is at `start`, as a place of a path: what is written there.
    starts: Vec<[f64; 2]>,
    /// Each, as it is followed, in samples of the frames read.
    patterns: Vec<columbina_track::Pattern>,
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
    let tracker = tracker_of(clip.fusion.as_ref(), &aim.node)?;
    let untrackable = |why: &str| {
        refused(
            "untrackable",
            &format!("this Tracker cannot be tracked: {why}"),
        )
    };
    let (media, video) = picture_of(project, clip, &untrackable)?;
    let followed = followed(tracker, aim.trackers.as_deref())?;
    let Shown { map, first, last } = shown_frames(
        project,
        timeline,
        clip,
        (media, video),
        request.frame,
        &untrackable,
    )?;
    let size = analysis_size(video);
    let starts: Vec<[f64; 2]> = followed
        .iter()
        .map(|&at| start_of(&tracker.trackers[at], request.frame))
        .collect();
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
        patterns: sought(&tracker.trackers, &followed, &starts, size),
        followed,
        starts,
    })
}

/// The Tracker `node` of `composition`; or why there is none: no composition or no such node
/// (`target`), or a node of another kind (`wrong_node`, with its `kind`).
fn tracker_of<'a>(composition: Option<&'a Composition>, node: &str) -> Result<&'a Tracker, Fault> {
    let found = composition
        .and_then(|composition| composition.node(node))
        .ok_or_else(|| refused("target", "the clip's composition has no such node"))?;
    let NodeKind::Tracker(tracker) = &found.kind else {
        let data = json!({"reason": "wrong_node", "node": node, "kind": found.kind.kind()});
        return Err(Fault::new(rpc::REFUSED, format!("{node} is not a tracker")).with_data(data));
    };
    Ok(tracker)
}

/// The patterns of `tracker` that a track follows, by their places in its list, in order and
/// each once: those `named`, each of which is to be there and on (`tracker`), or, where none
/// are named, each that is on; at least one (`no_trackers`).
fn followed(tracker: &Tracker, named: Option<&[usize]>) -> Result<Vec<usize>, Fault> {
    let mut out: Vec<usize> = match named {
        None => tracker
            .trackers
            .iter()
            .enumerate()
            .filter(|(_, pattern)| pattern.enabled)
            .map(|(at, _)| at)
            .collect(),
        Some(named) => {
            for &at in named {
                match tracker.trackers.get(at) {
                    None => return Err(wrong_pattern(at, "the Tracker has no pattern there")),
                    Some(pattern) if !pattern.enabled => {
                        return Err(wrong_pattern(at, "the pattern is off"));
                    }
                    Some(_) => {}
                }
            }
            named.to_vec()
        }
    };
    out.sort_unstable();
    out.dedup();
    if out.is_empty() {
        return Err(refused(
            "no_trackers",
            "the Tracker has no pattern to follow: none is on",
        ));
    }
    Ok(out)
}

/// The refusal of the pattern at `at` of the list, which cannot be followed for `why`.
fn wrong_pattern(at: usize, why: &str) -> Fault {
    Fault::new(rpc::REFUSED, format!("pattern {at}: {why}"))
        .with_data(json!({"reason": "tracker", "tracker": at}))
}

/// Where `pattern` is at `frame`: where its path has it there, or where it was put where it
/// has no path.
#[allow(clippy::cast_precision_loss, reason = "frames are within 2^53")]
fn start_of(pattern: &Pattern, frame: i64) -> [f64; 2] {
    pattern
        .place_at(frame as f64)
        .unwrap_or([pattern.pattern.x, pattern.pattern.y])
}

/// The width and the height of frames of `size`.
#[allow(clippy::cast_precision_loss, reason = "sides of at most 16384")]
fn extent(size: [usize; 2]) -> (f64, f64) {
    (size[0] as f64, size[1] as f64)
}

/// The place `share` of a picture, as a path has it, in samples of frames of `size` as the
/// tracker counts them (see the module).
fn pixels(share: [f64; 2], size: [usize; 2]) -> Point {
    let (width, height) = extent(size);
    [share[0] * width - 0.5, (1.0 - share[1]) * height - 0.5]
}

/// The place, as a path has it, of `point` of frames of `size`: the inverse of [`pixels`].
fn share(point: Point, size: [usize; 2]) -> [f64; 2] {
    let (width, height) = extent(size);
    [(point[0] + 0.5) / width, 1.0 - (point[1] + 0.5) / height]
}

/// `place` as a path is written: to the seventh decimal ([`DECIMALS`]).
fn written_place(place: [f64; 2]) -> [f64; 2] {
    place.map(|at| (at * DECIMALS).round() / DECIMALS)
}

/// The patterns at `followed` of `trackers` as they are followed from `starts` on frames of
/// `size`: their places, boxes and search areas in samples of the frames.
fn sought(
    trackers: &[Pattern],
    followed: &[usize],
    starts: &[[f64; 2]],
    size: [usize; 2],
) -> Vec<columbina_track::Pattern> {
    let (width, height) = extent(size);
    followed
        .iter()
        .zip(starts)
        .map(|(&at, &start)| {
            let pattern = &trackers[at];
            columbina_track::Pattern::new(
                pixels(start, size),
                [
                    pattern.pattern.width * width,
                    pattern.pattern.height * height,
                ],
                [pattern.search.width * width, pattern.search.height * height],
            )
        })
        .collect()
}

/// `trackers` with `paths` written into the paths of those at `followed`: each frame of a path
/// written over, the others kept. None where a pattern followed is not in the list.
fn merged(
    trackers: &[Pattern],
    followed: &[usize],
    paths: &[BTreeMap<i64, [f64; 2]>],
) -> Option<Vec<Pattern>> {
    let mut out = trackers.to_vec();
    for (&at, path) in followed.iter().zip(paths) {
        out.get_mut(at)?
            .path
            .extend(path.iter().map(|(&frame, &place)| (frame, place)));
    }
    Some(out)
}

/// The operation that writes what `found` has into the paths of the patterns that `plan`
/// follows, of the node as `project` has it: its list of patterns set whole.
fn write(project: &Project, plan: &Plan, found: &Found) -> Result<Value, Fault> {
    let (_, clip) = clip_of(project, &plan.timeline, &plan.clip)
        .ok_or_else(|| refused("target", "no such clip"))?;
    let tracker = tracker_of(clip.fusion.as_ref(), &plan.node)?;
    let trackers = merged(&tracker.trackers, &plan.followed, &found.paths).ok_or_else(|| {
        refused(
            "tracker",
            "the Tracker no longer has a pattern that is followed",
        )
    })?;
    Ok(
        json!({"op": "set_fusion_node", "timeline": plan.timeline, "clip": plan.clip,
        "id": plan.node, "params": {"trackers": trackers}}),
    )
}

/// What a track of a Tracker has found, pattern by pattern, in the order of the patterns
/// followed.
struct Found {
    /// The places found, by the frame, as they are written.
    paths: Vec<BTreeMap<i64, [f64; 2]>>,
    /// At how many frames each was found, those it started from not counted.
    frames: Vec<usize>,
    /// The frames at which each was lost: on each way, the first that it was not found at.
    lost: Vec<Vec<i64>>,
    /// Whether each is followed on the way going on.
    following: Vec<bool>,
    /// How many times a pattern was lost, in all.
    lost_count: usize,
    /// Whether a pattern was found at a frame other than the one it started from.
    stepped: bool,
    /// How many places were put into the paths, so that what was written is told from what
    /// is not.
    changes: u64,
}

impl Found {
    /// Nothing found yet of `count` patterns.
    fn new(count: usize) -> Self {
        Self {
            paths: vec![BTreeMap::new(); count],
            frames: vec![0; count],
            lost: vec![Vec::new(); count],
            following: vec![false; count],
            lost_count: 0,
            stepped: false,
            changes: 0,
        }
    }

    /// A way of the track that starts at `start`, the patterns at `starts` there as places of
    /// paths and at `points` as the tracker has them: each in the frame is followed and its
    /// place there written, and each outside it is lost there.
    fn begin(&mut self, start: i64, starts: &[[f64; 2]], points: &[Option<Point>]) {
        for (at, point) in points.iter().enumerate() {
            self.following[at] = point.is_some();
            if point.is_some() {
                self.paths[at].insert(start, starts[at]);
                self.changes += 1;
            } else {
                self.lost[at].push(start);
                self.lost_count += 1;
            }
        }
    }

    /// The patterns at `points` at `frame`, in samples of frames of `size`: each found is
    /// written there, and each that was followed and is no longer found is lost there.
    fn found(&mut self, frame: i64, points: &[Option<Point>], size: [usize; 2]) {
        for (at, point) in points.iter().enumerate() {
            match point {
                Some(point) => {
                    self.paths[at].insert(frame, written_place(share(*point, size)));
                    self.frames[at] += 1;
                    self.stepped = true;
                    self.changes += 1;
                }
                None if self.following[at] => {
                    self.following[at] = false;
                    self.lost[at].push(frame);
                    self.lost_count += 1;
                }
                None => {}
            }
        }
    }

    /// The first and the last frame written into a path, or none.
    fn range(&self) -> Option<(i64, i64)> {
        let low = self
            .paths
            .iter()
            .filter_map(|path| path.keys().next())
            .min()?;
        let high = self
            .paths
            .iter()
            .filter_map(|path| path.keys().next_back())
            .max()?;
        Some((*low, *high))
    }

    /// The places in the list of the node, `followed`, of the patterns followed on the way
    /// going on.
    fn following_of(&self, followed: &[usize]) -> Vec<usize> {
        followed
            .iter()
            .zip(&self.following)
            .filter(|(_, on)| **on)
            .map(|(&at, _)| at)
            .collect()
    }

    /// Pattern by pattern, as `track.state` tells it: its place in the list of the node
    /// (`followed`), at how many frames it was found, and the frames at which it was lost.
    fn report(&self, followed: &[usize]) -> Value {
        followed
            .iter()
            .zip(self.frames.iter().zip(&self.lost))
            .map(|(&at, (frames, lost))| json!({"tracker": at, "frames": frames, "lost": lost}))
            .collect()
    }
}

/// Follow the patterns of `plan` through the frames that `read` gives, way after way as its
/// direction says, into `found`; `step` hears of each frame as it is done, with what was found
/// so far. Stop is looked at between frames; a way ends where every pattern is lost.
///
/// # Errors
/// [`TrackError::MissingFrame`] where `read` has no frame, which is so once Stop is set too.
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
    for &(way, most) in ways {
        let end = if way > 0 { plan.last } else { plan.first };
        let at_start = read(plan.start).ok_or(TrackError::MissingFrame(plan.start))?;
        let mut tracker =
            PatternTracker::new(&at_start, &plan.patterns, PatternOptions::default())?;
        found.begin(plan.start, &plan.starts, tracker.points());
        let mut frame = plan.start;
        let mut done = 0;
        while frame != end && most.is_none_or(|m| done < m) && found.following.contains(&true) {
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            frame += way;
            let luma = read(frame).ok_or(TrackError::MissingFrame(frame))?;
            let points = tracker.step_owned(luma)?;
            done += 1;
            found.found(frame, points, plan.size);
            each_frame(frame, found);
        }
    }
    Ok(())
}

/// Track `plan` with `gesture` until it ends or `stop` is set: what a track of a Tracker does
/// on its thread (see the module).
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
    let mut found = Found::new(plan.followed.len());
    let mut done = 0_i64;
    let mut reached = plan.start;
    let mut written = None::<u64>;
    let mut stopped_by: Option<String> = None;
    let mut last_commit = Instant::now();
    let mut interval = COMMIT_EVERY;
    // The changes of `found` that were written.
    let mut committed = 0_u64;
    let tell = |state: &mut State,
                gesture: &mut OwnedGesture,
                found: &Found,
                reached: i64,
                done: i64|
     -> Result<(u64, usize), Fault> {
        let (revision, bytes) = state.track_write(plan.owner, plan.kind, gesture, |project| {
            write(project, plan, found)
        })?;
        let timeline = plan
            .shown
            .range(..=reached)
            .next_back()
            .or_else(|| plan.shown.range(reached..).next())
            .map(|(_, &t)| t);
        state.track_tell(
            plan.owner,
            "track.progress",
            &json!({"job":plan.job,"frame":reached,"timeline_frame":timeline,"done":done,
                "total":total,"lost":found.lost_count,"revision":revision,
                "following":found.following_of(&plan.followed)}),
        );
        Ok((revision, bytes))
    };
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
                    tell(&mut locked, &mut gesture, found, frame, done)
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
        match tell(&mut locked, &mut gesture, &found, reached, done) {
            Ok((revision, _)) => written = Some(revision),
            Err(fault) => stopped_by = Some(reason_of(&fault)),
        }
    }
    let user_stop = stop.load(Ordering::Relaxed) && stopped_by.is_none();
    let report = json!({"job":plan.job,"frames":done,"lost":found.lost_count,"frame":reached,
        "revision":written.unwrap_or(locked.history.revision()),"tracked":found.range(),
        "trackers":found.report(&plan.followed)});
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
        reason = "places that floats hold whole, or that are compared as they were written"
    )]

    use columbina_model::fusion::{Input, Kind as FusionKind, Link, Node};
    use columbina_model::retime::{Retime, SignedSpeed, SpeedPoint};
    use columbina_track::matchmove::from_fusion;

    use super::super::{Asked, Request};
    use super::*;

    /// The sample project with a composition on its clip `c1`: a `MediaIn` into the Tracker
    /// `track` into the `MediaOut`, the Tracker with `trackers`.
    fn project(trackers: Vec<Pattern>) -> Project {
        let mut project = Project::sample();
        let mut node = Node::new("track", FusionKind::Tracker);
        let NodeKind::Tracker(tracker) = &mut node.kind else {
            unreachable!("a new Tracker")
        };
        tracker.trackers = trackers;
        project.timelines[0].tracks[0].clips[0].fusion = Some(Composition {
            nodes: vec![
                Node::new("in", FusionKind::MediaIn),
                node,
                Node::new("out", FusionKind::MediaOut),
            ],
            links: vec![
                Link::new("in", "track", Input::Input),
                Link::new("track", "out", Input::Input),
            ],
        });
        project
    }

    /// Three patterns: one with a path from frame 0 to frame 10, one that is off, and one put
    /// with no path.
    fn three() -> Vec<Pattern> {
        let mut pathed = Pattern::new(0.25, 0.75);
        pathed.path = [(0, [0.25, 0.75]), (10, [0.5, 0.5])].into_iter().collect();
        let mut off = Pattern::new(0.5, 0.5);
        off.enabled = false;
        vec![pathed, off, Pattern::new(0.75, 0.25)]
    }

    /// A track forward from frame `frame` of the patterns `trackers` of the node `node` of `c1`.
    fn aimed(trackers: Option<Vec<usize>>, node: &str, frame: i64) -> Start {
        Start {
            aim: Aim {
                timeline: "t1".into(),
                clip: "c1".into(),
                node: node.into(),
                trackers,
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
    fn a_place_of_a_path_is_half_a_pixel_from_where_the_tracker_counts_it() {
        let size = [1280, 720];
        // The middle of a pixel is a whole place for the tracker, and back.
        for (i, j) in [(0, 0), (639, 359), (1279, 719), (17, 400)] {
            let place = [
                (f64::from(i) + 0.5) / 1280.0,
                1.0 - (f64::from(j) + 0.5) / 720.0,
            ];
            let at = pixels(place, size);
            assert!(
                (at[0] - f64::from(i)).abs() < 1e-9 && (at[1] - f64::from(j)).abs() < 1e-9,
                "{at:?}"
            );
            let back = share(at, size);
            assert!(
                (back[0] - place[0]).abs() < 1e-12 && (back[1] - place[1]).abs() < 1e-12,
                "{back:?}"
            );
        }
        // Half a pixel before where the mathematics of the Tracker has a place.
        for place in [
            [0.25, 0.75],
            [0.0, 1.0],
            [1.0, 0.0],
            [0.5, 0.5],
            [-0.25, 1.5],
        ] {
            let mathematics = from_fusion(place, (1280.0, 720.0)).unwrap();
            let at = pixels(place, size);
            assert!(
                (at[0] + 0.5 - mathematics[0]).abs() < 1e-9
                    && (at[1] + 0.5 - mathematics[1]).abs() < 1e-9,
                "{place:?}"
            );
        }
        // The lower left corner of the picture is the outer corner of its first column and its
        // last row.
        assert_eq!(pixels([0.0, 0.0], size), [-0.5, 719.5]);
        // The places are written to the seventh decimal.
        assert_eq!(written_place([0.123_456_789, 0.5]), [0.123_456_8, 0.5]);
        assert_eq!(written_place([-0.000_000_04, 2.0]), [0.0, 2.0]);
    }

    #[test]
    fn a_track_follows_the_patterns_that_are_on_or_those_named_from_where_they_are() {
        let project = project(three());
        let plan = planned(&project, &aimed(None, "track", 5)).unwrap();
        assert_eq!(plan.followed, [0, 2]);
        assert_eq!(plan.size, [1280, 720]);
        assert_eq!((plan.first, plan.last, plan.start), (0, 99, 5));
        assert_eq!(plan.direction, Direction::Forward);
        // Where its path has it at the playhead, between two of its frames; and where it was
        // put, where it has no path.
        assert_eq!(plan.starts, [[0.375, 0.625], [0.75, 0.25]]);
        let put = plan.patterns[1];
        assert_eq!(put.at, [0.75 * 1280.0 - 0.5, 0.75 * 720.0 - 0.5]);
        // A box of 64 × 36 and a search area of 160 × 90, the sizes that a pattern has at
        // first, on frames of 1280 × 720: a window of 35 × 35, which may move 48 across and 27
        // down.
        assert_eq!(put.radius, 17);
        assert!((put.reach[0] - 48.0).abs() < 1e-9 && (put.reach[1] - 27.0).abs() < 1e-9);
        // Named: in their order, each once.
        let named = planned(&project, &aimed(Some(vec![2, 0, 2]), "track", 5)).unwrap();
        assert_eq!(named.followed, [0, 2]);
        // A clip played backward is tracked backward in its source, as a window is.
        let mut backward = project.clone();
        backward.timelines[0].tracks[0].clips[0].set_mapping(Some(Retime {
            points: vec![SpeedPoint {
                frame: 0,
                speed: SignedSpeed { num: -1, den: 1 },
            }],
        }));
        let plan = planned(&backward, &aimed(None, "track", 12)).unwrap();
        assert_eq!(plan.direction, Direction::Reverse);
    }

    #[test]
    fn a_track_of_what_cannot_be_followed_is_refused() {
        let project = project(three());
        for (trackers, node, frame, wanted) in [
            (Some(vec![1]), "track", 5, "tracker"),
            (Some(vec![0, 3]), "track", 5, "tracker"),
            (Some(vec![]), "track", 5, "no_trackers"),
            (None, "in", 5, "wrong_node"),
            (None, "n404", 5, "target"),
            (None, "track", 100, "frame"),
            (None, "track", -1, "frame"),
        ] {
            let fault = planned(&project, &aimed(trackers.clone(), node, frame))
                .err()
                .unwrap_or_else(|| panic!("{trackers:?} {node} {frame} is tracked"));
            assert_eq!(reason(&fault), wanted, "{trackers:?} {node} {frame}");
        }
        let fault = planned(&project, &aimed(Some(vec![1]), "track", 5))
            .err()
            .unwrap();
        assert_eq!(fault.data.unwrap()["tracker"], 1);
        let fault = planned(&project, &aimed(None, "in", 5)).err().unwrap();
        assert_eq!(fault.data.unwrap()["kind"], "media_in");
        // None on.
        let mut off = three();
        off[0].enabled = false;
        off[2].enabled = false;
        let fault = planned(&self::project(off), &aimed(None, "track", 5))
            .err()
            .unwrap();
        assert_eq!(reason(&fault), "no_trackers");
        // A clip without a composition, and a still, which has no motion.
        let mut other = aimed(None, "track", 5);
        other.aim.clip = "c2".into();
        assert_eq!(reason(&planned(&project, &other).err().unwrap()), "target");
        let mut still = project.clone();
        still.media[0].facts.video.as_mut().unwrap().still = true;
        let fault = planned(&still, &aimed(None, "track", 5)).err().unwrap();
        assert_eq!(reason(&fault), "untrackable");
    }

    #[test]
    fn what_a_track_found_is_written_over_the_frames_that_it_reached_and_the_rest_is_kept() {
        let mut patterns = three();
        patterns[2].path = [(0, [0.1, 0.1]), (5, [0.2, 0.2]), (9, [0.3, 0.3])]
            .into_iter()
            .collect();
        let paths: Vec<BTreeMap<i64, [f64; 2]>> = vec![
            BTreeMap::new(),
            [(5, [0.4, 0.4]), (6, [0.5, 0.5])].into_iter().collect(),
        ];
        let out = merged(&patterns, &[0, 2], &paths).unwrap();
        assert_eq!(out[..2], patterns[..2]);
        assert_eq!(
            out[2]
                .path
                .iter()
                .map(|(&frame, &place)| (frame, place))
                .collect::<Vec<_>>(),
            [
                (0, [0.1, 0.1]),
                (5, [0.4, 0.4]),
                (6, [0.5, 0.5]),
                (9, [0.3, 0.3])
            ]
        );
        assert_eq!(out[2].pattern, patterns[2].pattern);
        // A pattern followed that the node no longer has.
        assert!(merged(&patterns[..2], &[0, 2], &paths).is_none());
        // As the operation that writes it: the list of the node whole, as the project has it.
        let project = project(three());
        let plan = planned(&project, &aimed(None, "track", 5)).unwrap();
        let mut found = Found::new(2);
        found.begin(5, &plan.starts, &[Some([0.0; 2]), Some([0.0; 2])]);
        found.found(6, &[Some([639.5, 359.5]), None], plan.size);
        let op = write(&project, &plan, &found).unwrap();
        assert_eq!(op["op"], "set_fusion_node");
        assert_eq!(
            [&op["timeline"], &op["clip"], &op["id"]],
            [&json!("t1"), &json!("c1"), &json!("track")]
        );
        let trackers = &op["params"]["trackers"];
        assert_eq!(trackers.as_array().unwrap().len(), 3);
        assert_eq!(trackers[1], serde_json::to_value(&three()[1]).unwrap());
        assert_eq!(
            trackers[0]["path"],
            json!({"0": [0.25, 0.75], "5": [0.375, 0.625], "6": [0.5, 0.5], "10": [0.5, 0.5]})
        );
        assert_eq!(trackers[2]["path"], json!({"5": [0.75, 0.25]}));
        assert_eq!(
            trackers[2]["pattern"],
            serde_json::to_value(three()[2].pattern).unwrap()
        );
    }

    #[test]
    fn a_track_tells_what_it_found_pattern_by_pattern() {
        let size = [100, 50];
        let starts = [[0.5, 0.5], [2.0, 0.5]];
        let mut found = Found::new(2);
        // The second lies outside the frame: it is lost where it starts.
        found.begin(4, &starts, &[Some([49.5, 24.5]), None]);
        assert_eq!(found.following, [true, false]);
        assert!(!found.stepped);
        found.found(5, &[Some([50.5, 24.5]), None], size);
        found.found(6, &[None, None], size);
        // One lost is followed no more.
        found.found(7, &[None, None], size);
        assert!(found.stepped);
        assert_eq!(frames_of(&found, 0), [4, 5]);
        assert_eq!(found.paths[0][&5], [0.51, 0.5]);
        assert!(found.paths[1].is_empty());
        assert_eq!(found.range(), Some((4, 5)));
        assert_eq!(found.lost, [vec![6], vec![4]]);
        assert_eq!(found.lost_count, 2);
        assert_eq!(found.following_of(&[0, 3]), [] as [usize; 0]);
        assert_eq!(
            found.report(&[0, 3]),
            json!([{"tracker": 0, "frames": 1, "lost": [6]},
                   {"tracker": 3, "frames": 0, "lost": [4]}])
        );
        // The way back starts again from where they were.
        found.begin(4, &starts, &[Some([49.5, 24.5]), None]);
        assert_eq!(found.following_of(&[0, 3]), [0]);
        assert_eq!(found.lost[1], [4, 4]);
    }

    /// Frames of 160 × 120 of a textured blob that moves 2 pixels to the right a frame.
    const BLOB: [usize; 2] = [160, 120];

    /// Where the blob is at frame `k`, in samples of the frames.
    fn blob_at(k: i64) -> Point {
        [40.0 + 2.0 * f64::from(i32::try_from(k).unwrap()), 60.0]
    }

    /// Frame `k` of the blob, on a flat ground.
    fn blob(k: i64) -> Luma {
        let at = blob_at(k);
        let mut data = Vec::with_capacity(BLOB[0] * BLOB[1]);
        for y in 0..120_u32 {
            for x in 0..160_u32 {
                let (dx, dy) = (f64::from(x) - at[0], f64::from(y) - at[1]);
                let fall = (-(dx * dx + dy * dy) / (2.0 * 6.0 * 6.0)).exp();
                let texture = 0.5 + 0.2 * (0.3 * dx).sin() * (0.25 * dy).cos();
                #[allow(clippy::cast_possible_truncation, reason = "a sample")]
                let sample = (0.2 + fall * texture) as f32;
                data.push(sample);
            }
        }
        Luma::new(BLOB[0], BLOB[1], data).unwrap()
    }

    /// A track `direction` from frame 5 of frames 0 to 10 of the blob of two patterns of boxes
    /// of 24 × 24 searched for in 60 × 60: one on the blob, and one on the flat ground.
    fn blob_plan(direction: Direction) -> Plan {
        let mut on = Pattern::new(0.0, 0.0);
        on.pattern.width = 24.0 / 160.0;
        on.pattern.height = 24.0 / 120.0;
        on.search.width = 60.0 / 160.0;
        on.search.height = 60.0 / 120.0;
        let trackers = vec![on.clone(), on];
        let followed = vec![0, 1];
        let starts = vec![share(blob_at(5), BLOB), share([130.0, 20.0], BLOB)];
        Plan {
            job: "track-1".into(),
            owner: 7,
            kind: Kind::Agent,
            timeline: "t1".into(),
            clip: "c1".into(),
            node: "track".into(),
            media: Project::sample().media[0].clone(),
            size: BLOB,
            first: 0,
            last: 10,
            start: 5,
            direction,
            shown: (0..=10).map(|frame| (frame, frame)).collect(),
            patterns: sought(&trackers, &followed, &starts, BLOB),
            followed,
            starts,
        }
    }

    /// The frames of the paths of `found` of the pattern at `at`.
    fn frames_of(found: &Found, at: usize) -> Vec<i64> {
        found.paths[at].keys().copied().collect()
    }

    #[test]
    fn a_track_follows_its_patterns_each_way_and_one_lost_no_further() {
        let plan = blob_plan(Direction::ForwardAndBack);
        let mut found = Found::new(2);
        let mut steps = Vec::new();
        follow(
            &plan,
            &mut found,
            &mut |frame| (0..=10).contains(&frame).then(|| blob(frame)),
            &AtomicBool::new(false),
            &mut |frame, _| steps.push(frame),
        )
        .unwrap();
        // Forward to the last frame, then back from the playhead to the first.
        assert_eq!(steps, [6, 7, 8, 9, 10, 4, 3, 2, 1, 0]);
        assert_eq!(frames_of(&found, 0), (0..=10).collect::<Vec<_>>());
        for (&frame, &place) in &found.paths[0] {
            let at = pixels(place, BLOB);
            let truth = blob_at(frame);
            let off = (at[0] - truth[0]).hypot(at[1] - truth[1]);
            assert!(off < 0.1, "{frame}: {at:?}");
        }
        // The pattern on the flat ground is lost at once each way, and has its start alone.
        assert_eq!(found.lost[1], [6, 4]);
        assert_eq!(frames_of(&found, 1), [5]);
        assert_eq!((found.frames[0], found.frames[1]), (10, 0));
        // Stopped between frames: what was found before stays.
        let mut found = Found::new(2);
        let stop = AtomicBool::new(false);
        follow(
            &blob_plan(Direction::Forward),
            &mut found,
            &mut |frame| (0..=10).contains(&frame).then(|| blob(frame)),
            &stop,
            &mut |frame, _| {
                if frame == 7 {
                    stop.store(true, Ordering::Relaxed);
                }
            },
        )
        .unwrap();
        assert_eq!(frames_of(&found, 0), [5, 6, 7]);
        // A frame that cannot be had ends it, and what was found before stays.
        let mut found = Found::new(2);
        let error = follow(
            &blob_plan(Direction::Reverse),
            &mut found,
            &mut |frame| (3..=10).contains(&frame).then(|| blob(frame)),
            &AtomicBool::new(false),
            &mut |_, _| {},
        )
        .unwrap_err();
        assert_eq!(error, TrackError::MissingFrame(2));
        assert_eq!(frames_of(&found, 0), [3, 4, 5]);
        // One frame forward.
        let mut found = Found::new(2);
        follow(
            &blob_plan(Direction::OneForward),
            &mut found,
            &mut |frame| (0..=10).contains(&frame).then(|| blob(frame)),
            &AtomicBool::new(false),
            &mut |_, _| {},
        )
        .unwrap();
        assert_eq!(frames_of(&found, 0), [5, 6]);
    }

    #[test]
    fn a_track_is_of_a_window_or_of_a_tracker_and_never_of_both() {
        let ask = |more: &Value| {
            let mut request = json!({"revision": 3, "frame": 5, "direction": "forward"});
            for (name, value) in more.as_object().unwrap() {
                request[name] = value.clone();
            }
            serde_json::from_value::<Request>(request).unwrap().asked()
        };
        let window = json!({"grade": {"level": "clip", "timeline": "t1", "clip": "c1"},
            "node": "n1", "window": "w1"});
        let aim = json!({"timeline": "t1", "clip": "c1", "node": "track", "trackers": [2, 0]});
        let Ok((3, Asked::Window(asked))) = ask(&window) else {
            panic!("a window is asked")
        };
        assert_eq!(asked.window, "w1");
        let Ok((3, Asked::Fusion(asked))) = ask(&json!({"fusion": aim})) else {
            panic!("a Tracker is asked")
        };
        assert_eq!(asked.aim.trackers, Some(vec![2, 0]));
        assert_eq!((asked.frame, asked.direction), (5, Direction::Forward));
        let mut both = window.clone();
        both["fusion"] = aim.clone();
        let partial = json!({"grade": window["grade"], "node": "n1"});
        let point = json!({"fusion": aim, "tracker": "point"});
        for wrong in [both, json!({}), partial, point] {
            let Err(fault) = ask(&wrong) else {
                panic!("{wrong} is asked")
            };
            assert_eq!(fault.code, rpc::INVALID_PARAMS, "{wrong}");
            assert_eq!(reason(&fault), "fusion", "{wrong}");
        }
    }
}
