//! `track.*`: the window tracker of the Color page (T24; Reference Manual of Resolve 20,
//! chapter 140, pp. 3229–3255) run on the frames of a clip, and the edits of the Tracker palette.
//!
//! # A track
//!
//! `track.start` checks the request against the project as it is, reserves a native gesture of
//! the connection that asked, answers at once, and tracks on a thread of its own, off the
//! engine's tasks and the playback:
//!
//! - The frames are those of the clip's media, as the keys of its grade count them (its source
//!   frames, T3c), decoded on the processor (its proxy where it has a usable one), turned upright,
//!   and scaled to at most [`ANALYSIS_HEIGHT`] lines at the picture's own aspect: the frames the
//!   window's track data is in pixels of. A track goes no further than the frames that the clip
//!   shows: a clip played backward is tracked forward in time on the timeline (p. 3231).
//! - Every [`COMMIT_EVERY`] the track so far is written into the window, as the next change of
//!   the same gesture: the viewer shows the window following and the palette its curves, and the
//!   whole track is one step to undo. `track.progress` tells the frame reached.
//! - Stop (`track.stop`, Ctrl+Alt+T) ends it between frames; what was tracked stays, as does
//!   the part of a track that ended at a frame that could not be decoded (p. 3231 says a new
//!   track overwrites the frames it reaches; nothing says a stopped track is taken back).
//!   A change of the project by anyone else, Undo or Redo, or the connection going, ends it
//!   too, as a change of the project ends a drag; what it wrote before stays in the history as
//!   its own step, under what was done after it.
//! - `track.state` tells how it ended.
//!
//! Frames the tracker lost the subject at are taken off the curves, which a line bridges
//! (the core's behaviour). A clip's transition handles, timeline and group grades, and titles
//! or colours, which have no frames of a source to track, are not tracked.

use std::collections::{BTreeMap, VecDeque};
use std::path::Path;

use columbina_media::{Converter, Hardware, VideoDecoder};
use columbina_model::grade::window::tracking::{Mode, WindowTracking};
use columbina_model::grade::window::{bounds, DEFAULT_EXTENT};
use columbina_model::grade::{
    Bounds, Corrector, NodeKind, Param, Target, Window, WindowPoint, WindowShape,
};
use columbina_model::keyframes::KeyClock;
use columbina_model::{Clip, Media, Source};
use columbina_track::{
    CloudOptions, Component, Components, Direction, Job, Luma, PointOptions, Region, TrackData,
    TrackError, Tracker,
};

use super::{
    json, rpc, Arc, AtomicBool, Connection, Deserialize, Engine, Fault, Instant, Kind, Mutex,
    Ordering, OwnedGesture, RawValue, Reply, State, Value,
};
use crate::proxy;

/// The most lines of the frames tracked: tracking needs the motion, not every pixel, and a
/// frame of this size is decoded and tracked many times a second on the processor. A project
/// choice.
pub const ANALYSIS_HEIGHT: u32 = 720;

/// How often the track so far is written into the window while it goes on.
pub const COMMIT_EVERY: std::time::Duration = std::time::Duration::from_millis(150);

/// Frames decoded at once to track backward, from the earliest of them, so that a reverse track
/// does not seek for every frame.
const REVERSE_BLOCK: i64 = 12;

/// The one track of the engine that is going on.
#[derive(Default)]
pub(super) struct Tracks {
    next: u64,
    running: Option<Running>,
}

struct Running {
    job: String,
    owner: u64,
    stop: Arc<AtomicBool>,
}

