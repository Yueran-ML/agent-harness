//! The sound of the timeline as it plays (plan T5): the graph of `columbina-audio` built, started
//! and read ahead off the threads of the sound, mixed on a thread of its own, and changed as the
//! project changes, without the sound starting again.
//!
//! # The threads
//!
//! - **The thread that builds** (one for each timeline monitor, for as long as it lives; T12:
//!   `RNNoise` plans its transforms once on each thread that uses it): builds a graph of the
//!   project, starts it (its preroll processed, `Graph::start`), reads the first of its clips
//!   ahead, and hands it to the thread of the mix; it drops what that thread lets go of.
//! - **The thread of the mix** (`columbina_audio::play::thread`, one for each timeline monitor,
//!   for as long as it lives, `RNNoise` warmed on it): mixes ahead of the device.
//! - **The readers ahead** (one for each graph that plays, `columbina_audio::play::fetcher`).
//! - **The device's thread** (`crate::audio`).
//!
//! # What a change of the project does ([`Playback::changed`])
//!
//! - A change that is heard nowhere from where the mix is on: nothing.
//! - A change of settings that the graph has controls of (a fader, a pan, a send's level, the
//!   parameters of Dynamics, the equalizer and the inserts, a mute, a solo; `graph::retune`):
//!   the values are sent to the thread of the mix, which ramps them over 10 ms from its next
//!   block.
//! - Any other change of what is heard (an effect added or turned on, the routing, a lane, a clip
//!   added, moved or trimmed, its gain or its fades): another graph is built in the background
//!   from the project as it is then and swapped in at a sample ahead of what is mixed, crossfaded
//!   over 10 ms (`columbina_audio::play::engine`). Changes that come while it is built are sent
//!   to the graph that plays where they are settings, and another graph follows once the first
//!   is swapped in.
//! - A change of the rates of the timeline: the sound starts again (the clock of the picture is
//!   counted at the rate of the device and the timeline).
//!
//! Playing starts, and starts again on a seek or after the device failed: a new session, whose
//! graph is built in the background while the device plays silence, which is not counted, so that
//! the picture waits for its sound (as phases 1 and 2 did).
//!
//! # Speeds and scrubbing (T18)
//!
//! - A session plays at any whole speed, forward or backward: its graphs play the timeline through
//!   a [`Warp`] (`Graph::set_warp`), their clips resampled to the speed, so that the pitch follows
//!   it. A change of speed while it plays is another graph, built at the new speed from where the
//!   one that plays will be, swapped in and crossfaded as a change of the project is, in the same
//!   session ([`Playback::set_speed`]): the device's clock goes on, and the picture changes its
//!   speed at the sample where the sound does ([`Event::Speed`]).
//! - Scrubbing ([`Playback::scrub`]): a session of grains, each 60 ms of the timeline from where
//!   the playhead was dragged, faded 5 ms at either end (`play::engine::Grain`); each next grain
//!   is a graph swapped in, which takes the place of one that still sounds by a crossfade of its
//!   fade, and grains asked for while one is built are taken together (only the last is built).

use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use columbina_audio::constants::{
    samples, GRAPH_SWAP_CROSSFADE, MIX_BLOCK, SCRUB_FADE, SCRUB_GRAIN,
};
use columbina_audio::feed::{ClipFeed, ClipSource};
use columbina_audio::graph::{
    retune, without_clip_levels, Controls, Graph, Options, Purpose, Warp,
};
use columbina_audio::play::device;
use columbina_audio::play::engine::{Command, Engine, Grain, Retired, Run, Session, MOST_BLOCK};
use columbina_audio::play::fetcher::{ring_feed, Fetcher};
use columbina_audio::play::thread::MixThread;
use columbina_audio::play::Counters;
use columbina_audio::rt::{messages, MessageConsumer, MessageProducer, SampleProducer};
use columbina_audio::stream::Resampler;
use columbina_media::AudioFormat;
use columbina_model::Project;
use serde_json::json;

use super::mix::{end_of, Feed, Readers};
use crate::audio::Output;
use crate::log::log;

/// How far ahead of what is mixed a new graph is swapped in, at the least; further where the last
/// graph took longer to build (half as long again, and this).
const SWAP_AHEAD: f64 = 0.150;

/// The furthest ahead of what is mixed a new graph is swapped in.
const LONGEST_AHEAD: f64 = 5.0;

/// The most preroll of a graph swapped in as the sound plays (T5's review; a graph that starts
/// playing prerolls as an export does, up to `PREROLL_MAX`): what settles longer (a compressor's
/// release of seconds) is brought in by the crossfade not quite settled.
const SWAP_PREROLL: f64 = 1.0;

/// How long the thread that builds reads the clips ahead itself before it hands a graph over;
/// the readers ahead read the rest as it plays.
const PRIME: Duration = Duration::from_millis(100);

/// How long a graph to swap in is caught up, at the most, where the mix went on past where it
/// was to start while it was built: past that, it is sent all the same and comes too late, and is
/// built again further ahead.
const CATCH_UP: Duration = Duration::from_millis(200);

/// How long the first graph of a session may take to be handed over before the picture goes on
/// by the clock of the system (a drive that sleeps, a decoder that hangs): as long as a device
/// may let no sound be heard ([`crate::audio::NO_SOUND`]).
pub(crate) const SLOW: Duration = crate::audio::NO_SOUND;

/// How often the thread that builds drops what the thread of the mix let go of, when it has
/// nothing to build.
const BUILDER_NAP: Duration = Duration::from_millis(20);

/// How far ahead of what is mixed a grain of scrubbing is swapped in (T18, "我们定"): what it
/// takes to build its graph and read the start of its clips is waited for before, and where the
/// mix went on past it meanwhile, the graph is caught up to a little after what is mixed.
const GRAIN_AHEAD: f64 = 0.020;

/// The furthest ahead of what is mixed a grain of scrubbing is swapped in, however long the last
/// took to build (T18's review).
const GRAIN_LONGEST_AHEAD: f64 = 0.5;

/// How many times a grain is built, each further ahead, before it is caught up to what is mixed
/// (and so starts after its frame) rather than built again (T18's review).
const GRAIN_ATTEMPTS: usize = 3;

/// The most preroll of the graph of a grain of scrubbing (T18, "我们定"): a grain is heard at
/// once, its effects settled for this long (a compressor's release of seconds is not quite
/// settled in it).
const GRAIN_PREROLL: f64 = 0.100;

/// How long the thread that builds reads the clips of a grain ahead itself before it hands the
/// graph over (T18): the start of the grain; its readers ahead read the rest.
const GRAIN_PRIME: Duration = Duration::from_millis(30);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What opens the feed of the clips `sources` of timeline `timeline` of a project, its readers
/// taken from and given back to a pool: the media readers of the server, or a test's own.
pub(crate) type Opener =
    fn(&Arc<Project>, usize, &[ClipSource], &Arc<Readers>) -> Box<dyn ClipFeed + Send>;

/// The media readers of the server, a clip that cannot be read silent and told of once.
pub(crate) fn media_feed(
    project: &Arc<Project>,
    timeline: usize,
    sources: &[ClipSource],
    pool: &Arc<Readers>,
) -> Box<dyn ClipFeed + Send> {
    Box::new(Feed::new(Arc::clone(project), timeline, sources, true).pooled(pool))
}

/// What the thread that builds is told.
enum Job {
    /// Build the graph of a session, started at `from`, played at `speed`, into `out`; a grain
    /// of scrubbing alone where `grain` (T18).
    Start {
        epoch: u64,
        project: Arc<Project>,
        timeline: usize,
        from: i64,
        speed: i32,
        grain: bool,
        format: AudioFormat,
        out: SampleProducer,
        device: Arc<device::Shared>,
    },
    /// Build a graph of the newest project of session `epoch`, at the speed it is to play at,
    /// to swap in, `ahead` seconds after what is mixed.
    Swap { epoch: u64, ahead: f64 },
    /// Build the graph of the grain of scrubbing asked for last in session `epoch` and swap it in
    /// (T18).
    Grain { epoch: u64 },
}

/// A graph handed to the thread of the mix: what it was built from, its controls, whether it is
/// the first of its session, and how long it took to build.
struct Built {
    epoch: u64,
    project: Arc<Project>,
    timeline: usize,
    controls: Controls,
    start: bool,
    took: Duration,
    /// Where it is swapped in, and its warp, where it changes the speed (T18).
    speed: Option<(i64, Warp)>,
}

