//! Playback (plan T5): the thread of the mix and the callback of the device, run on a clock of
//! their own ([`session::Sim`]), and on threads of their own in real time. A change of a setting
//! is heard within the sound kept ahead of the device and a block; two hundred moves of a fader
//! play on with no click; a new graph is swapped in at its sample, crossfaded, its time kept, and
//! one that comes too late is sent back; a seek plays a new session with nothing short; a media
//! slow to read never holds up the mix; and sixteen tracks play thirty seconds on threads with
//! nothing short, the mix within half of a core.

mod session;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use columbina_audio::constants::{samples, MIX_BLOCK, OUTPUT_AHEAD};
use columbina_audio::feed::{ClipFeed, FeedError};
use columbina_audio::graph::{node_of, Context, Graph, Made, Options, Owner, Purpose};
use columbina_audio::node::{Block, Node, ParamId, Spec, Unsupported};
use columbina_audio::play::device::{self, Feeding, Order};
use columbina_audio::play::engine::{Command, Engine, Run, Session, MOST_BLOCK};
use columbina_audio::play::fetcher::{ring_feed, Fetcher};
use columbina_audio::play::thread::MixThread;
use columbina_audio::play::Counters;
use columbina_audio::rt::{messages, samples as ring};
use columbina_audio::stream::Resampler;
use columbina_audio::testing::Rng;
use columbina_model::audio::fx::GainFx;
use columbina_model::{Effect, Insert, Project};

use session::{largest_step, one_track, Sim, Tone, PERIOD, RATE};

/// A tone at a quarter of full scale.
const TONE: Tone = Tone {
    hz: 440.0,
    amplitude: 0.5,
};

/// A level of a half.
const LEVEL: Tone = Tone {
    hz: 0.0,
    amplitude: 0.5,
};

fn a1() -> Owner {
    Owner::Track("A1".into())
}

#[test]
#[allow(
    clippy::cast_precision_loss,
    clippy::float_cmp,
    reason = "milliseconds of a test; the first sample that differs at all"
)]
fn a_fader_moved_is_heard_within_the_sound_ahead_and_a_block_and_gone_within_60_ms() {
    let ahead = samples(OUTPUT_AHEAD, RATE) + samples(MIX_BLOCK, RATE);
    let mut sim = Sim::of(&one_track(), 0, LEVEL);
    for _ in 0..50 {
        sim.tick();
    }
    let left = sim.left();
    let steady = *left.last().unwrap();
    assert!((steady - 0.5).abs() < 1e-6, "{steady}");
    // The fader to −∞ while it plays: what was given before is as it was.
    let sent = sim.sent();
    sim.set(&a1(), "fader_db", -100.0);
    for _ in 0..20 {
        sim.tick();
    }
    let left = sim.left();
    let onset = (sent..left.len()).find(|&n| left[n] != steady).unwrap() - sent;
    let gone = (sent..left.len()).find(|&n| left[n].abs() < 1e-3).unwrap() - sent;
    println!(
        "a fader to −∞: first heard {onset} frames ({:.1} ms) after it was sent, below −60 dBFS \
         after {gone} ({:.1} ms)",
        onset as f64 / 48.0,
        gone as f64 / 48.0
    );
    assert!(onset <= ahead, "{onset} > {ahead}");
    assert!(gone <= samples(0.060, RATE), "{gone}");
    assert!(left[left.len() - 100..].iter().all(|&x| x == 0.0));
    assert_eq!(sim.control.shared.short(), 0);
    assert_eq!(sim.counters.underruns(), 0);
}

#[test]
fn two_hundred_moves_of_a_fader_play_on_without_a_click() {
    let mut still = Sim::of(&one_track(), 0, TONE);
    let mut moved = Sim::of(&one_track(), 0, TONE);
    let mut rng = Rng::new(5);
    for tick in 0..420 {
        if tick >= 10 && tick % 2 == 0 {
            #[allow(clippy::cast_precision_loss, reason = "a fader in decibels")]
            let db = -(rng.range(0, 1200) as f32) / 100.0;
            moved.set(&a1(), "fader_db", db);
        }
        still.tick();
        moved.tick();
    }
    let normal = largest_step(&still.left());
    let steps = largest_step(&moved.left());
    println!("the largest step: {steps} moved, {normal} still");
    assert!(steps <= normal * 1.1, "{steps} > {normal} · 1.1");
    assert_ne!(still.left(), moved.left());
    assert_eq!(moved.control.shared.short(), 0);
    assert_eq!(moved.counters.underruns(), 0);
}

