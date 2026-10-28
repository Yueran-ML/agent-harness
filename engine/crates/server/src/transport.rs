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
//!   clients, a moment at a time. If it panics, a guard tells everyone that nothing is loaded,
//!   and the methods of the transport are refused from then on.
//! - **The converter** (`columbina-convert`) makes the frames that the decoder gives
//!   ([`Decoded`], on the graphics card or in memory) into [`Picture`]s, while the decoder
//!   decodes the next: copying a frame of 4K from the card takes 5–12 ms, decoding it 7–11 ms.
//!   It writes into a **pool** of pictures, each used again once neither the transport nor the
//!   view holds it (the view lets a picture go when the next one is drawn); when every one is
//!   held, a new picture is made and let go after it is shown.
//! - **The sound**: the feeder of [`crate::audio`] and the device's own thread.
//!
//! # Memory
//!
//! The pictures held at once while playing are those decoded ahead, the two that the view may
//! hold (the one it drew and the one it is to draw), and the few last shown: [`Limits`] sizes
//! them to the pictures of the media so that they take at most [`POOL_BYTES`] (12 pictures
//! up to 4K, 4 of 8K), and the pool lets the pictures that no one holds go when media is
//! unloaded.
//!
//! # Playing
//!
//! Forward, the decoder decodes frame after frame, a few at the converter at once (a decoder
//! on the card has only so many frames to give) and a few more ready before they are shown.
//! The frame shown at a time is the one that the clock is at ([`clock::Clock`]): the newest
//! ready frame at or before it; a frame is never shown before its time. A frame that the
//! decoder gives after its time has passed is not converted (skipped), unless nothing was shown
//! for [`STALE`]; where the decoder is so far behind that a key frame lies between it and the
//! clock, it seeks to that key frame (as it does at 4× and 8× on media of long groups of
//! frames, which it cannot decode at 8 times their rate). A change of speed forward keeps
//! what was decoded ahead.
//!
//! The clock: at speed 1 with sound, the sound that the device has played (its latency
//! counted) is the clock, so that the picture follows what is heard; otherwise (no sound, the
//! device unavailable, or any other speed, which is played without sound) the monotonic clock
//! of the system. The device tells where it is a run at a time, each run told up to 10 ms early
//! or late (WASAPI), so the clock is the monotonic clock plus how far the sound is ahead of it,
//! smoothed over some 40 ms ([`smoothed`]); and while it plays, the timers of the system are
//! made fine ([`crate::timer`]), so that the transport wakes for each frame in time. Where the
//! device fails or stops asking for sound ([`Output::failed`]), playing goes on from where it
//! is by the monotonic clock, and the device is opened anew the next time it plays. Backward,
//! the transport decodes the frame that the clock is at, seeking for each one: on media of long
//! groups of frames only some are decoded in time, and those are shown (the all-intra proxies
//! of Task 13b make it smooth). Playing stops at the last frame forward, and at frame 0
//! backward.
//!
//! # Sound alone
//!
//! Media that is sound alone has no picture: the view is cleared, and its frames are those of
//! a grid of the rate of the first timeline of the project when it is loaded, or 25 a second
//! if there is none ([`GRID`]), from its duration. It plays at speed 1 by its sound; at other
//! speeds and backward the playhead moves on the monotonic clock, in silence.
//!
//! # Seeking and scrubbing
//!
//! A seek decodes the frame sought (`exact`). A seek while **scrubbing** shows first the key
//! frame at or before the frame ([`VideoDecoder::keyframe_at_or_before`]), which is decoded at
//! once, and then, once no newer seek came for [`SETTLE`], the frame itself; meanwhile what is
//! told says `exact: false`, as it does when a frame sought cannot be decoded.
//!
//! # The source of the pictures
//!
//! Everything that decodes goes through [`Loaded::show`] (a frame sought, stepped to, played
//! backward) and the forward loop of [`Worker::forward`], both on the one [`VideoDecoder`] of
//! [`Picture`]s of [`Loaded`] (`video`), while the sound is read from `sound_path`, the media
//! itself. Where the media has a proxy ([`crate::proxy`]) and proxies are used
//! (`transport.proxies`, on at first), the decoder is of the proxy: its frames are those of the
//! media, numbered alike, at the same rate, and the sound is laid by the start of the picture
//! of the media ([`columbina_media::video_start`]). A proxy that cannot be opened, or is not at
//! the rate of the media, is let go for the media itself, and made again. A proxy that is made
//! while its media is loaded, and proxies turned on or off, take the place of the picture at
//! once while it does not play, else as soon as it stops or is sought ([`Worker::settle`]):
//! the frame shown stays.

pub(crate) mod clock;

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use columbina_media::{
    AudioDecoder, Converter, Decoded, Facts, Hardware, MediaError, Picture, Rate, Time,
    VideoDecoder, VideoFacts,
};
use columbina_model::Project;
use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_json::value::RawValue;
use tokio::sync::oneshot;

use crate::audio::{self, Output};
use crate::log::log;
use crate::rpc::{self, Fault};
use crate::view::ViewSlot;
use clock::{Clock, First, NANOS};

/// The most that the pictures held at once while playing may take.
pub const POOL_BYTES: u64 = 384 * 1024 * 1024;

/// The grid of frames of media that is sound alone, where the project has no timeline.
pub const GRID: Rate = Rate { num: 25, den: 1 };

/// How long a scrubbing seek waits for a newer one before the frame itself is decoded.
pub const SETTLE: Duration = Duration::from_millis(150);

/// How often the state is told at most while playing: 30 times a second.
pub const TOLD_EVERY: Duration = Duration::from_millis(33);

/// How long nothing new may be shown while playing before a frame that is late is shown
/// rather than dropped.
pub const STALE: Duration = Duration::from_millis(100);

/// The fastest the transport plays, forward or backward.
pub const FASTEST: i32 = 8;

/// How many pictures may be held at once, and what for, for pictures of some size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Frames decoded ahead of the one shown, at the converter or ready.
    pub ahead: usize,
    /// Frames at the converter at once.
    pub in_flight: usize,
    /// Pictures shown last that are kept while playing.
    pub recent_playing: usize,
    /// Pictures kept after a pause, to be shown at once by a step.
    pub recent_paused: usize,
    /// Pictures that the converter keeps to write into again: the view holds two more.
    pub pool: usize,
}

impl Limits {
    /// The limits for pictures of `bytes` each: as many as fit in [`POOL_BYTES`], from 4 to
    /// 12, two of them the view's.
    pub fn of(bytes: u64) -> Limits {
        let fit = usize::try_from(POOL_BYTES / bytes.max(1))
            .unwrap_or(usize::MAX)
            .clamp(4, 12);
        let recent_playing = if fit >= 8 { 2 } else { 0 };
        let ahead = fit - 2 - recent_playing;
        Limits {
            ahead,
            in_flight: ahead.min(3),
            recent_playing,
            recent_paused: (fit - 3).min(8),
            pool: fit,
        }
    }

    /// The bytes of a picture of `width` × `height` of `bits`, as [`Picture`] lays it out.
    pub fn picture_bytes(width: u32, height: u32, bits: u8) -> u64 {
        let bytes = if bits > 8 { 2 } else { 1 };
        let stride = (u64::from(width).div_ceil(2) * 2 * bytes).next_multiple_of(64);
        stride * (u64::from(height) + u64::from(height).div_ceil(2))
    }
}