fn refused(reason: &str, message: &str) -> Fault {
    Fault::new(rpc::REFUSED, message.to_owned()).with_data(json!({"reason":reason}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Start {
    grade: Target,
    node: String,
    window: String,
    revision: u64,
    /// The playhead, as the keys of the grade count frames.
    frame: i64,
    direction: Direction,
    #[serde(default)]
    tracker: TrackerKind,
    #[serde(default)]
    components: Components,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TrackerKind {
    #[default]
    Cloud,
    Point,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Stop {
    job: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EditRequest {
    grade: Target,
    node: String,
    window: String,
    revision: Option<u64>,
    edit: Edit,
}

/// An edit of the Tracker palette, each one step to undo.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Edit {
    /// Clear Selected Track Data (p. 3238): the samples from `from` to `to` of `components`
    /// (all where left out) taken away, a line bridging them.
    Clear {
        from: i64,
        to: i64,
        #[serde(default)]
        components: Option<Vec<Component>>,
    },
    /// The samples between `from` and `to` made the line between their values there.
    Interpolate {
        from: i64,
        to: i64,
        #[serde(default)]
        components: Option<Vec<Component>>,
    },
    /// Reset Track Data on Active Window (p. 3238). Frame mode's keyframes stay.
    Reset,
    /// Clip or Frame (p. 3236).
    Mode { mode: Mode },
    /// A keyframe of Frame mode at `frame`, at the motion there (p. 3252).
    AddKey { frame: i64 },
    /// Delete Keyframe (p. 3238).
    DeleteKey { frame: i64 },
    /// The crosshairs of the point tracker, in normalized picture coordinates (Add Tracker
    /// puts one at the centre of the frame, Delete Tracker takes one away, p. 3238).
    Crosshairs { points: Vec<[f64; 2]> },
    /// Paste Track Data (p. 3233) of the window `from` of the same node.
    Paste { from: String },
}

/// The corrector of `target`'s grade with the node `node`.
fn corrector<'a>(
    project: &'a columbina_model::Project,
    target: &Target,
    node: &str,
) -> Option<&'a Corrector> {
    let grade = match target {
        Target::Clip { timeline, clip } => &clip_of(project, timeline, clip)?.1.grade,
        Target::Timeline { timeline } => {
            &project.timelines.iter().find(|t| &t.id == timeline)?.grade
        }
        Target::PreClip { .. } | Target::PostClip { .. } => return None,
    };
    match &grade.nodes.iter().find(|n| n.id == node)?.kind {
        NodeKind::Corrector(c) => Some(c),
        _ => None,
    }
}

fn clip_of<'a>(
    project: &'a columbina_model::Project,
    timeline: &str,
    clip: &str,
) -> Option<(&'a columbina_model::Timeline, &'a Clip)> {
    let timeline = project.timelines.iter().find(|t| t.id == timeline)?;
    let clip = timeline
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .find(|c| c.id == clip)?;
    Some((timeline, clip))
}

/// The windows of the node with `window`'s tracking made `change` of it, as an operation.
fn set_tracking(
    project: &columbina_model::Project,
    target: &Target,
    node: &str,
    window: &str,
    change: impl FnOnce(&mut WindowTracking, &[Window]) -> Result<(), Fault>,
) -> Result<Value, Fault> {
    let c = corrector(project, target, node).ok_or_else(|| refused("target", "no such node"))?;
    let mut windows = c.windows.clone();
    let index = windows
        .iter()
        .position(|w| w.id == window)
        .ok_or_else(|| refused("window", "no such window"))?;
    let mut tracking = windows[index].tracking.clone().unwrap_or_default();
    change(&mut tracking, &c.windows)?;
    windows[index].tracking = Some(tracking);
    Ok(json!({"op":"set_node","grade":target,"id":node,"params":{"windows":windows}}))
}

/// What a track reads and where it writes, fixed when it starts.
struct Plan {
    job: String,
    owner: u64,
    kind: Kind,
    target: Target,
    node: String,
    window: String,
    media: Media,
    /// The upright frames' analysis size.
    size: [usize; 2],
    first: i64,
    last: i64,
    /// Where each source frame is on the timeline, to show it there.
    timeline: BTreeMap<i64, i64>,
    track: Job,
    data: TrackData,
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::many_single_char_names,
    clippy::too_many_lines,
    reason = "sizes of pictures within 16384, frames within the model's; one check of a request"
)]
fn plan(
    project: &columbina_model::Project,
    request: &Start,
    owner: u64,
    kind: Kind,
    job: String,
) -> Result<Plan, Fault> {
    let untrackable = |why: &str| {
        refused(
            "untrackable",
            &format!("this window cannot be tracked: {why}"),
        )
    };
    let Target::Clip { timeline, clip } = &request.grade else {
        return Err(untrackable(
            "only a clip's grade has frames of a source to track",
        ));
    };
    let (timeline, clip) =
        clip_of(project, timeline, clip).ok_or_else(|| refused("target", "no such clip"))?;
    let Source::Media { media, .. } = &clip.source else {
        return Err(untrackable("a title or a colour has no frames to track"));
    };
    let media = project
        .media
        .iter()
        .find(|m| &m.id == media)
        .ok_or_else(|| untrackable("its media is not in the project"))?;
    let video = media
        .facts
        .video
        .as_ref()
        .filter(|v| !v.still)
        .ok_or_else(|| untrackable("a still has no motion"))?;
    let c = corrector(project, &request.grade, &request.node)
        .ok_or_else(|| refused("target", "no such corrector"))?;
    let window = c
        .windows
        .iter()
        .find(|w| w.id == request.window)
        .ok_or_else(|| refused("window", "no such window"))?;
    if !window.enabled || window.shape.is_draft() {
        return Err(untrackable("the window is off or an unfinished Curve"));
    }
    // The frames: those of the media the clip shows, within the media.
    let clock = KeyClock::of(clip, project, timeline.rate);
    let mut map = BTreeMap::new();
    for t in clip.start..clip.start + clip.length.max(1) {
        map.entry(clock.at(t)).or_insert(t);
    }
    let count = {
        let d = media.facts.duration;
        let n = i128::from(d.ticks) * i128::from(video.rate.num);
        let s = i128::from(d.scale) * i128::from(video.rate.den);
        i64::try_from((n + s - 1) / s.max(1)).unwrap_or(i64::MAX)
    };
    let (Some(&lo), Some(&hi)) = (map.keys().next(), map.keys().next_back()) else {
        return Err(untrackable("the clip has no frames"));
    };
    let (first, last) = (lo.max(0), hi.min(count - 1));
    if request.frame < first || request.frame > last {
        return Err(refused(
            "frame",
            &format!(
                "frame {} is outside the clip, from {first} to {last}",
                request.frame
            ),
        ));
    }
    // The upright analysis size, at the picture's aspect.
    let (w0, h0) = if video.rotation % 180 != 0 {
        (video.height, video.width)
    } else {
        (video.width, video.height)
    };
    let h = h0.clamp(1, ANALYSIS_HEIGHT);
    let w = ((f64::from(w0) * f64::from(h) / f64::from(h0.max(1))).round() as u32).max(1);
    let size = [w as usize, h as usize];
    let (wf, hf) = (f64::from(w), f64::from(h));
    let at = request.frame as f64;
    // Where the window is at the playhead, as it is seen: its own placement, then its track.
    let own = window
        .transform
        .affine_at(at, wf / hf)
        .ok_or_else(|| untrackable("its transform collapses it"))?;
    let tracking = window.tracking.clone().unwrap_or_default();
    let shown = tracking.picture_matrix(at);
    let to_pixels = |local: [f64; 2]| {
        let [a, b, cc, d, tx, ty] = own;
        let mut uv = [
            a * local[0] + cc * local[1] + tx,
            b * local[0] + d * local[1] + ty,
        ];
        if let Some(m) = shown {
            let z = m[2][0] * uv[0] + m[2][1] * uv[1] + m[2][2];
            uv = [
                (m[0][0] * uv[0] + m[0][1] * uv[1] + m[0][2]) / z,
                (m[1][0] * uv[0] + m[1][1] * uv[1] + m[1][2]) / z,
            ];
        }
        [uv[0] * wf - 0.5, uv[1] * hf - 0.5]
    };
    let region = Region::Polygon(
        outline(&window.shape, at)
            .into_iter()
            .map(to_pixels)
            .collect(),
    );
    let mut data = tracking.data;
    if data.frame_size != size {
        if data.is_empty() && data.keys().is_empty() {
            data = TrackData::new([own[4] * wf - 0.5, own[5] * hf - 0.5], size);
        } else {
            return Err(untrackable("it was tracked on frames of another size"));
        }
    }
    let perspective = request.components.perspective;
    let tracker = match request.tracker {
        TrackerKind::Cloud => Tracker::Cloud(CloudOptions {
            perspective,
            ..CloudOptions::default()
        }),
        TrackerKind::Point => {
            // A crosshair is where its path is at the playhead, or where it was put.
            let points: Vec<[f64; 2]> = tracking
                .crosshairs
                .iter()
                .enumerate()
                .map(|(i, uv)| {
                    data.points()
                        .get(i)
                        .and_then(|path| path.get(&request.frame).copied())
                        .unwrap_or([uv[0] * wf - 0.5, uv[1] * hf - 0.5])
                })
                .collect();
            if points.is_empty() {
                return Err(refused("no_points", "the point tracker has no crosshair"));
            }
            Tracker::Point {
                points,
                options: PointOptions {
                    perspective,
                    ..PointOptions::default()
                },
            }
        }
    };
    // Forward is forward on the timeline: backward in the source of a clip played backward.
    let direction = if clip.reverse {
        match request.direction {
            Direction::Forward => Direction::Reverse,
            Direction::Reverse => Direction::Forward,
            Direction::OneForward => Direction::OneReverse,
            Direction::OneReverse => Direction::OneForward,
            Direction::ForwardAndBack => Direction::ForwardAndBack,
        }
    } else {
        request.direction
    };
    Ok(Plan {
        job,
        owner,
        kind,
        target: request.grade.clone(),
        node: request.node.clone(),
        window: request.window.clone(),
        media: media.clone(),
        size,
        first,
        last,
        timeline: map,
        track: Job {
            tracker,
            components: request.components,
            region,
            start: request.frame,
            first,
            last,
            direction,
        },
        data,
    })
}

/// The window's outline in its local coordinates at `at`, for the tracker to look within.
#[allow(clippy::many_single_char_names, reason = "the points of a cubic")]
fn outline(shape: &WindowShape, at: f64) -> Vec<[f64; 2]> {
    let value = |p: &Param, b: Bounds| p.value_at(at).clamp(b.low, b.high);
    let point = |p: &WindowPoint| {
        [
            value(&p.x, bounds::COORDINATE),
            value(&p.y, bounds::COORDINATE),
        ]
    };
    match shape {
        WindowShape::Circular { .. } => (0..48)
            .map(|i| {
                let a = f64::from(i) * std::f64::consts::TAU / 48.0;
                [DEFAULT_EXTENT * a.cos(), DEFAULT_EXTENT * a.sin()]
            })
            .collect(),
        // The white side of the boundary, as far as any picture goes.
        WindowShape::Gradient { .. } => vec![[-8.0, -8.0], [8.0, -8.0], [8.0, 0.0], [-8.0, 0.0]],
        WindowShape::Linear { points, .. } => points.iter().map(point).collect(),
        WindowShape::Polygon { points, .. } => points.iter().map(point).collect(),
        WindowShape::Curve { points, .. } => {
            let mut out = Vec::new();
            for (i, a) in points.iter().enumerate() {
                let b = &points[(i + 1) % points.len()];
                let p0 = point(&a.position);
                let p3 = point(&b.position);
                let (o, n) = (point(&a.out), point(&b.into));
                let p1 = [p0[0] + o[0], p0[1] + o[1]];
                let p2 = [p3[0] + n[0], p3[1] + n[1]];
                for k in 0..8 {
                    let t = f64::from(k) / 8.0;
                    let u = 1.0 - t;
                    let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
                    out.push([
                        w[0] * p0[0] + w[1] * p1[0] + w[2] * p2[0] + w[3] * p3[0],
                        w[0] * p0[1] + w[1] * p1[1] + w[2] * p2[1] + w[3] * p3[1],
                    ]);
                }
            }
            out
        }
    }
}

/// Frames of a media file as the tracker reads them: luma of the upright picture, at the
/// analysis size, at a fixed scale (no frame is normalized on its own).
struct Frames {
    decoder: VideoDecoder,
    converter: Converter,
    /// The size decoded at, before it is turned upright.
    decoded: (u32, u32),
    rotation: i16,
    first: i64,
    cache: VecDeque<(i64, Luma)>,
}

impl Frames {
    fn open(media: &Media, size: [usize; 2], first: i64) -> Result<Self, Fault> {
        let video = media.facts.video.as_ref().expect("a picture, checked");
        let open = |path: &Path, is_proxy: bool| {
            if is_proxy {
                VideoDecoder::open_at_rate(path, Hardware::Off, video.rate)
            } else {
                VideoDecoder::open(path, Hardware::Off)
            }
        };
        let decoder = proxy::usable(media)
            .and_then(|p| open(Path::new(&p.path), true).ok())
            .map_or_else(|| open(Path::new(&media.path), false), Ok)
            .map_err(|error| super::cannot_read(&media.path, error.reason(), &error.to_string()))?;
        #[allow(clippy::cast_possible_truncation, reason = "within 16384")]
        let (w, h) = (size[0] as u32, size[1] as u32);
        Ok(Self {
            decoder,
            converter: Converter::new(),
            decoded: if video.rotation % 180 != 0 {
                (h, w)
            } else {
                (w, h)
            },
            rotation: video.rotation,
            first,
            cache: VecDeque::new(),
        })
    }

    fn decode(&mut self, frame: i64) -> Option<Luma> {
        let decoded = self.decoder.decoded_at(frame).ok()?;
        let rgba = self
            .converter
            .rgba_scaled(&decoded, self.decoded.0, self.decoded.1)
            .ok()?;
        let (w, h) = (rgba.width as usize, rgba.height as usize);
        let luma: Vec<f32> = rgba
            .data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| {
                (0.2126 * f32::from(p[0]) + 0.7152 * f32::from(p[1]) + 0.0722 * f32::from(p[2]))
                    / 255.0
            })
            .collect();
        let (width, height, data) = match self.rotation {
            90 | 180 | 270 => {
                let (uw, uh) = if self.rotation == 180 { (w, h) } else { (h, w) };
                let mut out = vec![0.0; w * h];
                for y in 0..h {
                    for x in 0..w {
                        let (dx, dy) = match self.rotation {
                            90 => (h - 1 - y, x),
                            180 => (w - 1 - x, h - 1 - y),
                            _ => (y, w - 1 - x),
                        };
                        out[dy * uw + dx] = luma[y * w + x];
                    }
                }
                (uw, uh, out)
            }
            _ => (w, h, luma),
        };
        Luma::new(width, height, data).ok()
    }

    /// The frame `frame`; going backward, the frames before it are decoded with it.
    fn get(&mut self, frame: i64) -> Option<Luma> {
        if let Some((_, luma)) = self.cache.iter().find(|(f, _)| *f == frame) {
            return Some(luma.clone());
        }
        let backward = self.cache.front().is_some_and(|(f, _)| *f == frame + 1);
        if !backward {
            let luma = self.decode(frame)?;
            self.cache.clear();
            self.cache.push_back((frame, luma.clone()));
            return Some(luma);
        }
        self.cache.clear();
        let from = (frame - REVERSE_BLOCK + 1).max(self.first);
        for f in from..=frame {
            let luma = self.decode(f)?;
            self.cache.push_back((f, luma));
        }
        self.cache.back().map(|(_, l)| l.clone())
    }
}

