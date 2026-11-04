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

use columbina_audio::feed::{ClipFeed, FeedError};
use columbina_audio::graph::{node_of, Controls, Graph, Maker, Options, Owner, Purpose};
use columbina_audio::play::device::{self, Feeding, Order, Player};
use columbina_audio::play::engine::{Command, Engine, Retired, Run, Session, MOST_BLOCK};
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
}

impl Sim {
    /// A session of `project` from sample `from`, its clips sounding `tone`, its effects made by
    /// `maker`; nothing played yet.
    pub fn start(project: &Project, from: i64, tone: Tone, maker: Maker) -> Sim {
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
        };
        sim.session(project, from);
        sim
    }

    /// A session of `project` with [`node_of`]'s effects.
    pub fn of(project: &Project, from: i64, tone: Tone) -> Sim {
        Sim::start(project, from, tone, node_of)
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
        let mut tone = self.tone;
        graph.start(at, &mut tone).unwrap();
        let (feed, mut fetch) = ring_feed(&graph, graph.reads_at(), Arc::clone(&self.counters));
        while fetch.pass(&mut tone, 64) > 0 {}
        self.fetches.push((fetch, tone));
        self.controls = Some(graph.controls());
        Box::new(Run::new(graph, feed, None, i64::MAX))
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
        self.commands
            .push(Command::Swap {
                epoch: self.epoch,
                run,
                at,
            })
            .unwrap();
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
