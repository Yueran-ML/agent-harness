//! The timeline monitor. Decode workers prepare pictures; this thread owns the clock and
//! compositor, and never holds the project/session lock while waiting for a picture or GPU.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::{mpsc, Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use columbina_compose::{Compositor, Scene, SceneLayer, Source as LayerSource, Target, Text};
use columbina_media::Hardware;
use columbina_model::project::TrackKind;
use columbina_model::{resolve, Project};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::oneshot;

use super::ahead::{Request, TrackReader, Want};
use super::sources::{Read, Sources};
use super::{Action, Notify};
use crate::gpu::{Graphics, Subscription};
use crate::log::log;
use crate::rpc::{self, Fault};
use crate::transport::clock::{self, Clock};
use crate::transport::{Again, Sound, SoundSlot, Source, POOL_BYTES, SETTLE, TOLD_EVERY};
use crate::view::{Monitor, ViewSlot};

const SEEK_WAIT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Status {
    pub monitor: Monitor,
    pub timeline: Option<String>,
    pub frame: Option<i64>,
    pub playing: bool,
    pub speed: i32,
    pub exact: bool,
    pub last: Option<i64>,
    pub shown: Option<i64>,
    pub late: u64,
    pub sound: Sound,
    pub source: Option<Source>,
    pub proxies: bool,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            monitor: Monitor::Timeline,
            timeline: None,
            frame: None,
            playing: false,
            speed: 1,
            exact: true,
            last: None,
            shown: None,
            late: 0,
            sound: Sound::None,
            source: None,
            proxies: true,
        }
    }
}

#[derive(Default)]
struct Shared {
    status: Status,
    loading: usize,
    stopped: bool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn stopped() -> Fault {
    Fault::new(rpc::TRANSPORT_FAILED, "the timeline transport has stopped")
        .with_data(json!({"reason":"failed"}))
}

fn not_loaded() -> Fault {
    Fault::new(
        rpc::TRANSPORT_FAILED,
        "no timeline is loaded in the transport",
    )
    .with_data(json!({"reason":"not_loaded"}))
}

enum Command {
    Load {
        project: Arc<Project>,
        id: String,
        reply: oneshot::Sender<Result<Status, Fault>>,
    },
    Act(Action),
    Project(Arc<Project>),
    Proxy(String),
    Redraw,
    Pause(oneshot::Sender<()>),
    Digest(oneshot::Sender<Result<Value, Fault>>),
    Stop,
}

struct Inner {
    inbox: mpsc::Sender<Command>,
    shared: Arc<Mutex<Shared>>,
    _graphics: Subscription,
}

#[derive(Clone)]
pub struct Timeline(Arc<Inner>);

pub struct Loading(oneshot::Receiver<Result<Status, Fault>>);

impl Loading {
    pub async fn done(self) -> Result<Value, Fault> {
        let status = self.0.await.map_err(|_| stopped())??;
        Ok(json!(status))
    }
}

impl Timeline {
    pub fn new(
        graphics: Arc<Graphics>,
        view: ViewSlot,
        notify: Notify,
        again: Again,
        sound: crate::audio::Kind,
    ) -> Self {
        let (inbox, received) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let subscription = graphics.subscribe({
            let inbox = inbox.clone();
            move |_| {
                let _ = inbox.send(Command::Redraw);
            }
        });
        let started = std::thread::Builder::new()
            .name("columbina-timeline".into())
            .spawn({
                let shared = Arc::clone(&shared);
                move || {
                    let _guard = Guard {
                        shared: Arc::clone(&shared),
                        notify: Arc::clone(&notify),
                    };
                    Worker {
                        inbox: received,
                        shared,
                        notify,
                        view,
                        graphics,
                        sources: Sources::new(Hardware::Auto, again),
                        status: Status::default(),
                        project: None,
                        timeline: None,
                        generation: 0,
                        readers: BTreeMap::new(),
                        reader_budget: 0,
                        previous: BTreeMap::new(),
                        compositor: None,
                        targets: Vec::new(),
                        current: None,
                        clock: None,
                        sound: SoundSlot::new(sound),
                        mixed_from: None,
                        heard: Cell::new(0),
                        fine: None,
                        epoch: Instant::now(),
                        settle: None,
                        told: None,
                    }
                    .run();
                }
            });
        if let Err(error) = started {
            lock(&shared).stopped = true;
            log(
                "transport_error",
                &json!({"monitor":"timeline", "message":error.to_string()}),
            );
        }
        Self(Arc::new(Inner {
            inbox,
            shared,
            _graphics: subscription,
        }))
    }

