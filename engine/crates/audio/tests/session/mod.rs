//! What the tests of playback (T5) share: projects, a feed of tones, and a session of the thread
//! of the mix and the callback of a device run on a clock of their own, a run of the device at a
//! time ([`Sim`]): the reading ahead is done between the runs, off the two threads of the sound,
//! so that a run is the same on every machine.

#![allow(
    dead_code,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "each test binary uses some of these; samples and levels of tests"
)]

use std::ops::Range;
use std::sync::Arc;

use columbina_audio::constants::{samples as of_seconds, SCRUB_FADE, SCRUB_GRAIN};
use columbina_audio::feed::{ClipFeed, FeedError};
use columbina_audio::graph::{node_of, Controls, Graph, Maker, Options, Owner, Purpose, Warp};
use columbina_audio::play::device::{self, Feeding, Order, Player};
use columbina_audio::play::engine::{Command, Engine, Grain, Retired, Run, Session, MOST_BLOCK};
use columbina_audio::play::fetcher::{ring_feed, Fetch};
use columbina_audio::play::Counters;
use columbina_audio::rt::{messages, samples, MessageConsumer, MessageProducer};
use columbina_audio::stream::Resampler;
use columbina_model::Project;

/// The rate of the timelines and of the device.
pub const RATE: u32 = 48_000;
/// A run of the device: 10 ms.
pub const PERIOD: usize = 480;
/// How long after it is asked for a run is heard: 20 ms.
pub const LATENCY: i64 = 20_000_000;

/// The sound of every clip: a sine of `hz` (a level, at 0 Hz) of `amplitude`, by the sample of
/// its media, on every channel.
#[derive(Clone, Copy, Debug)]
pub struct Tone {
    pub hz: f64,
    pub amplitude: f32,
}

impl Tone {
    /// The tone at sample `sample` of a media.
    pub fn at(&self, sample: i64) -> f32 {
        if self.hz == 0.0 {
            return self.amplitude;
        }
        let t = sample as f64 / f64::from(RATE);
        (f64::from(self.amplitude) * (std::f64::consts::TAU * self.hz * t).sin()) as f32
    }
}

impl ClipFeed for Tone {
    fn read(
        &mut self,
        _: usize,
        range: Range<i64>,
        out: &mut [&mut [f32]],
    ) -> Result<(), FeedError> {
        for out in out.iter_mut() {
            for (n, sample) in out.iter_mut().enumerate() {
                *sample = self.at(range.start + n as i64);
            }
        }
        Ok(())
    }
}

/// The sample project with its track of sound only: A1, with clip c2 of 4 s of media m1, out to
/// B1, its clip made 30 s long.
pub fn one_track() -> Project {
    let mut project = Project::sample();
    let timeline = &mut project.timelines[0];
    timeline
        .tracks
        .retain(|track| track.kind == columbina_model::TrackKind::Audio);
    timeline.tracks[0].clips[0].length = 30 * 25;
    project.media[0].facts.duration = columbina_model::Time {
        ticks: 60 * 48_000,
        scale: 48_000,
    };
    project
}

/// A session made and not yet sent ([`Sim::prepare`]).
pub struct Prepared {
    epoch: u64,
    session: Box<Session>,
    ring: columbina_audio::rt::SampleConsumer,
}

/// A session played on a clock of its own. See the module.
pub struct Sim {
    pub engine: Engine,
    pub commands: MessageProducer<Command>,
    retired: MessageConsumer<Retired>,
    pub counters: Arc<Counters>,
    pub player: Player,
    pub control: device::Control,
    fetches: Vec<(Fetch, Tone)>,
    pub tone: Tone,
    pub maker: Maker,
    /// The controls of the graph built last.
    pub controls: Option<Controls>,
    /// Every frame given to the device, left and right.
    pub played: Vec<f32>,
    run: Vec<f32>,
    now: i64,
    /// The graphs sent back as too late.
    pub late: usize,
    epoch: u64,
    /// Where the samples out of the graphs it builds lie on the timeline (T18): the identity, but
    /// at a speed.
    pub warp: Warp,
    /// The grain that the graph it builds next plays, where it plays one (T18).
    grain: Option<Grain>,
}

