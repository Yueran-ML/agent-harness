//! The timeline monitor. Decode workers prepare pictures; this thread owns the clock and
//! compositor, and never holds the project/session lock while waiting for a picture or GPU.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{mpsc, Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use columbina_compose::{
    Compositor, LayerColor, Scene, SceneLayer, Source as LayerSource, Target, Text,
};
use columbina_media::Hardware;
use columbina_model::project::TrackKind;
use columbina_model::{resolve, Facts, Grade, Identity, Media, Project, Proxy};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::oneshot;

use super::ahead::{Request, TrackReader, Want};
use super::sources::{Read, Sources};
use super::{Action, Notify};
use crate::compare::{Change, Comparison};
use crate::gpu::{Graphics, Subscription};
use crate::grading::{Grading, LutPlaces, MissingLut};
use crate::highlight::Highlight;
use crate::log::log;
use crate::presentation::{self, FrameSnapshot, Origin, Registry};
use crate::rpc::{self, Fault};
use crate::scopes::{Scopes, Shown};
use crate::transport::clock::{self, Clock};
use crate::transport::{Again, Sound, SoundSlot, Source, POOL_BYTES, SETTLE, TOLD_EVERY};
use crate::view::{Monitor, ViewSlot};
use crate::viewer_preview::Preview;
use columbina_model::keyframes::KeyClock;

const SEEK_WAIT: Duration = Duration::from_secs(5);

/// How often a monitor that is still, and shows a frame drawn while the pipelines of its grades
/// were being made, looks whether they are made, to draw it again.
const COMPILED_POLL: Duration = Duration::from_millis(10);

/// How far ahead of the picture the sound is started again after a session that was slow to
/// start (T5's review), the picture going on by the clock of the system until it is heard.
const REJOIN_LEAD: Duration = Duration::from_millis(500);

/// How long a session of grains of scrubbing is kept once its last grain was asked for (T18),
/// so that the grains of a drag play in one: after that, it ends.
const SCRUB_IDLE: Duration = Duration::from_secs(1);

/// How long a session of grains of scrubbing told to fade out is kept before it ends (T18's
/// review): the sound ahead of the device, its latency, and the fade.
const SCRUB_FADE_OUT: Duration = Duration::from_millis(100);

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "the state of the monitor as the protocol writes it, a field each"
)]
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
    /// Whether dragging the playhead plays the sound under it (`transport.scrub_audio`, Timeline ›
    /// Audio Scrubbing, p. 731; on at first).
    pub scrub_audio: bool,
    /// The LUTs that the grades of the timeline name and that cannot be read (left out where
    /// there are none).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub missing_luts: Vec<MissingLut>,
    /// The Highlight mode that the monitor shows, where one is on (`viewer.highlight`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlight: Option<Highlight>,
    /// Whether the monitor shows the picture without its grades (`viewer.bypass`, Bypass
    /// Color); left out where it does not.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub bypass: bool,
    /// The Live Preview that the monitor shows, where one is on (`viewer.preview`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<Preview>,
    /// Whether the view shows the Color Viewer LUT (`viewer.lut`); written only where it does
    /// not.
    #[serde(skip_serializing_if = "crate::viewer_preview::is_on")]
    pub viewer_lut: bool,
    /// Whether the colour management that the project asks for is being made, the colour in
    /// force shown meanwhile; left out where it is not.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub color_pending: bool,
    /// Why the colour management that the project asks for could not be made, while it cannot;
    /// left out where it was.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_error: Option<String>,
    /// The frame of the timeline that each reference of the wipe or split screen shows for the
    /// frame shown, as it moves with the playhead (`viewer.wipe`, `viewer.split`); left out
    /// where nothing is compared.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compared: Option<Vec<Option<i64>>>,
    /// Whether automation is recorded as the timeline plays (`automation.record`, T16); left
    /// out where it is not.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub recording: bool,
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
            scrub_audio: true,
            missing_luts: Vec::new(),
            highlight: None,
            bypass: false,
            preview: None,
            viewer_lut: true,
            color_pending: false,
            color_error: None,
            compared: None,
            recording: false,
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
        revision: u64,
        project: Arc<Project>,
        id: String,
        reply: oneshot::Sender<Result<Status, Fault>>,
    },
    Act(Action),
    Project(u64, Arc<Project>),
    Proxy(String),
    Redraw,
    Window(crate::window::Update),
    FreezeAt {
        timeline: String,
        revision: u64,
        frame: i64,
        capture: u64,
        reply: oneshot::Sender<Result<(), Fault>>,
    },
    Pause(oneshot::Sender<()>),
    Digest(oneshot::Sender<Result<Value, Fault>>),
    /// What is shown, for the scopes to measure.
    Shown(oneshot::Sender<Result<Shown, Fault>>),
    /// What the frame is compared with (`viewer.wipe`, `viewer.split`).
    Compare(Change, oneshot::Sender<Result<Value, Fault>>),
    /// The frame itself, not what it is compared with, for a still (`still.grab`).
    Original(oneshot::Sender<Result<crate::stills::Frame, Fault>>),
    /// The Highlight mode that is shown (`viewer.highlight`).
    Highlight(Option<Highlight>, oneshot::Sender<Value>),
    /// The Live Preview that is shown (`viewer.preview`).
    Preview(Option<Preview>, oneshot::Sender<Value>),
    /// Whether the view shows the Color Viewer LUT (`viewer.lut`).
    ViewerLut(bool, oneshot::Sender<Value>),
    /// Whether the grades are left out of the picture (`viewer.bypass`).
    Bypass(bool, oneshot::Sender<Value>),
    /// The recording of automation (T16): turned on or off, a control touched, Punch In, and
    /// what writes a pass.
    Automation(super::record::Order),
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
    #[allow(
        clippy::too_many_arguments,
        reason = "the existing monitor services plus its frame registry"
    )]
    pub fn new(
        graphics: Arc<Graphics>,
        view: ViewSlot,
        frames: Registry,
        notify: Notify,
        again: Again,
        sound: crate::audio::Kind,
        luts: LutPlaces,
        scopes: Scopes,
    ) -> Self {
        let (inbox, received) = mpsc::channel();
        frames.redraw_on_invalidate(Monitor::Timeline, {
            let inbox = inbox.clone();
            Arc::new(move || {
                let _ = inbox.send(Command::Redraw);
            })
        });
        let shared = Arc::new(Mutex::new(Shared::default()));
        let subscription = graphics.subscribe({
            let inbox = inbox.clone();
            move |_| {
                let _ = inbox.send(Command::Redraw);
            }
        });
        // The colour management of a project made ready on a thread of its own: draw again.
        let wake: Arc<dyn Fn() + Send + Sync> = {
            let inbox = Mutex::new(inbox.clone());
            Arc::new(move || {
                let _ = lock(&inbox).send(Command::Redraw);
            })
        };
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
                        frames,
                        revision: 0,
                        window: crate::window::Update::default(),
                        frozen: 0,
                        graphics,
                        sources: Sources::new(Hardware::Auto, again),
                        grading: Grading::monitor(luts, wake),
                        scopes,
                        output: None,
                        compare: Comparison::new(),
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
                        aside: false,
                        clock: None,
                        sound: SoundSlot::new(sound),
                        playback: super::sound::Playback::new(),
                        automation: super::record::Automation::default(),
                        joining: None,
                        sound_late: false,
                        mixed_from: None,
                        scrub_until: None,
                        speed_change: None,
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

    pub fn begin_load(
        &self,
        revision: u64,
        project: Arc<Project>,
        id: String,
    ) -> Result<Loading, Fault> {
        let (reply, receiver) = oneshot::channel();
        lock(&self.0.shared).loading += 1;
        if let Err(error) = self.send(Command::Load {
            revision,
            project,
            id,
            reply,
        }) {
            lock(&self.0.shared).loading -= 1;
            return Err(error);
        }
        Ok(Loading(receiver))
    }

    pub fn act(&self, action: Action) {
        let _ = self.send(Command::Act(action));
    }
    pub fn project_changed(&self, revision: u64, project: &Project) {
        let _ = self.send(Command::Project(revision, Arc::new(project.clone())));
    }
    pub fn proxy_ready(&self, media: &str) {
        let _ = self.send(Command::Proxy(media.into()));
    }
    pub fn redraw(&self) {
        let _ = self.send(Command::Redraw);
    }

    pub fn window(&self, update: crate::window::Update) {
        let _ = self.send(Command::Window(update));
    }

    pub fn freeze_at(
        &self,
        timeline: String,
        revision: u64,
        frame: i64,
        capture: u64,
    ) -> Result<oneshot::Receiver<Result<(), Fault>>, Fault> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::FreezeAt {
            timeline,
            revision,
            frame,
            capture,
            reply,
        })?;
        Ok(receiver)
    }
    pub fn stop(&self) {
        let _ = self.send(Command::Stop);
    }

    /// Tells the thread of the monitor an order of the recording of automation (T16).
    ///
    /// # Errors
    /// The monitor has stopped.
    pub(crate) fn automation(&self, order: super::record::Order) -> Result<(), Fault> {
        self.send(Command::Automation(order))
    }

    pub async fn pause(&self) -> Result<(), Fault> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::Pause(reply))?;
        receiver.await.map_err(|_| stopped())
    }

    /// The frame shown, as it was composed.
    pub async fn shown(&self) -> Result<Shown, Fault> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::Shown(reply))?;
        receiver.await.map_err(|_| stopped())?
    }

    pub async fn digest(&self) -> Result<Value, Fault> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::Digest(reply))?;
        receiver.await.map_err(|_| stopped())?
    }

    /// Change what the frame is compared with, and give what it is compared with now.
    pub async fn compare(&self, change: Change) -> Result<Value, Fault> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::Compare(change, reply))?;
        receiver.await.map_err(|_| stopped())?
    }

    /// The frame shown, as it was composed, before it was compared with anything; with the
    /// project and timeline it was drawn of.
    pub async fn original(&self) -> Result<crate::stills::Frame, Fault> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::Original(reply))?;
        receiver.await.map_err(|_| stopped())?
    }

    /// Show a Highlight mode, or none, and give the mode shown now.
    pub async fn highlight(&self, highlight: Option<Highlight>) -> Result<Value, Fault> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::Highlight(highlight, reply))?;
        receiver.await.map_err(|_| stopped())
    }

    /// Show the picture without its grades, or with them, and say which it shows now.
    pub async fn bypass(&self, on: bool) -> Result<Value, Fault> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::Bypass(on, reply))?;
        receiver.await.map_err(|_| stopped())
    }

    /// Show a Live Preview, or none, and give the preview shown now.
    pub async fn preview(&self, preview: Option<Preview>) -> Result<Value, Fault> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::Preview(preview, reply))?;
        receiver.await.map_err(|_| stopped())
    }

    /// Show the Color Viewer LUT on the view, or not, and say which it does now.
    pub async fn viewer_lut(&self, on: bool) -> Result<Value, Fault> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::ViewerLut(on, reply))?;
        receiver.await.map_err(|_| stopped())
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