/// The project with a Gain of `db` on A1.
fn with_gain(db: f64) -> Project {
    let mut project = one_track();
    project.timelines[0].tracks[0]
        .audio
        .strip
        .inserts
        .push(Insert {
            id: "fx1".into(),
            on: true,
            effect: Effect::Gain(GainFx { gain_db: db }),
        });
    project
}

/// A node that delays by 64 samples, for an insert of Gain.
fn delays(effect: &Effect, _: &Context) -> Option<Made> {
    let Effect::Gain(_) = effect else {
        return None;
    };
    Some(Made {
        reading: None,
        node: Box::new(columbina_audio::testing::Delay::new(64)),
        param: |_| None,
        settle: 0,
        switched: false,
    })
}

#[test]
fn a_new_graph_is_swapped_in_at_its_sample_on_time_and_without_a_click() {
    // The same graph again: the sound goes on as if nothing were swapped.
    let mut reference = Sim::of(&one_track(), 0, TONE);
    let mut swapped = Sim::of(&one_track(), 0, TONE);
    for tick in 0..80 {
        if tick == 30 {
            let at = swapped.swap(&one_track(), 2_400);
            assert!(at > 0);
        }
        reference.tick();
        swapped.tick();
    }
    assert_eq!(swapped.counters.swaps(), 1);
    let (want, got) = (reference.left(), swapped.left());
    let most = want
        .iter()
        .zip(&got)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f32::max);
    assert!(most < 1e-6, "{most}");

    // A graph that sounds 6 dB lower, and one that is 64 samples late inside: from the end of
    // the crossfade each sounds as it does played from the start, and the swap makes no click.
    for (project, maker) in [
        (with_gain(-6.0), node_of as columbina_audio::graph::Maker),
        (with_gain(-6.0), delays),
    ] {
        let mut before = Sim::start(&one_track(), 0, TONE, maker);
        let mut after = Sim::start(&project, 0, TONE, maker);
        let mut sim = Sim::start(&one_track(), 0, TONE, maker);
        let mut at = 0;
        for tick in 0..80 {
            if tick == 30 {
                at = usize::try_from(sim.swap(&project, 2_400)).unwrap();
            }
            before.tick();
            after.tick();
            sim.tick();
        }
        assert_eq!(sim.counters.swaps(), 1);
        let (got, want, old) = (sim.left(), after.left(), before.left());
        let fade = samples(0.010, RATE);
        let differs = |from: usize, to: usize, other: &[f32]| {
            got[from..to]
                .iter()
                .zip(&other[from..to])
                .map(|(a, b)| (a - b).abs())
                .fold(0.0, f32::max)
        };
        assert!(differs(at + fade, got.len(), &want) < 1e-6);
        assert!(differs(0, at, &old) < 1e-6);
        let steps = largest_step(&got);
        let normal = largest_step(&old);
        assert!(steps <= normal * 1.1, "{steps} > {normal} · 1.1");
        assert_eq!(sim.control.shared.short(), 0);
        assert_eq!(sim.counters.underruns(), 0);
    }
}

#[test]
fn a_graph_that_comes_after_its_sample_was_mixed_is_sent_back_and_the_old_plays_on() {
    let mut reference = Sim::of(&one_track(), 0, TONE);
    let mut sim = Sim::of(&one_track(), 0, TONE);
    for tick in 0..60 {
        if tick == 30 {
            let at = sim.counters.mixed() - 100;
            sim.swap_at(&with_gain(-6.0), at);
        }
        reference.tick();
        sim.tick();
    }
    assert_eq!(sim.counters.swaps(), 0);
    assert_eq!(sim.counters.late(), 1);
    assert_eq!(sim.late, 1);
    assert_eq!(sim.left(), reference.left());
}