impl Sim {
    /// A session of `project` from sample `from`, its clips sounding `tone`, its effects made by
    /// `maker`; nothing played yet.
    pub fn start(project: &Project, from: i64, tone: Tone, maker: Maker) -> Sim {
        Sim::through(project, from, tone, (maker, Warp::IDENTITY, None))
    }

    /// A session of `project` played through `warp` from sample `from` out, its first graph
    /// playing only `grain` where there is one; nothing played yet.
    pub fn through(
        project: &Project,
        from: i64,
        tone: Tone,
        (maker, warp, grain): (Maker, Warp, Option<Grain>),
    ) -> Sim {
        let counters = Arc::new(Counters::default());
        let (commands, taken) = messages(64);
        let (gone, retired) = messages(64);
        let engine = Engine::new(taken, gone, Arc::clone(&counters));
        let (player, control) = device::player((2, RATE), false, false, None);
        let mut sim = Sim {
            engine,
            commands,
            retired,
            counters,
            player,
            control,
            fetches: Vec::new(),
            tone,
            maker,
            controls: None,
            played: Vec::with_capacity(2 * 60 * RATE as usize),
            run: vec![0.0; 2 * PERIOD],
            now: 0,
            late: 0,
            epoch: 0,
            warp,
            grain,
        };
        sim.session(project, from);
        sim.grain = None;
        sim
    }

    /// A session of `project` played at `speed` from sample `from` of the timeline (T18).
    pub fn at_speed(project: &Project, from: i64, tone: Tone, speed: i64) -> Sim {
        Sim::through(
            project,
            from,
            tone,
            (node_of, Warp::new(speed, from, from), None),
        )
    }

    /// A grain of scrubbing from sample `from` out ([`SCRUB_GRAIN`], faded [`SCRUB_FADE`]).
    pub fn grain_at(from: i64) -> Grain {
        Grain {
            from,
            length: of_seconds(SCRUB_GRAIN, RATE) as i64,
            fade: of_seconds(SCRUB_FADE, RATE) as i64,
        }
    }

    /// A session of scrubbing: its first graph plays the grain of sample `at` of the timeline.
    pub fn scrubbing(project: &Project, at: i64, tone: Tone) -> Sim {
        Sim::through(
            project,
            at,
            tone,
            (node_of, Warp::IDENTITY, Some(Sim::grain_at(at))),
        )
    }

    /// Sends a graph of `project` played at `speed` from where the timeline is `margin` samples
    /// after what is mixed (as the thread that builds places a swap), to be swapped in there;
    /// where. The graphs built after it play at that speed.
    pub fn speed_swap(&mut self, project: &Project, speed: i64, margin: i64) -> i64 {
        let at =
            (self.counters.mixed() + margin).max(self.counters.scheduled().saturating_add(720));
        self.warp = self.warp.with_speed(speed, at);
        self.swap_at(project, at);
        at
    }

    /// Sends a graph of `project` playing the grain of sample `timeline` of the timeline, to be
    /// swapped in `margin` samples after what is mixed; where.
    pub fn grain(&mut self, project: &Project, timeline: i64, margin: i64) -> i64 {
        let at =
            (self.counters.mixed() + margin).max(self.counters.scheduled().saturating_add(720));
        self.warp = Warp::new(1, at, timeline);
        self.grain = Some(Sim::grain_at(at));
        self.swap_at(project, at);
        self.grain = None;
        at
    }

    /// A session of `project` with [`node_of`]'s effects.
    pub fn of(project: &Project, from: i64, tone: Tone) -> Sim {
        Sim::start(project, from, tone, node_of)
    }