#[derive(Clone)]
struct Previous {
    read: Read,
    origin: Origin,
}

struct Worker {
    inbox: mpsc::Receiver<Command>,
    shared: Arc<Mutex<Shared>>,
    notify: Notify,
    view: ViewSlot,
    frames: Registry,
    revision: u64,
    window: crate::window::Update,
    frozen: u64,
    graphics: Arc<Graphics>,
    sources: Sources,
    /// The grades of the timeline loaded, ready on the card.
    grading: Grading,
    /// What the frames are put out as, as the scopes and the view were last told.
    output: Option<crate::managed::Output>,
    /// The scopes of the timeline monitor, told each frame shown.
    scopes: Scopes,
    /// What the frame is compared with: a wipe or a split screen.
    compare: Comparison,
    status: Status,
    project: Option<Arc<Project>>,
    timeline: Option<usize>,
    generation: u64,
    readers: BTreeMap<String, TrackReader>,
    reader_budget: usize,
    previous: BTreeMap<(String, String), Previous>,
    compositor: Option<Compositor>,
    targets: Vec<Arc<Target>>,
    current: Option<Arc<Target>>,
    /// Whether the frame being drawn is drawn aside, for a still, and not shown: handed to no
    /// view and told to no scope ([`Worker::graded_original`]).
    aside: bool,
    clock: Option<Clock>,
    sound: SoundSlot,
    /// The sound of the timeline as it plays: its graph, the thread of the mix, and the changes
    /// of the project sent to it (plan T5).
    playback: super::sound::Playback,
    /// The recording of automation (plan T16).
    automation: super::record::Automation,
    /// Where a session started ahead of the picture, after one that was slow to start, starts
    /// sounding: the picture goes by the clock of the system until its sound is heard.
    joining: Option<i128>,
    /// The first graph of the session was slow to start: the picture goes by the clock of the
    /// system, and the sound joins it once that graph is ready.
    sound_late: bool,
    /// The sample of the timeline that the mixer that plays started at, and the timeline's
    /// rate of samples: where the sound is, for the test hook to tell.
    mixed_from: Option<(i64, u32)>,
    /// When the session of grains of scrubbing that plays ends, its last grain long played (T18).
    scrub_until: Option<Instant>,
    /// The change of speed of the sound heard last, and whether the clock of the picture is yet
    /// to take it: the instant of the sound (`clock_now`) from which it plays at that speed
    /// (T18's review: the picture changes its speed when that instant comes, not before).
    speed_change: Option<(i128, i32, bool)>,
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
        self.playback.stop();
        self.sound.stop();
        self.joining = None;
        self.sound_late = false;
        self.status.sound = if self.has_sound() {
            Sound::Off
        } else {
            Sound::None
        };
        self.clock = Some(Clock::at(place, clock.speed(), clock.rate(), self.now()));
        self.scrub_until = None;
        self.speed_change = None;
        if !self.has_sound() {
            return;
        }
        let (Some(project), Some(index)) = (&self.project, self.timeline) else {
            return;
        };
        let timeline = &project.timelines[index];
        // At any speed, forward or backward, its pitch following it (T18).
        let speed = clock.speed();
        let nanos = Clock::at(place, 1, clock.rate(), 0).media_nanos(0);
        let from = i64::try_from(nanos * i128::from(timeline.sample_rate) / clock::NANOS);
        let started = from.as_ref().ok().copied();
        let result = from
            .map_err(|_| "timeline audio start is outside sample time".to_owned())
            .and_then(|from| {
                let output = self.sound.ready().ok_or("audio output is unavailable")?;
                self.playback
                    .start(output, Arc::clone(project), index, (from, speed))
            });
        match result {
            Ok(()) => {
                self.status.sound = Sound::On;
                self.heard.set(0);
                self.clock = Some(Clock::at(place, speed, clock.rate(), 0));
                self.mixed_from = started.map(|from| (from, timeline.sample_rate));
                // A pass of the recording of automation, where it is on (T16).
                if let Some(from) = started {
                    self.automation.begin(project, index, from);
                }
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

    /// Starts the sound again where the picture will be [`REJOIN_LEAD`] on, the picture going on
    /// by the clock of the system until it is heard ([`Worker::check_sound`]): after a session
    /// that was slow to start (T5's review). The graph is built again as the one that was slow
    /// was, now quickly (its media read once, its decoders kept); where it is slow again, the
    /// picture steps back to the sound when it comes.
    fn rejoin_sound(&mut self) {
        let (Some(clock), Some(project), Some(index)) = (self.clock, &self.project, self.timeline)
        else {
            return;
        };
        // At the speed asked for last, which a change still in flight may not have brought the
        // picture to yet (T18's review).
        let clock = clock.with_speed(self.status.speed, self.now());
        self.clock = Some(clock);
        self.speed_change = None;
        if !self.has_sound() {
            return;
        }
        log(
            "transport_sound_rejoins",
            &json!({"monitor":"timeline",
                "built_ms": self.playback.last_start().as_millis(),
                "lead_ms": REJOIN_LEAD.as_millis()}),
        );
        let lead = REJOIN_LEAD;
        let place = clock.place(self.now())
            + i128::try_from(lead.as_nanos()).unwrap_or(0)
                * i128::from(clock.speed())
                * i128::from(clock.rate().num);
        let timeline = &project.timelines[index];
        let nanos = Clock::at(place, 1, clock.rate(), 0).media_nanos(0);
        let Ok(from) = i64::try_from(nanos * i128::from(timeline.sample_rate) / clock::NANOS)
        else {
            return;
        };
        let rate = timeline.sample_rate;
        let project = Arc::clone(project);
        let Some(output) = self.sound.ready() else {
            return;
        };
        match self
            .playback
            .start(output, project, index, (from, clock.speed()))
        {
            Ok(()) => {
                self.joining = Some(place);
                self.mixed_from = Some((from, rate));
            }
            Err(message) => log(
                "sound_unavailable",
                &json!({"monitor":"timeline","message":message}),
            ),
        }
    }

    /// What the thread that builds the mix has done taken: where the first graph of a session
    /// is slow to start, the picture goes on by the clock of the system and the sound joins it
    /// once that graph is ready; where the device or the graph of the session failed, playing
    /// goes on by the clock of the system.
    fn check_sound(&mut self) {
        let event = self.playback.poll();
        if let (Some(place), Some(clock)) = (self.joining, self.clock) {
            let heard = self
                .sound
                .working()
                .and_then(|output| output.played(Instant::now()))
                .is_some();
            if heard {
                self.joining = None;
                self.sound_late = false;
                self.status.sound = Sound::On;
                self.heard.set(0);
                self.clock = Some(Clock::at(place, clock.speed(), clock.rate(), 0));
                log(
                    "transport_sound_back",
                    &json!({"monitor":"timeline","message":"the sound joins the picture"}),
                );
                self.tell(true);
            }
        }
        match &event {
            Some(super::sound::Event::Slow) if self.status.sound == Sound::On => {
                if let Some(clock) = self.clock {
                    let place = clock.place(self.heard.get());
                    self.clock = Some(Clock::at(place, clock.speed(), clock.rate(), self.now()));
                }
                self.status.sound = Sound::Unavailable;
                self.sound_late = true;
                log(
                    "transport_sound_late",
                    &json!({"monitor":"timeline","message":"the sound is slow to start; playback uses the system clock until it comes"}),
                );
                self.tell(true);
            }
            Some(super::sound::Event::Ready)
                if self.sound_late && self.joining.is_none() && self.status.playing =>
            {
                self.rejoin_sound();
            }
            Some(super::sound::Event::Speed { at, speed }) => self.speed_heard(*at, *speed),
            _ => {}
        }
        let failed = matches!(event, Some(super::sound::Event::Failed(_)));
        if failed && (self.joining.is_some() || self.sound_late) {
            self.playback.stop();
            self.sound.stop();
            self.joining = None;
            self.sound_late = false;
        }
        if self.status.sound == Sound::On && (self.sound.working().is_none() || failed) {
            if let Some(clock) = self.clock {
                let place = clock.place(self.heard.get());
                self.clock = Some(Clock::at(place, clock.speed(), clock.rate(), self.now()));
            }
            self.playback.stop();
            self.sound.stop();
            self.status.sound = Sound::Unavailable;
            log(
                "transport_sound_lost",
                &json!({"monitor":"timeline","message":"the sound failed; playback uses the system clock"}),
            );
            self.tell(true);
        }
    }

    /// The sound plays at `speed` from its sample `at` out on (T18): the clock of the picture,
    /// which the sound is, changes its speed at the instant of the sound when that sample is
    /// heard, so that the place does not jump and the picture keeps to the sound.
    fn speed_heard(&mut self, at: i64, speed: i32) {
        let (Some(_), Some((from, rate))) = (self.clock, self.mixed_from) else {
            return;
        };
        if self.status.sound != Sound::On || self.joining.is_some() {
            return;
        }
        // The sound out is counted from `from` at the rate of the timeline, as the device's
        // instants are (`clock_now`).
        let since = i128::from(at - from) * clock::NANOS / i128::from(rate.max(1));
        self.speed_change = Some((since, speed, true));
        self.clock_due();
        self.tell(true);
    }

    /// The clock of the picture, its change of speed taken where the instant of it has come.
    fn clock_due(&mut self) -> Option<Clock> {
        if let (Some((since, speed, true)), Some(clock)) = (self.speed_change, self.clock) {
            if self.clock_now() >= since {
                self.clock = Some(clock.with_speed(speed, since));
                self.speed_change = Some((since, speed, false));
            }
        }
        self.clock
    }

    /// Plays the grain of scrubbing of `frame` (T18), where scrubbing plays sound: in the session
    /// of grains that plays, or a new one.
    fn scrub_sound(&mut self, frame: i64) {
        if !self.status.scrub_audio || !self.has_sound() {
            return;
        }
        let (Some(project), Some(index)) = (&self.project, self.timeline) else {
            return;
        };
        let timeline = &project.timelines[index];
        let from = resolve::samples_at(frame, timeline.rate, timeline.sample_rate);
        let project = Arc::clone(project);
        let Some(output) = self.sound.ready() else {
            return;
        };
        match self.playback.scrub(output, project, index, from) {
            Ok(()) => self.scrub_until = Some(Instant::now() + SCRUB_IDLE),
            Err(message) => log(
                "sound_unavailable",
                &json!({"monitor":"timeline","message":message}),
            ),
        }
    }

    /// Ends the session of grains of scrubbing, if one plays: at once, or, where it `fade`s,
    /// once what it plays has faded out over 5 ms and been heard (T18's review).
    fn end_scrub(&mut self, fade: bool) {
        if self.scrub_until.is_none() || !self.playback.scrubbing() {
            self.scrub_until = None;
            return;
        }
        if fade && self.playback.fade_out() {
            self.scrub_until = Some(Instant::now() + SCRUB_FADE_OUT);
            return;
        }
        self.scrub_until = None;
        self.playback.stop();
        self.sound.stop();
    }

    fn wait(&self) -> Option<Duration> {
        let wait = self.wait_for_picture();
        // A change of speed still to come: woken when its instant does.
        let wait = match self.speed_change {
            Some((since, _, true)) => {
                let nanos = u64::try_from((since - self.clock_now()).max(0)).unwrap_or(u64::MAX);
                let change = Duration::from_nanos(nanos);
                Some(wait.map_or(change, |wait| wait.min(change)))
            }
            _ => wait,
        };
        match self.scrub_until {
            Some(until) => {
                let scrub = until.saturating_duration_since(Instant::now());
                Some(wait.map_or(scrub, |wait| wait.min(scrub)))
            }
            None => wait,
        }
    }

    fn wait_for_picture(&self) -> Option<Duration> {
        if let Some(clock) = self.clock {
            let now = self.clock_now();
            let next = clock
                .frame(now)
                .saturating_add(i64::from(clock.speed().signum()));
            let nanos = (clock.when(next) - now).max(0);
            Some(Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX)).min(TOLD_EVERY))
        } else {
            let settle = self
                .settle
                .map(|at| at.saturating_duration_since(Instant::now()));
            if self.is_provisional() {
                Some(settle.map_or(COMPILED_POLL, |settle| settle.min(COMPILED_POLL)))
            } else {
                settle
            }
        }
    }

