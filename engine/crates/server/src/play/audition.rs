//! File audition: an independent, bounded worker, fed by the ordinary raw-channel mixer.
//! The project projection never enters history or replaces either monitor's loaded source.

use std::sync::{mpsc, Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use columbina_audio::graph::Purpose;
use columbina_model::audio::{ChannelOrder, TrackFormat};
use columbina_model::{
    ChannelMap, ChannelSource, ChannelSources, Media, MediaAudioTrack, Project, Rate, RemixSource,
    Time,
};
use serde::{Deserialize, Serialize};
use serde_json::{value::RawValue, Value};
use tokio::sync::oneshot;

use crate::{
    audio,
    rpc::{self, Fault},
};

const RATE: u32 = 48_000;
const POLL: Duration = Duration::from_millis(10);

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "lowercase", deny_unknown_fields)]
pub(crate) enum Request {
    Play {
        target: Target,
        at: Time,
        channel: Option<usize>,
        #[serde(default)]
        skim: bool,
    },
    Stop {
        audition: u64,
    },
    // A struct variant: serde ignores unknown fields beside the tag of a unit variant.
    State {},
}

#[derive(Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub(crate) enum Target {
    Media { media: String },
    Clip { timeline: String, clip: String },
}

impl Request {
    pub(crate) fn read(params: Option<&RawValue>) -> Result<Self, Fault> {
        rpc::params(
            "audio.audition",
            r#"{"action":"play"|"stop"|"state", ...}"#,
            params,
        )
    }
}

#[derive(Clone, Default, Serialize)]
struct Status {
    audition: Option<u64>,
    playing: bool,
    position: Option<Time>,
    error: Option<String>,
}

#[derive(Default)]
struct Shared {
    status: Status,
    owner: Option<u64>,
    sequence: u64,
    generation: u64,
    /// What a continuous audition plays, so that a project change can plan it again.
    replay: Option<(Target, Option<usize>)>,
}

fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) struct Plan {
    project: Project,
    from: i64,
    end: i64,
    target: Target,
    channel: Option<usize>,
    skim: bool,
}

fn sample(time: Time) -> i64 {
    i64::try_from(i128::from(time.ticks) * i128::from(RATE) / i128::from(time.scale))
        .unwrap_or(if time.ticks < 0 { i64::MIN } else { i64::MAX })
}

fn automatic(count: u16, channel: u16) -> ChannelMap {
    ChannelMap {
        source: ChannelSources(vec![]),
        remix: Some(RemixSource {
            channels: count,
            channel,
        }),
        enabled: true,
        mute: false,
        level_db: 0.0,
    }
}

fn original_rows(media: &Media) -> Vec<MediaAudioTrack> {
    if let Some(config) = &media.audio_config {
        return config.clone();
    }
    let Some(audio) = &media.facts.audio else {
        return vec![];
    };
    let streams: Vec<_> = match &audio.streams {
        Some(streams) => streams
            .iter()
            .map(|s| (Some(s.index), s.channels))
            .collect(),
        None => vec![(None, audio.channels)],
    };
    let mut rows = vec![];
    for (stream, count) in streams {
        for first in (0..count).step_by(24) {
            let length = (count - first).min(24);
            rows.push(MediaAudioTrack {
                name: String::new(),
                format: match length {
                    1 => TrackFormat::Mono,
                    2 => TrackFormat::Stereo,
                    6 => TrackFormat::FiveOne,
                    _ => TrackFormat::Adaptive,
                },
                order: ChannelOrder::Smpte,
                channels: (first..first + length)
                    .map(|channel| match stream {
                        Some(stream) => ChannelMap {
                            source: ChannelSources(vec![ChannelSource {
                                stream,
                                channel: u32::from(channel),
                            }]),
                            remix: None,
                            enabled: true,
                            mute: false,
                            level_db: 0.0,
                        },
                        None => automatic(count, channel),
                    })
                    .collect(),
            });
        }
    }
    rows
}