impl State {
    fn track_tell(&self, owner: u64, method: &str, value: &Value) {
        if let Some(peer) = self.peers.get(&owner) {
            if peer
                .queue
                .push(rpc::notification_line(method, value))
                .is_err()
            {
                let _ = peer.stop.send(true);
            }
        }
    }

    /// Write `data` into the tracked window as the next change of the track's gesture.
    fn track_commit(
        &mut self,
        plan: &Plan,
        gesture: &mut OwnedGesture,
        data: &TrackData,
    ) -> Result<u64, Fault> {
        let now = Instant::now();
        self.gesture_last = Some(now);
        let result = self
            .apply_worked_out_locked(
                plan.kind,
                None,
                Some((plan.owner, gesture)),
                now,
                |project| {
                    Ok(vec![set_tracking(
                        project,
                        &plan.target,
                        &plan.node,
                        &plan.window,
                        |tracking, _| {
                            tracking.data = data.clone();
                            Ok(())
                        },
                    )?])
                },
            )
            .map_err(|error| match error {
                super::WorkedOutError::Input(fault) | super::WorkedOutError::Apply(fault) => fault,
            })?;
        if self.tell_at.is_some_and(|due| due <= Instant::now()) {
            self.flush();
        }
        Ok(result.revision)
    }

    /// End the track's gesture, as a drag's ends: its own if it is still current, or, where
    /// the project was changed under it, the history's gesture if it is still the track's.
    fn track_finish(&mut self, owner: u64, gesture: OwnedGesture) {
        if self.check_owned_gesture(owner, &gesture).is_ok() {
            let _ = self.finish_owned_gesture(owner, gesture);
        } else if self.history.gesture() == Some((gesture.key.as_str(), gesture.base)) {
            self.finish_gesture();
        }
    }
}