/// What the thread that plays and the thread that builds share.
struct Shared {
    /// The session that plays: the epoch of the last start or stop.
    epoch: AtomicU64,
    /// The newest project of the session, which a graph to swap in is built from.
    latest: Mutex<Option<(u64, Arc<Project>, usize)>>,
    /// The graph the thread that builds handed over last.
    built: Mutex<Option<Built>>,
    /// Why the graph of a session could not be built.
    failed: Mutex<Option<(u64, String)>>,
    /// A graph to swap in for this session could not be built or sent.
    unswapped: Mutex<Option<u64>>,
    /// A graph of this session came after its sample: another is built, and the controls of the
    /// one that came late are of no graph that plays.
    late: Mutex<Option<u64>>,
    /// The speed that the session is to play at (T18).
    speed: AtomicI32,
    /// The sample of the timeline of the grain of scrubbing asked for last and not yet built, and
    /// its session (T18).
    grain: Mutex<Option<(u64, i64)>>,
}

/// The threads of the sound of a timeline monitor, made when it first plays.
struct Threads {
    /// The ring of commands to the thread of the mix, which this thread and the thread that
    /// builds both write to (they lock it between them; the thread of the mix never does).
    commands: Arc<Mutex<MessageProducer<Command>>>,
    jobs: mpsc::Sender<Job>,
    shared: Arc<Shared>,
    counters: Arc<Counters>,
    mix: MixThread,
}

impl Threads {
    fn spawn(opener: Opener) -> Result<Threads, String> {
        let counters = Arc::new(Counters::default());
        let (commands, taken) = messages(1024);
        let (gone, retired) = messages(256);
        let mix = MixThread::spawn(Engine::new(taken, gone, Arc::clone(&counters)))
            .map_err(|error| format!("the thread of the mix cannot be started: {error}"))?;
        let shared = Arc::new(Shared {
            epoch: AtomicU64::new(0),
            latest: Mutex::new(None),
            built: Mutex::new(None),
            failed: Mutex::new(None),
            unswapped: Mutex::new(None),
            late: Mutex::new(None),
            speed: AtomicI32::new(1),
            grain: Mutex::new(None),
        });
        let commands = Arc::new(Mutex::new(commands));
        let (jobs, inbox) = mpsc::channel();
        std::thread::Builder::new()
            .name("columbina-sound-build".into())
            .spawn({
                let builder = Builder {
                    commands: Arc::clone(&commands),
                    retired,
                    shared: Arc::clone(&shared),
                    counters: Arc::clone(&counters),
                    opener,
                    pool: Arc::new(Readers::default()),
                    last_swap: Duration::ZERO,
                    warp: (0, Warp::IDENTITY),
                    changing: None,
                    last_grain: Duration::ZERO,
                };
                move || builder.run(&inbox)
            })
            .map_err(|error| {
                format!("the thread that builds the mix cannot be started: {error}")
            })?;
        Ok(Threads {
            commands,
            jobs,
            shared,
            counters,
            mix,
        })
    }

    /// Tells the thread of the mix `command`; whether it could (its ring may be full).
    fn send(&self, command: Command) -> bool {
        lock(&self.commands).push(command).is_ok()
    }
}

/// The thread that builds. See the module.
struct Builder {
    commands: Arc<Mutex<MessageProducer<Command>>>,
    retired: MessageConsumer<Retired>,
    shared: Arc<Shared>,
    counters: Arc<Counters>,
    opener: Opener,
    /// The readers that the feeds of graphs let go of, for the next.
    pool: Arc<Readers>,
    /// How long the last graph to swap in took to build.
    last_swap: Duration,
    /// The warp of the graph handed over last, and its session (T18): a graph to swap in plays
    /// on from where that one is, at the speed asked for.
    warp: (u64, Warp),
    /// The graph that changes the speed, sent and not yet taken: its sample, its warp, and the
    /// warp before it. Where that graph (and no other) comes too late to be swapped in, what
    /// plays is still at the speed before, and the one built again starts from there; once the
    /// thread of the mix takes it, it is forgotten (T18's review).
    changing: Option<(i64, Warp, Warp)>,
    /// How long the graph of the last grain of scrubbing took to build: the next is swapped in
    /// further ahead where it was slow (T18's review).
    last_grain: Duration,
}

/// How a graph of [`Builder::run_of`] starts.
#[derive(Clone, Copy, Debug, PartialEq)]
enum How {
    /// The first of a session: it prerolls as an export does.
    Start,
    /// One to swap in as the sound plays: it prerolls [`SWAP_PREROLL`] at the most, and is caught
    /// up where the mix went on past where it was to start; one that changes the speed is not
    /// (its warp starts where it was to start), and comes too late instead.
    Swap { speed: bool },
    /// A grain of scrubbing (T18): it prerolls [`GRAIN_PREROLL`] at the most, its clips are read
    /// ahead for [`GRAIN_PRIME`] at the most, and then, one to swap in (`swap`), it is caught up
    /// where the mix went on, if it may (`catch`); else it is left where it was to start, and
    /// how far behind it is told, so that it is built again further ahead (T18's review: its
    /// sound starts at the frame dragged to).
    Grain { swap: bool, catch: bool },
}

impl Builder {
    fn run(mut self, inbox: &mpsc::Receiver<Job>) {
        loop {
            let job = match inbox.recv_timeout(BUILDER_NAP) {
                Ok(job) => Some(job),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            };
            // What the thread of the mix let go of is dropped here; a graph that came after its
            // sample was mixed is built again, further ahead.
            let mut late = None;
            let mut grain = None;
            // A change of speed that the thread of the mix took can no longer come too late.
            if self
                .changing
                .is_some_and(|(at, _, _)| self.counters.scheduled() >= at)
            {
                self.changing = None;
            }
            while let Some(retired) = self.retired.pop() {
                if let Retired::Late { epoch, run } = retired {
                    if let Some(sound) = run.grain() {
                        // A grain asked for again where none newer is (T18).
                        grain = Some((epoch, run.graph().warp().timeline(sound.from)));
                    } else {
                        // The one that changed the speed never played: what plays is at the
                        // speed before. Any other is of the warp that plays.
                        if let Some((at, warp, before)) = self.changing {
                            if self.warp == (epoch, warp)
                                && run.graph().warp() == warp
                                && run.graph().at() == at
                            {
                                self.warp.1 = before;
                                self.changing = None;
                            }
                        }
                        late = Some(epoch);
                    }
                }
            }
            let now = self.shared.epoch.load(Ordering::Acquire);
            if late == Some(now) {
                *lock(&self.shared.late) = Some(now);
                self.swap(now, SWAP_AHEAD * 2.0);
            }
            if let Some((epoch, place)) = grain.filter(|(epoch, _)| *epoch == now) {
                lock(&self.shared.grain).get_or_insert((epoch, place));
                self.grain(now);
            }
            if Fetcher::live() > 64 {
                log(
                    "sound_readers_waiting",
                    &json!({"threads": Fetcher::live(),
                        "message": "readers ahead of graphs gone wait for reads that do not return"}),
                );
            }
            match job {
                Some(Job::Start {
                    epoch,
                    project,
                    timeline,
                    from,
                    speed,
                    grain,
                    format,
                    out,
                    device,
                }) if epoch == self.shared.epoch.load(Ordering::Acquire) => {
                    self.start(
                        epoch,
                        (project, timeline, from, speed, grain),
                        format,
                        (out, device),
                    );
                }
                Some(Job::Swap { epoch, ahead })
                    if epoch == self.shared.epoch.load(Ordering::Acquire) =>
                {
                    self.swap(epoch, ahead);
                }
                Some(Job::Grain { epoch })
                    if epoch == self.shared.epoch.load(Ordering::Acquire) =>
                {
                    self.grain(epoch);
                }
                _ => {}
            }
        }
    }

    /// Tells the thread of the mix `command`; whether it could.
    fn send(&self, command: Command) -> bool {
        let sent = lock(&self.commands).push(command).is_ok();
        if !sent {
            log(
                "sound_failed",
                &json!({"message": "the thread of the mix takes no more commands"}),
            );
        }
        sent
    }