    /// Another session of `project` from `from`, made (off the threads of the sound) and not yet
    /// sent ([`Sim::send`]).
    pub fn prepare(&mut self, project: &Project, from: i64) -> Prepared {
        let (producer, consumer) = samples(16_384, 2);
        let rate = project.timelines[0].sample_rate;
        let mut resampler = Resampler::new(rate, RATE, 2, MOST_BLOCK);
        resampler.start(from);
        let run = self.run_of(project, resampler.first_input());
        let epoch = self.epoch + 1;
        let session = Session::new(
            epoch,
            run,
            resampler,
            producer,
            Arc::clone(&self.control.shared),
        );
        Prepared {
            epoch,
            session: Box::new(session),
            ring: consumer,
        }
    }

    /// Sends a session made by [`Sim::prepare`], as a seek does: the session before stopped, the
    /// device given the new ring, the thread of the mix the new graph. Takes no memory.
    pub fn send(&mut self, prepared: Prepared) {
        self.epoch = prepared.epoch;
        self.commands.push(Command::Stop(self.epoch)).unwrap();
        self.control
            .orders
            .push(Order::Play(Feeding {
                generation: self.epoch,
                ring: prepared.ring,
            }))
            .unwrap();
        self.commands
            .push(Command::Start(prepared.session))
            .unwrap();
    }

    /// Starts another session of `project` from `from`, as a seek does: the device is given its
    /// ring at once, the thread of the mix its graph `after` runs of the device later.
    pub fn seek(&mut self, project: &Project, from: i64, after: usize) {
        self.epoch += 1;
        let (producer, consumer) = samples(16_384, 2);
        self.commands.push(Command::Stop(self.epoch)).unwrap();
        self.control
            .orders
            .push(Order::Play(Feeding {
                generation: self.epoch,
                ring: consumer,
            }))
            .unwrap();
        for _ in 0..after {
            self.tick();
        }
        self.begin(project, from, producer);
    }

    fn session(&mut self, project: &Project, from: i64) {
        self.epoch += 1;
        let (producer, consumer) = samples(16_384, 2);
        self.control
            .orders
            .push(Order::Play(Feeding {
                generation: self.epoch,
                ring: consumer,
            }))
            .unwrap();
        self.begin(project, from, producer);
    }

    fn begin(
        &mut self,
        project: &Project,
        from: i64,
        producer: columbina_audio::rt::SampleProducer,
    ) {
        let rate = project.timelines[0].sample_rate;
        let mut resampler = Resampler::new(rate, RATE, 2, MOST_BLOCK);
        resampler.start(from);
        let run = self.run_of(project, resampler.first_input());
        let session = Session::new(
            self.epoch,
            run,
            resampler,
            producer,
            Arc::clone(&self.control.shared),
        );
        self.commands
            .push(Command::Start(Box::new(session)))
            .unwrap();
    }

    /// A graph of `project` started at `at`, its clips read ahead from there.
    pub fn run_of(&mut self, project: &Project, at: i64) -> Box<Run> {
        let options = Options::new(MOST_BLOCK, Purpose::Preview).with_maker(self.maker);
        let mut graph = Graph::build(project, 0, options).unwrap();
        graph.set_warp(self.warp);
        let mut tone = self.tone;
        graph.start(at, &mut tone).unwrap();
        let (feed, fetches) = ring_feed(&graph, graph.reads_at(), Arc::clone(&self.counters));
        for mut fetch in fetches {
            while fetch.pass(&mut tone, 64) > 0 {}
            self.fetches.push((fetch, tone));
        }
        self.controls = Some(graph.controls());
        let run = Run::new(graph, feed, None, i64::MAX);
        Box::new(match self.grain {
            Some(grain) => run.with_grain(grain),
            None => run,
        })
    }