#[test]
fn a_seek_plays_a_session_built_off_the_threads_of_the_sound_with_nothing_short() {
    let mut sim = Sim::of(&one_track(), 0, TONE);
    for _ in 0..30 {
        sim.tick();
    }
    let before = sim.sent();
    // Its graph comes four runs of the device after the device was given its ring.
    sim.seek(&one_track(), 96_000, 4);
    for _ in 0..40 {
        sim.tick();
    }
    let left = sim.left();
    let first = (before..left.len()).find(|&n| left[n] != 0.0).unwrap();
    let reference = Sim::of(&one_track(), 96_000, TONE);
    let mut reference = reference;
    for _ in 0..40 {
        reference.tick();
    }
    let want = reference.left();
    let got = &left[first..];
    // From the seek, the very sound of a session started there (its first sample is 0).
    assert_eq!(got[..2000], want[..2000]);
    assert_eq!(
        sim.control.shared.short(),
        0,
        "silence before the sound is not short"
    );
    assert_eq!(sim.counters.underruns(), 0);
}

/// A tone whose first piece takes `stall` to read, as a drive that spins up does, and each piece
/// after it `wait`.
struct Slow {
    tone: Tone,
    stall: Duration,
    wait: Duration,
}

impl ClipFeed for Slow {
    fn read(
        &mut self,
        clip: usize,
        range: std::ops::Range<i64>,
        out: &mut [&mut [f32]],
    ) -> Result<(), FeedError> {
        std::thread::sleep(std::mem::replace(&mut self.stall, self.wait));
        self.tone.read(clip, range, out)
    }
}

#[test]
fn a_media_slow_to_read_never_holds_up_the_mix() {
    let project = one_track();
    let counters = Arc::new(Counters::default());
    let (mut commands, taken) = messages(64);
    let (gone, _retired) = messages(64);
    let mut engine = Engine::new(taken, gone, Arc::clone(&counters));
    let (mut player, mut control) = device::player((2, RATE), false, false, None);
    let mut graph = Graph::build(&project, 0, Options::new(MOST_BLOCK, Purpose::Preview)).unwrap();
    let mut tone = TONE;
    graph.start(0, &mut tone).unwrap();
    let (feed, readers) = ring_feed(&graph, graph.reads_at(), Arc::clone(&counters));
    // The first piece takes 300 ms to read, and each after it 10 ms (a piece is 43 ms).
    let fetcher = Fetcher::spawn(
        readers,
        vec![Box::new(Slow {
            tone: TONE,
            stall: Duration::from_millis(300),
            wait: Duration::from_millis(10),
        })],
    )
    .unwrap();
    let mut resampler = Resampler::new(RATE, RATE, 2, MOST_BLOCK);
    resampler.start(0);
    let (producer, consumer) = ring(16_384, 2);
    control
        .orders
        .push(Order::Play(Feeding {
            generation: 1,
            ring: consumer,
        }))
        .unwrap();
    let run = Box::new(Run::new(graph, feed, Some(fetcher), i64::MAX));
    let session = Session::new(1, run, resampler, producer, Arc::clone(&control.shared));
    commands.push(Command::Start(Box::new(session))).unwrap();
    let mut out = vec![0.0; 2 * PERIOD];
    let mut longest = Duration::ZERO;
    let mut heard = false;
    for n in 0..100 {
        let started = Instant::now();
        engine.step();
        longest = longest.max(started.elapsed());
        player.fill(&mut out, (n * 10_000_000, n * 10_000_000), 0.0, |x| x);
        heard |= out.iter().any(|&x| x != 0.0);
        // As fast as a device takes its runs.
        std::thread::sleep(Duration::from_millis(10));
    }
    println!(
        "slow media: the longest step {longest:?}, {} reads short",
        counters.underruns()
    );
    assert!(longest < Duration::from_millis(20), "{longest:?}");
    assert!(counters.underruns() > 0);
    assert!(heard, "what was read is heard");
    assert_eq!(control.shared.short(), 0, "the device is never short");
}

/// A node that takes `cost` of the processor for each block, and passes its sound on: a stand-in
/// for an equalizer, Dynamics and noise reduction until they are measured themselves (plan T5).
struct Costly {
    cost: Duration,
}

