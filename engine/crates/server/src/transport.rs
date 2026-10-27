//! The transport of the source monitor: one media item at a time is loaded, played forward and
//! backward at several speeds, sought, scrubbed and stepped a frame at a time; its picture is
//! shown in the view (if one is attached) and its sound is played (see [`crate::audio`]).
//! Timelines are played in phase 1.
//!
//! # Threads
//!
//! - **The transport's own** (`columbina-transport`) owns the [`VideoDecoder`] of the media, the
//!   clock and what is shown. It takes the commands of the clients from a channel, a batch at a
//!   time (so that seeks that came while it decoded are taken together, and only the last is
//!   decoded), decodes, and hands the pictures to the view, which returns at once. The state of
//!   the engine is never locked while it decodes or waits: it is locked only to tell the
//!   clients, a moment at a time.
//! - **The converter** (`columbina-convert`) makes the frames that the decoder gives
//!   ([`Decoded`], on the graphics card or in memory) into [`Picture`]s, while the decoder
//!   decodes the next: copying a frame of 4K from the card takes 5–12 ms, decoding it 7–11 ms.
//!   It writes into a **pool** of up to [`POOL`] pictures, each used again once neither the
//!   transport nor the view holds it (the view lets a picture go when the next one is drawn);
//!   when every one is held, a new picture is made and let go after it is shown.
//! - **The sound**: the feeder of [`crate::audio`] and the device's own thread.
//!
//! # Playing
//!
//! Forward, the decoder decodes frame after frame, at most [`IN_FLIGHT`] at the converter at
//! once (a decoder on the card has only so many frames to give) and at most [`AHEAD`] decoded or
//! ready before they are shown. The frame shown at a time is the one that the clock is at
//! ([`clock::Clock`]): the newest ready frame at or before it; a frame is never shown before its
//! time. A frame that the decoder gives after its time has passed is not converted (skipped),
//! unless nothing was shown for [`STALE`]; where the decoder is so far behind that a key
//! frame lies between it and the clock, it seeks to that key frame (as it does at 4× and 8× on
//! media of long groups of frames, which it cannot decode at 8 times their rate).
//!
//! The clock: at speed 1 with sound, the sound that the device has played (its latency
//! counted) is the clock, so that the picture follows what is heard; otherwise (no sound, the
//! device unavailable, or any other speed, which is played without sound) the monotonic clock
//! of the system. The device tells where it is a run at a time, each run told up to 10 ms early
//! or late (WASAPI), so the clock is the monotonic clock plus how far the sound is ahead of it,
//! smoothed over some 40 ms ([`smoothed`]); and while it plays, the timers of the system are
//! made fine ([`crate::timer`]), so that the transport wakes for each frame in time. Backward,
//! the transport decodes the frame that the clock is at, seeking for each one: on media of long
//! groups of frames only some are decoded in time, and those are shown (the all-intra proxies
//! of Task 13b make it smooth). Playing stops at the last frame forward, and at frame 0
//! backward.
//!
//! # Seeking and scrubbing
//!
//! A seek decodes the frame sought (`exact`). A seek while **scrubbing** shows first the key
//! frame at or before the frame ([`VideoDecoder::keyframe_at_or_before`]), which is decoded at
//! once, and then, once no newer seek came for [`SETTLE`], the frame itself; meanwhile what is
//! told says `exact: false`. A key frame already shown is not decoded again.
//!
//! # The source of the pictures
//!
//! Everything that decodes goes through [`Loaded::show`] (a frame sought, stepped to, played
//! backward) and the forward loop of [`Worker::forward`], both on the one [`VideoDecoder`] of
//! [`Loaded`]: a proxy of Task 13b takes its place there, opened by `open` where the media has
//! one.

mod clock;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use columbina_media::{
    Converter, Decoded, Facts, Hardware, MediaError, Picture, Rate, Time, VideoDecoder,
};
use columbina_model::Project;
use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_json::value::RawValue;
use tokio::sync::oneshot;

use crate::audio::Output;
use crate::log::log;
use crate::rpc::{self, Fault};
use crate::view::ViewSlot;
use clock::{Clock, First, NANOS};

/// How many decoded frames may be at the converter at once.
pub const IN_FLIGHT: usize = 3;

/// How many frames may be decoded ahead of the one shown, at the converter or ready.
pub const AHEAD: usize = 8;

/// How many pictures the converter keeps to write into again: as many as may be held at once
/// while playing ([`AHEAD`], and four more: see [`RECENT_PLAYING`]).
pub const POOL: usize = AHEAD + 4;

/// How long a scrubbing seek waits for a newer one before the frame itself is decoded.
pub const SETTLE: Duration = Duration::from_millis(150);

/// How often the state is told at most while playing: 30 times a second.
pub const TOLD_EVERY: Duration = Duration::from_millis(33);

/// How long nothing new may be shown while playing before a frame that is late is shown
/// rather than dropped.
pub const STALE: Duration = Duration::from_millis(100);

/// How many recent pictures are kept to be shown again at once: those shown last, and those
/// that were ready when playing stopped, so that a step after a pause takes no decoding.
const RECENT: usize = 8;

/// How many of the pictures shown last are kept while playing: the pool has a picture for each
/// frame decoded ahead ([`AHEAD`]), the two that the view may hold (the one it drew and the one
/// it is to draw), and these.
const RECENT_PLAYING: usize = 2;

/// The fastest the transport plays, forward or backward.
pub const FASTEST: i32 = 8;

/// The transport, as the engine's methods reach it. It is cheap to clone: the transport's own
/// thread is the same.
#[derive(Clone)]
pub struct Transport {
    /// None in a headless engine, which shows nothing and plays no sound.
    inbox: Option<mpsc::Sender<Inbox>>,
    status: Arc<Mutex<Status>>,
}

/// What the clients are told of the transport, in `transport.state`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Status {
    /// The media loaded, if any.
    pub media: Option<String>,
    /// The playhead: the frame shown while playing, the frame sought while scrubbing.
    pub frame: Option<i64>,
    pub playing: bool,
    /// The speed it plays at, or played at last: −8 to 8, not 0.
    pub speed: i32,
    /// Whether the picture shown is the frame at the playhead, not a key frame standing in.
    pub exact: bool,
    /// The frame whose picture is shown.
    pub shown: Option<i64>,
    /// The last frame of the media.
    pub last: Option<i64>,
    pub sound: Sound,
}

impl Status {
    fn unloaded() -> Status {
        Status {
            media: None,
            frame: None,
            playing: false,
            speed: 1,
            exact: true,
            shown: None,
            last: None,
            sound: Sound::None,
        }
    }
}