/// A timeline clip's displayed lanes, and its source extent in 48 kHz samples: from its In to
/// In plus its timeline duration times its speed, bounded by the media's `available` samples.
fn clip_rows(
    timeline: &columbina_model::Timeline,
    track: &columbina_model::Track,
    clip: &columbina_model::Clip,
    available: i64,
) -> (Vec<MediaAudioTrack>, i64, i64) {
    let begin = sample(clip.source_in().expect("media clip"));
    let end = begin
        .saturating_add(columbina_model::span::source_span(
            clip,
            timeline.rate,
            RATE,
        ))
        .clamp(0, available);
    let count = u16::from(
        track
            .audio
            .format
            .channels()
            .unwrap_or(track.audio.channels),
    );
    let channels = clip.channels.clone().unwrap_or_else(|| {
        (0..count)
            .map(|channel| {
                let channel = if track.audio.format == TrackFormat::FiveOne
                    && track.audio.order == ChannelOrder::Film
                {
                    [0, 2, 1, 4, 5, 3][usize::from(channel)]
                } else {
                    channel
                };
                automatic(count, channel)
            })
            .collect()
    });
    let row = MediaAudioTrack {
        name: String::new(),
        format: track.audio.format,
        order: track.audio.order,
        channels,
    };
    (vec![row], begin.clamp(0, end), end)
}

pub(crate) fn plan(
    project: &Project,
    target: &Target,
    at: Time,
    channel: Option<usize>,
    skim: bool,
) -> Result<Plan, Fault> {
    if at.scale == 0 {
        return Err(Fault::params("audition time scale must be positive"));
    }
    let missing =
        |what: &str, id: &str| Fault::params(format!("there is no {what} {id} in the project"));
    let (media_id, clip) = match target {
        Target::Media { media } => (media, None),
        Target::Clip { timeline, clip } => {
            let timeline = project
                .timelines
                .iter()
                .find(|t| t.id == *timeline)
                .ok_or_else(|| missing("timeline", timeline))?;
            let (track, found) = timeline
                .tracks
                .iter()
                .find_map(|track| {
                    track
                        .clips
                        .iter()
                        .find(|c| c.id == *clip)
                        .map(|c| (track, c))
                })
                .ok_or_else(|| missing("clip", clip))?;
            let media = found
                .media()
                .ok_or_else(|| Fault::params("the audition clip has no source media"))?;
            (media, Some((timeline, track, found)))
        }
    };
    let media = project
        .media
        .iter()
        .find(|m| m.id == *media_id)
        .ok_or_else(|| missing("media", media_id))?;
    if media.facts.audio.is_none() {
        return Err(Fault::params("the audition source has no audio"));
    }
    let available = sample(media.facts.duration).max(0);
    let (rows, begin, end) = match clip {
        Some((timeline, track, clip)) => clip_rows(timeline, track, clip, available),
        None => (original_rows(media), 0, available),
    };
    if rows.iter().all(|row| row.channels.is_empty()) {
        return Err(Fault::params(
            "the audition source has no configured audio channels",
        ));
    }
    let from = sample(at).clamp(begin, end);
    let end = if skim {
        end.min(from.saturating_add(i64::from(RATE) * 80 / 1000))
    } else {
        end
    };
    let projected =
        super::mix::raw::project(media, Rate { num: RATE, den: 1 }, end, &rows, channel)
            .map_err(Fault::params)?;
    Ok(Plan {
        project: projected,
        from,
        end,
        target: target.clone(),
        channel,
        skim,
    })
}

enum Command {
    Play {
        generation: u64,
        plan: Box<Plan>,
        reply: oneshot::Sender<()>,
    },
    Barrier(oneshot::Sender<()>),
}

#[derive(Clone)]
pub(crate) struct Audition {
    commands: mpsc::SyncSender<Command>,
    shared: Arc<Mutex<Shared>>,
}

impl Audition {
    pub(crate) fn new(kind: audio::Kind) -> Self {
        // A separate capture base, retaining Output's normal numbered-session suffixes.
        let kind = match kind {
            audio::Kind::Capture(path) => {
                let mut name = path.into_os_string();
                name.push(".audition");
                audio::Kind::Capture(name.into())
            }
            other => other,
        };
        let (commands, inbox) = mpsc::sync_channel(4);
        let shared = Arc::new(Mutex::new(Shared::default()));
        let worker = Arc::clone(&shared);
        if let Err(error) = std::thread::Builder::new()
            .name("columbina-file-audition".into())
            .spawn(move || run(&kind, &inbox, &worker))
        {
            lock(&shared).status.error = Some(error.to_string());
        }
        Self { commands, shared }
    }

    pub(crate) fn state(&self) -> Value {
        serde_json::to_value(&lock(&self.shared).status).expect("audition status")
    }