/// The phase a track ended in, and why.
fn ended(phase: &str, reason: Option<&str>, report: &Value) -> Value {
    let mut out = report.clone();
    out["phase"] = json!(phase);
    out["reason"] = json!(reason);
    out
}

#[allow(
    clippy::too_many_lines,
    reason = "one track from its frames to its end"
)]
fn run(state: &Arc<Mutex<State>>, plan: &Plan, mut gesture: OwnedGesture, stop: &Arc<AtomicBool>) {
    let opened = Frames::open(&plan.media, plan.size, plan.first);
    let unreadable = opened.as_ref().err().map(|fault| fault.message.clone());
    let mut frames = opened.ok();
    let total = match plan.track.direction {
        Direction::Forward => plan.last - plan.track.start,
        Direction::Reverse => plan.track.start - plan.first,
        Direction::ForwardAndBack => plan.last - plan.first,
        Direction::OneForward | Direction::OneReverse => 1,
    }
    .max(0);
    let mut data = plan.data.clone();
    let mut done = 0_i64;
    let mut written = None::<u64>;
    let mut stopped_by: Option<String> = None;
    let mut last_commit = Instant::now();
    let mut reached = plan.track.start;
    let mut lost = 0_usize;
    let tell = |state: &mut State,
                gesture: &mut OwnedGesture,
                data: &TrackData,
                reached: i64,
                done: i64,
                lost: usize|
     -> Result<u64, Fault> {
        let revision = state.track_commit(plan, gesture, data)?;
        let timeline = plan
            .timeline
            .range(..=reached)
            .next_back()
            .or_else(|| plan.timeline.range(reached..).next())
            .map(|(_, &t)| t);
        state.track_tell(
            plan.owner,
            "track.progress",
            &json!({"job":plan.job,"frame":reached,"timeline_frame":timeline,"done":done,
                "total":total,"lost":lost,"revision":revision}),
        );
        Ok(revision)
    };
    let outcome = match frames.as_mut() {
        None => Err(TrackError::MissingFrame(plan.track.start)),
        Some(frames) => {
            let mut read = |f: i64| {
                if stop.load(Ordering::Relaxed) {
                    return None;
                }
                frames.get(f)
            };
            columbina_track::run_watching(
                &plan.track,
                &mut data,
                &mut read,
                stop,
                &mut |frame, step, now| {
                    done += 1;
                    reached = frame;
                    if step.lost {
                        lost += 1;
                    }
                    if last_commit.elapsed() < COMMIT_EVERY {
                        return;
                    }
                    last_commit = Instant::now();
                    let mut locked = state.blocking_lock();
                    let gone = !locked.peers.contains_key(&plan.owner);
                    let result = if gone {
                        Err(refused(
                            "disconnected",
                            "the connection that asked has gone",
                        ))
                    } else {
                        tell(&mut locked, &mut gesture, now, frame, done, lost)
                    };
                    match result {
                        Ok(revision) => written = Some(revision),
                        Err(fault) => {
                            stopped_by = Some(
                                fault
                                    .data
                                    .as_ref()
                                    .and_then(|d| d["reason"].as_str())
                                    .unwrap_or("project_changed")
                                    .to_owned(),
                            );
                            stop.store(true, Ordering::Relaxed);
                        }
                    }
                },
            )
        }
    };
    let mut locked = state.blocking_lock();
    // The rest of the track, unless the project was changed under it or nothing was tracked.
    if stopped_by.is_none() && done > 0 && data != plan.data {
        match tell(&mut locked, &mut gesture, &data, reached, done, lost) {
            Ok(revision) => written = Some(revision),
            Err(fault) => {
                stopped_by = Some(
                    fault
                        .data
                        .as_ref()
                        .and_then(|d| d["reason"].as_str())
                        .unwrap_or("project_changed")
                        .to_owned(),
                );
            }
        }
    }
    let user_stop = stop.load(Ordering::Relaxed) && stopped_by.is_none();
    let report = json!({"job":plan.job,"frames":done,"lost":lost,"frame":reached,
        "revision":written.unwrap_or(locked.history.revision()),"tracked":data.tracked_range()});
    let state_value = match (&outcome, &stopped_by) {
        (_, Some(reason)) => ended("stopped", Some(reason), &report),
        (Ok(_), None) if user_stop => ended("stopped", Some("stop"), &report),
        (Ok(_), None) => ended("done", None, &report),
        (Err(TrackError::MissingFrame(_)), None) if user_stop => {
            ended("stopped", Some("stop"), &report)
        }
        (Err(error), None) => {
            let reason = match error {
                TrackError::NoFeatures | TrackError::EmptyRegion => "no_features",
                TrackError::NoPoints => "no_points",
                TrackError::MissingFrame(_) => "missing_frame",
                _ => "failed",
            };
            let reason = if unreadable.is_some() {
                "cannot_read"
            } else {
                reason
            };
            let mut v = ended("failed", Some(reason), &report);
            v["message"] = json!(unreadable.clone().unwrap_or_else(|| error.to_string()));
            v
        }
    };
    locked.track_finish(plan.owner, gesture);
    if locked
        .tracks
        .running
        .as_ref()
        .is_some_and(|r| r.job == plan.job)
    {
        locked.tracks.running = None;
    }
    locked.track_tell(plan.owner, "track.state", &state_value);
}