impl Node for Costly {
    fn prepare(&mut self, _: &Spec) -> Result<(), Unsupported> {
        Ok(())
    }
    fn process(&mut self, _: &Block, input: &[&[f32]], output: &mut [&mut [f32]]) {
        let started = Instant::now();
        for (output, input) in output.iter_mut().zip(input) {
            output.copy_from_slice(input);
        }
        while started.elapsed() < self.cost {
            std::hint::spin_loop();
        }
    }
    fn latency(&self) -> u32 {
        0
    }
    fn settle(&self) -> u32 {
        0
    }
    fn reset(&mut self) {}
    fn set(&mut self, _: ParamId, _: f32) {}
}

/// The cost of each costly node for a block of the mix.
const COST: Duration = Duration::from_micros(20);

fn costly(effect: &Effect, _: &Context) -> Option<Made> {
    let Effect::Gain(_) = effect else {
        return None;
    };
    Some(Made {
        reading: None,
        node: Box::new(Costly { cost: COST }),
        param: |_| None,
        settle: 0,
        switched: false,
    })
}

/// Sixteen stereo tracks, each with a costly node, out to B1 and sending to B2, which goes out
/// to B1 (plan T5: 16 tracks and 2 buses). The clips last half a minute more than `seconds` (a
/// minute at least), so that a longer run reads its sound to its end.
fn sixteen_costly_tracks(seconds: i64) -> Project {
    let length = seconds.max(30) + 30;
    let mut project = one_track();
    let timeline = &mut project.timelines[0];
    let mut b2 = columbina_model::Bus::first();
    b2.id = "B2".into();
    b2.strip.outputs = vec![columbina_model::Output {
        bus: "B1".into(),
        on: true,
    }];
    timeline.buses.push(b2);
    let first = timeline.tracks[0].clone();
    timeline.tracks = (0..16)
        .map(|n| {
            let mut track = first.clone();
            track.id = format!("A{n}");
            track.clips[0].id = format!("c{n}");
            track.clips[0].length = length * 25;
            let strip = &mut track.audio.strip;
            strip.inserts.push(Insert {
                id: "fx1".into(),
                on: true,
                effect: Effect::Gain(GainFx { gain_db: 0.0 }),
            });
            strip.fader_db = -24.0;
            strip.sends.push(columbina_model::Send {
                bus: "B2".into(),
                on: true,
                pre: n % 2 == 0,
                level_db: -12.0,
                pan: columbina_model::Pan::default(),
            });
            track
        })
        .collect();
    project.media[0].facts.duration = columbina_model::Time {
        ticks: (length + 60) * 48_000,
        scale: 48_000,
    };
    project
}