    /// A graph of `project` played through `warp` (T18), started at `at`, its clips read ahead
    /// from there; and its controls. How it starts and is caught up: [`How`].
    fn run_of(
        &self,
        project: &Arc<Project>,
        timeline: usize,
        (at, warp): (i64, Warp),
        how: How,
    ) -> Result<(Box<Run>, Controls, i64), String> {
        let mut graph = Graph::build(
            project,
            timeline,
            Options::new(MOST_BLOCK, Purpose::Preview),
        )
        .map_err(|error| error.to_string())?;
        for clip in graph.unplayable() {
            log("sound_of_a_clip_unplayable", &json!({ "clip": clip }));
        }
        graph.set_warp(warp);
        let rate = graph.rate();
        // Playback goes on where a clip cannot be read: that clip is silent, and told of once.
        let mut feed = (self.opener)(project, timeline, graph.clips(), &self.pool);
        // Caught up to `guard` after what is mixed, and a guard more.
        let catch_up = |graph: &mut Graph, feed: &mut dyn ClipFeed, guard: f64| {
            let guard = i64::try_from(samples(guard, rate)).unwrap_or(0);
            let deadline = Instant::now() + CATCH_UP;
            while self.counters.mixed() + guard > graph.at() && Instant::now() < deadline {
                let wanted = self.counters.mixed() + 2 * guard - graph.at();
                let frames = usize::try_from(wanted).unwrap_or(1).clamp(1, MOST_BLOCK);
                let _ = graph.process(feed, frames);
            }
        };
        match how {
            How::Start => {
                let _ = graph.start(at, feed.as_mut());
            }
            How::Swap { speed } => {
                let most = u32::try_from(samples(SWAP_PREROLL, rate)).unwrap_or(u32::MAX);
                let _ = graph.start_within(at, most, feed.as_mut());
                if !speed {
                    catch_up(&mut graph, feed.as_mut(), SWAP_AHEAD / 2.0);
                }
            }
            How::Grain { .. } => {
                let most = u32::try_from(samples(GRAIN_PREROLL, rate)).unwrap_or(u32::MAX);
                let _ = graph.start_within(at, most, feed.as_mut());
            }
        }
        // Its readers go to the pool, from which the readers ahead take them; a grain's to swap
        // in catch it up first.
        let mut feed = matches!(how, How::Grain { swap: true, .. }).then_some(feed);
        let (ring, mut readers) = ring_feed(&graph, graph.reads_at(), Arc::clone(&self.counters));
        let mut feeds: Vec<Box<dyn ClipFeed + Send>> = readers
            .iter()
            .map(|_| (self.opener)(project, timeline, graph.clips(), &self.pool))
            .collect();
        let grain = matches!(how, How::Grain { .. });
        let deadline = Instant::now() + if grain { GRAIN_PRIME } else { PRIME };
        loop {
            let mut read = 0;
            for (fetch, feed) in readers.iter_mut().zip(&mut feeds) {
                read += fetch.pass(feed.as_mut(), 4);
            }
            if read == 0 || Instant::now() >= deadline {
                break;
            }
        }
        let fetcher = Fetcher::spawn(readers, feeds)
            .map_err(|error| format!("the reader ahead cannot be started: {error}"))?;
        // How far a grain is behind the mix (a little after what is mixed).
        let guard = i64::try_from(samples(GRAIN_AHEAD / 2.0, rate)).unwrap_or(0);
        let behind = (self.counters.mixed() + guard - graph.at()).max(0);
        if let (Some(feed), How::Grain { catch: true, .. }) = (&mut feed, how) {
            // Heard as soon as it can be: from a little after what is mixed, where the mix went
            // on past where it was to start.
            catch_up(&mut graph, feed.as_mut(), GRAIN_AHEAD / 2.0);
        }
        drop(feed);
        let controls = graph.controls();
        // The meters read the graph from when the thread of the mix plays it (T9b); the first
        // graph of a session that starts where the playhead was put clears the loudness, with
        // Lock Metering to Transport on.
        let of = &project.timelines[timeline];
        crate::meters::hub().offer(&of.id, &of.audio, &graph, how == How::Start);
        let end = graph.end_out(end_of(&project.timelines[timeline]));
        let run = Run::new(graph, ring, Some(fetcher), end);
        let run = if grain {
            let from = run.graph().at();
            run.with_grain(Grain {
                from,
                length: i64::try_from(samples(SCRUB_GRAIN, rate)).unwrap_or(0),
                fade: i64::try_from(samples(SCRUB_FADE, rate)).unwrap_or(1),
            })
        } else {
            run
        };
        Ok((Box::new(run), controls, behind))
    }

    /// Builds the session `epoch` and hands it to the thread of the mix.
    fn start(
        &mut self,
        epoch: u64,
        (project, timeline, from, speed, grain): (Arc<Project>, usize, i64, i32, bool),
        format: AudioFormat,
        (out, device): (SampleProducer, Arc<device::Shared>),
    ) {
        let started = Instant::now();
        let rate = project.timelines[timeline].sample_rate;
        let built = (|| {
            if rate == 0 {
                return Err("the timeline has no rate of sound".to_owned());
            }
            let mut resampler =
                Resampler::new(rate, format.rate, usize::from(format.channels), MOST_BLOCK);
            resampler.start(from);
            // Sample `from` out is sample `from` of the timeline, at whatever speed (T18).
            let warp = Warp::new(i64::from(speed), from, from);
            let how = if grain {
                How::Grain {
                    swap: false,
                    catch: false,
                }
            } else {
                How::Start
            };
            // A session starts where it is told, however long its graph took to build.
            let (run, controls, _) =
                self.run_of(&project, timeline, (resampler.first_input(), warp), how)?;
            let run = if grain {
                // The grain from `from`, not from where the resampler takes the mix first.
                let sound = run.grain().map(|sound| Grain { from, ..sound });
                Box::new(match sound {
                    Some(sound) => (*run).with_grain(sound),
                    None => *run,
                })
            } else {
                run
            };
            let session = Session::new(epoch, run, resampler, out, device);
            Ok((session, controls))
        })();
        let failed = match built {
            Ok((session, controls)) => {
                if epoch != self.shared.epoch.load(Ordering::Acquire) {
                    return;
                }
                let warp = session.run().graph().warp();
                if self.send(Command::Start(Box::new(session))) {
                    self.warp = (epoch, warp);
                    self.changing = None;
                    *lock(&self.shared.built) = Some(Built {
                        epoch,
                        project,
                        timeline,
                        controls,
                        start: true,
                        took: started.elapsed(),
                        speed: None,
                    });
                    return;
                }
                "the thread of the mix takes no more commands".to_owned()
            }
            Err(message) => message,
        };
        log(
            "sound_unavailable",
            &json!({"monitor": "timeline", "message": failed}),
        );
        *lock(&self.shared.failed) = Some((epoch, failed));
    }

    /// Builds a graph of the newest project of session `epoch` and sends it to be swapped in,
    /// `ahead` seconds after what is mixed (further where the last took longer to build), or as
    /// soon after as it is ready. One that cannot be built or sent is told of, so that the next
    /// change builds another.
    fn swap(&mut self, epoch: u64, ahead: f64) {
        let Some((latest, project, timeline)) = lock(&self.shared.latest).clone() else {
            return;
        };
        if latest != epoch {
            return;
        }
        let started = Instant::now();
        let rate = project.timelines[timeline].sample_rate;
        let ahead = ahead
            .max(self.last_swap.as_secs_f64() * 1.5 + SWAP_AHEAD)
            .min(LONGEST_AHEAD);
        // Ahead of what is mixed, and a crossfade and a block after a graph that waits to be
        // swapped in, so that it follows that one (`Engine`: changes that keep coming are heard
        // as each graph is ready).
        let after = self.counters.scheduled()
            + i64::try_from(samples(GRAPH_SWAP_CROSSFADE + MIX_BLOCK, rate)).unwrap_or(0);
        let at =
            (self.counters.mixed() + i64::try_from(samples(ahead, rate)).unwrap_or(0)).max(after);
        // On from where the graph that plays then is, at the speed asked for (T18).
        let base = match self.warp {
            (session, warp) if session == epoch => warp,
            _ => {
                *lock(&self.shared.unswapped) = Some(epoch);
                return;
            }
        };
        let speed = i64::from(self.shared.speed.load(Ordering::Acquire));
        let warp = if speed == base.speed() {
            base
        } else {
            base.with_speed(speed, at)
        };
        let changes = warp != base;
        let how = How::Swap { speed: changes };
        match self.run_of(&project, timeline, (at, warp), how) {
            Ok((run, controls, _)) => {
                if epoch != self.shared.epoch.load(Ordering::Acquire) {
                    return;
                }
                let at = run.graph().at();
                let took = started.elapsed();
                self.last_swap = took;
                if self.send(Command::Swap { epoch, run, at }) {
                    if changes {
                        self.changing = Some((at, warp, base));
                        self.warp = (epoch, warp);
                    }
                    *lock(&self.shared.built) = Some(Built {
                        epoch,
                        project,
                        timeline,
                        controls,
                        start: false,
                        took,
                        speed: changes.then_some((at, warp)),
                    });
                    return;
                }
            }
            Err(message) => log(
                "sound_failed",
                &json!({"monitor": "timeline", "message": message}),
            ),
        }
        *lock(&self.shared.unswapped) = Some(epoch);
    }

