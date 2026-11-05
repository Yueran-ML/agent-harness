//! `audio.*` (plan T9b): the meters of the timeline as it plays (`audio.meters`, `audio.loudness`;
//! the board is [`crate::meters`]), and the analyses that are run off the threads of the sound, as
//! fast as the clips can be read: `audio.analyze` (Analyze Audio Level, p. 3986–3987, and the
//! loudness of a track, a bus or the mix), `audio.normalize` (Normalize Audio Levels,
//! p. 3840–3841) and `audio.level_match` (Level Matcher, p. 3804–3805), each a job that answers
//! at once with its name, tells how far it is (`audio.analysis`, `running`) and how it ended
//! (`done` with what it found, `failed`, `cancelled`); `audio.cancel` stops one.
//!
//! What is measured (`columbina_audio::analysis`): a track or a bus after its fader, over a range
//! of the timeline or the whole of it; a clip as its track takes it, after its Volume and before
//! its pan (through its equalizer and its effects), so that the gain found is what its Volume is
//! moved by, and a clip normalized again is moved by nothing. Normalize
//! and Level Matcher change only the Volume of the clips, the whole curve of each moved by its
//! gain (`nudge_clip_gain` of T2, one edit for each clip, all in one transaction: one step to
//! undo), and only where they are asked to (`apply`); a clip that has no level (silence, or too
//! short for a loudness) is left as it is.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use columbina_audio::analysis::{
    alone, level_match, normalize_clips, run_strips, Analysis, Analyzer, Level, SetLevel, Volume,
};
use columbina_audio::constants::{CLIP_VOLUME_RANGE, NORMALIZE_PEAK_DEFAULT, NORMALIZE_RANGE};
use columbina_audio::graph::{Graph, Options, Owner, Point, Purpose};
use columbina_audio::node::Layout;
use columbina_audio::standards::{self, Measure, Standard, Verdict};
use columbina_model::{Clip, Project, Timeline, TrackFormat, TrackKind};
use serde::Deserialize;
use serde_json::{json, value::RawValue, Value};
use tokio::sync::Mutex;

use super::{log_kicked, Connection, Engine, Kind, Limits, Reply, State, WorkedOutError};
use crate::meters::{self, decibels, standard_of, Action};
use crate::play::mix::Feed;
use crate::rpc::{self, Fault};

/// The most samples that the graph of an analysis processes at once.
const MAX_BLOCK: usize = 4096;

/// The most analyses that run at once (we decide): each reads its clips as fast as it can.
const MOST_JOBS: usize = 4;

/// The most clips or targets of one analysis (we decide).
const MOST_TARGETS: usize = 256;

/// The least gain that Normalize or Level Matcher applies, in dB (we decide): what a clip that is
/// at its level already is measured off by.
const SMALLEST_GAIN: f64 = 0.001;

/// How often a job tells how far it is, at the most.
const TELL_EVERY: Duration = Duration::from_millis(250);

/// The analyses that run, each with what stops it.
#[derive(Default)]
struct Jobs {
    next: AtomicU64,
    running: std::sync::Mutex<HashMap<String, Arc<AtomicBool>>>,
}

fn jobs() -> &'static Jobs {
    static JOBS: OnceLock<Jobs> = OnceLock::new();
    JOBS.get_or_init(Jobs::default)
}