    /// Invalidation is synchronous with project publication; queued old plays cannot start.
    pub(crate) fn cancel(&self, owner: Option<u64>) {
        let mut shared = lock(&self.shared);
        if owner.is_some() && owner != shared.owner {
            return;
        }
        shared.generation += 1;
        shared.status.playing = false;
    }

    /// Called as a project is published. A continuous audition is planned again from where it
    /// is heard, keeping its ID, so that a change of Active, Mute, source or level is heard at
    /// once; a skim or a target that no longer plans stops.
    pub(crate) fn project_changed(&self, project: &Project) {
        let mut shared = lock(&self.shared);
        shared.generation += 1;
        let again = match (
            &shared.replay,
            shared.status.playing,
            shared.status.position,
        ) {
            (Some((target, channel)), true, Some(at)) => {
                plan(project, target, at, *channel, false).ok()
            }
            _ => None,
        };
        let Some(plan) = again else {
            shared.status.playing = false;
            shared.replay = None;
            return;
        };
        let generation = shared.generation;
        shared.status.playing = plan.from < plan.end;
        let (reply, _done) = oneshot::channel();
        if let Err(error) = self.commands.try_send(Command::Play {
            generation,
            plan: Box::new(plan),
            reply,
        }) {
            shared.status.playing = false;
            shared.status.error = Some(error.to_string());
        }
    }

    pub(crate) async fn stopped(&self) -> Result<(), Fault> {
        self.cancel(None);
        let (reply, done) = oneshot::channel();
        self.commands
            .try_send(Command::Barrier(reply))
            .map_err(|e| failed(e.to_string()))?;
        done.await.map_err(|e| failed(e.to_string()))
    }

    pub(crate) async fn stop(&self, owner: u64, audition: u64) -> Result<Value, Fault> {
        let matches = {
            let shared = lock(&self.shared);
            shared.owner == Some(owner) && shared.status.audition == Some(audition)
        };
        if matches {
            self.stopped().await?;
        }
        Ok(self.state())
    }

    /// Called with project State and the transport order locked, before either can change.
    pub(crate) fn begin(&self, owner: u64, plan: Plan) -> Result<oneshot::Receiver<()>, Fault> {
        let (reply, done) = oneshot::channel();
        let mut shared = lock(&self.shared);
        shared.sequence += 1;
        shared.generation += 1;
        shared.owner = Some(owner);
        shared.replay = (!plan.skim).then(|| (plan.target.clone(), plan.channel));
        shared.status = Status {
            audition: Some(shared.sequence),
            playing: plan.from < plan.end,
            position: Some(Time {
                ticks: plan.from,
                scale: RATE,
            }),
            error: None,
        };
        let generation = shared.generation;
        if let Err(error) = self.commands.try_send(Command::Play {
            generation,
            plan: Box::new(plan),
            reply,
        }) {
            shared.status.playing = false;
            shared.status.error = Some(error.to_string());
            return Err(failed(error.to_string()));
        }
        Ok(done)
    }
}

fn failed(message: String) -> Fault {
    Fault::new(rpc::TRANSPORT_FAILED, message)
}

#[derive(Default)]
struct Fed {
    frames: i64,
    end: bool,
    /// When the source ended; the device's count of played frames may stop short of `frames`
    /// once its session has nothing more to take.
    ended_at: Option<Instant>,
    error: Option<String>,
}

impl Fed {
    fn finish(&mut self) {
        self.end = true;
        self.ended_at.get_or_insert_with(Instant::now);
    }
}

/// How long past its source's end the device may take to play out what was fed to it.
const DRAIN: Duration = Duration::from_millis(250);

struct Reporting {
    source: Box<dyn audio::Source>,
    channels: usize,
    fed: Arc<Mutex<Fed>>,
    /// Device frames from the plan's start to its end; the mixer itself plays on in silence.
    limit: i64,
}
impl audio::Source for Reporting {
    fn next(&mut self) -> Result<Option<Vec<f32>>, String> {
        if lock(&self.fed).frames >= self.limit {
            lock(&self.fed).finish();
            return Ok(None);
        }
        let mut result = self.source.next();
        let mut fed = lock(&self.fed);
        if let Ok(Some(samples)) = &mut result {
            let left = usize::try_from(self.limit - fed.frames).unwrap_or(usize::MAX);
            samples.truncate(left.saturating_mul(self.channels));
        }
        match &result {
            Ok(Some(samples)) => {
                fed.frames += i64::try_from(samples.len() / self.channels).unwrap_or(i64::MAX);
            }
            Ok(None) => fed.finish(),
            Err(error) => {
                fed.finish();
                fed.error = Some(error.clone());
            }
        }
        result
    }
}