    /// Sends a graph of `project` to be swapped in `margin` samples after what is mixed, or a
    /// crossfade and a block after the one that waits to be swapped in, as the thread that
    /// builds does; where.
    pub fn swap(&mut self, project: &Project, margin: i64) -> i64 {
        let at =
            (self.counters.mixed() + margin).max(self.counters.scheduled().saturating_add(720));
        self.swap_at(project, at);
        at
    }

    /// Sends a graph of `project` to be swapped in at `at`.
    pub fn swap_at(&mut self, project: &Project, at: i64) {
        let run = self.run_of(project, at);
        self.send_swap(run, at);
    }

    /// Sends `run` (made by [`Sim::run_of`]) to be swapped in at `at`. Takes no memory.
    pub fn send_swap(&mut self, run: Box<Run>, at: i64) {
        self.commands
            .push(Command::Swap {
                epoch: self.epoch,
                run,
                at,
            })
            .unwrap();
    }

    /// Tells the thread of the mix to fade out what the session plays (T18's review).
    pub fn fade(&mut self) {
        self.commands.push(Command::Fade(self.epoch)).unwrap();
    }

    /// Sets the control `path` of `owner` of the graph built last to `value`.
    pub fn set(&mut self, owner: &Owner, path: &str, value: f32) {
        let control = self
            .controls
            .as_ref()
            .and_then(|controls| controls.control(owner, path))
            .unwrap();
        self.commands.push(Command::Set(control, value)).unwrap();
    }

    /// Holds `live` in place of the lane `path` of `owner` of the graph built last (T16).
    pub fn live(&mut self, owner: &Owner, path: &str, live: columbina_audio::graph::Live) {
        let key = self
            .controls
            .as_ref()
            .and_then(|controls| controls.lane_key(owner, path))
            .unwrap();
        self.commands.push(Command::Live(key, live)).unwrap();
    }

    /// Glides the control `path` of `owner` of the graph built last to `value` (T16).
    pub fn glide(&mut self, owner: &Owner, path: &str, value: f32, samples: u32) {
        let control = self
            .controls
            .as_ref()
            .and_then(|controls| controls.control(owner, path))
            .unwrap();
        self.commands
            .push(Command::Glide(control, value, samples))
            .unwrap();
    }

    /// What is done off the threads of the sound between runs: the clips read ahead, and what
    /// the thread of the mix let go of dropped.
    pub fn pump(&mut self) {
        for (fetch, tone) in &mut self.fetches {
            while fetch.pass(tone, 64) > 0 {}
        }
        while let Some(retired) = self.retired.pop() {
            if matches!(retired, Retired::Late { .. }) {
                self.late += 1;
            }
        }
        self.fetches.retain(|(fetch, _)| !fetch.abandoned());
    }

    /// One run of the device on the threads of the sound: the mix topped up, and the device
    /// given its run; the frames given kept (in memory taken beforehand).
    pub fn device_run(&mut self) {
        self.engine.step();
        self.player
            .fill(&mut self.run, (self.now, self.now + LATENCY), 0.0, |x| x);
        assert!(self.played.len() + self.run.len() <= self.played.capacity());
        self.played.extend_from_slice(&self.run);
        self.now += 10_000_000;
    }

    /// [`Sim::pump`], then [`Sim::device_run`].
    pub fn tick(&mut self) {
        self.pump();
        self.device_run();
    }

    /// The frames given to the device so far.
    pub fn sent(&self) -> usize {
        self.played.len() / 2
    }

    /// The left side of what was given.
    pub fn left(&self) -> Vec<f32> {
        self.played.iter().step_by(2).copied().collect()
    }
}

/// The largest step from a sample to the next.
pub fn largest_step(signal: &[f32]) -> f32 {
    signal
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).abs())
        .fold(0.0, f32::max)
}

