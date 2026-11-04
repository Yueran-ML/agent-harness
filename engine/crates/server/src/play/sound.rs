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
//! Playing starts, and starts again on a seek, a change of speed, or after the device failed: a
//! new session, whose graph is built in the background while the device plays silence, which is
//! not counted, so that the picture waits for its sound (as phases 1 and 2 did).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use columbina_audio::constants::{samples, GRAPH_SWAP_CROSSFADE, MIX_BLOCK};
use columbina_audio::graph::{retune, Controls, Graph, Options, Purpose};
use columbina_audio::play::device;
use columbina_audio::play::engine::{Command, Engine, Retired, Run, Session, MOST_BLOCK};
use columbina_audio::play::fetcher::{ring_feed, Fetcher};
use columbina_audio::play::thread::MixThread;
use columbina_audio::play::Counters;
use columbina_audio::rt::{messages, MessageConsumer, MessageProducer, SampleProducer};
use columbina_audio::stream::Resampler;
use columbina_media::AudioFormat;
use columbina_model::Project;
use serde_json::json;

use super::mix::{end_of, Feed};
use crate::audio::Output;
use crate::log::log;

/// How far ahead of what is mixed a new graph is swapped in, at the least.
const SWAP_AHEAD: f64 = 0.150;

/// How often the thread that builds drops what the thread of the mix let go of, when it has
/// nothing to build.
const BUILDER_NAP: Duration = Duration::from_millis(20);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the thread that builds is told.
enum Job {
    /// Build the graph of a session, started at `from`, into `out`.
    Start {
        epoch: u64,
        project: Arc<Project>,
        timeline: usize,
        from: i64,
        format: AudioFormat,
        out: SampleProducer,
        device: Arc<device::Shared>,
    },
    /// Build a graph of the newest project of session `epoch` to swap in, `ahead` seconds after
    /// what is mixed.
    Swap { epoch: u64, ahead: f64 },
}

/// A graph handed to the thread of the mix: what it was built from, and its controls.
struct Built {
    epoch: u64,
    project: Arc<Project>,
    timeline: usize,
    controls: Controls,
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
}

/// The threads of the sound of a timeline monitor, made when it first plays.
struct Threads {
    /// The ring of commands to the thread of the mix, which this thread and the thread that
    /// builds both write to (they lock it between them; the thread of the mix never does).
    commands: Arc<Mutex<MessageProducer<Command>>>,
    jobs: mpsc::Sender<Job>,
    shared: Arc<Shared>,
    counters: Arc<Counters>,
    _mix: MixThread,
}