/// What becomes of the sound of the media.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Sound {
    /// The media has none (or none is loaded).
    None,
    /// It is not played now: paused, or played at a speed other than 1.
    Off,
    /// It is played, and is the clock.
    On,
    /// It cannot be played: no output device, or the device failed. The picture plays alone.
    Unavailable,
}

/// What the transport tells the clients: the line of each `transport.state`, sent to every
/// connection that said hello.
pub type Notify = Arc<dyn Fn(&Status) + Send + Sync>;

/// What comes to the transport's thread.
enum Inbox {
    Command(Command),
    Converted(Converted),
}

/// What a client asked of the transport.
enum Command {
    Load {
        media: String,
        path: PathBuf,
        facts: Facts,
        reply: oneshot::Sender<Result<Status, Fault>>,
    },
    /// Unload the media, if it is still the one loaded.
    Unload {
        media: String,
    },
    Play {
        speed: i32,
    },
    Pause,
    Seek {
        frame: i64,
        scrubbing: bool,
        at: Instant,
    },
    Step {
        frames: i64,
    },
    /// Show the picture again, in a view that was attached since.
    Redraw,
}

/// A frame for the converter.
struct Job {
    generation: u64,
    decoded: Decoded,
}

/// A picture that the converter made.
struct Converted {
    generation: u64,
    frame: i64,
    picture: Result<Arc<Picture>, MediaError>,
}

/// The params of `transport.load`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Load {
    pub media: String,
}

/// The params of `transport.play`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Play {
    #[serde(default = "one")]
    speed: i32,
}

fn one() -> i32 {
    1
}

/// The params of `transport.seek`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Seek {
    frame: i64,
    #[serde(default)]
    scrubbing: bool,
}