    fn send(&self, command: Command) -> Result<(), Fault> {
        if lock(&self.0.shared).stopped {
            return Err(stopped());
        }
        self.0.inbox.send(command).map_err(|_| stopped())
    }

    pub fn status(&self) -> Value {
        json!(lock(&self.0.shared).status)
    }

    pub fn check_loaded(&self) -> Result<(), Fault> {
        let shared = lock(&self.0.shared);
        if shared.stopped {
            Err(stopped())
        } else if shared.status.timeline.is_none() && shared.loading == 0 {
            Err(not_loaded())
        } else {
            Ok(())
        }
    }

    pub fn begin_load(&self, project: Arc<Project>, id: String) -> Result<Loading, Fault> {
        let (reply, receiver) = oneshot::channel();
        lock(&self.0.shared).loading += 1;
        if let Err(error) = self.send(Command::Load { project, id, reply }) {
            lock(&self.0.shared).loading -= 1;
            return Err(error);
        }
        Ok(Loading(receiver))
    }

    pub fn act(&self, action: Action) {
        let _ = self.send(Command::Act(action));
    }
    pub fn project_changed(&self, project: &Project) {
        let _ = self.send(Command::Project(Arc::new(project.clone())));
    }
    pub fn proxy_ready(&self, media: &str) {
        let _ = self.send(Command::Proxy(media.into()));
    }
    pub fn redraw(&self) {
        let _ = self.send(Command::Redraw);
    }
    pub fn stop(&self) {
        let _ = self.send(Command::Stop);
    }

    pub async fn pause(&self) -> Result<(), Fault> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::Pause(reply))?;
        receiver.await.map_err(|_| stopped())
    }

    pub async fn digest(&self) -> Result<Value, Fault> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::Digest(reply))?;
        receiver.await.map_err(|_| stopped())?
    }
}

struct Guard {
    shared: Arc<Mutex<Shared>>,
    notify: Notify,
}

impl Drop for Guard {
    fn drop(&mut self) {
        let status = {
            let mut shared = lock(&self.shared);
            shared.stopped = true;
            shared.loading = 0;
            shared.status = Status::default();
            json!(shared.status)
        };
        (self.notify)(&status);
    }
}

struct Worker {
    inbox: mpsc::Receiver<Command>,
    shared: Arc<Mutex<Shared>>,
    notify: Notify,
    view: ViewSlot,
    graphics: Arc<Graphics>,
    sources: Sources,
    status: Status,
    project: Option<Arc<Project>>,
    timeline: Option<usize>,
    generation: u64,
    readers: BTreeMap<String, TrackReader>,
    reader_budget: usize,
    previous: BTreeMap<(String, String), Read>,
    compositor: Option<Compositor>,
    targets: Vec<Arc<Target>>,
    current: Option<Arc<Target>>,
    clock: Option<Clock>,
    sound: SoundSlot,
    /// The sample of the timeline that the mixer that plays started at, and the timeline's
    /// rate of samples: where the sound is, for the test hook to tell.
    mixed_from: Option<(i64, u32)>,
    heard: Cell<i128>,
    fine: Option<crate::timer::Fine>,
    epoch: Instant,
    settle: Option<Instant>,
    told: Option<Instant>,
}