impl Threads {
    fn spawn() -> Result<Threads, String> {
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
            _mix: mix,
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
            while let Some(retired) = self.retired.pop() {
                if let Retired::Late { epoch, .. } = retired {
                    late = Some(epoch);
                }
            }
            let now = self.shared.epoch.load(Ordering::Acquire);
            if late == Some(now) {
                self.swap(now, SWAP_AHEAD * 2.0);
            }
            match job {
                Some(Job::Start {
                    epoch,
                    project,
                    timeline,
                    from,
                    format,
                    out,
                    device,
                }) if epoch == self.shared.epoch.load(Ordering::Acquire) => {
                    self.start(epoch, (project, timeline, from), format, (out, device));
                }
                Some(Job::Swap { epoch, ahead })
                    if epoch == self.shared.epoch.load(Ordering::Acquire) =>
                {
                    self.swap(epoch, ahead);
                }
                _ => {}
            }
        }
    }

    fn send(&self, command: Command) {
        if lock(&self.commands).push(command).is_err() {
            log(
                "sound_failed",
                &json!({"message": "the thread of the mix takes no more commands"}),
            );
        }
    }

    /// A graph of `project` started at `at`, its clips read ahead from there; and its controls.
    /// One to swap in (`catch_up`) is caught up to a little after what is mixed where the mix
    /// went on past that while it was built.
    fn run_of(
        &self,
        project: &Arc<Project>,
        timeline: usize,
        at: i64,
        catch_up: bool,
    ) -> Result<(Box<Run>, Controls), String> {
        let mut graph = Graph::build(
            project,
            timeline,
            Options::new(MOST_BLOCK, Purpose::Preview),
        )
        .map_err(|error| error.to_string())?;
        for clip in graph.unplayable() {
            log("sound_of_a_clip_unplayable", &json!({ "clip": clip }));
        }
        // Playback goes on where a clip cannot be read: that clip is silent, and told of once.
        let mut feed = Feed::new(Arc::clone(project), timeline, graph.clips(), true);
        let _ = graph.start(at, &mut feed);
        // The mix may have gone on while the graph was built and started: it is caught up to a
        // little after what is mixed, its sound thrown away.
        let rate = graph.rate();
        let guard = i64::try_from(samples(SWAP_AHEAD / 2.0, rate)).unwrap_or(0);
        while catch_up && self.counters.mixed() + guard > graph.at() {
            let wanted = self.counters.mixed() + 2 * guard - graph.at();
            let frames = usize::try_from(wanted).unwrap_or(1).clamp(1, MOST_BLOCK);
            let _ = graph.process(&mut feed, frames);
        }
        let (ring, mut fetch) = ring_feed(&graph, graph.reads_at(), Arc::clone(&self.counters));
        while fetch.pass(&mut feed, 64) > 0 {}
        let fetcher = Fetcher::spawn(fetch, Box::new(feed))
            .map_err(|error| format!("the reader ahead cannot be started: {error}"))?;
        let controls = graph.controls();
        let end = end_of(&project.timelines[timeline]);
        Ok((
            Box::new(Run::new(graph, ring, Some(fetcher), end)),
            controls,
        ))
    }

    /// Builds the session `epoch` and hands it to the thread of the mix.
    fn start(
        &self,
        epoch: u64,
        (project, timeline, from): (Arc<Project>, usize, i64),
        format: AudioFormat,
        (out, device): (SampleProducer, Arc<device::Shared>),
    ) {
        let rate = project.timelines[timeline].sample_rate;
        let built = (|| {
            if rate == 0 {
                return Err("the timeline has no rate of sound".to_owned());
            }
            let mut resampler =
                Resampler::new(rate, format.rate, usize::from(format.channels), MOST_BLOCK);
            resampler.start(from);
            // A session starts where it is told, however long its graph took to build.
            let (run, controls) =
                self.run_of(&project, timeline, resampler.first_input(), false)?;
            let session = Session::new(epoch, run, resampler, out, device);
            Ok((session, controls))
        })();
        match built {
            Ok((session, controls)) => {
                if epoch != self.shared.epoch.load(Ordering::Acquire) {
                    return;
                }
                self.send(Command::Start(Box::new(session)));
                *lock(&self.shared.built) = Some(Built {
                    epoch,
                    project,
                    timeline,
                    controls,
                });
            }
            Err(message) => {
                log(
                    "sound_unavailable",
                    &json!({"monitor": "timeline", "message": message}),
                );
                *lock(&self.shared.failed) = Some((epoch, message));
            }
        }
    }

    /// Builds a graph of the newest project of session `epoch` and sends it to be swapped in,
    /// `ahead` seconds after what is mixed, or as soon after as it is ready.
    fn swap(&self, epoch: u64, ahead: f64) {
        let Some((latest, project, timeline)) = lock(&self.shared.latest).clone() else {
            return;
        };
        if latest != epoch {
            return;
        }
        let rate = project.timelines[timeline].sample_rate;
        // Ahead of what is mixed, and a crossfade and a block after a graph that waits to be
        // swapped in, so that it follows that one (`Engine`: changes that keep coming are heard
        // as each graph is ready).
        let after = self.counters.scheduled()
            + i64::try_from(samples(GRAPH_SWAP_CROSSFADE + MIX_BLOCK, rate)).unwrap_or(0);
        let at =
            (self.counters.mixed() + i64::try_from(samples(ahead, rate)).unwrap_or(0)).max(after);
        let built = self.run_of(&project, timeline, at, true);
        match built {
            Ok((run, controls)) => {
                if epoch != self.shared.epoch.load(Ordering::Acquire) {
                    return;
                }
                let at = run.graph().at();
                self.send(Command::Swap { epoch, run, at });
                *lock(&self.shared.built) = Some(Built {
                    epoch,
                    project,
                    timeline,
                    controls,
                });
            }
            Err(message) => log(
                "sound_failed",
                &json!({"monitor": "timeline", "message": message}),
            ),
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

/// What plays: the project its graph was built from, or that settings brought it to, and the
/// newest project.
struct Current {
    base: Arc<Project>,
    timeline: usize,
    controls: Option<Controls>,
    /// A graph is being built (the session's first, or one to swap in).
    pending: bool,
    /// The project changed while it was.
    dirty: bool,
    latest: (Arc<Project>, usize),
}

/// The sound of a timeline monitor as it plays. See the module.
pub(crate) struct Playback {
    threads: Option<Threads>,
    epoch: u64,
    current: Option<Current>,
}

impl Playback {
    /// Nothing playing; the threads are made when it first plays.
    pub(crate) fn new() -> Self {
        Playback {
            threads: None,
            epoch: 0,
            current: None,
        }
    }

    fn threads(&mut self) -> Result<&Threads, String> {
        if self.threads.is_none() {
            self.threads = Some(Threads::spawn()?);
        }
        Ok(self.threads.as_ref().expect("made above"))
    }

    /// Plays timeline `timeline` of `project` from sample `from` on `output`, in place of what
    /// played: the device plays silence, not counted, until the graph is built and started.
    ///
    /// # Errors
    ///
    /// A thread cannot be started, or the device cannot.
    pub(crate) fn start(
        &mut self,
        output: &mut Output,
        project: Arc<Project>,
        timeline: usize,
        from: i64,
    ) -> Result<(), String> {
        self.epoch += 1;
        let epoch = self.epoch;
        self.current = None;
        let threads = self.threads()?;
        threads.shared.epoch.store(epoch, Ordering::Release);
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

    /// Takes what the thread that builds has done: the controls of the graph it handed over, and
    /// what changed while it was built. Why the session could not be built, if it could not.
    pub(crate) fn poll(&mut self) -> Option<String> {
        let threads = self.threads.as_ref()?;
        let current = self.current.as_mut()?;
        if let Some((epoch, message)) = lock(&threads.shared.failed).take() {
            if epoch == self.epoch {
                return Some(message);
            }
        }
        let built = lock(&threads.shared.built).take();
        let built = built.filter(|built| built.epoch == self.epoch)?;
        current.base = built.project;
        current.timeline = built.timeline;
        current.controls = Some(built.controls);
        current.pending = false;
        if std::mem::take(&mut current.dirty) {
            let (project, timeline) = current.latest.clone();
            self.changed(&project, timeline);
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
        let from = threads.counters.mixed();
        let clips = super::timeline::same_clips(&current.base, was, project, now, from);
        if let (true, Some(sets)) = (clips, &sets) {
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

    /// The reads of a clip that came before it was read ahead, and the graphs swapped in, so far.
    pub(crate) fn counts(&self) -> (u64, u64) {
        self.threads.as_ref().map_or((0, 0), |threads| {
            (threads.counters.underruns(), threads.counters.swaps())
        })
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
        let deadline = Instant::now() + Duration::from_secs(20);
        while !until() {
            assert!(Instant::now() < deadline, "{what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_setting_is_sent_to_the_graph_another_change_swaps_one_in_and_the_rates_start_again() {
        let mut output = Output::open(&Kind::Null).unwrap();
        let mut playback = Playback::new();
        let project = Project::sample();
        playback
            .start(&mut output, Arc::new(project.clone()), 0, 0)
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
        // A clip's gain: another graph, swapped in.
        let mut gained = muted.clone();
        gained.timelines[0].tracks[1].clips[0].gain_db = (-3.0).into();
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
}