/// A project of 16 tracks of every kind of strip: Dynamics keyed by another track, Duckers keyed
/// by two, equalizers, sends before and after the fader to a bus of 5.1, a VCA, lanes of the
/// fader, the pan, a threshold and an amount, a crossfade, clips at speeds, mono, stereo, 5.1
/// and Adaptive tracks.
#[allow(clippy::too_many_lines, reason = "a step for each kind of strip")]
pub fn sixteen_tracks() -> columbina_model::Project {
    use columbina_model::audio::{BusFormat, TrackFormat};
    use columbina_model::{
        Bus, Lane, LaneKind, LanePoint, Output, Send, SourceRef, Speed, Time, TrackKind, Vca,
    };
    let mut project = columbina_model::Project::sample();
    let timeline = &mut project.timelines[0];
    timeline
        .tracks
        .retain(|track| track.kind == TrackKind::Audio);
    let first = timeline.tracks[0].clone();
    let mut b2 = Bus::first();
    b2.id = "B2".into();
    b2.format = BusFormat::FiveOne;
    b2.strip.outputs = vec![Output {
        bus: "B1".into(),
        on: true,
    }];
    b2.strip.eq.on = true;
    timeline.buses.push(b2);
    timeline.vcas.push(Vca {
        id: "VCA1".into(),
        name: "VCA 1".into(),
        fader_db: -3.0,
        mute: false,
        solo: false,
        vcas: vec![],
        lanes: columbina_model::Lanes::new(),
    });
    let lane = |points: &[(i64, f64)]| Lane {
        points: points
            .iter()
            .map(|&(ticks, value)| LanePoint {
                at: Time {
                    ticks,
                    scale: 48000,
                },
                value,
            })
            .collect(),
        kind: LaneKind::Continuous,
        scale: None,
    };
    timeline.tracks = (0..16)
        .map(|n| {
            let mut track = first.clone();
            track.id = format!("A{n}");
            track.clips[0].id = format!("c{n}");
            track.clips[0].speed = [Speed { num: 1, den: 1 }, Speed { num: 3, den: 2 }][n % 2];
            let audio = &mut track.audio;
            (audio.format, audio.channels) = match n % 4 {
                0 => (TrackFormat::Mono, 1),
                1 => (TrackFormat::Stereo, 2),
                2 => (TrackFormat::FiveOne, 6),
                _ => (TrackFormat::Adaptive, 3),
            };
            audio.strip.eq.on = true;
            audio.strip.dynamics.on = n % 3 == 0;
            audio.strip.dynamics.compressor.on = true;
            if n > 0 && n % 3 == 0 {
                audio.strip.dynamics.compressor.sidechain.on = true;
                audio.strip.dynamics.compressor.sidechain.source =
                    Some(SourceRef::Track(format!("A{}", n - 1)));
            }
            audio.strip.sends.push(Send {
                bus: "B2".into(),
                on: true,
                pre: n % 2 == 0,
                level_db: -6.0,
                pan: columbina_model::Pan {
                    y: -0.5,
                    ..columbina_model::Pan::default()
                },
            });
            audio.strip.vcas = vec!["VCA1".into()];
            // Every fourth track ducked by the two before it (T11), its amount on a lane.
            if n >= 2 && n % 4 == 3 {
                audio.strip.track_fx = Some(columbina_model::audio::fx::Ducker {
                    on: true,
                    sources: vec![
                        SourceRef::Track(format!("A{}", n - 1)),
                        SourceRef::Track(format!("A{}", n - 2)),
                    ],
                    ..Default::default()
                });
                audio.lanes.insert(
                    "track_fx.amount_db".into(),
                    lane(&[(0, 3.0), (96_000, 20.0)]),
                );
            }
            audio
                .lanes
                .insert("fader_db".into(), lane(&[(0, -20.0), (96_000, 0.0)]));
            audio
                .lanes
                .insert("pan.x".into(), lane(&[(0, -1.0), (96_000, 1.0)]));
            audio
                .lanes
                .insert("eq.2.gain_db".into(), lane(&[(0, -6.0), (96_000, 6.0)]));
            track
        })
        .collect();
    project
}