impl Worker {
    fn now(&self) -> i128 {
        i128::try_from(self.epoch.elapsed().as_nanos()).unwrap_or(i128::MAX)
    }

    fn clock_now(&self) -> i128 {
        if self.status.sound != Sound::On {
            return self.now();
        }
        if let Some(output) = self.sound.working() {
            if let Some(samples) = output.played(Instant::now()) {
                let heard = i128::from(samples) * clock::NANOS / i128::from(output.format().rate);
                self.heard.set(self.heard.get().max(heard));
            }
        }
        self.heard.get()
    }

    fn has_sound(&self) -> bool {
        self.project
            .as_ref()
            .zip(self.timeline)
            .is_some_and(|(project, index)| {
                project.timelines[index].tracks.iter().any(|track| {
                    track.enabled
                        && track.kind == TrackKind::Audio
                        && track.clips.iter().any(|clip| clip.enabled)
                })
            })
    }

    fn restart_sound(&mut self) {
        let Some(clock) = self.clock else {
            return;
        };
        let place = clock.place(self.clock_now());
        self.sound.stop();
        self.status.sound = if self.has_sound() {
            Sound::Off
        } else {
            Sound::None
        };
        self.clock = Some(Clock::at(place, clock.speed(), clock.rate(), self.now()));
        if clock.speed() != 1 || !self.has_sound() {
            return;
        }
        let (Some(project), Some(index)) = (&self.project, self.timeline) else {
            return;
        };
        let timeline = &project.timelines[index];
        let nanos = Clock::at(place, 1, clock.rate(), 0).media_nanos(0);
        let from = i64::try_from(nanos * i128::from(timeline.sample_rate) / clock::NANOS);
        let started = from.as_ref().ok().copied();
        let result = from
            .map_err(|_| "timeline audio start is outside sample time".to_owned())
            .and_then(|from| {
                let output = self.sound.ready().ok_or("audio output is unavailable")?;
                let mixer = super::mix::Mixer::new(Arc::clone(project), index);
                output.start(mixer.source(output.format(), from))
            });
        match result {
            Ok(()) => {
                self.status.sound = Sound::On;
                self.heard.set(0);
                self.clock = Some(Clock::at(place, 1, clock.rate(), 0));
                self.mixed_from = started.map(|from| (from, timeline.sample_rate));
            }
            Err(message) => {
                self.status.sound = Sound::Unavailable;
                log(
                    "sound_unavailable",
                    &json!({"monitor":"timeline","message":message}),
                );
            }
        }
    }

    fn check_sound(&mut self) {
        if self.status.sound == Sound::On && self.sound.working().is_none() {
            if let Some(clock) = self.clock {
                let place = clock.place(self.heard.get());
                self.clock = Some(Clock::at(place, clock.speed(), clock.rate(), self.now()));
            }
            self.sound.stop();
            self.status.sound = Sound::Unavailable;
            log(
                "transport_sound_lost",
                &json!({"monitor":"timeline","message":"the sound failed; playback uses the system clock"}),
            );
            self.tell(true);
        }
    }