#[test]
#[allow(clippy::cast_precision_loss, reason = "a share of the time")]
fn sixteen_tracks_play_thirty_seconds_on_threads_with_nothing_short() {
    let seconds = std::env::var("COLUMBINA_STRESS_SECONDS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(30);
    let project = sixteen_costly_tracks(i64::try_from(seconds).unwrap());
    let counters = Arc::new(Counters::default());
    let (mut commands, taken) = messages(64);
    let (gone, mut retired) = messages(64);
    let engine = Engine::new(taken, gone, Arc::clone(&counters));
    let (mut player, mut control) = device::player((2, RATE), false, false, None);
    let options = Options::new(MOST_BLOCK, Purpose::Preview).with_maker(costly);
    let mut graph = Graph::build(&project, 0, options).unwrap();
    let mut tone = TONE;
    graph.start(0, &mut tone).unwrap();
    let (feed, readers) = ring_feed(&graph, graph.reads_at(), Arc::clone(&counters));
    let feeds = readers
        .iter()
        .map(|_| Box::new(TONE) as Box<dyn ClipFeed + Send>)
        .collect();
    let fetcher = Fetcher::spawn(readers, feeds).unwrap();
    let mut resampler = Resampler::new(RATE, RATE, 2, MOST_BLOCK);
    resampler.start(0);
    let (producer, consumer) = ring(16_384, 2);
    control
        .orders
        .push(Order::Play(Feeding {
            generation: 1,
            ring: consumer,
        }))
        .unwrap();
    let run = Box::new(Run::new(graph, feed, Some(fetcher), i64::MAX));
    let session = Session::new(1, run, resampler, producer, Arc::clone(&control.shared));
    // Let the reader get ahead before the first run, as a session that starts does.
    std::thread::sleep(Duration::from_millis(300));
    commands.push(Command::Start(Box::new(session))).unwrap();
    let mix = MixThread::spawn(engine).unwrap();
    // The device: a run of 10 ms at each tick of the clock of the system.
    let quit = Arc::new(AtomicBool::new(false));
    let device = std::thread::spawn({
        let quit = Arc::clone(&quit);
        move || {
            let mut out = vec![0.0_f32; 2 * PERIOD];
            let epoch = Instant::now();
            let mut next = Instant::now();
            let mut runs = 0_u64;
            while !quit.load(Ordering::Acquire) {
                let now = Instant::now();
                if now < next {
                    std::thread::sleep(next - now);
                    continue;
                }
                let at = i64::try_from(now.duration_since(epoch).as_nanos()).unwrap();
                player.fill(&mut out, (at, at), 0.0, |x| x);
                runs += 1;
                next += Duration::from_millis(10);
            }
            runs
        }
    });
    let started = Instant::now();
    std::thread::sleep(Duration::from_secs(seconds));
    quit.store(true, Ordering::Release);
    let runs = device.join().unwrap();
    let elapsed = started.elapsed();
    let busy = Duration::from_nanos(mix.busy());
    // The time of the processor that the thread took (Windows), not the time of the clock it
    // was busy, into which the other threads of the machine come.
    let cpu = Duration::from_nanos(mix.cpu());
    let priority = mix.priority();
    drop(mix);
    while retired.pop().is_some() {}
    let share = if cpu > Duration::ZERO { cpu } else { busy }.as_secs_f64() / elapsed.as_secs_f64();
    println!(
        "the thread of the mix: priority {priority:?}, busy {busy:?}, on the processor {cpu:?}"
    );
    if cfg!(windows) {
        assert_ne!(priority, columbina_audio::play::thread::Priority::Normal);
    }
    println!(
        "{seconds} s of 16 tracks: {runs} runs, {} short, {} reads short, the mix busy {:.1} % of \
         the time",
        control.shared.short(),
        counters.underruns(),
        share * 100.0
    );
    assert_eq!(control.shared.short(), 0);
    assert_eq!(counters.underruns(), 0);
    // Half a core, built for release; the graph built for debugging takes some 40 % on its own.
    let most = if cfg!(debug_assertions) { 0.75 } else { 0.5 };
    assert!(share < most, "{share}");
    assert!(counters.mixed() > i64::try_from(seconds).unwrap() * 47_000);
}

#[test]
fn changes_that_keep_coming_are_heard_one_graph_after_another() {
    // A graph every 20 ms for a second, each 150 ms ahead: each waits behind the one before.
    let mut sim = Sim::of(&one_track(), 0, TONE);
    let mut last = one_track();
    for tick in 0..200 {
        if (20..120).contains(&tick) && tick % 2 == 0 {
            #[allow(clippy::cast_precision_loss, reason = "a gain in decibels")]
            let db = -f64::from(tick % 7);
            last = with_gain(db);
            sim.swap(&last, 7_200);
        }
        sim.tick();
    }
    let swaps = sim.counters.swaps();
    println!("{swaps} graphs swapped in for 50 sent");
    assert!(swaps >= 6, "{swaps}");
    assert_eq!(sim.counters.late(), 0);
    // The last one sent is heard in the end, on time.
    let mut after = Sim::of(&last, 0, TONE);
    for _ in 0..200 {
        after.tick();
    }
    let (got, want) = (sim.left(), after.left());
    let tail = got.len() - 9_600..got.len();
    let most = got[tail.clone()]
        .iter()
        .zip(&want[tail])
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f32::max);
    assert!(most < 1e-6, "{most}");
    // No click: no step larger than those of the tone at 0 dB, which it starts at.
    let normal = largest_step(&got[..9_600]);
    let steps = largest_step(&got);
    assert!(steps <= normal * 1.1, "{steps} > {normal} · 1.1");
}