    /// Whether the frame shown was drawn while the pipelines of its grades were being made,
    /// with the grades of the frames before (see `Compositor::compile_in_background`).
    fn is_provisional(&self) -> bool {
        self.compositor
            .as_ref()
            .is_some_and(Compositor::is_provisional)
    }

    /// The frame shown drawn again once the pipelines of its grades are made, if it was drawn
    /// before they were and the monitor is still: what is measured is what the monitor settles
    /// on.
    fn finish_compiling(&mut self) {
        if !self.is_provisional() {
            return;
        }
        if let Some(compositor) = &mut self.compositor {
            compositor.finish_compiling();
        }
        if self.clock.is_none() {
            let _ = self.draw(true, false);
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
                    | (Some(old @ Command::Project(..)), new @ Command::Project(..)) => *old = new,
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

    /// Draw again where not playing; and tell the state where the LUTs that cannot be read are
    /// others, as a colour management made since may read LUTs that the one before could not,
    /// or not those it could.
    fn redraw(&mut self) {
        if !self.status.playing {
            let missing = self.status.missing_luts.clone();
            let colour = (self.status.color_pending, self.status.color_error.clone());
            let _ = self.draw(true, false);
            if self.status.missing_luts != missing
                || (self.status.color_pending, &self.status.color_error) != (colour.0, &colour.1)
            {
                self.tell(true);
            }
        }
    }

    /// The sample of the timeline heard now, while it plays at speed 1 (T16): by the device's
    /// clock where the sound is on, else by the clock of the system.
    fn heard_sample(&self) -> Option<i64> {
        let (Some(clock), Some(project), Some(index)) = (self.clock, &self.project, self.timeline)
        else {
            return None;
        };
        // A change of speed asked for and still to come counts as the speed asked for (T18).
        if !self.status.playing || clock.speed() != 1 || self.status.speed != 1 {
            return None;
        }
        let place = clock.place(self.clock_now());
        let nanos = Clock::at(place, 1, clock.rate(), 0).media_nanos(0);
        let rate = i128::from(project.timelines[index].sample_rate);
        i64::try_from(nanos * rate / clock::NANOS).ok()
    }

    /// Ends the pass of the recording of automation under way where the timeline is heard now,
    /// and writes it (T16): where it stops, or plays at another speed.
    fn end_pass(&mut self) {
        if let Some(project) = self.project.clone() {
            let at = self.heard_sample();
            self.automation.end(&project, at, &mut self.playback);
        }
    }

    fn pause(&mut self) {
        // A pass of the recording of automation ends where the timeline stops (T16).
        self.end_pass();
        self.playback.stop();
        self.sound.stop();
        self.scrub_until = None;
        self.speed_change = None;
        self.joining = None;
        self.sound_late = false;
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
        self.grading.clear();
        self.scopes.cleared(Monitor::Timeline);
        self.compare.forget();
        self.current = None;
        self.targets.clear();
        self.status = Status {
            proxies: self.status.proxies,
            scrub_audio: self.status.scrub_audio,
            ..Status::default()
        };
        let instance = {
            let slot = lock(&self.view);
            slot.as_ref().map(|view| {
                view.clear();
                view.instance
            })
        };
        if let Some(instance) = instance {
            self.frames.reset(Monitor::Timeline, instance);
        }
    }

    #[allow(clippy::too_many_lines, reason = "an arm for each command")]
    fn apply(&mut self, command: Command) {
        match command {
            Command::Load {
                revision,
                project,
                id,
                reply,
            } => {
                self.unload();
                self.status.timeline = Some(id);
                self.status.frame = Some(0);
                self.replace(revision, project);
                let drawn = self.draw(true, false);
                lock(&self.shared).loading -= 1;
                self.tell(true);
                let _ = reply.send(drawn.map(|()| self.status.clone()));
            }
            Command::Project(revision, project) => {
                if self.status.timeline.is_none() {
                    return;
                }
                let changed = self.replace(revision, project);
                if (changed || self.window.selection.is_some()) && !self.status.playing {
                    let _ = self.draw(true, false);
                }
                self.tell(true);
            }
            Command::Proxy(media) => {
                self.sources.proxy_ready(&media);
                if self.status.timeline.is_none() || !self.status.proxies || !self.shows(&media) {
                    return;
                }
                self.invalidate();
                if !self.status.playing {
                    let _ = self.draw(true, false);
                }
                self.tell(true);
            }
            Command::Redraw => self.redraw(),
            Command::Window(update) => {
                if update.sequence > self.window.sequence {
                    self.window = update;
                    // A State-owned move sends its refreshed immutable selection before
                    // Project. Do not present an intermediate old-project/no-overlay frame.
                    if self
                        .window
                        .selection
                        .as_ref()
                        .is_none_or(|s| s.revision == self.revision)
                    {
                        self.redraw();
                    }
                }
            }
            Command::FreezeAt {
                timeline,
                revision,
                frame,
                capture,
                reply,
            } => {
                let result = if self.status.timeline.as_ref() != Some(&timeline)
                    || self.revision != revision
                {
                    Err(crate::window::refused("freeze_target"))
                } else {
                    self.pause();
                    self.settle = None;
                    self.status.frame = Some(frame);
                    self.frozen = capture;
                    let drawn = self.draw(true, false);
                    self.tell(true);
                    drawn
                };
                let _ = reply.send(result);
            }
            Command::Automation(order) => self.automate(order),
            Command::Pause(reply) => {
                self.pause();
                if self.status.timeline.is_some() {
                    self.tell(true);
                }
                let _ = reply.send(());
            }
            Command::Digest(reply) => {
                self.finish_compiling();
                let sound = (self.status.sound == Sound::On)
                    .then(|| {
                        self.sound.working().and_then(|output| {
                            let samples = output.played(Instant::now())?;
                            // Where the mixer is in the timeline: the sample that it started
                            // at, and as many more as have been heard, at the timeline's rate.
                            // At a speed, where the timeline is at that sample out (T18).
                            let mixed = self.mixed_from.map(|(from, rate)| {
                                let out = from
                                    + i64::try_from(
                                        i128::from(samples) * i128::from(rate)
                                            / i128::from(output.format().rate),
                                    )
                                    .unwrap_or(i64::MAX);
                                self.playback.timeline_at(out).unwrap_or(out)
                            });
                            Some((
                                samples,
                                self.clock.map(|clock| clock.frame(self.clock_now())),
                                mixed,
                                (output.short_runs(), output.capture_dropped()),
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
                        if let Some((samples, frame, mixed, (short, dropped))) = sound {
                            digest["audio_samples"] = json!(samples);
                            digest["audio_frame"] = json!(frame);
                            digest["audio_sample"] = json!(mixed);
                            // What the threads of the sound counted (T5): runs of the device short
                            // of sound, reads of clips not yet read ahead, graphs swapped in.
                            let (underruns, swaps) = self.playback.counts();
                            digest["audio_short_runs"] = json!(short);
                            digest["audio_underruns"] = json!(underruns);
                            digest["audio_swaps"] = json!(swaps);
                            let (late, priority) = self.playback.mix_thread();
                            digest["audio_late_swaps"] = json!(late);
                            digest["audio_mix_priority"] = json!(format!("{priority:?}"));
                            digest["audio_capture_dropped"] = json!(dropped);
                        }
                        digest
                    });
                let _ = reply.send(result);
            }
            Command::Shown(reply) => {
                self.finish_compiling();
                let shown = match (&self.current, &self.compositor, self.status.shown) {
                    (Some(target), Some(compositor), Some(frame)) => Ok(Shown::Target {
                        gpu: Arc::clone(compositor.gpu()),
                        target: Arc::clone(target),
                        frame,
                    }),
                    _ => Err(not_loaded()),
                };
                let _ = reply.send(shown);
            }
            Command::Compare(change, reply) => {
                let _ = reply.send(self.compare_with(change));
            }
            Command::Original(reply) => {
                self.finish_compiling();
                let _ = reply.send(self.graded_original());
            }
            Command::Highlight(highlight, reply) => {
                if self.status.highlight != highlight {
                    self.status.highlight = highlight;
                    if !self.status.playing {
                        let _ = self.draw(true, false);
                    }
                    self.tell(true);
                }
                let _ = reply.send(Highlight::written(self.status.highlight.as_ref()));
            }
            Command::Bypass(on, reply) => {
                if self.status.bypass != on {
                    self.status.bypass = on;
                    if !self.status.playing {
                        let _ = self.draw(true, false);
                    }
                    self.tell(true);
                }
                let _ = reply.send(json!({"monitor": "timeline", "on": self.status.bypass}));
            }
            Command::Preview(preview, reply) => {
                if self.status.preview != preview {
                    self.status.preview = preview;
                    if !self.status.playing {
                        let _ = self.draw(true, false);
                    }
                    self.tell(true);
                }
                let _ = reply.send(Preview::written(self.status.preview.as_ref()));
            }
            Command::ViewerLut(on, reply) => {
                if self.status.viewer_lut != on {
                    self.status.viewer_lut = on;
                    self.redraw();
                    self.tell(true);
                }
                let _ = reply.send(json!({"monitor": "timeline", "on": self.status.viewer_lut}));
            }
            Command::Act(action) => self.action(action),
            Command::Stop => unreachable!("handled by run"),
        }
    }

    /// The frame shown, before it is compared with anything (`still.grab`).
    /// An order of the recording of automation (T16, `play::record`).
    fn automate(&mut self, order: super::record::Order) {
        use super::record::Order;
        let loaded = |worker: &Self, timeline: &str| {
            worker.status.timeline.as_deref() == Some(timeline) && worker.project.is_some()
        };
        match order {
            Order::Hook(hook) => self.automation.set_hook(hook),
            Order::Record {
                timeline,
                on,
                glide,
                by,
                reply,
            } => {
                let result = if loaded(self, &timeline) {
                    let project = Arc::clone(self.project.as_ref().expect("loaded"));
                    let index = self.timeline.unwrap_or(0);
                    let at = self.heard_sample();
                    let state = self.automation.record(
                        (on, glide, by),
                        (&project, index),
                        at,
                        &mut self.playback,
                    );
                    self.status.recording = self.automation.is_on();
                    self.tell(true);
                    Ok(state)
                } else {
                    Err(not_loaded())
                };
                let _ = reply.send(result);
            }
            Order::Touch {
                timeline,
                owner,
                path,
                value,
                touching,
            } => {
                if !loaded(self, &timeline) {
                    return;
                }
                let project = Arc::clone(self.project.as_ref().expect("loaded"));
                let index = self.timeline.unwrap_or(0);
                let at = self.heard_sample();
                self.automation.touch(
                    (&project, index),
                    at,
                    (&owner, &path),
                    value,
                    touching,
                    &mut self.playback,
                );
            }
            Order::Punch { timeline, reply } => {
                let result = if loaded(self, &timeline) {
                    Ok(json!({ "lanes": self.automation.punch() }))
                } else {
                    Err(not_loaded())
                };
                let _ = reply.send(result);
            }
        }
    }

    fn original(&self) -> Result<crate::stills::Frame, Fault> {
        match (
            &self.current,
            self.status.shown,
            &self.project,
            self.timeline,
        ) {
            (Some(shown), Some(at), Some(project), Some(timeline)) => Ok(crate::stills::Frame {
                target: self.compare.original(shown),
                at,
                project: Arc::clone(project),
                timeline,
            }),
            _ => Err(not_loaded()),
        }
    }

    /// The frame shown as [`Worker::original`] gives it, as it is graded: a still keeps the grade
    /// of its clip (p. 3256), so where Bypass Color or a Highlight mode shows the frame
    /// otherwise, it is drawn again aside with neither, for no view and no scope, and the frame
    /// shown is drawn again as it was shown.
    fn graded_original(&mut self) -> Result<crate::stills::Frame, Fault> {
        if !self.status.bypass && self.status.highlight.is_none() && self.status.preview.is_none() {
            return self.original();
        }
        let bypass = std::mem::take(&mut self.status.bypass);
        let highlight = self.status.highlight.take();
        let preview = self.status.preview.take();
        self.aside = true;
        let drawn = self.draw_inner(true, false);
        self.aside = false;
        let grabbed = drawn.and_then(|_| self.original());
        self.status.bypass = bypass;
        self.status.highlight = highlight;
        self.status.preview = preview;
        if !self.status.playing {
            let _ = self.draw(true, false);
        }
        grabbed
    }

    /// Change what the frame is compared with, and draw it again where it is not playing.
    fn compare_with(&mut self, change: Change) -> Result<Value, Fault> {
        let loaded = self
            .project
            .as_ref()
            .zip(self.timeline)
            .map(|(project, index)| (project.as_ref(), index, self.status.frame.unwrap_or(0)));
        let before = self.status.compared.clone();
        let result = self.compare.change(change, loaded);
        if result.is_ok() && !self.status.playing {
            let _ = self.draw(true, false);
        }
        if result.is_ok() {
            // Told at once, as the references change; while playing, as each frame is drawn.
            if self.compare.compared().is_none() {
                self.status.compared = None;
            }
            if self.status.compared != before {
                self.tell(true);
            }
        }
        result
    }

    fn invalidate(&mut self) {
        self.generation += 1;
        self.previous.clear();
        self.request(self.status.frame.unwrap_or(0), false);
    }

    /// Take a project in place of the one there was. What the change does not touch goes on as
    /// it was: a change of another timeline, of bins or of media that the timeline does not
    /// use keeps the pictures read ahead and the sound playing; one that changes only what is
    /// seen keeps the sound, and one that changes only what is heard keeps the pictures, as one
    /// that changes only grades does. Gives whether anything of the timeline loaded changed.
    fn replace(&mut self, revision: u64, project: Arc<Project>) -> bool {
        let Some(id) = self.status.timeline.as_ref() else {
            return false;
        };
        let Some(index) = project
            .timelines
            .iter()
            .position(|timeline| timeline.id == *id)
        else {
            self.unload();
            return true;
        };
        let timeline = &project.timelines[index];
        let before = self
            .project
            .as_ref()
            .zip(self.timeline)
            .map(|(before, at)| (Arc::clone(before), at));
        let (picture, sound, graded) =
            before
                .as_ref()
                .map_or((false, false, false), |(before, at)| {
                    let was = &before.timelines[*at];
                    let from = self.clock.map_or(0, |clock| {
                        let frame = clock.frame(self.clock_now()).saturating_sub(1).max(0);
                        resolve::samples_at(frame, was.rate, was.sample_rate)
                    });
                    (
                        same_picture(before, was, &project, timeline),
                        same_sound(before, was, &project, timeline, from),
                        same_grades(before, was, &project, timeline)
                            && crate::managed::Key::of(before) == crate::managed::Key::of(&project),
                    )
                });
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
        let mut fresh = false;
        if self.reader_budget != budget {
            // Track counts can change during an edit. Keeping the old per-track budgets
            // would multiply the total allowance when tracks are added.
            self.readers.clear();
            self.reader_budget = budget;
        }
        self.sources.demand(&demand(&project, timeline));
        for track in tracks {
            if !self.readers.contains_key(&track) {
                fresh = true;
                match TrackReader::new(self.sources.clone(), budget) {
                    Ok(reader) => {
                        self.readers.insert(track, reader);
                    }
                    Err(error) => Self::error(&error.to_string()),
                }
            }
        }
        self.revision = revision;
        self.project = Some(project);
        self.timeline = Some(index);
        if !picture || fresh {
            self.invalidate();
        }
        if !self.status.playing {
            self.status.sound = if self.has_sound() {
                Sound::Off
            } else {
                Sound::None
            };
        } else if (self.status.sound == Sound::On || self.joining.is_some() || self.sound_late)
            && self.playback.playing()
        {
            // The graph that plays takes the change as it plays, or another is swapped in: the
            // sound starts again only where the rates of the timeline changed (T5).
            let project = Arc::clone(self.project.as_ref().expect("taken above"));
            let heard = self.playback.changed(&project, index);
            if heard == super::sound::Heard::Again {
                self.restart_sound();
            }
        } else if !sound {
            // Sound that is not on (another speed, a device that failed) is started again as
            // before, by which it may come on.
            self.restart_sound();
        }
        !(picture && sound && graded) || fresh
    }

    /// Whether a new proxy of `media` changes what the timeline loaded shows.
    fn shows(&self, media: &str) -> bool {
        self.project
            .as_ref()
            .zip(self.timeline)
            .is_some_and(|(project, index)| {
                demand(project, &project.timelines[index]).contains_key(media)
            })
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
                // A pass is recorded at speed 1 alone: another speed ends it here (T16).
                if self.clock.is_some() && speed != self.status.speed {
                    self.end_pass();
                }
                if self.clock.is_some()
                    && self.status.sound == Sound::On
                    && self.joining.is_none()
                    && self.playback.playing()
                    && self.playback.set_speed(speed)
                {
                    // The sound changes its speed a little ahead, in the same session, and the
                    // picture with it there (`speed_heard`, T18).
                    self.status.speed = speed;
                    self.request(frame, false);
                    self.tell(true);
                    return;
                }
                self.end_scrub(false);
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
            // A grain of scrubbing is faded out (T18's review).
            Action::Pause if self.playback.scrubbing() => self.end_scrub(true),
            Action::Pause => self.pause(),
            Action::Seek { frame, scrubbing } => {
                let resume = self.clock.filter(|_| !scrubbing);
                // A session of grains of scrubbing plays on through a seek while still: the
                // grain of the last drag ends as it is (T18).
                if self.clock.is_some() {
                    self.pause();
                }
                let frame = clock::clamped(frame, last);
                self.status.frame = Some(frame);
                self.settle = scrubbing.then(|| Instant::now() + SETTLE);
                let _ = self.draw(true, scrubbing);
                if scrubbing {
                    self.scrub_sound(frame);
                }
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
            Action::ScrubAudio { on } => {
                self.status.scrub_audio = on;
                if !on {
                    self.end_scrub(true);
                }
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
        if let (Some(at), Some(project), Some(index)) =
            (self.heard_sample(), &self.project, self.timeline)
        {
            let rate = project.timelines[index].sample_rate;
            self.automation.tick(at, rate, &mut self.playback);
        }
        if self
            .scrub_until
            .is_some_and(|until| Instant::now() >= until)
        {
            self.end_scrub(false);
        }
        if let Some(clock) = self.clock_due() {
            let target = clock.frame(self.clock_now());
            let frame = clock::clamped(target, self.status.last.unwrap_or(0));
            if self.status.frame != Some(frame) {
                self.status.frame = Some(frame);
                // Drawing asks the track readers for what follows this frame.
                let _ = self.draw(false, false);
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
        } else if self.is_provisional()
            && !self
                .compositor
                .as_ref()
                .is_some_and(Compositor::is_compiling)
        {
            // The pipelines of the grades of the frame shown are made: it is drawn again.
            let _ = self.draw(true, false);
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

    /// The frame of the timeline drawn: the playhead's, or, where it is still, the frame that a
    /// Live Preview asks for (Hover Scrub Preview, `viewer.preview`).
    fn drawn_frame(&self) -> i64 {
        let frame = self.status.frame.unwrap_or(0);
        match self
            .status
            .preview
            .as_ref()
            .and_then(|preview| preview.frame)
        {
            Some(asked) if self.clock.is_none() => {
                clock::clamped(asked, self.status.last.unwrap_or(0))
            }
            _ => frame,
        }
    }

    fn draw(&mut self, wait: bool, scrubbing: bool) -> Result<(), Fault> {
        let result = self.draw_inner(wait, scrubbing).map(|_| ());
        if let Err(error) = &result {
            self.status.exact = false;
            Self::error(&error.message);
        }
        result
    }

    #[allow(
        clippy::too_many_lines,
        reason = "each kind of layer, then the grades, in the order they are drawn"
    )]
    fn draw_inner(
        &mut self,
        blocking: bool,
        scrubbing: bool,
    ) -> Result<Option<Arc<FrameSnapshot>>, Fault> {
        let (Some(project), Some(index)) = (&self.project, self.timeline) else {
            return Ok(None);
        };
        let project = Arc::clone(project);
        let timeline = &project.timelines[index];
        let frame = self.drawn_frame();
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
        let mut clips = Vec::new();
        let mut origins = Vec::new();
        for layer in resolve::video_at(timeline, &project, frame) {
            let clip = layer.clip.clone();
            let clock = timeline
                .tracks
                .iter()
                .flat_map(|track| &track.clips)
                .find(|found| found.id == clip)
                .map(|clip| KeyClock::of(clip, &project, timeline.rate));
            let mut origin = Origin {
                clip: clip.clone(),
                timeline_frame: frame,
                key_frame: clock.map_or(0, |clock| clock.at(frame)),
                media: None,
            };
            let mut editable = clock.is_some();
            let (source, color) = match layer.content {
                resolve::Content::Color { rgba } => {
                    (LayerSource::color(rgba), LayerColor::default())
                }
                resolve::Content::Title { text, style } => (
                    LayerSource::Title(Text { text, style }),
                    LayerColor::default(),
                ),
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
                        Some(Ok(read)) => {
                            origin.clip.clone_from(&read.provenance.clip);
                            if clock.is_some_and(|clock| clock.counts_source()) {
                                origin.key_frame = read.provenance.decoded_frame;
                            }
                            origin.media = Some(read.provenance.clone());
                            Some(Previous {
                                read,
                                origin: origin.clone(),
                            })
                        }
                        other => {
                            if let Some(Err(error)) = other {
                                Self::error(&error);
                            }
                            editable = false;
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
                    let Some(previous_read) = read else {
                        continue;
                    };
                    origin = previous_read.origin.clone();
                    let read = previous_read.read.clone();
                    editable &= read.exact
                        && read.provenance.decoded_frame == read.provenance.requested_frame
                        && read.provenance.clip == clip
                        && project.media.iter().any(|media| {
                            media.id == read.provenance.media
                                && media.identity == read.provenance.identity
                        });
                    exact &= read.exact;
                    original |= read.source == Source::Original;
                    proxy |= read.source == Source::Proxy;
                    previous.insert((layer.track, layer.clip), previous_read);
                    let color = crate::render::layer_color(&read.picture);
                    (LayerSource::Picture(read.picture, read.rotation), color)
                }
            };
            scene.layers.push(SceneLayer {
                source,
                transform: layer.transform.into(),
                opacity: layer.opacity,
                color,
            });
            origins.push((clip.clone(), origin, editable));
            clips.push(clip);
        }
        // Keep only the layers being shown (at most two per track), not a full-resolution
        // picture of every clip ever visited. At a late cut, that track's old picture stays.
        self.previous = previous;
        self.grade(&mut scene, &clips, frame)?;
        // Bypass Color: the picture without its grades, its colour management kept; nothing of
        // a key to highlight.
        let bypass = self.status.bypass;
        if bypass {
            crate::grading::bypass(&mut scene);
        }
        let highlight = self
            .status
            .highlight
            .as_ref()
            .filter(|highlight| {
                !bypass
                    && highlight.show(
                        &project,
                        index,
                        frame,
                        &self.grading,
                        &mut scene,
                        &mut clips,
                    )
            })
            .cloned();
        self.compare.prepare(&project, index, frame, &self.grading);
        self.status.compared = self.compare.compared();
        let snapshot = FrameSnapshot {
            revision: self.revision,
            project: Arc::clone(&project),
            timeline: index,
            timeline_frame: frame,
            canvas: (scene.width, scene.height),
            layers: presentation::layers(&scene, &clips, &origins),
            comparison: json!({"mode":"off"}),
            ready: false,
            selection: None,
            overlay: None,
            frozen: self.frozen,
            highlight,
        };
        let snapshot = self.present(&scene, frame, Some(snapshot))?;
        self.status.shown = Some(frame);
        self.status.exact = exact;
        self.status.late += u64::from(late && self.status.playing);
        self.status.source = Some(match (original, proxy) {
            (true, true) => Source::Mixed,
            (_, true) => Source::Proxy,
            _ => Source::Original,
        });
        Ok(snapshot)
    }

    /// Grade the layers of `scene`, of the clips `clips`, and its timeline, as they are at
    /// `frame`, their grades made ready on the card first where the project or the card is
    /// another; and tell the LUTs that cannot be read.
    fn grade(&mut self, scene: &mut Scene, clips: &[String], frame: i64) -> Result<(), Fault> {
        let (Some(project), Some(index)) = (&self.project, self.timeline) else {
            return Ok(());
        };
        let gpu = self
            .graphics
            .gpu()
            .map_err(|error| drawing(&error.to_string()))?;
        let project = self
            .grading
            .previewed(project, self.status.preview.as_ref());
        self.grading.update(&gpu, &project, index);
        self.grading.grade(scene, clips, frame);
        self.grading
            .make_ahead(&project, index, (scene, clips, frame));
        if self.status.missing_luts != self.grading.missing() {
            self.status.missing_luts = self.grading.missing().to_vec();
        }
        self.status.color_pending = self.grading.making();
        if self.status.color_error.as_deref() != self.grading.colour_error() {
            self.status.color_error = self.grading.colour_error().map(str::to_owned);
        }
        // The scopes measure the frame as its output space has it, and the view shows HDR as
        // HDR (each frame, so that a view attached since is told too; it changes nothing where
        // it is told the same).
        let output = self.grading.output();
        if self.output != Some(output) {
            self.output = Some(output);
            self.scopes.signal(Monitor::Timeline, output.signal);
        }
        if let Some(view) = lock(&self.view).as_ref() {
            view.set_content(crate::managed::content(&output));
        }
        Ok(())
    }

    /// Compose `scene`, the frame `frame`, hand it to the view, and tell the scopes.
    #[allow(
        clippy::too_many_lines,
        reason = "the frame composed, compared, made ready for the view and told, in order"
    )]
    fn present(
        &mut self,
        scene: &Scene,
        frame: i64,
        snapshot: Option<FrameSnapshot>,
    ) -> Result<Option<Arc<FrameSnapshot>>, Fault> {
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
            let mut compositor = Compositor::new(Arc::clone(&gpu));
            // A frame is not held up by the shaders of a grade whose shape changed: it is drawn
            // with the grade as it was until they are made (the monitor looks, COMPILED_POLL).
            compositor.compile_in_background(|| {});
            self.compositor = Some(compositor);
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
        let compositor = self.compositor.as_mut().expect("made above");
        compositor
            .render(scene, &target)
            .map_err(|error| drawing(&error.to_string()))?;
        // What the viewer shows: the frame, or the frame compared with a reference.
        let shown = self
            .compare
            .show(&gpu, compositor, &target)
            .map_err(|error| drawing(&error))?
            .unwrap_or_else(|| Arc::clone(&target));
        // A frame drawn aside is handed to no view.
        let attached = if self.aside {
            None
        } else {
            let slot = lock(&self.view);
            slot.as_ref()
                .map(|view| (view.instance, view.presentation()))
        };
        // Viewer LUT is composed before the final readiness is frozen. No ViewSlot is held
        // during this GPU work, and no registry guard spans any view operation.
        let viewed = if attached.is_some() && self.status.viewer_lut {
            Some(
                self.grading
                    .viewed(compositor, &shown)
                    .map_err(|error| drawing(&error))?,
            )
        } else {
            attached.as_ref().map(|_| Arc::clone(&shown))
        };
        // The words of a LUT that cannot be read, over the picture of the view alone (p. 3413).
        let viewed = match (viewed, crate::notice::words(&self.status.missing_luts)) {
            (Some(viewed), Some(words)) => Some(self.grading.noticed(
                &viewed,
                &words,
                crate::notice::white(self.output.as_ref()),
            )),
            (viewed, _) => viewed,
        };
        let snapshot = snapshot.map(|mut snapshot| {
            snapshot.ready =
                !compositor.is_provisional() && self.grading.colour_matches(&snapshot.project);
            snapshot.comparison = self
                .compare
                .snapshot(&shown, Some((&snapshot.project, snapshot.timeline, frame)));
            if let Some(selection) = &self.window.selection {
                let coverage = self.compare.coverage(&shown);
                snapshot.overlay = selection.overlay(&snapshot, coverage);
                if snapshot.overlay.is_some() {
                    snapshot.selection = Some(Arc::clone(selection));
                }
            }
            Arc::new(snapshot)
        });
        if let (Some((instance, _)), Some(viewed)) = (attached, viewed) {
            self.submit_frame(instance, &viewed, &gpu, snapshot.as_ref())?;
        }
        let measured = if crate::compare::SCOPES_SEE_COMPARISON {
            &shown
        } else {
            &target
        };
        if !self.aside {
            self.scopes.shown(
                Monitor::Timeline,
                Shown::Target {
                    gpu,
                    target: Arc::clone(measured),
                    frame,
                },
            );
        }
        self.current = Some(shown);
        Ok(snapshot)
    }

    /// Register before submitting, releasing each short guard before taking another. A view
    /// replacement during composition/submission cannot receive another instance's tag.
    fn submit_frame(
        &self,
        instance: u64,
        target: &Target,
        gpu: &Arc<columbina_compose::Gpu>,
        snapshot: Option<&Arc<FrameSnapshot>>,
    ) -> Result<(), Fault> {
        let presentation = {
            let slot = lock(&self.view);
            let Some(view) = slot.as_ref().filter(|view| view.instance == instance) else {
                return Ok(());
            };
            view.presentation()
        };
        self.frames
            .observe(Monitor::Timeline, instance, presentation);
        let tag = snapshot.and_then(|snapshot| {
            self.frames
                .register(Monitor::Timeline, instance, Arc::clone(snapshot))
        });
        let result = {
            let slot = lock(&self.view);
            slot.as_ref()
                .filter(|view| view.instance == instance)
                .map(|view| match tag {
                    Some(tag) => view.present_texture_overlay(
                        Arc::clone(target.texture()),
                        Arc::clone(gpu),
                        tag,
                        snapshot.and_then(|s| s.overlay.clone()),
                    ),
                    None => view.present_texture(Arc::clone(target.texture()), Arc::clone(gpu)),
                })
        };
        if !matches!(result, Some(Ok(()))) {
            if let Some(tag) = tag {
                self.frames.discard(Monitor::Timeline, instance, tag);
            }
        }
        if let Some(Err(error)) = result {
            if error == columbina_view::ViewError::Gone {
                self.frames.gone(Monitor::Timeline, instance);
            }
            return Err(drawing(&error.to_string()));
        }
        Ok(())
    }

    fn error(message: &str) {
        log(
            "transport_error",
            &json!({"monitor":"timeline", "message":message}),
        );
    }
}

/// What of media the picture of a timeline depends on: its file, what it is and holds, and its
/// proxy.
fn seen(media: &Media) -> (&str, &Identity, &Facts, Option<&Proxy>) {
    (
        &media.path,
        &media.identity,
        &media.facts,
        media.proxy.as_ref(),
    )
}

/// What of media the sound depends on, which is always read from the file itself.
fn heard(media: &Media) -> (&str, &Identity, &Facts) {
    (&media.path, &media.identity, &media.facts)
}

fn media_of<'p>(project: &'p Project, id: &str) -> Option<&'p Media> {
    project.media.iter().find(|media| media.id == id)
}

/// How many tracks of picture of a timeline use each media: how many decoders of it the
/// track readers may want at once.
fn demand(project: &Project, timeline: &columbina_model::Timeline) -> BTreeMap<String, usize> {
    let mut demand = BTreeMap::new();
    for track in &timeline.tracks {
        if track.kind != TrackKind::Video || !track.enabled {
            continue;
        }
        let used: BTreeSet<&str> = track
            .clips
            .iter()
            .filter(|clip| clip.enabled)
            .filter_map(|clip| clip.media().map(String::as_str))
            .filter(|media| media_of(project, media).is_some())
            .collect();
        for media in used {
            *demand.entry(media.to_owned()).or_insert(0) += 1;
        }
    }
    demand
}

/// Whether a timeline shows the same pictures in `after` as it did in `before`: the same
/// size and rate, tracks of picture (but for the grades of their clips), and media (with its
/// proxies) under them. Subtitles, sound and grades are not pictures that are read ahead.
fn same_picture(
    before: &Project,
    was: &columbina_model::Timeline,
    after: &Project,
    now: &columbina_model::Timeline,
) -> bool {
    let video = |timeline: &columbina_model::Timeline| {
        timeline
            .tracks
            .iter()
            .filter(|track| track.kind == TrackKind::Video)
            .map(|track| {
                let mut track = track.clone();
                for clip in &mut track.clips {
                    clip.grade = Grade::default();
                    clip.group = None;
                }
                track
            })
            .collect::<Vec<_>>()
    };
    (was.width, was.height, was.rate) == (now.width, now.height, now.rate)
        && video(was) == video(now)
        && demand(after, now)
            .keys()
            .all(|id| media_of(before, id).map(seen) == media_of(after, id).map(seen))
}

/// Whether a timeline is graded the same in `after` as in `before`: its grade, and the grades
/// of each clip of its tracks of picture and of the group that the clip is in.
fn same_grades(
    before: &Project,
    was: &columbina_model::Timeline,
    after: &Project,
    now: &columbina_model::Timeline,
) -> bool {
    fn grades<'p>(project: &'p Project, timeline: &'p columbina_model::Timeline) -> GradesOf<'p> {
        let clips = timeline
            .tracks
            .iter()
            .filter(|track| track.kind == TrackKind::Video)
            .flat_map(|track| &track.clips)
            .map(|clip| {
                let group = clip
                    .group
                    .as_ref()
                    .and_then(|id| project.groups.iter().find(|group| group.id == *id))
                    .map(|group| (&group.pre, &group.post));
                (clip.id.as_str(), &clip.grade, group)
            })
            .collect();
        (&timeline.grade, clips)
    }
    grades(before, was) == grades(after, now)
}

/// The grade of a timeline, and of each clip of its tracks of picture with the pre-clip and
/// post-clip grades of its group.
type GradesOf<'p> = (
    &'p Grade,
    Vec<(&'p str, &'p Grade, Option<(&'p Grade, &'p Grade)>)>,
);

/// Whether a timeline sounds the same in `after` as in `before` from sample `from` to its end:
/// the same pieces of the same clips, of the same media, at the same rates and length, mixed the
/// same ([`same_clips`], [`same_mixing`]).
fn same_sound(
    before: &Project,
    was: &columbina_model::Timeline,
    after: &Project,
    now: &columbina_model::Timeline,
    from: i64,
) -> bool {
    same_mixing(was, now) && same_clips(before, was, after, now, from)
}

/// Whether the clips of a timeline sound the same in `after` as in `before` from sample `from`
/// to its end: the same pieces of the same clips, of the same media, at the same rates and
/// length, through the same equalizers, effects and pitch. Only then may a graph of the old
/// project play on, its settings changed (T5, `sound.rs`).
pub(super) fn same_clips(
    before: &Project,
    was: &columbina_model::Timeline,
    after: &Project,
    now: &columbina_model::Timeline,
    from: i64,
) -> bool {
    if (was.rate, was.sample_rate) != (now.rate, now.sample_rate)
        || resolve::duration(was) != resolve::duration(now)
        || !same_chains(was, now)
    {
        return false;
    }
    let end = resolve::samples_at(resolve::duration(now), now.rate, now.sample_rate);
    // Every track that is on: a muted one is not heard, but it may drive a sidechain.
    let pieces = resolve::sources_between(now, after, from, end);
    let channels = |timeline: &columbina_model::Timeline, clip: &str| {
        timeline
            .tracks
            .iter()
            .flat_map(|track| &track.clips)
            .find(|each| each.id == clip)
            .map(|each| each.channels.clone())
    };
    pieces == resolve::sources_between(was, before, from, end)
        && pieces.iter().all(|piece| {
            media_of(before, &piece.media).map(heard) == media_of(after, &piece.media).map(heard)
                && channels(was, &piece.clip) == channels(now, &piece.clip)
        })
}

/// Whether the graph of the sound (T4b) mixes the clips of `was` as those of `now`: the same
/// tracks of sound with the same strips, formats, mutes, solos and lanes, the same buses and
/// VCAs, and the same bus monitored. While the sound plays, the graph takes a change of them as
/// it plays, or another graph is swapped in (T5, `sound.rs`); this tells whether there is any.
fn same_mixing(was: &columbina_model::Timeline, now: &columbina_model::Timeline) -> bool {
    fn mixing(
        timeline: &columbina_model::Timeline,
    ) -> impl Iterator<Item = (&str, bool, &columbina_model::TrackAudio)> {
        timeline
            .tracks
            .iter()
            .filter(|track| track.kind == TrackKind::Audio)
            .map(|track| (track.id.as_str(), track.enabled, &track.audio))
    }
    mixing(was).eq(mixing(now))
        && was.buses == now.buses
        && was.vcas == now.vcas
        && was.audio.monitor == now.audio.monitor
}

/// Whether the chain of each clip of sound does the same besides its gain, pan and fades: its
/// equalizer, effects and pitch.
fn same_chains(was: &columbina_model::Timeline, now: &columbina_model::Timeline) -> bool {
    fn chains(
        timeline: &columbina_model::Timeline,
    ) -> impl Iterator<
        Item = (
            &str,
            &columbina_model::Equalizer,
            &[columbina_model::Insert],
            bool,
            &columbina_model::ClipPitch,
        ),
    > {
        timeline
            .tracks
            .iter()
            .filter(|track| track.kind == TrackKind::Audio)
            .flat_map(|track| &track.clips)
            .map(|clip| {
                (
                    clip.id.as_str(),
                    &clip.eq,
                    clip.effects.as_slice(),
                    clip.pitch_correction,
                    &clip.pitch,
                )
            })
    }
    chains(was).eq(chains(now))
}

fn drawing(message: &str) -> Fault {
    Fault::new(rpc::TRANSPORT_FAILED, message).with_data(json!({"reason":"draw"}))
}

#[cfg(test)]
mod colour;
#[cfg(test)]
mod grades;

#[cfg(test)]
mod tests {
    use super::*;
    use columbina_compose::Transform;
    use columbina_model::Rgba;

    #[test]
    fn an_older_selection_or_clear_cannot_replace_a_newer_worker_selection() {
        let mut worker = worker();
        worker.apply(Command::Window(crate::window::Update {
            sequence: 10,
            selection: None,
        }));
        let selection = Arc::new(crate::window::Selection {
            monitor: Monitor::Timeline,
            revision: 0,
            timeline: "t1".into(),
            target: columbina_model::grade::Target::Timeline {
                timeline: "t1".into(),
            },
            clip: None,
            node: "n1".into(),
            selected: None,
            visible: true,
            outline: true,
            tool: crate::window::Tool::Window,
            binding: 9,
            chosen: None,
            band: None,
        });
        worker.apply(Command::Window(crate::window::Update {
            sequence: 9,
            selection: Some(selection.clone()),
        }));
        assert!(worker.window.selection.is_none());
        worker.apply(Command::Window(crate::window::Update {
            sequence: 11,
            selection: Some(selection),
        }));
        worker.apply(Command::Window(crate::window::Update {
            sequence: 10,
            selection: None,
        }));
        assert_eq!(worker.window.selection.as_ref().unwrap().binding, 9);
        assert_eq!(worker.window.sequence, 11);
    }

    fn worker() -> Worker {
        let (_, inbox) = mpsc::channel();
        Worker {
            inbox,
            shared: Arc::default(),
            notify: Arc::new(|_| {}),
            view: Arc::default(),
            frames: Registry::default(),
            revision: 0,
            window: crate::window::Update::default(),
            frozen: 0,
            graphics: Graphics::new(),
            sources: Sources::new(Hardware::Off, Arc::new(|_, _| {})),
            grading: Grading::new(LutPlaces::with_library(std::path::Path::new("."), vec![])),
            scopes: Scopes::new(true, Graphics::new()),
            output: None,
            compare: Comparison::new(),
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
            aside: false,
            clock: None,
            sound: SoundSlot::new(crate::audio::Kind::Null),
            playback: super::super::sound::Playback::new(),
            automation: super::super::record::Automation::default(),
            joining: None,
            sound_late: false,
            mixed_from: None,
            scrub_until: None,
            speed_change: None,
            heard: Cell::new(0),
            fine: None,
            epoch: Instant::now(),
            settle: None,
            told: None,
        }
    }

    fn filled(rgba: [u8; 4]) -> Scene {
        Scene {
            width: 64,
            height: 36,
            layers: vec![SceneLayer {
                source: LayerSource::color(Rgba(rgba)),
                transform: Transform::NONE,
                opacity: 1.0,
                color: LayerColor::default(),
            }],
            ..Scene::default()
        }
    }

    fn proxied(mut project: Project) -> Project {
        project.media[0].proxy = Some(Proxy {
            path: "C:/project/cache/proxies/m1-000000000000.mov".into(),
            width: 960,
            height: 540,
            codec: "prores".into(),
            digest_of_source: "0".repeat(64),
        });
        project
    }

    #[test]
    fn a_change_that_the_timeline_does_not_show_keeps_what_was_read_ahead() {
        let mut worker = worker();
        worker.status.timeline = Some("t1".into());
        let project = Project::sample();
        assert!(worker.replace(0, Arc::new(project.clone())));
        let read_ahead = worker.generation;
        // Another timeline and the name of media change nothing that t1 shows.
        let mut other = project.clone();
        let mut second = other.timelines[0].clone();
        second.id = "t2".into();
        other.timelines.push(second);
        other.media[0].name = "Renamed".into();
        assert!(!worker.replace(0, Arc::new(other.clone())));
        assert_eq!(worker.generation, read_ahead);
        // What is heard only keeps the pictures that were read.
        let mut louder = other.clone();
        louder.timelines[0].tracks[1].clips[0].gain_db = (-3.0).into();
        assert!(worker.replace(0, Arc::new(louder.clone())));
        assert_eq!(worker.generation, read_ahead);
        // A proxy of the media under the picture changes what is read.
        assert!(worker.replace(0, Arc::new(proxied(louder.clone()))));
        assert_eq!(worker.generation, read_ahead + 1);
        let mut moved = proxied(louder);
        moved.timelines[0].tracks[0].clips[0].transform.x = 0.25;
        assert!(worker.replace(0, Arc::new(moved)));
        assert_eq!(worker.generation, read_ahead + 2);
    }

    #[test]
    fn sound_starts_again_for_a_change_of_how_it_is_mixed() {
        let before = Project::sample();
        let check = |after: &Project, from: i64| {
            same_sound(
                &before,
                &before.timelines[0],
                after,
                &after.timelines[0],
                from,
            )
        };
        // The graph (T4b) mixes by the strips, the buses and the VCAs: a fader moved, a bus
        // added or the monitor changed is heard, until T5 hears them without starting again.
        let mut moved = before.clone();
        moved.timelines[0].tracks[1].audio.strip.fader_db = -6.0;
        assert!(!check(&moved, 0));
        let mut bus = before.clone();
        bus.timelines[0].buses[0].strip.fader_db = -6.0;
        assert!(!check(&bus, 0));
        let mut monitor = before.clone();
        monitor.timelines[0].audio.monitor = None;
        assert!(!check(&monitor, 0));
        // The equalizer and the effects of a clip are heard too.
        let mut equalized = before.clone();
        equalized.timelines[0].tracks[1].clips[0].eq.on = true;
        assert!(!check(&equalized, 0));
        let mut affected = before.clone();
        affected.timelines[0].tracks[1].clips[0]
            .effects
            .push(columbina_model::Insert {
                id: String::new(),
                on: true,
                effect: columbina_model::Effect::Gain(columbina_model::audio::fx::GainFx {
                    gain_db: 3.0,
                }),
            });
        assert!(!check(&affected, 0));
        // A muted track is not heard, but it may drive a sidechain: its clips count.
        let mut muted = before.clone();
        muted.timelines[0].tracks[1].audio.mute = true;
        let mut quieter_muted = muted.clone();
        quieter_muted.timelines[0].tracks[1].clips[0].gain_db = (-6.0).into();
        assert!(!same_sound(
            &muted,
            &muted.timelines[0],
            &quieter_muted,
            &quieter_muted.timelines[0],
            0,
        ));
    }

    #[test]
    fn a_change_of_what_is_heard_while_it_plays_never_starts_the_sound_again() {
        let mut worker = worker();
        worker.status.timeline = Some("t1".into());
        let project = Project::sample();
        worker.replace(0, Arc::new(project.clone()));
        let rate = project.timelines[0].rate;
        worker.clock = Some(Clock::starting(0, 1, rate, worker.now()));
        worker.status.playing = true;
        worker.restart_sound();
        assert_eq!(worker.status.sound, Sound::On);
        let session = worker.sound.working().unwrap().session();
        let mut changed = project.clone();
        let edits: [fn(&mut Project); 5] = [
            |p| p.timelines[0].tracks[1].audio.strip.fader_db = -6.0,
            |p| p.timelines[0].tracks[1].clips[0].gain_db = (-3.0).into(),
            |p| p.timelines[0].tracks[1].audio.strip.eq.on = true,
            |p| p.timelines[0].tracks[1].clips[0].fade_in = 10,
            |p| p.timelines[0].tracks[1].audio.mute = true,
        ];
        for edit in edits {
            edit(&mut changed);
            worker.replace(0, Arc::new(changed.clone()));
            worker.check_sound();
            assert_eq!(worker.status.sound, Sound::On);
            assert_eq!(worker.sound.working().unwrap().session(), session);
        }
        // Only another rate of the sound of the timeline starts it again.
        changed.timelines[0].sample_rate = 44_100;
        worker.replace(0, Arc::new(changed));
        assert_eq!(worker.status.sound, Sound::On);
        assert_eq!(worker.sound.working().unwrap().session(), session + 1);
        worker.pause();
        assert!(!worker.playback.playing());
    }

    #[test]
    fn a_pass_recorded_at_speed_1_ends_where_the_speed_changes() {
        use super::super::record::Order;
        use columbina_audio::graph::Owner;
        let mut worker = worker();
        worker.status.timeline = Some("t1".into());
        let mut project = Project::sample();
        project.timelines[0].audio.record.touch = columbina_model::audio::Touch::Latch;
        worker.replace(0, Arc::new(project.clone()));
        let written: Arc<Mutex<Vec<columbina_model::Edit>>> = Arc::default();
        worker.automate(Order::Hook(Arc::new({
            let written = Arc::clone(&written);
            move |edit, _| lock(&written).push(edit)
        })));
        let (reply, _answer) = oneshot::channel();
        worker.automate(Order::Record {
            timeline: "t1".into(),
            on: true,
            glide: None,
            by: crate::session::Kind::Desktop,
            reply,
        });
        let rate = project.timelines[0].rate;
        worker.clock = Some(Clock::starting(0, 1, rate, worker.now()));
        worker.status.playing = true;
        worker.restart_sound();
        assert_eq!(worker.status.sound, Sound::On);
        let deadline = Instant::now() + Duration::from_secs(60);
        while worker.heard_sample().unwrap_or(0) < 4_800 {
            worker.check_sound();
            assert!(Instant::now() < deadline, "nothing heard");
            std::thread::sleep(Duration::from_millis(5));
        }
        worker.automate(Order::Touch {
            timeline: "t1".into(),
            owner: Owner::Track("A1".into()),
            path: "fader_db".into(),
            value: -6.0,
            touching: true,
        });
        std::thread::sleep(Duration::from_millis(100));
        let before = worker.heard_sample().unwrap();
        worker.action(Action::Play { speed: 2 });
        let after = before + 48_000 / 10;
        // At speed 2 a touch is heard and not recorded.
        worker.automate(Order::Touch {
            timeline: "t1".into(),
            owner: Owner::Track("A1".into()),
            path: "fader_db".into(),
            value: -12.0,
            touching: true,
        });
        let edits = lock(&written).clone();
        let [columbina_model::Edit::RecordPass { lanes, .. }] = &edits[..] else {
            panic!("one pass written at the change: {edits:?}");
        };
        let to = lanes[0].runs[0].to.ticks;
        assert!(to >= before && to < after, "{before} <= {to} < {after}");
        // At speed 2 nothing more is recorded; the pause writes nothing more.
        worker.pause();
        assert_eq!(lock(&written).len(), 1);
    }

    /// How long the next feed that [`stalls_once`] opens waits first.
    static STALL_ONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn stalls_once(
        project: &Arc<Project>,
        timeline: usize,
        sources: &[columbina_audio::feed::ClipSource],
        pool: &Arc<super::super::mix::Readers>,
    ) -> Box<dyn columbina_audio::feed::ClipFeed + Send> {
        let wait = STALL_ONCE.swap(0, std::sync::atomic::Ordering::AcqRel);
        std::thread::sleep(Duration::from_millis(wait));
        super::super::sound::media_feed(project, timeline, sources, pool)
    }

    #[test]
    fn a_session_slow_to_start_lets_the_picture_go_on_and_its_sound_joins_it_later() {
        let mut worker = worker();
        worker.playback = super::super::sound::Playback::with_opener(stalls_once);
        worker.status.timeline = Some("t1".into());
        let project = Project::sample();
        worker.replace(0, Arc::new(project.clone()));
        let rate = project.timelines[0].rate;
        worker.clock = Some(Clock::starting(0, 1, rate, worker.now()));
        worker.status.playing = true;
        STALL_ONCE.store(3_000, std::sync::atomic::Ordering::Release);
        let started = Instant::now();
        worker.restart_sound();
        let session = worker.sound.working().unwrap().session();
        let frame = |worker: &Worker| worker.clock.unwrap().frame(worker.clock_now());
        // The picture waits for the sound for two seconds, then goes on by the clock of the
        // system.
        while worker.status.sound == Sound::On {
            assert!(started.elapsed() < Duration::from_secs(4));
            worker.check_sound();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(worker.status.sound, Sound::Unavailable);
        assert!(started.elapsed() >= super::super::sound::SLOW);
        let late = frame(&worker);
        std::thread::sleep(Duration::from_millis(400));
        assert!(frame(&worker) >= late + 9, "{late} then {}", frame(&worker));
        // Once the graph is ready, a session starts where the picture will be, and its sound,
        // once heard, is the clock again.
        while worker.status.sound != Sound::On {
            assert!(started.elapsed() < Duration::from_secs(20));
            worker.check_sound();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(worker.sound.working().unwrap().session() > session);
        assert!(worker.joining.is_none());
        // Where the picture was going by the clock of the system since it waited, and the
        // half second ahead that the sound was started at, at the most.
        let joined = frame(&worker);
        let since = i64::try_from(
            started
                .elapsed()
                .saturating_sub(super::super::sound::SLOW)
                .as_millis(),
        )
        .unwrap()
            / 40;
        assert!(
            (since - 3..=since + 15).contains(&joined),
            "the picture at {joined}, frame {since} by the clock since it waited"
        );
        worker.pause();
    }

    #[test]
    fn sound_plays_on_through_a_change_that_it_does_not_hear() {
        let before = Project::sample();
        let check = |after: &Project, from: i64| {
            same_sound(
                &before,
                &before.timelines[0],
                after,
                &after.timelines[0],
                from,
            )
        };
        assert!(check(&before, 0));
        let mut seen = before.clone();
        seen.timelines[0].tracks[0].clips[0].transform.scale = 0.5;
        assert!(check(&seen, 0), "only the picture changed");
        assert!(
            check(&proxied(before.clone()), 0),
            "sound is never read from a proxy"
        );
        let mut louder = before.clone();
        louder.timelines[0].tracks[1].clips[0].gain_db = (-3.0).into();
        assert!(!check(&louder, 0));
        let mut faded = before.clone();
        faded.timelines[0].tracks[1].clips[0].fade_in = 10;
        assert!(!check(&faded, 0));
        // Its channels mapped otherwise, the clip is read otherwise.
        let mut mapped = before.clone();
        mapped.timelines[0].tracks[1].clips[0].channels = Some(vec![
            columbina_model::ChannelMap {
                source: columbina_model::ChannelSources(vec![]),
                enabled: true,
                mute: false,
                level_db: 0.0,
            };
            2
        ]);
        assert!(!check(&mapped, 0));
        // What was heard before the playhead does not matter: a clip of sound that has ended.
        let mut earlier = before.clone();
        let sound = &mut earlier.timelines[0].tracks[1].clips;
        sound[0].length = 40;
        let mut later = sound[0].clone();
        (later.id, later.start) = ("c9".into(), 40);
        sound.push(later);
        let mut quieter = earlier.clone();
        quieter.timelines[0].tracks[1].clips[0].gain_db = (-6.0).into();
        let check_from = |from: i64| {
            same_sound(
                &earlier,
                &earlier.timelines[0],
                &quieter,
                &quieter.timelines[0],
                from,
            )
        };
        assert!(!check_from(0));
        assert!(check_from(48000 * 2));
        // A timeline that ends elsewhere would end the old mixer elsewhere too.
        let mut longer = before.clone();
        longer.timelines[0].tracks[0].clips[0].length = 120;
        assert!(!check(&longer, 48000));
        assert!(same_picture(
            &before,
            &before.timelines[0],
            &louder,
            &louder.timelines[0]
        ));
        assert!(!same_picture(
            &before,
            &before.timelines[0],
            &seen,
            &seen.timelines[0]
        ));
    }

    #[test]
    fn a_picture_that_a_view_still_shows_is_never_drawn_over() {
        if columbina_compose::gpu_for_tests().is_none() {
            return;
        }
        let mut worker = worker();
        worker.present(&filled([255, 0, 0, 255]), 0, None).unwrap();
        let shown = Arc::downgrade(worker.current.as_ref().unwrap());
        // A view holds the texture that it shows, as present_texture keeps it, until it shows
        // another; the monitor no longer holds the target once it draws the next frame.
        let held = Arc::clone(worker.current.as_ref().unwrap().texture());
        worker.present(&filled([0, 255, 0, 255]), 0, None).unwrap();
        worker.present(&filled([0, 0, 255, 255]), 0, None).unwrap();
        let red = shown.upgrade().expect("the monitor keeps its targets");
        assert_eq!(red.read_rgba().unwrap()[..4], [255, 0, 0, 255]);
        assert_eq!(worker.targets.len(), 3);
        // Once the view lets it go, the target is drawn on again.
        drop((held, red));
        worker
            .present(&filled([255, 255, 255, 255]), 0, None)
            .unwrap();
        assert_eq!(worker.targets.len(), 3);
    }

    /// Waits for `until`, taking what the sound did meanwhile, for at most 30 s.
    fn sound_until(worker: &mut Worker, what: &str, mut until: impl FnMut(&Worker) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !until(worker) {
            assert!(Instant::now() < deadline, "{what}");
            worker.check_sound();
            worker.clock_due();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn at_another_speed_the_sound_plays_is_the_clock_and_changes_its_speed_in_the_same_session() {
        let mut worker = worker();
        worker.status.timeline = Some("t1".into());
        let project = Project::sample();
        worker.replace(0, Arc::new(project.clone()));
        let rate = project.timelines[0].rate;
        worker.clock = Some(Clock::starting(50, -2, rate, worker.now()));
        worker.status.playing = true;
        worker.status.speed = -2;
        worker.restart_sound();
        assert_eq!(worker.status.sound, Sound::On, "sound at −2 (T18)");
        assert_eq!(worker.clock.unwrap().speed(), -2);
        let session = worker.sound.working().unwrap().session();
        sound_until(&mut worker, "its first sound", |worker| {
            worker
                .sound
                .working()
                .and_then(|output| output.played(Instant::now()))
                .is_some()
        });
        // Another speed while it plays: the sound changes in the same session, and the clock of
        // the picture with it where it does.
        worker.action(Action::Play { speed: 4 });
        assert_eq!(worker.status.speed, 4);
        assert_eq!(worker.status.sound, Sound::On);
        assert_eq!(
            worker.clock.unwrap().speed(),
            -2,
            "the picture keeps its speed until the sound changes it"
        );
        sound_until(&mut worker, "the clock at 4", |worker| {
            worker.clock.is_some_and(|clock| clock.speed() == 4)
        });
        assert_eq!(worker.sound.working().unwrap().session(), session);
        assert_eq!(worker.playback.counts().1, 1, "one graph swapped in");
        worker.pause();
        assert!(!worker.playback.playing());
    }

    #[test]
    fn scrubbing_plays_grains_in_one_session_until_audio_scrubbing_is_turned_off() {
        let mut worker = worker();
        worker.status.timeline = Some("t1".into());
        let project = Project::sample();
        worker.replace(0, Arc::new(project));
        assert!(worker.status.scrub_audio, "on at first (p. 731)");
        worker.action(Action::Seek {
            frame: 10,
            scrubbing: true,
        });
        assert!(worker.playback.scrubbing());
        let session = worker.sound.working().unwrap().session();
        worker.action(Action::Seek {
            frame: 20,
            scrubbing: true,
        });
        // The seek that ends the drag lets the last grain sound on.
        worker.action(Action::Seek {
            frame: 20,
            scrubbing: false,
        });
        assert!(worker.playback.scrubbing());
        assert_eq!(worker.sound.working().unwrap().session(), session);
        assert!(worker.wait().is_some_and(|wait| wait <= SCRUB_IDLE));
        // Off: the grain fades out and the session ends after, and none plays.
        worker.action(Action::ScrubAudio { on: false });
        assert!(!worker.status.scrub_audio);
        std::thread::sleep(SCRUB_FADE_OUT);
        worker.tick();
        assert!(!worker.playback.playing());
        worker.action(Action::Seek {
            frame: 30,
            scrubbing: true,
        });
        assert!(!worker.playback.playing());
        // On again: a grain, which ends once no other is asked for within a second.
        worker.action(Action::ScrubAudio { on: true });
        worker.action(Action::Seek {
            frame: 40,
            scrubbing: true,
        });
        assert!(worker.playback.scrubbing());
        worker.scrub_until = Some(Instant::now());
        worker.tick();
        assert!(!worker.playback.playing());
    }

    #[test]
    fn the_picture_turns_back_at_the_instant_the_sound_does_never_ahead_of_it() {
        // Forward at 2, then backward at 2: the place of the picture rises to where the sound
        // turns, and never past it (T18's review: the clock took the new speed for the instants
        // before the change and jumped ahead, then back).
        let mut worker = worker();
        worker.status.timeline = Some("t1".into());
        let project = Project::sample();
        worker.replace(0, Arc::new(project.clone()));
        let rate = project.timelines[0].rate;
        worker.clock = Some(Clock::starting(20, 2, rate, worker.now()));
        worker.status.playing = true;
        worker.status.speed = 2;
        worker.restart_sound();
        sound_until(&mut worker, "its first sound", |worker| {
            worker
                .sound
                .working()
                .and_then(|output| output.played(Instant::now()))
                .is_some()
        });
        let forward = worker.clock.unwrap();
        worker.action(Action::Play { speed: -2 });
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut highest = i128::MIN;
        let mut turned = None;
        loop {
            assert!(Instant::now() < deadline, "the turn");
            worker.check_sound();
            let clock = worker.clock_due().unwrap();
            let now = worker.clock_now();
            highest = highest.max(clock.place(now));
            if clock.speed() == -2 && turned.is_none() {
                turned = Some(now);
            }
            if turned.is_some_and(|at| now > at + 100_000_000) {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let (since, speed, pending) = worker.speed_change.unwrap();
        assert_eq!((speed, pending), (-2, false));
        let turn = forward.place(since);
        // A millisecond of the place at 2, for the instants between two looks.
        let slack = 2 * 1_000_000 * i128::from(rate.num);
        assert!(highest <= turn + slack, "{highest} > {turn}");
        assert!(worker.clock.unwrap().place(worker.clock_now()) < turn);
        worker.pause();
    }

    #[test]
    fn a_grain_paused_or_turned_off_fades_out_and_its_session_ends_after() {
        let mut worker = worker();
        worker.status.timeline = Some("t1".into());
        worker.replace(0, Arc::new(Project::sample()));
        for stop in [Action::Pause, Action::ScrubAudio { on: false }] {
            worker.action(Action::ScrubAudio { on: true });
            worker.action(Action::Seek {
                frame: 10,
                scrubbing: true,
            });
            assert!(worker.playback.scrubbing());
            worker.action(stop);
            // Fading: the session plays on a little, silent, and ends.
            assert!(worker.playback.scrubbing(), "{stop:?}");
            assert!(worker
                .scrub_until
                .is_some_and(|until| until <= Instant::now() + SCRUB_FADE_OUT));
            std::thread::sleep(SCRUB_FADE_OUT);
            worker.tick();
            assert!(!worker.playback.playing(), "{stop:?}");
        }
    }
}