/// The params of `transport.step`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    frames: i64,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Transport {
    /// The transport of an engine; with its threads, unless the engine is headless. `mute`
    /// plays silence in the place of the sound (for the tests).
    pub fn new(headless: bool, mute: bool, view: ViewSlot, notify: Notify) -> Transport {
        let status = Arc::new(Mutex::new(Status::unloaded()));
        if headless {
            return Transport {
                inbox: None,
                status,
            };
        }
        let (inbox, received) = mpsc::channel();
        let (jobs, taken) = mpsc::channel();
        let generation = Arc::new(AtomicU64::new(0));
        let started = std::thread::Builder::new()
            .name("columbina-convert".into())
            .spawn({
                let inbox = inbox.clone();
                let generation = Arc::clone(&generation);
                move || convert_all(&taken, &inbox, &generation)
            })
            .and_then(|_| {
                let status = Arc::clone(&status);
                std::thread::Builder::new()
                    .name("columbina-transport".into())
                    .spawn(move || {
                        Worker {
                            pipe: Pipe {
                                inbox: received,
                                jobs,
                                generation,
                                waiting: VecDeque::new(),
                            },
                            world: World {
                                status,
                                notify,
                                view,
                                epoch: Instant::now(),
                                told: None,
                            },
                            output: None,
                            mute,
                            loaded: None,
                        }
                        .run();
                    })
            });
        if let Err(error) = started {
            log(
                "transport_failed",
                &json!({ "message": format!("the transport cannot be started: {error}") }),
            );
        }
        Transport {
            inbox: Some(inbox),
            status,
        }
    }

    /// Refuse a method that is not the transport's, or any in a headless engine.
    ///
    /// # Errors
    /// The engine is headless, or there is no such method.
    pub fn check(&self, method: &str) -> Result<(), Fault> {
        if self.inbox.is_none() {
            return Err(Fault::new(
                rpc::METHOD_NOT_FOUND,
                format!(
                    "there is no method {method} in a headless engine: it has nothing to show \
                     or to play the sound on"
                ),
            ));
        }
        if matches!(
            method,
            "transport.load"
                | "transport.play"
                | "transport.pause"
                | "transport.seek"
                | "transport.step"
                | "transport.state"
        ) {
            Ok(())
        } else {
            Err(Fault::new(
                rpc::METHOD_NOT_FOUND,
                format!("there is no method {method}"),
            ))
        }
    }

    /// The state of the transport, as it was last told.
    pub fn status(&self) -> Status {
        lock(&self.status).clone()
    }

    fn send(&self, command: Command) -> Result<(), Fault> {
        self.inbox
            .as_ref()
            .and_then(|inbox| inbox.send(Inbox::Command(command)).ok())
            .ok_or_else(|| {
                Fault::new(rpc::TRANSPORT_FAILED, "the transport has stopped")
                    .with_data(json!({ "reason": "failed" }))
            })
    }

    /// `transport.load`: load a media item of the project, in the place of the one loaded, and
    /// show its frame 0; answered once that is done, with the state.
    ///
    /// # Errors
    /// Its file cannot be opened, or its first frame cannot be decoded (`-32002`).
    pub async fn load(&self, media: String, path: PathBuf, facts: Facts) -> Result<Status, Fault> {
        let (reply, replied) = oneshot::channel();
        self.send(Command::Load {
            media,
            path,
            facts,
            reply,
        })?;
        replied.await.map_err(|_| {
            Fault::new(rpc::TRANSPORT_FAILED, "the transport has stopped")
                .with_data(json!({ "reason": "failed" }))
        })?
    }

    /// The methods of the transport other than `transport.load`, which are answered at once:
    /// what they do is told in `transport.state`.
    ///
    /// # Errors
    /// The params are not what the method takes, or no media is loaded.
    pub fn call(
        &self,
        method: &str,
        params: Option<&RawValue>,
    ) -> Result<serde_json::Value, Fault> {
        self.check(method)?;
        if method == "transport.state" {
            rpc::no_params(method, params)?;
            return Ok(serde_json::to_value(self.status()).unwrap_or_default());
        }
        let command = match method {
            "transport.play" => {
                let play: Play = rpc::params(method, r#"{"speed": 1}"#, params)?;
                if play.speed == 0 || play.speed.abs() > FASTEST {
                    return Err(Fault::params(format!(
                        "the speed of transport.play is a whole number from -{FASTEST} to \
                         {FASTEST}, not 0, and {} is not one",
                        play.speed
                    ))
                    .with_data(json!({ "reason": "speed" })));
                }
                Command::Play { speed: play.speed }
            }
            "transport.pause" => {
                rpc::no_params(method, params)?;
                Command::Pause
            }
            "transport.seek" => {
                let seek: Seek =
                    rpc::params(method, r#"{"frame": 0, "scrubbing": false}"#, params)?;
                Command::Seek {
                    frame: seek.frame,
                    scrubbing: seek.scrubbing,
                    at: Instant::now(),
                }
            }
            "transport.step" => {
                let step: Step = rpc::params(method, r#"{"frames": 1}"#, params)?;
                Command::Step {
                    frames: step.frames,
                }
            }
            _ => unreachable!("checked above"),
        };
        if lock(&self.status).media.is_none() {
            return Err(
                Fault::new(rpc::TRANSPORT_FAILED, "no media is loaded in the transport")
                    .with_data(json!({ "reason": "not_loaded" })),
            );
        }
        self.send(command)?;
        Ok(json!({}))
    }

    /// The project changed: the media loaded is unloaded if it was removed. This only reads the
    /// state of the transport and sends to its thread: it may be called while the state of
    /// the engine is locked.
    pub fn project_changed(&self, project: &Project) {
        let Some(media) = lock(&self.status).media.clone() else {
            return;
        };
        if !project.media.iter().any(|item| item.id == media) {
            let _ = self.send(Command::Unload { media });
        }
    }

    /// A view was attached: show it the picture.
    pub fn redraw(&self) {
        if lock(&self.status).media.is_some() {
            let _ = self.send(Command::Redraw);
        }
    }
}

/// Take commands that came together as one: of seeks one after another, the last; of steps
/// one after another, their sum. Everything else is kept, in order.
fn coalesce(commands: Vec<Command>) -> Vec<Command> {
    let mut taken: Vec<Command> = Vec::with_capacity(commands.len());
    for command in commands {
        match (taken.last_mut(), command) {
            (Some(last @ Command::Seek { .. }), seek @ Command::Seek { .. }) => *last = seek,
            (Some(Command::Step { frames }), Command::Step { frames: more }) => {
                *frames = frames.saturating_add(more);
            }
            (_, command) => taken.push(command),
        }
    }
    taken
}

/// Make each frame that comes a picture, in a picture of the pool where one is free, until the
/// transport ends. A frame of a generation that has passed is let go unconverted.
fn convert_all(jobs: &mpsc::Receiver<Job>, inbox: &mpsc::Sender<Inbox>, current: &AtomicU64) {
    let mut converter = Converter::new();
    let mut pool: Vec<Arc<Picture>> = Vec::new();
    for job in jobs {
        if job.generation != current.load(Ordering::Acquire) {
            continue;
        }
        let frame = job.decoded.frame();
        let picture = make(&mut converter, &mut pool, &job.decoded);
        drop(job.decoded);
        let made = Converted {
            generation: job.generation,
            frame,
            picture,
        };
        if inbox.send(Inbox::Converted(made)).is_err() {
            return;
        }
    }
}

/// A frame made a picture: in a picture of the pool that no one else holds, or a new one put
/// in the pool, or, when the pool is full and all are held, a new one that is not kept.
fn make(
    converter: &mut Converter,
    pool: &mut Vec<Arc<Picture>>,
    decoded: &Decoded,
) -> Result<Arc<Picture>, MediaError> {
    let free = pool
        .iter_mut()
        .position(|kept| Arc::get_mut(kept).is_some());
    let index = match free {
        Some(index) => index,
        None if pool.len() < POOL => {
            pool.push(Arc::new(Picture::default()));
            pool.len() - 1
        }
        None => {
            let mut picture = Picture::default();
            converter.picture_into(decoded, &mut picture)?;
            return Ok(Arc::new(picture));
        }
    };
    let slot = Arc::get_mut(&mut pool[index]).expect("a free picture is held by no one else");
    converter.picture_into(decoded, slot)?;
    Ok(Arc::clone(&pool[index]))
}

/// The way to the converter and back.
struct Pipe {
    inbox: mpsc::Receiver<Inbox>,
    jobs: mpsc::Sender<Job>,
    /// Raised to let go of what is at the converter: what it makes for an earlier generation
    /// is dropped.
    generation: Arc<AtomicU64>,
    /// Commands that came while a picture was waited for.
    waiting: VecDeque<Command>,
}

impl Pipe {
    fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Let go of what is at the converter.
    fn flush(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::AcqRel) + 1
    }

    fn send(&self, decoded: Decoded) {
        let _ = self.jobs.send(Job {
            generation: self.generation(),
            decoded,
        });
    }

    /// A frame made a picture now, by the converter, while what is at it is let go. Commands
    /// that come meanwhile are kept for later.
    fn convert_now(&mut self, decoded: Decoded) -> Result<Arc<Picture>, MediaError> {
        let generation = self.flush();
        self.send(decoded);
        loop {
            match self.inbox.recv_timeout(Duration::from_secs(30)) {
                Ok(Inbox::Converted(converted)) if converted.generation == generation => {
                    return converted.picture;
                }
                Ok(Inbox::Converted(_)) => {}
                Ok(Inbox::Command(command)) => self.waiting.push_back(command),
                Err(_) => {
                    return Err(MediaError::Decode(
                        "the converter of the transport does not answer".into(),
                    ))
                }
            }
        }
    }
}

/// What the transport's thread tells and shows.
struct World {
    status: Arc<Mutex<Status>>,
    notify: Notify,
    view: ViewSlot,
    /// The start of the monotonic clock.
    epoch: Instant,
    /// What was told last, and when.
    told: Option<(Status, Instant)>,
}

impl World {
    /// The monotonic clock, in nanoseconds.
    fn now(&self) -> i128 {
        i128::try_from(self.epoch.elapsed().as_nanos()).unwrap_or(i128::MAX)
    }

    /// Hand a picture to the view, if one is attached.
    fn present(&self, picture: &Arc<Picture>, rotation: i16) {
        if let Some(view) = lock(&self.view).as_ref() {
            view.present(Arc::clone(picture), rotation);
        }
    }

    fn clear(&self) {
        if let Some(view) = lock(&self.view).as_ref() {
            view.clear();
        }
    }

    /// Keep the state for the clients, and tell it: at once if `now` (after a command, even
    /// one that changed nothing, so that whoever sent it hears back), else if it changed, no
    /// more often than [`TOLD_EVERY`].
    fn tell(&mut self, status: Status, now: bool) {
        *lock(&self.status) = status.clone();
        if let Some((told, at)) = &self.told {
            if !now && (*told == status || at.elapsed() < TOLD_EVERY) {
                return;
            }
        }
        (self.notify)(&status);
        self.told = Some((status, Instant::now()));
    }
}

/// The media loaded, and what the transport does with it.
struct Loaded {
    media: String,
    path: PathBuf,
    rotation: i16,
    rate: Rate,
    /// The time of frame 0 in the file, for the sound.
    start: Time,
    /// The last frame: as the facts say, until the decoder finds the end.
    last: i64,
    decoder: VideoDecoder,
    has_sound: bool,
    /// The picture shown, and so its frame.
    picture: Option<Arc<Picture>>,
    /// The pictures shown last, and those that were ready when playing stopped.
    recent: VecDeque<Arc<Picture>>,
    playhead: i64,
    exact: bool,
    speed: i32,
    mode: Mode,
}

enum Mode {
    Paused,
    /// A key frame stands in for `target`, which is decoded at `due`.
    Scrubbing {
        target: i64,
        due: Instant,
    },
    Playing(Box<Playing>),
}

/// What playing is at.
struct Playing {
    clock: Clock,
    /// Whether the sound is the clock.
    sound: bool,
    /// While the sound is the clock: how far its time is ahead of the monotonic clock, as it is
    /// read, smoothed ([`smoothed`]); none until the sound is first heard.
    offset: Option<i128>,
    /// The latest time of the clock that was read, which it never goes back from.
    latest: std::cell::Cell<i128>,
    /// Fine timers of the system, while it plays.
    _fine: crate::timer::Fine,
    /// Forward: pictures ready to be shown, in order.
    ready: VecDeque<Arc<Picture>>,
    /// Forward: how many frames are at the converter.
    in_flight: usize,
    /// Forward: the decoder gave every frame up to the end.
    decoded_all: bool,
    /// Forward: the frame the decoder gives next.
    next: i64,
    /// When a picture was last shown.
    shown_at: Instant,
    stats: Stats,
}

/// How playing went, for the log.
struct Stats {
    started: Instant,
    presented: u64,
    /// Frames decoded after their time, not converted.
    skipped: u64,
    /// Frames converted, but passed over by a newer one before they were shown.
    passed: u64,
    late: u64,
    worst_late: i128,
    total_late: i128,
    /// When the sound was first heard, and how much of it had been by then, in nanoseconds.
    heard: Option<(Instant, i128)>,
    /// The last time of the sound, and when it was read.
    sound_now: Option<(Instant, i128)>,
    /// The most that the sound moved more or less than the monotonic clock between two
    /// readings, in nanoseconds.
    jitter: i128,
}

impl Stats {
    fn new() -> Stats {
        Stats {
            started: Instant::now(),
            presented: 0,
            skipped: 0,
            passed: 0,
            jitter: 0,
            late: 0,
            worst_late: 0,
            total_late: 0,
            heard: None,
            sound_now: None,
        }
    }
}

/// What the transport's thread owns.
struct Worker {
    pipe: Pipe,
    world: World,
    /// The output device of the sound, once media with sound was loaded: or why there is none.
    output: Option<Result<Output, String>>,
    mute: bool,
    loaded: Option<Loaded>,
}

impl Worker {
    fn run(mut self) {
        loop {
            let mut commands: Vec<Command> = self.pipe.waiting.drain(..).collect();
            let mut messages = Vec::new();
            if commands.is_empty() {
                // Every sender gone, the transport ends.
                match self.wait() {
                    None => match self.pipe.inbox.recv() {
                        Ok(message) => messages.push(message),
                        Err(_) => return,
                    },
                    Some(wait) => match self.pipe.inbox.recv_timeout(wait) {
                        Ok(message) => messages.push(message),
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    },
                }
            }
            while let Ok(message) = self.pipe.inbox.try_recv() {
                messages.push(message);
            }
            for message in messages {
                match message {
                    Inbox::Command(command) => commands.push(command),
                    Inbox::Converted(converted) => self.converted(converted),
                }
            }
            for command in coalesce(commands) {
                self.apply(command);
            }
            self.work();
        }
    }

    /// How long to wait for a command before there is something to do; none while there is
    /// nothing to do until one comes.
    fn wait(&self) -> Option<Duration> {
        let loaded = self.loaded.as_ref()?;
        match &loaded.mode {
            Mode::Paused => None,
            Mode::Scrubbing { due, .. } => Some(due.saturating_duration_since(Instant::now())),
            Mode::Playing(playing) => {
                let now = clock_time(playing, &self.world);
                let target = playing.clock.frame(now);
                let until = |frame: i64| {
                    let nanos = (playing.clock.when(frame) - now).max(0);
                    Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
                };
                let most =
                    TOLD_EVERY.min(Duration::from_millis(if playing.sound { 5 } else { 20 }));
                let wait = if playing.clock.speed() < 0 {
                    if loaded.picture.as_ref().map(|picture| picture.frame) == Some(target) {
                        until(target - 1)
                    } else {
                        Duration::ZERO
                    }
                } else if !playing.decoded_all
                    && playing.in_flight < IN_FLIGHT
                    && playing.in_flight + playing.ready.len() < AHEAD
                {
                    Duration::ZERO
                } else {
                    match playing.ready.front() {
                        Some(front) => until(front.frame),
                        None if playing.in_flight > 0 => most,
                        // Every frame decoded and shown: until the clock reaches the last.
                        None if playing.decoded_all => until(loaded.last),
                        None => Duration::ZERO,
                    }
                };
                Some(wait.min(most))
            }
        }
    }

    fn status(&self) -> Status {
        let Some(loaded) = &self.loaded else {
            return Status::unloaded();
        };
        let output = self.output.as_ref();
        let unavailable = match output {
            Some(Ok(output)) => output.failed().is_some(),
            Some(Err(_)) | None => true,
        };
        let (playing, sounding) = match &loaded.mode {
            Mode::Playing(playing) => (true, playing.sound),
            _ => (false, false),
        };
        let shown = loaded.picture.as_ref().map(|picture| picture.frame);
        Status {
            media: Some(loaded.media.clone()),
            frame: Some(if playing {
                shown.unwrap_or(loaded.playhead)
            } else {
                loaded.playhead
            }),
            playing,
            speed: loaded.speed,
            exact: loaded.exact,
            shown,
            last: Some(loaded.last),
            sound: if !loaded.has_sound {
                Sound::None
            } else if sounding {
                Sound::On
            } else if unavailable {
                Sound::Unavailable
            } else {
                Sound::Off
            },
        }
    }

    fn tell(&mut self, now: bool) {
        let status = self.status();
        self.world.tell(status, now);
    }

    fn apply(&mut self, command: Command) {
        match command {
            Command::Load {
                media,
                path,
                facts,
                reply,
            } => {
                self.unload();
                let opened = self.open(media, path, &facts);
                if opened.is_err() {
                    self.world.clear();
                }
                self.tell(true);
                let _ = reply.send(opened.map(|()| self.status()));
            }
            Command::Unload { media } => {
                if self
                    .loaded
                    .as_ref()
                    .is_some_and(|loaded| loaded.media == media)
                {
                    self.unload();
                    self.world.clear();
                    log("transport_unloaded", &json!({ "media": media }));
                    self.tell(true);
                }
            }
            Command::Play { speed } => self.play(speed),
            Command::Pause => {
                self.pause();
                self.tell(true);
            }
            Command::Seek {
                frame,
                scrubbing,
                at,
            } => self.seek(frame, scrubbing, at),
            Command::Step { frames } => self.step(frames),
            Command::Redraw => {
                if let Some(loaded) = &self.loaded {
                    if let Some(picture) = &loaded.picture {
                        self.world.present(picture, loaded.rotation);
                    }
                }
            }
        }
    }

    /// Let the media go, and stop its sound.
    fn unload(&mut self) {
        self.pause();
        self.pipe.flush();
        self.loaded = None;
    }

    /// Open the media and show its frame 0.
    fn open(&mut self, media: String, path: PathBuf, facts: &Facts) -> Result<(), Fault> {
        let text = path.to_string_lossy().into_owned();
        let cannot = |error: &MediaError| {
            crate::session::cannot_read(&text, error.reason(), &error.to_string())
        };
        let decoder = VideoDecoder::open(&path, Hardware::Auto).map_err(|error| cannot(&error))?;
        let fallbacks: Vec<_> = decoder
            .fallbacks()
            .iter()
            .map(|(device, why)| json!({ "device": device.name(), "why": why }))
            .collect();
        log(
            "transport_loaded",
            &json!({ "media": media, "decoder": decoder.path_used().name(), "fallbacks": fallbacks }),
        );
        let rate = decoder.rate();
        let has_sound = facts.audio.is_some();
        if has_sound && !matches!(self.output, Some(Ok(_))) {
            let output = Output::open(self.mute);
            match &output {
                Ok(output) => log(
                    "sound_ready",
                    &json!({
                        "device": output.name(),
                        "rate": output.format().rate,
                        "channels": output.format().channels,
                    }),
                ),
                Err(message) => log("sound_unavailable", &json!({ "message": message })),
            }
            self.output = Some(output);
        }
        let mut loaded = Loaded {
            media,
            path,
            rotation: facts.video.as_ref().map_or(0, |video| video.rotation),
            rate,
            start: decoder.start(),
            last: clock::last_frame(facts.duration, rate),
            decoder,
            has_sound,
            picture: None,
            recent: VecDeque::new(),
            playhead: 0,
            exact: true,
            speed: 1,
            mode: Mode::Paused,
        };
        loaded
            .show_exact(0, &mut self.pipe, &self.world)
            .map_err(|error| cannot(&error))?;
        self.loaded = Some(loaded);
        Ok(())
    }

    /// Play at `speed`, from the playhead, or, if it plays already, from where it is.
    fn play(&mut self, speed: i32) {
        let Some(loaded) = &mut self.loaded else {
            return;
        };
        let has_sound = loaded.has_sound;
        let then = match &mut loaded.mode {
            Mode::Playing(playing) if playing.clock.speed() == speed => None,
            Mode::Playing(playing) => {
                let now = clock_time(playing, &self.world);
                // Faster or slower forward, without the sound before or after: the frames
                // decoded ahead are still the ones to show, and the clock goes on.
                let keep = playing.clock.speed() > 0
                    && speed > 0
                    && !playing.sound
                    && !(speed == 1 && has_sound);
                if keep {
                    playing.clock = playing.clock.with_speed(speed, now);
                    loaded.speed = speed;
                    None
                } else {
                    Some(Some(playing.clock.with_speed(speed, now).place(now)))
                }
            }
            Mode::Scrubbing { .. } | Mode::Paused => Some(None),
        };
        if let Some(place) = then {
            // From where it is, at the new speed: the place does not jump.
            self.pause();
            self.start(speed, place);
        }
        self.tell(true);
    }

    /// Start playing at `speed` from a place, or from the playhead.
    fn start(&mut self, speed: i32, place: Option<i128>) {
        let Some(loaded) = &mut self.loaded else {
            return;
        };
        let rate = loaded.rate;
        let place =
            place.unwrap_or_else(|| Clock::starting(loaded.playhead, speed, rate, 0).place(0));
        let frame = clock::clamped(clock::frame_of(place, rate), loaded.last);
        loaded.speed = speed;
        loaded.playhead = frame;
        if (speed > 0 && frame >= loaded.last) || (speed < 0 && frame <= 0) {
            // At the end it goes no further.
            if let Err(error) = loaded.show_exact(frame, &mut self.pipe, &self.world) {
                failed(&loaded.media, &error);
            }
            loaded.mode = Mode::Paused;
            return;
        }
        let shown = loaded.picture.as_ref().map(|picture| picture.frame);
        let mut ready = Ok(());
        if shown != Some(frame) || !loaded.exact {
            ready = loaded.show_exact(frame, &mut self.pipe, &self.world);
        }
        if speed > 0 && ready.is_ok() {
            // The decoder is to give the frames after it next: it gives the frame it gave
            // last again without decoding.
            ready = loaded.decoder.decoded_at(frame).map(drop);
        }
        if let Err(error) = ready {
            failed(&loaded.media, &error);
            loaded.mode = Mode::Paused;
            return;
        }
        self.pipe.flush();
        let place = clock::clamped_place(place, frame, speed, rate);
        // What was kept for steps is let go, for the pool to use again.
        loaded.recent.clear();
        let mut sound = false;
        if speed == 1 && loaded.has_sound {
            if let Some(Ok(output)) = &mut self.output {
                let media = Clock::at(place, 1, rate, 0).media_nanos(0);
                let from = clock::sample_at(loaded.start, media, output.format().rate);
                match output.start(&loaded.path, from) {
                    Ok(()) => sound = true,
                    Err(message) => {
                        log("sound_failed", &json!({ "message": message }));
                    }
                }
            }
        }
        let since = if sound { 0 } else { self.world.now() };
        loaded.mode = Mode::Playing(Box::new(Playing {
            clock: Clock::at(place, speed, rate, since),
            sound,
            offset: None,
            latest: std::cell::Cell::new(since),
            _fine: crate::timer::Fine::new(),
            ready: VecDeque::new(),
            in_flight: 0,
            decoded_all: false,
            next: frame + 1,
            shown_at: Instant::now(),
            stats: Stats::new(),
        }));
    }

    /// Stop playing where it is, with the frame shown as the playhead; keep the pictures that
    /// were ready, to be shown at once by a step.
    fn pause(&mut self) {
        let Some(loaded) = &mut self.loaded else {
            return;
        };
        let Mode::Playing(playing) = std::mem::replace(&mut loaded.mode, Mode::Paused) else {
            return;
        };
        let playing = *playing;
        if playing.sound {
            if let Some(Ok(output)) = &mut self.output {
                output.stop();
            }
        }
        self.pipe.flush();
        if let Some(picture) = &loaded.picture {
            loaded.playhead = picture.frame;
        }
        loaded.exact = true;
        for picture in playing.ready {
            loaded.remember(picture);
        }
        let stats = &playing.stats;
        let seconds = stats.started.elapsed().as_secs_f64();
        #[allow(clippy::cast_precision_loss, reason = "a few frames, for the log")]
        let mean_late = if stats.presented > 0 {
            stats.total_late as f64 / stats.presented as f64 / 1e6
        } else {
            0.0
        };
        #[allow(clippy::cast_precision_loss, reason = "milliseconds, for the log")]
        let drift = match (stats.heard, stats.sound_now) {
            (Some((first, heard)), Some((last, now))) => {
                let sound = (now - heard) as f64 / 1e6;
                let system = last.duration_since(first).as_secs_f64() * 1e3;
                Some(sound - system)
            }
            _ => None,
        };
        #[allow(clippy::cast_precision_loss, reason = "milliseconds, for the log")]
        let (worst, jitter) = (stats.worst_late as f64 / 1e6, stats.jitter as f64 / 1e6);
        log(
            "transport_played",
            &json!({
                "media": loaded.media,
                "speed": playing.clock.speed(),
                "clock": if playing.sound { "sound" } else { "system" },
                "decoder": loaded.decoder.path_used().name(),
                "seconds": seconds,
                "presented": stats.presented,
                "skipped": stats.skipped,
                "passed": stats.passed,
                "sound_jitter_ms": jitter,
                "late": stats.late,
                "late_worst_ms": worst,
                "late_mean_ms": mean_late,
                "sound_minus_system_ms": drift,
            }),
        );
    }

    fn seek(&mut self, frame: i64, scrubbing: bool, at: Instant) {
        let Some(loaded) = &mut self.loaded else {
            return;
        };
        let target = clock::clamped(frame, loaded.last);
        let playing = match &loaded.mode {
            Mode::Playing(playing) => Some(playing.clock.speed()),
            _ => None,
        };
        if let (Some(speed), false) = (playing, scrubbing) {
            // A seek while playing plays on from there.
            self.pause();
            if let Some(loaded) = &mut self.loaded {
                loaded.playhead = target;
            }
            let rate = loaded_rate(self.loaded.as_ref());
            self.start(
                speed,
                Some(Clock::starting(target, speed, rate, 0).place(0)),
            );
            self.tell(true);
            return;
        }
        self.pause();
        let Some(loaded) = &mut self.loaded else {
            return;
        };
        loaded.playhead = target;
        let shown = loaded.picture.as_ref().map(|picture| picture.frame);
        let first = if scrubbing {
            clock::scrub_first(target, loaded.decoder.keyframe_at_or_before(target), shown)
        } else {
            First::Exact
        };
        match first {
            First::Exact => {
                loaded.mode = Mode::Paused;
                match loaded.show_exact(target, &mut self.pipe, &self.world) {
                    Ok(()) => loaded.exact = true,
                    Err(error) => failed(&loaded.media, &error),
                }
            }
            First::Key(key) => {
                match loaded.show(key, &mut self.pipe, &self.world) {
                    Ok(()) => loaded.exact = key == target,
                    Err(error) => failed(&loaded.media, &error),
                }
                loaded.mode = Mode::Scrubbing {
                    target,
                    due: at + SETTLE,
                };
            }
            First::Keep => {
                loaded.exact = false;
                loaded.mode = Mode::Scrubbing {
                    target,
                    due: at + SETTLE,
                };
            }
        }
        self.tell(true);
    }

    fn step(&mut self, frames: i64) {
        self.pause();
        let Some(loaded) = &mut self.loaded else {
            return;
        };
        let target = clock::stepped(loaded.playhead, frames, loaded.last);
        loaded.playhead = target;
        loaded.mode = Mode::Paused;
        if let Err(error) = loaded.show_exact(target, &mut self.pipe, &self.world) {
            failed(&loaded.media, &error);
        }
        self.tell(true);
    }

    /// A picture came from the converter: ready to be shown, if it is of the frames being
    /// played.
    fn converted(&mut self, converted: Converted) {
        if converted.generation != self.pipe.generation() {
            return;
        }
        let Some(loaded) = &mut self.loaded else {
            return;
        };
        let Mode::Playing(playing) = &mut loaded.mode else {
            return;
        };
        playing.in_flight = playing.in_flight.saturating_sub(1);
        match converted.picture {
            Ok(picture) => playing.ready.push_back(picture),
            Err(MediaError::Device(why)) => {
                // The card failed: the decoder goes on on the processor, from this frame.
                log(
                    "transport_device_failed",
                    &json!({ "media": loaded.media, "frame": converted.frame, "message": why }),
                );
                loaded.decoder.device_failed(&why);
                self.pipe.flush();
                playing.in_flight = 0;
                match loaded.decoder.decoded_at(converted.frame) {
                    Ok(decoded) => {
                        playing.next = decoded.frame() + 1;
                        playing.in_flight = 1;
                        self.pipe.send(decoded);
                    }
                    Err(error) => failed(&loaded.media, &error),
                }
            }
            Err(error) => {
                playing.stats.passed += 1;
                failed(&loaded.media, &error);
            }
        }
    }

    /// Do what is due: decode the frame sought once scrubbing settles, or play on.
    fn work(&mut self) {
        let Some(loaded) = &mut self.loaded else {
            return;
        };
        match &loaded.mode {
            Mode::Scrubbing { target, due } if Instant::now() >= *due => {
                let target = *target;
                loaded.mode = Mode::Paused;
                match loaded.show_exact(target, &mut self.pipe, &self.world) {
                    Ok(()) => loaded.exact = true,
                    Err(error) => failed(&loaded.media, &error),
                }
                self.tell(true);
            }
            Mode::Playing(playing) => {
                let ended = if playing.clock.speed() > 0 {
                    self.forward()
                } else {
                    self.backward()
                };
                if ended {
                    self.pause();
                    self.tell(true);
                } else {
                    self.tell(false);
                }
            }
            Mode::Scrubbing { .. } | Mode::Paused => {}
        }
    }

    /// Play forward a step: show what is due, and decode one more frame if there is room.
    /// Whether it reached the end.
    fn forward(&mut self) -> bool {
        let Some(loaded) = &mut self.loaded else {
            return true;
        };
        let Mode::Playing(playing) = &mut loaded.mode else {
            return true;
        };
        if playing.sound {
            sound_check(playing, self.output.as_ref(), &self.world);
        }
        let now = clock_time(playing, &self.world);
        let target = playing.clock.frame(now);
        // The newest ready frame at or before the clock; those before it are dropped.
        let mut chosen: Option<Arc<Picture>> = None;
        while playing
            .ready
            .front()
            .is_some_and(|front| front.frame <= target)
        {
            let next = playing.ready.pop_front();
            if chosen.is_some() {
                playing.stats.passed += 1;
            }
            chosen = next;
        }
        if let Some(picture) = chosen {
            let late = playing.clock.late(picture.frame, now);
            let stats = &mut playing.stats;
            stats.presented += 1;
            stats.worst_late = stats.worst_late.max(late);
            stats.total_late += late;
            if picture.frame < target {
                stats.late += 1;
            }
            playing.shown_at = Instant::now();
            self.world.present(&picture, loaded.rotation);
            loaded.exact = true;
            loaded.playhead = picture.frame;
            let old = loaded.picture.replace(picture);
            if let Some(old) = old {
                loaded.recent.push_back(old);
                if loaded.recent.len() > RECENT_PLAYING {
                    loaded.recent.pop_front();
                }
            }
        }
        let Mode::Playing(playing) = &mut loaded.mode else {
            return true;
        };
        if target >= loaded.last && playing.in_flight == 0 && playing.ready.is_empty() {
            let shown = loaded.picture.as_ref().map(|picture| picture.frame);
            if shown != Some(loaded.last) {
                let last = loaded.last;
                if let Err(error) = loaded.show_exact(last, &mut self.pipe, &self.world) {
                    failed(&loaded.media, &error);
                }
            }
            return true;
        }
        if playing.decoded_all
            || playing.in_flight >= IN_FLIGHT
            || playing.in_flight + playing.ready.len() >= AHEAD
        {
            return false;
        }
        // Far behind, with a key frame between: on from the key frame. Aimed a little ahead
        // of the clock, for the time a frame takes to come.
        let speed = i64::from(playing.clock.speed());
        let ahead = (speed * i64::from(loaded.rate.num) / i64::from(loaded.rate.den.max(1))) / 10;
        let aim = clock::clamped(target + ahead, loaded.last);
        let jump = loaded
            .decoder
            .keyframe_at_or_before(aim)
            .filter(|&key| target - playing.next > 4 && key > playing.next + 4);
        let decoded = match jump {
            Some(key) => loaded.decoder.decoded_at(key).map(Some),
            None => loaded.decoder.next_decoded(),
        };
        match decoded {
            Ok(Some(decoded)) => {
                let frame = decoded.frame();
                playing.next = frame + 1;
                let now = clock_time(playing, &self.world);
                let late = frame < playing.clock.frame(now) && frame < loaded.last;
                if late && playing.shown_at.elapsed() < STALE {
                    playing.stats.skipped += 1;
                } else {
                    playing.in_flight += 1;
                    self.pipe.send(decoded);
                }
            }
            Ok(None) => {
                playing.decoded_all = true;
                if let Some(last) = loaded.decoder.last_frame() {
                    loaded.last = last;
                }
            }
            Err(error) => {
                failed(&loaded.media, &error);
                playing.decoded_all = true;
            }
        }
        false
    }

    /// Play backward a step: decode the frame that the clock is at, if it is not shown.
    /// Whether it reached frame 0.
    fn backward(&mut self) -> bool {
        let Some(loaded) = &mut self.loaded else {
            return true;
        };
        let Mode::Playing(playing) = &mut loaded.mode else {
            return true;
        };
        let now = clock_time(playing, &self.world);
        let target = clock::clamped(playing.clock.frame(now), loaded.last);
        let late = playing.clock.late(target, now);
        let shown = loaded.picture.as_ref().map(|picture| picture.frame);
        if shown != Some(target) {
            playing.stats.presented += 1;
            playing.stats.worst_late = playing.stats.worst_late.max(late);
            playing.stats.total_late += late;
            if let Err(error) = loaded.show_exact(target, &mut self.pipe, &self.world) {
                failed(&loaded.media, &error);
                return true;
            }
            loaded.playhead = target;
        }
        target <= 0
    }
}

/// The time of the clock that `playing` goes by, in nanoseconds, never going back: the time of
/// the sound heard, smoothed, or 0 until it is first heard; or the monotonic clock.
fn clock_time(playing: &Playing, world: &World) -> i128 {
    let now = if playing.sound {
        playing.offset.map_or(0, |offset| world.now() + offset)
    } else {
        world.now()
    };
    let now = now.max(playing.latest.get());
    playing.latest.set(now);
    now
}

/// How far the time of the sound is ahead of the monotonic clock, smoothed: a reading of the
/// device moves it an eighth of the way to what it reads, so that a run of the device heard a
/// little earlier or later than the last (up to 10 ms on WASAPI) does not move the picture; a
/// reading further than [`SNAP`] from it (as when the sound stalls) is taken as it is.
fn smoothed(offset: Option<i128>, read: i128) -> i128 {
    match offset {
        Some(offset) if (read - offset).abs() <= SNAP => offset + (read - offset) / 8,
        _ => read,
    }
}

/// How far a reading of the sound may be from the time smoothed before it is taken as it is.
const SNAP: i128 = 40_000_000;

/// How much of the sound started last has been heard, in nanoseconds of it.
fn sound_nanos(output: Option<&Result<Output, String>>, now: Instant) -> Option<i128> {
    let Some(Ok(output)) = output else {
        return None;
    };
    let played = output.played(now)?;
    Some(i128::from(played) * NANOS / i128::from(output.format().rate.max(1)))
}

/// Keep how the sound goes against the monotonic clock, for the log; where the device failed,
/// go on with the monotonic clock from where the sound was.
fn sound_check(playing: &mut Playing, output: Option<&Result<Output, String>>, world: &World) {
    let now = Instant::now();
    let heard = sound_nanos(output, now);
    if let Some(heard) = heard {
        if heard > 0 {
            playing.offset = Some(smoothed(playing.offset, heard - world.now()));
            let stats = &mut playing.stats;
            stats.heard.get_or_insert((now, heard));
            if let Some((then, before)) = stats.sound_now {
                let system = i128::try_from(now.duration_since(then).as_nanos()).unwrap_or(0);
                stats.jitter = stats.jitter.max((heard - before - system).abs());
            }
            stats.sound_now = Some((now, heard));
        }
    }
    let failed = match output {
        Some(Ok(output)) => output.failed().is_some(),
        _ => true,
    };
    if failed {
        let place = playing.clock.place(clock_time(playing, world));
        let speed = playing.clock.speed();
        let rate = playing.clock.rate();
        let now = world.now();
        playing.clock = Clock::at(place, speed, rate, now);
        playing.latest.set(now);
        playing.sound = false;
    }
}

fn loaded_rate(loaded: Option<&Loaded>) -> Rate {
    loaded.map_or(Rate { num: 25, den: 1 }, |loaded| loaded.rate)
}

/// Log what failed while the transport decoded.
fn failed(media: &str, error: &MediaError) {
    log(
        "transport_error",
        &json!({ "media": media, "reason": error.reason(), "message": error.to_string() }),
    );
}

impl Loaded {
    /// Keep a picture to be shown again at once.
    fn remember(&mut self, picture: Arc<Picture>) {
        self.recent.retain(|kept| kept.frame != picture.frame);
        self.recent.push_back(picture);
        while self.recent.len() > RECENT {
            self.recent.pop_front();
        }
    }

    /// Show `frame` as it is, and say it is the frame at the playhead.
    fn show_exact(&mut self, frame: i64, pipe: &mut Pipe, world: &World) -> Result<(), MediaError> {
        self.show(frame, pipe, world)?;
        self.exact = true;
        Ok(())
    }

    /// Show `frame`: from the recent pictures, or decoded and made a picture now. A frame past
    /// the end, which the facts of the media promised, is the last that is there.
    fn show(&mut self, frame: i64, pipe: &mut Pipe, world: &World) -> Result<(), MediaError> {
        if self
            .picture
            .as_ref()
            .is_some_and(|picture| picture.frame == frame)
        {
            return Ok(());
        }
        if let Some(index) = self.recent.iter().position(|kept| kept.frame == frame) {
            let picture = self.recent.remove(index).expect("it is there");
            self.present(picture, world);
            return Ok(());
        }
        let decoded = match self.decoder.decoded_at(frame) {
            Err(MediaError::OutOfRange {
                last: Some(last), ..
            }) if frame > last => {
                self.last = last;
                self.playhead = self.playhead.min(last);
                self.decoder.decoded_at(last)?
            }
            other => other?,
        };
        let frame = decoded.frame();
        let picture = match pipe.convert_now(decoded) {
            Err(MediaError::Device(why)) => {
                // The card failed as the frame was copied: the decoder decodes it again on the
                // processor.
                log(
                    "transport_device_failed",
                    &json!({ "media": self.media, "frame": frame, "message": why }),
                );
                self.decoder.device_failed(&why);
                let decoded = self.decoder.decoded_at(frame)?;
                pipe.convert_now(decoded)?
            }
            other => other?,
        };
        self.present(picture, world);
        Ok(())
    }

    fn present(&mut self, picture: Arc<Picture>, world: &World) {
        world.present(&picture, self.rotation);
        if let Some(old) = self.picture.replace(picture) {
            self.remember(old);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seek(frame: i64) -> Command {
        Command::Seek {
            frame,
            scrubbing: true,
            at: Instant::now(),
        }
    }

    /// What the commands are, as text.
    fn written(commands: &[Command]) -> Vec<String> {
        commands
            .iter()
            .map(|command| match command {
                Command::Seek { frame, .. } => format!("seek {frame}"),
                Command::Step { frames } => format!("step {frames}"),
                Command::Play { speed } => format!("play {speed}"),
                Command::Pause => "pause".into(),
                Command::Unload { media } => format!("unload {media}"),
                Command::Redraw => "redraw".into(),
                Command::Load { media, .. } => format!("load {media}"),
            })
            .collect()
    }

    #[test]
    fn of_seeks_one_after_another_only_the_last_is_taken() {
        let taken = coalesce((0..20).map(seek).collect());
        assert_eq!(written(&taken), ["seek 19"]);
        let taken = coalesce(vec![
            seek(1),
            seek(2),
            Command::Play { speed: 1 },
            seek(3),
            seek(4),
            Command::Pause,
            seek(5),
        ]);
        assert_eq!(
            written(&taken),
            ["seek 2", "play 1", "seek 4", "pause", "seek 5"]
        );
    }

    #[test]
    fn steps_one_after_another_are_taken_as_their_sum() {
        let step = |frames| Command::Step { frames };
        let taken = coalesce(vec![
            step(1),
            step(1),
            step(-3),
            seek(7),
            step(2),
            step(i64::MAX),
        ]);
        assert_eq!(
            written(&taken),
            ["step -1", "seek 7", format!("step {}", i64::MAX).as_str()]
        );
        assert!(coalesce(Vec::new()).is_empty());
    }

    #[test]
    fn the_time_of_the_sound_is_smoothed_unless_it_jumps() {
        const MS: i128 = 1_000_000;
        assert_eq!(smoothed(None, 7 * MS), 7 * MS);
        assert_eq!(smoothed(Some(0), 8 * MS), MS);
        assert_eq!(smoothed(Some(10 * MS), 2 * MS), 9 * MS);
        assert_eq!(smoothed(Some(0), 41 * MS), 41 * MS);
        assert_eq!(smoothed(Some(0), -41 * MS), -41 * MS);
        // Readings that wander by 10 ms about a time move it by little.
        let mut offset = Some(0);
        for read in [10, -10, 10, -10, 10, -10, 10, -10] {
            offset = Some(smoothed(offset, read * MS));
            assert!(offset.unwrap().abs() <= 2 * MS, "{offset:?}");
        }
    }

    #[test]
    fn a_picture_of_the_pool_is_used_again_once_no_one_holds_it() {
        let mut pool: Vec<Arc<Picture>> = (0..3).map(|_| Arc::new(Picture::default())).collect();
        let held = Arc::clone(&pool[0]);
        assert_eq!(
            pool.iter_mut()
                .position(|kept| Arc::get_mut(kept).is_some()),
            Some(1)
        );
        drop(held);
        assert_eq!(
            pool.iter_mut()
                .position(|kept| Arc::get_mut(kept).is_some()),
            Some(0)
        );
    }
}