#[test]
fn a_setting_of_a_graph_that_waits_to_be_swapped_in_is_heard_once_it_is() {
    let mut sim = Sim::of(&one_track(), 0, LEVEL);
    for _ in 0..20 {
        sim.tick();
    }
    // The graph built last waits 150 ms; its fader is set to −∞ meanwhile.
    let at = usize::try_from(sim.swap(&with_gain(0.0), 7_200)).unwrap();
    sim.set(&a1(), "fader_db", -100.0);
    for _ in 0..40 {
        sim.tick();
    }
    assert_eq!(sim.counters.swaps(), 1);
    let left = sim.left();
    assert!(left[..at].iter().skip(100).all(|&x| (x - 0.5).abs() < 1e-6));
    assert!(left[at + 1_000..].iter().all(|&x| x == 0.0));
}

#[test]
fn the_clips_are_read_half_a_second_ahead_of_the_graph() {
    let project = session::sixteen_tracks();
    let counters = Arc::new(Counters::default());
    let mut graph = Graph::build(&project, 0, Options::new(MOST_BLOCK, Purpose::Preview)).unwrap();
    let mut tone = TONE;
    graph.start(9_600, &mut tone).unwrap();
    let (mut feed, mut fetches) = ring_feed(&graph, graph.reads_at(), Arc::clone(&counters));
    for fetch in &mut fetches {
        while fetch.pass(&mut tone, 64) > 0 {}
        assert!(fetch.primed());
    }
    // Every clip has its sound from where the graph reads to half a second after it (the
    // filter's reach of the clips at 1.5 times aside), and not much more.
    let from = graph.reads_at();
    let reaches = graph.reaches();
    let ahead = i64::try_from(samples(columbina_audio::constants::PREFETCH_AHEAD, RATE)).unwrap();
    let mut out = vec![vec![0.0_f32; 2048]; 24];
    for (clip, reach) in reaches.iter().enumerate() {
        let channels = graph.clips()[clip].channels;
        let near = reach.source(from + ahead - 100..from + ahead).unwrap();
        let length = usize::try_from(near.end - near.start).unwrap();
        let mut slices: Vec<&mut [f32]> = out
            .iter_mut()
            .take(channels)
            .map(|channel| &mut channel[..length])
            .collect();
        feed.read(clip, near, &mut slices).unwrap();
    }
    assert_eq!(counters.underruns(), 0);
    let far = reaches[0]
        .source(from + 4 * ahead..from + 4 * ahead + 100)
        .unwrap();
    let mut slices: Vec<&mut [f32]> = out.iter_mut().take(1).map(|c| &mut c[..201]).collect();
    feed.read(0, far.start..far.start + 201, &mut slices)
        .unwrap();
    assert_eq!(counters.underruns(), 1, "two seconds ahead is not read yet");
}