    /// Builds the graph of the grain of scrubbing asked for last in session `epoch`, if one is
    /// waiting, and sends it to be swapped in [`GRAIN_AHEAD`] after what is mixed, or as soon after
    /// as it is ready (T18). Grains asked for while it was built wait for the next.
    fn grain(&mut self, epoch: u64) {
        let Some((asked, place)) = lock(&self.shared.grain).take() else {
            return;
        };
        let Some((latest, project, timeline)) = lock(&self.shared.latest).clone() else {
            return;
        };
        if asked != epoch || latest != epoch {
            return;
        }
        // What is mixed is of this session once the thread of the mix took it, which it does
        // within a step of being sent it.
        let deadline = Instant::now() + CATCH_UP;
        while self.counters.session() != epoch && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        let rate = project.timelines[timeline].sample_rate;
        for attempt in 0..GRAIN_ATTEMPTS {
            // Ahead of what is mixed by as long as the last grain took to build and half again,
            // and the lead of a grain (T18's review).
            let lead = (self.last_grain.as_secs_f64() * 1.5 + GRAIN_AHEAD).min(GRAIN_LONGEST_AHEAD);
            let after = self.counters.scheduled()
                + i64::try_from(samples(GRAPH_SWAP_CROSSFADE + MIX_BLOCK, rate)).unwrap_or(0);
            let at = (self.counters.mixed() + i64::try_from(samples(lead, rate)).unwrap_or(0))
                .max(after);
            let warp = Warp::new(1, at, place);
            let started = Instant::now();
            // Caught up only at the last attempt: the sound of a grain starts at its frame.
            let how = How::Grain {
                swap: true,
                catch: attempt + 1 == GRAIN_ATTEMPTS,
            };
            match self.run_of(&project, timeline, (at, warp), how) {
                Ok((run, _, behind)) => {
                    self.last_grain = started.elapsed();
                    if epoch != self.shared.epoch.load(Ordering::Acquire) {
                        return;
                    }
                    if behind > 0 && attempt + 1 < GRAIN_ATTEMPTS {
                        // Too late to start at its frame: again, further ahead.
                        continue;
                    }
                    let at = run.graph().at();
                    self.send(Command::Swap { epoch, run, at });
                    return;
                }
                Err(message) => {
                    log(
                        "sound_failed",
                        &json!({"monitor": "timeline", "message": message}),
                    );
                    return;
                }
            }
        }
    }
}

/// What a change of the project did to the sound that plays ([`Playback::changed`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Heard {
    /// Nothing that is heard from what is mixed on changed.
    Same,
    /// So many settings were sent to the graph that plays.
    Set(usize),
    /// Another graph is built to be swapped in.
    Swap,
    /// The sound is to start again (the rates of the timeline changed).
    Again,
}

/// What the thread that builds did that the thread that plays is to act on ([`Playback::poll`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    /// The first graph of the session could not be built.
    Failed(String),
    /// The first graph of the session has not been handed over within [`SLOW`]: the picture goes
    /// on by the clock of the system, and the graph is built on.
    Slow,
    /// The first graph of a session that was slow has been handed over: the sound may start
    /// again where the picture is.
    Ready,
    /// The sound plays at `speed` from sample `at` out of the session on (T18): the picture
    /// changes its speed there too.
    Speed {
        /// The sample out, counted as the session counts them (it started at its sample `from` of
        /// the timeline).
        at: i64,
        /// The speed.
        speed: i32,
    },
}

/// What plays: the project its graph was built from, or that settings brought it to, and the
/// newest project.
#[allow(
    clippy::struct_excessive_bools,
    reason = "the states of a graph being built"
)]
struct Current {
    base: Arc<Project>,
    timeline: usize,
    controls: Option<Controls>,
    /// A graph is being built (the session's first, or one to swap in).
    pending: bool,
    /// The project changed while it was.
    dirty: bool,
    latest: (Arc<Project>, usize),
    /// When the session was started, whether its first graph was handed over, and whether it
    /// was told to be slow.
    started: Instant,
    handed: bool,
    slow: bool,
    /// The speed the sound plays at, and the speed asked for (T18).
    speed: i32,
    wanted: i32,
    /// A graph that changes the speed, sent to be swapped in at this sample with this warp: it is
    /// waited for before the next change is sent, so that it is never put aside by a newer graph.
    changing: Option<(i64, Warp)>,
    /// Where each speed began, in samples out: the session's own from its sample `from` of the
    /// timeline, then each change.
    warps: Vec<(i64, Warp)>,
    /// Whether it is a session of grains of scrubbing (T18).
    grains: bool,
}

/// The sound of a timeline monitor as it plays. See the module.
pub(crate) struct Playback {
    threads: Option<Threads>,
    opener: Opener,
    epoch: u64,
    current: Option<Current>,
    /// How long the first graph of the last session took to build.
    last_start: Duration,
}

impl Playback {
    /// Nothing playing; the threads are made when it first plays.
    pub(crate) fn new() -> Self {
        Self::with_opener(media_feed)
    }

    /// The same, the clips read through what `opener` opens.
    pub(crate) fn with_opener(opener: Opener) -> Self {
        Playback {
            threads: None,
            opener,
            epoch: 0,
            current: None,
            last_start: Duration::ZERO,
        }
    }

    fn threads(&mut self) -> Result<&Threads, String> {
        if self.threads.is_none() {
            self.threads = Some(Threads::spawn(self.opener)?);
        }
        Ok(self.threads.as_ref().expect("made above"))
    }

    /// Plays timeline `timeline` of `project` from sample `from` at `speed` (T18) on `output`,
    /// in place of what played: the device plays silence, not counted, until the graph is built
    /// and started.
    ///
    /// # Errors
    ///
    /// A thread cannot be started, or the device cannot.
    pub(crate) fn start(
        &mut self,
        output: &mut Output,
        project: Arc<Project>,
        timeline: usize,
        (from, speed): (i64, i32),
    ) -> Result<(), String> {
        self.begin(output, project, timeline, (from, speed, false))
    }

    /// Plays the grain of scrubbing of sample `from` of timeline `timeline` of `project` (T18):
    /// in the session of grains that plays, or in a new one on `output`.
    ///
    /// # Errors
    ///
    /// A thread cannot be started, or the device cannot.
    pub(crate) fn scrub(
        &mut self,
        output: &mut Output,
        project: Arc<Project>,
        timeline: usize,
        from: i64,
    ) -> Result<(), String> {
        let epoch = self.epoch;
        // A session of grains of another rate of the timeline starts again (T18's review).
        let same_rates = |current: &Current| {
            let was = &current.base.timelines[current.timeline];
            let now = &project.timelines[timeline];
            (was.rate, was.sample_rate) == (now.rate, now.sample_rate)
        };
        match (&self.threads, &mut self.current) {
            (Some(threads), Some(current)) if current.grains && same_rates(current) => {
                current.latest = (Arc::clone(&project), timeline);
                *lock(&threads.shared.latest) = Some((epoch, project, timeline));
                *lock(&threads.shared.grain) = Some((epoch, from));
                threads
                    .jobs
                    .send(Job::Grain { epoch })
                    .map_err(|_| "the thread that builds the mix has ended".to_owned())
            }
            _ => self.begin(output, project, timeline, (from, 1, true)),
        }
    }

    /// Fades out what the session of grains plays over [`SCRUB_FADE`] (T18's review), the
    /// session going on in silence until it is stopped; whether one plays.
    pub(crate) fn fade_out(&mut self) -> bool {
        let epoch = self.epoch;
        match (&self.threads, &self.current) {
            (Some(threads), Some(current)) if current.grains => threads.send(Command::Fade(epoch)),
            _ => false,
        }
    }

    /// Whether a session of grains of scrubbing plays (T18).
    pub(crate) fn scrubbing(&self) -> bool {
        self.current.as_ref().is_some_and(|current| current.grains)
    }