#[allow(
    clippy::too_many_lines,
    reason = "three RPC routes with their State lock boundaries"
)]
pub(super) async fn call(
    engine: &Engine,
    connection: &Connection,
    kind: Kind,
    id: &RawValue,
    method: &str,
    params: Option<&RawValue>,
) -> Result<Reply, Reply> {
    let result = match method {
        "track.start" => {
            let request: Start = rpc::params(
                method,
                "{grade,node,window,revision,frame,direction,tracker?,components?}",
                params,
            )?;
            let mut state = engine.state.lock().await;
            if state.tracks.running.is_some() {
                return Err(refused("busy", "a track is going on").into());
            }
            if request.revision != state.history.revision() {
                return Err(
                    super::stale_revision(request.revision, state.history.revision()).into(),
                );
            }
            state.tracks.next += 1;
            let job = format!("track-{}", state.tracks.next);
            let plan = plan(
                state.history.project(),
                &request,
                connection.id,
                kind,
                job.clone(),
            )?;
            let gesture = state.begin_owned_gesture(connection.id, request.revision)?;
            let stop = Arc::new(AtomicBool::new(false));
            state.tracks.running = Some(Running {
                job: job.clone(),
                owner: connection.id,
                stop: Arc::clone(&stop),
            });
            let answer = json!({"job":job,"first":plan.first,"last":plan.last,
                "frame_size":plan.size,"direction":plan.track.direction});
            drop(state);
            let shared = Arc::clone(&engine.state);
            tokio::task::spawn_blocking(move || run(&shared, &plan, gesture, &stop));
            answer
        }
        "track.stop" => {
            let request: Stop = rpc::params(method, "{job}", params)?;
            let state = engine.state.lock().await;
            let running = state
                .tracks
                .running
                .as_ref()
                .filter(|r| r.job == request.job && r.owner == connection.id)
                .ok_or_else(|| refused("job", "no such track is going on"))?;
            running.stop.store(true, Ordering::Relaxed);
            json!({})
        }
        "track.edit" => {
            let request: EditRequest = rpc::params(
                method,
                "{grade,node,window,revision?,edit:{kind,...}}",
                params,
            )?;
            let mut state = engine.state.lock().await;
            let applied = state.apply_worked_out_locked(
                kind,
                request.revision,
                None,
                Instant::now(),
                |project| Ok(vec![edit(project, &request)?]),
            );
            let result = match applied {
                Ok(result) => result,
                Err(super::WorkedOutError::Input(fault) | super::WorkedOutError::Apply(fault)) => {
                    return Err(fault.into());
                }
            };
            let line = rpc::result_line(id, &json!({"revision":result.revision}));
            return Ok(engine.finish(state, connection, line));
        }
        _ => unreachable!("track routes"),
    };
    Ok(match connection.queue(rpc::result_line(id, &result)) {
        Ok(()) => Reply::Queued,
        Err(refused) => Reply::Refused(refused),
    })
}