#[test]
fn a_media_that_hangs_holds_up_only_the_clips_of_its_reader() {
    // A2 of a second media: the media of A1 hangs for 5 s at its first read, that of A2 does not.
    let mut project = one_track();
    let timeline = &mut project.timelines[0];
    let mut a2 = timeline.tracks[0].clone();
    a2.id = "A2".into();
    a2.clips[0].id = "c9".into();
    a2.clips[0].source = columbina_model::Source::Media {
        media: "m2".into(),
        source_in: columbina_model::Time {
            ticks: 0,
            scale: 48_000,
        },
    };
    timeline.tracks.push(a2);
    let mut m2 = project.media[0].clone();
    m2.id = "m2".into();
    project.media.push(m2);
    let counters = Arc::new(Counters::default());
    let (mut commands, taken) = messages(64);
    let (gone, _retired) = messages(64);
    let mut engine = Engine::new(taken, gone, Arc::clone(&counters));
    let (mut player, mut control) = device::player((2, RATE), false, false, None);
    let mut graph = Graph::build(&project, 0, Options::new(MOST_BLOCK, Purpose::Preview)).unwrap();
    let mut tone = LEVEL;
    graph.start(0, &mut tone).unwrap();
    let (feed, readers) = ring_feed(&graph, graph.reads_at(), Arc::clone(&counters));
    assert_eq!(readers.len(), 2, "a reader for each media");
    let reader_of_a1 = usize::from(graph.clips()[0].media != "m1");
    let feeds: Vec<Box<dyn ClipFeed + Send>> = (0..2)
        .map(|reader| -> Box<dyn ClipFeed + Send> {
            if reader == reader_of_a1 {
                Box::new(Slow {
                    tone: LEVEL,
                    stall: Duration::from_secs(5),
                    wait: Duration::ZERO,
                })
            } else {
                Box::new(LEVEL)
            }
        })
        .collect();
    let fetcher = Fetcher::spawn(readers, feeds).unwrap();
    let mut resampler = Resampler::new(RATE, RATE, 2, MOST_BLOCK);
    resampler.start(0);
    let (producer, consumer) = ring(16_384, 2);
    control
        .orders
        .push(Order::Play(Feeding {
            generation: 1,
            ring: consumer,
        }))
        .unwrap();
    let run = Box::new(Run::new(graph, feed, Some(fetcher), i64::MAX));
    let session = Session::new(1, run, resampler, producer, Arc::clone(&control.shared));
    commands.push(Command::Start(Box::new(session))).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let mut out = vec![0.0; 2 * PERIOD];
    let mut a2_alone = 0;
    for n in 0..60 {
        engine.step();
        player.fill(&mut out, (n * 10_000_000, n * 10_000_000), 0.0, |x| x);
        // A1 silent for want of its media, A2 heard: a half on each side.
        a2_alone += out.iter().filter(|&&x| (x - 0.5).abs() < 1e-6).count();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(a2_alone > 2 * PERIOD * 40, "{a2_alone}");
    assert!(counters.underruns() > 0, "A1 is short");
}

#[test]
fn a_graph_for_a_sample_inside_a_crossfade_under_way_is_sent_back() {
    let mut sim = Sim::of(&one_track(), 0, TONE);
    for _ in 0..20 {
        sim.tick();
    }
    // A block ahead: the next run of the device mixes up to it, and half the crossfade.
    let at = sim.counters.mixed() + 240;
    sim.swap_at(&with_gain(-6.0), at);
    sim.tick();
    let mixed = sim.counters.mixed();
    assert_eq!(mixed, at + 240);
    sim.swap_at(&one_track(), mixed);
    for _ in 0..20 {
        sim.tick();
    }
    assert_eq!(sim.counters.swaps(), 1);
    assert_eq!(sim.counters.late(), 1);
}

#[test]
fn what_the_thread_of_the_mix_cannot_send_back_waits_and_holds_up_the_mix_until_it_can() {
    // A ring back of two that is not emptied: the third graph let go of waits on the thread.
    let counters = Arc::new(Counters::default());
    let (mut commands, taken) = messages(64);
    let (gone, mut retired) = messages(2);
    let mut engine = Engine::new(taken, gone, Arc::clone(&counters));
    let mut sim = Sim::of(&one_track(), 0, TONE);
    let mut graphs = Vec::new();
    for at in [48_000, 48_100, 48_200, 48_300] {
        graphs.push((sim.run_of(&one_track(), at), at));
    }
    let (_player, control) = device::player((2, RATE), false, false, None);
    let mut resampler = Resampler::new(RATE, RATE, 2, MOST_BLOCK);
    resampler.start(0);
    let (producer, _consumer) = ring(16_384, 2);
    let run = sim.run_of(&one_track(), 0);
    let session = Session::new(1, run, resampler, producer, Arc::clone(&control.shared));
    commands.push(Command::Start(Box::new(session))).unwrap();
    engine.step();
    // Four graphs too late: two go back, the rest wait.
    for (run, at) in graphs {
        commands
            .push(Command::Swap {
                epoch: 1,
                run,
                at: at - 48_000,
            })
            .unwrap();
    }
    commands.push(Command::Stop(2)).unwrap();
    engine.step();
    // Two went back, the third waits, and the rest are not taken yet.
    assert_eq!(engine.stashed(), 1);
    // While anything waits, no command is taken and nothing is mixed.
    let mixed = counters.mixed();
    assert!(!engine.step());
    assert_eq!(counters.mixed(), mixed);
    let mut back = 0;
    for _ in 0..10 {
        while retired.pop().is_some() {
            back += 1;
        }
        engine.step();
    }
    // The four graphs, and the session the Stop ended.
    assert_eq!(back, 5);
    assert_eq!(engine.stashed(), 0);
    assert!(engine.session().is_none());
}