    /// Asks for the sound to play at `speed` from a little after where it is (T18): another graph,
    /// swapped in, in the same session; [`Event::Speed`] tells where it does. False where no
    /// session plays (nor a session of grains), which is then to be started at that speed.
    pub(crate) fn set_speed(&mut self, speed: i32) -> bool {
        let (Some(threads), Some(current)) = (&self.threads, &mut self.current) else {
            return false;
        };
        if current.grains {
            return false;
        }
        current.wanted = speed;
        threads.shared.speed.store(speed, Ordering::Release);
        let (project, timeline) = current.latest.clone();
        self.changed(&project, timeline);
        true
    }

    /// The sample of the timeline that sample `sample` out of the session is: where the sound
    /// is, at the speeds it played at (T18).
    pub(crate) fn timeline_at(&self, sample: i64) -> Option<i64> {
        let current = self.current.as_ref()?;
        current
            .warps
            .iter()
            .rev()
            .find(|(from, _)| *from <= sample)
            .or(current.warps.first())
            .map(|(_, warp)| warp.timeline(sample))
    }

    fn begin(
        &mut self,
        output: &mut Output,
        project: Arc<Project>,
        timeline: usize,
        (from, speed, grains): (i64, i32, bool),
    ) -> Result<(), String> {
        self.epoch += 1;
        let epoch = self.epoch;
        self.current = None;
        let threads = self.threads()?;
        threads.shared.epoch.store(epoch, Ordering::Release);
        threads.shared.speed.store(speed, Ordering::Release);
        *lock(&threads.shared.grain) = None;
        *lock(&threads.shared.latest) = Some((epoch, Arc::clone(&project), timeline));
        // The session before ends at once, not when this one is built.
        threads.send(Command::Stop(epoch));
        let out = output.begin()?;
        threads
            .jobs
            .send(Job::Start {
                epoch,
                project: Arc::clone(&project),
                timeline,
                from,
                speed,
                grain: grains,
                format: output.format(),
                out,
                device: output.device(),
            })
            .map_err(|_| "the thread that builds the mix has ended".to_owned())?;
        self.current = Some(Current {
            base: Arc::clone(&project),
            timeline,
            controls: None,
            pending: true,
            dirty: false,
            latest: (project, timeline),
            started: Instant::now(),
            handed: false,
            slow: false,
            speed,
            wanted: speed,
            changing: None,
            warps: vec![(i64::MIN, Warp::new(i64::from(speed), from, from))],
            grains,
        });
        Ok(())
    }

    /// Stops what plays.
    pub(crate) fn stop(&mut self) {
        if self.current.take().is_none() {
            return;
        }
        self.epoch += 1;
        if let Some(threads) = &self.threads {
            threads.shared.epoch.store(self.epoch, Ordering::Release);
            *lock(&threads.shared.latest) = None;
            threads.send(Command::Stop(self.epoch));
        }
    }

    /// Takes what the thread that builds has done: the controls of the graph it handed over, a
    /// graph that came late or could not be swapped in, and what changed while one was built;
    /// and tells what the thread that plays is to act on.
    pub(crate) fn poll(&mut self) -> Option<Event> {
        let threads = self.threads.as_ref()?;
        let current = self.current.as_mut()?;
        let mine = |epoch: Option<u64>| epoch == Some(self.epoch);
        if let Some((epoch, message)) = lock(&threads.shared.failed).take() {
            if epoch == self.epoch {
                return Some(Event::Failed(message));
            }
        }
        if mine(lock(&threads.shared.unswapped).take()) {
            // The next change builds another.
            current.pending = false;
            current.dirty = false;
        }
        if mine(lock(&threads.shared.late).take()) {
            // Another is built from the newest project; the controls of the one that came late
            // set nothing that plays.
            current.pending = true;
            current.controls = None;
        }
        // A graph that changes the speed, once it plays: the picture changes its speed where it
        // does, and the changes that waited for it are taken (T18).
        if let Some((at, warp)) = current.changing {
            let counters = &threads.counters;
            if counters.scheduled() == at && counters.mixed() > at {
                current.changing = None;
                current.pending = false;
                let speed = i32::try_from(warp.speed()).unwrap_or(1);
                current.speed = speed;
                current.warps.push((at, warp));
                if std::mem::take(&mut current.dirty) || current.wanted != speed {
                    let (project, timeline) = current.latest.clone();
                    self.changed(&project, timeline);
                }
                return Some(Event::Speed { at, speed });
            }
        }
        let built = lock(&threads.shared.built).take();
        if let Some(built) = built.filter(|built| built.epoch == self.epoch) {
            current.base = built.project;
            current.timeline = built.timeline;
            current.controls = Some(built.controls);
            if let Some(changing) = built.speed {
                // Pending until it plays.
                current.changing = Some(changing);
                return None;
            }
            current.pending = false;
            let ready = built.start && std::mem::take(&mut current.slow);
            if built.start {
                current.handed = true;
                self.last_start = built.took;
            }
            if std::mem::take(&mut current.dirty) || current.wanted != current.speed {
                let (project, timeline) = current.latest.clone();
                self.changed(&project, timeline);
            }
            return ready.then_some(Event::Ready);
        }
        if !current.handed && !current.slow && current.started.elapsed() > SLOW {
            current.slow = true;
            return Some(Event::Slow);
        }
        None
    }

    /// Takes `project` in place of the one that plays (timeline `timeline` of it is the one that
    /// plays), and changes the sound as it must. See the module.
    pub(crate) fn changed(&mut self, project: &Arc<Project>, timeline: usize) -> Heard {
        let epoch = self.epoch;
        let (Some(threads), Some(current)) = (self.threads.as_ref(), self.current.as_mut()) else {
            return Heard::Same;
        };
        current.latest = (Arc::clone(project), timeline);
        *lock(&threads.shared.latest) = Some((epoch, Arc::clone(project), timeline));
        let was = &current.base.timelines[current.timeline];
        let now = &project.timelines[timeline];
        if (was.rate, was.sample_rate) != (now.rate, now.sample_rate) {
            return Heard::Again;
        }
        if current.grains {
            // A grain of scrubbing plays what it was built of; the next is of the newest project.
            return Heard::Same;
        }
        let sets = current
            .controls
            .as_ref()
            .and_then(|controls| retune(was, now, controls));
        let send = |sets: &[(columbina_audio::graph::Control, f32)]| {
            sets.iter()
                .all(|&(control, value)| threads.send(Command::Set(control, value)))
        };
        if current.pending {
            // Heard at once where the graph that plays has them; the graph being built is of an
            // older project, and another follows it.
            if let Some(sets) = &sets {
                send(sets);
            }
            current.dirty = true;
            return Heard::Swap;
        }
        // Where the mix is on the timeline: what is heard from there on, at any speed (T18).
        let mixed = threads.counters.mixed();
        let from = current
            .warps
            .iter()
            .rev()
            .find(|(from, _)| *from <= mixed)
            .map_or(mixed, |(_, warp)| warp.timeline(mixed));
        let from = if current.speed < 0 { 0 } else { from };
        // The fixed gains and pans of clips are controls; what else the clips sound is compared.
        let levelled = without_clip_levels(was, now);
        let clips = super::timeline::same_clips(&current.base, was, project, &levelled, from);
        let speed = current.wanted == current.speed;
        if let (true, true, Some(sets)) = (clips, speed, &sets) {
            if send(sets) {
                current.base = Arc::clone(project);
                current.timeline = timeline;
                return if sets.is_empty() {
                    Heard::Same
                } else {
                    Heard::Set(sets.len())
                };
            }
        }
        current.pending = true;
        if threads
            .jobs
            .send(Job::Swap {
                epoch,
                ahead: SWAP_AHEAD,
            })
            .is_err()
        {
            return Heard::Again;
        }
        Heard::Swap
    }

    /// Whether a session plays.
    pub(crate) fn playing(&self) -> bool {
        self.current.is_some()
    }

    /// The sample out at which the graph taken last to be swapped in is swapped in.
    #[cfg(test)]
    pub(crate) fn scheduled(&self) -> i64 {
        self.threads
            .as_ref()
            .map_or(i64::MIN, |threads| threads.counters.scheduled())
    }

    /// How long the first graph of the last session took to build.
    pub(crate) fn last_start(&self) -> Duration {
        self.last_start
    }