/// How the transport is made, beside the engine: what plays its sound, and, for the tests, a
/// frame from which on the first frame on the card that the converter is given fails to be
/// copied (`--fail-card-at`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Setup {
    pub sound: audio::Kind,
    pub fail_card_at: Option<i64>,
}

/// The transport, as the engine's methods reach it. It is cheap to clone: the transport's own
/// thread is the same.
#[derive(Clone)]
pub struct Transport {
    /// None in a headless engine, which shows nothing and plays no sound.
    inbox: Option<mpsc::Sender<Inbox>>,
    shared: Arc<Mutex<Shared>>,
}

/// What the methods and the transport's thread share.
struct Shared {
    /// The state, as it was last told.
    status: Status,
    /// The media of the loads sent and not yet done, in order.
    loading: Vec<String>,
    /// Whether the transport's thread has stopped.
    stopped: bool,
    /// The media of the load sent last, and the proxy that was sent with it, or since.
    proxy_sent: Option<(String, Option<PathBuf>)>,
    /// The proxies that could not be opened or decoded, and were not made again since.
    failed: HashSet<PathBuf>,
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
    /// The frame whose picture is shown; none for sound alone.
    pub shown: Option<i64>,
    /// The last frame of the media.
    pub last: Option<i64>,
    pub sound: Sound,
    /// Where the picture is read from: the proxy of the media, or the media itself (as for sound
    /// alone); none when nothing is loaded.
    pub source: Option<Source>,
    /// Whether proxies are used, where media has one.
    pub proxies: bool,
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
            source: None,
            proxies: true,
        }
    }
}

/// Where the picture of the media loaded is read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// The media itself.
    Original,
    /// Its proxy.
    Proxy,
}