    fn wait(&self) -> Option<Duration> {
        if let Some(clock) = self.clock {
            let now = self.clock_now();
            let next = clock
                .frame(now)
                .saturating_add(i64::from(clock.speed().signum()));
            let nanos = (clock.when(next) - now).max(0);
            Some(Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX)).min(TOLD_EVERY))
        } else {
            self.settle
                .map(|at| at.saturating_duration_since(Instant::now()))
        }
    }

    fn run(mut self) {
        loop {
            let next = match self.wait() {
                Some(wait) => match self.inbox.recv_timeout(wait) {
                    Ok(command) => Some(command),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                },
                None => match self.inbox.recv() {
                    Ok(command) => Some(command),
                    Err(_) => return,
                },
            };
            let mut commands: Vec<Command> = next.into_iter().collect();
            commands.extend(self.inbox.try_iter());
            let mut merged = Vec::new();
            for command in commands {
                match (merged.last_mut(), command) {
                    (
                        Some(old @ Command::Act(Action::Seek { .. })),
                        new @ Command::Act(Action::Seek { .. }),
                    )
                    | (Some(old @ Command::Project(_)), new @ Command::Project(_)) => *old = new,
                    (
                        Some(Command::Act(Action::Step { frames })),
                        Command::Act(Action::Step { frames: more }),
                    ) => *frames = frames.saturating_add(more),
                    (_, command) => merged.push(command),
                }
            }
            for command in merged {
                if matches!(command, Command::Stop) {
                    self.unload();
                    return;
                }
                self.apply(command);
            }
            self.tick();
        }
    }

    fn tell(&mut self, force: bool) {
        lock(&self.shared).status = self.status.clone();
        if force || self.told.is_none_or(|at| at.elapsed() >= TOLD_EVERY) {
            (self.notify)(&json!(self.status));
            self.told = Some(Instant::now());
        }
    }

    fn pause(&mut self) {
        self.sound.stop();
        self.status.sound = if self.has_sound() {
            Sound::Off
        } else {
            Sound::None
        };
        self.clock = None;
        self.fine = None;
        self.status.playing = false;
    }

    fn unload(&mut self) {
        self.pause();
        self.settle = None;
        self.project = None;
        self.timeline = None;
        self.readers.clear();
        self.previous.clear();
        self.current = None;
        self.targets.clear();
        self.status = Status {
            proxies: self.status.proxies,
            ..Status::default()
        };
        if let Some(view) = lock(&self.view).as_ref() {
            view.clear();
        }
    }

    fn apply(&mut self, command: Command) {
        match command {
            Command::Load { project, id, reply } => {
                self.unload();
                self.status.timeline = Some(id);
                self.status.frame = Some(0);
                self.replace(project);
                let drawn = self.draw(true, false);
                lock(&self.shared).loading -= 1;
                self.tell(true);
                let _ = reply.send(drawn.map(|()| self.status.clone()));
            }
            Command::Project(project) => {
                if self.status.timeline.is_none() {
                    return;
                }
                self.replace(project);
                if !self.status.playing {
                    let _ = self.draw(true, false);
                }
                self.tell(true);
            }
            Command::Proxy(media) => {
                self.sources.proxy_ready(&media);
                if self.status.timeline.is_none() {
                    return;
                }
                self.invalidate();
                if !self.status.playing {
                    let _ = self.draw(true, false);
                }
                self.tell(true);
            }
            Command::Redraw => {
                if !self.status.playing {
                    let _ = self.draw(true, false);
                }
            }
            Command::Pause(reply) => {
                self.pause();
                if self.status.timeline.is_some() {
                    self.tell(true);
                }
                let _ = reply.send(());
            }
            Command::Digest(reply) => {
                let sound = (self.status.sound == Sound::On)
                    .then(|| {
                        self.sound.working().and_then(|output| {
                            let samples = output.played(Instant::now())?;
                            // Where the mixer is in the timeline: the sample that it started
                            // at, and as many more as have been heard, at the timeline's rate.
                            let mixed = self.mixed_from.map(|(from, rate)| {
                                from + i64::try_from(
                                    i128::from(samples) * i128::from(rate)
                                        / i128::from(output.format().rate),
                                )
                                .unwrap_or(i64::MAX)
                            });
                            Some((
                                samples,
                                self.clock.map(|clock| clock.frame(self.clock_now())),
                                mixed,
                            ))
                        })
                    })
                    .flatten();
                let result = self
                    .current
                    .as_ref()
                    .ok_or_else(not_loaded)
                    .and_then(|target| super::digest(target, self.status.shown, Monitor::Timeline))
                    .map(|mut digest| {
                        if let Some((samples, frame, mixed)) = sound {
                            digest["audio_samples"] = json!(samples);
                            digest["audio_frame"] = json!(frame);
                            digest["audio_sample"] = json!(mixed);
                        }
                        digest
                    });
                let _ = reply.send(result);
            }
            Command::Act(action) => self.action(action),
            Command::Stop => unreachable!("handled by run"),
        }
    }

    fn invalidate(&mut self) {
        self.generation += 1;
        self.previous.clear();
        self.request(self.status.frame.unwrap_or(0), false);
    }

    fn replace(&mut self, project: Arc<Project>) {
        let Some(id) = self.status.timeline.as_ref() else {
            return;
        };
        let Some(index) = project
            .timelines
            .iter()
            .position(|timeline| timeline.id == *id)
        else {
            self.unload();
            return;
        };
        let timeline = &project.timelines[index];
        let last = (resolve::duration(timeline) - 1).max(0);
        let frame = clock::clamped(self.status.frame.unwrap_or(0), last);
        self.status.last = Some(last);
        self.status.frame = Some(frame);
        if let Some(clock) = self.clock {
            // A changed duration/rate cannot leave the clock outside the new project.
            if clock.rate() != timeline.rate || clock.frame(self.clock_now()) > last {
                self.clock = Some(Clock::starting(
                    frame,
                    self.status.speed,
                    timeline.rate,
                    self.clock_now(),
                ));
            }
        }
        let tracks: Vec<_> = timeline
            .tracks
            .iter()
            .filter(|track| track.enabled && track.kind == TrackKind::Video)
            .map(|track| track.id.clone())
            .collect();
        self.readers.retain(|id, _| tracks.contains(id));
        let budget = usize::try_from(POOL_BYTES).unwrap_or(usize::MAX) / tracks.len().max(1);
        if self.reader_budget != budget {
            // Track counts can change during an edit. Keeping the old per-track budgets
            // would multiply the total allowance when tracks are added.
            self.readers.clear();
            self.reader_budget = budget;
        }
        for track in tracks {
            if !self.readers.contains_key(&track) {
                match TrackReader::new(self.sources.clone(), budget) {
                    Ok(reader) => {
                        self.readers.insert(track, reader);
                    }
                    Err(error) => Self::error(&error.to_string()),
                }
            }
        }
        self.project = Some(project);
        self.timeline = Some(index);
        self.invalidate();
        if self.status.playing {
            self.restart_sound();
        } else {
            self.status.sound = if self.has_sound() {
                Sound::Off
            } else {
                Sound::None
            };
        }
    }

    fn action(&mut self, action: Action) {
        let frame = self.status.frame.unwrap_or(0);
        let last = self.status.last.unwrap_or(0);
        match action {
            Action::Play { speed } => {
                if self.status.playing && self.status.speed == speed {
                    self.tell(true);
                    return;
                }
                let Some(project) = &self.project else {
                    return;
                };
                let Some(index) = self.timeline else {
                    return;
                };
                let rate = project.timelines[index].rate;
                self.settle = None;
                if let Some(clock) = self.clock {
                    self.clock = Some(clock.with_speed(speed, self.clock_now()));
                } else {
                    self.status.speed = speed;
                    let _ = self.draw(true, false);
                    // Prime the next frame before the clock begins, then read ahead continues
                    // independently while each current frame is composed/presented.
                    self.prime(frame.saturating_add(i64::from(speed.signum())));
                    self.clock = Some(Clock::starting(frame, speed, rate, self.now()));
                    self.status.late = 0;
                    self.fine = Some(crate::timer::Fine::new());
                }
                self.status.speed = speed;
                self.status.playing = true;
                self.restart_sound();
                self.request(frame, false);
            }
            Action::Pause => self.pause(),
            Action::Seek { frame, scrubbing } => {
                let resume = self.clock.filter(|_| !scrubbing);
                self.pause();
                let frame = clock::clamped(frame, last);
                self.status.frame = Some(frame);
                self.settle = scrubbing.then(|| Instant::now() + SETTLE);
                let _ = self.draw(true, scrubbing);
                if let Some(clock) = resume {
                    self.prime(frame.saturating_add(i64::from(clock.speed().signum())));
                    self.clock = Some(Clock::starting(
                        frame,
                        clock.speed(),
                        clock.rate(),
                        self.now(),
                    ));
                    self.fine = Some(crate::timer::Fine::new());
                    self.status.playing = true;
                    self.restart_sound();
                }
            }
            Action::Step { frames } => {
                self.pause();
                self.settle = None;
                self.status.frame = Some(clock::stepped(frame, frames, last));
                let _ = self.draw(true, false);
            }
            Action::Proxies { on } => {
                self.status.proxies = on;
                self.invalidate();
                if !self.status.playing {
                    let _ = self.draw(true, false);
                }
            }
        }
        self.tell(true);
    }

    fn tick(&mut self) {
        self.check_sound();
        if let Some(clock) = self.clock {
            let target = clock.frame(self.clock_now());
            let frame = clock::clamped(target, self.status.last.unwrap_or(0));
            if self.status.frame != Some(frame) {
                self.status.frame = Some(frame);
                let _ = self.draw(false, false);
                self.request(frame, false);
                self.tell(false);
            }
            if (clock.speed() > 0 && target >= self.status.last.unwrap_or(0))
                || (clock.speed() < 0 && target <= 0)
            {
                self.pause();
                // The endpoint is exact even if read-ahead was late on the last tick.
                let _ = self.draw(true, false);
                self.tell(true);
            }
        } else if self.settle.is_some_and(|at| Instant::now() >= at) {
            self.settle = None;
            let _ = self.draw(true, false);
            self.tell(true);
        }
    }

    fn request(&self, frame: i64, scrubbing: bool) {
        let (Some(project), Some(timeline)) = (&self.project, self.timeline) else {
            return;
        };
        for (track, row) in project.timelines[timeline].tracks.iter().enumerate() {
            if let Some(reader) = self.readers.get(&row.id) {
                reader.request(Request {
                    project: Arc::clone(project),
                    timeline,
                    track,
                    frame,
                    speed: self.status.speed,
                    proxies: self.status.proxies,
                    scrubbing,
                    generation: self.generation,
                });
            }
        }
    }

    fn prime(&self, frame: i64) {
        self.request(frame, false);
        let (Some(project), Some(index)) = (&self.project, self.timeline) else {
            return;
        };
        let until = Instant::now() + SEEK_WAIT;
        for layer in resolve::video_at(&project.timelines[index], project, frame) {
            if let (resolve::Content::Media { media, tick }, Some(reader)) =
                (&layer.content, self.readers.get(&layer.track))
            {
                let want = Want {
                    clip: layer.clip,
                    media: media.clone(),
                    tick: *tick,
                    frame,
                };
                let _ = reader.wait(
                    self.generation,
                    &want,
                    true,
                    until.saturating_duration_since(Instant::now()),
                );
            }
        }
    }

    fn draw(&mut self, wait: bool, scrubbing: bool) -> Result<(), Fault> {
        let result = self.draw_inner(wait, scrubbing);
        if let Err(error) = &result {
            self.status.exact = false;
            Self::error(&error.message);
        }
        result
    }

    fn draw_inner(&mut self, blocking: bool, scrubbing: bool) -> Result<(), Fault> {
        let (Some(project), Some(index)) = (&self.project, self.timeline) else {
            return Ok(());
        };
        let project = Arc::clone(project);
        let timeline = &project.timelines[index];
        let frame = self.status.frame.unwrap_or(0);
        self.request(frame, scrubbing);
        let mut scene = Scene {
            width: timeline.width,
            height: timeline.height,
            subtitles: resolve::cues_at(timeline, frame)
                .iter()
                .map(|cue| cue.text.clone())
                .collect(),
            ..Scene::default()
        };
        let until = Instant::now() + SEEK_WAIT;
        let mut exact = true;
        let mut late = false;
        let mut original = false;
        let mut proxy = false;
        let mut previous = BTreeMap::new();
        for layer in resolve::video_at(timeline, &project, frame) {
            let source = match layer.content {
                resolve::Content::Color { rgba } => LayerSource::color(rgba),
                resolve::Content::Title { text, style } => LayerSource::Title(Text { text, style }),
                resolve::Content::Media { media, tick } => {
                    let want = Want {
                        clip: layer.clip.clone(),
                        media,
                        tick,
                        frame,
                    };
                    let read = self.readers.get(&layer.track).and_then(|reader| {
                        if blocking {
                            reader.wait(
                                self.generation,
                                &want,
                                !scrubbing,
                                until.saturating_duration_since(Instant::now()),
                            )
                        } else {
                            reader.ready(self.generation, &want, true)
                        }
                    });
                    let read = match read {
                        Some(Ok(read)) => Some(read),
                        other => {
                            if let Some(Err(error)) = other {
                                Self::error(&error);
                            }
                            late = true;
                            exact = false;
                            self.previous
                                .get(&(layer.track.clone(), layer.clip.clone()))
                                .or_else(|| {
                                    self.previous.iter().rev().find_map(|((track, _), read)| {
                                        (track == &layer.track).then_some(read)
                                    })
                                })
                                .cloned()
                        }
                    };
                    let Some(read) = read else {
                        continue;
                    };
                    exact &= read.exact;
                    original |= read.source == Source::Original;
                    proxy |= read.source == Source::Proxy;
                    previous.insert((layer.track, layer.clip), read.clone());
                    LayerSource::Picture(read.picture, read.rotation)
                }
            };
            scene.layers.push(SceneLayer {
                source,
                transform: layer.transform.into(),
                opacity: layer.opacity,
            });
        }
        // Keep only the layers being shown (at most two per track), not a full-resolution
        // picture of every clip ever visited. At a late cut, that track's old picture stays.
        self.previous = previous;
        self.present(&scene)?;
        self.status.shown = Some(frame);
        self.status.exact = exact;
        self.status.late += u64::from(late && self.status.playing);
        self.status.source = Some(match (original, proxy) {
            (true, true) => Source::Mixed,
            (_, true) => Source::Proxy,
            _ => Source::Original,
        });
        Ok(())
    }

    fn present(&mut self, scene: &Scene) -> Result<(), Fault> {
        let gpu = self
            .graphics
            .gpu()
            .map_err(|error| drawing(&error.to_string()))?;
        if self
            .compositor
            .as_ref()
            .is_none_or(|compositor| !Arc::ptr_eq(compositor.gpu(), &gpu))
        {
            self.current = None;
            self.targets.clear();
            self.compositor = Some(Compositor::new(Arc::clone(&gpu)));
        }
        self.targets
            .retain(|target| target.size() == (scene.width, scene.height));
        let free = self
            .targets
            .iter()
            .find(|target| {
                Arc::strong_count(target) == 1 && Arc::strong_count(target.texture()) == 1
            })
            .cloned();
        let target = if let Some(target) = free {
            target
        } else {
            let target = Arc::new(
                Target::preview(&gpu, scene.width, scene.height)
                    .map_err(|error| drawing(&error.to_string()))?,
            );
            self.targets.push(Arc::clone(&target));
            target
        };
        let _turn = self.graphics.draw_turn(true);
        self.compositor
            .as_mut()
            .expect("made above")
            .render(scene, &target)
            .map_err(|error| drawing(&error.to_string()))?;
        if let Some(view) = lock(&self.view).as_ref() {
            view.present_texture(Arc::clone(target.texture()), gpu)
                .map_err(|error| drawing(&error.to_string()))?;
        }
        self.current = Some(target);
        Ok(())
    }

    fn error(message: &str) {
        log(
            "transport_error",
            &json!({"monitor":"timeline", "message":message}),
        );
    }
}

fn drawing(message: &str) -> Fault {
    Fault::new(rpc::TRANSPORT_FAILED, message).with_data(json!({"reason":"draw"}))
}