    /// The reads of a clip that came before it was read ahead, and the graphs swapped in, so far.
    pub(crate) fn counts(&self) -> (u64, u64) {
        self.threads.as_ref().map_or((0, 0), |threads| {
            (threads.counters.underruns(), threads.counters.swaps())
        })
    }

    /// The graphs that came too late to be swapped in, so far; and the priority of the thread of
    /// the mix.
    pub(crate) fn mix_thread(&self) -> (u64, columbina_audio::play::thread::Priority) {
        self.threads.as_ref().map_or(
            (0, columbina_audio::play::thread::Priority::Normal),
            |threads| (threads.counters.late(), threads.mix.priority()),
        )
    }
}

impl Drop for Playback {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::audio::Kind;

    /// Whether the graph of what plays has been handed to the thread of the mix.
    fn ready(playback: &mut Playback) -> bool {
        assert_eq!(playback.poll(), None);
        playback
            .current
            .as_ref()
            .is_some_and(|current| !current.pending)
    }

    fn wait(what: &str, mut until: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(90);
        while !until() {
            assert!(Instant::now() < deadline, "{what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Sixteen stereo tracks of the WAV of the tests (2 s of 440 Hz), each with Dynamics on, out
    /// to B1; none where the media cannot be made.
    fn sixteen_tracks_of_media() -> Option<Project> {
        let made = crate::test_media::media(crate::test_media::Sample::Wav)?;
        let mut project = Project::sample();
        let facts = columbina_media::probe(&made.path).unwrap();
        project.media[0].path = made.path.to_string_lossy().into_owned();
        project.media[0].facts = facts.clone();
        let timeline = &mut project.timelines[0];
        timeline
            .tracks
            .retain(|track| track.kind == columbina_model::TrackKind::Audio);
        let mut first = timeline.tracks[0].clone();
        first.clips[0].source = columbina_model::Source::Media {
            media: "m1".into(),
            source_in: columbina_model::Time {
                ticks: 0,
                scale: facts.duration.scale,
            },
        };
        first.clips[0].length = 50;
        first.audio.strip.dynamics.on = true;
        first.audio.strip.dynamics.compressor.on = true;
        timeline.tracks = (0..16)
            .map(|n| {
                let mut track = first.clone();
                track.id = format!("A{n}");
                track.clips[0].id = format!("c{n}");
                track
            })
            .collect();
        Some(project)
    }

    #[test]
    #[allow(clippy::cast_precision_loss, reason = "milliseconds of a test")]
    fn the_time_to_start_and_to_swap_sixteen_tracks() {
        let Some(project) = sixteen_tracks_of_media() else {
            return;
        };
        let mut output = Output::open(&Kind::Null).unwrap();
        let mut playback = Playback::new();
        let started = Instant::now();
        playback
            .start(&mut output, Arc::new(project.clone()), 0, (0, 1))
            .unwrap();
        wait("the first sound", || {
            playback.poll();
            output.played(Instant::now()).is_some()
        });
        let start = started.elapsed();
        println!("sixteen tracks: the first sound {start:?} after the start");
        wait("the first graph", || ready(&mut playback));
        let mut changed = project.clone();
        changed.timelines[0].tracks[3].audio.strip.eq.on = true;
        let asked = Instant::now();
        assert_eq!(playback.changed(&Arc::new(changed.clone()), 0), Heard::Swap);
        wait("the graph swapped in", || {
            playback.poll();
            playback.counts().1 == 1
        });
        let swap = asked.elapsed();
        // A clip's volume line dragged: a setting, not another graph.
        wait("the graph", || ready(&mut playback));
        let mut louder = changed.clone();
        louder.timelines[0].tracks[5].clips[0].gain_db = (-4.0).into();
        louder.timelines[0].tracks[6].clips[0].pan = 0.3.into();
        assert_eq!(playback.changed(&Arc::new(louder), 0), Heard::Set(2));
        println!(
            "sixteen tracks: the first sound {:.0} ms after the start, a graph swapped in {:.0} ms              after the change",
            start.as_secs_f64() * 1000.0,
            swap.as_secs_f64() * 1000.0
        );
        playback.stop();
    }

    #[test]
    fn a_setting_is_sent_to_the_graph_another_change_swaps_one_in_and_the_rates_start_again() {
        let mut output = Output::open(&Kind::Null).unwrap();
        let mut playback = Playback::new();
        let project = Project::sample();
        playback
            .start(&mut output, Arc::new(project.clone()), 0, (0, 1))
            .unwrap();
        let session = output.session();
        wait("the first graph", || ready(&mut playback));
        // A fader, a mute, a solo: sent to the graph as it plays.
        let mut quieter = project.clone();
        quieter.timelines[0].tracks[1].audio.strip.fader_db = -6.0;
        assert_eq!(
            playback.changed(&Arc::new(quieter.clone()), 0),
            Heard::Set(1)
        );
        let mut muted = quieter.clone();
        muted.timelines[0].tracks[1].audio.mute = true;
        assert_eq!(playback.changed(&Arc::new(muted.clone()), 0), Heard::Set(1));
        assert_eq!(playback.changed(&Arc::new(muted.clone()), 0), Heard::Same);
        // A clip's fade: another graph, swapped in.
        let mut gained = muted.clone();
        gained.timelines[0].tracks[1].clips[0].fade_in = 10;
        assert_eq!(playback.changed(&Arc::new(gained.clone()), 0), Heard::Swap);
        wait("the graph swapped in", || {
            playback.poll();
            playback.counts().1 == 1
        });
        assert!(ready(&mut playback));
        // A change while one is built: the settings at once, another graph after it.
        let mut equalized = gained.clone();
        equalized.timelines[0].tracks[1].audio.strip.eq.on = true;
        assert_eq!(
            playback.changed(&Arc::new(equalized.clone()), 0),
            Heard::Swap
        );
        let mut moved = equalized.clone();
        moved.timelines[0].tracks[1].audio.strip.fader_db = -12.0;
        assert_eq!(playback.changed(&Arc::new(moved.clone()), 0), Heard::Swap);
        wait("the second graph", || {
            playback.poll();
            playback.counts().1 == 2 && ready(&mut playback)
        });
        // What the newest graph was built from is the newest project: nothing more to do.
        assert_eq!(playback.changed(&Arc::new(moved.clone()), 0), Heard::Same);
        // The sound never started again.
        assert_eq!(output.session(), session);
        assert_eq!(
            playback.counts().0,
            0,
            "nothing came before it was read ahead"
        );
        // Another rate of sound: it starts again.
        let mut other = moved.clone();
        other.timelines[0].sample_rate = 44_100;
        assert_eq!(playback.changed(&Arc::new(other), 0), Heard::Again);
        playback.stop();
        assert!(!playback.playing());
    }

    /// How long the next feed that [`stalls_once`] opens waits first.
    static STALL_ONCE: AtomicU64 = AtomicU64::new(0);

    /// The media readers, the first opened after [`STALL_ONCE`] was set waiting that long first,
    /// as a drive that sleeps does.
    fn stalls_once(
        project: &Arc<Project>,
        timeline: usize,
        sources: &[ClipSource],
        pool: &Arc<Readers>,
    ) -> Box<dyn ClipFeed + Send> {
        let wait = STALL_ONCE.swap(0, Ordering::AcqRel);
        std::thread::sleep(Duration::from_millis(wait));
        media_feed(project, timeline, sources, pool)
    }

    /// How long each feed that [`slow_to_open`] opens waits first.
    static SLOW_TO_OPEN: AtomicU64 = AtomicU64::new(0);

    fn slow_to_open(
        project: &Arc<Project>,
        timeline: usize,
        sources: &[ClipSource],
        pool: &Arc<Readers>,
    ) -> Box<dyn ClipFeed + Send> {
        std::thread::sleep(Duration::from_millis(SLOW_TO_OPEN.load(Ordering::Acquire)));
        media_feed(project, timeline, sources, pool)
    }

    #[test]
    fn a_session_slow_to_start_is_told_after_two_seconds_and_again_once_it_is_ready() {
        let mut output = Output::open(&Kind::Null).unwrap();
        let mut playback = Playback::with_opener(stalls_once);
        STALL_ONCE.store(3_500, Ordering::Release);
        let started = Instant::now();
        playback
            .start(&mut output, Arc::new(Project::sample()), 0, (0, 1))
            .unwrap();
        let mut slow = None;
        wait("told slow", || {
            let event = playback.poll();
            assert_ne!(event, Some(Event::Ready));
            slow = (event == Some(Event::Slow)).then(|| started.elapsed());
            slow.is_some()
        });
        let slow = slow.unwrap();
        assert!(
            slow >= SLOW && slow < SLOW + Duration::from_millis(500),
            "{slow:?}"
        );
        // Told once, and then that it is ready once the graph is handed over.
        let mut ready = None;
        wait("told ready", || {
            let event = playback.poll();
            assert_ne!(event, Some(Event::Slow));
            ready = (event == Some(Event::Ready)).then(|| started.elapsed());
            ready.is_some()
        });
        assert!(ready.unwrap() >= Duration::from_millis(3_500));
        assert!(playback.last_start() >= Duration::from_millis(3_500));
        wait("its sound", || output.played(Instant::now()).is_some());
        playback.stop();
    }

    #[test]
    fn a_graph_that_cannot_be_built_lets_the_next_change_build_another() {
        let mut output = Output::open(&Kind::Null).unwrap();
        let mut playback = Playback::new();
        let project = Project::sample();
        playback
            .start(&mut output, Arc::new(project.clone()), 0, (0, 1))
            .unwrap();
        wait("the first graph", || ready(&mut playback));
        // B2 and B1 into each other: a loop, which no graph is built of.
        let mut looped = project.clone();
        let mut b2 = columbina_model::Bus::first();
        b2.id = "B2".into();
        b2.strip.outputs = vec![columbina_model::Output {
            bus: "B1".into(),
            on: true,
        }];
        looped.timelines[0].buses[0].strip.outputs = vec![columbina_model::Output {
            bus: "B2".into(),
            on: true,
        }];
        looped.timelines[0].buses.push(b2);
        assert_eq!(playback.changed(&Arc::new(looped), 0), Heard::Swap);
        wait("the graph given up", || ready(&mut playback));
        assert_eq!(playback.counts().1, 0);
        // The next change builds another, from the project as it is.
        let mut equalized = project.clone();
        equalized.timelines[0].tracks[1].audio.strip.eq.on = true;
        assert_eq!(playback.changed(&Arc::new(equalized), 0), Heard::Swap);
        wait("the graph swapped in", || {
            playback.poll();
            playback.counts().1 == 1
        });
        playback.stop();
    }

    #[test]
    fn a_graph_that_came_too_late_is_built_again_with_the_settings_changed_meanwhile() {
        let mut output = Output::open(&Kind::Null).unwrap();
        let mut playback = Playback::with_opener(slow_to_open);
        SLOW_TO_OPEN.store(0, Ordering::Release);
        let project = Project::sample();
        playback
            .start(&mut output, Arc::new(project.clone()), 0, (0, 1))
            .unwrap();
        wait("the first graph", || ready(&mut playback));
        // Each feed now takes 200 ms to open: the graph to swap in is ready after its sample.
        SLOW_TO_OPEN.store(200, Ordering::Release);
        let mut equalized = project.clone();
        equalized.timelines[0].tracks[1].audio.strip.eq.on = true;
        assert_eq!(
            playback.changed(&Arc::new(equalized.clone()), 0),
            Heard::Swap
        );
        wait("a graph too late", || {
            playback.poll();
            playback.mix_thread().0 > 0
        });
        // A fader moved while another is built: the graph built again has it.
        // The settings of the graph that came late reach no graph that plays; the graph built
        // again is of the newest project, which has them.
        let mut moved = equalized.clone();
        moved.timelines[0].tracks[1].audio.strip.fader_db = -9.0;
        playback.changed(&Arc::new(moved.clone()), 0);
        wait("the graph swapped in", || {
            playback.poll();
            playback.counts().1 >= 1 && ready(&mut playback)
        });
        SLOW_TO_OPEN.store(0, Ordering::Release);
        assert_eq!(playback.changed(&Arc::new(moved), 0), Heard::Same);
        playback.stop();
    }

    /// The sample project with its track of sound only, its clip the 2 s of 440 Hz of the WAV
    /// of the tests; none where the media cannot be made.
    fn a_tone_of_two_seconds() -> Option<Project> {
        let made = crate::test_media::media(crate::test_media::Sample::Wav)?;
        let mut project = Project::sample();
        let facts = columbina_media::probe(&made.path).unwrap();
        project.media[0].path = made.path.to_string_lossy().into_owned();
        project.media[0].facts = facts.clone();
        let timeline = &mut project.timelines[0];
        timeline
            .tracks
            .retain(|track| track.kind == columbina_model::TrackKind::Audio);
        let clip = &mut timeline.tracks[0].clips[0];
        clip.source = columbina_model::Source::Media {
            media: "m1".into(),
            source_in: columbina_model::Time {
                ticks: 0,
                scale: facts.duration.scale,
            },
        };
        clip.length = 50;
        Some(project)
    }

    /// The left side of what a capture wrote into the file of its session `session`.
    fn captured(path: &std::path::Path, session: u64) -> Vec<f32> {
        std::fs::read(crate::audio::capture_file(path, session))
            .unwrap()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|word| f32::from_le_bytes(*word))
            .step_by(2)
            .collect()
    }

    /// The frequency of a steady tone in `signal` at 48 kHz: its upward zero crossings, each
    /// placed between its two samples on a line, counted over the time from the first to the
    /// last.
    #[allow(clippy::cast_precision_loss, reason = "samples of a test")]
    fn frequency(signal: &[f32]) -> f64 {
        let crossings: Vec<f64> = signal
            .windows(2)
            .enumerate()
            .filter(|(_, pair)| pair[0] < 0.0 && pair[1] >= 0.0)
            .map(|(n, pair)| n as f64 + f64::from(-pair[0]) / f64::from(pair[1] - pair[0]))
            .collect();
        let (first, last) = (crossings[0], crossings[crossings.len() - 1]);
        (crossings.len() - 1) as f64 * 48_000.0 / (last - first)
    }

    /// The runs of `signal` that are not silent, gaps of fewer than 16 silent samples within
    /// them taken in (where a tone crosses 0): where each starts, and how long it is.
    fn sounding(signal: &[f32]) -> Vec<(usize, usize)> {
        let mut runs: Vec<(usize, usize)> = Vec::new();
        for (n, sample) in signal.iter().enumerate() {
            if *sample == 0.0 {
                continue;
            }
            match runs.last_mut() {
                Some((start, length)) if n - (*start + *length) < 16 => *length = n + 1 - *start,
                _ => runs.push((n, 1)),
            }
        }
        runs
    }

    #[test]
    fn at_twice_the_speed_a_tone_of_440_hz_sounds_at_880_hz_and_backward_at_440_hz() {
        let Some(project) = a_tone_of_two_seconds() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("heard.f32");
        for speed in [2, -1] {
            let mut output = Output::open(&Kind::Capture(path.clone())).unwrap();
            let mut playback = Playback::new();
            // Backward from the end of the clip.
            let from = if speed > 0 { 0 } else { 96_000 };
            playback
                .start(&mut output, Arc::new(project.clone()), 0, (from, speed))
                .unwrap();
            wait("sound", || {
                playback.poll();
                output.played(Instant::now()).is_some_and(|n| n > 40_000)
            });
            playback.stop();
            output.stop();
            drop(output);
            let left = captured(&path, 1);
            let measured = frequency(&left[4_800..38_400]);
            let want = 440.0 * f64::from(speed.unsigned_abs());
            println!("at {speed}: {measured:.3} Hz");
            assert!((measured - want).abs() < 1.0, "at {speed}: {measured} Hz");
        }
    }

    #[test]
    fn a_change_of_speed_while_it_plays_is_heard_in_the_same_session_and_told_where() {
        let Some(project) = a_tone_of_two_seconds() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("heard.f32");
        let mut output = Output::open(&Kind::Capture(path.clone())).unwrap();
        let mut playback = Playback::new();
        playback
            .start(&mut output, Arc::new(project.clone()), 0, (0, 1))
            .unwrap();
        let session = output.session();
        wait("the first graph", || ready(&mut playback));
        wait("sound", || {
            output.played(Instant::now()).is_some_and(|n| n > 14_400)
        });
        assert!(playback.set_speed(2));
        let mut told = None;
        wait("the change of speed", || {
            if let Some(Event::Speed { at, speed }) = playback.poll() {
                told = Some((at, speed));
            }
            told.is_some()
        });
        let (at, speed) = told.unwrap();
        assert_eq!(speed, 2);
        // Where the timeline is: at speed 1 up to the change, at 2 from there.
        assert_eq!(playback.timeline_at(at - 100), Some(at - 100));
        assert_eq!(playback.timeline_at(at + 100), Some(at + 200));
        wait("sound at the new speed", || {
            output
                .played(Instant::now())
                .is_some_and(|n| n > at + 14_400)
        });
        assert_eq!(output.session(), session, "the sound never started again");
        assert_eq!(playback.counts(), (0, 1));
        assert_eq!(output.short_runs(), 0);
        playback.stop();
        output.stop();
        drop(output);
        let left = captured(&path, 1);
        let at = usize::try_from(at).unwrap();
        let before = frequency(&left[at - 9_600..at]);
        let after = frequency(&left[at + 480..at + 9_600]);
        println!("a change of speed at {at}: {before:.3} Hz, then {after:.3} Hz");
        assert!((before - 440.0).abs() < 1.0, "{before}");
        assert!((after - 880.0).abs() < 1.0, "{after}");
        let step = |range: std::ops::Range<usize>| {
            left[range]
                .windows(2)
                .map(|pair| (pair[1] - pair[0]).abs())
                .fold(0.0, f32::max)
        };
        let (across, normal) = (step(at - 480..at + 960), step(at + 960..at + 9_600));
        assert!(across <= normal * 1.1, "{across} > {normal} · 1.1");
    }

    #[test]
    fn grains_of_scrubbing_play_in_one_session_60_ms_each() {
        let Some(project) = a_tone_of_two_seconds() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("heard.f32");
        let mut output = Output::open(&Kind::Capture(path.clone())).unwrap();
        let mut playback = Playback::new();
        let project = Arc::new(project);
        let session = output.session() + 1;
        for (n, place) in [4_800, 48_000, 21_000, 72_000].into_iter().enumerate() {
            playback
                .scrub(&mut output, Arc::clone(&project), 0, place)
                .unwrap();
            assert!(playback.scrubbing());
            wait("the grain", || {
                playback.poll();
                output.played(Instant::now()).is_some()
                    && playback.counts().1 == u64::try_from(n).unwrap()
            });
            std::thread::sleep(Duration::from_millis(150));
            // A change of the project builds nothing: the next grain is of it.
            let mut changed = (*project).clone();
            changed.timelines[0].tracks[0].audio.strip.fader_db = -1.0;
            assert_eq!(playback.changed(&Arc::new(changed), 0), Heard::Same);
        }
        assert_eq!(output.session(), session, "one session");
        playback.stop();
        output.stop();
        drop(output);
        let left = captured(&path, 1);
        let grains = sounding(&left);
        println!("grains: {grains:?}");
        assert_eq!(grains.len(), 4);
        for (start, length) in grains {
            assert!(length.abs_diff(2_880) <= 1, "{length}");
            let measured = frequency(&left[start + 240..start + length - 240]);
            assert!((measured - 440.0).abs() < 3.0, "{measured}");
        }
    }

    #[test]
    fn a_graph_late_after_a_change_of_speed_plays_on_at_that_speed_from_where_it_is() {
        // T18's review: an ordinary graph that came too late after a change of speed put the
        // warp back to the one before the change, and the sound jumped.
        let mut output = Output::open(&Kind::Null).unwrap();
        let mut playback = Playback::with_opener(slow_to_open);
        SLOW_TO_OPEN.store(0, Ordering::Release);
        let project = Project::sample();
        playback
            .start(&mut output, Arc::new(project.clone()), 0, (0, 1))
            .unwrap();
        let mut speeds = Vec::new();
        let mut poll = |playback: &mut Playback| {
            if let Some(Event::Speed { at, speed }) = playback.poll() {
                speeds.push((at, speed));
            }
        };
        wait("the first graph", || {
            poll(&mut playback);
            playback
                .current
                .as_ref()
                .is_some_and(|current| current.handed)
        });
        assert!(playback.set_speed(2));
        wait("the change of speed", || {
            poll(&mut playback);
            playback
                .current
                .as_ref()
                .is_some_and(|current| current.speed == 2 && !current.pending)
        });
        let warp = playback.current.as_ref().unwrap().warps.last().unwrap().1;
        // An ordinary graph, slow to build: it comes too late and is built again.
        SLOW_TO_OPEN.store(200, Ordering::Release);
        let mut equalized = project.clone();
        equalized.timelines[0].tracks[1].audio.strip.eq.on = true;
        assert_eq!(playback.changed(&Arc::new(equalized), 0), Heard::Swap);
        wait("a graph too late", || {
            poll(&mut playback);
            playback.mix_thread().0 > 0
        });
        SLOW_TO_OPEN.store(0, Ordering::Release);
        wait("the graph built again swapped in", || {
            poll(&mut playback);
            playback.counts().1 >= 2
                && playback
                    .current
                    .as_ref()
                    .is_some_and(|current| !current.pending)
        });
        std::thread::sleep(Duration::from_millis(200));
        poll(&mut playback);
        // Still the one change of speed, and the timeline where that change put it.
        assert_eq!(speeds.len(), 1, "{speeds:?}");
        assert_eq!(
            playback.current.as_ref().unwrap().warps.last().unwrap().1,
            warp
        );
        playback.stop();
    }

    /// The sample project with its track of sound only, its clip the 4 s of the clicks of the
    /// tests (a beep of 1 kHz for 10 ms at the start of each second); none where the media
    /// cannot be made.
    fn clicks() -> Option<Project> {
        let made = crate::test_media::media(crate::test_media::Sample::Clicks)?;
        let mut project = a_tone_of_two_seconds()?;
        let facts = columbina_media::probe(&made.path).unwrap();
        project.media[0].path = made.path.to_string_lossy().into_owned();
        project.media[0].facts = facts.clone();
        let clip = &mut project.timelines[0].tracks[0].clips[0];
        clip.source = columbina_model::Source::Media {
            media: "m1".into(),
            source_in: columbina_model::Time {
                ticks: 0,
                scale: facts.duration.scale,
            },
        };
        clip.length = 100;
        Some(project)
    }

    #[test]
    fn a_grain_built_slowly_still_starts_at_the_frame_dragged_to() {
        let Some(project) = clicks() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("heard.f32");
        let mut output = Output::open(&Kind::Capture(path.clone())).unwrap();
        let mut playback = Playback::with_opener(slow_to_open);
        SLOW_TO_OPEN.store(0, Ordering::Release);
        let project = Arc::new(project);
        let first = 24_000;
        playback
            .scrub(&mut output, Arc::clone(&project), 0, first)
            .unwrap();
        wait("the first grain", || {
            playback.poll();
            output.played(Instant::now()).is_some()
        });
        // Each next grain is slow to build: 60 ms for each reader it opens.
        SLOW_TO_OPEN.store(60, Ordering::Release);
        let mut grains = Vec::new();
        for (n, place) in [47_520, 95_520].into_iter().enumerate() {
            playback
                .scrub(&mut output, Arc::clone(&project), 0, place)
                .unwrap();
            wait("the grain", || {
                playback.poll();
                playback.counts().1 == u64::try_from(n + 1).unwrap()
            });
            grains.push((playback.scheduled(), place));
            std::thread::sleep(Duration::from_millis(200));
        }
        SLOW_TO_OPEN.store(0, Ordering::Release);
        playback.stop();
        output.stop();
        drop(output);
        let left = captured(&path, 1);
        for (at, place) in grains {
            // The beep at 1 s, 2 s of the media: 480 samples into a grain from 990 ms, 1990 ms.
            let start = usize::try_from(at - first).unwrap();
            let beep = (start..start + 2_880)
                .find(|&n| left[n].abs() > 0.01)
                .unwrap();
            println!(
                "the grain of {place} at {start}: its beep {} samples in",
                beep - start
            );
            assert_eq!(beep - start, 480, "the grain of {place}");
        }
    }

    #[test]
    fn grains_of_another_rate_of_the_timeline_start_a_session_again() {
        let Some(project) = a_tone_of_two_seconds() else {
            return;
        };
        let mut output = Output::open(&Kind::Null).unwrap();
        let mut playback = Playback::new();
        playback
            .scrub(&mut output, Arc::new(project.clone()), 0, 4_800)
            .unwrap();
        let session = output.session();
        let mut other = project;
        other.timelines[0].sample_rate = 44_100;
        assert_eq!(playback.changed(&Arc::new(other.clone()), 0), Heard::Again);
        playback
            .scrub(&mut output, Arc::new(other), 0, 4_410)
            .unwrap();
        assert_eq!(output.session(), session + 1);
        assert!(playback.scrubbing());
        playback.stop();
    }
}