impl Source {
    fn name(self) -> &'static str {
        match self {
            Source::Original => "original",
            Source::Proxy => "proxy",
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

/// What the transport asks of the engine when the proxy of media cannot be opened or decoded:
/// that it be made again; and, for one that could not be decoded, that its file (given) be
/// deleted, if it is one of the proxies of the project.
pub type Again = Arc<dyn Fn(&str, Option<&Path>) + Send + Sync>;

/// What comes to the transport's thread.
enum Inbox {
    Command(Command),
    Converted(Converted),
    /// The converter's thread ended, as it does only when it panics.
    ConverterGone,
}

/// What a client asked of the transport.
enum Command {
    Load {
        media: String,
        path: PathBuf,
        /// Its proxy, if it has one that may be used.
        proxy: Option<PathBuf>,
        facts: Facts,
        /// The rate of the first timeline of the project, for media that is sound alone.
        grid: Option<Rate>,
        reply: oneshot::Sender<Result<Status, Fault>>,
    },
    /// Unload the media, if it is the one loaded.
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
    /// The proxy of media is now this, or none.
    Source {
        media: String,
        proxy: Option<PathBuf>,
    },
    /// Use proxies, or not.
    Proxies {
        on: bool,
    },
}

/// What the converter is given.
enum Work {
    Convert(Job),
    /// Let the pictures of the pool that no one holds go, beyond those it may keep.
    Trim,
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

/// The params of `transport.proxies`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UseProxies {
    on: bool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The error of a transport whose thread has stopped.
fn stopped() -> Fault {
    Fault::new(rpc::TRANSPORT_FAILED, "the transport has stopped")
        .with_data(json!({ "reason": "failed" }))
}

/// A command of a method, to be sent once its answer is queued, so that the state it makes is
/// told after the answer.
#[must_use]
pub struct Order {
    transport: Transport,
    command: Option<Command>,
}

impl Order {
    /// Send the command to the transport's thread.
    pub fn send(self) {
        if let Some(command) = self.command {
            let _ = self.transport.send(command);
        }
    }
}

/// A load that was sent, to be waited for.
pub struct Loading(oneshot::Receiver<Result<Status, Fault>>);

impl Loading {
    /// Wait for the load to be done, and give the state then.
    ///
    /// # Errors
    /// The file cannot be opened or decoded (`-32002`), or the transport has stopped.
    pub async fn done(self) -> Result<Status, Fault> {
        self.0.await.map_err(|_| stopped())?
    }
}

impl Transport {
    /// The transport of an engine; with its threads, unless the engine is headless.
    pub fn new(
        headless: bool,
        setup: Setup,
        view: ViewSlot,
        notify: Notify,
        again: Again,
    ) -> Transport {
        let shared = Arc::new(Mutex::new(Shared {
            status: Status::unloaded(),
            loading: Vec::new(),
            stopped: false,
            proxy_sent: None,
            failed: HashSet::new(),
        }));
        if headless {
            return Transport {
                inbox: None,
                shared,
            };
        }
        let (inbox, received) = mpsc::channel();
        let (work, taken) = mpsc::channel();
        let generation = Arc::new(AtomicU64::new(0));
        let pool = Arc::new(AtomicUsize::new(Limits::of(0).pool));
        let started = std::thread::Builder::new()
            .name("columbina-convert".into())
            .spawn({
                let inbox = inbox.clone();
                let generation = Arc::clone(&generation);
                let pool = Arc::clone(&pool);
                move || {
                    let _gone = ConverterGuard(inbox.clone());
                    convert_all(&taken, &inbox, &generation, &pool, setup.fail_card_at);
                }
            })
            .and_then(|_| {
                let shared = Arc::clone(&shared);
                std::thread::Builder::new()
                    .name("columbina-transport".into())
                    .spawn(move || {
                        let _guard = Guard {
                            shared: Arc::clone(&shared),
                            notify: Arc::clone(&notify),
                        };
                        Worker {
                            pipe: Pipe {
                                inbox: received,
                                work,
                                generation,
                                pool,
                                waiting: VecDeque::new(),
                                gone: false,
                            },
                            world: World {
                                shared,
                                notify,
                                view,
                                epoch: Instant::now(),
                                told: None,
                            },
                            sound: SoundSlot {
                                kind: setup.sound,
                                output: None,
                            },
                            loaded: None,
                            proxies: true,
                            again,
                        }
                        .run();
                    })
            });
        if let Err(error) = started {
            lock(&shared).stopped = true;
            log(
                "transport_failed",
                &json!({ "message": format!("the transport cannot be started: {error}") }),
            );
        }
        Transport {
            inbox: Some(inbox),
            shared,
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
                | "transport.proxies"
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
        lock(&self.shared).status.clone()
    }

    fn send(&self, command: Command) -> Result<(), Fault> {
        if lock(&self.shared).stopped {
            return Err(stopped());
        }
        self.inbox
            .as_ref()
            .and_then(|inbox| inbox.send(Inbox::Command(command)).ok())
            .ok_or_else(stopped)
    }

    /// `transport.load`: send the load of a media item of the project, in the place of the one
    /// loaded. Called while the state of the engine is locked, after the media was found in the
    /// project: so that a change that removes it, made after, unloads it after it is loaded
    /// ([`Transport::project_changed`]).
    ///
    /// # Errors
    /// The transport has stopped.
    pub fn begin_load(
        &self,
        media: &str,
        path: PathBuf,
        proxy: Option<PathBuf>,
        facts: Facts,
        grid: Option<Rate>,
    ) -> Result<Loading, Fault> {
        let (reply, replied) = oneshot::channel();
        {
            let mut shared = lock(&self.shared);
            shared.loading.push(media.to_owned());
            shared.proxy_sent = Some((media.to_owned(), proxy.clone()));
        }
        let sent = self.send(Command::Load {
            media: media.to_owned(),
            path,
            proxy,
            facts,
            grid,
            reply,
        });
        if let Err(fault) = sent {
            let mut shared = lock(&self.shared);
            if let Some(index) = shared
                .loading
                .iter()
                .position(|loading| **loading == *media)
            {
                shared.loading.remove(index);
            }
            return Err(fault);
        }
        Ok(Loading(replied))
    }

    /// The methods of the transport other than `transport.load`, which are answered at once:
    /// the result, and the command to send once the answer is queued, so that what it does is
    /// told after the answer.
    ///
    /// # Errors
    /// The params are not what the method takes, or no media is loaded or being loaded.
    pub fn prepare(
        &self,
        method: &str,
        params: Option<&RawValue>,
    ) -> Result<(serde_json::Value, Order), Fault> {
        self.check(method)?;
        if method == "transport.state" {
            rpc::no_params(method, params)?;
            let status = serde_json::to_value(self.status()).unwrap_or_default();
            let order = Order {
                transport: self.clone(),
                command: None,
            };
            return Ok((status, order));
        }
        if method == "transport.proxies" {
            let proxies: UseProxies = rpc::params(method, r#"{"on": true}"#, params)?;
            if lock(&self.shared).stopped {
                return Err(stopped());
            }
            let order = Order {
                transport: self.clone(),
                command: Some(Command::Proxies { on: proxies.on }),
            };
            return Ok((json!({}), order));
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
        {
            let shared = lock(&self.shared);
            if shared.stopped {
                return Err(stopped());
            }
            if shared.status.media.is_none() && shared.loading.is_empty() {
                return Err(Fault::new(
                    rpc::TRANSPORT_FAILED,
                    "no media is loaded in the transport",
                )
                .with_data(json!({ "reason": "not_loaded" })));
            }
        }
        let order = Order {
            transport: self.clone(),
            command: Some(command),
        };
        Ok((json!({}), order))
    }

    /// The project changed: the media loaded, or being loaded, is unloaded if it was removed.
    /// This only reads the state of the transport and sends to its thread: it is called while
    /// the state of the engine is locked.
    pub fn project_changed(&self, project: &Project) {
        let gone: Vec<String> = {
            let shared = lock(&self.shared);
            shared
                .status
                .media
                .iter()
                .chain(&shared.loading)
                .filter(|media| !project.media.iter().any(|item| item.id == **media))
                .cloned()
                .collect()
        };
        for media in gone {
            let _ = self.send(Command::Unload { media });
        }
        // A proxy made, made again or taken away while its media is loaded.
        let changed = {
            let mut shared = lock(&self.shared);
            let Some((media, _)) = &shared.proxy_sent else {
                return;
            };
            let Some(found) = project.media.iter().find(|item| item.id == *media) else {
                return;
            };
            let now = crate::proxy::usable(found)
                .map(|proxy| PathBuf::from(&proxy.path))
                .filter(|path| !shared.failed.contains(path));
            let Some((media, sent)) = &mut shared.proxy_sent else {
                return;
            };
            if now == *sent {
                return;
            }
            sent.clone_from(&now);
            (media.clone(), now)
        };
        let _ = self.send(Command::Source {
            media: changed.0,
            proxy: changed.1,
        });
    }

    /// The proxy of media was made, or made again: if the media is loaded, its picture is read
    /// from it, even where a proxy at the same path could not be read before. This only sends to
    /// the transport's thread.
    pub fn proxy_ready(&self, media: &str, path: PathBuf) {
        {
            let mut shared = lock(&self.shared);
            shared.failed.remove(&path);
            match &mut shared.proxy_sent {
                Some((sent, proxy)) if sent == media => *proxy = Some(path.clone()),
                _ => return,
            }
        }
        let _ = self.send(Command::Source {
            media: media.to_owned(),
            proxy: Some(path),
        });
    }

    /// A view was attached: show it the picture.
    pub fn redraw(&self) {
        if lock(&self.shared).status.media.is_some() {
            let _ = self.send(Command::Redraw);
        }
    }
}

/// Tells everyone that nothing is loaded when the transport's thread ends, as it does only when
/// it panics or its converter's thread does; the methods are refused from then on.
struct Guard {
    shared: Arc<Mutex<Shared>>,
    notify: Notify,
}

impl Drop for Guard {
    fn drop(&mut self) {
        log(
            "transport_failed",
            &json!({
                "message": if std::thread::panicking() {
                    "the transport panicked; nothing is loaded, and it plays no more"
                } else {
                    "the transport ended; nothing is loaded, and it plays no more"
                },
            }),
        );
        let status = Status::unloaded();
        {
            let mut shared = lock(&self.shared);
            shared.stopped = true;
            shared.loading.clear();
            shared.status = status.clone();
        }
        (self.notify)(&status);
    }
}

/// Tells the transport that the converter's thread ended.
struct ConverterGuard(mpsc::Sender<Inbox>);

impl Drop for ConverterGuard {
    fn drop(&mut self) {
        let _ = self.0.send(Inbox::ConverterGone);
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
/// transport ends. A frame of a generation that has passed is let go unconverted. With
/// `fail_card_at`, the first frame on the card from that frame on fails to be copied, as when
/// the card is lost.
fn convert_all(
    work: &mpsc::Receiver<Work>,
    inbox: &mpsc::Sender<Inbox>,
    current: &AtomicU64,
    most: &AtomicUsize,
    mut fail_card_at: Option<i64>,
) {
    let mut converter = Converter::new();
    let mut pool: Vec<Arc<Picture>> = Vec::new();
    for work in work {
        let job = match work {
            Work::Convert(job) => job,
            Work::Trim => {
                pool.retain(|kept| Arc::strong_count(kept) > 1);
                continue;
            }
        };
        if job.generation != current.load(Ordering::Acquire) {
            continue;
        }
        let frame = job.decoded.frame();
        let picture = if fail_card_at.is_some_and(|at| frame >= at) && job.decoded.on_device() {
            fail_card_at = None;
            Err(MediaError::Device(format!(
                "frame {frame} was made to fail to be copied (--fail-card-at)"
            )))
        } else {
            make(
                &mut converter,
                &mut pool,
                most.load(Ordering::Acquire),
                &job.decoded,
            )
        };
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
/// in the pool, or, when the pool has `most` and all are held, a new one that is not kept.
fn make(
    converter: &mut Converter,
    pool: &mut Vec<Arc<Picture>>,
    most: usize,
    decoded: &Decoded,
) -> Result<Arc<Picture>, MediaError> {
    if pool.len() > most {
        // The limits are lower than they were: the pictures that no one holds beyond them go.
        let mut free = pool.len() - most;
        pool.retain(|kept| {
            let drop = free > 0 && Arc::strong_count(kept) == 1;
            free -= usize::from(drop);
            !drop
        });
    }
    let free = pool
        .iter_mut()
        .position(|kept| Arc::get_mut(kept).is_some());
    let index = match free {
        Some(index) => index,
        None if pool.len() < most => {
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
    work: mpsc::Sender<Work>,
    /// Raised to let go of what is at the converter: what it makes for an earlier generation
    /// is dropped.
    generation: Arc<AtomicU64>,
    /// How many pictures the converter keeps.
    pool: Arc<AtomicUsize>,
    /// Commands that came while a picture was waited for.
    waiting: VecDeque<Command>,
    /// Whether the converter's thread ended.
    gone: bool,
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
        let _ = self.work.send(Work::Convert(Job {
            generation: self.generation(),
            decoded,
        }));
    }

    /// Keep at most `pool` pictures from now on, and let those that no one holds go.
    fn trim(&self, pool: usize) {
        self.pool.store(pool, Ordering::Release);
        let _ = self.work.send(Work::Trim);
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
                Ok(Inbox::ConverterGone) | Err(_) => {
                    self.gone = true;
                    return Err(MediaError::Decode(
                        "the converter of the transport does not answer".into(),
                    ));
                }
            }
        }
    }
}

/// What the transport's thread tells and shows.
struct World {
    shared: Arc<Mutex<Shared>>,
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

    /// The proxy of media at `path` could not be read: it is not read again, nor sent again by
    /// a change of the project, until it is made again ([`Transport::proxy_ready`]).
    fn forget_proxy(&self, media: &str, path: &Path) {
        let mut shared = lock(&self.shared);
        shared.failed.insert(path.to_owned());
        if let Some((sent, proxy)) = &mut shared.proxy_sent {
            if sent == media {
                *proxy = None;
            }
        }
    }

    /// Whether the proxy at `path` could not be read, and was not made again since.
    fn failed(&self, path: &Path) -> bool {
        lock(&self.shared).failed.contains(path)
    }

    /// Keep the state for the clients, and tell it: at once if `now` (after a command, even
    /// one that changed nothing, so that whoever sent it hears back), else if it changed, no
    /// more often than [`TOLD_EVERY`]. `loaded` is a load that is done, in the same breath.
    fn tell(&mut self, status: Status, now: bool, loaded: Option<&str>) {
        {
            let mut shared = lock(&self.shared);
            shared.status = status.clone();
            if let Some(media) = loaded {
                if let Some(index) = shared.loading.iter().position(|loading| loading == media) {
                    shared.loading.remove(index);
                }
            }
        }
        if let Some((told, at)) = &self.told {
            if !now && (*told == status || at.elapsed() < TOLD_EVERY) {
                return;
            }
        }
        (self.notify)(&status);
        self.told = Some((status, Instant::now()));
    }
}

/// The output of the sound, opened when it is first needed and again after it failed.
struct SoundSlot {
    kind: audio::Kind,
    /// The output, or why there is none.
    output: Option<Result<Output, String>>,
}

impl SoundSlot {
    /// The output to play on: the one opened, unless it failed or the default device of the
    /// system is another now; then it is opened anew (and told in the log).
    fn ready(&mut self) -> Option<&mut Output> {
        let again = match &self.output {
            None | Some(Err(_)) => true,
            Some(Ok(output)) => output.failed().is_some() || !output.current(),
        };
        if again {
            // The old one is closed before another is opened.
            self.output = None;
            let output = Output::open(self.kind);
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
        self.output.as_mut().and_then(|output| output.as_mut().ok())
    }

    /// The output, if one is open and has not failed.
    fn working(&self) -> Option<&Output> {
        match &self.output {
            Some(Ok(output)) if output.failed().is_none() => Some(output),
            _ => None,
        }
    }

    /// Stop what plays, and end its feeder.
    fn stop(&mut self) {
        if let Some(Ok(output)) = &mut self.output {
            output.stop();
        }
    }
}

/// The picture of the media loaded: its decoder, and how far it is turned.
struct Video {
    decoder: VideoDecoder,
    rotation: i16,
    limits: Limits,
}

/// The media loaded, and what the transport does with it.
struct Loaded {
    media: String,
    /// The file whose sound is played: the media itself.
    sound_path: PathBuf,
    /// The proxy of the media, if it has one that may be used.
    proxy: Option<PathBuf>,
    /// What the media's picture is, as its facts say; none for sound alone.
    facts_video: Option<VideoFacts>,
    /// Where the picture is read from.
    source: Source,
    /// The file that the picture is read from: the media's, or its proxy's.
    reading: PathBuf,
    /// Whether the proxy read from failed to decode, and is to be let go for the media itself.
    broken: bool,
    /// Its picture; none for sound alone.
    video: Option<Video>,
    /// The rate of its frames: of its picture, or of the grid of sound alone.
    rate: Rate,
    /// The time of frame 0 in the file, for the sound.
    start: Time,
    /// The last frame: as the facts say, until the decoder finds the end.
    last: i64,
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
    /// read, smoothed ([`smoothed`]); none until the sound is first read.
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

impl Playing {
    fn new(clock: Clock, sound: bool, next: i64) -> Playing {
        Playing {
            latest: std::cell::Cell::new(clock.since()),
            clock,
            sound,
            offset: None,
            _fine: crate::timer::Fine::new(),
            ready: VecDeque::new(),
            in_flight: 0,
            decoded_all: false,
            next,
            shown_at: Instant::now(),
            stats: Stats::new(),
        }
    }

    /// Go on from where it is now at `speed` by the monotonic clock, or by the sound (whose
    /// time starts at 0).
    fn reclock(&mut self, place: i128, speed: i32, sound: bool, world: &World) {
        let since = if sound { 0 } else { world.now() };
        self.clock = Clock::at(place, speed, self.clock.rate(), since);
        self.latest.set(since);
        self.sound = sound;
        self.offset = None;
    }
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
    /// The latency of the device and the length of its runs, as last read.
    latency: Option<(Duration, Duration)>,
    /// How many runs of the device had less sound than it asked for.
    short_runs: u64,
}

impl Stats {
    fn new() -> Stats {
        Stats {
            started: Instant::now(),
            presented: 0,
            skipped: 0,
            passed: 0,
            late: 0,
            worst_late: 0,
            total_late: 0,
            heard: None,
            sound_now: None,
            jitter: 0,
            latency: None,
            short_runs: 0,
        }
    }

    /// Count a picture shown `late` nanoseconds after its time, behind the clock if `behind`.
    fn shown(&mut self, late: i128, behind: bool) {
        self.presented += 1;
        self.worst_late = self.worst_late.max(late);
        self.total_late += late;
        self.late += u64::from(behind);
    }
}

/// What the transport's thread owns.
struct Worker {
    pipe: Pipe,
    world: World,
    sound: SoundSlot,
    loaded: Option<Loaded>,
    /// Whether proxies are used.
    proxies: bool,
    again: Again,
}

impl Worker {
    fn run(mut self) {
        loop {
            if self.pipe.gone {
                return;
            }
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
                    Inbox::ConverterGone => self.pipe.gone = true,
                }
            }
            if self.pipe.gone {
                return;
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
                let backward = playing.clock.speed() < 0;
                let wait = match &loaded.video {
                    None if backward => until(target - 1),
                    None => until(target + 1),
                    Some(_) if backward => {
                        if loaded.picture.as_ref().map(|picture| picture.frame) == Some(target) {
                            until(target - 1)
                        } else {
                            Duration::ZERO
                        }
                    }
                    Some(video) => {
                        let limits = video.limits;
                        if !playing.decoded_all
                            && playing.in_flight < limits.in_flight
                            && playing.in_flight + playing.ready.len() < limits.ahead
                        {
                            Duration::ZERO
                        } else {
                            match playing.ready.front() {
                                Some(front) => until(front.frame),
                                None if playing.in_flight > 0 => most,
                                // Every frame decoded and shown: until the clock reaches the
                                // last.
                                None if playing.decoded_all => until(loaded.last),
                                None => Duration::ZERO,
                            }
                        }
                    }
                };
                Some(wait.min(most))
            }
        }
    }

    fn status(&self) -> Status {
        let Some(loaded) = &self.loaded else {
            return Status {
                proxies: self.proxies,
                ..Status::unloaded()
            };
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
            } else if self.sound.working().is_none() {
                Sound::Unavailable
            } else {
                Sound::Off
            },
            source: Some(loaded.source),
            proxies: self.proxies,
        }
    }

    fn tell(&mut self, now: bool) {
        let status = self.status();
        self.world.tell(status, now, None);
    }

    fn apply(&mut self, command: Command) {
        match command {
            Command::Load {
                media,
                path,
                proxy,
                facts,
                grid,
                reply,
            } => {
                self.unload();
                let opened = self.open(media.clone(), path, proxy, &facts, grid);
                if opened.is_err() {
                    self.world.clear();
                }
                let status = self.status();
                self.world.tell(status.clone(), true, Some(&media));
                let _ = reply.send(opened.map(|()| status));
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
            Command::Redraw => match &self.loaded {
                Some(Loaded {
                    picture: Some(picture),
                    video: Some(video),
                    ..
                }) => self.world.present(picture, video.rotation),
                Some(_) => self.world.clear(),
                None => {}
            },
            Command::Source { media, proxy } => {
                if let Some(loaded) = &mut self.loaded {
                    if loaded.media == media && loaded.video.is_some() {
                        loaded.proxy = proxy;
                    }
                }
            }
            Command::Proxies { on } => {
                self.proxies = on;
                self.settle();
                self.tell(true);
            }
        }
        if self.settle() {
            self.tell(true);
        }
        self.recover();
    }

    /// Let go of a proxy that failed to decode: the picture is read from the media itself from
    /// the frame at the playhead on, playing on if it played, the file of the proxy is deleted,
    /// and it is made again.
    fn recover(&mut self) {
        let Some(loaded) = &mut self.loaded else {
            return;
        };
        if !std::mem::take(&mut loaded.broken) || loaded.source != Source::Proxy {
            return;
        }
        let bad = loaded.reading.clone();
        let media = loaded.media.clone();
        log(
            "transport_proxy_failed",
            &json!({
                "media": media,
                "path": bad.to_string_lossy(),
                "reason": "decode",
                "message": "the proxy cannot be decoded: the media itself is read, and the \
                            proxy is made again",
            }),
        );
        self.world.forget_proxy(&media, &bad);
        loaded.proxy = None;
        let speed = match &loaded.mode {
            Mode::Playing(playing) => Some(playing.clock.speed()),
            _ => None,
        };
        if speed.is_some() {
            self.pause();
        }
        self.settle();
        let Some(loaded) = &mut self.loaded else {
            return;
        };
        // The decoder of the proxy is let go: its file can be deleted, by the engine, which
        // deletes only a file of the proxies of the project.
        let damaged = (loaded.reading != bad).then_some(bad.as_path());
        (self.again)(&media, damaged);
        if !matches!(loaded.mode, Mode::Scrubbing { .. }) {
            let target = loaded.playhead;
            if let Err(error) = loaded.show_exact(target, &mut self.pipe, &self.world) {
                failed(&loaded.media, &error);
            }
        }
        if let Some(speed) = speed {
            self.start(speed, None);
        }
        self.tell(true);
    }

    /// Read the picture from the proxy or from the media itself, as it is to be now, unless it
    /// plays: then this is done when it stops or is sought. The frame shown stays, and is the
    /// same frame; what is decoded from now on is decoded from the new source. Whether the
    /// source changed, to be told by the caller.
    fn settle(&mut self) -> bool {
        let Some(loaded) = &mut self.loaded else {
            return false;
        };
        let Some(facts) = loaded.facts_video.clone() else {
            return false;
        };
        if matches!(loaded.mode, Mode::Playing(_)) {
            return false;
        }
        let proxy = loaded
            .proxy
            .as_deref()
            .filter(|proxy| self.proxies && !self.world.failed(proxy));
        if proxy.unwrap_or(&loaded.sound_path) == loaded.reading {
            return false;
        }
        match open_picture(&loaded.media, &loaded.sound_path, proxy, &facts) {
            Ok(opened) => {
                if let (true, Some(proxy)) = (opened.proxy_failed, proxy) {
                    self.world.forget_proxy(&loaded.media, proxy);
                    loaded.proxy = None;
                    (self.again)(&loaded.media, None);
                }
                if opened.path == loaded.reading {
                    return false;
                }
                self.pipe.flush();
                loaded.recent.clear();
                self.pipe.trim(opened.video.limits.pool);
                loaded.video = Some(opened.video);
                loaded.source = opened.source;
                loaded.reading = opened.path;
                log(
                    "transport_source",
                    &json!({ "media": loaded.media, "source": opened.source.name() }),
                );
                true
            }
            Err(error) => {
                failed(&loaded.media, &error);
                false
            }
        }
    }

    /// Let the media go, stop its sound, and let the pictures of the pool go.
    fn unload(&mut self) {
        self.pause();
        self.sound.stop();
        self.pipe.flush();
        self.loaded = None;
        self.pipe.trim(Limits::of(0).pool);
    }

    /// Open the media and show its frame 0: its picture from its proxy where it has one and
    /// proxies are used, unless the proxy cannot be opened.
    fn open(
        &mut self,
        media: String,
        path: PathBuf,
        proxy: Option<PathBuf>,
        facts: &Facts,
        grid: Option<Rate>,
    ) -> Result<(), Fault> {
        let text = path.to_string_lossy().into_owned();
        let cannot = |error: &MediaError| {
            crate::session::cannot_read(&text, error.reason(), &error.to_string())
        };
        let has_sound = facts.audio.is_some();
        let mut proxy = proxy;
        let mut reading = path.clone();
        let (video, rate, start, source) = if let Some(facts_video) = &facts.video {
            let wanted = proxy
                .as_deref()
                .filter(|proxy| self.proxies && !self.world.failed(proxy));
            let opened =
                open_picture(&media, &path, wanted, facts_video).map_err(|error| cannot(&error))?;
            if let (true, Some(wanted)) = (opened.proxy_failed, wanted) {
                self.world.forget_proxy(&media, wanted);
                proxy = None;
                (self.again)(&media, None);
            }
            let decoder = &opened.video.decoder;
            let fallbacks: Vec<_> = decoder
                .fallbacks()
                .iter()
                .map(|(device, why)| json!({ "device": device.name(), "why": why }))
                .collect();
            log(
                "transport_loaded",
                &json!({
                    "media": media,
                    "decoder": decoder.path_used().name(),
                    "fallbacks": fallbacks,
                    "source": opened.source.name(),
                }),
            );
            let rate = decoder.rate();
            reading.clone_from(&opened.path);
            (Some(opened.video), rate, opened.start, opened.source)
        } else {
            // Sound alone: its frames are on a grid, from the start of its sound.
            let sound = AudioDecoder::open(&path, None).map_err(|error| cannot(&error))?;
            let rate = grid.unwrap_or(GRID);
            log(
                "transport_loaded",
                &json!({ "media": media, "decoder": null, "grid": rate }),
            );
            (None, rate, sound.start(), Source::Original)
        };
        self.pipe.trim(
            video
                .as_ref()
                .map_or(Limits::of(0).pool, |video| video.limits.pool),
        );
        if has_sound {
            self.sound.ready();
        }
        let mut loaded = Loaded {
            media,
            sound_path: path,
            proxy,
            facts_video: facts.video.clone(),
            source,
            reading,
            broken: false,
            video,
            rate,
            start,
            last: clock::last_frame(facts.duration, rate),
            has_sound,
            picture: None,
            recent: VecDeque::new(),
            playhead: 0,
            exact: true,
            speed: 1,
            mode: Mode::Paused,
        };
        if loaded.video.is_none() {
            self.world.clear();
        }
        loaded
            .show_exact(0, &mut self.pipe, &self.world)
            .map_err(|error| cannot(&error))?;
        self.loaded = Some(loaded);
        Ok(())
    }

    /// Play at `speed`, from the playhead, or, if it plays already, from where it is.
    fn play(&mut self, speed: i32) {
        let Some(loaded) = &mut self.loaded else {
            self.tell(true);
            return;
        };
        let restart = match &mut loaded.mode {
            Mode::Playing(playing) if playing.clock.speed() == speed => None,
            Mode::Playing(playing) => {
                let now = clock_time(playing, &self.world);
                // Where it is, at the new speed: the place does not jump.
                let place = playing.clock.with_speed(speed, now).place(now);
                if (playing.clock.speed() > 0) == (speed > 0) {
                    // On the same way: what was decoded ahead is still to be shown, and the
                    // clock goes on from where it is, by the sound at speed 1.
                    if playing.sound {
                        self.sound.stop();
                    }
                    let sound = speed == 1
                        && loaded.has_sound
                        && start_sound(
                            &mut self.sound,
                            &loaded.sound_path,
                            loaded.start,
                            place,
                            loaded.rate,
                        );
                    playing.reclock(place, speed, sound, &self.world);
                    loaded.speed = speed;
                    None
                } else {
                    Some(Some(place))
                }
            }
            Mode::Scrubbing { .. } | Mode::Paused => Some(None),
        };
        if let Some(place) = restart {
            // From where it is, the other way: the place does not jump.
            self.pause();
            self.settle();
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
                loaded.failed(&error);
            }
            loaded.mode = Mode::Paused;
            return;
        }
        let shown = loaded.picture.as_ref().map(|picture| picture.frame);
        let mut ready = Ok(());
        if loaded.video.is_some() && (shown != Some(frame) || !loaded.exact) {
            ready = loaded.show_exact(frame, &mut self.pipe, &self.world);
        }
        if let (Some(video), true, true) = (&mut loaded.video, speed > 0, ready.is_ok()) {
            // The decoder is to give the frames after it next: it gives the frame it gave
            // last again without decoding.
            ready = video.decoder.decoded_at(frame).map(drop);
        }
        if let Err(error) = ready {
            loaded.failed(&error);
            loaded.mode = Mode::Paused;
            return;
        }
        self.pipe.flush();
        let place = clock::clamped_place(place, frame, speed, rate);
        // What was kept for steps is let go, for the pool to use again.
        loaded.recent.clear();
        let sound = speed == 1
            && loaded.has_sound
            && start_sound(
                &mut self.sound,
                &loaded.sound_path,
                loaded.start,
                place,
                rate,
            );
        let since = if sound { 0 } else { self.world.now() };
        loaded.mode = Mode::Playing(Box::new(Playing::new(
            Clock::at(place, speed, rate, since),
            sound,
            frame + 1,
        )));
    }

    /// Stop playing where it is, with the frame shown as the playhead (for sound alone, the
    /// frame of the clock); keep the pictures that were ready, to be shown at once by a step.
    fn pause(&mut self) {
        self.sound.stop();
        let Some(loaded) = &mut self.loaded else {
            return;
        };
        if !matches!(loaded.mode, Mode::Playing(_)) {
            return;
        }
        let Mode::Playing(playing) = std::mem::replace(&mut loaded.mode, Mode::Paused) else {
            return;
        };
        let mut playing = *playing;
        self.pipe.flush();
        match &loaded.picture {
            Some(picture) => loaded.playhead = picture.frame,
            None if loaded.video.is_none() => {
                let now = clock_time(&playing, &self.world);
                loaded.playhead = clock::clamped(playing.clock.frame(now), loaded.last);
            }
            None => {}
        }
        loaded.exact = true;
        // The pictures just shown are kept; those ready after them as there is room.
        let room = loaded
            .video
            .as_ref()
            .map_or(0, |video| video.limits.recent_paused);
        for picture in std::mem::take(&mut playing.ready) {
            if loaded.recent.len() >= room {
                break;
            }
            loaded.recent.push_back(picture);
        }
        log_played(loaded, &playing);
    }

    fn seek(&mut self, frame: i64, scrubbing: bool, at: Instant) {
        let Some(loaded) = &mut self.loaded else {
            self.tell(true);
            return;
        };
        let target = clock::clamped(frame, loaded.last);
        let playing = match &loaded.mode {
            Mode::Playing(playing) => Some(playing.clock.speed()),
            _ => None,
        };
        if let (Some(speed), false) = (playing, scrubbing) {
            // A seek while playing plays on from there.
            let rate = loaded.rate;
            self.pause();
            if let Some(loaded) = &mut self.loaded {
                loaded.playhead = target;
            }
            self.settle();
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
            let key = loaded
                .video
                .as_ref()
                .and_then(|video| video.decoder.keyframe_at_or_before(target));
            clock::scrub_first(target, key, shown)
        } else {
            First::Exact
        };
        match first {
            First::Exact => {
                loaded.mode = Mode::Paused;
                if let Err(error) = loaded.show_exact(target, &mut self.pipe, &self.world) {
                    loaded.failed(&error);
                }
            }
            First::Key(key) => {
                match loaded.show(key, &mut self.pipe, &self.world) {
                    Ok(()) => loaded.exact = key == target,
                    Err(error) => {
                        loaded.exact = false;
                        loaded.failed(&error);
                    }
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
            self.tell(true);
            return;
        };
        let target = clock::stepped(loaded.playhead, frames, loaded.last);
        loaded.playhead = target;
        loaded.mode = Mode::Paused;
        if let Err(error) = loaded.show_exact(target, &mut self.pipe, &self.world) {
            loaded.failed(&error);
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
        let (Mode::Playing(playing), Some(video)) = (&mut loaded.mode, &mut loaded.video) else {
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
                video.decoder.device_failed(&why);
                self.pipe.flush();
                playing.in_flight = 0;
                match video.decoder.decoded_at(converted.frame) {
                    Ok(decoded) => {
                        playing.next = decoded.frame() + 1;
                        playing.in_flight = 1;
                        self.pipe.send(decoded);
                    }
                    Err(error) => loaded.failed(&error),
                }
            }
            Err(error) => {
                playing.stats.passed += 1;
                loaded.failed(&error);
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
                if let Err(error) = loaded.show_exact(target, &mut self.pipe, &self.world) {
                    loaded.failed(&error);
                }
                self.tell(true);
            }
            Mode::Playing(playing) => {
                let forward = playing.clock.speed() > 0;
                if playing.sound {
                    self.check_sound();
                }
                let ended = match (forward, self.loaded.as_ref().map(|l| l.video.is_some())) {
                    (_, Some(false)) => self.sound_alone(),
                    (true, _) => self.forward(),
                    (false, _) => self.backward(),
                };
                if ended {
                    self.pause();
                    self.settle();
                    self.tell(true);
                } else {
                    self.tell(false);
                }
            }
            Mode::Scrubbing { .. } | Mode::Paused => {}
        }
        self.recover();
    }

    /// While the sound is the clock: read it, for the smoothed clock and the log; where the
    /// device failed or stopped asking for sound, go on from where it is by the monotonic
    /// clock, in silence, and stop the device.
    fn check_sound(&mut self) {
        let Some(Loaded {
            mode: Mode::Playing(playing),
            ..
        }) = &mut self.loaded
        else {
            return;
        };
        let output = self.sound.working();
        let now = Instant::now();
        if let Some(heard) = output.and_then(|output| sound_nanos(output, now)) {
            playing.offset = Some(smoothed(playing.offset, heard - self.world.now()));
            let stats = &mut playing.stats;
            if heard > 0 {
                stats.heard.get_or_insert((now, heard));
                if let Some((then, before)) = stats.sound_now {
                    let system = i128::try_from(now.duration_since(then).as_nanos()).unwrap_or(0);
                    stats.jitter = stats.jitter.max((heard - before - system).abs());
                }
                stats.sound_now = Some((now, heard));
            }
            stats.latency = output.and_then(Output::latency).or(stats.latency);
            stats.short_runs = output.map_or(stats.short_runs, Output::short_runs);
        }
        if output.is_none() {
            let place = playing.clock.place(clock_time(playing, &self.world));
            let speed = playing.clock.speed();
            playing.reclock(place, speed, false, &self.world);
            log(
                "transport_sound_lost",
                &json!({ "message": "the sound cannot be played; the picture plays on alone" }),
            );
            self.sound.stop();
        }
    }

    /// Play sound alone a step: the playhead is where the clock is. Whether it reached an end.
    fn sound_alone(&mut self) -> bool {
        let Some(loaded) = &mut self.loaded else {
            return true;
        };
        let Mode::Playing(playing) = &mut loaded.mode else {
            return true;
        };
        let now = clock_time(playing, &self.world);
        let frame = playing.clock.frame(now);
        loaded.playhead = clock::clamped(frame, loaded.last);
        if playing.clock.speed() > 0 {
            frame >= loaded.last
        } else {
            frame <= 0
        }
    }

    /// Play forward a step: show what is due, and decode one more frame if there is room.
    /// Whether it reached the end.
    fn forward(&mut self) -> bool {
        let Some(loaded) = &mut self.loaded else {
            return true;
        };
        let (Mode::Playing(playing), Some(video)) = (&mut loaded.mode, &mut loaded.video) else {
            return true;
        };
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
            playing.stats.shown(late, picture.frame < target);
            playing.shown_at = Instant::now();
            self.world.present(&picture, video.rotation);
            loaded.exact = true;
            loaded.playhead = picture.frame;
            if let Some(old) = loaded.picture.replace(picture) {
                loaded.recent.push_back(old);
                while loaded.recent.len() > video.limits.recent_playing {
                    loaded.recent.pop_front();
                }
            }
        }
        if target >= loaded.last && playing.in_flight == 0 && playing.ready.is_empty() {
            let shown = loaded.picture.as_ref().map(|picture| picture.frame);
            if shown != Some(loaded.last) {
                let last = loaded.last;
                if let Err(error) = loaded.show_exact(last, &mut self.pipe, &self.world) {
                    loaded.failed(&error);
                }
            }
            return true;
        }
        let limits = video.limits;
        if playing.decoded_all
            || playing.in_flight >= limits.in_flight
            || playing.in_flight + playing.ready.len() >= limits.ahead
        {
            return false;
        }
        // Far behind, with a key frame between: on from the key frame. Aimed a little ahead
        // of the clock, for the time a frame takes to come.
        let speed = i64::from(playing.clock.speed());
        let ahead = (speed * i64::from(loaded.rate.num) / i64::from(loaded.rate.den.max(1))) / 10;
        let aim = clock::clamped(target + ahead, loaded.last);
        let jump = video
            .decoder
            .keyframe_at_or_before(aim)
            .filter(|&key| target - playing.next > 4 && key > playing.next + 4);
        let decoded = match jump {
            Some(key) => video.decoder.decoded_at(key).map(Some),
            None => video.decoder.next_decoded(),
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
                if let Some(last) = video.decoder.last_frame() {
                    loaded.last = last;
                }
            }
            Err(error) => {
                playing.decoded_all = true;
                loaded.failed(&error);
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
            playing.stats.shown(late, false);
            if let Err(error) = loaded.show_exact(target, &mut self.pipe, &self.world) {
                loaded.failed(&error);
                // A proxy that failed is let go, and playing goes on from the media itself.
                return !loaded.broken;
            }
            loaded.playhead = target;
        }
        target <= 0
    }
}

/// The picture of media, opened.
struct Opened {
    video: Video,
    /// Where it is read from.
    source: Source,
    /// The file that it is read from.
    path: PathBuf,
    /// The time of frame 0 in the file of the media, for its sound.
    start: Time,
    /// Whether a proxy was to be read and could not be: it is to be made again.
    proxy_failed: bool,
}

/// Open the picture of media: from `proxy`, if it is given and can be opened, its frames
/// numbered at the rate of the media, else from the media itself (`original`). The pictures held at once are as many
/// as fit of those of the source opened.
fn open_picture(
    media: &str,
    original: &Path,
    proxy: Option<&Path>,
    facts: &VideoFacts,
) -> Result<Opened, MediaError> {
    let video_of = |decoder: VideoDecoder| {
        let (width, height, bits) = decoder.shape();
        let (width, height) = if width == 0 || height == 0 {
            (facts.width, facts.height)
        } else {
            (width, height)
        };
        Video {
            limits: Limits::of(Limits::picture_bytes(width, height, bits.max(8))),
            decoder,
            rotation: facts.rotation,
        }
    };
    let mut proxy_failed = false;
    if let Some(proxy) = proxy {
        // Its frames are numbered at the rate of the media, whatever rate its container is
        // found to have.
        match VideoDecoder::open_at_rate(proxy, Hardware::Auto, facts.rate) {
            Ok(decoder) => {
                // The sound is laid by the picture of the media; where the media cannot be read
                // (a drive that is not there), by that of the proxy, which plays all the same.
                let start = columbina_media::video_start(original).unwrap_or_else(|error| {
                    log(
                        "transport_error",
                        &json!({ "media": media, "reason": error.reason(), "message": error.to_string() }),
                    );
                    decoder.start()
                });
                return Ok(Opened {
                    video: video_of(decoder),
                    source: Source::Proxy,
                    path: proxy.to_owned(),
                    start,
                    proxy_failed: false,
                });
            }
            Err(error) => {
                log(
                    "transport_proxy_failed",
                    &json!({
                        "media": media,
                        "path": proxy.to_string_lossy(),
                        "reason": error.reason(),
                        "message": error.to_string(),
                    }),
                );
                proxy_failed = true;
            }
        }
    }
    let decoder = VideoDecoder::open(original, Hardware::Auto)?;
    let start = decoder.start();
    Ok(Opened {
        video: video_of(decoder),
        source: Source::Original,
        path: original.to_owned(),
        start,
        proxy_failed,
    })
}

/// Start the sound at `place` of media whose frame 0 is at `start` in its file, on the output,
/// opened anew if it failed or the default device changed. Whether it plays.
fn start_sound(
    slot: &mut SoundSlot,
    path: &std::path::Path,
    start: Time,
    place: i128,
    rate: Rate,
) -> bool {
    let Some(output) = slot.ready() else {
        return false;
    };
    let media = Clock::at(place, 1, rate, 0).media_nanos(0);
    let from = clock::sample_at(start, media, output.format().rate);
    match output.start(path, from) {
        Ok(()) => true,
        Err(message) => {
            log("sound_failed", &json!({ "message": message }));
            false
        }
    }
}

/// Log how playing went.
fn log_played(loaded: &Loaded, playing: &Playing) {
    let stats = &playing.stats;
    let seconds = stats.started.elapsed().as_secs_f64();
    #[allow(clippy::cast_precision_loss, reason = "milliseconds, for the log")]
    let millis = |nanos: i128| nanos as f64 / 1e6;
    #[allow(clippy::cast_precision_loss, reason = "a count of frames, for the log")]
    let mean_late = if stats.presented > 0 {
        millis(stats.total_late) / stats.presented as f64
    } else {
        0.0
    };
    let drift = match (stats.heard, stats.sound_now) {
        (Some((first, heard)), Some((last, now))) => {
            let system = i128::try_from(last.duration_since(first).as_nanos()).unwrap_or(0);
            Some(millis(now - heard - system))
        }
        _ => None,
    };
    let duration = |time: Duration| time.as_secs_f64() * 1e3;
    log(
        "transport_played",
        &json!({
            "media": loaded.media,
            "speed": playing.clock.speed(),
            "clock": if playing.sound { "sound" } else { "system" },
            "decoder": loaded.video.as_ref().map(|video| video.decoder.path_used().name()),
            "seconds": seconds,
            "presented": stats.presented,
            "skipped": stats.skipped,
            "passed": stats.passed,
            "late": stats.late,
            "late_worst_ms": millis(stats.worst_late),
            "late_mean_ms": mean_late,
            "sound_jitter_ms": millis(stats.jitter),
            "sound_minus_system_ms": drift,
            "sound_latency_ms": stats.latency.map(|(latency, _)| duration(latency)),
            "sound_run_ms": stats.latency.map(|(_, run)| duration(run)),
            "sound_short_runs": stats.short_runs,
        }),
    );
}

/// The time of the clock that `playing` goes by, in nanoseconds, never going back: the time of
/// the sound heard, smoothed, or 0 until it is first read; or the monotonic clock.
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

/// How much of the sound started last has been heard, in nanoseconds of it: fewer than none
/// before its first sample is heard.
fn sound_nanos(output: &Output, now: Instant) -> Option<i128> {
    let played = output.played(now)?;
    Some(i128::from(played) * NANOS / i128::from(output.format().rate.max(1)))
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

/// Log what failed while the transport decoded.
fn failed(media: &str, error: &MediaError) {
    log(
        "transport_error",
        &json!({ "media": media, "reason": error.reason(), "message": error.to_string() }),
    );
}

impl Loaded {
    /// Log what failed while the picture was decoded; a proxy that cannot be decoded (a frame of
    /// it, a seek in it, or reading it, as a file that was damaged) is to be let go
    /// ([`Worker::recover`]).
    fn failed(&mut self, error: &MediaError) {
        failed(&self.media, error);
        if self.source == Source::Proxy
            && matches!(
                error,
                MediaError::Decode(_) | MediaError::Read(_) | MediaError::Seek(_)
            )
        {
            self.broken = true;
        }
    }

    /// Keep a picture to be shown again at once; past the most that is kept, the one furthest
    /// from the playhead goes, so that those just shown stay.
    fn remember(&mut self, picture: Arc<Picture>) {
        let most = self
            .video
            .as_ref()
            .map_or(0, |video| video.limits.recent_paused);
        self.recent.retain(|kept| kept.frame != picture.frame);
        self.recent.push_back(picture);
        while self.recent.len() > most {
            let playhead = self.playhead;
            let furthest = self
                .recent
                .iter()
                .enumerate()
                .max_by_key(|(_, kept)| (kept.frame - playhead).abs())
                .map(|(index, _)| index);
            match furthest {
                Some(index) => drop(self.recent.remove(index)),
                None => break,
            }
        }
    }

    /// Show `frame` as it is, and say whether it is the frame at the playhead: not if it
    /// could not be decoded.
    fn show_exact(&mut self, frame: i64, pipe: &mut Pipe, world: &World) -> Result<(), MediaError> {
        let shown = self.show(frame, pipe, world);
        self.exact = shown.is_ok();
        shown
    }

    /// Show `frame`: the picture shown already, one of the recent pictures, or decoded and
    /// made a picture now. A frame past the end, which the facts of the media promised, is the
    /// last that is there. Sound alone shows nothing.
    fn show(&mut self, frame: i64, pipe: &mut Pipe, world: &World) -> Result<(), MediaError> {
        let probe = matches!(&self.mode, Mode::Playing(playing) if playing.clock.speed() < 0);
        let begun = Instant::now();
        let at = world.now();
        let Some(video) = &mut self.video else {
            return Ok(());
        };
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
        let decoded = match video.decoder.decoded_at(frame) {
            Err(MediaError::OutOfRange {
                last: Some(last), ..
            }) if frame > last => {
                self.last = last;
                self.playhead = self.playhead.min(last);
                video.decoder.decoded_at(last)?
            }
            other => other?,
        };
        let frame = decoded.frame();
        let decoded_at = Instant::now();
        let picture = match pipe.convert_now(decoded) {
            Err(MediaError::Device(why)) => {
                // The card failed as the frame was copied: the decoder decodes it again on the
                // processor.
                log(
                    "transport_device_failed",
                    &json!({ "media": self.media, "frame": frame, "message": why }),
                );
                video.decoder.device_failed(&why);
                let decoded = video.decoder.decoded_at(frame)?;
                pipe.convert_now(decoded)?
            }
            other => other?,
        };
        self.present(picture, world);
        if probe {
            log(
                "transport_probe",
                &json!({
                    "frame": frame,
                    "at_ns": at,
                    "decode_us": decoded_at.duration_since(begun).as_micros(),
                    "convert_us": decoded_at.elapsed().as_micros(),
                }),
            );
        }
        Ok(())
    }

    fn present(&mut self, picture: Arc<Picture>, world: &World) {
        if let Some(video) = &self.video {
            world.present(&picture, video.rotation);
        }
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
                Command::Source { media, .. } => format!("source {media}"),
                Command::Proxies { on } => format!("proxies {on}"),
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
    fn the_pictures_held_take_at_most_384_mib() {
        let mib = |bytes: u64| bytes / (1024 * 1024);
        for (width, height, bits, pool, ahead) in [
            (1920, 1080, 8, 12, 8),
            (3840, 2160, 10, 12, 8),
            (5760, 3240, 10, 7, 5),
            (7680, 4320, 10, 4, 2),
            (15360, 8640, 10, 4, 2),
        ] {
            let bytes = Limits::picture_bytes(width, height, bits);
            let limits = Limits::of(bytes);
            assert_eq!(
                (limits.pool, limits.ahead),
                (pool, ahead),
                "{width}×{height}"
            );
            assert_eq!(
                limits.pool,
                limits.ahead + limits.recent_playing + 2,
                "{width}×{height}"
            );
            assert!(limits.in_flight <= limits.ahead && limits.in_flight > 0);
            assert!(limits.recent_paused + 3 <= limits.pool);
            if pool > 4 {
                assert!(
                    mib(bytes * u64::try_from(pool).unwrap()) <= 384,
                    "{width}×{height}"
                );
            }
        }
        // 4K in P010: rows of 7680 bytes, a plane and a half of them.
        assert_eq!(Limits::picture_bytes(3840, 2160, 10), 7680 * 3240);
        assert_eq!(Limits::picture_bytes(1921, 1081, 8), 1984 * (1081 + 541));
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
