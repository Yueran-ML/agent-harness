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

mod noise_learn;

use std::collections::HashMap;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use columbina_audio::analysis::{
    alone, level_match_level, level_match_to, normalize_clips, run_strips, Analysis, Analyzer,
    Level, SetLevel, Volume,
};
use columbina_audio::constants::{CLIP_VOLUME_RANGE, NORMALIZE_PEAK_DEFAULT, NORMALIZE_RANGE};
use columbina_audio::graph::{Graph, Options, Owner, Point, Purpose};
use columbina_audio::node::Layout;
use columbina_audio::standards::{self, Measure, Standard, Verdict};
use columbina_model::{Clip, Project, Timeline, TrackFormat, TrackKind};
use serde::Deserialize;
use serde_json::{json, value::RawValue, Value};
use tokio::sync::Mutex;

use super::{
    log_kicked, Connection, Engine, Kind, Limits, Reply, State, WorkedOut, WorkedOutError,
};
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
    /// The actual stereo sum of these post-strip tracks, including their panners.
    Sum(Vec<String>),
}

impl Target {
    fn json(&self) -> Value {
        match self {
            Target::Clip(id) => json!({"clip": id}),
            Target::Track(id) => json!({"track": id}),
            Target::Bus(id) => json!({"bus": id}),
            Target::Sum(ids) => json!({"sum": ids}),
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
    #[serde(default)]
    revision: Option<u64>,
    #[serde(default)]
    candidate: Option<Vec<Value>>,
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
    #[serde(default)]
    reference: Option<String>,
    #[serde(default)]
    profile: Option<LevelProfile>,
    clips: Vec<String>,
    #[serde(default)]
    apply: bool,
    #[serde(default)]
    revision: Option<u64>,
}

#[derive(Clone, Debug, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LevelProfile {
    version: u8,
    level_db: Option<f64>,
    basis: Option<ProfileBasis>,
}

#[derive(Clone, Copy, Debug, serde::Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProfileBasis {
    Integrated,
    Mean,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Capture {
    timeline: String,
    clip: String,
    revision: u64,
}

enum MatchReference {
    Clip(String),
    Profile(LevelProfile),
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
        "audio.standards" => {
            rpc::no_params(method, params)?;
            standards_table()
        }
        "audio.meters" => {
            let request: Metering = rpc::params(method, r#"{"timeline": "t1"}"#, params)?;
            let audio = settings(engine, &request.timeline).await?;
            meters::hub().shown(&request.timeline, &audio, request.reset)
        }
        "audio.fx_meters" => {
            let request: Metering = rpc::params(method, r#"{"timeline":"t1"}"#, params)?;
            settings(engine, &request.timeline).await?;
            crate::fx_meters::shown(&request.timeline)
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
        "audio.analyze"
        | "audio.normalize"
        | "audio.level_match"
        | "audio.level_profile"
        | "audio.noise_learn" => {
            let state = engine.state.lock().await;
            let (project, work) = prepare_snapshot(
                method,
                params,
                state.history.project(),
                state.history.revision(),
                &crate::proxy::proxy_dir(std::path::Path::new(&engine.project)),
            )?;
            drop(state);
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

/// The source table, without a second copy of its targets or provenance.
fn standards_table() -> Value {
    json!({"standards": standards::STANDARDS.iter().map(|standard| json!({
        "id": standard.id,
        "name": standard.name,
        "target": standard.target,
        "tolerance": standard.tolerance,
        "true_peak": standard.true_peak,
        "measure": match standard.measure { Measure::Gated => "gated", Measure::Ungated => "ungated" },
        "dialogue": standard.dialogue,
        "resolve": standard.resolve,
        "official": standard.official,
        "verified": standard.verified,
        "source": standard.source,
    })).collect::<Vec<_>>()})
}

/// The settings of the sound of timeline `id`.
async fn settings(engine: &Engine, id: &str) -> Result<columbina_model::TimelineAudio, Fault> {
    let state = engine.state.lock().await;
    let (_, timeline) = timeline_of(state.history.project(), id)?;
    Ok(timeline.audio.clone())
}

/// An analysis checked, ready to run.
enum Work {
    NoiseLearn(noise_learn::Work),
    Capture {
        timeline: usize,
        clip: String,
        revision: u64,
    },
    Analyze {
        timeline: usize,
        targets: Vec<Target>,
        range: Option<Range<i64>>,
        standard: &'static Standard,
        target: f32,
        revision: Option<u64>,
        candidate: Option<Vec<Value>>,
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
        reference: MatchReference,
        clips: Vec<String>,
        apply: Option<Apply>,
        revision: Option<u64>,
    },
}

/// Whether the gains found are applied, and to which revision the transaction is bound.
#[derive(Clone, Copy, Debug)]
struct Apply(Option<u64>);

/// Bind the snapshot under its lock, before allocating or starting a job. Apply also checks
/// the revision again when committing its gains, as another client may edit during analysis.
fn check_revision(work: &Work, actual: u64) -> Result<(), Fault> {
    let expected = match work {
        Work::NoiseLearn(work) => Some(work.revision),
        Work::Capture { revision, .. } => Some(*revision),
        Work::Analyze { revision, .. } | Work::Match { revision, .. } => *revision,
        Work::Normalize { apply, .. } => apply.as_ref().and_then(|Apply(revision)| *revision),
    };
    if let Some(expected) = expected.filter(|expected| *expected != actual) {
        return Err(super::stale_revision(expected, actual));
    }
    Ok(())
}

/// Checks the params of `method` against `project`.
fn prepare(method: &str, params: Option<&RawValue>, project: &Project) -> Result<Work, Fault> {
    match method {
        "audio.noise_learn" => noise_learn::prepare(method, params, project).map(Work::NoiseLearn),
        "audio.level_profile" => {
            let request: Capture = rpc::params(
                method,
                r#"{"timeline":"t1","clip":"c1","revision":n}"#,
                params,
            )?;
            let (timeline, t) = timeline_of(project, &request.timeline)?;
            clip_of(t, &request.clip)?;
            Ok(Work::Capture {
                timeline,
                clip: request.clip,
                revision: request.revision,
            })
        }
        "audio.analyze" => analyze_work(method, params, project),
        "audio.normalize" => normalize_work(method, params, project),
        _ => match_work(method, params, project),
    }
}

/// A candidate uses the normal transaction parser and model rules on a disposable history.
/// No published state, gesture, redo stack or project id is changed by trial measurements.
fn prepare_snapshot(
    method: &str,
    params: Option<&RawValue>,
    project: &Project,
    revision: u64,
    proxy_dir: &std::path::Path,
) -> Result<(Project, Work), Fault> {
    let work = prepare(method, params, project)?;
    check_revision(&work, revision)?;
    let Work::Analyze {
        candidate: Some(ops),
        revision: expected,
        ..
    } = &work
    else {
        return Ok((project.clone(), work));
    };
    if expected.is_none() {
        return Err(refused(
            "candidate analysis requires the project revision",
            json!({"reason":"revision_required"}),
        ));
    }
    let mut inputs = ops
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, op)| op.input(index))
        .collect::<Result<Vec<_>, _>>()?;
    super::check_proxies(&inputs, proxy_dir)?;
    let mut names = crate::names::prepare(&mut inputs).map_err(|error| {
        refused(
            error.to_string(),
            json!({"index":error.index,"name":error.name,"reason":error.reason}),
        )
    })?;
    let mut trial = columbina_model::History::new(project.clone());
    trial
        .change_with(
            inputs.into_iter().map(|input| input.change).collect(),
            "analysis",
            &mut names,
        )
        .map_err(|error| Fault::refused(&error))?;
    let project = trial.project().clone();
    // Validate the targets/range again after the candidate may have changed them.
    let work = prepare(method, params, &project)?;
    Ok((project, work))
}

/// A sum measures distinct, enabled audio tracks before their bus routing.
fn check_sum(timeline: &Timeline, ids: &[String]) -> Result<(), Fault> {
    if !(1..MOST_TARGETS).contains(&ids.len()) {
        return Err(refused(
            "a sum takes 1 to 255 tracks",
            json!({"reason":"out_of_range","what":"sum"}),
        ));
    }
    for (index, id) in ids.iter().enumerate() {
        if ids[..index].contains(id) {
            return Err(refused(
                "a sum names a track twice",
                json!({"reason":"twice","track":id}),
            ));
        }
        if !timeline
            .tracks
            .iter()
            .any(|track| track.id == *id && track.kind == TrackKind::Audio && track.enabled)
        {
            return Err(refused(
                "a sum requires enabled audio tracks",
                json!({"reason":"missing","track":id}),
            ));
        }
    }
    Ok(())
}

/// `audio.analyze` checked.
fn analyze_work(method: &str, params: Option<&RawValue>, project: &Project) -> Result<Work, Fault> {
    let request: Analyze = rpc::params(
        method,
        r#"{"timeline": "t1", "targets": [{"clip": "c1"} | {"track": "A1"} | {"bus": "B1"} | {"sum": ["A2", "A3"]}], "range"?: {"from": 0, "to": 250}, "standard"?: "ebu-r128", "revision"?: 0, "candidate"?: [...]}"#,
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
            Target::Sum(ids) => check_sum(timeline, ids)?,
        }
    }
    let range = match request.range {
        None => None,
        Some(Frames { from, to }) if from >= 0 && to > from => {
            // Not past the end of the timeline (the review of T9b).
            let end = crate::play::mix::end_of(timeline);
            Some(sample_at(timeline, from).min(end)..sample_at(timeline, to).min(end))
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
        revision: request.revision,
        candidate: request.candidate,
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
    let reference = match (request.reference, request.profile) {
        (Some(clip), None) => {
            clip_of(timeline, &clip)?;
            MatchReference::Clip(clip)
        }
        (None, Some(profile)) => {
            if request.revision.is_none() {
                return Err(refused(
                    "a captured profile requires revision",
                    json!({"reason":"revision_required"}),
                ));
            }
            if profile.version != 1 {
                return Err(refused(
                    "unsupported level profile version",
                    json!({"reason":"out_of_range","what":"profile.version"}),
                ));
            }
            if !profile.level_db.is_some_and(f64::is_finite) || profile.basis.is_none() {
                return Err(refused(
                    "the profile has no finite level",
                    json!({"reason":"no_level"}),
                ));
            }
            MatchReference::Profile(profile)
        }
        _ => {
            return Err(refused(
                "provide either reference or profile",
                json!({"reason":"reference_required"}),
            ))
        }
    };
    let revision = if matches!(reference, MatchReference::Profile(_)) || request.apply {
        request.revision
    } else {
        None
    };
    clips_named(timeline, &request.clips)?;
    Ok(Work::Match {
        timeline: at,
        reference,
        clips: request.clips,
        apply: request.apply.then_some(Apply(request.revision)),
        revision,
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
        let message = format!("the thread of the analysis cannot be started: {error}");
        crate::log::log("analysis_failed", &json!({"job": job, "message": message}));
        // Its end is told all the same, after its name (the review of T9b).
        Teller {
            state: Arc::clone(&engine.state),
            limits: engine.limits,
            job: job.clone(),
        }
        .told_now(&json!({"state": "failed", "message": message}));
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
    /// Tells `audio.analysis` with `what`, the job's name added, from a task of the runtime (where
    /// the state cannot be locked by blocking).
    fn told_now(self, what: &Value) {
        let mut what = what.clone();
        what["job"] = json!(self.job);
        tokio::spawn(async move {
            let mut state = self.state.lock().await;
            state.broadcast(&rpc::notification_line("audio.analysis", &what));
            let kicked = std::mem::take(&mut state.kicked);
            drop(state);
            for (id, kind) in kicked {
                log_kicked(id, Some(kind), self.limits);
            }
        });
    }

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

/// An analysis-only destination: existing routes, sidechains and strip settings stay intact.
/// This is not a model operation, so a full 64-bus/10-output project still has room to measure.
/// The extra sink cannot add reachability between existing strips under Solo.
fn sum_snapshot(project: &Project, timeline: usize, tracks: &[String]) -> (Project, String) {
    let mut private = project.clone();
    let t = &mut private.timelines[timeline];
    let mut n = 1;
    let id = loop {
        let id = format!("analysis-sum-{n}");
        if !t.buses.iter().any(|bus| bus.id == id) && !t.tracks.iter().any(|track| track.id == id) {
            break id;
        }
        n += 1;
    };
    // Solo-safe keeps this meter open even when an original bus is soloed. As a sink,
    // its safe flag cannot make any original strip audible.
    let bus = serde_json::from_value(json!({"id":id,"name":"Analysis sum","color":"#000000ff","format":"stereo","solo_safe":true}))
        .expect("a neutral analysis bus");
    t.buses.push(bus);
    for track in &mut t.tracks {
        if tracks.contains(&track.id) {
            track.audio.strip.outputs.push(columbina_model::Output {
                bus: id.clone(),
                on: true,
            });
        }
    }
    (private, id)
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

/// Clip analyses use the intersection in timeline samples, including speed/reverse mapping
/// already handled by the existing clip graph. No source-time conversion is duplicated here.
fn limited_clip_range(timeline: &Timeline, clip: &str, range: &Range<i64>) -> Range<i64> {
    let clip = clip_range(timeline, clip);
    let start = clip.start.max(range.start);
    start..clip.end.min(range.end).max(start)
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
        "k_weighted_mean": analysis.mean.map_or(Value::Null, decibels),
        "silence": analysis.seconds > 0.0 && analysis.sample_peak == f64::NEG_INFINITY,
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
        Work::NoiseLearn(work) => noise_learn::run(teller, kind, project, &work, stop),
        Work::Capture {
            timeline,
            clip,
            revision,
        } => captured(teller, project, timeline, &clip, revision, stop),
        Work::Analyze {
            timeline,
            targets,
            range,
            standard,
            target,
            candidate,
            ..
        } => analyze(
            teller,
            project,
            (timeline, &targets, range),
            (standard, target),
            stop,
        )
        .map(|done| {
            done.map(|mut done| {
                if candidate.is_some() {
                    done["candidate"] = json!(true);
                }
                done
            })
        }),
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
            ..
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
            Target::Clip(_) | Target::Sum(_) => None,
        })
        .collect();
    let clips: u64 = targets
        .iter()
        .map(|what| match what {
            Target::Clip(clip) => frames(&limited_clip_range(t, clip, &range)),
            Target::Sum(_) => frames(&range),
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
        let pass = Pass::strips(project, timeline, &only, range.clone())?;
        let analyzers = pass.analyzers(true);
        let Some(analyses) = pass.run(timeline, analyzers, None, &mut told)? else {
            return Ok(None);
        };
        for ((at, _), analysis) in owners.iter().zip(analyses) {
            found_of[*at] = Some(analysis);
        }
    }
    for (at, what) in targets.iter().enumerate() {
        if let Target::Sum(ids) = what {
            if stop.load(Ordering::Acquire) {
                return Ok(None);
            }
            let (private, bus) = sum_snapshot(project, timeline, ids);
            let pass = Pass::strips(
                &Arc::new(private),
                timeline,
                &[Owner::Bus(bus)],
                range.clone(),
            )?;
            let analyzers = pass.analyzers(true);
            let Some(mut analyses) = pass.run(timeline, analyzers, None, &mut told)? else {
                return Ok(None);
            };
            found_of[at] = analyses.pop();
            continue;
        }
        let Target::Clip(clip) = what else { continue };
        if stop.load(Ordering::Acquire) {
            return Ok(None);
        }
        let mut pass = Pass::clip(project, timeline, clip)?;
        pass.range = limited_clip_range(t, clip, &range);
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

/// Capture the actual clip snapshot; no project or analysis cache is changed.
fn captured(
    teller: &Teller,
    project: &Arc<Project>,
    timeline: usize,
    clip: &str,
    revision: u64,
    stop: &AtomicBool,
) -> Result<Option<Value>, String> {
    let t = &project.timelines[timeline];
    let mut told = progress(teller, stop, t.sample_rate, frames(&clip_range(t, clip)));
    let Some(readings) = clips_measured(
        project,
        timeline,
        &[clip.to_owned()],
        (false, None),
        &mut told,
    )?
    else {
        return Ok(None);
    };
    let analysis = &readings[0];
    let level_db = level_match_level(analysis);
    let basis = level_db.map(|_| {
        if analysis.integrated.is_some() {
            ProfileBasis::Integrated
        } else {
            ProfileBasis::Mean
        }
    });
    Ok(Some(json!({
        "state":"done", "method":"audio.level_profile",
        "source":{"timeline":t.id,"clip":clip,"revision":revision},
        "profile":LevelProfile { version: 1, level_db, basis }, "seconds":analysis.seconds,
    })))
}

/// `audio.level_match`.
fn matched(
    teller: &Teller,
    kind: Kind,
    project: &Arc<Project>,
    (timeline, reference, clips): (usize, &MatchReference, &[String]),
    apply: Option<Apply>,
    stop: &AtomicBool,
) -> Result<Option<Value>, String> {
    let t = &project.timelines[timeline];
    let rate = t.sample_rate;
    let mut all = match reference {
        MatchReference::Clip(clip) => vec![clip.clone()],
        MatchReference::Profile(_) => vec![],
    };
    all.extend(clips.iter().cloned());
    let total = all.iter().map(|clip| frames(&clip_range(t, clip))).sum();
    let mut told = progress(teller, stop, rate, total);
    let Some(analyses) = clips_measured(project, timeline, &all, (false, None), &mut told)? else {
        return Ok(None);
    };
    let (level, each) = match reference {
        MatchReference::Clip(_) => (level_match_level(&analyses[0]), &analyses[1..]),
        MatchReference::Profile(profile) => (profile.level_db, &analyses[..]),
    };
    let (low, high) = CLIP_VOLUME_RANGE;
    let volumes = volumes(t, clips);
    let gains: Vec<(String, Option<f64>)> = clips
        .iter()
        .zip(each)
        .map(|(clip, analysis)| {
            (
                clip.clone(),
                level.and_then(|level| level_match_to(level, analysis)),
            )
        })
        .collect();
    let mut done = json!({
        "state": "done",
        "method": "audio.level_match",
        "clips": gains.iter().zip(&volumes).zip(each).map(|(((clip, gain), volume), analysis)| json!({
            "clip": clip,
            "gain_db": gain.map_or(Value::Null, gain_of),
            "clamped": gain.is_some_and(|gain| volume.lowest + gain < low || volume.highest + gain > high),
            "measured": level_match_level(analysis).map_or(Value::Null, decibels),
        })).collect::<Vec<_>>(),
        "applied": null,
    });
    match reference {
        MatchReference::Clip(clip) => {
            done["reference"] = json!({"clip":clip,"measured":level.map_or(Value::Null, decibels)});
        }
        MatchReference::Profile(profile) => done["profile"] = json!(profile),
    }
    applied(teller, kind, &t.id, &gains, apply, &mut done);
    Ok(Some(done))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captured_level_requests_validate_profiles_and_bind_both_snapshots() {
        let project = Project::sample();
        let raw = |value: Value| RawValue::from_string(value.to_string()).unwrap();
        let capture = raw(json!({"timeline":"t1","clip":"c2","revision":7}));
        let work = prepare("audio.level_profile", Some(&capture), &project).unwrap();
        check_revision(&work, 7).unwrap();
        assert_eq!(
            check_revision(&work, 8).unwrap_err().data.unwrap()["reason"],
            "stale_revision"
        );
        let profile = json!({"version":1,"level_db":-82.123_456_789,"basis":"mean"});
        let matching = json!({"timeline":"t1","clips":["c2"],"profile":profile,"revision":7});
        let work = prepare("audio.level_match", Some(&raw(matching.clone())), &project).unwrap();
        check_revision(&work, 7).unwrap();
        assert!(check_revision(&work, 8).is_err()); // dry-run also binds its snapshot
        for invalid in [
            json!({"timeline":"t1","clip":"c2"}),
            json!({"timeline":"t1","clip":"absent","revision":7}),
            json!({"timeline":"t1","clip":"c2","revision":7,"candidate":[]}),
        ] {
            assert!(prepare("audio.level_profile", Some(&raw(invalid)), &project).is_err());
        }
        for (field, value) in [
            ("version", json!(2)),
            ("level_db", Value::Null),
            ("basis", Value::Null),
            ("basis", json!("rms")),
            ("extra", json!(true)),
        ] {
            let mut invalid = matching.clone();
            invalid["profile"][field] = value;
            assert!(
                prepare("audio.level_match", Some(&raw(invalid)), &project).is_err(),
                "{field}"
            );
        }
        let mut both = matching.clone();
        both["reference"] = json!("c2");
        assert!(prepare("audio.level_match", Some(&raw(both)), &project).is_err());
        let mut unbound = matching;
        unbound.as_object_mut().unwrap().remove("revision");
        assert_eq!(
            prepare("audio.level_match", Some(&raw(unbound)), &project)
                .err()
                .unwrap()
                .data
                .unwrap()["reason"],
            "revision_required"
        );
        // Legacy reference dry-run still has its original unbound contract.
        let legacy = raw(json!({"timeline":"t1","reference":"c2","clips":["c2"],"revision":7}));
        check_revision(
            &prepare("audio.level_match", Some(&legacy), &project).unwrap(),
            99,
        )
        .unwrap();
    }

    #[test]
    fn cancellation_between_passes_skips_the_next_graph_construction() {
        let teller = Teller {
            state: Arc::new(Mutex::new(super::super::tests::state())),
            limits: Limits {
                messages: 16,
                bytes: 65536,
            },
            job: "test-between-passes".into(),
        };
        let stop = AtomicBool::new(false);
        let mut empty = Project::sample();
        for track in &mut empty.timelines[0].tracks {
            track.clips.clear();
        }
        let empty = Arc::new(empty);
        let mut told = progress(&teller, &stop, 48000, 8192);
        let first = Pass::strips(&empty, 0, &[Owner::Track("A1".into())], 0..4096).unwrap();
        let meters = first.analyzers(false);
        assert!(first.run(0, meters, None, &mut told).unwrap().is_some());
        assert_eq!(told.done, 4096);
        // A deliberately poisoned next snapshot is a construction sentinel: if the
        // pre-pass stop check moves below Graph::build, this would fail instead of cancel.
        let mut next = Project::sample();
        next.timelines[0].sample_rate = 0;
        assert!(Pass::clip(&next, 0, "c2").is_err());
        let next = Arc::new(next);
        stop.store(true, Ordering::Release);
        assert!(
            clips_measured(&next, 0, &["c2".into()], (false, None), &mut told)
                .unwrap()
                .is_none()
        );
        for target in [
            Target::Clip("c2".into()),
            Target::Track("A1".into()),
            Target::Sum(vec!["A1".into(), "A2".into()]),
        ] {
            assert!(analyze(
                &teller,
                &next,
                (0, &[target], Some(0..4096)),
                (&standards::STANDARDS[0], -23.0),
                &stop
            )
            .unwrap()
            .is_none());
        }
        // This tests the boundary between passes, not interruption inside Graph::build.
        assert_eq!(told.done, 4096);
    }

    #[test]
    fn sum_requires_distinct_enabled_audio_tracks() {
        let mut project = Project::sample();
        project.timelines[0]
            .tracks
            .push(columbina_model::Track::new("A2", TrackKind::Audio));
        for (ids, reason) in [
            (json!([]), "out_of_range"),
            (json!(["A1", "A1"]), "twice"),
            (json!(["A1", "V1"]), "missing"),
            (json!(["A1", "absent"]), "missing"),
        ] {
            let raw =
                RawValue::from_string(json!({"timeline":"t1","targets":[{"sum":ids}]}).to_string())
                    .unwrap();
            let fault = prepare("audio.analyze", Some(&raw), &project)
                .err()
                .unwrap();
            assert_eq!(fault.data.unwrap()["reason"], reason);
        }
        let raw = RawValue::from_string(
            json!({"timeline":"t1","targets":[{"sum":["A1","A2"]}]}).to_string(),
        )
        .unwrap();
        prepare("audio.analyze", Some(&raw), &project).unwrap();
        let single =
            RawValue::from_string(json!({"timeline":"t1","targets":[{"sum":["A1"]}]}).to_string())
                .unwrap();
        prepare("audio.analyze", Some(&single), &project).unwrap();
        project.timelines[0].tracks[2].enabled = false;
        assert_eq!(
            prepare("audio.analyze", Some(&raw), &project)
                .err()
                .unwrap()
                .data
                .unwrap()["reason"],
            "missing"
        );
    }

    #[test]
    fn sum_private_sink_preserves_full_routes_sidechains_and_all_original_solo_states() {
        use columbina_audio::graph::audible;
        let mut project = Project::sample();
        let t = &mut project.timelines[0];
        t.tracks
            .push(columbina_model::Track::new("A2", TrackKind::Audio));
        // A full valid public routing configuration. The private meter is allowed to exceed
        // these UI/model limits, without asking the user to remove their normal buses/routes.
        while t.buses.len() < 64 {
            let id = format!("B{}", t.buses.len() + 1);
            t.buses.push(
                serde_json::from_value(
                    json!({"id":id,"name":"Bus","color":"#000000ff","format":"stereo"}),
                )
                .unwrap(),
            );
        }
        for track in t
            .tracks
            .iter_mut()
            .filter(|track| track.kind == TrackKind::Audio)
        {
            track.audio.strip.outputs = t.buses[..10]
                .iter()
                .map(|bus| columbina_model::Output {
                    bus: bus.id.clone(),
                    on: true,
                })
                .collect();
        }
        t.tracks[2].audio.strip.track_fx =
            serde_json::from_value(json!({"on":true,"sources":[{"track":"A1"}]})).unwrap();
        project.validate().unwrap();
        for solo in 0..3 {
            project.timelines[0].tracks[1].audio.solo = solo == 1;
            project.timelines[0].buses[0].solo = solo == 2;
            let original = project.clone();
            let heard = audible(&original.timelines[0]);
            let (mut private, id) = sum_snapshot(&project, 0, &["A1".into(), "A2".into()]);
            assert_eq!(project, original);
            assert_eq!(private.timelines[0].buses.len(), 65);
            let trial_heard = audible(&private.timelines[0]);
            assert_eq!(&trial_heard[..heard.len()], &heard);
            assert_eq!(trial_heard.last(), Some(&(Owner::Bus(id.clone()), true)));
            let graph =
                Graph::build(&private, 0, Options::new(MAX_BLOCK, Purpose::Analysis)).unwrap();
            assert!(graph.strip_of(&Owner::Bus(id.clone())).is_some());
            // Removing only the private additions recovers the exact original snapshot,
            // including every sidechain, output, monitor choice, clip and automation lane.
            private.timelines[0].buses.pop();
            for track in private.timelines[0]
                .tracks
                .iter_mut()
                .filter(|track| track.kind == TrackKind::Audio)
            {
                assert_eq!(track.audio.strip.outputs.len(), 11);
                assert_eq!(track.audio.strip.outputs.pop().unwrap().bus, id);
            }
            assert_eq!(private, original);
        }
    }

    #[test]
    fn clip_analysis_limits_its_samples_to_the_requested_timeline_interval() {
        let project = Project::sample();
        let timeline = &project.timelines[0];
        let whole = clip_range(timeline, "c2");
        assert!(whole.end > whole.start + 4);
        let middle = whole.start + 1..whole.end - 1;
        assert_eq!(limited_clip_range(timeline, "c2", &middle), middle);
        assert_eq!(
            limited_clip_range(timeline, "c2", &(0..whole.end + 1)),
            whole
        );
        let outside = whole.end + 1..whole.end + 10;
        assert!(limited_clip_range(timeline, "c2", &outside).is_empty());
    }

    #[test]
    fn candidate_analysis_changes_only_its_snapshot_and_requires_the_current_revision() {
        let mut history = columbina_model::History::new(Project::sample());
        let before = history.project().clone();
        let request = json!({"timeline":"t1", "targets":[{"clip":"c2"}], "revision":0,
            "candidate":[{"edit":"nudge_clip_gain", "timeline":"t1", "clips":["c2"], "delta_db":3.0, "linked":false}]});
        let raw = RawValue::from_string(request.to_string()).unwrap();
        let (trial, _) = prepare_snapshot(
            "audio.analyze",
            Some(&raw),
            history.project(),
            0,
            std::path::Path::new("proxies"),
        )
        .unwrap();
        assert_ne!(trial, before);
        assert_eq!(history.project(), &before);
        assert_eq!(history.revision(), 0);
        assert!(history.undo("test").is_none());
        let fault = prepare_snapshot(
            "audio.analyze",
            Some(&raw),
            &before,
            1,
            std::path::Path::new("proxies"),
        )
        .err()
        .unwrap();
        assert_eq!(fault.data.unwrap()["reason"], "stale_revision");
        let mut unbound = request;
        unbound.as_object_mut().unwrap().remove("revision");
        let raw = RawValue::from_string(unbound.to_string()).unwrap();
        let fault = prepare_snapshot(
            "audio.analyze",
            Some(&raw),
            &before,
            0,
            std::path::Path::new("proxies"),
        )
        .err()
        .unwrap();
        assert_eq!(fault.data.unwrap()["reason"], "revision_required");
    }

    #[test]
    fn candidate_analysis_refuses_invalid_changes_without_mutating_the_project() {
        let mut project = Project::sample();
        project.timelines[0].tracks[1].locked = true;
        let before = project.clone();
        let raw = RawValue::from_string(json!({"timeline":"t1", "targets":[{"clip":"c2"}], "revision":0,
            "candidate":[{"edit":"nudge_clip_gain", "timeline":"t1", "clips":["c2"], "delta_db":3.0, "linked":false}]}).to_string()).unwrap();
        let fault = prepare_snapshot(
            "audio.analyze",
            Some(&raw),
            &project,
            0,
            std::path::Path::new("proxies"),
        )
        .err()
        .unwrap();
        assert_eq!(fault.data.unwrap()["reason"], "locked");
        assert_eq!(project, before);
    }

    #[test]
    fn standards_publish_the_source_table_without_own_defaults() {
        let table = standards_table();
        let rows = table["standards"].as_array().unwrap();
        assert_eq!(rows.len(), 15);
        for (row, standard) in rows.iter().zip(standards::STANDARDS) {
            assert_eq!(row["id"], standard.id);
            assert_eq!(row["name"], standard.name);
            assert_eq!(row["target"], json!(standard.target));
            assert_eq!(row["tolerance"], json!(standard.tolerance));
            assert_eq!(row["true_peak"], json!(standard.true_peak));
            assert_eq!(row["official"], standard.official);
            assert_eq!(row["verified"], standard.verified);
            assert_eq!(row["source"], standard.source);
            assert_eq!(row["resolve"], standard.resolve);
            assert_eq!(row["dialogue"], standard.dialogue);
            assert_eq!(
                row["measure"],
                if standard.measure == Measure::Gated {
                    "gated"
                } else {
                    "ungated"
                }
            );
        }
    }

    #[test]
    fn analysis_revision_is_checked_before_starting_a_job() {
        let project = Project::sample();
        let timeline = &project.timelines[0];
        let params = json!({"timeline": timeline.id, "targets": [{"bus": timeline.buses[0].id}], "revision": 4});
        let raw = RawValue::from_string(params.to_string()).unwrap();
        let work = prepare("audio.analyze", Some(&raw), &project).unwrap();
        let fault = check_revision(&work, 5).unwrap_err();
        assert_eq!(fault.code, rpc::REFUSED);
        assert_eq!(fault.data.unwrap()["reason"], "stale_revision");
        check_revision(&work, 4).unwrap();
        let mut unbound = params;
        unbound.as_object_mut().unwrap().remove("revision");
        let raw = RawValue::from_string(unbound.to_string()).unwrap();
        let work = prepare("audio.analyze", Some(&raw), &project).unwrap();
        check_revision(&work, 99).unwrap();
    }

    #[test]
    fn normalize_snapshot_refuses_stale_revision_without_jobs_or_history_changes() {
        let mut history = columbina_model::History::new(Project::sample());
        let before = history.project().clone();
        let running = jobs().running().len();
        let raw = RawValue::from_string(
            json!({
                "timeline": "t1", "clips": ["c2"], "apply": true, "revision": 1
            })
            .to_string(),
        )
        .unwrap();
        let work = prepare("audio.normalize", Some(&raw), history.project()).unwrap();
        let fault = check_revision(&work, history.revision()).unwrap_err();
        assert_eq!(
            fault.data.unwrap(),
            json!({"reason": "stale_revision", "expected": 1, "actual": 0})
        );
        assert_eq!(history.project(), &before);
        assert_eq!(history.revision(), 0);
        assert!(history.undo("agent").is_none());
        assert_eq!(jobs().running().len(), running);
        let raw = RawValue::from_string(
            json!({
                "timeline": "t1", "clips": ["c2"], "apply": true
            })
            .to_string(),
        )
        .unwrap();
        let work = prepare("audio.normalize", Some(&raw), history.project()).unwrap();
        check_revision(&work, 99).unwrap();
    }

    #[test]
    fn normalized_nudges_keep_keys_clamp_and_undo_as_one_step() {
        for (gain, expected) in [(4.0, [-2.0, 1.0]), (50.0, [24.0, 24.0])] {
            let mut project = Project::sample();
            project.timelines[0].tracks[1].clips[0].gain_db = serde_json::from_value(json!({
                "keys": [
                    {"at": {"ticks": 0, "scale": 25}, "value": -6},
                    {"at": {"ticks": 25, "scale": 25}, "value": -3}
                ]
            }))
            .unwrap();
            let before = project.clone();
            let mut history = columbina_model::History::new(project);
            let changes = nudges("t1", &[("c2".into(), Some(gain)), ("silent".into(), None)]);
            assert_eq!(changes.len(), 1);
            history
                .edit(
                    changes
                        .into_iter()
                        .map(|change| serde_json::from_value(change).unwrap())
                        .collect(),
                    "agent",
                )
                .unwrap();
            let keys =
                serde_json::to_value(&history.project().timelines[0].tracks[1].clips[0].gain_db)
                    .unwrap();
            assert_eq!(keys["keys"][0]["at"], json!({"ticks": 0, "scale": 25}));
            assert_eq!(keys["keys"][1]["at"], json!({"ticks": 25, "scale": 25}));
            assert_eq!(keys["keys"][0]["value"], expected[0]);
            assert_eq!(keys["keys"][1]["value"], expected[1]);
            assert_eq!(history.revision(), 1);
            history.undo("agent").unwrap().unwrap();
            assert_eq!(history.project(), &before);
            assert!(history.undo("agent").is_none());
        }
    }
}