struct Playing {
    generation: u64,
    from: i64,
    end: i64,
    fed: Arc<Mutex<Fed>>,
}

/// Open the device if it is not open yet and start feeding the plan's mix, at most from its
/// start to its end, to it.
fn start(
    kind: &audio::Kind,
    output: &mut Option<audio::Output>,
    generation: u64,
    plan: Plan,
) -> Result<Playing, String> {
    if output.is_none() {
        *output = Some(audio::Output::open(kind)?);
    }
    let output = output.as_mut().expect("opened");
    let fed = Arc::new(Mutex::new(Fed::default()));
    let source = super::mix::Mixer::new(Arc::new(plan.project), 0, Purpose::Preview)
        .source(output.format(), plan.from);
    let limit = i64::try_from(
        i128::from(plan.end - plan.from) * i128::from(output.format().rate) / i128::from(RATE),
    )
    .unwrap_or(i64::MAX);
    output.start_finite(Box::new(Reporting {
        source,
        channels: usize::from(output.format().channels),
        fed: Arc::clone(&fed),
        limit,
    }))?;
    Ok(Playing {
        generation,
        from: plan.from,
        end: plan.end,
        fed,
    })
}

fn run(kind: &audio::Kind, inbox: &mpsc::Receiver<Command>, shared: &Mutex<Shared>) {
    let mut output: Option<audio::Output> = None;
    let mut playing: Option<Playing> = None;
    loop {
        let command = match inbox.recv_timeout(POLL) {
            Ok(command) => Some(command),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if playing
            .as_ref()
            .is_some_and(|p| p.generation != lock(shared).generation)
        {
            if let Some(output) = &mut output {
                output.stop();
            }
            playing = None;
        }
        match command {
            Some(Command::Play {
                generation,
                plan,
                reply,
            }) => {
                if let Some(output) = &mut output {
                    output.stop();
                }
                playing = None;
                if generation == lock(shared).generation && plan.from < plan.end {
                    match start(kind, &mut output, generation, *plan) {
                        Ok(started) => playing = Some(started),
                        Err(error) => {
                            output = None;
                            let mut state = lock(shared);
                            if state.generation == generation {
                                state.status.playing = false;
                                state.status.error = Some(error);
                            }
                        }
                    }
                }
                let _ = reply.send(());
            }
            Some(Command::Barrier(reply)) => {
                let _ = reply.send(());
            }
            None => {}
        }
        if let (Some(current), Some(device)) = (&playing, &mut output) {
            let now = Instant::now();
            let fed = lock(&current.fed);
            let rate = i64::from(device.format().rate);
            // The feeder runs at most about a second ahead of the device; once that and the
            // device's own latency have passed since the source ended, all of it was heard.
            let drained = fed.ended_at.is_some_and(|at| {
                let ahead = (u64::try_from(fed.frames).unwrap_or(0) * 1000
                    / u64::try_from(rate.max(1)).unwrap_or(1))
                .min(1000);
                now.duration_since(at) >= Duration::from_millis(ahead) + DRAIN
            });
            let heard = if drained {
                fed.frames
            } else {
                device.played(now).unwrap_or(0).max(0)
            };
            let error = fed.error.clone().or_else(|| device.failed());
            let ended = fed.end && heard >= fed.frames;
            let mut state = lock(shared);
            if state.generation == current.generation {
                let position = current
                    .from
                    .saturating_add(
                        i64::try_from(
                            i128::from(heard) * i128::from(RATE) / i128::from(device.format().rate),
                        )
                        .unwrap_or(i64::MAX),
                    )
                    .min(current.end);
                state.status.position = Some(Time {
                    ticks: position,
                    scale: RATE,
                });
                if ended || error.is_some() {
                    state.status.playing = false;
                    state.status.error.clone_from(&error);
                }
            }
            drop(state);
            drop(fed);
            if ended || error.is_some() {
                device.stop();
                playing = None;
                if error.is_some() {
                    output = None;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