fn components(which: Option<&Vec<Component>>) -> Vec<Component> {
    which.map_or_else(|| Component::ALL.to_vec(), Clone::clone)
}

fn edit(project: &columbina_model::Project, request: &EditRequest) -> Result<Value, Fault> {
    set_tracking(
        project,
        &request.grade,
        &request.node,
        &request.window,
        |tracking, windows| {
            let data = &mut tracking.data;
            match &request.edit {
                Edit::Clear {
                    from,
                    to,
                    components: which,
                } => data.clear_range(*from, *to, &components(which.as_ref())),
                Edit::Interpolate {
                    from,
                    to,
                    components: which,
                } => data.interpolate_range(*from, *to, &components(which.as_ref())),
                Edit::Reset => data.reset(),
                Edit::Mode { mode } => tracking.mode = *mode,
                Edit::AddKey { frame } => {
                    if data.frame_size.contains(&0) {
                        return Err(refused(
                            "untracked",
                            "the window has no track to key: track it first",
                        ));
                    }
                    let here = data.key_at(*frame);
                    data.set_key(*frame, here);
                }
                Edit::DeleteKey { frame } => {
                    data.delete_key(*frame);
                }
                Edit::Crosshairs { points } => tracking.crosshairs.clone_from(points),
                Edit::Paste { from } => {
                    let source = windows
                        .iter()
                        .find(|w| &w.id == from)
                        .and_then(|w| w.tracking.as_ref())
                        .filter(|t| !t.data.is_empty())
                        .ok_or_else(|| refused("nothing_to_paste", "that window has no track"))?;
                    if data.frame_size.contains(&0) {
                        // A window never tracked takes the source's frames and its own centre.
                        *data = TrackData::new(source.data.anchor, source.data.frame_size);
                    }
                    data.paste(&source.data);
                }
            }
            Ok(())
        },
    )
}