impl Jobs {
    fn running(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<AtomicBool>>> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// What is measured.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum Target {
    Clip(String),
    Track(String),
    Bus(String),
}

impl Target {
    fn json(&self) -> Value {
        match self {
            Target::Clip(id) => json!({"clip": id}),
            Target::Track(id) => json!({"track": id}),
            Target::Bus(id) => json!({"bus": id}),
        }
    }
}

/// A range of frames of the timeline, `to` not included.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Frames {
    from: i64,
    to: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Metering {
    timeline: String,
    #[serde(default)]
    reset: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Loudness {
    timeline: String,
    #[serde(default = "read")]
    action: Action,
}

fn read() -> Action {
    Action::Read
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Analyze {
    timeline: String,
    targets: Vec<Target>,
    #[serde(default)]
    range: Option<Frames>,
    #[serde(default)]
    standard: Option<String>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Mode {
    Relative,
    Independent,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Normalize {
    timeline: String,
    clips: Vec<String>,
    #[serde(default)]
    level: Option<String>,
    #[serde(default)]
    target: Option<f64>,
    #[serde(default)]
    mode: Option<Mode>,
    #[serde(default)]
    apply: bool,
    #[serde(default)]
    revision: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Match {
    timeline: String,
    reference: String,
    clips: Vec<String>,
    #[serde(default)]
    apply: bool,
    #[serde(default)]
    revision: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cancel {
    job: String,
}

/// A refusal of the params of `audio.*`, with its reason.
fn refused(message: impl Into<String>, data: Value) -> Fault {
    Fault::params(message).with_data(data)
}

/// The timeline `id` of `project`, and its place.
fn timeline_of<'a>(project: &'a Project, id: &str) -> Result<(usize, &'a Timeline), Fault> {
    project
        .timelines
        .iter()
        .enumerate()
        .find(|(_, t)| t.id == id)
        .ok_or_else(|| {
            refused(
                format!("there is no timeline {id} in the project"),
                json!({"reason": "missing", "timeline": id}),
            )
        })
}

/// The clip `id` of a track of sound of `timeline`, and the place of its track.
fn clip_of<'a>(timeline: &'a Timeline, id: &str) -> Result<(usize, &'a Clip), Fault> {
    timeline
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, track)| track.kind == TrackKind::Audio)
        .find_map(|(at, track)| track.clips.iter().find(|c| c.id == id).map(|c| (at, c)))
        .ok_or_else(|| {
            refused(
                format!("there is no clip {id} on a track of sound of the timeline"),
                json!({"reason": "missing", "what": "clip", "id": id}),
            )
        })
}

/// The standard `id`.
fn standard_named(id: &str) -> Result<&'static Standard, Fault> {
    standards::by_id(id).ok_or_else(|| {
        refused(
            format!("there is no standard of loudness {id}"),
            json!({"reason": "standard", "standard": id}),
        )
    })
}

/// The clips named, at most [`MOST_TARGETS`] and each once.
fn clips_named(timeline: &Timeline, clips: &[String]) -> Result<(), Fault> {
    if clips.is_empty() || clips.len() > MOST_TARGETS {
        return Err(refused(
            format!("from 1 to {MOST_TARGETS} clips are measured"),
            json!({"reason": "out_of_range", "what": "clips"}),
        ));
    }
    for (at, clip) in clips.iter().enumerate() {
        clip_of(timeline, clip)?;
        if clips[..at].contains(clip) {
            return Err(refused(
                format!("clip {clip} is named twice"),
                json!({"reason": "twice", "clip": clip}),
            ));
        }
    }
    Ok(())
}

/// `audio.*`.
pub(super) async fn call(
    engine: &Engine,
    connection: &Connection,
    kind: Kind,
    id: &RawValue,
    method: &str,
    params: Option<&RawValue>,
) -> Result<Reply, Reply> {
    let result = match method {
        "audio.meters" => {
            let request: Metering = rpc::params(method, r#"{"timeline": "t1"}"#, params)?;
            let audio = settings(engine, &request.timeline).await?;
            meters::hub().shown(&request.timeline, &audio, request.reset)
        }
        "audio.loudness" => {
            let request: Loudness = rpc::params(
                method,
                r#"{"timeline": "t1", "action"?: "start" | "pause" | "reset"}"#,
                params,
            )?;
            let audio = settings(engine, &request.timeline).await?;
            meters::hub()
                .control(&request.timeline, &audio, request.action)
                .ok_or_else(|| {
                    Fault::new(
                        rpc::REFUSED,
                        format!(
                            "timeline {} is not what the meters measure",
                            request.timeline
                        ),
                    )
                    .with_data(json!({"reason": "not_metered", "timeline": request.timeline}))
                })?
        }
        "audio.cancel" => {
            let request: Cancel = rpc::params(method, r#"{"job": "audio-1"}"#, params)?;
            let Some(stop) = jobs().running().get(&request.job).cloned() else {
                return Err(refused(
                    format!("there is no analysis {} running", request.job),
                    json!({"reason": "missing", "job": request.job}),
                )
                .into());
            };
            stop.store(true, Ordering::Release);
            json!({})
        }
        "audio.analyze" | "audio.normalize" | "audio.level_match" => {
            let project = engine.state.lock().await.history.project().clone();
            let work = prepare(method, params, &project)?;
            return start(engine, connection, kind, id, Arc::new(project), work);
        }
        _ => {
            return Err(Fault::new(rpc::METHOD_NOT_FOUND, format!("no method {method}")).into());
        }
    };
    Ok(match connection.queue(rpc::result_line(id, &result)) {
        Ok(()) => Reply::Queued,
        Err(refused) => Reply::Refused(refused),
    })
}

/// The settings of the sound of timeline `id`.
async fn settings(engine: &Engine, id: &str) -> Result<columbina_model::TimelineAudio, Fault> {
    let state = engine.state.lock().await;
    let (_, timeline) = timeline_of(state.history.project(), id)?;
    Ok(timeline.audio.clone())
}

/// An analysis checked, ready to run.
enum Work {
    Analyze {
        timeline: usize,
        targets: Vec<Target>,
        range: Option<Range<i64>>,
        standard: &'static Standard,
        target: f32,
    },
    Normalize {
        timeline: usize,
        clips: Vec<String>,
        level: (String, Level),
        target: f64,
        mode: Mode,
        apply: Option<Apply>,
    },
    Match {
        timeline: usize,
        reference: String,
        clips: Vec<String>,
        apply: Option<Apply>,
    },
}

/// Whether the gains found are applied, and to which revision the transaction is bound.
#[derive(Clone, Copy, Debug)]
struct Apply(Option<u64>);

/// Checks the params of `method` against `project`.
fn prepare(method: &str, params: Option<&RawValue>, project: &Project) -> Result<Work, Fault> {
    match method {
        "audio.analyze" => analyze_work(method, params, project),
        "audio.normalize" => normalize_work(method, params, project),
        _ => match_work(method, params, project),
    }
}

/// `audio.analyze` checked.
fn analyze_work(method: &str, params: Option<&RawValue>, project: &Project) -> Result<Work, Fault> {
    let request: Analyze = rpc::params(
        method,
        r#"{"timeline": "t1", "targets": [{"clip": "c1"} | {"track": "A1"} | {"bus": "B1"}], "range"?: {"from": 0, "to": 250}, "standard"?: "ebu-r128"}"#,
        params,
    )?;
    let (at, timeline) = timeline_of(project, &request.timeline)?;
    if request.targets.is_empty() || request.targets.len() > MOST_TARGETS {
        return Err(refused(
            format!("from 1 to {MOST_TARGETS} targets are measured"),
            json!({"reason": "out_of_range", "what": "targets"}),
        ));
    }
    for (at, target) in request.targets.iter().enumerate() {
        if request.targets[..at].contains(target) {
            return Err(refused(
                "a target is named twice",
                json!({"reason": "twice", "target": target.json()}),
            ));
        }
        match target {
            Target::Clip(clip) => {
                clip_of(timeline, clip)?;
            }
            Target::Track(track) => {
                let found = timeline
                    .tracks
                    .iter()
                    .find(|t| t.id == *track && t.kind == TrackKind::Audio)
                    .ok_or_else(|| {
                        refused(
                            format!("there is no track of sound {track}"),
                            json!({"reason": "missing", "what": "track", "id": track}),
                        )
                    })?;
                if !found.enabled {
                    return Err(refused(
                        format!("track {track} is off: it is not in the mix"),
                        json!({"reason": "disabled", "track": track}),
                    ));
                }
            }
            Target::Bus(bus) => {
                if !timeline.buses.iter().any(|b| b.id == *bus) {
                    return Err(refused(
                        format!("there is no bus {bus}"),
                        json!({"reason": "missing", "what": "bus", "id": bus}),
                    ));
                }
            }
        }
    }
    let range = match request.range {
        None => None,
        Some(Frames { from, to }) if from >= 0 && to > from => {
            Some(sample_at(timeline, from)..sample_at(timeline, to))
        }
        Some(_) => {
            return Err(refused(
                "a range is from a frame to a later one, from 0",
                json!({"reason": "out_of_range", "what": "range"}),
            ))
        }
    };
    let standard = match &request.standard {
        Some(id) => standard_named(id)?,
        None => standard_of(timeline.audio.loudness.standard),
    };
    #[allow(clippy::cast_possible_truncation, reason = "a loudness in LUFS")]
    let target = standard.target_or(timeline.audio.loudness.target as f32);
    Ok(Work::Analyze {
        timeline: at,
        targets: request.targets,
        range,
        standard,
        target,
    })
}

/// `audio.normalize` checked.
fn normalize_work(
    method: &str,
    params: Option<&RawValue>,
    project: &Project,
) -> Result<Work, Fault> {
    let request: Normalize = rpc::params(
        method,
        r#"{"timeline": "t1", "clips": ["c1", ...], "level"?: "sample_peak" | "true_peak" | "<standard>", "target"?: -9, "mode"?: "relative" | "independent", "apply"?: false}"#,
        params,
    )?;
    let (at, timeline) = timeline_of(project, &request.timeline)?;
    clips_named(timeline, &request.clips)?;
    let name = request.level.unwrap_or_else(|| "sample_peak".to_owned());
    let (level, default) = match name.as_str() {
        "sample_peak" => (Level::SamplePeak, f64::from(NORMALIZE_PEAK_DEFAULT)),
        "true_peak" => (Level::TruePeak, f64::from(NORMALIZE_PEAK_DEFAULT)),
        id => {
            let standard = standard_named(id)?;
            #[allow(clippy::cast_possible_truncation, reason = "a loudness in LUFS")]
            let target = standard.target_or(timeline.audio.loudness.target as f32);
            (Level::of(standard), f64::from(target))
        }
    };
    let target = request.target.unwrap_or(default);
    let (low, high) = NORMALIZE_RANGE;
    if !(f64::from(low)..=f64::from(high)).contains(&target) {
        return Err(refused(
            format!("the target level is from {low} to {high}"),
            json!({"reason": "out_of_range", "what": "target"}),
        ));
    }
    Ok(Work::Normalize {
        timeline: at,
        clips: request.clips,
        level: (name, level),
        target,
        mode: request.mode.unwrap_or(Mode::Relative),
        apply: request.apply.then_some(Apply(request.revision)),
    })
}

/// `audio.level_match` checked.
fn match_work(method: &str, params: Option<&RawValue>, project: &Project) -> Result<Work, Fault> {
    let request: Match = rpc::params(
        method,
        r#"{"timeline": "t1", "reference": "c1", "clips": ["c2", ...], "apply"?: false}"#,
        params,
    )?;
    let (at, timeline) = timeline_of(project, &request.timeline)?;
    clip_of(timeline, &request.reference)?;
    clips_named(timeline, &request.clips)?;
    Ok(Work::Match {
        timeline: at,
        reference: request.reference,
        clips: request.clips,
        apply: request.apply.then_some(Apply(request.revision)),
    })
}

/// The sample of the timeline at the start of frame `frame`, to the sample below (as an export).
fn sample_at(timeline: &Timeline, frame: i64) -> i64 {
    i64::try_from(
        i128::from(frame) * i128::from(timeline.sample_rate) * i128::from(timeline.rate.den)
            / i128::from(timeline.rate.num),
    )
    .unwrap_or(i64::MAX)
}

/// Starts `work` as a job: its name answered, then the job run on a thread of its own.
fn start(
    engine: &Engine,
    connection: &Connection,
    kind: Kind,
    id: &RawValue,
    project: Arc<Project>,
    work: Work,
) -> Result<Reply, Reply> {
    let stop = Arc::new(AtomicBool::new(false));
    let job = {
        let mut running = jobs().running();
        if running.len() >= MOST_JOBS {
            return Err(Fault::new(
                rpc::REFUSED,
                format!("{MOST_JOBS} analyses are running already"),
            )
            .with_data(json!({"reason": "busy"}))
            .into());
        }
        let job = format!("audio-{}", jobs().next.fetch_add(1, Ordering::Relaxed) + 1);
        running.insert(job.clone(), Arc::clone(&stop));
        job
    };
    let reply = match connection.queue(rpc::result_line(id, &json!({"job": job}))) {
        Ok(()) => Reply::Queued,
        Err(refused) => Reply::Refused(refused),
    };
    let teller = Teller {
        state: Arc::clone(&engine.state),
        limits: engine.limits,
        job: job.clone(),
    };
    let started = std::thread::Builder::new()
        .name("columbina-analysis".into())
        .spawn({
            let job = job.clone();
            move || {
                let ended = run(&teller, kind, &project, work, &stop);
                jobs().running().remove(&job);
                teller.tell(&ended);
            }
        });
    if let Err(error) = started {
        jobs().running().remove(&job);
        crate::log::log(
            "analysis_failed",
            &json!({"job": job, "message": error.to_string()}),
        );
    }
    Ok(reply)
}

/// What tells every client of a job.
struct Teller {
    state: Arc<Mutex<State>>,
    limits: Limits,
    job: String,
}

impl Teller {
    /// Tells `audio.analysis` with `what`, the job's name added.
    fn tell(&self, what: &Value) {
        let mut what = what.clone();
        what["job"] = json!(self.job);
        let mut state = self.state.blocking_lock();
        state.broadcast(&rpc::notification_line("audio.analysis", &what));
        let kicked = std::mem::take(&mut state.kicked);
        drop(state);
        for (id, kind) in kicked {
            log_kicked(id, Some(kind), self.limits);
        }
    }

    /// Applies `edits` as one transaction made by `kind`, bound to `revision` where there is one:
    /// what `transaction.apply` answers, or why it was refused.
    fn apply(&self, kind: Kind, revision: Option<u64>, edits: Vec<Value>) -> Value {
        let mut state = self.state.blocking_lock();
        let result =
            state.apply_changes_locked(kind, revision, None, Instant::now(), |_| Ok(edits));
        let kicked = std::mem::take(&mut state.kicked);
        drop(state);
        for (id, kind) in kicked {
            log_kicked(id, Some(kind), self.limits);
        }
        match result {
            Ok(result) => json!({"applied": result}),
            Err(WorkedOutError::Input(fault) | WorkedOutError::Apply(fault)) => {
                json!({"applied": null, "refused": fault})
            }
        }
    }
}

/// How far a job is: samples measured of all it measures; it tells it every [`TELL_EVERY`].
struct Progress<'a> {
    teller: &'a Teller,
    stop: &'a AtomicBool,
    rate: u32,
    done: u64,
    total: u64,
    told: Option<Instant>,
}

impl Progress<'_> {
    /// `frames` more measured; whether to go on.
    fn more(&mut self, frames: usize) -> bool {
        self.done += u64::try_from(frames).unwrap_or(0);
        if self.told.is_none_or(|told| told.elapsed() >= TELL_EVERY) {
            self.told = Some(Instant::now());
            #[allow(clippy::cast_precision_loss, reason = "seconds told")]
            let seconds =
                |samples: u64| (samples as f64 / f64::from(self.rate) * 100.0).round() / 100.0;
            self.teller.tell(&json!({
                "state": "running",
                "done": seconds(self.done),
                "total": seconds(self.total),
            }));
        }
        !self.stop.load(Ordering::Acquire)
    }
}

/// What a job measures of one or more targets in one run of one graph, built only when it is to
/// run and dropped once it has run (the review of T9b): its graph, the points measured, its range
/// of the timeline, and the project it reads.
struct Pass {
    graph: Graph,
    project: Arc<Project>,
    points: Vec<(usize, Point)>,
    range: Range<i64>,
}

impl Pass {
    /// The measure of clip `clip` (see the module).
    fn clip(project: &Project, timeline: usize, clip: &str) -> Result<Pass, String> {
        let (project, track, range) =
            alone(project, timeline, clip).ok_or_else(|| format!("there is no clip {clip}"))?;
        let graph = Graph::build(
            &project,
            timeline,
            Options::new(MAX_BLOCK, Purpose::Analysis),
        )
        .map_err(|error| error.to_string())?;
        let strip = graph
            .strip_of(&Owner::Track(track))
            .ok_or_else(|| format!("the track of clip {clip} is not in the mix"))?;
        Ok(Pass {
            graph,
            project: Arc::new(project),
            points: vec![(strip, Point::BeforeFader)],
            range,
        })
    }

    /// The measure of the tracks and buses `owners`, after their faders, in one graph over
    /// `range`.
    fn strips(
        project: &Arc<Project>,
        timeline: usize,
        owners: &[Owner],
        range: Range<i64>,
    ) -> Result<Pass, String> {
        let graph = Graph::build(
            project,
            timeline,
            Options::new(MAX_BLOCK, Purpose::Analysis),
        )
        .map_err(|error| error.to_string())?;
        let points = owners
            .iter()
            .map(|owner| {
                graph
                    .strip_of(owner)
                    .map(|strip| (strip, Point::AfterFader))
                    .ok_or_else(|| format!("{owner:?} is not in the mix"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Pass {
            graph,
            project: Arc::clone(project),
            points,
            range,
        })
    }

    /// An analyzer of each of its points.
    fn analyzers(&self, true_peak: bool) -> Vec<Analyzer> {
        let rate = self.graph.rate();
        self.points
            .iter()
            .map(|&(strip, _)| Analyzer::new(rate, self.graph.layout_of(strip), true_peak))
            .collect()
    }

    /// Runs it, giving the sound of each point to its analyzer, and that of the first to `also`
    /// where there is one; what they found, `None` where it was stopped.
    fn run(
        mut self,
        timeline: usize,
        mut analyzers: Vec<Analyzer>,
        mut also: Option<&mut Analyzer>,
        progress: &mut Progress,
    ) -> Result<Option<Vec<Analysis>>, String> {
        let mut feed = Feed::new(
            Arc::clone(&self.project),
            timeline,
            self.graph.clips(),
            false,
        );
        let ran = run_strips(
            &mut self.graph,
            &mut feed,
            &self.points,
            self.range.clone(),
            &mut |at, sound| {
                analyzers[at].take(sound);
                if at > 0 {
                    return true;
                }
                if let Some(also) = also.as_deref_mut() {
                    also.take(sound);
                }
                progress.more(sound.first().map_or(0, |channel| channel.len()))
            },
        );
        match ran {
            Ok(true) => Ok(Some(analyzers.into_iter().map(Analyzer::finish).collect())),
            Ok(false) => Ok(None),
            Err(_) => Err(feed.take_error()),
        }
    }
}

/// The samples of the timeline of clip `clip`, as [`alone`] measures it.
fn clip_range(timeline: &Timeline, clip: &str) -> Range<i64> {
    clip_of(timeline, clip).map_or(0..0, |(_, c)| {
        sample_at(timeline, c.start)..sample_at(timeline, c.end())
    })
}

/// The channels of the track of clip `clip`.
fn clip_layout(timeline: &Timeline, clip: &str) -> Layout {
    clip_of(timeline, clip).map_or(Layout::Stereo, |(at, _)| {
        let audio = &timeline.tracks[at].audio;
        match audio.format {
            TrackFormat::Mono => Layout::Mono,
            TrackFormat::Stereo => Layout::Stereo,
            TrackFormat::FiveOne => Layout::Surround51,
            TrackFormat::Adaptive => Layout::discrete(usize::from(audio.channels)),
        }
    })
}

/// Samples in a range.
fn frames(range: &Range<i64>) -> u64 {
    u64::try_from(range.end - range.start).unwrap_or(0)
}

/// What an analysis found, by `level` of a standard: the integrated loudness as it measures.
fn found(analysis: &Analysis, measure: Measure) -> Value {
    let integrated = match measure {
        Measure::Gated => analysis.integrated,
        Measure::Ungated => analysis.integrated_ungated,
    };
    json!({
        "integrated": integrated.map_or(Value::Null, decibels),
        "range": analysis.range.map_or(Value::Null, decibels),
        "momentary_max": decibels(analysis.momentary_max),
        "short_term_max": decibels(analysis.short_term_max),
        "true_peak": decibels(analysis.true_peak),
        "sample_peak": decibels(analysis.sample_peak),
        "seconds": (analysis.seconds * 100.0).round() / 100.0,
    })
}

/// Runs `work`: how it ended, as `audio.analysis` tells it.
fn run(
    teller: &Teller,
    kind: Kind,
    project: &Arc<Project>,
    work: Work,
    stop: &AtomicBool,
) -> Value {
    let ended = match work {
        Work::Analyze {
            timeline,
            targets,
            range,
            standard,
            target,
        } => analyze(
            teller,
            project,
            (timeline, &targets, range),
            (standard, target),
            stop,
        ),
        Work::Normalize {
            timeline,
            clips,
            level,
            target,
            mode,
            apply,
        } => normalize(
            teller,
            kind,
            project,
            (timeline, &clips),
            (level, target, mode),
            apply,
            stop,
        ),
        Work::Match {
            timeline,
            reference,
            clips,
            apply,
        } => matched(
            teller,
            kind,
            project,
            (timeline, &reference, &clips),
            apply,
            stop,
        ),
    };
    match ended {
        Ok(Some(done)) => done,
        Ok(None) => json!({"state": "cancelled"}),
        Err(message) => json!({"state": "failed", "message": message}),
    }
}

fn progress<'a>(teller: &'a Teller, stop: &'a AtomicBool, rate: u32, total: u64) -> Progress<'a> {
    Progress {
        teller,
        stop,
        rate,
        done: 0,
        total,
        told: None,
    }
}

/// `audio.analyze`: the tracks and buses in one pass, then each clip in a pass of its own, each
/// pass built as it is to run (`stop` read before it is built) and dropped after.
fn analyze(
    teller: &Teller,
    project: &Arc<Project>,
    (timeline, targets, range): (usize, &[Target], Option<Range<i64>>),
    (standard, target): (&'static Standard, f32),
    stop: &AtomicBool,
) -> Result<Option<Value>, String> {
    let t = &project.timelines[timeline];
    let rate = t.sample_rate;
    let range = range.unwrap_or_else(|| 0..crate::play::mix::end_of(t));
    let owners: Vec<(usize, Owner)> = targets
        .iter()
        .enumerate()
        .filter_map(|(at, what)| match what {
            Target::Track(id) => Some((at, Owner::Track(id.clone()))),
            Target::Bus(id) => Some((at, Owner::Bus(id.clone()))),
            Target::Clip(_) => None,
        })
        .collect();
    let clips: u64 = targets
        .iter()
        .map(|what| match what {
            Target::Clip(clip) => frames(&clip_range(t, clip)),
            _ => 0,
        })
        .sum();
    let strips = if owners.is_empty() { 0 } else { frames(&range) };
    let mut told = progress(teller, stop, rate, clips + strips);
    let mut found_of: Vec<Option<Analysis>> = vec![None; targets.len()];
    if !owners.is_empty() {
        if stop.load(Ordering::Acquire) {
            return Ok(None);
        }
        let only: Vec<Owner> = owners.iter().map(|(_, owner)| owner.clone()).collect();
        let pass = Pass::strips(project, timeline, &only, range)?;
        let analyzers = pass.analyzers(true);
        let Some(analyses) = pass.run(timeline, analyzers, None, &mut told)? else {
            return Ok(None);
        };
        for ((at, _), analysis) in owners.iter().zip(analyses) {
            found_of[*at] = Some(analysis);
        }
    }
    for (at, what) in targets.iter().enumerate() {
        let Target::Clip(clip) = what else { continue };
        if stop.load(Ordering::Acquire) {
            return Ok(None);
        }
        let pass = Pass::clip(project, timeline, clip)?;
        let analyzers = pass.analyzers(true);
        let Some(mut analyses) = pass.run(timeline, analyzers, None, &mut told)? else {
            return Ok(None);
        };
        found_of[at] = analyses.pop();
    }
    let mut results = Vec::new();
    for (what, analysis) in targets.iter().zip(found_of) {
        let analysis = analysis.ok_or("a target was not measured")?;
        let mut result = found(&analysis, standard.measure);
        result["target"] = what.json();
        let integrated = match standard.measure {
            Measure::Gated => analysis.integrated,
            Measure::Ungated => analysis.integrated_ungated,
        };
        result["verdict"] =
            integrated
                .filter(|lufs| lufs.is_finite())
                .map_or(Value::Null, |lufs| {
                    json!(match standard.verdict(lufs, target) {
                        Verdict::Low => "low",
                        Verdict::Within => "within",
                        Verdict::High => "high",
                    })
                });
        results.push(result);
    }
    Ok(Some(json!({
        "state": "done",
        "method": "audio.analyze",
        "standard": standard.id,
        "level": decibels(f64::from(target)),
        "results": results,
    })))
}

/// Measures each clip of `clips` in a pass of its own, in the order of the timeline, each pass
/// built as it is to run (`stop` read before it is built) and dropped after; and, where
/// `together` is given, all of them one after another into it. What each was found, `None` where
/// the job was stopped.
fn clips_measured(
    project: &Arc<Project>,
    timeline: usize,
    clips: &[String],
    (true_peak, mut together): (bool, Option<&mut Analyzer>),
    told: &mut Progress,
) -> Result<Option<Vec<Analysis>>, String> {
    let t = &project.timelines[timeline];
    let mut each: Vec<Option<Analysis>> = vec![None; clips.len()];
    for at in in_order(t, clips) {
        if told.stop.load(Ordering::Acquire) {
            return Ok(None);
        }
        let pass = Pass::clip(project, timeline, &clips[at])?;
        let analyzers = pass.analyzers(true_peak);
        let Some(mut found) = pass.run(timeline, analyzers, together.as_deref_mut(), told)? else {
            return Ok(None);
        };
        each[at] = found.pop();
    }
    Ok(Some(each.into_iter().flatten().collect()))
}

/// The clips of `clips` in the order of the timeline (by where they start, then by their track).
fn in_order(timeline: &Timeline, clips: &[String]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..clips.len()).collect();
    let place = |id: &str| {
        timeline
            .tracks
            .iter()
            .enumerate()
            .find_map(|(at, track)| {
                track
                    .clips
                    .iter()
                    .find(|c| c.id == id)
                    .map(|c| (c.start, at))
            })
            .unwrap_or((i64::MAX, usize::MAX))
    };
    order.sort_by_key(|&at| place(&clips[at]));
    order
}

/// The Volume of each clip of `clips`.
fn volumes(timeline: &Timeline, clips: &[String]) -> Vec<Volume> {
    clips
        .iter()
        .map(|id| {
            clip_of(timeline, id).map_or(Volume::fixed(0.0), |(_, clip)| Volume::of(&clip.gain_db))
        })
        .collect()
}

/// The edits that move the Volume of each clip by its gain, none where it has none or it is 0.
fn nudges(timeline: &str, gains: &[(String, Option<f64>)]) -> Vec<Value> {
    gains
        .iter()
        .filter_map(|(clip, gain)| {
            // A gain under a thousandth of a decibel is none (a clip normalized again).
            gain.filter(|gain| gain.abs() >= SMALLEST_GAIN).map(|gain| {
                json!({"edit": "nudge_clip_gain", "timeline": timeline, "clips": [clip],
                       "delta_db": gain, "linked": false})
            })
        })
        .collect()
}

/// `audio.normalize`.
fn normalize(
    teller: &Teller,
    kind: Kind,
    project: &Arc<Project>,
    (timeline, clips): (usize, &[String]),
    ((name, level), target, mode): ((String, Level), f64, Mode),
    apply: Option<Apply>,
    stop: &AtomicBool,
) -> Result<Option<Value>, String> {
    let t = &project.timelines[timeline];
    let rate = t.sample_rate;
    // The clips together in one analysis, as one programme (Relative): of the layout of the most
    // channels among them; a clip of fewer is heard on the first of its channels.
    let layout = clips
        .iter()
        .map(|clip| clip_layout(t, clip))
        .max_by_key(|layout| layout.channels())
        .unwrap_or(Layout::Stereo);
    let mut together = Analyzer::new(rate, layout, level.needs_true_peak());
    let total = clips.iter().map(|clip| frames(&clip_range(t, clip))).sum();
    let mut told = progress(teller, stop, rate, total);
    let Some(each) = clips_measured(
        project,
        timeline,
        clips,
        (level.needs_true_peak(), Some(&mut together)),
        &mut told,
    )?
    else {
        return Ok(None);
    };
    let together = together.finish();
    let set = match mode {
        Mode::Relative => SetLevel::Relative,
        Mode::Independent => SetLevel::Independent,
    };
    let normalized = normalize_clips(level, set, target, &each, &together, &volumes(t, clips));
    let gains: Vec<(String, Option<f64>)> = clips
        .iter()
        .cloned()
        .zip(normalized.iter().map(|n| n.gain))
        .collect();
    let mut done = json!({
        "state": "done",
        "method": "audio.normalize",
        "level": name,
        "target": decibels(target),
        "mode": match mode { Mode::Relative => "relative", Mode::Independent => "independent" },
        "measured": match mode {
            Mode::Relative => level.of_analysis(&together).map_or(Value::Null, decibels),
            Mode::Independent => Value::Null,
        },
        "clips": clips.iter().zip(&normalized).zip(&each).map(|((clip, n), analysis)| json!({
            "clip": clip,
            "gain_db": n.gain.map_or(Value::Null, gain_of),
            "clamped": n.clamped,
            "measured": level.of_analysis(analysis).map_or(Value::Null, decibels),
        })).collect::<Vec<_>>(),
        "applied": null,
    });
    applied(teller, kind, &t.id, &gains, apply, &mut done);
    Ok(Some(done))
}

/// Applies the gains where it is asked to, and says so in `done`.
fn applied(
    teller: &Teller,
    kind: Kind,
    timeline: &str,
    gains: &[(String, Option<f64>)],
    apply: Option<Apply>,
    done: &mut Value,
) {
    let Some(Apply(revision)) = apply else {
        return;
    };
    let edits = nudges(timeline, gains);
    if edits.is_empty() {
        return;
    }
    let result = teller.apply(kind, revision, edits);
    done["applied"] = result["applied"].clone();
    if let Some(refused) = result.get("refused") {
        done["refused"] = refused.clone();
    }
}

/// A gain in decibels as it is applied, not rounded.
fn gain_of(db: f64) -> Value {
    json!(db)
}

/// `audio.level_match`.
fn matched(
    teller: &Teller,
    kind: Kind,
    project: &Arc<Project>,
    (timeline, reference, clips): (usize, &str, &[String]),
    apply: Option<Apply>,
    stop: &AtomicBool,
) -> Result<Option<Value>, String> {
    let t = &project.timelines[timeline];
    let rate = t.sample_rate;
    let mut all = vec![reference.to_owned()];
    all.extend(clips.iter().cloned());
    let total = all.iter().map(|clip| frames(&clip_range(t, clip))).sum();
    let mut told = progress(teller, stop, rate, total);
    let Some(analyses) = clips_measured(project, timeline, &all, (false, None), &mut told)? else {
        return Ok(None);
    };
    let (reference_analysis, each) = analyses.split_first().expect("the reference first");
    let (low, high) = CLIP_VOLUME_RANGE;
    let volumes = volumes(t, clips);
    let gains: Vec<(String, Option<f64>)> = clips
        .iter()
        .zip(each)
        .map(|(clip, analysis)| (clip.clone(), level_match(reference_analysis, analysis)))
        .collect();
    let loudness = |analysis: &Analysis| analysis.integrated.or(analysis.mean);
    let mut done = json!({
        "state": "done",
        "method": "audio.level_match",
        "reference": {"clip": reference, "measured": loudness(reference_analysis).map_or(Value::Null, decibels)},
        "clips": gains.iter().zip(&volumes).zip(each).map(|(((clip, gain), volume), analysis)| json!({
            "clip": clip,
            "gain_db": gain.map_or(Value::Null, gain_of),
            "clamped": gain.is_some_and(|gain| volume.lowest + gain < low || volume.highest + gain > high),
            "measured": loudness(analysis).map_or(Value::Null, decibels),
        })).collect::<Vec<_>>(),
        "applied": null,
    });
    applied(teller, kind, &t.id, &gains, apply, &mut done);
    Ok(Some(done))
}
